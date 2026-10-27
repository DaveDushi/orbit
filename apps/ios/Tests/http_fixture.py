"""Loopback-only transport fixture for the native HTTP client; no real Orbit data."""
import json
import signal
import uuid
from email.parser import BytesParser
from email.policy import default
from http.server import BaseHTTPRequestHandler, HTTPServer


class Handler(BaseHTTPRequestHandler):
    redirects = 0

    def log_message(self, *_):
        pass

    def respond(self, value, status=200, headers=None):
        body = json.dumps(value).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        for key, value in (headers or {}).items():
            self.send_header(key, value)
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/api/v1/login":
            Handler.redirects += 1
        if self.path == "/api/v1/gateway":
            self.respond({}, 307, {"Location": "/api/v1/login"})
        else:
            self.respond({"redirects": Handler.redirects})

    def do_POST(self):
        size = int(self.headers.get("Content-Length", 0))
        if size > 1024 * 1024:
            self.respond({}, 413)
            return
        body = self.rfile.read(size)
        origin = f"http://127.0.0.1:{self.server.server_port}"
        if self.headers.get("Origin") != origin or self.headers.get("Cookie") != "session=test-only":
            self.respond({}, 401)
            return
        if self.path == "/api/v1/gateway":
            self.respond({}, 307, {"Location": "/api/v1/login"})
        elif self.path == "/api/v1/messages":
            message = json.loads(body)
            if message.get("body") != "Caption" or not message.get("nonce"):
                self.respond({}, 422)
            else:
                self.respond({"result": {"id": str(uuid.uuid4())}})
        else:
            try:
                parsed = BytesParser(policy=default).parsebytes(
                    b"Content-Type: " + self.headers["Content-Type"].encode() + b"\r\n\r\n" + body)
                parts = list(parsed.iter_parts())
                assert len(parts) == 1
                part = parts[0]
                assert part.get_param("name", header="content-disposition") == "file"
                assert part.get_content_type() == "image/jpeg"
                assert part.get_filename() and "\n" not in part.get_filename()
                assert part.get_payload(decode=True) == b"fixture-photo-bytes"
            except (AssertionError, KeyError, ValueError):
                self.respond({}, 422)
                return
            record = {"id": str(uuid.uuid4())}
            self.respond([record] if self.path == "/api/v1/task" else record, 201)


# Swift's async XCTest executor can pass a blocked signal mask through Process.
# Unblock termination so the test can stop exactly this child in its defer.
signal.pthread_sigmask(signal.SIG_UNBLOCK, {signal.SIGTERM})
server = HTTPServer(("127.0.0.1", 0), Handler)
print(server.server_port, flush=True)
server.serve_forever()
