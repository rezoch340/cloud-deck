#!/Library/Frameworks/Python.framework/Versions/3.12/bin/python3 -u
"""Notebook-free Google Colab Linux GPU controller + Magpie MCP server.

ponytail: single script serves both CLI (`colabctl status|up|ssh|tunnel|down`)
and Magpie HTTP/stdio MCP (`--http 127.0.0.1:18425`), using official Consumer OAuth
+ native protected `/colab/ssh` WebSocket tunnel + Google Drive tar.zst persistence.
"""
import argparse
import asyncio
import datetime as dt
import http.server
import inspect
import json
import os
import pathlib
import shlex
import signal
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid

import websockets

_WS_HDR_KW = (
    "additional_headers"
    if "additional_headers" in inspect.signature(websockets.connect).parameters
    else "extra_headers"
)

COLAB_DOMAIN = "https://colab.research.google.com"
BASE_DIR = pathlib.Path(__file__).resolve().parent
TMP_DIR = pathlib.Path("/Volumes/VoltDrive/AI/tmp/comfyui-colab")
AUTH_DIR = BASE_DIR / "auth"
TOKEN_FILE = AUTH_DIR / "google-oauth-token.json"
SSH_KEY_FILE = AUTH_DIR / "colab_ed25519"
SSH_PUB_FILE = AUTH_DIR / "colab_ed25519.pub"
MANIFEST_FILE = BASE_DIR / "manifest.json"
BOOTSTRAP_FILE = BASE_DIR / "cloud_bootstrap.py"
TUNNEL_PID_FILE = TMP_DIR / "ssh-tunnel.pid"
TUNNEL_EP_FILE = TMP_DIR / "ssh-tunnel.endpoint"
KEEPALIVE_PID_FILE = TMP_DIR / "keepalive.pid"
LIVE_STATE_FILE = TMP_DIR / "live-state.json"
INVENTORY_CACHE_FILE = TMP_DIR / "inventory-cache.json"

_GCP_REGION_CN = {
    "asia-east1": "台湾",
    "asia-east2": "香港",
    "asia-northeast1": "日本东京",
    "asia-northeast2": "日本大阪",
    "asia-northeast3": "韩国首尔",
    "asia-southeast1": "新加坡",
    "asia-southeast2": "印尼雅加达",
    "asia-south1": "印度孟买",
    "asia-south2": "印度德里",
    "us-central1": "美国爱荷华",
    "us-east1": "美国南卡",
    "us-east4": "美国弗吉尼亚",
    "us-east5": "美国俄亥俄",
    "us-south1": "美国达拉斯",
    "us-west1": "美国俄勒冈",
    "us-west2": "美国洛杉矶",
    "us-west3": "美国盐湖城",
    "us-west4": "美国内华达",
    "europe-west1": "比利时",
    "europe-west2": "英国伦敦",
    "europe-west3": "德国法兰克福",
    "europe-west4": "荷兰",
    "europe-west6": "瑞士苏黎世",
    "europe-west9": "法国巴黎",
    "europe-north1": "芬兰",
    "australia-southeast1": "澳大利亚悉尼",
    "southamerica-east1": "巴西圣保罗",
    "me-west1": "中东特拉维夫",
}


def _parse_gcp_region(proxy_url: str | None) -> tuple[str, str]:
    if not proxy_url:
        return "", ""
    host = urllib.parse.urlsplit(proxy_url).hostname or ""
    idx = host.find(".prod.")
    if idx == -1:
        return "", ""
    prefix = host[:idx]
    raw = prefix.rsplit(".", 1)[-1]
    parts = raw.split("-")
    if len(parts) >= 3 and len(parts[-1]) == 1 and parts[-1].isalpha():
        code = "-".join(parts[:-1])
    else:
        code = raw
    cn = _GCP_REGION_CN.get(code)
    return code, (f"{cn} · {code}" if cn else code)


