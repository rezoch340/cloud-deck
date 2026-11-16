#!/usr/bin/env python3
"""Execute a versioned application manifest; never allocate or release a VM."""
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.parse
import urllib.request

DRIVE_ROOT = Path('/content/drive/MyDrive')


def progress(stage, message, **details):
    value = {'stage': stage, 'message': message, 'updatedAt': int(time.time() * 1000), **details}
    destination = os.environ.get('CLOUD_DECK_STATUS_FILE')
    if destination:
        target = Path(destination)
        temporary = target.with_suffix('.tmp')
        descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(descriptor, 'w') as stream:
            json.dump(value, stream, ensure_ascii=False)
        os.replace(temporary, target)
    print(time.strftime('[%H:%M:%S] ') + message, flush=True)


def absolute_path(value):
    if not isinstance(value, str) or not value or any(character in value for character in '\0\n\r'):
        raise ValueError('Invalid manifest path')
    result = Path(value)
    if not result.is_absolute() or '..' in result.parts or result == Path('/'):
        raise ValueError('Manifest paths must be specific absolute paths')
    return result


def instance_path(value):
    result = absolute_path(value)
    if result.is_relative_to(DRIVE_ROOT.parent):
        raise ValueError('Instance targets cannot be inside the Drive mount')
    return result


def drive_path(value):
    result = absolute_path(value)
    if not result.is_relative_to(DRIVE_ROOT) or result == DRIVE_ROOT:
        raise ValueError('Drive paths must be below MyDrive')
    return result


def safe_parents(path):
    if any(parent.is_symlink() for parent in (path, *path.parents)):
        raise ValueError('Manifest file operations cannot follow symbolic links')


def readiness_config(runtime, kind='service'):
    check = runtime.get('readiness')
    legacy = runtime.get('healthPath')
    if check is None and kind == 'service' and legacy is not None and (not isinstance(legacy, str) or not legacy.startswith('/') or legacy.startswith('//') or len(legacy) > 512 or any(character in legacy for character in '\0\n\r')):
        raise ValueError('Invalid legacy health-check path')
    if check is None and kind == 'service' and runtime.get('healthPath') and runtime.get('remotePort'):
        check = {'type': 'http', 'url': f"http://127.0.0.1:{runtime['remotePort']}{runtime['healthPath']}"}
    if kind == 'task':
        if check is not None:
            raise ValueError('Tasks use their exit code, not a readiness probe')
        return None
    if not isinstance(check, dict) or check.get('type') not in ('http', 'tcp', 'command'):
        raise ValueError('Services require an http, tcp or command readiness check')
    for field, default, maximum in (('timeoutSeconds', 120, 86400), ('intervalSeconds', 2, 300), ('probeTimeoutSeconds', 5, 30)):
        value = check.get(field, default)
        if type(value) is not int or not 1 <= value <= maximum:
            raise ValueError('Invalid readiness ' + field)
    if check['type'] == 'http':
        if not isinstance(check.get('url'), str):
            raise ValueError('Readiness requires an HTTP(S) URL')
        address = urllib.parse.urlsplit(check.get('url', ''))
        if address.scheme not in ('http', 'https') or not address.hostname or address.username or address.password or any(character in check['url'] for character in '\0\n\r'):
            raise ValueError('Readiness requires an HTTP(S) URL without credentials')
        if address.port is not None and not 1 <= address.port <= 65535:
            raise ValueError('Invalid readiness HTTP port')
        status = check.get('status', 200)
        if type(status) is not int or not 100 <= status <= 599:
            raise ValueError('Invalid readiness HTTP status')
    elif check['type'] == 'tcp':
        host, port = check.get('host'), check.get('port')
        if not isinstance(host, str) or not host or len(host) > 253 or any(character.isspace() or character in '/\0' for character in host):
            raise ValueError('Invalid readiness TCP host')
        if type(port) is not int or not 1 <= port <= 65535:
            raise ValueError('Invalid readiness TCP port')
    else:
        command = check.get('command')
        if not isinstance(command, str) or not command.strip() or len(command.encode()) > 4000 or '\0' in command:
            raise ValueError('Invalid readiness command')
        directory = check.get('workingDirectory', runtime.get('workingDirectory'))
        if directory != '/':
            absolute_path(directory)
    return dict(check)


