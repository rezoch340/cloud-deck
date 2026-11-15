#!/usr/bin/env python3
"""Cloud Deck's bundled Linux application supervisor.

Owns application processes, not the VM. HTTP is loopback-only and authenticated.
A disconnected desktop never stops apps or releases the instance. No Google
credentials, cloud allocation or claims of durable Drive synchronization here.
"""
import argparse
import contextlib
import fcntl
import hmac
import hashlib
import http.server
import ipaddress
import importlib.util
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.parse
import urllib.request

MAX_BODY = 1024 * 1024
MAX_LOG = 64 * 1024
LIFECYCLE_PATH = Path(__file__).with_name('cloud_lifecycle.py')
LIFECYCLE_SPEC = importlib.util.spec_from_file_location('cloud_deck_lifecycle', LIFECYCLE_PATH)
lifecycle = importlib.util.module_from_spec(LIFECYCLE_SPEC)
LIFECYCLE_SPEC.loader.exec_module(lifecycle)
HEARTBEAT_PATH = Path(__file__).with_name('cloud_heartbeat.py')
HEARTBEAT_SPEC = importlib.util.spec_from_file_location('cloud_deck_heartbeat', HEARTBEAT_PATH)
heartbeat_module = importlib.util.module_from_spec(HEARTBEAT_SPEC)
HEARTBEAT_SPEC.loader.exec_module(heartbeat_module)
SCRIPT_REVISION = hashlib.sha256(Path(__file__).read_bytes() + LIFECYCLE_PATH.read_bytes() + HEARTBEAT_PATH.read_bytes()).hexdigest()
DRIVE_MOUNT = Path('/content/drive')


def atomic_json(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    temporary = path.with_suffix('.tmp')
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, 'w') as stream:
        json.dump(value, stream, ensure_ascii=False)
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temporary, path)


def process_identity(pid):
    """Linux start time + boot ID prevent a recycled PID from becoming ours."""
    try:
        fields = Path(f'/proc/{pid}/stat').read_text().rsplit(') ', 1)[1].split()
        if fields[0] == 'Z':
            return None
        return Path('/proc/sys/kernel/random/boot_id').read_text().strip() + ':' + fields[19]
    except (OSError, IndexError):
        return None


def validate_recipe(value):
    if isinstance(value, dict) and value.get('manifest') is not None:
        value = lifecycle.recipe_from_manifest(value['manifest'])
    if not isinstance(value, dict) or not re.fullmatch(r'[a-zA-Z0-9][a-zA-Z0-9_-]{0,79}', str(value.get('id', ''))):
        raise ValueError('Invalid application ID')
    if value.get('kind') not in ('service', 'task'):
        raise ValueError('Expected service or task')
    for key in ('name', 'command', 'workingDirectory'):
        if not isinstance(value.get(key), str) or not value[key].strip() or '\0' in value[key] or len(value[key]) > 4000:
            raise ValueError('Invalid ' + key)
    directory = Path(value['workingDirectory'])
    if not directory.is_absolute() or '..' in directory.parts:
        raise ValueError('Working directory must be absolute')
    if value.get('prepareCommand') is not None and (not isinstance(value['prepareCommand'], str) or len(value['prepareCommand']) > 4000 or '\0' in value['prepareCommand']):
        raise ValueError('Invalid preparation command')
    environment = value.get('environment') or {}
    if not isinstance(environment, dict) or len(environment) > 64 or any(not isinstance(key, str) or not re.fullmatch(r'[A-Za-z_][A-Za-z0-9_]*', key) or not isinstance(item, str) or '\0' in item or len(item) > 8192 for key, item in environment.items()):
        raise ValueError('Invalid environment')
    if value['kind'] == 'service':
        port = value.get('remotePort')
        if port is not None and (type(port) is not int or not 1 <= port <= 65535):
            raise ValueError('Invalid service port')
    value['readiness'] = lifecycle.readiness_config(value, value['kind'])
    return dict(value)


class NoProbeRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, address):
        return None


def probe_readiness(recipe, check, environment, timeout, attempt=None):
    try:
        if check['type'] == 'http':
            opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoProbeRedirect())
            try:
                with opener.open(check['url'], timeout=timeout) as response:
                    return response.status == check.get('status', 200)
            except urllib.error.HTTPError as error:
                return error.code == check.get('status', 200)
        if check['type'] == 'tcp':
            with socket.create_connection((check['host'], check['port']), timeout=timeout):
                return True
        with attempt.lock if attempt else contextlib.nullcontext():
            if attempt and attempt.cancelled.is_set():
                return False
            process = subprocess.Popen(['/bin/sh', '-c', check['command']],
                                       cwd=check.get('workingDirectory', recipe['workingDirectory']),
                                       env=environment, stdin=subprocess.DEVNULL,
                                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
            if attempt:
                attempt.process = process
        try:
            return process.wait(timeout=timeout) == 0
        finally:
            # A timed-out or successful probe must not leave background writers behind.
            with contextlib.suppress(ProcessLookupError):
                os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=2)
            deadline = time.monotonic() + 2
            while group_active(process.pid) and time.monotonic() < deadline:
                time.sleep(0.02)
            if group_active(process.pid):
                raise ValueError('Readiness probe descendants did not exit')
    except (OSError, ValueError, urllib.error.URLError, subprocess.TimeoutExpired):
        return False


