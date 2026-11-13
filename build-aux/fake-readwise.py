#!/usr/bin/env python3
"""A tiny Readwise for the UI tests: fake-readwise.py PORT_FILE REQUEST_LOG TOKEN.
Binds a free loopback port, writes it to PORT_FILE, and accepts POST /api/v2/highlights/ with
"Authorization: Token TOKEN". Every accepted body is appended to REQUEST_LOG, one JSON per line."""
import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

port_file, log, token = sys.argv[1], sys.argv[2], sys.argv[3]


class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def do_POST(self):
        n = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(n)
        ok = self.path == "/api/v2/highlights/" and self.headers.get("Authorization") == "Token " + token
        if ok:
            with open(log, "a") as f:
                f.write(json.dumps(json.loads(body)) + "\n")
        out = b"[]" if ok else b'{"detail":"Invalid token."}'
        self.send_response(200 if ok else 401)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(out)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(out)


s = ThreadingHTTPServer(("127.0.0.1", 0), H)
open(port_file, "w").write(str(s.server_address[1]))
s.serve_forever()