def validate_manifest(manifest):
    if not isinstance(manifest, dict) or manifest.get('schemaVersion') != 1:
        raise ValueError('Unsupported application manifest version')
    if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_-]{0,79}', str(manifest.get('id', ''))):
        raise ValueError('Invalid manifest application ID')
    if manifest.get('kind') not in ('service', 'task'):
        raise ValueError('Invalid application kind')
    if len(json.dumps(manifest).encode()) > 512 * 1024:
        raise ValueError('Manifest exceeds 512 KiB')
    if 'details' in manifest and (not isinstance(manifest['details'], str) or len(manifest['details'].encode()) > 64 * 1024):
        raise ValueError('details must be Markdown text of at most 64 KiB')
    if 'category' in manifest and manifest['category'] not in ('audio', 'video', 'llm', 'workflow', 'general'):
        raise ValueError('Invalid application category')
    for section in ('runtime', 'resources', 'startup', 'shutdown'):
        if not isinstance(manifest.get(section), dict):
            raise ValueError('Missing manifest section: ' + section)
    for section, name in (('resources', 'files'), ('startup', 'commands'), ('startup', 'restore'), ('shutdown', 'save')):
        entries = manifest[section].get(name)
        if not isinstance(entries, list) or len(entries) > 128 or any(not isinstance(entry, dict) for entry in entries):
            raise ValueError('Invalid manifest list: ' + section + '.' + name)
    runtime = manifest['runtime']
    readiness_config(runtime, manifest['kind'])
    environment = runtime.get('environment')
    if not isinstance(environment, dict) or len(environment) > 64 or any(not isinstance(key, str) or not re.fullmatch(r'[A-Za-z_][A-Za-z0-9_]*', key) or not isinstance(value, str) or '\0' in value or len(value) > 8192 for key, value in environment.items()):
        raise ValueError('Invalid runtime environment')
    if 'installation' in manifest or 'usePreparedLaunch' in runtime:
        raise ValueError('Use commands or scripts for application setup and launch')
    if runtime.get('workingDirectory') != '/':
        absolute_path(runtime.get('workingDirectory'))
    command = runtime.get('command')
    if not isinstance(command, str) or not command.strip() or '\0' in command or len(command) > 4000:
        raise ValueError('Invalid application command')
    startup = manifest.get('startup', {})
    shutdown = manifest.get('shutdown', {})
    for step in startup.get('commands', []):
        command = step.get('command')
        if not isinstance(command, str) or not command.strip() or '\0' in command or len(command) > 4000:
            raise ValueError('Invalid startup command')
        if step.get('workingDirectory') != '/':
            absolute_path(step.get('workingDirectory'))
    for rule in startup.get('restore', []):
        if type(rule.get('optional', True)) is not bool:
            raise ValueError('optional must be a boolean')
        drive_path(rule.get('source'))
        instance_path(rule.get('target'))
    save_sources = set()
    for rule in shutdown.get('save', []):
        if rule.get('source') in save_sources:
            raise ValueError('Duplicate save source')
        save_sources.add(rule.get('source'))
        instance_path(rule.get('source'))
        drive_path(rule.get('target'))
    resources = manifest.get('resources', {})
    targets = set()
    for resource in resources.get('files', []):
        target = instance_path(resource.get('target'))
        if target in targets:
            raise ValueError('Duplicate resource target')
        targets.add(target)
        if type(resource.get('size')) is not int or resource['size'] <= 0 or not re.fullmatch(r'[a-fA-F0-9]{64}', str(resource.get('sha256', ''))):
            raise ValueError('Resources require a size and SHA-256')
        if not isinstance(resource.get('sources'), list) or not resource['sources'] or len(resource['sources']) > 128 or any(not isinstance(source, dict) for source in resource['sources']):
            raise ValueError('Resource sources are required')
        for source in resource['sources']:
            if source.get('kind') == 'drive':
                drive_path(source.get('path'))
            elif source.get('kind') == 'download':
                parsed = urllib.parse.urlsplit(source.get('url', ''))
                if parsed.scheme != 'https' or not parsed.hostname or parsed.username or parsed.password:
                    raise ValueError('Downloads require an HTTPS URL without credentials')
            else:
                raise ValueError('Unknown resource source kind')
    return manifest