def _atomic_write_json(path: pathlib.Path, payload: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(".json.tmp")
    tmp.write_text(json.dumps(payload, ensure_ascii=False, indent=2))
    os.replace(tmp, path)


def _save_inventory_cache(inv: dict | None) -> None:
    if not isinstance(inv, dict) or not isinstance(inv.get("files"), list):
        return
    _atomic_write_json(INVENTORY_CACHE_FILE, inv)


def _read_inventory_cache() -> dict | None:
    if not INVENTORY_CACHE_FILE.is_file():
        return None
    try:
        data = json.loads(INVENTORY_CACHE_FILE.read_text())
        return data if isinstance(data.get("files"), list) else None
    except Exception:
        return None


def _write_live_state(payload: dict) -> None:
    _atomic_write_json(LIVE_STATE_FILE, {**payload, "updatedAt": int(time.time())})


def _ensure_ssh_key() -> tuple[pathlib.Path, str]:
    AUTH_DIR.mkdir(parents=True, exist_ok=True, mode=0o700)
    if not SSH_KEY_FILE.is_file() or not SSH_PUB_FILE.is_file():
        subprocess.run(
            ["ssh-keygen", "-t", "ed25519", "-N", "", "-C", "colab-linux", "-f", str(SSH_KEY_FILE)],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    os.chmod(SSH_KEY_FILE, 0o600)
    return SSH_KEY_FILE, SSH_PUB_FILE.read_text().strip()


def _load_valid_access_token() -> str:
    data = json.loads(TOKEN_FILE.read_text())
    now = int(time.time())
    if now < int(data.get("obtained_at", 0)) + int(data.get("expires_in", 3600)) - 120:
        return data["access_token"]
    if not data.get("refresh_token"):
        raise RuntimeError("OAuth access_token expired and no refresh_token available.")

    form = urllib.parse.urlencode({
        "client_id": data["client_id"],
        "client_secret": data["client_secret"],
        "refresh_token": data["refresh_token"],
        "grant_type": "refresh_token",
    }).encode()
    req = urllib.request.Request(
        data.get("token_uri", "https://oauth2.googleapis.com/token"),
        data=form,
        headers={"Content-Type": "application/x-www-form-urlencoded"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=20) as resp:
        refreshed = json.load(resp)

    data["access_token"] = refreshed["access_token"]
    data["expires_in"] = refreshed.get("expires_in", 3600)
    data["obtained_at"] = int(time.time())
    if refreshed.get("refresh_token"):
        data["refresh_token"] = refreshed["refresh_token"]

    tmp = TOKEN_FILE.with_suffix(".json.tmp")
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f:
        json.dump(data, f, indent=2)
    os.replace(tmp, TOKEN_FILE)
    return data["access_token"]


def _strip_xssi(raw: str) -> str:
    if raw.startswith(")]}'"):
        nl = raw.find("\n")
        return raw[nl + 1 :] if nl != -1 else ""
    return raw


def _colab_request(
    path: str,
    method: str = "GET",
    params: dict | None = None,
    extra_headers: dict | None = None,
    body: bytes | None = None,
    timeout: int = 45,
) -> dict:
    token = _load_valid_access_token()
    q = {"authuser": "0"}
    if params:
        q.update(params)
    url = f"{COLAB_DOMAIN}{path}?{urllib.parse.urlencode(q)}"
    headers = {
        "Authorization": f"Bearer {token}",
        "Accept": "application/json",
        "X-Colab-Client-Agent": "colab-cli",
    }
    if extra_headers:
        headers.update(extra_headers)
    data = b"" if method == "POST" and body is None else body
    for attempt in range(2):
        req = urllib.request.Request(url, data=data, headers=headers, method=method)
        try:
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                text = _strip_xssi(resp.read().decode("utf-8", "replace")).strip()
                return json.loads(text) if text else {}
        except (http.client.RemoteDisconnected, ConnectionResetError):
            if attempt == 1:
                raise
            time.sleep(0.6)
    return {}


def _proxy_request(proxy_url: str, proxy_token: str, path: str, method: str = "GET", json_body: dict | None = None) -> dict | list:
    url = proxy_url.rstrip("/") + path
    headers = {
        "X-Colab-Runtime-Proxy-Token": proxy_token,
        "X-Colab-Client-Agent": "colab-cli",
        "Accept": "application/json",
    }
    data = None
    if json_body is not None:
        data = json.dumps(json_body).encode()
        headers["Content-Type"] = "application/json"
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    with urllib.request.urlopen(req, timeout=30) as resp:
        raw = resp.read().decode("utf-8", "replace").strip()
        return json.loads(raw) if raw else {}


def _resolve_runtime_proxy(endpoint: str | None = None) -> dict:
    items = _colab_request("/tun/m/assignments").get("assignments", [])
    if not items:
        raise RuntimeError("No active Colab Linux instance found. Run `colabctl up` or `create_runtime` first.")
    if endpoint:
        for item in items:
            if item.get("endpoint") == endpoint:
                return item
        raise RuntimeError(f"Runtime endpoint {endpoint!r} not found in active assignments.")
    return items[0]


def _ensure_kernel_id(proxy_url: str, proxy_token: str, session_name: str = "comfyui-colab.ipynb") -> str:
    sessions = _proxy_request(proxy_url, proxy_token, "/api/sessions")
    if isinstance(sessions, list):
        for s in sessions:
            if s.get("path") == session_name and s.get("kernel", {}).get("id"):
                return s["kernel"]["id"]
        if sessions and sessions[0].get("kernel", {}).get("id"):
            return sessions[0]["kernel"]["id"]
    created = _proxy_request(
        proxy_url,
        proxy_token,
        "/api/sessions",
        method="POST",
        json_body={
            "path": session_name,
            "name": session_name,
            "type": "notebook",
            "kernel": {"name": "python3"},
        },
    )
    return created["kernel"]["id"]


def _multipart_empty_ipynb() -> tuple[bytes, str]:
    boundary = f"----ColabBoundary{uuid.uuid4().hex}"
    body = (
        f"--{boundary}\r\n"
        f'Content-Disposition: form-data; name="file_id"\r\n\r\n'
        f"empty.ipynb\r\n"
        f"--{boundary}--\r\n"
    ).encode()
    return body, f"multipart/form-data; boundary={boundary}"


def _propagate_drive_credentials(endpoint: str) -> dict:
    base_params = {
        "authtype": "dfs_ephemeral",
        "version": "2",
        "propagate": "true",
        "record": "false",
    }
    step1 = _colab_request(
        f"/tun/m/credentials-propagation/{endpoint}",
        method="GET",
        params={**base_params, "dryrun": "true"},
    )
    xsrf = step1["token"]
    def _post(dryrun: str) -> dict:
        body, ctype = _multipart_empty_ipynb()
        try:
            return _colab_request(
                f"/tun/m/credentials-propagation/{endpoint}",
                method="POST",
                params={**base_params, "dryrun": dryrun},
                extra_headers={"X-Goog-Colab-Token": xsrf, "Content-Type": ctype},
                body=body,
            )
        except urllib.error.HTTPError:
            return {"success": False}

    real = _post("false")
    if real.get("success"):
        return {"propagated": True}
    dry = _post("true")
    return {
        "propagated": False,
        "unauthorized_redirect_uri": dry.get("unauthorized_redirect_uri", ""),
    }


async def _ws_execute(
    proxy_url: str,
    proxy_token: str,
    kernel_id: str,
    code: str,
    timeout_sec: int = 300,
    endpoint_for_drive: str | None = None,
) -> dict:
    ws_base = "wss://" + proxy_url.split("://", 1)[1].rstrip("/")
    client_session = uuid.uuid4().hex
    q = urllib.parse.urlencode({
        "session_id": client_session,
        "colab-runtime-proxy-token": proxy_token,
    })
    ws_url = f"{ws_base}/api/kernels/{kernel_id}/channels?{q}"
    headers = {
        "X-Colab-Runtime-Proxy-Token": proxy_token,
        "X-Colab-Client-Agent": "colab-cli",
    }
    msg_id = uuid.uuid4().hex
    now_iso = dt.datetime.now(dt.timezone.utc).isoformat()
    req_msg = {
        "header": {
            "msg_id": msg_id,
            "username": "colab",
            "session": client_session,
            "date": now_iso,
            "msg_type": "execute_request",
            "version": "5.3",
        },
        "parent_header": {},
        "metadata": {},
        "content": {
            "code": code,
            "silent": False,
            "store_history": True,
            "user_expressions": {},
            "allow_stdin": bool(endpoint_for_drive),
            "stop_on_error": True,
        },
        "buffers": [],
        "channel": "shell",
    }

    stdout_parts: list[str] = []
    stderr_parts: list[str] = []
    results: list[str] = []
    errors: list[dict] = []
    drive_consent_url: str | None = None

    async with websockets.connect(ws_url, **{_WS_HDR_KW: headers}, max_size=32 * 1024 * 1024, open_timeout=30) as ws:
        await ws.send(json.dumps(req_msg))
        deadline = time.monotonic() + timeout_sec
        got_shell_reply = False
        is_idle = False

        while time.monotonic() < deadline:
            remaining = max(1.0, deadline - time.monotonic())
            raw = await asyncio.wait_for(ws.recv(), timeout=min(30.0, remaining))
            if isinstance(raw, bytes):
                raw = raw.decode("utf-8", "replace")
            msg = json.loads(raw)
            mtype = msg.get("header", {}).get("msg_type") or msg.get("msg_type", "")
            parent_id = msg.get("parent_header", {}).get("msg_id")
            content = msg.get("content", {})

            if mtype == "colab_request" and endpoint_for_drive:
                if content.get("request", {}).get("authType") == "dfs_ephemeral":
                    colab_msg_id = msg.get("metadata", {}).get("colab_msg_id")
                    prop = _propagate_drive_credentials(endpoint_for_drive)
                    if not prop.get("propagated"):
                        drive_consent_url = prop.get("unauthorized_redirect_uri") or ""
                        _proxy_request(proxy_url, proxy_token, f"/api/kernels/{kernel_id}/interrupt", method="POST", json_body={})
                        break
                    reply = {
                        "header": {
                            "msg_id": uuid.uuid4().hex,
                            "username": "colab",
                            "session": client_session,
                            "date": dt.datetime.now(dt.timezone.utc).isoformat(),
                            "msg_type": "input_reply",
                            "version": "5.3",
                        },
                        "parent_header": msg.get("header", {}),
                        "metadata": {},
                        "content": {"value": {"type": "colab_reply", "colab_msg_id": colab_msg_id}},
                        "buffers": [],
                        "channel": "stdin",
                    }
                    await ws.send(json.dumps(reply))
                continue

            if parent_id != msg_id:
                continue

            if mtype == "stream":
                if content.get("name") == "stderr":
                    stderr_parts.append(content.get("text", ""))
                else:
                    stdout_parts.append(content.get("text", ""))
            elif mtype in ("execute_result", "display_data"):
                txt = content.get("data", {}).get("text/plain")
                if txt:
                    results.append(txt)
            elif mtype == "error":
                errors.append({
                    "ename": content.get("ename", "Error"),
                    "evalue": content.get("evalue", ""),
                    "traceback": content.get("traceback", []),
                })
            elif mtype == "execute_reply":
                got_shell_reply = True
                if content.get("status") == "error" and not errors:
                    errors.append({
                        "ename": content.get("ename", "Error"),
                        "evalue": content.get("evalue", ""),
                        "traceback": content.get("traceback", []),
                    })
            elif mtype == "status" and content.get("execution_state") == "idle":
                is_idle = True

            if got_shell_reply and is_idle:
                break

    out = {
        "stdout": "".join(stdout_parts),
        "stderr": "".join(stderr_parts),
        "results": results,
        "errors": errors,
    }
    if drive_consent_url is not None:
        out["needs_drive_consent"] = True
        out["drive_consent_url"] = drive_consent_url
    return out


# ---------------- Native Protected /colab/ssh Bridge ----------------

def _build_ssh_ws_url(proxy_url: str, proxy_token: str, mode: str = "proxy") -> str:
    parsed = urllib.parse.urlsplit(proxy_url)
    scheme = "wss" if parsed.scheme == "https" else "ws"
    tok = urllib.parse.quote(proxy_token, safe="")
    return (
        f"{scheme}://{parsed.netloc}/colab/ssh"
        f"?colab-runtime-proxy-token={tok}"
        f"&colab-client-agent=colab-cli"
        f"&colab-client-agent=ssh-mode-{mode}"
    )


async def _ssh_ws_stdio_bridge(ws_url: str, pubkey: str) -> None:
    headers = {
        "X-Colab-Ssh-Pubkey": pubkey,
        "X-Colab-Client-Agent": "colab-cli",
    }
    loop = asyncio.get_running_loop()
    async with websockets.connect(
        ws_url,
        **{_WS_HDR_KW: headers},
        max_size=None,
        open_timeout=30,
        ping_interval=20,
        ping_timeout=20,
    ) as ws:
        async def stdin_to_ws():
            while True:
                # Raw POSIX read(2) avoids BufferedReader blocking on <8192-byte SSH banners
                chunk = await loop.run_in_executor(None, os.read, 0, 8192)
                if not chunk:
                    await ws.close()
                    break
                await ws.send(chunk)

        async def ws_to_stdout():
            async for frame in ws:
                data = frame.encode("utf-8") if isinstance(frame, str) else frame
                view = memoryview(data)
                written = 0
                while written < len(view):
                    n = await loop.run_in_executor(None, os.write, 1, view[written:])
                    written += n

        done, pending = await asyncio.wait(
            [asyncio.create_task(stdin_to_ws()), asyncio.create_task(ws_to_stdout())],
            return_when=asyncio.FIRST_COMPLETED,
        )
        for task in pending:
            task.cancel()
        for task in done:
            exc = task.exception()
            if exc and not isinstance(exc, websockets.exceptions.ConnectionClosed):
                raise exc


def _ssh_base_cmd(endpoint: str, mode: str = "cmd") -> list[str]:
    key_file, _ = _ensure_ssh_key()
    TMP_DIR.mkdir(parents=True, exist_ok=True)
    ctrl_sock = TMP_DIR / f"ssh-mux-{endpoint[:24]}"
    proxy_cmd = (
        f"{shlex.quote(sys.executable)} -I -B -u "
        f"{shlex.quote(str(BASE_DIR / 'colab_mcp_server.py'))} --ssh-proxy {shlex.quote(endpoint)} {shlex.quote(mode)}"
    )
    return [
        "ssh",
        "-i", str(key_file),
        "-o", f"ProxyCommand={proxy_cmd}",
        "-o", "ControlMaster=auto",
        "-o", f"ControlPath={ctrl_sock}",
        "-o", "ControlPersist=10m",
        "-o", "StrictHostKeyChecking=no",
        "-o", f"UserKnownHostsFile={os.devnull}",
        "-o", "LogLevel=ERROR",
        "-o", "ServerAliveInterval=20",
        "-o", "ServerAliveCountMax=3",
    ]


def _stop_keepalive_daemon() -> None:
    if KEEPALIVE_PID_FILE.is_file():
        try:
            os.kill(int(KEEPALIVE_PID_FILE.read_text().strip()), signal.SIGTERM)
        except Exception:
            pass
        KEEPALIVE_PID_FILE.unlink(missing_ok=True)
    subprocess.run(["pkill", "-f", "--keepalive-daemon"], capture_output=True)


def _ensure_keepalive_daemon(endpoint: str, local_port: int = 18188) -> None:
    if KEEPALIVE_PID_FILE.is_file():
        try:
            pid = int(KEEPALIVE_PID_FILE.read_text().strip())
            os.kill(pid, 0)
            return
        except Exception:
            KEEPALIVE_PID_FILE.unlink(missing_ok=True)
    TMP_DIR.mkdir(parents=True, exist_ok=True)
    cmd = [
        sys.executable, "-I", "-B", "-u",
        str(BASE_DIR / "colab_mcp_server.py"),
        "--keepalive-daemon", endpoint, str(local_port),
    ]
    with open(TMP_DIR / "keepalive.log", "ab") as lf:
        proc = subprocess.Popen(cmd, stdin=subprocess.DEVNULL, stdout=lf, stderr=lf, start_new_session=True)
    KEEPALIVE_PID_FILE.write_text(f"{proc.pid}\n")


def _stop_tunnel(stop_keepalive: bool = True) -> bool:
    stopped = False
    if stop_keepalive:
        _stop_keepalive_daemon()
    if TUNNEL_PID_FILE.is_file():
        try:
            pid = int(TUNNEL_PID_FILE.read_text().strip())
            os.kill(pid, signal.SIGTERM)
            stopped = True
        except Exception:
            pass
        TUNNEL_PID_FILE.unlink(missing_ok=True)
    if TMP_DIR.is_dir():
        for sock in TMP_DIR.glob("ssh-mux-*"):
            subprocess.run(["ssh", "-O", "exit", "-o", f"ControlPath={sock}", "root@colab-vm"], capture_output=True)
            sock.unlink(missing_ok=True)
            stopped = True
    if subprocess.run(["pkill", "-f", ":18188:127.0.0.1:8188"], capture_output=True).returncode == 0:
        stopped = True
    TUNNEL_EP_FILE.unlink(missing_ok=True)
    time.sleep(0.2)
    return stopped


def _is_port_open(port: int) -> bool:
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=0.4):
            pass
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        with opener.open(f"http://127.0.0.1:{port}/system_stats", timeout=2.0) as r:
            return r.status == 200
    except Exception:
        return False


LAN_HOST = "192.168.1.115"


def _start_tunnel(endpoint: str, local_port: int = 18188, remote_port: int = 8188, start_keepalive: bool = True) -> dict:
    cur_ep = TUNNEL_EP_FILE.read_text().strip() if TUNNEL_EP_FILE.is_file() else ""
    if cur_ep == endpoint and _is_port_open(local_port):
        if start_keepalive:
            _ensure_keepalive_daemon(endpoint, local_port)
        return {"listening": True, "url": f"http://127.0.0.1:{local_port}", "lan_url": f"http://{LAN_HOST}:{local_port}", "reused": True}
    _stop_tunnel(stop_keepalive=False)
    cmd = _ssh_base_cmd(endpoint, mode="proxy") + [
        "-N",
        "-o", "ExitOnForwardFailure=yes",
        "-L", f"0.0.0.0:{local_port}:127.0.0.1:{remote_port}",
        "root@colab-vm",
    ]
    TMP_DIR.mkdir(parents=True, exist_ok=True)
    log_path = TMP_DIR / "ssh-tunnel.log"
    with open(log_path, "ab") as lf:
        proc = subprocess.Popen(cmd, stdin=subprocess.DEVNULL, stdout=lf, stderr=lf, start_new_session=True)
    TUNNEL_PID_FILE.write_text(f"{proc.pid}\n")
    TUNNEL_EP_FILE.write_text(f"{endpoint}\n")
    for _ in range(50):
        if _is_port_open(local_port):
            if start_keepalive:
                _ensure_keepalive_daemon(endpoint, local_port)
            return {
                "listening": True,
                "pid": proc.pid,
                "url": f"http://127.0.0.1:{local_port}",
                "lan_url": f"http://{LAN_HOST}:{local_port}",
                "reused": False,
            }
        if proc.poll() is not None:
            err = log_path.read_text(errors="replace")[-1000:] if log_path.is_file() else ""
            raise RuntimeError(f"SSH tunnel exited ({proc.returncode}): {err}")
        time.sleep(0.3)
    raise RuntimeError("Timed out waiting for local SSH port-forward to pass HTTP health check.")


def _send_tfe_keepalive(endpoint: str) -> None:
    token = _load_valid_access_token()
    url = f"{COLAB_DOMAIN}/tun/m/{endpoint}/keep-alive/?authuser=0"
    req = urllib.request.Request(
        url,
        headers={
            "Authorization": f"Bearer {token}",
            "X-Colab-Tunnel": "Google",
            "X-Colab-Client-Agent": "colab-cli",
        },
        method="GET",
    )
    try:
        with urllib.request.urlopen(req, timeout=4):
            pass
    except (TimeoutError, socket.timeout):
        # TFE records LastActiveTime upon receipt; read timeout from VM is expected.
        pass
    except urllib.error.URLError as exc:
        if isinstance(getattr(exc, "reason", None), (TimeoutError, socket.timeout)):
            return
        raise


def _run_keepalive_daemon(endpoint: str, local_port: int = 18188) -> None:
    """Prevent Colab ~15min notebook-free idle reclamation & auto-heal dropped SSH tunnels."""
    tick = 0
    while True:
        time.sleep(20)
        tick += 1
        try:
            items = _colab_request("/tun/m/assignments").get("assignments", [])
            matched = next((x for x in items if x.get("endpoint") == endpoint), None)
            if not matched:
                print(f"[{time.strftime('%H:%M:%S')}] Endpoint {endpoint} unassigned; keepalive daemon exiting.", flush=True)
                _stop_tunnel(stop_keepalive=False)
                return

            # Auto-heal local SSH port-forward if dropped
            if not _is_port_open(local_port):
                print(f"[{time.strftime('%H:%M:%S')}] Tunnel {local_port} down while VM alive; reconnecting...", flush=True)
                try:
                    _start_tunnel(endpoint, local_port=local_port, remote_port=8188, start_keepalive=False)
                    print(f"[{time.strftime('%H:%M:%S')}] Tunnel {local_port} healed.", flush=True)
                except Exception as heal_exc:
                    print(f"[{time.strftime('%H:%M:%S')}] Tunnel heal error: {heal_exc}", flush=True)

            # Every ~60s: send both TFE keep-alive and a real Jupyter kernel execution heartbeat
            if tick % 3 == 0:
                try:
                    _send_tfe_keepalive(endpoint)
                except Exception as tfe_exc:
                    print(f"[{time.strftime('%H:%M:%S')}] TFE keepalive warn: {tfe_exc}", flush=True)
                rpi = matched.get("runtimeProxyInfo") or {}
                if rpi.get("url") and rpi.get("token"):
                    kid = _ensure_kernel_id(rpi["url"], rpi["token"])
                    asyncio.run(_ws_execute(rpi["url"], rpi["token"], kid, "1", timeout_sec=20))
                    print(f"[{time.strftime('%H:%M:%S')}] Kernel + TFE keepalive ok ({endpoint})", flush=True)
        except Exception as exc:
            print(f"[{time.strftime('%H:%M:%S')}] Keepalive loop error: {exc}", flush=True)


# ---------------- Tool Handlers ----------------

def tool_get_usage(_args: dict) -> dict:
    info = _colab_request("/tun/m/ccu-info")
    info["localTunnel18188"] = _is_port_open(18188)
    return info


def tool_list_runtimes(_args: dict) -> dict:
    items = _colab_request("/tun/m/assignments").get("assignments", [])
    cleaned = []
    for item in items:
        rpi = item.get("runtimeProxyInfo") or {}
        purl = rpi.get("url")
        rcode, rlabel = _parse_gcp_region(purl)
        cleaned.append({
            "endpoint": item.get("endpoint"),
            "accelerator": item.get("accelerator"),
            "variant": item.get("variant"),
            "machineShape": item.get("machineShape"),
            "proxyUrl": purl,
            "region": rcode,
            "regionLabel": rlabel,
            "tokenExpiresInSeconds": rpi.get("tokenExpiresInSeconds"),
        })
    return {"assignments": cleaned, "localTunnel18188": _is_port_open(18188)}


def tool_create_runtime(args: dict) -> dict:
    acc = (args.get("accelerator") or "G4").upper()
    high_mem = bool(args.get("high_mem", True))
    reuse = bool(args.get("reuse_existing", True))
    if reuse:
        for existing in _colab_request("/tun/m/assignments").get("assignments", []):
            if acc in ("ANY", existing.get("accelerator", "").upper()):
                rpi = existing.get("runtimeProxyInfo") or {}
                purl = rpi.get("url")
                rcode, rlabel = _parse_gcp_region(purl)
                return {
                    "reused": True,
                    "endpoint": existing.get("endpoint"),
                    "accelerator": existing.get("accelerator"),
                    "proxyUrl": purl,
                    "region": rcode,
                    "regionLabel": rlabel,
                }

    nbh_seed = (args.get("session_key") or str(uuid.uuid4())).replace("-", "_")
    nbh = (nbh_seed[:44]).ljust(44, ".")
    if acc in ("G4", "H100", "A100", "L4", "T4"):
        variant = "GPU"
    elif acc in ("V5E1", "V6E1"):
        variant = "TPU"
    else:
        variant = "DEFAULT"
        acc = "NONE"

    params = {"nbh": nbh, "variant": variant, "accelerator": acc}
    if high_mem and acc not in ("G4", "L4", "H100", "V5E1", "V6E1"):
        params["shape"] = "hm"

    try:
        step1 = _colab_request("/tun/m/assign", method="GET", params=params)
    except urllib.error.HTTPError as exc:
        if exc.code == 400 and "shape" in params:
            params.pop("shape", None)
            step1 = _colab_request("/tun/m/assign", method="GET", params=params)
        else:
            raise
    if "runtimeProxyInfo" in step1:
        assigned = step1
    else:
        try:
            assigned = _colab_request(
                "/tun/m/assign",
                method="POST",
                params=params,
                extra_headers={"X-Goog-Colab-Token": step1["token"]},
                timeout=90,
            )
        except urllib.error.HTTPError as exc:
            if exc.code in (503, 429):
                raise RuntimeError(
                    f"Colab 当前暂无空闲的 {acc} 实例（HTTP {exc.code} 资源池已满），请稍后重试或改选 G4 / A100。"
                ) from None
            raise
    rpi = assigned.get("runtimeProxyInfo") or {}
    purl = rpi.get("url")
    rcode, rlabel = _parse_gcp_region(purl)
    return {
        "reused": False,
        "endpoint": assigned.get("endpoint"),
        "accelerator": assigned.get("accelerator", acc),
        "proxyUrl": purl,
        "region": rcode,
        "regionLabel": rlabel,
    }


_SHUTDOWN_SYNC_FALLBACK = r"""import json, os, pathlib, subprocess, time
sb = pathlib.Path('/usr/local/bin/comfy-sync-drive')
if sb.is_file():
    p = subprocess.run(['python3', str(sb), '--flush'], text=True, capture_output=True)
    if p.returncode == 0 and p.stdout.strip().startswith('{'):
        print(p.stdout.strip())
        raise SystemExit(0)
os.sync()
def tx():
    tot = 0
    for ln in pathlib.Path('/proc/net/dev').read_text().splitlines()[2:]:
        if ':' in ln:
            i, r = ln.split(':', 1)
            if i.strip() != 'lo':
                c = r.split()
                if len(c) >= 9:
                    tot += int(c[8])
    return tot
prev = tx()
quiet = 0
for _ in range(45):
    time.sleep(1.0)
    cur = tx()
    if cur - prev <= 32768:
        quiet += 1
        if quiet >= 2:
            break
    else:
        quiet = 0
    prev = cur
print(json.dumps({'synced': 0, 'flushed': True}))
"""


def _sync_endpoint_before_down(endpoint: str) -> dict:
    _write_live_state({
        "phase": "syncing_shutdown",
        "endpoint": endpoint,
        "logTail": f"[{time.strftime('%H:%M:%S')}] 正在将工作流 JSON 与 output 图片同步回 Google Drive…",
    })
    try:
        cmd = f"python3 -c {shlex.quote(_SHUTDOWN_SYNC_FALLBACK)}"
        res = tool_ssh_exec({"endpoint": endpoint, "command": cmd, "timeout_sec": 75})
        for ln in reversed(res.get("stdout", "").splitlines()):
            ln = ln.strip()
            if ln.startswith("{"):
                return json.loads(ln)
    except Exception as exc:
        return {"flushed": False, "error": str(exc)}
    return {"flushed": False}


def tool_delete_runtime(args: dict) -> dict:
    endpoint = args["endpoint"]
    sync_info = _sync_endpoint_before_down(endpoint)
    _stop_tunnel()
    step1 = _colab_request(f"/tun/m/unassign/{endpoint}", method="GET")
    _colab_request(
        f"/tun/m/unassign/{endpoint}",
        method="POST",
        extra_headers={"X-Goog-Colab-Token": step1["token"]},
    )
    _write_live_state({"phase": "off"})
    return {"deleted": True, "endpoint": endpoint, "sync": sync_info}


def tool_down_all(_args: dict) -> dict:
    items = _colab_request("/tun/m/assignments").get("assignments", [])
    sync_results = {}
    for item in items:
        ep = item.get("endpoint")
        if ep:
            sync_results[ep] = _sync_endpoint_before_down(ep)
    tunnel_stopped = _stop_tunnel()
    deleted = []
    for item in items:
        ep = item.get("endpoint")
        if ep:
            step1 = _colab_request(f"/tun/m/unassign/{ep}", method="GET")
            _colab_request(
                f"/tun/m/unassign/{ep}",
                method="POST",
                extra_headers={"X-Goog-Colab-Token": step1["token"]},
            )
            deleted.append(ep)
    _write_live_state({"phase": "off"})
    return {"tunnel_stopped": tunnel_stopped, "deleted_endpoints": deleted, "sync": sync_results}


def tool_execute_code(args: dict) -> dict:
    rt = _resolve_runtime_proxy(args.get("endpoint"))
    rpi = rt["runtimeProxyInfo"]
    kernel_id = _ensure_kernel_id(rpi["url"], rpi["token"], args.get("session_name") or "comfyui-colab.ipynb")
    timeout_sec = int(args.get("timeout_sec", 300))
    res = asyncio.run(_ws_execute(rpi["url"], rpi["token"], kernel_id, args["code"], timeout_sec=timeout_sec))
    res["endpoint"] = rt.get("endpoint")
    return res


def tool_mount_drive(args: dict) -> dict:
    rt = _resolve_runtime_proxy(args.get("endpoint"))
    endpoint = rt["endpoint"]
    rpi = rt["runtimeProxyInfo"]
    mount_point = args.get("mount_point") or "/content/drive"
    kernel_id = _ensure_kernel_id(rpi["url"], rpi["token"])
    check_code = (
        f"import os\n"
        f"print('MOUNTED' if os.path.ismount({mount_point!r}) or os.path.isdir({mount_point + '/MyDrive'!r}) else 'NOT_MOUNTED')"
    )
    chk = asyncio.run(_ws_execute(rpi["url"], rpi["token"], kernel_id, check_code, timeout_sec=30))
    if "MOUNTED" in chk.get("stdout", "") and "NOT_MOUNTED" not in chk.get("stdout", ""):
        return {"mounted": True, "mount_point": mount_point, "endpoint": endpoint}

    mount_code = f"from google.colab import drive\ndrive.mount({mount_point!r})\n"
    res = asyncio.run(
        _ws_execute(
            rpi["url"],
            rpi["token"],
            kernel_id,
            mount_code,
            timeout_sec=120,
            endpoint_for_drive=endpoint,
        )
    )
    if res.get("needs_drive_consent"):
        return {
            "mounted": False,
            "needs_drive_consent": True,
            "drive_consent_url": res.get("drive_consent_url"),
            "endpoint": endpoint,
        }
    return {
        "mounted": not bool(res.get("errors")),
        "mount_point": mount_point,
        "endpoint": endpoint,
        "stdout": res.get("stdout", ""),
        "errors": res.get("errors", []),
    }


def tool_ssh_exec(args: dict) -> dict:
    rt = _resolve_runtime_proxy(args.get("endpoint"))
    endpoint = rt["endpoint"]
    timeout_sec = int(args.get("timeout_sec", 600))
    stdin_bytes = args.get("_stdin_bytes")
    cmd = _ssh_base_cmd(endpoint, mode="cmd") + ["-T", "root@colab-vm", f"cd /content 2>/dev/null; {args['command']}"]
    proc = subprocess.run(cmd, input=stdin_bytes, capture_output=True, timeout=timeout_sec)
    return {
        "endpoint": endpoint,
        "returncode": proc.returncode,
        "stdout": proc.stdout.decode("utf-8", "replace"),
        "stderr": proc.stderr.decode("utf-8", "replace"),
    }


_REMOTE_PROBE_SCRIPT = r"""import json, os, pathlib, time, urllib.request
drive = pathlib.Path('/content/drive/MyDrive/ComfyUI_Cloud')
local_m = pathlib.Path('/opt/comfy-models')
local_d = pathlib.Path('/opt/comfy-local')
out = {'driveMounted': drive.parent.is_dir()}
sf = pathlib.Path('/tmp/comfy-bootstrap-status.json')
if sf.is_file():
    try:
        out['bootstrap'] = json.loads(sf.read_text())
    except Exception:
        pass
mf = pathlib.Path('/tmp/comfy-manifest.json')
merged = {}
if mf.is_file():
    try:
        for item in json.loads(mf.read_text()).get('models', []):
            rel = item['path'].lstrip('/')
            merged[rel] = {'size': int(item.get('size') or 0), 'source': 'hf'}
    except Exception:
        pass
if (drive / 'models').is_dir():
    for src in sorted((drive / 'models').rglob('*')):
        if src.is_file():
            rel = str(src.relative_to(drive / 'models'))
            if rel not in merged:
                merged[rel] = {'size': src.stat().st_size, 'source': 'drive'}
bs_inv = (out.get('bootstrap') or {}).get('inventory')
bs_map = {f['path']: f for f in bs_inv.get('files', [])} if isinstance(bs_inv, dict) else {}
if merged or out['driveMounted']:
    files = []
    for rel in sorted(merged):
        sz = merged[rel]['size']
        dst = local_m / rel
        part = dst.with_name(dst.name + '.part')
        if dst.is_file() and (sz == 0 or dst.stat().st_size == sz) and dst.stat().st_size > 0:
            actual = dst.stat().st_size
            st, cb, sz = 'ready', actual, actual
        elif rel in bs_map and bs_map[rel].get('status') == 'copying':
            st, cb = 'copying', int(bs_map[rel].get('copiedBytes') or 0)
        elif part.is_file():
            st, cb = 'copying', min(sz, part.stat().st_size) if sz > 0 else part.stat().st_size
        else:
            st, cb = 'pending', 0
        files.append({'path': rel, 'size': sz, 'status': st, 'copiedBytes': cb, 'source': merged[rel]['source']})
    wf_local = local_d / 'user' / 'default' / 'workflows'
    wf_drive = drive / 'user' / 'default' / 'workflows'
    wf_dir = wf_local if wf_local.is_dir() else wf_drive
    wf_count = sum(1 for p in wf_dir.glob('*.json') if not p.name.startswith('.')) if wf_dir.is_dir() else None
    out['inventory'] = {
        'root': 'Hugging Face 直拉本地盘 + Google Drive 自定义模型',
        'observedAt': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
        'workflowCount': wf_count,
        'files': files,
    }
try:
    with urllib.request.urlopen('http://127.0.0.1:8188/system_stats', timeout=2) as r:
        out['systemStats'] = json.load(r)
except Exception:
    out['systemStats'] = None
link = pathlib.Path('/opt/ComfyUI/models')
try:
    out['modelsLocal'] = link.is_symlink() and link.resolve().is_relative_to(local_m)
except OSError:
    out['modelsLocal'] = False
logs = []
for lp in ('/var/log/comfy-bootstrap.log', '/var/log/comfyui.log'):
    p = pathlib.Path(lp)
    if p.is_file():
        lines = p.read_text(errors='replace').splitlines()[-15:]
        if lines:
            logs.append(f'=== {p.name} ===\n' + '\n'.join(lines))
out['logTail'] = '\n'.join(logs)[-3000:]
print(json.dumps(out, ensure_ascii=False))
"""


def _probe_remote(endpoint: str, timeout_sec: int = 12) -> dict | None:
    cmd = f"python3 -c {shlex.quote(_REMOTE_PROBE_SCRIPT)}"
    res = tool_ssh_exec({"endpoint": endpoint, "command": cmd, "timeout_sec": timeout_sec})
    if res["returncode"] != 0:
        return None
    for ln in reversed(res["stdout"].splitlines()):
        ln = ln.strip()
        if ln.startswith("{"):
            data = json.loads(ln)
            _save_inventory_cache(data.get("inventory"))
            return data
    return None


def _fallback_offline_inventory() -> dict | None:
    cached = _read_inventory_cache()
    manifest_models = []
    if MANIFEST_FILE.is_file():
        try:
            manifest_models = json.loads(MANIFEST_FILE.read_text()).get("models", [])
        except Exception:
            manifest_models = []
    merged: dict[str, dict] = {}
    for m in manifest_models:
        rel = m["path"].lstrip("/")
        merged[rel] = {"path": rel, "size": int(m.get("size") or 0), "status": "pending", "copiedBytes": 0, "source": "hf"}
    if cached and isinstance(cached.get("files"), list):
        for f in cached["files"]:
            rel = f.get("path")
            if rel and rel not in merged:
                merged[rel] = {**f, "status": "pending", "copiedBytes": 0, "source": f.get("source") or "drive"}
    if not merged and not cached:
        return None
    return {
        "root": "Hugging Face 直拉本地盘 + Google Drive 自定义模型",
        "observedAt": (cached or {}).get("observedAt"),
        "workflowCount": (cached or {}).get("workflowCount"),
        "files": [merged[k] for k in sorted(merged)],
        "cached": True,
    }


def tool_probe(_args: dict) -> dict:
    usage = tool_get_usage({})
    rts = tool_list_runtimes({})
    assignments = rts.get("assignments", [])
    tunnel_ok = bool(rts.get("localTunnel18188", False))
    remote = None
    live_state = None
    if LIVE_STATE_FILE.is_file():
        try:
            live_state = json.loads(LIVE_STATE_FILE.read_text())
        except Exception:
            live_state = None
    if not assignments:
        if live_state and live_state.get("phase") not in (None, "off"):
            _write_live_state({"phase": "off"})
            live_state = {"phase": "off"}
        return {
            "balance": round(usage.get("currentBalance", 0), 2),
            "burnRateHourly": usage.get("consumptionRateHourly", 0),
            "eligibleGpus": usage.get("eligibleGpus", []),
            "activeInstances": [],
            "localTunnel18188": tunnel_ok,
            "lanUrl": f"http://{LAN_HOST}:18188",
            "accountConnected": TOKEN_FILE.is_file(),
            "liveState": live_state,
            "remote": None,
            "inventory": _fallback_offline_inventory(),
        }
    endpoint = assignments[0]["endpoint"]
    try:
        remote = _probe_remote(endpoint, timeout_sec=12)
    except Exception:
        remote = None
    inv = (remote or {}).get("inventory")
    if inv:
        inv = {**inv, "cached": False}
    else:
        inv = _fallback_offline_inventory()
    return {
        "balance": round(usage.get("currentBalance", 0), 2),
        "burnRateHourly": usage.get("consumptionRateHourly", 0),
        "eligibleGpus": usage.get("eligibleGpus", []),
        "activeInstances": assignments,
        "localTunnel18188": tunnel_ok,
        "lanUrl": f"http://{LAN_HOST}:18188",
        "accountConnected": TOKEN_FILE.is_file(),
        "liveState": live_state,
        "remote": remote,
        "inventory": inv,
    }


def tool_up_comfy_linux(args: dict) -> dict:
    acc = (args.get("accelerator") or "G4").upper()
    high_mem = bool(args.get("high_mem", True))
    local_port = int(args.get("local_port", 18188))

    ts = lambda: time.strftime("%H:%M:%S")
    _write_live_state({"phase": "allocating", "accelerator": acc, "logTail": f"[{ts()}] 正在向 Colab 请求分配 {acc} 实例…"})
    rt_info = tool_create_runtime({"accelerator": acc, "high_mem": high_mem, "reuse_existing": True})
    endpoint = rt_info["endpoint"]
    rlabel = rt_info.get("regionLabel") or ""
    rtag = f" [{rlabel}]" if rlabel else ""
    _write_live_state({
        "phase": "mounting_drive",
        "accelerator": rt_info.get("accelerator", acc),
        "endpoint": endpoint,
        "regionLabel": rlabel,
        "logTail": f"[{ts()}] 实例 {endpoint}{rtag} 已就绪，正在挂载 Google Drive…",
    })

    drv = tool_mount_drive({"endpoint": endpoint})
    if not drv.get("mounted"):
        if drv.get("needs_drive_consent"):
            _write_live_state({
                "phase": "needs_drive_consent",
                "accelerator": rt_info.get("accelerator", acc),
                "endpoint": endpoint,
                "regionLabel": rlabel,
                "driveConsentUrl": drv.get("drive_consent_url", ""),
                "logTail": f"[{ts()}] 等待在浏览器中允许 Google Drive 授权 ({endpoint}{rtag})",
            })
        else:
            err_msg = "; ".join(str(e) for e in drv.get("errors", [])) or "Drive mount failed"
            _write_live_state({
                "phase": "error",
                "accelerator": rt_info.get("accelerator", acc),
                "endpoint": endpoint,
                "regionLabel": rlabel,
                "error": err_msg,
                "logTail": f"[{ts()}] 云盘挂载失败: {err_msg}",
            })
        return {
            "ready": False,
            "step": "mount_drive",
            "runtime": rt_info,
            "drive": drv,
        }

    _write_live_state({
        "phase": "bootstrapping",
        "accelerator": rt_info.get("accelerator", acc),
        "endpoint": endpoint,
        "regionLabel": rlabel,
        "logTail": f"[{ts()}] Google Drive 已挂载{rtag}，正在上传启动脚本并准备环境…",
    })
    # Upload then detach via Python start_new_session so non-interactive sshd exit never kills child.
    bundle = json.dumps({
        "manifest": MANIFEST_FILE.read_text(),
        "bootstrap": BOOTSTRAP_FILE.read_text(),
    }).encode("utf-8")
    upload_and_launch_cmd = (
        "python3 -c 'import json,pathlib,subprocess,sys; "
        "d=json.loads(sys.stdin.read()); "
        "pathlib.Path(\"/tmp/comfy-manifest.json\").write_text(d[\"manifest\"]); "
        "pathlib.Path(\"/tmp/cloud_bootstrap.py\").write_text(d[\"bootstrap\"]); "
        "running=subprocess.run([\"pgrep\",\"-f\",\"[p]ython3 .*cloud_bootstrap.py\"],capture_output=True).returncode==0; "
        "(!running) and None' 2>/dev/null || true"
    )
    upload_cmd = (
        "python3 -c 'import json,pathlib,subprocess,sys\n"
        "d=json.loads(sys.stdin.read())\n"
        "pathlib.Path(\"/tmp/comfy-manifest.json\").write_text(d[\"manifest\"])\n"
        "pathlib.Path(\"/tmp/cloud_bootstrap.py\").write_text(d[\"bootstrap\"])\n"
        "if subprocess.run([\"pgrep\", \"-f\", \"[p]ython3 -u /tmp/cloud_bootstrap.py\"], capture_output=True).returncode != 0:\n"
        "    pathlib.Path(\"/tmp/comfy-bootstrap-status.json\").unlink(missing_ok=True)\n"
        "    lf = open(\"/var/log/comfy-bootstrap.log\", \"ab\")\n"
        "    subprocess.Popen([\"python3\", \"-u\", \"/tmp/cloud_bootstrap.py\", \"/tmp/comfy-manifest.json\"], stdin=subprocess.DEVNULL, stdout=lf, stderr=lf, start_new_session=True)\n"
        "'"
    )
    del upload_and_launch_cmd
    uploaded = tool_ssh_exec({"endpoint": endpoint, "command": upload_cmd, "timeout_sec": 60, "_stdin_bytes": bundle})
    if uploaded["returncode"] != 0:
        raise RuntimeError(f"Bootstrap launch failed (rc={uploaded['returncode']}):\n{uploaded['stderr'][-1000:]}")

    phase_labels = {
        "extracting": "正在解压恢复 ComfyUI 与 34 个插件环境包…",
        "syncing_models": "正在准备本地工作流目录与模型清单…",
        "copying_models": "正在从 Hugging Face (16线程) 拉取模型到实例本地盘…",
        "starting_comfy": "模型已就位，正在启动 ComfyUI 服务…",
        "ready": "ComfyUI 已启动，准备建立本地隧道…",
    }
    boot_meta = None
    last_tail = ""
    deadline = time.monotonic() + 1200
    while time.monotonic() < deadline:
        remote = _probe_remote(endpoint, timeout_sec=15)
        if remote:
            bs = remote.get("bootstrap") or {}
            phase = bs.get("phase") or ("ready" if remote.get("systemStats") and remote.get("modelsLocal") else "bootstrapping")
            inv = remote.get("inventory") or bs.get("inventory")
            progress_hint = ""
            if inv and isinstance(inv.get("files"), list) and inv["files"]:
                tot = sum(int(f.get("size", 0)) for f in inv["files"])
                cpd = sum(int(f.get("copiedBytes", 0)) for f in inv["files"])
                if tot > 0 and phase == "copying_models":
                    progress_hint = f" ({cpd / 1024**3:.2f} / {tot / 1024**3:.2f} GiB)"
            header = f"[{ts()}] {phase_labels.get(phase, '正在准备云端环境…')}{progress_hint}"
            rtail = (remote.get("logTail") or "").strip()
            last_tail = f"{header}\n{rtail}".strip() if rtail else header
            st = bs.get("status")
            # Do NOT publish phase="ready" until _start_tunnel passes HTTP check below!
            interim_phase = "starting_comfy" if (phase == "ready" or st in ("ready", "already_running")) else phase
            _write_live_state({
                "phase": interim_phase,
                "accelerator": rt_info.get("accelerator", acc),
                "endpoint": endpoint,
                "regionLabel": rlabel,
                "bootstrap": bs,
                "inventory": inv,
                "logTail": last_tail,
            })
            if st in ("ready", "already_running"):
                boot_meta = bs
                break
            if st == "error":
                raise RuntimeError(bs.get("error") or "Remote bootstrap failed")
        time.sleep(4)
    if boot_meta is None:
        tail = tool_ssh_exec({"endpoint": endpoint, "command": "tail -c 2000 /var/log/comfy-bootstrap.log", "timeout_sec": 30})
        raise RuntimeError(f"Bootstrap did not finish within 1200s:\n{tail.get('stdout', '')}")

    tunnel = _start_tunnel(endpoint, local_port=local_port, remote_port=8188)
    _write_live_state({
        "phase": "ready",
        "accelerator": rt_info.get("accelerator", acc),
        "endpoint": endpoint,
        "regionLabel": rlabel,
        "bootstrap": boot_meta,
        "inventory": boot_meta.get("inventory"),
        "lanUrl": tunnel["lan_url"],
        "logTail": f"[{ts()}] 就绪{rtag}，隧道已验证连通 {tunnel['lan_url']}\n{last_tail}".strip(),
    })
    return {
        "ready": True,
        "runtime": rt_info,
        "local_comfyui_url": tunnel["url"],
        "tunnel": tunnel,
        "bootstrap": boot_meta,
    }


TOOLS = [
    {
        "name": "get_usage",
        "description": "Read-only check of Colab compute unit balance, burn rate, eligible GPUs/TPUs, and local tunnel status.",
        "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False},
        "handler": tool_get_usage,
    },
    {
        "name": "list_runtimes",
        "description": "Read-only list of active Colab Linux instances on this account.",
        "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False},
        "handler": tool_list_runtimes,
    },
    {
        "name": "create_runtime",
        "description": "Allocate or reuse a Colab Linux GPU instance (starts compute unit billing while running).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "accelerator": {"type": "string", "default": "G4", "description": "G4 (96GB), H100 (80GB), A100, L4, T4, NONE."},
                "high_mem": {"type": "boolean", "default": True},
                "reuse_existing": {"type": "boolean", "default": True},
                "session_key": {"type": "string"},
            },
            "additionalProperties": False,
        },
        "handler": tool_create_runtime,
    },
    {
        "name": "ssh_exec",
        "description": "Execute native Linux shell commands as root over Colab's protected /colab/ssh endpoint.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "Bash command to run as root on the Colab Linux VM."},
                "endpoint": {"type": "string"},
                "timeout_sec": {"type": "integer", "default": 600},
            },
            "required": ["command"],
            "additionalProperties": False,
        },
        "handler": tool_ssh_exec,
    },
    {
        "name": "up_comfy_linux",
        "description": "One-click notebook-free boot: allocate GPU Linux VM, mount Google Drive, restore/install ComfyUI v0.45.4 + 34 plugins (no DLSS), and open local encrypted tunnel http://127.0.0.1:18188.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "accelerator": {"type": "string", "default": "G4", "description": "G4 (96GB), H100 (80GB), A100, L4."},
                "high_mem": {"type": "boolean", "default": True},
                "local_port": {"type": "integer", "default": 18188},
            },
            "additionalProperties": False,
        },
        "handler": tool_up_comfy_linux,
    },
    {
        "name": "down_all",
        "description": "Stop local SSH tunnel and unassign all Colab instances immediately to stop compute unit billing.",
        "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False},
        "handler": tool_down_all,
    },
    {
        "name": "delete_runtime",
        "description": "Unassign a specific Colab runtime endpoint and stop local tunnel.",
        "inputSchema": {
            "type": "object",
            "properties": {"endpoint": {"type": "string"}},
            "required": ["endpoint"],
            "additionalProperties": False,
        },
        "handler": tool_delete_runtime,
    },
    {
        "name": "mount_drive",
        "description": "Mount Google Drive at /content/drive on the Colab Linux VM without opening a browser notebook.",
        "inputSchema": {
            "type": "object",
            "properties": {"endpoint": {"type": "string"}, "mount_point": {"type": "string", "default": "/content/drive"}},
            "additionalProperties": False,
        },
        "handler": tool_mount_drive,
    },
    {
        "name": "execute_code",
        "description": "Execute Python code in the runtime kernel when needed.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "code": {"type": "string"},
                "endpoint": {"type": "string"},
                "session_name": {"type": "string"},
                "timeout_sec": {"type": "integer", "default": 300},
            },
            "required": ["code"],
            "additionalProperties": False,
        },
        "handler": tool_execute_code,
    },
]
TOOL_MAP = {t["name"]: t["handler"] for t in TOOLS}