class ReadinessAttempt:
    """Keep slow probes outside the supervisor lock; cancel their writers before saving."""
    def __init__(self, recipe, check, environment, timeout):
        self.lock = threading.Lock()
        self.cancelled = threading.Event()
        self.done = threading.Event()
        self.process = None
        self.ready = False
        def execute():
            try:
                self.ready = probe_readiness(recipe, check, environment, timeout, self) and not self.cancelled.is_set()
            finally:
                self.done.set()
        self.thread = threading.Thread(target=execute, daemon=True)
        self.thread.start()

    def cancel(self):
        self.cancelled.set()
        with self.lock:
            if self.process is not None:
                with contextlib.suppress(ProcessLookupError):
                    os.killpg(self.process.pid, signal.SIGKILL)
                self.process.wait(timeout=2)
                deadline = time.monotonic() + 2
                while group_active(self.process.pid) and time.monotonic() < deadline:
                    time.sleep(0.02)
                if group_active(self.process.pid):
                    raise ValueError('Readiness probe is still running; save is blocked')


def group_active(group):
    if not group:
        return False
    if sys.platform.startswith('linux'):
        for entry in Path('/proc').glob('[0-9]*/stat'):
            try:
                fields = entry.read_text().rsplit(') ', 1)[1].split()
                if fields[0] != 'Z' and int(fields[2]) == group:
                    return True
            except (OSError, IndexError, ValueError):
                continue
        return False
    # The agent runs on Linux; POSIX ps also lets its offline checks run on macOS.
    output = subprocess.check_output(['ps', '-axo', 'pgid=,stat='], text=True)
    return any(int(fields[0]) == group and not fields[1].startswith('Z')
               for line in output.splitlines() if len(fields := line.split()) >= 2)


def signal_owned_group(group, requested_signal):
    try:
        os.killpg(group, requested_signal)
    except ProcessLookupError:
        pass
    except PermissionError:
        # macOS can return EPERM when the last member exits during killpg.
        # A live or unobservable group still blocks stopping and saving.
        if group_active(group):
            raise


def file_hashes(path):
    sha, md5 = hashlib.sha256(), hashlib.md5()
    with path.open('rb') as stream:
        while block := stream.read(8 * 1024 * 1024):
            sha.update(block); md5.update(block)
    return {'size': path.stat().st_size, 'sha256': sha.hexdigest(), 'md5': md5.hexdigest()}


def saved_files(recipes):
    """Snapshot configured data only; symlinks and ambiguous destinations fail closed."""
    records, destinations = [], set()
    drive_root = DRIVE_MOUNT / 'MyDrive'
    for recipe in recipes:
        rules = recipe.get('manifest', {}).get('shutdown', {}).get('save') if recipe.get('manifest') else None
        directories = recipe.get('saveDirectories', [])
        if rules is not None:
            directories = [rule['source'] for rule in rules]
        if not directories:
            continue
        storage = Path(recipe.get('storageDirectory', ''))
        if rules is not None and rules:
            storage = Path(rules[0]['target'])
        if not storage.is_absolute() or '..' in storage.parts or not storage.is_relative_to(drive_root) or storage == drive_root:
            raise ValueError('Storage must be a directory below Google Drive MyDrive')
        names = set()
        for directory in directories:
            source = Path(directory)
            if not source.is_absolute() or '..' in source.parts or source.is_symlink() or not (source.is_dir() or (rules is not None and source.is_file())) or source == Path('/') or source.is_relative_to(DRIVE_MOUNT) or (rules is None and source.name in names):
                raise ValueError('Save directory is missing, ambiguous or unsafe')
            lifecycle.safe_parents(source)
            names.add(source.name)
            target_root = Path(next(rule['target'] for rule in rules if rule['source'] == directory)) if rules is not None else storage / source.name
            if not target_root.is_relative_to(drive_root) or target_root == drive_root or '..' in target_root.parts:
                raise ValueError('Save targets must be below Google Drive MyDrive')
            for path in sorted(source.rglob('*')) if source.is_dir() else [source]:
                if path.is_symlink():
                    raise ValueError('Save directories cannot contain symbolic links')
                if path.is_dir():
                    continue
                if not path.is_file():
                    raise ValueError('Save directories can contain only regular files')
                relative = path.relative_to(source) if source.is_dir() else Path('.')
                if '.cloud-deck-backups' in relative.parts:
                    continue
                target = target_root / relative if source.is_dir() else target_root
                lifecycle.safe_parents(target)
                if str(target) in destinations:
                    raise ValueError('Applications have overlapping save destinations')
                destinations.add(str(target))
                records.append({'applicationId': recipe['id'], 'path': str(Path(source.name) / relative),
                                'source': str(path), 'target': str(target), **file_hashes(path)})
    return records