def recipe_from_manifest(manifest):
    validate_manifest(manifest)
    runtime = manifest['runtime']
    rules = manifest.get('shutdown', {}).get('save', [])
    return {
        'id': manifest['id'], 'name': manifest['name'], 'description': manifest.get('description', ''),
        'kind': manifest['kind'], **runtime, 'manifest': manifest,
        'readiness': readiness_config(runtime, manifest['kind']),
        'saveDirectories': [rule['source'] for rule in rules],
        'storageDirectory': str(Path(rules[0]['target']).parent) if rules else '',
    }


def matches_resource(path, resource):
    safe_parents(path)
    if not path.is_file() or path.stat().st_size != resource['size']:
        return False
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        while block := stream.read(8 * 1024 * 1024):
            digest.update(block)
    return digest.hexdigest() == resource['sha256'].lower()


def stage_resource(resource, opener=urllib.request.urlopen):
    target = instance_path(resource['target'])
    progress('resource_check', '检查已有资源：' + str(target), target=str(target))
    if matches_resource(target, resource):
        progress('resource_ready', '已有资源校验通过：' + str(target), target=str(target))
        return {'target': str(target), 'source': 'local', 'size': resource['size']}
    target.parent.mkdir(parents=True, exist_ok=True)
    for source in resource['sources']:
        temporary = None
        try:
            descriptor, filename = tempfile.mkstemp(prefix='.cloud-deck-resource-', dir=target.parent)
            os.close(descriptor)
            temporary = Path(filename)
            if source['kind'] == 'drive':
                progress('resource_copy', '从 Drive 复制：' + str(target), target=str(target))
                origin = drive_path(source['path'])
                safe_parents(origin)
                if not origin.is_file() or origin.stat().st_size != resource['size']:
                    continue
                shutil.copyfile(origin, temporary)
            else:
                progress('resource_download', '下载资源：' + str(target), target=str(target), completedBytes=0, totalBytes=resource['size'])
                request = urllib.request.Request(source['url'], headers={'User-Agent': 'Cloud-Deck/1'})
                with opener(request, timeout=60) as response, temporary.open('wb') as output:
                    if urllib.parse.urlsplit(response.geturl()).scheme != 'https':
                        raise ValueError('Resource redirected away from HTTPS')
                    copied = 0
                    reported = time.monotonic()
                    while block := response.read(8 * 1024 * 1024):
                        copied += len(block)
                        if copied > resource['size']:
                            raise ValueError('Resource exceeded its declared size')
                        output.write(block)
                        if time.monotonic() - reported >= 2 or copied == resource['size']:
                            progress('resource_download', f'下载 {target.name}：{copied} / {resource["size"]} 字节',
                                     target=str(target), completedBytes=copied, totalBytes=resource['size'])
                            reported = time.monotonic()
                    output.flush()
                    os.fsync(output.fileno())
            progress('resource_verify', '校验下载文件 SHA-256：' + str(target), target=str(target))
            if not matches_resource(temporary, resource):
                raise ValueError('Resource checksum did not match')
            safe_parents(target)
            os.replace(temporary, target)
            return {'target': str(target), 'source': source['kind'], 'size': resource['size']}
        except (OSError, ValueError) as error:
            progress('resource_fallback', '资源来源失败，尝试下一来源：' + str(target) + '（' + str(error) + '）', target=str(target))
            print(json.dumps({'stage': 'resource_source_failed', 'target': str(target), 'source': source['kind']}), flush=True)
        finally:
            if temporary is not None:
                temporary.unlink(missing_ok=True)
    raise ValueError('No verified resource source was available: ' + str(target))


