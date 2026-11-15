#!/usr/bin/env python3
"""Manual browser OAuth probe. No token persistence or runtime mutations.

Run --self-test for local checks. Actual login needs the existing local OAuth
client and explicit browser consent. Only sanitized results are written.
"""
import argparse
import base64
import hashlib
import hmac
import html
import http.server
import json
import os
from pathlib import Path
import secrets
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import webbrowser

AUTH_URL = 'https://accounts.google.com/o/oauth2/v2/auth'
TOKEN_URL = 'https://oauth2.googleapis.com/token'
SCOPES = 'openid https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/colaboratory'
TTL = 600


def challenge(verifier):
    return base64.urlsafe_b64encode(hashlib.sha256(verifier.encode('ascii')).digest()).decode().rstrip('=')


def valid_callback(query, expected, created, consumed, now=None):
    if consumed or (time.monotonic() if now is None else now) - created >= TTL:
        return False
    supplied = query.get('state', [])
    if len(supplied) != 1 or not hmac.compare_digest(supplied[0].encode(), expected.encode()):
        return False
    codes, errors = query.get('code', []), query.get('error', [])
    return (len(codes) == 1 and bool(codes[0]) and not errors) or (len(errors) == 1 and bool(errors[0]) and not codes)


def request_json(url, fields=None, bearer=None):
    headers = {'Accept': 'application/json'}
    if bearer:
        headers['Authorization'] = 'Bearer ' + bearer
    body = urllib.parse.urlencode(fields).encode() if fields is not None else None
    try:
        with urllib.request.urlopen(urllib.request.Request(url, data=body, headers=headers), timeout=25) as response:
            raw = response.read().decode()
            if raw.startswith(")]}'"):
                raw = raw.split('\n', 1)[1]
            return response.status, json.loads(raw)
    except urllib.error.HTTPError as error:
        return error.code, {}
    except (urllib.error.URLError, TimeoutError, ValueError):
        return 'network_or_response_error', {}


def self_test():
    good = {'state': ['expected'], 'code': ['one-use-code']}
    assert valid_callback(good, 'expected', 0, False, now=1)
    assert valid_callback({'state': ['expected'], 'error': ['access_denied']}, 'expected', 0, False, now=1)
    assert not valid_callback(good, 'other', 0, False, now=1)
    assert not valid_callback(good, 'expected', 0, True, now=1)
    assert not valid_callback(good, 'expected', 0, False, now=TTL)
    assert not valid_callback({'state': ['expected', 'expected'], 'code': ['x']}, 'expected', 0, False, now=1)
    assert not valid_callback({'state': ['expected'], 'code': ['x', 'y']}, 'expected', 0, False, now=1)
    assert not valid_callback({'state': ['expected'], 'code': ['x'], 'error': ['access_denied']}, 'expected', 0, False, now=1)
    print('PASS: OAuth state, cancellation, expiry, replay and duplicate-parameter checks', flush=True)


