"""Offline lifecycle tests. All files live in the runner's scratch TMPDIR."""
import ast
import copy
import hashlib
import io
import json
import os
from pathlib import Path
import shlex
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

from test_cloud_agent import agent, bootstrap

lifecycle = agent.lifecycle


class Download(io.BytesIO):
    def geturl(self):
        return 'https://mirror.example/model.bin'


class LifecycleTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.drive = self.root / 'drive' / 'MyDrive'
        self.drive.mkdir(parents=True)
        self.patch_drive = patch.object(lifecycle, 'DRIVE_ROOT', self.drive)
        self.patch_drive.start()
        self.content = b'verified resource content'
        self.resource = {
            'target': str(self.root / 'local' / 'model.bin'),
            'size': len(self.content), 'sha256': hashlib.sha256(self.content).hexdigest(),
            'sources': [{'kind': 'download', 'url': 'https://mirror.example/model.bin'},
                        {'kind': 'drive', 'path': str(self.drive / 'model.bin')}],
        }
        (self.drive / 'model.bin').write_bytes(self.content)

    def tearDown(self):
        self.patch_drive.stop()
        self.temporary.cleanup()

    def manifest(self):
        return {'schemaVersion': 1, 'id': 'test-app', 'name': 'Test', 'description': '', 'kind': 'task',
                'runtime': {'command': 'echo application-started', 'workingDirectory': str(self.root),
                            'environment': {}, 'remotePort': None, 'localPort': None, 'healthPath': None},
                'resources': {'files': []},
                'startup': {'commands': [], 'restore': []}, 'shutdown': {'save': []}}

    def test_external_first_uses_verified_download(self):
        calls = []
        def opener(request, timeout):
            calls.append(request.full_url)
            return Download(self.content)
        result = lifecycle.stage_resource(self.resource, opener)
        self.assertEqual(result['source'], 'download')
        self.assertEqual(calls, ['https://mirror.example/model.bin'])
        self.assertEqual(Path(self.resource['target']).read_bytes(), self.content)

    def test_drive_first_does_not_request_download(self):
        self.resource['sources'].reverse()
        result = lifecycle.stage_resource(self.resource, lambda *args, **kwargs: self.fail('Unexpected network'))
        self.assertEqual(result['source'], 'drive')

    def test_download_error_and_corruption_fall_back(self):
        for response in (OSError('unavailable'), b'X' * len(self.content), self.content + b'extra'):
            with self.subTest(response=response):
                def opener(*args, **kwargs):
                    if isinstance(response, Exception):
                        raise response
                    return Download(response)
                result = lifecycle.stage_resource(self.resource, opener)
                self.assertEqual(result['source'], 'drive')
                Path(self.resource['target']).unlink()
                self.assertFalse(list((self.root / 'local').glob('.cloud-deck-resource-*')))

    def test_valid_local_file_reused_and_invalid_file_preserved_on_failure(self):
        target = Path(self.resource['target'])
        target.parent.mkdir()
        target.write_bytes(self.content)
        result = lifecycle.stage_resource(self.resource, lambda *args, **kwargs: self.fail('Unexpected network'))
        self.assertEqual(result['source'], 'local')
        target.write_bytes(b'existing invalid content')
        self.resource['sources'] = [{'kind': 'drive', 'path': str(self.drive / 'missing')}]
        with self.assertRaises(ValueError):
            lifecycle.stage_resource(self.resource)
        self.assertEqual(target.read_bytes(), b'existing invalid content')

    def test_restore_preserves_local_changes_and_excludes_backups(self):
        source = self.drive / 'user'
        source.mkdir()
        (source / 'workflow.json').write_text('old')
        (source / 'settings.json').write_text('restore me')
        (source / '.cloud-deck-backups').mkdir()
        (source / '.cloud-deck-backups' / 'backup.json').write_text('skip')
        target = self.root / 'user'
        target.mkdir()
        (target / 'workflow.json').write_text('local changes')
        self.assertEqual(lifecycle.restore_files({'source': str(source), 'target': str(target)}), 1)
        self.assertEqual((target / 'workflow.json').read_text(), 'local changes')
        self.assertEqual((target / 'settings.json').read_text(), 'restore me')
        self.assertFalse((target / '.cloud-deck-backups').exists())

    def test_missing_required_restore_and_symlinks_fail(self):
        with self.assertRaises(ValueError):
            lifecycle.restore_files({'source': str(self.drive / 'missing'), 'target': str(self.root / 'data'), 'optional': False})
        target = Path(self.resource['target'])
        target.parent.mkdir()
        target.symlink_to(self.drive / 'model.bin')
        with self.assertRaises(ValueError):
            lifecycle.stage_resource(self.resource)

    def test_preparation_runs_commands_in_order(self):
        manifest = self.manifest()
        marker = self.root / 'order'
        manifest['startup']['commands'] = [
            {'command': f'printf first > {shlex.quote(str(marker))}', 'workingDirectory': '/'},
            {'command': f'printf second >> {shlex.quote(str(marker))}', 'workingDirectory': '/'},
        ]
        lifecycle.prepare(manifest, self.root / 'manifest')
        self.assertEqual(marker.read_text(), 'firstsecond')

    def test_failed_preparation_never_starts_application(self):
        manifest = self.manifest()
        manifest['startup']['commands'] = [{'command': 'exit 7', 'workingDirectory': '/'}]
        supervisor = agent.Supervisor(self.root / 'state')
        supervisor.start({'manifest': manifest})
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline and supervisor.apps['test-app']['state'] != 'failed':
            supervisor.poll()
            time.sleep(0.02)
        self.assertEqual(supervisor.apps['test-app']['state'], 'failed')
        self.assertNotIn('application-started', supervisor.logs('test-app')['text'])
        self.assertFalse(any(event['message'] == 'Application process started' for event in supervisor.events))

    def test_explicit_save_targets_include_files_and_reject_collisions(self):
        source = self.root / 'settings.json'
        source.write_text('settings')
        manifest = self.manifest()
        manifest['shutdown']['save'] = [{'source': str(source), 'target': str(self.drive / 'custom' / 'renamed.json')}]
        with patch.object(agent, 'DRIVE_MOUNT', self.drive.parent):
            records = agent.saved_files([lifecycle.recipe_from_manifest(manifest)])
            self.assertEqual(records[0]['target'], str(self.drive / 'custom' / 'renamed.json'))
            with self.assertRaises(ValueError):
                agent.saved_files([lifecycle.recipe_from_manifest(manifest)] * 2)

    def test_malformed_manifest_is_rejected(self):
        changes = [('runtime', 'environment', []), ('runtime', 'usePreparedLaunch', 'false'),
                   ('startup', 'commands', {})]
        for section, field, value in changes:
            with self.subTest(field=field):
                manifest = self.manifest()
                manifest[section][field] = value
                with self.assertRaises(ValueError):
                    lifecycle.validate_manifest(manifest)

    def test_comfyui_script_retains_every_pinned_source(self):
        root = Path(__file__).parent
        baseline = json.loads((root / 'manifest.json').read_text())
        script = ast.parse((root / 'client/applications/comfyui/setup.py').read_text())
        calls = [node for node in ast.walk(script) if isinstance(node, ast.Call)]
        values = [[argument.value for argument in node.args if isinstance(argument, ast.Constant)] for node in calls]
        self.assertIn([baseline['comfyui']['repository'], baseline['comfyui']['commit']], values)
        plugin_calls = [node for node in calls if isinstance(node.func, ast.Attribute) and node.func.attr == 'submit']
        observed = []
        for node in plugin_calls:
            directory = ast.literal_eval(node.args[1].right)
            arguments = [ast.literal_eval(argument) for argument in node.args[2:]]
            observed.append([directory, *arguments])
        expected = [[plugin['directory'], plugin['repository'], plugin['commit']] if plugin['source'] == 'git'
                    else [plugin['directory'], plugin['download_url']] for plugin in baseline['plugins']]
        self.assertEqual(observed, expected)

    def test_empty_downloads_run_inline_commands_and_arbitrary_script(self):
        manifest = self.manifest()
        manifest['runtime']['environment'] = {'GREETING': 'hello command executor'}
        manifest['startup']['commands'] = [
            {'command': 'printf first > order', 'workingDirectory': str(self.root)},
            {'command': 'python3 "$CLOUD_DECK_APP_DIR/prepare.py"', 'workingDirectory': str(self.root)},
        ]
        manifest['runtime']['command'] = 'exec bash "$CLOUD_DECK_APP_DIR/start.sh"'
        files = {'prepare.py': 'from pathlib import Path\nPath("order").write_text(Path("order").read_text()+"second")\n',
                 'start.sh': 'test "$(cat order)" = firstsecond && printf "%s" "$GREETING" > result\n'}
        supervisor = agent.Supervisor(self.root / 'state')
        supervisor.start({'manifest': manifest}, files)
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline and supervisor.apps['test-app']['state'] not in ('stopped', 'failed'):
            supervisor.poll()
            time.sleep(0.02)
        self.assertEqual(supervisor.apps['test-app']['state'], 'stopped', supervisor.logs('test-app'))
        self.assertEqual((self.root / 'result').read_text(), 'hello command executor')
        # Restarting the daemon keeps the same script paths and command semantics.
        restarted = agent.Supervisor(self.root / 'state')
        self.assertEqual(restarted.recipes['test-app']['manifest'], manifest)
        self.assertEqual((self.root / 'state/packages/test-app/start.sh').read_text(), files['start.sh'])

    def test_package_traversal_cannot_write_or_start_processes(self):
        supervisor = agent.Supervisor(self.root / 'state')
        for filename in ('../escaped', '/absolute', 'nested/../../escaped', 'nested\\escaped'):
            with self.subTest(filename=filename), self.assertRaises(ValueError):
                supervisor.start({'manifest': self.manifest()}, {filename: 'echo unexpected'})
        self.assertEqual(supervisor.apps, {})
        self.assertFalse((self.root / 'state/packages').exists())

    def test_empty_save_rules_stop_tasks_and_allow_release_without_drive(self):
        manifest = self.manifest()
        manifest['runtime']['command'] = 'exec sleep 30'
        supervisor = agent.Supervisor(self.root / 'state')
        supervisor.start({'manifest': manifest})
        with patch.object(agent.os.path, 'ismount', return_value=False), patch.object(agent, 'DriveReader', side_effect=AssertionError('Unexpected Drive access')):
            supervisor.synchronize()
            self.assertEqual(supervisor.apps['test-app']['state'], 'stopped')
            self.assertEqual(supervisor.release_guard()['method'], 'no_save_rules')
            manifest['shutdown']['save'] = [{'source': str(self.root / 'data'), 'target': str(self.drive / 'data')}]
            supervisor.recipes['test-app'] = lifecycle.recipe_from_manifest(manifest)
            with self.assertRaises(ValueError):
                supervisor.release_guard()

    def test_adopting_existing_model_directory_preserves_files_and_conflicts(self):
        application = self.root / 'ComfyUI'
        models = application / 'models' / 'checkpoints'
        models.mkdir(parents=True)
        (models / 'put_checkpoints_here').touch()
        (models / 'custom.bin').write_bytes(b'user model')
        local = self.root / 'local-models'
        local.mkdir()
        with patch.object(bootstrap, 'APP_DIR', application), patch.object(bootstrap, 'LOCAL_MODELS', local), patch.dict(os.environ, CLOUD_DECK_LIFECYCLE_MANAGED='1'), patch.object(bootstrap, '_scan_drive_extra_models', return_value={}), patch.object(bootstrap, '_build_inventory_snapshot', return_value={}):
            bootstrap._stage_all_models_local([], self.root / 'cache')
        self.assertTrue((application / 'models').is_symlink())
        self.assertEqual((local / 'checkpoints/custom.bin').read_bytes(), b'user model')
        (application / 'models').unlink()
        models.mkdir(parents=True)
        (models / 'custom.bin').write_bytes(b'conflicting user model')
        with patch.object(bootstrap, 'APP_DIR', application), patch.object(bootstrap, 'LOCAL_MODELS', local), patch.dict(os.environ, CLOUD_DECK_LIFECYCLE_MANAGED='1'):
            with self.assertRaises(RuntimeError):
                bootstrap._stage_all_models_local([], self.root / 'cache')
        self.assertEqual((models / 'custom.bin').read_bytes(), b'conflicting user model')
        self.assertEqual((local / 'checkpoints/custom.bin').read_bytes(), b'user model')


if __name__ == '__main__':
    unittest.main()
