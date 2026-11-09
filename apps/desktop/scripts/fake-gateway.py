#!/usr/bin/env python3
"""A fake exe.dev gateway in front of a fake Orbit, for local end-to-end runs of the sign-in
window (`--gateway-signin`) and `--check-gateway`. Synthetic data only; listens on 127.0.0.1.

Usage: python3 -I fake-gateway.py PORT PASSWORD_FILE [MODE[,MODE]]
  forward (default): with the gateway cookie, Authorization reaches Orbit.
  strip:  the gateway removes Authorization before Orbit.
  reject: the gateway answers 401 itself when Authorization is present.
  keepcookie: the gateway passes its own cookie on to Orbit (by default it removes it).
The fake Orbit applies Orbit's origin rule: a POST or upgrade without Origin that carries a
cookie gets 403 origin_forbidden. The account is ada@example.test, password in PASSWORD_FILE.
"""
import base64, hashlib, json, secrets, struct, sys, time, urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT, PASSWORD_FILE = int(sys.argv[1]), sys.argv[2]
MODES = set((sys.argv[3] if len(sys.argv) > 3 else "forward").split(","))
PASSWORD = open(PASSWORD_FILE).read().strip()
COOKIE = "exe-proxy=signed-in-" + secrets.token_hex(8)
SESSIONS = set()
WORKSPACE = "0190aaaa-0000-7000-8000-000000000001"
USER = {"id": "u1", "email": "ada@example.test", "display_name": "Ada Example", "installation_admin": False, "root": False, "status": {}}