def restore_files(rule):
    source, target = drive_path(rule['source']), instance_path(rule['target'])
    safe_parents(source)
    safe_parents(target)
    if not source.exists():
        if rule.get('optional', True):
            return 0
        raise ValueError('Required restore source is missing')
    sources = sorted(source.rglob('*')) if source.is_dir() else [source]
    if source.is_dir():
        target.mkdir(parents=True, exist_ok=True)
    restored = 0
    for origin in sources:
        relative = origin.relative_to(source) if source.is_dir() else Path('.')
        if '.cloud-deck-backups' in relative.parts or origin.name.endswith('.sync.part'):
            continue
        if origin.is_symlink():
            raise ValueError('Restore sources cannot contain symbolic links')
        if origin.is_dir():
            continue
        destination = target / relative if source.is_dir() else target
        safe_parents(destination)
        if not origin.is_file():
            raise ValueError('Restore sources must contain regular files')
        if destination.exists():
            if not destination.is_file():
                raise ValueError('Restore destination is not a regular file')
            continue  # Preserve instance-local changes, including after a failed save.
        destination.parent.mkdir(parents=True, exist_ok=True)
        descriptor, filename = tempfile.mkstemp(prefix='.cloud-deck-restore-', dir=destination.parent)
        os.close(descriptor)
        temporary = Path(filename)
        try:
            shutil.copy2(origin, temporary)
            os.replace(temporary, destination)
            restored += 1
        finally:
            temporary.unlink(missing_ok=True)
    return restored


def prepare(manifest, directory):
    validate_manifest(manifest)
    progress('preparing', '开始准备应用：' + manifest['name'])
    directory = Path(directory)
    safe_parents(directory)
    directory.mkdir(parents=True, exist_ok=True)
    resources = manifest.get('resources', {})
    manifest_file = directory / 'manifest.json'
    safe_parents(manifest_file)
    manifest_file.write_text(json.dumps(manifest, ensure_ascii=False))
    manifest_file.chmod(0o600)
    evidence = []
    for resource in resources.get('files', []):
        result = stage_resource(resource)
        evidence.append(result)
        print(json.dumps({'stage': 'resource_ready', **result}), flush=True)
    for rule in manifest.get('startup', {}).get('restore', []):
        progress('restoring', '恢复文件：' + rule['target'], target=rule['target'])
        count = restore_files(rule)
        progress('restored', f'文件恢复完成：{rule["target"]}（{count} 个文件）', target=rule['target'])
        print(json.dumps({'stage': 'files_restored', 'target': rule['target'], 'count': count}), flush=True)
    environment = {**os.environ, **manifest['runtime'].get('environment', {}),
                   'CLOUD_DECK_MANIFEST_FILE': str(manifest_file)}
    if 'CLOUD_DECK_APP_DIR' in os.environ:
        environment['CLOUD_DECK_APP_DIR'] = os.environ['CLOUD_DECK_APP_DIR']
    commands = manifest.get('startup', {}).get('commands', [])
    for index, step in enumerate(commands, 1):
        progress('command', f'执行准备命令 {index}/{len(commands)}：{step["command"]}',
                 command=step['command'], step=index, totalSteps=len(commands), workingDirectory=step['workingDirectory'])
        try:
            subprocess.run(['/bin/sh', '-c', step['command']], cwd=step['workingDirectory'], env=environment, check=True)
        except subprocess.CalledProcessError as error:
            progress('failed', f'准备命令 {index}/{len(commands)} 失败，退出码 {error.returncode}',
                     command=step['command'], step=index, totalSteps=len(commands), exitCode=error.returncode)
            raise
        progress('command_completed', f'准备命令 {index}/{len(commands)} 已完成，退出码 0', step=index, totalSteps=len(commands))
    (directory / 'resources-verified.json').write_text(json.dumps(evidence, indent=2))
    print(json.dumps({'stage': 'manifest_prepared', 'applicationId': manifest['id']}), flush=True)
    progress('prepared', '资源与准备命令已完成，等待启动应用。')


if __name__ == '__main__':
    manifest_file = Path(sys.argv[1])
    prepare(json.loads(manifest_file.read_text()), manifest_file.parent)