# ---------------- MCP Transport (HTTP & stdio) ----------------

def _handle_rpc_dict(req: dict) -> dict | None:
    msg_id = req.get("id")
    method = req.get("method", "")
    params = req.get("params") or {}
    if msg_id is None:
        return None

    def ok(res):
        return {"jsonrpc": "2.0", "id": msg_id, "result": res}

    def err(code, message):
        return {"jsonrpc": "2.0", "id": msg_id, "error": {"code": code, "message": message}}

    if method == "initialize":
        return ok({
            "protocolVersion": params.get("protocolVersion", "2025-03-26"),
            "capabilities": {"tools": {"listChanged": False}},
            "serverInfo": {"name": "google-colab", "version": "2.0.0"},
        })
    if method == "ping":
        return ok({})
    if method == "tools/list":
        return ok({"tools": [{k: v for k, v in t.items() if k != "handler"} for t in TOOLS]})
    if method == "tools/call":
        name = params.get("name", "")
        fn = TOOL_MAP.get(name)
        if not fn:
            return err(-32601, f"Unknown tool: {name}")
        try:
            res = fn(params.get("arguments") or {})
            return ok({"content": [{"type": "text", "text": json.dumps(res, ensure_ascii=False, indent=2)}], "isError": False})
        except urllib.error.HTTPError as exc:
            detail = exc.read(1500).decode("utf-8", "replace")
            return ok({"content": [{"type": "text", "text": f"HTTP {exc.code}: {detail}"}], "isError": True})
        except Exception as exc:
            return ok({"content": [{"type": "text", "text": f"{type(exc).__name__}: {exc}"}], "isError": True})
    return err(-32601, f"Method not found: {method}")


