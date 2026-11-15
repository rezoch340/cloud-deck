"""Maintain Colab kernel activity from the VM, without desktop OAuth credentials.

Colab retired the client /keep-alive endpoint on 2026-09-25. This worker talks
only to the VM's Jupyter loopback service; a reply confirms kernel activity,
not a promise about platform lifetime or billing.
"""
import json
import os
from pathlib import Path
import threading
import time
import urllib.parse
import urllib.request
import uuid


class KernelActivity:
    def __init__(self):
        self.kernel = None
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def request(self, path, body=None):
        request = urllib.request.Request('http://127.0.0.1:8080' + path,
                                         data=json.dumps(body).encode() if body is not None else None,
                                         headers={'Content-Type': 'application/json'})
        with self.opener.open(request, timeout=5) as response:
            return json.load(response)

    def execute(self):
        # websocket-client is included in the Colab runtime, not required locally.
        import websocket
        try:
            if self.kernel is None:
                sessions = self.request('/api/sessions')
                existing = next((item for item in sessions if item.get('path') == 'cloud-deck-heartbeat.ipynb'), None)
                session = existing or self.request('/api/sessions', {
                    'path': 'cloud-deck-heartbeat.ipynb', 'name': 'cloud-deck-heartbeat.ipynb',
                    'type': 'notebook', 'kernel': {'name': 'python3'}})
                self.kernel = session['kernel']['id']
            identity = uuid.uuid4().hex
            kernel = urllib.parse.quote(self.kernel, safe='')
            address = f'ws://127.0.0.1:8080/api/kernels/{kernel}/channels?session_id={identity}'
            connection = websocket.create_connection(address, timeout=5, suppress_origin=True,
                                                       http_no_proxy=['127.0.0.1', 'localhost'])
            try:
                connection.send(json.dumps({
                    'header': {'msg_id': identity, 'username': 'cloud-deck', 'session': identity,
                               'msg_type': 'execute_request', 'version': '5.3'},
                    'parent_header': {}, 'metadata': {}, 'channel': 'shell',
                    'content': {'code': '1', 'silent': True, 'store_history': False,
                                'user_expressions': {}, 'allow_stdin': False, 'stop_on_error': True}}))
                deadline = time.monotonic() + 15
                while time.monotonic() < deadline:
                    connection.settimeout(max(0.1, deadline - time.monotonic()))
                    value = json.loads(connection.recv())
                    if value.get('parent_header', {}).get('msg_id') != identity:
                        continue
                    if value.get('msg_type', value.get('header', {}).get('msg_type')) == 'execute_reply':
                        if value.get('content', {}).get('status') != 'ok':
                            raise RuntimeError('Kernel activity failed')
                        return
                raise TimeoutError('Kernel activity was not acknowledged')
            finally:
                connection.close()
        except Exception:
            self.kernel = None  # Recover from a dead/replaced kernel on the next attempt.
            raise


class CloudHeartbeat:
    def __init__(self, root, enabled=False, interval=60, activity=None):
        self.path = Path(root) / 'heartbeat.json'
        self.interval = interval
        self.activity = activity or KernelActivity()
        self.lock = threading.Lock()
        self.stopped = threading.Event()
        self.thread = None
        self.state = {'enabled': enabled, 'owner': 'cloud', 'status': 'waiting' if enabled else 'inactive',
                      'intervalSeconds': interval, 'successCount': 0, 'lastSuccessAt': None,
                      'daemonPid': os.getpid(), 'kernel': 'unobserved', 'error': None}

    def snapshot(self):
        with self.lock:
            return dict(self.state)

    def tick(self):
        with self.lock:
            self.state['lastAttemptAt'] = int(time.time() * 1000)
        try:
            self.activity.execute()
            update = {'status': 'confirmed', 'kernel': 'confirmed', 'error': None,
                      'lastSuccessAt': int(time.time() * 1000)}
        except Exception as error:
            # Never persist headers, tokens or a library's full request representation.
            update = {'status': 'error', 'kernel': 'failed',
                      'error': '云端 kernel 活动未确认（' + type(error).__name__ + '），将在下一轮重试。'}
        with self.lock:
            self.state.update(update)
            if update['status'] == 'confirmed':
                self.state['successCount'] += 1
            value = dict(self.state)
        temporary = self.path.with_suffix('.tmp')
        descriptor = os.open(temporary, os.O_CREAT | os.O_TRUNC | os.O_WRONLY, 0o600)
        with os.fdopen(descriptor, 'w') as stream:
            json.dump(value, stream, ensure_ascii=False)
        os.replace(temporary, self.path)

    def start(self):
        if not self.state['enabled']:
            return
        def run():
            while not self.stopped.is_set():
                try:
                    self.tick()
                except OSError:
                    # A state-file write failure must not kill the cloud activity worker.
                    pass
                if self.stopped.wait(self.interval):
                    break
        self.thread = threading.Thread(target=run, name='cloud-kernel-activity', daemon=True)
        self.thread.start()

    def stop(self):
        self.stopped.set()
