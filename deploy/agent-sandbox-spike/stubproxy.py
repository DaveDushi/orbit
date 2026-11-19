#!/usr/bin/python3 -I
"""Stub egress proxy (spike): CONNECT only; allows only allowed.test and answers itself. Never dials out."""
import socketserver
import sys


class Handler(socketserver.StreamRequestHandler):
    def handle(self):
        line = self.rfile.readline(4096).decode("latin-1").strip()
        headers = []
        while (h := self.rfile.readline(4096).strip()):
            headers.append(h.decode("latin-1"))
        auth = next((h for h in headers if h.lower().startswith("proxy-authorization")), "none")
        print(f"{self.client_address[0]} {line!r} auth={auth[:40]!r}", flush=True)
        parts = line.split()
        if len(parts) != 3 or parts[0] != "CONNECT":
            self.wfile.write(b"HTTP/1.1 405 Method Not Allowed\r\n\r\n")
            return
        if parts[1] == "allowed.test:443":
            self.wfile.write(b"HTTP/1.1 200 Connection established\r\n\r\nhello from stub upstream\n")
        else:
            body = b'{"error":"egress_approval_required","host":"%s"}' % parts[1].encode()
            self.wfile.write(b"HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\n\r\n" + body)


socketserver.ThreadingTCPServer.allow_reuse_address = True
socketserver.ThreadingTCPServer((sys.argv[1], 3128), Handler).serve_forever()