def run(args):
    from google.auth.transport.requests import Request
    from google.oauth2.id_token import verify_oauth2_token

    config = json.loads(args.client_file.read_text())
    client = {key: config[key] for key in ('client_id', 'client_secret')}
    redirect = f'http://127.0.0.1:{args.port}/callback'
    nonce, oauth_state, verifier = secrets.token_urlsafe(32), secrets.token_urlsafe(32), secrets.token_urlsafe(64)
    auth_url = AUTH_URL + '?' + urllib.parse.urlencode({
        'client_id': client['client_id'], 'redirect_uri': redirect,
        'response_type': 'code', 'scope': SCOPES, 'access_type': 'offline',
        'prompt': 'select_account consent', 'state': oauth_state, 'nonce': nonce,
        'code_challenge': challenge(verifier), 'code_challenge_method': 'S256',
    })
    created = time.monotonic()
    lock = threading.Lock()
    consumed = False
    result = {'phase': 'waiting', 'browser_flow': 'system browser + PKCE S256 + loopback', 'original_credentials_modified': False}
    args.result_file.parent.mkdir(parents=True, exist_ok=True, mode=0o700)

    # This is a one-shot test; only nonsecret checks survive process shutdown.
    def publish(phase, **checks):
        with lock:
            result.update(checks, phase=phase)
            temp = args.result_file.with_suffix('.tmp')
            descriptor = os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
            with os.fdopen(descriptor, 'w') as output:
                json.dump(result, output, ensure_ascii=False, indent=2)
            os.replace(temp, args.result_file)
        print('OAuth probe: ' + phase, flush=True)

    def verify(code):
        status, token = request_json(TOKEN_URL, {
            **client, 'grant_type': 'authorization_code', 'code': code,
            'redirect_uri': redirect, 'code_verifier': verifier,
        })
        publish('verifying', authorization_code_http=status)
        if status != 200 or not token.get('access_token') or not token.get('id_token'):
            publish('error', reason='authorization_code_exchange_failed')
            return
        try:
            claims = verify_oauth2_token(token['id_token'], Request(), client['client_id'])
            if not hmac.compare_digest(str(claims.get('nonce', '')).encode(), nonce.encode()):
                raise ValueError('nonce mismatch')
            if not claims.get('sub'):
                raise ValueError('missing subject')
        except Exception:
            publish('error', reason='id_token_verification_failed')
            return
        identity_http, identity = request_json('https://openidconnect.googleapis.com/v1/userinfo', bearer=token['access_token'])
        identity_match = identity_http == 200 and identity.get('sub') == claims['sub']
        usage_http, usage = request_json('https://colab.research.google.com/tun/m/ccu-info?authuser=0', bearer=token['access_token'])
        assignments_http, assignments = request_json('https://colab.research.google.com/tun/m/assignments?authuser=0', bearer=token['access_token'])
        refresh_http = 'not_issued'
        if token.get('refresh_token'):
            refresh_http, refreshed = request_json(TOKEN_URL, {**client, 'grant_type': 'refresh_token', 'refresh_token': token['refresh_token']})
            if refresh_http == 200 and not refreshed.get('access_token'):
                refresh_http = 'missing_access_token'
        complete = identity_match and usage_http == assignments_http == refresh_http == 200
        publish('complete' if complete else 'partial',
                id_token_verified=True, identity_http=identity_http, identity_subject_matches=identity_match,
                email_verified=bool(identity.get('email_verified')), colab_usage_http=usage_http,
                colab_assignments_http=assignments_http, refresh_token_received=bool(token.get('refresh_token')),
                refresh_exchange_http=refresh_http, runtime_mutation_requests=0,
                granted_scopes=token.get('scope', '').split())
        # No ID/access/refresh token or Google account identifier goes to disk/logs.

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass  # Callback query contains an authorization code.

        def respond(self, status, content, content_type='text/html; charset=utf-8'):
            payload = content.encode()
            self.send_response(status)
            self.send_header('Content-Type', content_type)
            self.send_header('Content-Length', str(len(payload)))
            self.send_header('Cache-Control', 'no-store')
            self.send_header('Referrer-Policy', 'no-referrer')
            self.send_header('X-Content-Type-Options', 'nosniff')
            self.send_header('Content-Security-Policy', "default-src 'none'; style-src 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'")
            self.end_headers()
            self.wfile.write(payload)

        def do_GET(self):
            nonlocal consumed
            if self.headers.get('Host') not in (f'127.0.0.1:{args.port}', f'localhost:{args.port}'):
                self.respond(400, 'Invalid host')
                return
            parsed = urllib.parse.urlsplit(self.path)
            if parsed.path == '/callback':
                query = urllib.parse.parse_qs(parsed.query, keep_blank_values=True)
                with lock:
                    valid = valid_callback(query, oauth_state, created, consumed)
                    if valid:
                        consumed = True
                if not valid:
                    self.respond(400, 'Invalid, expired or previously consumed OAuth response. No credentials changed.')
                    return
                if 'error' in query:
                    publish('denied', reason='google_authorization_not_granted')
                else:
                    publish('verifying', callback_state_verified=True)
                    try:
                        verify(query['code'][0])
                    except Exception:
                        publish('error', reason='verification_failed')
                self.respond(200, '<!doctype html><meta charset="utf-8"><meta name="referrer" content="no-referrer"><title>Cloud Deck OAuth</title><h1>授权返回已处理</h1><p>请回到 Cloud Deck 会话查看验证结果。本测试没有覆盖原授权，也没有申请实例。</p><a href="/">查看脱敏测试结果</a>')
                return
            if parsed.path == '/status':
                with lock:
                    payload = json.dumps(result, ensure_ascii=False)
                self.respond(200, payload, 'application/json; charset=utf-8')
                return
            if parsed.path != '/':
                self.respond(404, 'Not found')
                return
            with lock:
                phase, safe = result['phase'], html.escape(json.dumps(result, ensure_ascii=False, indent=2))
            link = f'<a href="{html.escape(auth_url, quote=True)}" target="_blank" rel="noreferrer noopener">在系统浏览器中打开 Google 登录</a>' if phase == 'waiting' else ''
            self.respond(200, '<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><link rel="icon" href="data:,"><title>Cloud Deck · OAuth 实测</title><style>body{font:15px/1.8 -apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;background:#f1f2f7;color:#262b39;max-width:760px;margin:9vh auto;padding:24px}h1{font-size:27px}a{color:#5b50cb}pre{font-size:12px;background:white;padding:20px;border-radius:9px;white-space:pre-wrap;overflow-wrap:anywhere}p{color:#626c7f}</style></head><body><h1>Google OAuth 真实登录测试</h1><p>请在系统默认浏览器选择 Google 账号并授权。测试自动接回结果，只检查身份、刷新令牌与 Colab 只读接口，不申请实例，也不覆盖原授权。</p>' + link + '<p>状态：' + html.escape(phase) + '。完成后刷新本页可查看结果。</p><pre>' + safe + '</pre></body></html>')

    server = http.server.ThreadingHTTPServer(('127.0.0.1', args.port), Handler)
    publish('waiting')

    def expire():
        with lock:
            pending = result['phase'] == 'waiting'
        if pending:
            publish('timeout', reason='browser_login_timed_out')

    deadline = threading.Timer(TTL, expire)
    deadline.daemon = True
    deadline.start()
    if args.open_browser:
        def open_browser():
            try:
                opened = webbrowser.open(auth_url, new=2)
            except Exception:
                opened = False
            print('System browser open requested: ' + str(opened), flush=True)
        threading.Thread(target=open_browser, daemon=True).start()
    print(f'OAuth probe ready at http://127.0.0.1:{args.port}', flush=True)
    try:
        server.serve_forever()
    finally:
        deadline.cancel()
        server.server_close()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--self-test', action='store_true')
    parser.add_argument('--client-file', type=Path, default=Path(__file__).resolve().parent / 'auth/google-oauth-token.json')
    parser.add_argument('--result-file', type=Path)
    parser.add_argument('--port', type=int, default=18773)
    parser.add_argument('--open-browser', action='store_true')
    args = parser.parse_args()
    if args.self_test:
        self_test()
    else:
        if not args.result_file:
            parser.error('--result-file is required; choose a path under /Volumes/VoltDrive/AI/')
        run(args)