def has_save_rules(recipes):
    return any(recipe['manifest']['shutdown']['save'] if recipe.get('manifest')
               else recipe.get('saveDirectories', []) for recipe in recipes)


class DriveReader:
    """Read server metadata using the existing Colab Drive authorization; never export it."""
    def __init__(self):
        environment = dict(part.split(b'=', 1) for part in Path('/proc/1/environ').read_bytes().split(b'\0') if b'=' in part)
        base = environment[b'TBE_EPHEM_CREDS_ADDR'].decode()
        if not base.startswith(('http://', 'https://')):
            base = 'http://' + base
        address = urllib.parse.urlsplit(base)
        try:
            private = ipaddress.ip_address(address.hostname).is_private
        except ValueError:
            private = address.hostname == 'localhost' or address.hostname.endswith('.internal')
        if address.scheme not in ('http', 'https') or not private or address.username or address.password or address.query or address.fragment:
            raise ValueError('Unexpected Colab credential service')
        request = urllib.request.Request(base.rstrip('/') + '/computeMetadata/v1/instance/service-accounts/default/token', headers={'Metadata-Flavor': 'Google'})
        with urllib.request.urlopen(request, timeout=15) as response:
            self.token = json.load(response)['access_token']

    def children(self, parent):
        files, page = [], None
        while True:
            query = {'q': "'" + parent + "' in parents and trashed = false", 'pageSize': 1000,
                     'fields': 'nextPageToken,files(id,name,mimeType,size,md5Checksum)'}
            if page:
                query['pageToken'] = page
            request = urllib.request.Request('https://www.googleapis.com/drive/v3/files?' + urllib.parse.urlencode(query), headers={'Authorization': 'Bearer ' + self.token})
            with urllib.request.urlopen(request, timeout=30) as response:
                result = json.load(response)
            files.extend(result.get('files', []))
            page = result.get('nextPageToken')
            if not page:
                return files

    def verify(self, records):
        # ponytail: list each directory once per pass; no local FUSE cache can satisfy this check.
        directories = {'root': self.children('root')}
        for record in records:
            parts = Path(record['target']).relative_to(DRIVE_MOUNT / 'MyDrive').parts
            parent = 'root'
            for index, part in enumerate(parts):
                if parent not in directories:
                    directories[parent] = self.children(parent)
                matches = [item for item in directories[parent] if item['name'] == part]
                if len(matches) != 1:
                    return False
                entry = matches[0]
                if index < len(parts) - 1:
                    if entry['mimeType'] != 'application/vnd.google-apps.folder':
                        return False
                    parent = entry['id']
                elif entry.get('md5Checksum') != record['md5'] or int(entry.get('size', -1)) != record['size']:
                    return False
        return True


def copy_saved_files(records, receipt_id):
    """Never propagate deletions. Preserve differing target files as versioned backups."""
    backups = []
    for record in records:
        source, target = Path(record['source']), Path(record['target'])
        if file_hashes(source) != {key: record[key] for key in ('size', 'sha256', 'md5')}:
            raise ValueError('Source changed during synchronization')
        for parent in (target, *target.parents):
            if parent.is_symlink():
                raise ValueError('Save target cannot contain symbolic links')
        target.parent.mkdir(parents=True, exist_ok=True)
        if target.exists():
            if not target.is_file():
                raise ValueError('Save target is not a regular file')
            old = file_hashes(target)
            if old['sha256'] == record['sha256']:
                continue
            backup = target.parent / '.cloud-deck-backups' / receipt_id / target.name
            if any(parent.is_symlink() for parent in (backup, *backup.parents)):
                raise ValueError('Backup target cannot contain symbolic links')
            backup.parent.mkdir(parents=True, exist_ok=True)
            with target.open('rb') as incoming, backup.open('xb') as outgoing:
                shutil.copyfileobj(incoming, outgoing, length=8 * 1024 * 1024)
                outgoing.flush(); os.fsync(outgoing.fileno())
            if file_hashes(backup) != old:
                raise ValueError('Drive target changed while preserving its previous version')
            backups.append({'target': str(backup), **old})
        temporary = target.with_name(target.name + '.cloud-deck-' + receipt_id + '.part')
        try:
            with source.open('rb') as incoming, temporary.open('xb') as outgoing:
                shutil.copyfileobj(incoming, outgoing, length=8 * 1024 * 1024)
                outgoing.flush(); os.fsync(outgoing.fileno())
            if file_hashes(temporary)['sha256'] != record['sha256']:
                raise ValueError('Source changed while copying')
            os.replace(temporary, target)
        finally:
            temporary.unlink(missing_ok=True)
    return backups