class _MCPHTTPHandler(http.server.BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        return

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length)
        try:
            req = json.loads(raw.decode("utf-8"))
        except json.JSONDecodeError as exc:
            out = {"jsonrpc": "2.0", "id": None, "error": {"code": -32700, "message": str(exc)}}
            body = json.dumps(out).encode("utf-8")
            self.send_response(400)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return

        resp = _handle_rpc_dict(req)
        if resp is None:
            self.send_response(202)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        body = json.dumps(resp, ensure_ascii=False).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def _reply_stdio(msg_id, result=None, error=None, use_lsp: bool = False):
    out = {"jsonrpc": "2.0", "id": msg_id}
    if error is not None:
        out["error"] = error
    else:
        out["result"] = result
    payload = json.dumps(out, ensure_ascii=False).encode("utf-8")
    if use_lsp:
        sys.stdout.buffer.write(f"Content-Length: {len(payload)}\r\n\r\n".encode("ascii") + payload)
    else:
        sys.stdout.buffer.write(payload + b"\n")
    sys.stdout.buffer.flush()


# ---------------- CLI Entrypoints (`colabctl`) ----------------

def _run_cli(argv: list[str]) -> None:
    parser = argparse.ArgumentParser(prog="colabctl", description="Notebook-free Colab Linux GPU + ComfyUI controller")
    sub = parser.add_subparsers(dest="subcmd", required=True)

    sub.add_parser("status", help="Show compute unit balance, active Linux VM, and tunnel status")
    sub.add_parser("probe", help="Read-only full probe including live or cached Drive inventory and remote logs")

    p_up = sub.add_parser("up", help="Boot Linux GPU VM, mount Drive, start ComfyUI, and open local tunnel")
    p_up.add_argument("--gpu", default="G4", help="GPU type: G4 (96GB), H100 (80GB), A100, L4, T4, NONE")
    p_up.add_argument("--port", type=int, default=18188, help="Local port mapped to remote ComfyUI 8188")

    p_ssh = sub.add_parser("ssh", help="Open interactive root Linux shell (or pass a command to run)")
    p_ssh.add_argument("cmd", nargs=argparse.REMAINDER, help="Optional Linux command to run as root")

    p_tun = sub.add_parser("tunnel", help="Open or verify local encrypted SSH tunnel to remote 8188")
    p_tun.add_argument("--port", type=int, default=18188)

    sub.add_parser("down", help="Stop tunnel and shut down all active Colab Linux VMs (0 billing)")

    ns = parser.parse_args(argv)
    if ns.subcmd == "status":
        usage = tool_get_usage({})
        rts = tool_list_runtimes({})
        print(json.dumps({
            "balance": round(usage.get("currentBalance", 0), 2),
            "burnRateHourly": usage.get("consumptionRateHourly", 0),
            "eligibleGpus": usage.get("eligibleGpus", []),
            "activeInstances": rts.get("assignments", []),
            "localTunnel18188": rts.get("localTunnel18188", False),
        }, ensure_ascii=False, indent=2))
    elif ns.subcmd == "probe":
        print(json.dumps(tool_probe({}), ensure_ascii=False, indent=2))
    elif ns.subcmd == "up":
        try:
            res = tool_up_comfy_linux({"accelerator": ns.gpu, "local_port": ns.port})
        except Exception as exc:
            _write_live_state({"phase": "error", "accelerator": ns.gpu.upper(), "error": str(exc), "logTail": f"[{time.strftime('%H:%M:%S')}] {exc}"})
            print(str(exc), file=sys.stderr)
            sys.exit(1)
        print(json.dumps(res, ensure_ascii=False, indent=2))
        if not res.get("ready") and res.get("drive", {}).get("needs_drive_consent"):
            url = res["drive"]["drive_consent_url"]
            print(f"\n首次挂载云盘需点一次授权（请切到 20x 账号确认后再跑一次 colabctl up）：\n{url}")
    elif ns.subcmd == "ssh":
        rt = _resolve_runtime_proxy()
        endpoint = rt["endpoint"]
        extra = ns.cmd
        if extra and extra[0] == "--":
            extra = extra[1:]
        if not extra:
            ssh_args = _ssh_base_cmd(endpoint, mode="interactive") + [
                "-t", "root@colab-vm", "cd /opt/ComfyUI 2>/dev/null || cd /content; exec /bin/bash -l"
            ]
            os.execvp("ssh", ssh_args)
        else:
            joined = " ".join(extra)
            ssh_args = _ssh_base_cmd(endpoint, mode="cmd") + ["-T", "root@colab-vm", f"cd /content 2>/dev/null; {joined}"]
            os.execvp("ssh", ssh_args)
    elif ns.subcmd == "tunnel":
        rt = _resolve_runtime_proxy()
        print(json.dumps(_start_tunnel(rt["endpoint"], local_port=ns.port), ensure_ascii=False, indent=2))
    elif ns.subcmd == "down":
        print(json.dumps(tool_down_all({}), ensure_ascii=False, indent=2))


