"""Offline checks for the bundled cloud supervisor and managed ComfyUI preparation.

Run with TMPDIR on the external drive. These tests never contact Colab, install
packages, download models, mount Drive or run the real ComfyUI application.
"""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shlex
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import urllib.error
import urllib.request
from unittest.mock import patch, MagicMock

ROOT = Path(__file__).resolve().parent

def load_module(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module

agent = load_module('cloud_agent')
bootstrap = load_module('cloud_bootstrap')


class SupervisorTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.supervisor = agent.Supervisor(self.root / 'state')

    def tearDown(self):
        for app_id in list(self.supervisor.apps):
            with contextlib.suppress(Exception):
                self.supervisor.stop(app_id)
        self.temporary.cleanup()

    def recipe(self, app_id='task', code='print("completed", flush=True)'):
        return {'id': app_id, 'name': app_id, 'kind': 'task', 'command': shlex.join([sys.executable, '-I', '-B', '-u', '-c', code]), 'workingDirectory': str(self.root)}

    def until(self, predicate):
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            self.supervisor.poll()
            if predicate():
                return
            time.sleep(0.02)
        self.fail('Local supervisor transition did not finish')

    def test_task_result_and_logs_are_retained(self):
        self.supervisor.start(self.recipe())
        self.until(lambda: self.supervisor.apps['task']['state'] == 'stopped')
        self.assertEqual(self.supervisor.apps['task']['exitCode'], 0)
        self.assertIn('completed', self.supervisor.logs('task')['text'])
        self.assertEqual(self.supervisor.snapshot()['synchronization']['status'], 'pending')
        self.assertEqual(self.supervisor.snapshot()['instanceBilling'], 'not_managed_here')
        restored = agent.Supervisor(self.root / 'state')
        self.assertTrue(restored.events)
        self.assertEqual(restored.apps['task']['state'], 'stopped')

    def test_any_manifest_reports_command_progress_output_and_failure(self):
        manifest = {'schemaVersion': 1, 'id': 'ordinary-script', 'name': 'Generic script', 'kind': 'task',
                    'runtime': {'command': 'printf should-not-run', 'workingDirectory': str(self.root), 'environment': {}},
                    'resources': {'files': []}, 'startup': {'restore': [], 'commands': [
                        {'command': 'printf "first output\\n"; sleep 0.5; printf "last output\\n"; exit 7',
                         'workingDirectory': str(self.root)}]}, 'shutdown': {'save': []}}
        self.supervisor.start({'manifest': manifest})
        self.until(lambda: 'first output' in self.supervisor.logs('ordinary-script')['text'])
        active = self.supervisor.logs('ordinary-script')
        self.assertEqual(active['state'], 'preparing')
        self.assertEqual(active['progress']['stage'], 'command')
        self.assertEqual(active['progress']['step'], 1)
        self.until(lambda: self.supervisor.apps['ordinary-script']['state'] == 'failed')
        result = self.supervisor.logs('ordinary-script')
        self.assertIn('last output', result['text'])
        self.assertNotIn('should-not-run', result['text'])
        self.assertEqual(result['progress']['exitCode'], 7)
        restored = agent.Supervisor(self.root / 'state')
        self.assertIn('last output', restored.logs('ordinary-script')['text'])

    def test_failed_preparation_never_launches_application(self):
        recipe = self.recipe('failed', 'from pathlib import Path; Path("should-not-exist").write_text("bad")')
        recipe['prepareCommand'] = shlex.join([sys.executable, '-I', '-B', '-c', 'raise SystemExit(7)'])
        self.supervisor.start(recipe)
        self.until(lambda: self.supervisor.apps['failed']['state'] == 'failed')
        self.assertFalse((self.root / 'should-not-exist').exists())
        self.assertEqual(self.supervisor.apps['failed']['exitCode'], 7)
        errors = [event for event in self.supervisor.events if event['level'] == 'error']
        self.supervisor.poll()
        self.assertEqual(errors, [event for event in self.supervisor.events if event['level'] == 'error'])

    def test_prepared_arguments_are_used_and_child_survives_no_client_requests(self):
        recipe = self.recipe('prepared', 'raise SystemExit(9)')
        prepare = self.root / 'prepare.py'
        prepare.write_text('import json, os, sys\nfrom pathlib import Path\nPath(os.environ["CLOUD_DECK_PREPARED_FILE"]).write_text(json.dumps({"argv": [sys.executable, "-I", "-B", "-u", "-c", "import time; print(123, flush=True); time.sleep(30)"]}))\n')
        recipe['prepareCommand'] = shlex.join([sys.executable, '-I', '-B', str(prepare)])
        self.supervisor.start(recipe)
        self.until(lambda: self.supervisor.apps['prepared']['stage'] == 'application')
        self.assertTrue(self.supervisor.alive(self.supervisor.apps['prepared']))
        self.until(lambda: '123' in self.supervisor.logs('prepared')['text'])
        self.assertEqual(self.supervisor.apps['prepared']['state'], 'running')
        stopped = self.supervisor.stop('prepared')
        self.assertEqual(stopped['state'], 'stopped')
        self.assertTrue(prepare.exists())
        self.assertTrue(self.supervisor.log_path('prepared').exists())

    @unittest.skipUnless(sys.platform.startswith('linux'), 'Linux process group observation')
    def test_stop_waits_for_descendants_that_ignore_sigterm(self):
        child = 'import signal,time; signal.signal(signal.SIGTERM,signal.SIG_IGN); print("child-ready",flush=True); time.sleep(30)'
        code = f'import subprocess,sys,time; subprocess.Popen([sys.executable,"-I","-B","-u","-c",{child!r}]); time.sleep(30)'
        self.supervisor.start(self.recipe('tree', code))
        group = self.supervisor.apps['tree']['processGroup']
        self.until(lambda: 'child-ready' in self.supervisor.logs('tree')['text'])
        self.supervisor.stop('tree')
        self.assertFalse(agent.group_active(group))

    def test_stop_accepts_a_process_exiting_during_inspection_or_signaling(self):
        signal_group = os.killpg
        for operation, failure in (('getpgid', ProcessLookupError), ('killpg', ProcessLookupError), ('killpg', PermissionError)):
            with self.subTest(operation=operation, failure=failure):
                self.supervisor.start(self.recipe(operation, 'import time; print("ready", flush=True); time.sleep(30)'))
                self.until(lambda: 'ready' in self.supervisor.logs(operation)['text'])
                process = self.supervisor.processes[operation]

                def exit_before_operation(*arguments):
                    signal_group(process.pid, signal.SIGTERM)
                    process.wait(timeout=5)
                    raise failure('Process exited before the operation')

                with patch.object(agent.os, operation, side_effect=exit_before_operation):
                    stopped = self.supervisor.stop(operation)
                self.assertEqual(stopped['state'], 'stopped')
                self.assertFalse(agent.group_active(process.pid))

    def test_stop_keeps_permission_errors_and_does_not_mark_live_tasks_stopped(self):
        self.supervisor.start(self.recipe('denied', 'import time; print("ready", flush=True); time.sleep(30)'))
        self.until(lambda: 'ready' in self.supervisor.logs('denied')['text'])
        with patch.object(agent.os, 'killpg', side_effect=PermissionError('Denied')):
            with self.assertRaises(PermissionError):
                self.supervisor.stop('denied')
        self.assertTrue(self.supervisor.alive(self.supervisor.apps['denied']))
        self.assertNotEqual(self.supervisor.apps['denied']['state'], 'stopped')

    def test_service_requires_health_success_and_does_not_steal_occupied_port(self):
        with socket.socket() as probe:
            probe.bind(('127.0.0.1', 0))
            occupied_port = probe.getsockname()[1]
            recipe = {**self.recipe('web', 'import time; time.sleep(30)'), 'kind': 'service', 'remotePort': occupied_port, 'healthPath': '/health'}
            with self.assertRaises(ValueError):
                self.supervisor.start(recipe)
        recipe['remotePort'] = occupied_port
        self.supervisor.start(recipe)
        self.assertEqual(self.supervisor.apps['web']['state'], 'preparing')
        response = MagicMock()
        response.__enter__.return_value.status = 200
        response.__enter__.return_value.url = f'http://127.0.0.1:{occupied_port}/health'
        with patch.object(agent, 'probe_readiness', return_value=True):
            self.until(lambda: self.supervisor.apps['web']['state'] == 'running')
        self.assertEqual(self.supervisor.apps['web']['state'], 'running')
        self.assertTrue(self.supervisor.alive(self.supervisor.apps['web']))

    def test_readiness_http_status_and_tcp_connection_are_observed(self):
        class Handler(agent.http.server.BaseHTTPRequestHandler):
            status = 503
            def do_GET(self):
                self.send_response(self.status)
                self.end_headers()
            def log_message(self, *arguments):
                pass
        server = agent.http.server.HTTPServer(('127.0.0.1', 0), Handler)
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        port = server.server_port
        check = {'type': 'http', 'url': f'http://127.0.0.1:{port}/ready', 'status': 204}
        try:
            self.assertFalse(agent.probe_readiness(self.recipe(), check, os.environ, 0.5))
            Handler.status = 204
            self.assertTrue(agent.probe_readiness(self.recipe(), check, os.environ, 0.5))
            self.assertTrue(agent.probe_readiness(self.recipe(), {'type': 'tcp', 'host': '127.0.0.1', 'port': port}, os.environ, 0.5))
        finally:
            server.shutdown(); server.server_close(); worker.join(timeout=2)
        self.assertFalse(agent.probe_readiness(self.recipe(), {'type': 'tcp', 'host': '127.0.0.1', 'port': port}, os.environ, 0.5))

    def test_script_readiness_uses_package_environment_and_waits_for_success(self):
        marker = self.root / 'ready'
        manifest = {'schemaVersion': 1, 'id': 'script-service', 'name': 'Script service', 'kind': 'service',
                    'runtime': {'command': 'exec sleep 30', 'workingDirectory': str(self.root),
                                'environment': {'READY_MARKER': str(marker)}, 'remotePort': None, 'localPort': None,
                                'readiness': {'type': 'command', 'command': 'bash "$CLOUD_DECK_APP_DIR/ready.sh"', 'intervalSeconds': 1}},
                    'resources': {'files': []}, 'startup': {'commands': [], 'restore': []}, 'shutdown': {'save': []}}
        self.supervisor.start({'manifest': manifest}, {'ready.sh': 'test -f "$READY_MARKER"\n'})
        self.until(lambda: self.supervisor.apps['script-service']['stage'] == 'application')
        self.until(lambda: 'script-service' in self.supervisor.next_probe)
        self.assertEqual(self.supervisor.apps['script-service']['state'], 'preparing')
        marker.touch()
        self.until(lambda: self.supervisor.apps['script-service']['state'] == 'running')

    def test_readiness_timeout_marks_failure_and_retains_main_process(self):
        recipe = {**self.recipe('unready', 'import time; time.sleep(30)'), 'kind': 'service',
                  'readiness': {'type': 'command', 'command': 'exit 1', 'timeoutSeconds': 1}}
        self.supervisor.start(recipe)
        self.until(lambda: self.supervisor.apps['unready']['state'] == 'failed')
        self.assertTrue(self.supervisor.alive(self.supervisor.apps['unready']))
        self.assertIn('readiness timed out', self.supervisor.apps['unready']['error'])
        stopped = self.supervisor.stop('unready')
        self.assertEqual(stopped['state'], 'stopped')
        self.assertIsNone(stopped['error'])
        self.assertTrue(any('readiness timed out' in event['message'] for event in self.supervisor.events))
        restarted = self.supervisor.start(recipe)
        self.assertEqual(restarted['state'], 'preparing')
        self.assertIsNone(restarted['error'])

    def test_stopping_app_cancels_slow_probe_without_blocking_other_apps(self):
        marker = self.root / 'probe.pid'
        probe = f'import os,time; from pathlib import Path; Path({str(marker)!r}).write_text(str(os.getpid())); time.sleep(30)'
        recipe = {**self.recipe('slow-probe', 'import time; time.sleep(30)'), 'kind': 'service',
                  'readiness': {'type': 'command', 'command': 'exec ' + shlex.join([sys.executable, '-I', '-B', '-c', probe]), 'probeTimeoutSeconds': 30}}
        self.supervisor.start(recipe)
        self.until(marker.exists)
        process_id = int(marker.read_text())
        started = time.monotonic()
        self.supervisor.start(self.recipe('parallel'))
        self.assertLess(time.monotonic() - started, 1)
        self.supervisor.stop('slow-probe')
        self.assertFalse(agent.group_active(process_id))

    def test_invalid_recipes_cannot_escape_state_directory(self):
        with self.assertRaises(ValueError):
            self.supervisor.start(self.recipe('../escape'))
        with self.assertRaises(ValueError):
            self.supervisor.start({**self.recipe(), 'workingDirectory': '/workspace/../etc'})
        with self.assertRaises(ValueError):
            self.supervisor.start({**self.recipe(), 'environment': {1: 'invalid'}})
        with self.assertRaises(ValueError):
            self.supervisor.logs('../../other')

    def test_daemon_detaches_and_authenticates_its_local_api(self):
        with socket.socket() as reserved:
            reserved.bind(('127.0.0.1', 0))
            port = reserved.getsockname()[1]
        root = self.root / 'daemon'
        started = subprocess.run([sys.executable, '-I', '-B', str(ROOT / 'cloud_agent.py'), 'start', '--state-dir', str(root), '--port', str(port)], capture_output=True, text=True, timeout=15, check=True)
        pid = json.loads(started.stdout)['pid']
        try:
            address = f'http://127.0.0.1:{port}/v1/state'
            with self.assertRaises(urllib.error.HTTPError) as denied:
                urllib.request.urlopen(address, timeout=2)
            self.assertEqual(denied.exception.code, 401)
            token = (root / 'agent-token').read_text().strip()
            request = urllib.request.Request(address, headers={'Authorization': 'Bearer ' + token})
            with urllib.request.urlopen(request, timeout=2) as response:
                state = json.load(response)
            self.assertEqual(state['protocolVersion'], 1)
            self.assertEqual(state['daemon']['pid'], pid)
            self.assertEqual(state['synchronization']['status'], 'unobserved')
        finally:
            os.kill(pid, signal.SIGTERM)
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline:
                try:
                    with socket.create_connection(('127.0.0.1', port), timeout=0.1):
                        pass
                except OSError:
                    break
                time.sleep(0.03)

    def test_save_checks_server_contents_and_blocks_release_on_changes_or_failure(self):
        source = self.root / 'output'; source.mkdir()
        (source / 'image.png').write_bytes(b'new image')
        drive = self.root / 'drive'
        storage = drive / 'MyDrive/app'
        target = storage / 'output/image.png'
        target.parent.mkdir(parents=True); target.write_bytes(b'old image')
        recipe = {**self.recipe('saved'), 'saveDirectories': [str(source)], 'storageDirectory': str(storage)}
        self.supervisor.start(recipe)
        with patch.object(agent, 'DRIVE_MOUNT', drive), patch.object(agent.os.path, 'ismount', return_value=True), patch.object(agent, 'DriveReader') as reader:
            reader.return_value.verify.return_value = True
            self.supervisor.begin_sync()
            self.until(lambda: self.supervisor.synchronization['status'] != 'syncing')
            self.assertEqual(self.supervisor.synchronization['status'], 'verified')
            self.assertFalse(self.supervisor.alive(self.supervisor.apps['saved']))
            self.assertEqual(target.read_bytes(), b'new image')
            backup = self.supervisor.synchronization['backups'][0]
            self.assertEqual(Path(backup['target']).read_bytes(), b'old image')
            self.assertEqual(reader.return_value.verify.call_args.args[0][-1]['md5'], backup['md5'])
            self.assertTrue(self.supervisor.release_guard()['allowed'])
            with self.assertRaises(ValueError):
                self.supervisor.start(recipe)
            (source / 'image.png').write_bytes(b'changed after verification')
            with self.assertRaises(ValueError):
                self.supervisor.release_guard()
            self.assertEqual(self.supervisor.synchronization['status'], 'failed')
            reader.side_effect = OSError('Drive API unavailable')
            self.supervisor.begin_sync()
            self.until(lambda: self.supervisor.synchronization['status'] != 'syncing')
            self.assertEqual(self.supervisor.synchronization['status'], 'failed')
            with self.assertRaises(ValueError):
                self.supervisor.release_guard()
            self.assertTrue((source / 'image.png').exists())

    def test_save_rejects_missing_sources_and_symlinks(self):
        source = self.root / 'output'; source.mkdir()
        recipe = {**self.recipe(), 'saveDirectories': [str(source)], 'storageDirectory': '/content/drive/MyDrive/app'}
        (source / 'link').symlink_to(self.root / 'outside')
        with self.assertRaises(ValueError):
            agent.saved_files([recipe])
        (source / 'link').unlink(); source.rmdir()
        with self.assertRaises(ValueError):
            agent.saved_files([recipe])

    def test_drive_verifier_requires_unique_server_file_and_matching_checksum(self):
        reader = agent.DriveReader.__new__(agent.DriveReader)
        entries = {'root': [{'id': 'folder', 'name': 'app', 'mimeType': 'application/vnd.google-apps.folder'}],
                   'folder': [{'id': 'file', 'name': 'result.json', 'mimeType': 'application/json', 'size': '4', 'md5Checksum': 'correct'}]}
        records = [{'target': '/content/drive/MyDrive/app/result.json', 'size': 4, 'md5': 'correct'}]
        with patch.object(reader, 'children', side_effect=lambda parent: entries[parent]):
            self.assertTrue(reader.verify(records))
            entries['folder'][0]['md5Checksum'] = 'different'
            self.assertFalse(reader.verify(records))
            entries['folder'][0]['md5Checksum'] = 'correct'
            entries['folder'].append(dict(entries['folder'][0]))
            self.assertFalse(reader.verify(records))

    def test_agent_token_is_private_and_is_not_rotated_on_reopen(self):
        state = self.root / 'token-test'
        state.mkdir()
        token = agent.load_token(state)
        self.assertGreaterEqual(len(token), 32)
        self.assertEqual(token, agent.load_token(state))
        self.assertEqual((state / 'agent-token').stat().st_mode & 0o077, 0)


class PreparationTests(unittest.TestCase):
    def test_managed_start_requires_a_real_drive_mount(self):
        with patch.object(bootstrap.os.path, 'ismount', return_value=False):
            with self.assertRaisesRegex(RuntimeError, 'not mounted'):
                bootstrap._ensure_drive_dirs(require_mount=True)

    def test_pinned_launch_preserves_cuda_order_and_optional_sage(self):
        with patch.object(bootstrap.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0)):
            launch = bootstrap._comfy_launch_spec(Path('/opt/comfy-venv/bin/python'))
        self.assertIn('--use-sage-attention', launch['argv'])
        self.assertIn('--preview-method', launch['argv'])
        self.assertTrue(launch['environment']['LD_LIBRARY_PATH'].startswith('/usr/lib64-nvidia:/usr/local/cuda/lib64'))
        with patch.object(bootstrap.subprocess, 'run', return_value=subprocess.CompletedProcess([], 1)):
            self.assertNotIn('--use-sage-attention', bootstrap._comfy_launch_spec(Path('/opt/comfy-venv/bin/python'))['argv'])

    def test_managed_restart_preserves_local_workflow_changes(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            app, drive, local = root / 'app', root / 'drive', root / 'local'
            app.mkdir(); (drive / 'user').mkdir(parents=True); (local / 'user').mkdir(parents=True)
            (drive / 'user/workflow.json').write_text('old cloud version')
            (drive / 'user/.cloud-deck-backups/old').mkdir(parents=True)
            (drive / 'user/.cloud-deck-backups/old/workflow.json').write_text('old backup')
            (local / 'user/workflow.json').write_text('new local work')
            with patch.object(bootstrap, 'APP_DIR', app), patch.object(bootstrap, 'DRIVE_ROOT', drive), patch.object(bootstrap, 'LOCAL_DATA', local):
                bootstrap._stage_local_userdata(preserve_local=True)
            self.assertEqual((local / 'user/workflow.json').read_text(), 'new local work')
            self.assertFalse((local / 'user/.cloud-deck-backups').exists())
            self.assertEqual((app / 'user').resolve(), local / 'user')

    def test_managed_prepare_requires_models_before_launch_and_never_starts_services(self):
        for download_fails in (False, True):
            with self.subTest(download_fails=download_fails), tempfile.TemporaryDirectory() as name:
                root = Path(name)
                app, venv = root / 'app', root / 'venv'
                app.mkdir(); (venv / 'bin').mkdir(parents=True); (venv / 'bin/python').touch()
                manifest = root / 'manifest.json'
                manifest.write_text(json.dumps({'bundle_id': 'test', 'models': []}))
                prepared = root / 'prepared.json'
                with contextlib.ExitStack() as stack:
                    stack.enter_context(patch.dict(os.environ, {'CLOUD_DECK_PREPARE_ONLY': '1', 'CLOUD_DECK_PREPARED_FILE': str(prepared)}))
                    stack.enter_context(patch.object(sys, 'argv', ['cloud_bootstrap.py', str(manifest)]))
                    for key, value in [('APP_DIR', app), ('VENV_DIR', venv), ('DRIVE_ROOT', root / 'drive')]:
                        stack.enter_context(patch.object(bootstrap, key, value))
                    for key in ('_ensure_drive_dirs', '_stage_local_userdata', '_write_status'):
                        stack.enter_context(patch.object(bootstrap, key))
                    stack.enter_context(patch.object(bootstrap, '_is_comfy_listening', return_value=None))
                    stack.enter_context(patch.object(bootstrap, '_scan_drive_extra_models', return_value={}))
                    launch = stack.enter_context(patch.object(bootstrap, '_comfy_launch_spec', return_value={'argv': ['python', 'main.py'], 'environment': {}}))

                    def stage_models(*_):
                        self.assertFalse(prepared.exists())
                        launch.assert_not_called()
                        if download_fails:
                            raise RuntimeError('Model download failed')
                        return [], [], {}

                    models = stack.enter_context(patch.object(bootstrap, '_stage_all_models_local', side_effect=stage_models))
                    sync = stack.enter_context(patch.object(bootstrap, '_install_and_start_sync_daemon'))
                    popen = stack.enter_context(patch.object(bootstrap.subprocess, 'Popen'))
                    run = stack.enter_context(patch.object(bootstrap.subprocess, 'run'))
                    with contextlib.redirect_stdout(io.StringIO()):
                        if download_fails:
                            with self.assertRaisesRegex(RuntimeError, 'Model download failed'):
                                bootstrap.main()
                            launch.assert_not_called()
                            self.assertFalse(prepared.exists())
                        else:
                            bootstrap.main()
                            launch.assert_called_once()
                            self.assertEqual(json.loads(prepared.read_text())['argv'], ['python', 'main.py'])
                    models.assert_called_once()
                    sync.assert_not_called(); popen.assert_not_called(); run.assert_not_called()


if __name__ == '__main__':
    unittest.main()