class Supervisor:
    def __init__(self, root):
        self.root = Path(root)
        self.root.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.lock = threading.RLock()
        self.processes = {}
        self.next_probe = {}
        self.probes = {}
        self.recipes = {}
        self.apps = {}
        self.events = []
        self.synchronization = {'status': 'unobserved'}
        self.sealed = False
        self.started = time.time()
        self.heartbeat = heartbeat_module.CloudHeartbeat(self.root)
        path = self.root / 'applications.json'
        if path.exists():
            saved = json.loads(path.read_text())
            self.recipes = {item['id']: validate_recipe(item) for item in saved.get('recipes', [])}
            self.events = saved.get('events', [])[-300:]
            for item in saved.get('applications', []):
                identity = item.get('processIdentity')
                item.setdefault('processGroup', item.get('pid'))
                alive = identity and identity == process_identity(item.get('pid'))
                if alive:
                    item['state'] = 'preparing' if item.get('stage') == 'prepare' or item.get('kind') == 'service' else 'running'
                elif item.get('state') not in ('stopped', 'failed'):
                    item['state'] = 'unknown'
                if not alive:
                    item['pid'] = None
                self.apps[item['id']] = item
        receipt = self.root / 'synchronization.json'
        if receipt.exists():
            self.synchronization = json.loads(receipt.read_text())
            if self.synchronization.get('status') in ('syncing', 'verified'):
                self.synchronization.update(status='pending', error='Supervisor restarted; verify files again before release')

    def event(self, app_id, level, message):
        self.events.append({'time': int(time.time() * 1000), 'applicationId': app_id, 'level': level, 'message': message})
        self.events = self.events[-300:]
        if app_id:
            with self.log_path(app_id).open('a') as stream:
                stream.write(time.strftime('[%H:%M:%S] ') + message + '\n')

    def save(self):
        atomic_json(self.root / 'applications.json', {'recipes': list(self.recipes.values()), 'applications': list(self.apps.values()), 'events': self.events})

    def alive(self, item):
        process = self.processes.get(item['id'])
        if process is not None:
            return process.poll() is None
        return bool(item.get('processIdentity') and item['processIdentity'] == process_identity(item.get('pid')))

    def log_path(self, app_id):
        return self.root / (app_id + '.log')

    def environment(self, recipe):
        environment = {**os.environ, 'PYTHONUNBUFFERED': '1', **(recipe.get('environment') or {})}
        if recipe.get('manifest'):
            environment.update(CLOUD_DECK_APP_DIR=str(self.root / 'packages' / recipe['id']),
                               CLOUD_DECK_MANIFEST_FILE=str(self.root / 'manifests' / recipe['id'] / 'manifest.json'))
        return environment

    def spawn(self, recipe, stage, launch=None):
        app_id = recipe['id']
        environment = self.environment(recipe)
        self.next_probe.pop(app_id, None)
        prepared = self.root / (app_id + '.prepared.json')
        if stage == 'prepare':
            prepared.unlink(missing_ok=True)
            (self.root / (app_id + '.preparation.json')).unlink(missing_ok=True)
            environment.update(CLOUD_DECK_PREPARE_ONLY='1', CLOUD_DECK_PREPARED_FILE=str(prepared), CLOUD_DECK_STATUS_FILE=str(self.root / (app_id + '.preparation.json')))
            if recipe.get('manifest'):
                manifest_path = self.root / 'manifests' / app_id / 'manifest.json'
                atomic_json(manifest_path, recipe['manifest'])
                argv = [sys.executable, '-I', '-B', str(LIFECYCLE_PATH), str(manifest_path)]
            else:
                argv = ['/bin/sh', '-c', recipe['prepareCommand']]
            cwd = str(self.root)
        else:
            if launch:
                argv = launch.get('argv')
                if not isinstance(argv, list) or not argv or any(not isinstance(arg, str) or not arg or '\0' in arg for arg in argv):
                    raise ValueError('Preparation returned invalid launch arguments')
                extra = launch.get('environment', {})
                if not isinstance(extra, dict) or any(not isinstance(k, str) or not isinstance(v, str) or '\0' in k + v for k, v in extra.items()):
                    raise ValueError('Preparation returned an invalid environment')
                environment.update(extra)
            else:
                argv = ['/bin/sh', '-c', recipe['command']]
            cwd = recipe['workingDirectory']
        with self.log_path(app_id).open('ab', buffering=0) as output:
            process = subprocess.Popen(argv, cwd=cwd, env=environment, stdin=subprocess.DEVNULL, stdout=output, stderr=output, start_new_session=True)
        self.processes[app_id] = process
        self.apps[app_id] = {'id': app_id, 'name': recipe['name'], 'kind': recipe['kind'], 'stage': stage,
                             'state': 'preparing' if stage == 'prepare' or recipe['kind'] == 'service' else 'running',
                             'pid': process.pid, 'processIdentity': process_identity(process.pid), 'processGroup': process.pid,
                             'startedAt': int(time.time() * 1000), 'exitCode': None, 'error': None}
        self.event(app_id, 'info', 'Preparing environment' if stage == 'prepare' else 'Application process started')
        if stage == 'application':
            self.event(app_id, 'info', '启动命令：' + recipe['command'])
            if recipe['kind'] == 'service':
                self.event(app_id, 'info', '等待启动就绪检测：' + recipe['readiness']['type'])
        elif not recipe.get('manifest'):
            self.event(app_id, 'info', '准备命令：' + recipe['prepareCommand'])

    def install_files(self, app_id, files):
        if not isinstance(files, dict) or len(files) > 128 or len(json.dumps(files).encode()) > 512 * 1024:
            raise ValueError('Application scripts must be a file map of at most 512 KiB')
        for name, content in files.items():
            relative = Path(name)
            if not name or relative.is_absolute() or any(part in ('', '.', '..') for part in name.split('/')) or '\\' in name or '\0' in name:
                raise ValueError('Application files must use relative paths')
            if not isinstance(content, str) or '\0' in content:
                raise ValueError('Application files must contain text')
        directory = self.root / 'packages' / app_id
        lifecycle.safe_parents(directory)
        directory.mkdir(parents=True, exist_ok=True)
        for name, content in files.items():
            target = directory / name
            lifecycle.safe_parents(target)
            target.parent.mkdir(parents=True, exist_ok=True)
            descriptor, filename = tempfile.mkstemp(prefix='.upload-', dir=target.parent)
            temporary = Path(filename)
            try:
                with os.fdopen(descriptor, 'w') as stream:
                    stream.write(content)
                    stream.flush()
                    os.fsync(stream.fileno())
                temporary.chmod(0o700)
                os.replace(temporary, target)
            finally:
                temporary.unlink(missing_ok=True)

    def start(self, value, files=None):
        recipe = validate_recipe(value)
        with self.lock:
            if self.synchronization['status'] == 'syncing' or self.sealed:
                raise ValueError('Files are being saved or the instance is prepared for release')
            app_id = recipe['id']
            if app_id in self.apps and self.alive(self.apps[app_id]):
                raise ValueError('Application is already running')
            if recipe['kind'] == 'service' and recipe.get('remotePort'):
                for other_id, other in self.recipes.items():
                    if other_id != app_id and other.get('remotePort') == recipe['remotePort'] and other_id in self.apps and self.alive(self.apps[other_id]):
                        raise ValueError('Port is owned by another managed application')
                with socket.socket() as probe:
                    probe.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                    try:
                        probe.bind(('127.0.0.1', recipe['remotePort']))
                    except OSError as error:
                        raise ValueError('Service port is already occupied; existing processes were not changed') from error
            self.install_files(app_id, {} if files is None else files)
            self.recipes[app_id] = recipe
            self.synchronization = {'status': 'pending'}
            atomic_json(self.root / 'synchronization.json', self.synchronization)
            try:
                self.spawn(recipe, 'prepare' if recipe.get('manifest') or recipe.get('prepareCommand') else 'application')
            except (OSError, ValueError):
                self.apps[app_id] = {'id': app_id, 'name': recipe['name'], 'kind': recipe['kind'], 'stage': 'prepare' if recipe.get('manifest') or recipe.get('prepareCommand') else 'application', 'state': 'failed', 'pid': None, 'processIdentity': None, 'startedAt': int(time.time() * 1000), 'exitCode': None, 'error': 'Could not create application process; check its directory and command'}
                self.event(app_id, 'error', self.apps[app_id]['error'])
                self.save()
                raise ValueError(self.apps[app_id]['error'])
            self.save()
            return dict(self.apps[app_id])

    def stop(self, app_id):
        with self.lock:
            if app_id not in self.apps:
                raise ValueError('Unknown application')
            item = self.apps[app_id]
            attempt = self.probes.pop(app_id, None)
            if attempt:
                attempt.cancel()
            process = self.processes.get(app_id)
            group = item.get('processGroup') or item.get('pid')
            if group_active(group):
                owned = process is not None and process.pid == group
                if self.alive(item):
                    try:
                        owned = os.getpgid(item['pid']) == group
                    except ProcessLookupError:
                        pass  # The leader may exit between poll() and getpgid().
                if not owned and group_active(group):
                    raise ValueError('Process group ownership could not be confirmed')
                if owned:
                    signal_owned_group(group, signal.SIGTERM)
                deadline = time.monotonic() + 5
                while group_active(group) and time.monotonic() < deadline:
                    self.alive(item)  # Reap the leader; descendants may still be writing.
                    time.sleep(0.05)
                if group_active(group):
                    signal_owned_group(group, signal.SIGKILL)
                if process is not None:
                    process.wait(timeout=5)
                    item['exitCode'] = process.returncode
                deadline = time.monotonic() + 2
                while group_active(group) and time.monotonic() < deadline:
                    time.sleep(0.05)
                if group_active(group):
                    raise ValueError('Application descendants are still running; save is blocked')
            item.update(state='stopped', pid=None, processIdentity=None, processGroup=None, error=None)
            self.event(app_id, 'info', 'Application stopped; files and VM retained')
            self.save()
            return dict(item)

    def poll(self):
        # ponytail: short state transitions are serialized; per-app locks if many applications require them.
        with self.lock:
            changed = False
            for app_id, item in list(self.apps.items()):
                recipe = self.recipes.get(app_id)
                if not recipe or item['state'] in ('stopped', 'failed', 'unknown'):
                    continue
                process = self.processes.get(app_id)
                if not self.alive(item):
                    attempt = self.probes.pop(app_id, None)
                    if attempt:
                        attempt.cancel()
                    code = process.returncode if process is not None else None
                    if item['stage'] == 'prepare' and code == 0:
                        try:
                            prepared = self.root / (app_id + '.prepared.json')
                            launch = json.loads(prepared.read_text()) if not recipe.get('manifest') and prepared.exists() else None
                            self.spawn(recipe, 'application', launch)
                        except Exception:
                            item.update(state='failed', pid=None, error='Prepared application could not be started')
                            self.event(app_id, 'error', item['error'])
                    else:
                        success = recipe['kind'] == 'task' and item['stage'] == 'application' and code == 0
                        item.update(state='stopped' if success else 'failed' if code is not None else 'unknown', pid=None, exitCode=code)
                        item['error'] = None if success else 'Application exited; check its log'
                        self.event(app_id, 'info' if success else 'error', 'Task completed' if success else item['error'])
                    changed = True
                elif item['stage'] == 'application' and recipe['kind'] == 'service' and item['state'] == 'preparing':
                    check = recipe['readiness']
                    remaining = check.get('timeoutSeconds', 120) - (time.time() - item['startedAt'] / 1000)
                    attempt = self.probes.get(app_id)
                    if remaining <= 0:
                        if attempt:
                            attempt.cancel()
                            self.probes.pop(app_id, None)
                        ready = False
                    elif attempt and attempt.done.is_set():
                        ready = attempt.ready
                        self.probes.pop(app_id, None)
                        self.next_probe[app_id] = time.monotonic() + check.get('intervalSeconds', 2)
                    else:
                        if not attempt and time.monotonic() >= self.next_probe.get(app_id, 0):
                            self.probes[app_id] = ReadinessAttempt(recipe, check, self.environment(recipe), min(check.get('probeTimeoutSeconds', 5), remaining))
                        continue
                    if ready:
                        item.update(state='running', error=None)
                        self.event(app_id, 'info', 'Application readiness check passed: ' + check['type'])
                        changed = True
                    elif time.time() - item['startedAt'] / 1000 >= check.get('timeoutSeconds', 120):
                        item.update(state='failed', error='Application readiness timed out; process retained for inspection')
                        self.event(app_id, 'error', item['error'])
                        changed = True
                log = self.log_path(app_id)
                if log.exists() and log.stat().st_size > 8 * 1024 * 1024:
                    # ponytail: copy-truncate bounds debug logs; use a log service if lossless rotation is needed.
                    with log.open('r+b') as stream:
                        stream.seek(-2 * 1024 * 1024, os.SEEK_END)
                        recent = stream.read()
                        stream.seek(0)
                        stream.write(recent)
                        stream.truncate()
            if changed:
                self.save()

    def begin_sync(self):
        with self.lock:
            if self.synchronization['status'] == 'syncing':
                raise ValueError('Synchronization is already running')
            self.sealed = False
            self.synchronization = {'status': 'syncing', 'observedAt': int(time.time() * 1000)}
            atomic_json(self.root / 'synchronization.json', self.synchronization)
            threading.Thread(target=self.synchronize, daemon=True).start()
            return dict(self.synchronization)

    def synchronize(self):
        try:
            with self.lock:
                recipes = list(self.recipes.values())
                needs_drive = has_save_rules(recipes)
                if needs_drive and not os.path.ismount(DRIVE_MOUNT):
                    raise ValueError('Google Drive is not mounted')
                for app_id in list(self.apps):
                    if self.apps[app_id]['state'] == 'unknown':
                        raise ValueError('Application process state is unknown')
                    self.stop(app_id)
            records = saved_files(recipes)
            receipt_id = secrets.token_hex(8)
            backups = copy_saved_files(records, receipt_id)
            if needs_drive:
                verifier = DriveReader()
                for attempt in range(25):
                    if verifier.verify(records + backups):
                        break
                    if attempt == 24:
                        raise ValueError('Drive server contents did not match before the deadline')
                    time.sleep(5)
            if saved_files(recipes) != records:
                raise ValueError('Source changed during verification')
            result = {'status': 'verified', 'observedAt': int(time.time() * 1000), 'method': 'drive_api_md5' if needs_drive else 'no_save_rules',
                      'files': records, 'backups': backups, 'fileCount': len(records),
                      'totalBytes': sum(item['size'] for item in records), 'receiptId': receipt_id}
        except Exception:
            result = {'status': 'failed', 'observedAt': int(time.time() * 1000),
                      'error': '保存或 Drive 服务端校验失败；实例及本地文件保留，请检查挂载、目录和网络后重试。'}
        with self.lock:
            self.synchronization = result
            atomic_json(self.root / 'synchronization.json', result)
            self.event('', 'info' if result['status'] == 'verified' else 'error',
                       ('No save rules configured; applications stopped' if result.get('method') == 'no_save_rules' else 'Drive server file sizes and checksums verified; applications stopped') if result['status'] == 'verified' else result['error'])
            self.save()

    def release_guard(self, seal=True):
        with self.lock:
            receipt = self.synchronization
            recipes = list(self.recipes.values())
            needs_drive = has_save_rules(recipes)
            method = 'drive_api_md5' if needs_drive else 'no_save_rules'
            if receipt['status'] != 'verified' or receipt.get('method') != method or (needs_drive and not os.path.ismount(DRIVE_MOUNT)):
                raise ValueError('Configured save rules must be verified before release')
            if any(item['state'] != 'stopped' or self.alive(item) or group_active(item.get('processGroup')) for item in self.apps.values()):
                raise ValueError('Applications must be stopped before release')
            records = saved_files(recipes)
            if records != receipt['files'] or (needs_drive and not DriveReader().verify(records + receipt.get('backups', []))) or (not needs_drive and receipt.get('backups')):
                self.synchronization = {'status': 'failed', 'error': 'Files changed after verification; VM retained'}
                atomic_json(self.root / 'synchronization.json', self.synchronization)
                raise ValueError('Files changed after verification')
            if seal:
                self.sealed = True
            return {'allowed': True, 'receiptId': receipt['receiptId'], 'method': method, 'fileCount': len(records)}

    def snapshot(self):
        with self.lock:
            synchronization = {key: value for key, value in self.synchronization.items() if key not in ('files', 'backups')}
            synchronization['files'] = [{key: item[key] for key in ('applicationId', 'path', 'size', 'sha256', 'md5')} for item in self.synchronization.get('files', [])]
            return {'protocolVersion': 1, 'daemon': {'pid': os.getpid(), 'uptimeSeconds': int(time.time() - self.started), 'revision': SCRIPT_REVISION},
                    'heartbeat': self.heartbeat.snapshot(),
                    'applications': [{k: v for k, v in item.items() if k not in ('processIdentity', 'processGroup')} for item in self.apps.values()],
                    'events': list(self.events), 'synchronization': synchronization, 'instanceBilling': 'not_managed_here'}

    def logs(self, app_id):
        if not re.fullmatch(r'[a-zA-Z0-9][a-zA-Z0-9_-]{0,79}', app_id):
            raise ValueError('Invalid application ID')
        with self.lock:
            if app_id not in self.apps:
                return {'text': '', 'truncated': False, 'progress': None, 'state': 'unknown', 'stage': 'pending', 'observedAt': int(time.time() * 1000)}
            progress = None
            try:
                progress = json.loads((self.root / (app_id + '.preparation.json')).read_text())
            except (OSError, ValueError):
                pass
            with self.log_path(app_id).open('rb') as stream:
                stream.seek(0, os.SEEK_END)
                size = stream.tell()
                stream.seek(max(0, size - MAX_LOG))
                return {'text': stream.read(MAX_LOG).decode('utf-8', errors='replace'), 'truncated': size > MAX_LOG,
                        'progress': progress, 'state': self.apps[app_id]['state'], 'stage': self.apps[app_id]['stage'],
                        'error': self.apps[app_id].get('error'), 'observedAt': int(time.time() * 1000)}