def main() -> None:
    if len(sys.argv) >= 3 and sys.argv[1] == "--http":
        host, port_str = sys.argv[2].split(":", 1)
        srv = http.server.ThreadingHTTPServer((host, int(port_str)), _MCPHTTPHandler)
        srv.serve_forever()
        return

    if len(sys.argv) >= 3 and sys.argv[1] == "--keepalive-daemon":
        ep = sys.argv[2]
        port = int(sys.argv[3]) if len(sys.argv) >= 4 else 18188
        _run_keepalive_daemon(ep, port)
        return

    if len(sys.argv) >= 3 and sys.argv[1] == "--ssh-proxy":
        endpoint = sys.argv[2]
        mode = sys.argv[3] if len(sys.argv) >= 4 else "proxy"
        _, pubkey = _ensure_ssh_key()
        rt = _resolve_runtime_proxy(endpoint)
        rpi = rt["runtimeProxyInfo"]
        ws_url = _build_ssh_ws_url(rpi["url"], rpi["token"], mode=mode)
        asyncio.run(_ssh_ws_stdio_bridge(ws_url, pubkey))
        return

    if len(sys.argv) >= 2 and sys.argv[1] in ("status", "probe", "up", "ssh", "tunnel", "down", "-h", "--help"):
        _run_cli(sys.argv[1:])
        return

    # Default: MCP stdio mode
    while True:
        raw_line = sys.stdin.buffer.readline()
        if not raw_line:
            break
        use_lsp = False
        if raw_line.lower().startswith(b"content-length:"):
            use_lsp = True
            length = int(raw_line.split(b":", 1)[1].strip())
            while True:
                hdr = sys.stdin.buffer.readline()
                if not hdr or not hdr.strip():
                    break
            body = sys.stdin.buffer.read(length)
        else:
            body = raw_line.strip()
            if not body:
                continue
        try:
            req = json.loads(body.decode("utf-8"))
        except json.JSONDecodeError as exc:
            _reply_stdio(None, error={"code": -32700, "message": f"Parse error: {exc}"}, use_lsp=use_lsp)
            continue

        resp = _handle_rpc_dict(req)
        if resp is None:
            continue
        if "error" in resp:
            _reply_stdio(resp["id"], error=resp["error"], use_lsp=use_lsp)
        else:
            _reply_stdio(resp["id"], result=resp["result"], use_lsp=use_lsp)


if __name__ == "__main__":
    main()
