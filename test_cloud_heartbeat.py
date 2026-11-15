"""Offline cloud activity checks: no Google credentials or external requests."""
import importlib.util
import json
from pathlib import Path
import tempfile
import threading
import time
import types
import unittest
from unittest.mock import MagicMock, patch

spec = importlib.util.spec_from_file_location('cloud_heartbeat', Path(__file__).with_name('cloud_heartbeat.py'))
heartbeat = importlib.util.module_from_spec(spec)
spec.loader.exec_module(heartbeat)


class HeartbeatTests(unittest.TestCase):
    def test_worker_keeps_running_without_desktop_requests(self):
        completed = threading.Event()
        activity = MagicMock()
        def execute():
            if activity.execute.call_count >= 3:
                completed.set()
        activity.execute.side_effect = execute
        with tempfile.TemporaryDirectory() as temporary:
            worker = heartbeat.CloudHeartbeat(temporary, enabled=True, interval=0.02, activity=activity)
            worker.start()
            try:
                self.assertTrue(completed.wait(2), 'Cloud worker depended on a desktop poll')
            finally:
                worker.stop()
                worker.thread.join(timeout=2)
            self.assertGreaterEqual(worker.snapshot()['successCount'], 3)
            self.assertEqual(worker.snapshot()['owner'], 'cloud')
            self.assertFalse(worker.thread.is_alive())

    def test_failure_retains_last_confirmation_and_next_attempt_recovers(self):
        activity = MagicMock()
        activity.execute.side_effect = [None, TimeoutError('private request text'), None]
        with tempfile.TemporaryDirectory() as temporary:
            worker = heartbeat.CloudHeartbeat(temporary, enabled=True, activity=activity)
            worker.tick()
            success = worker.snapshot()['lastSuccessAt']
            worker.tick()
            failed = worker.snapshot()
            self.assertEqual(failed['status'], 'error')
            self.assertEqual(failed['lastSuccessAt'], success)
            self.assertEqual(failed['successCount'], 1)
            self.assertNotIn('private request text', json.dumps(failed))
            worker.tick()
            self.assertEqual(worker.snapshot()['successCount'], 2)
            record = Path(temporary) / 'heartbeat.json'
            self.assertEqual(json.loads(record.read_text())['status'], 'confirmed')
            self.assertEqual(record.stat().st_mode & 0o777, 0o600)

    def connection(self, status='ok'):
        connection = MagicMock()
        requests = []
        connection.send.side_effect = lambda text: requests.append(json.loads(text))
        responses = iter(['unrelated', 'matching'])
        def receive():
            parent = 'other-operation' if next(responses) == 'unrelated' else requests[0]['header']['msg_id']
            return json.dumps({'parent_header': {'msg_id': parent}, 'header': {'msg_type': 'execute_reply'},
                               'content': {'status': status}})
        connection.recv.side_effect = receive
        return connection, requests

    def test_only_own_kernel_reply_confirms_activity_without_oauth(self):
        connection, requests = self.connection()
        websocket = types.SimpleNamespace(create_connection=MagicMock(return_value=connection))
        activity = heartbeat.KernelActivity()
        activity.request = MagicMock(return_value=[{'path': 'cloud-deck-heartbeat.ipynb', 'kernel': {'id': 'existing-kernel'}}])
        with patch.dict('sys.modules', {'websocket': websocket}):
            activity.execute()
        self.assertEqual(connection.recv.call_count, 2)
        self.assertEqual(requests[0]['content']['code'], '1')
        self.assertFalse(requests[0]['content']['store_history'])
        address = websocket.create_connection.call_args.args[0]
        self.assertTrue(address.startswith('ws://127.0.0.1:8080/'))
        self.assertNotIn('token', address)
        connection.close.assert_called_once()

    def test_failed_kernel_is_rediscovered_instead_of_claimed_alive(self):
        connection, requests = self.connection('error')
        activity = heartbeat.KernelActivity()
        activity.kernel = 'dead-kernel'
        with patch.dict('sys.modules', {'websocket': types.SimpleNamespace(create_connection=lambda *arguments, **options: connection)}):
            with self.assertRaises(RuntimeError):
                activity.execute()
        self.assertIsNone(activity.kernel)
        connection.close.assert_called_once()

    def test_missing_matching_reply_times_out(self):
        connection = MagicMock()
        connection.recv.return_value = json.dumps({'parent_header': {'msg_id': 'other'}})
        activity = heartbeat.KernelActivity()
        activity.kernel = 'test-kernel'
        with patch.dict('sys.modules', {'websocket': types.SimpleNamespace(create_connection=lambda *arguments, **options: connection)}), patch.object(heartbeat.time, 'monotonic', side_effect=[0, 0, 0, 16]):
            with self.assertRaises(TimeoutError):
                activity.execute()
        self.assertIsNone(activity.kernel)


if __name__ == '__main__':
    unittest.main()