LOGIN_PAGE = """<!doctype html><html><head><meta charset="utf-8"><title>exe.dev</title>
<meta http-equiv="refresh" content="3;url=/__exe.dev/done?redirect=%s">
<style>body{font:15px system-ui,sans-serif;background:#f6f6f4;display:flex;align-items:center;justify-content:center;height:100vh;margin:0}
.card{background:#fff;border:1px solid #ddd;border-radius:12px;padding:32px;width:320px}
input{width:100%%;box-sizing:border-box;padding:8px;margin:6px 0 14px;border:1px solid #ccc;border-radius:6px}
button{width:100%%;padding:9px;border:0;border-radius:6px;background:#111;color:#fff}</style></head>
<body><div class="card"><h2>Sign in to exe.dev</h2><p>to continue to %s (fake gateway, local test)</p>
<label>Email</label><input value="ada@example.test"><button>Send sign-in link</button>
<p style="color:#888">Signing in automatically…</p></div></body></html>"""


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):  # one line per request, never headers
        sys.stderr.write("fake-gateway: %s %s\n" % (self.command, self.path.split("?")[0]))

    def reply(self, status, body=b"", headers=()):
        self.send_response(status)
        for name, value in headers:
            self.send_header(name, value)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def json(self, status, value):
        self.reply(status, json.dumps(value).encode(), [("Content-Type", "application/json")])

    def problem(self, status, code, detail):
        body = json.dumps({"code": code, "title": code, "detail": detail, "status": status}).encode()
        self.reply(status, body, [("Content-Type", "application/problem+json")])

    def gateway(self):
        """True when the request passes the gateway; else the gateway answered."""
        path = urllib.parse.urlparse(self.path)
        if path.path.startswith("/__exe.dev/login"):
            target = urllib.parse.parse_qs(path.query).get("redirect", ["/"])[0]
            page = LOGIN_PAGE % (urllib.parse.quote(target, safe=""), self.headers.get("Host"))
            self.reply(200, page.encode(), [("Content-Type", "text/html; charset=utf-8")])
            return False
        if path.path == "/__exe.dev/done":
            target = urllib.parse.parse_qs(path.query).get("redirect", ["/"])[0]
            self.reply(302, b"", [("Location", target), ("Set-Cookie", COOKIE + "; Path=/; HttpOnly; Max-Age=86400")])
            return False
        cookies = [c.strip() for c in (self.headers.get("Cookie") or "").split(";")]
        if COOKIE not in cookies:
            target = urllib.parse.quote(self.path, safe="")
            self.reply(307, b"", [("Location", "/__exe.dev/login?exedev_host=local&redirect=" + target)])
            return False
        if self.headers.get("Authorization") and "reject" in MODES:
            self.reply(401, b"invalid or missing authentication", [("Content-Type", "text/plain"), ("WWW-Authenticate", "Bearer, Basic")])
            return False
        if "strip" in MODES:
            del self.headers["Authorization"]
        if "keepcookie" not in MODES:
            rest = [c for c in cookies if c and c != COOKIE]
            del self.headers["Cookie"]
            if rest:
                self.headers["Cookie"] = "; ".join(rest)
        return self.origin_allowed()

    def origin_allowed(self):
        """Orbit's rule (crates/platform/src/http/security.rs): GETs pass; a POST or upgrade without
        Origin passes only without cookies and with a bearer (or as a sign-in)."""
        if self.command == "GET" and not self.headers.get("Upgrade"):
            return True
        origin = self.headers.get("Origin")
        if origin is None:
            ok = not self.headers.get("Cookie") and (bool(self.headers.get("Authorization")) or self.path.startswith("/api/v1/auth/login"))
        else:
            ok = origin == "http://127.0.0.1:%d" % PORT
        if not ok:
            self.problem(403, "origin_forbidden", "The request origin is not allowed.")
        return ok

    def session(self):
        value = self.headers.get("Authorization") or ""
        if value.startswith("Bearer ") and value[7:] in SESSIONS:
            return value[7:]
        for c in (self.headers.get("Cookie") or "").split(";"):
            name, _, token = c.strip().partition("=")
            if name == "orbit_session_dev" and token in SESSIONS:
                return token
        return None

    def do_GET(self):
        if not self.gateway():
            return
        path = urllib.parse.urlparse(self.path).path
        if path == "/api/v1/setup/status":
            return self.json(200, {"complete": True})
        if path in (f"/api/v1/workspaces/{WORKSPACE}/live", f"/api/v1/workspaces/{WORKSPACE}/events"):
            return self.socket(path.endswith("/live"))
        if not self.session():
            return self.problem(401, "authentication_required", "Sign in.")
        if path == "/api/v1/auth/me":
            return self.json(200, USER)
        if path == "/api/v1/workspaces":
            return self.json(200, [{"id": WORKSPACE, "name": "Demo workspace", "role": "member", "permissions": [], "version": 1}])
        if path.endswith("/chat/conversations"):
            return self.json(200, [{"id": "c1", "kind": "public", "name": "general", "position": 0}])
        if path.endswith("/chat/states"):
            return self.json(200, [{"conversation_id": "c1", "unread_count": 2, "notify": "all"}])
        self.problem(404, "not_found", "Not found.")

    def do_POST(self):
        if not self.gateway():
            return
        body = self.rfile.read(int(self.headers.get("Content-Length") or 0))
        path = urllib.parse.urlparse(self.path).path
        if path == "/api/v1/auth/login":
            data = json.loads(body or b"{}")
            if self.headers.get("Origin"):
                return self.problem(400, "browser_request", "A native sign-in sends no Origin.")
            if data.get("email") != USER["email"] or data.get("password") != PASSWORD:
                return self.problem(401, "invalid_credentials", "The email or password is not correct.")
            token = "sess-" + secrets.token_hex(16)
            SESSIONS.add(token)
            return self.json(200, {"user": USER, "session_id": "s-" + token[-6:], "session_token": token})
        if path == "/api/v1/auth/logout":
            token = self.session()
            if not token:
                return self.problem(401, "authentication_required", "Sign in.")
            SESSIONS.discard(token)
            return self.reply(204)
        self.problem(404, "not_found", "Not found.")

    def socket(self, live):
        if not self.session():
            if live:  # Orbit upgrades and closes with 4401; the events socket answers 401.
                self.upgrade()
                return self.frame(0x8, struct.pack("!H", 4401))
            return self.reply(401)
        self.upgrade()
        if live:
            self.frame(0x1, json.dumps({"type": "hello", "epoch": "fake", "seq": 0, "presence": []}).encode())
            self.frame(0x1, b'{"type":"resync"}')
        else:
            self.frame(0x1, json.dumps({"version": 1, "kind": "resync_required", "sequence": "0"}).encode())
        # Stay open like Orbit: a ping every few seconds until the client goes.
        try:
            while True:
                time.sleep(5)
                self.frame(0x1, b'{"type":"ping"}') if live else self.frame(0x9, b"")
        except OSError:
            pass

    def upgrade(self):
        accept = base64.b64encode(hashlib.sha1((self.headers["Sec-WebSocket-Key"] + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()).decode()
        self.send_response(101)
        self.send_header("Upgrade", "websocket")
        self.send_header("Connection", "Upgrade")
        self.send_header("Sec-WebSocket-Accept", accept)
        self.end_headers()
        self.close_connection = True

    def frame(self, opcode, payload):
        header = bytes([0x80 | opcode]) + (bytes([len(payload)]) if len(payload) < 126 else bytes([126]) + struct.pack("!H", len(payload)))
        self.wfile.write(header + payload)
        self.wfile.flush()


ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