def load_token(root):
    path = root / 'agent-token'
    try:
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    except FileExistsError:
        token = path.read_text().strip()
    else:
        token = secrets.token_urlsafe(32)
        with os.fdopen(fd, 'w') as stream:
            stream.write(token)
    if len(token) < 32 or any(c.isspace() for c in token):
        raise ValueError('Invalid local agent token')
    return token


def serve(root, port, enable_heartbeat=False):
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    lock_file = (root / 'agent.lock').open('a')
    fcntl.flock(lock_file, fcntl.LOCK_EX | fcntl.LOCK_NB)
    token = load_token(root)
    supervisor = Supervisor(root)
    supervisor.heartbeat = heartbeat_module.CloudHeartbeat(root, enabled=enable_heartbeat)
    supervisor.heartbeat.start()
    stopped = threading.Event()

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def reply(self, status, body):
            raw = json.dumps(body, ensure_ascii=False).encode()
            self.send_response(status)
            self.send_header('Content-Type', 'application/json; charset=utf-8')
            self.send_header('Content-Length', str(len(raw)))
            self.send_header('Cache-Control', 'no-store')
            self.end_headers()
            self.wfile.write(raw)

        def authorized(self):
            actual = self.headers.get('Authorization', '')
            return hmac.compare_digest(actual.encode(), ('Bearer ' + token).encode())

        def do_GET(self):
            if not self.authorized():
                self.reply(401, {'error': 'Unauthorized'}); return
            parsed = urllib.parse.urlsplit(self.path)
            try:
                if parsed.path in ('/v1/health', '/v1/state'):
                    self.reply(200, supervisor.snapshot())
                elif parsed.path == '/v1/logs':
                    query = urllib.parse.parse_qs(parsed.query)
                    self.reply(200, supervisor.logs(query.get('applicationId', [''])[0]))
                else:
                    self.reply(404, {'error': 'Not found'})
            except (ValueError, OSError):
                self.reply(400, {'error': 'Application or log unavailable'})

        def do_POST(self):
            if not self.authorized():
                self.reply(401, {'error': 'Unauthorized'}); return
            try:
                length = int(self.headers.get('Content-Length', '0'))
                if not 0 < length <= MAX_BODY:
                    self.reply(413, {'error': 'Invalid request size'}); return
                self.connection.settimeout(10)
                body = json.loads(self.rfile.read(length))
                if not isinstance(body, dict):
                    raise ValueError('Expected object')
                if self.path == '/v1/applications/start':
                    self.reply(202, supervisor.start(body.get('recipe'), body.get('files')))
                elif self.path == '/v1/applications/stop':
                    self.reply(200, supervisor.stop(str(body.get('applicationId', ''))))
                elif self.path == '/v1/synchronization/start':
                    self.reply(202, supervisor.begin_sync())
                elif self.path == '/v1/instance/release-check':
                    self.reply(200, supervisor.release_guard(seal=body.get('inspectOnly') is not True))
                else:
                    self.reply(404, {'error': 'Not found'})
            except (ValueError, OSError, subprocess.SubprocessError):
                self.reply(400, {'error': 'Application request failed; current processes and files are retained'})

    server = http.server.ThreadingHTTPServer(('127.0.0.1', port), Handler)
    def watch():
        while not stopped.wait(0.5):
            try:
                supervisor.poll()
            except Exception:
                with supervisor.lock:
                    supervisor.event('', 'error', 'Supervisor poll failed; VM retained')
    threading.Thread(target=watch, daemon=True).start()
    for sig in (signal.SIGTERM, signal.SIGINT):
        signal.signal(sig, lambda *_: threading.Thread(target=server.shutdown, daemon=True).start())
    print(json.dumps({'status': 'ready', 'port': port, 'pid': os.getpid()}), flush=True)
    try:
        server.serve_forever(poll_interval=0.25)
    finally:
        stopped.set()
        supervisor.heartbeat.stop()
        with supervisor.lock:
            supervisor.save()
        server.server_close()
        lock_file.close()
        # Child process groups and their append-only logs deliberately outlive this connection/service.


def daemon(root, port, enable_heartbeat=False):
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    token = load_token(root)
    def healthy():
        request = urllib.request.Request(f'http://127.0.0.1:{port}/v1/health', headers={'Authorization': 'Bearer ' + token})
        try:
            with urllib.request.urlopen(request, timeout=1) as response:
                state = json.load(response)
                return state if state.get('protocolVersion') == 1 else None
        except (OSError, ValueError):
            return None
    previous = healthy()
    if previous and previous.get('daemon', {}).get('revision') == SCRIPT_REVISION and previous.get('heartbeat', {}).get('enabled', False) == enable_heartbeat:
        print(json.dumps({'status': 'already_running', 'port': port})); return
    if previous:
        if previous.get('synchronization', {}).get('status') == 'syncing':
            raise RuntimeError('Synchronization is in progress; retry the daemon update afterwards')
        pid = previous['daemon']['pid']
        command = Path(f'/proc/{pid}/cmdline').read_bytes().split(b'\0')
        if str(Path(__file__).resolve()).encode() not in command or b'serve' not in command:
            raise RuntimeError('Existing daemon ownership could not be verified')
        identity = process_identity(pid)
        os.kill(pid, signal.SIGTERM)
        deadline = time.monotonic() + 10
        while identity and process_identity(pid) == identity and time.monotonic() < deadline:
            time.sleep(0.1)
        if identity and process_identity(pid) == identity:
            raise RuntimeError('Existing daemon has not stopped; application processes retained')
    with (root / 'agent.log').open('ab', buffering=0) as log:
        process = subprocess.Popen([sys.executable, '-I', '-B', str(Path(__file__).resolve()), 'serve', '--state-dir', str(root), '--port', str(port), *(['--heartbeat'] if enable_heartbeat else [])], stdin=subprocess.DEVNULL, stdout=log, stderr=log, start_new_session=True)
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline and process.poll() is None:
        if healthy():
            print(json.dumps({'status': 'ready', 'port': port, 'pid': process.pid})); return
        time.sleep(0.1)
    raise RuntimeError('Cloud agent did not become ready; inspect its local log')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=['serve', 'start'])
    parser.add_argument('--state-dir', type=Path, default=Path('/opt/cloud-deck/state'))
    parser.add_argument('--port', type=int, default=18775)
    parser.add_argument('--heartbeat', action='store_true', help='Maintain kernel activity independently on the Colab VM')
    args = parser.parse_args()
    if not 1 <= args.port <= 65535:
        parser.error('port must be between 1 and 65535')
    (daemon if args.mode == 'start' else serve)(args.state_dir, args.port, args.heartbeat)
