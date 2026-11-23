#!/usr/bin/env python3
"""A KOSync server that matches books by the content of their file, like Grimmory (BookLore): it knows
one document id and answers 404 "Book not found for hash ..." to a push or a pull for any other.

  kosync-content-server.py PORT KNOWN_DOC LOGFILE

Every request is appended to LOGFILE as "METHOD PATH STATUS" (a push also names its document).
"""
import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

PORT, KNOWN, LOG = int(sys.argv[1]), sys.argv[2], sys.argv[3]


class H(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def reply(self, status, body):
        data = json.dumps(body).encode()
        with open(LOG, "a") as f:
            f.write(f"{self.command} {self.path} {status} {self.doc}\n")
        self.send_response(status)
        self.send_header("Content-Type", "application/vnd.koreader.v1+json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    doc = ""

    def do_GET(self):
        if self.path.endswith("/healthcheck") or self.path.endswith("/users/auth"):
            return self.reply(200, {"username": "alice", "state": "OK"})
        self.doc = self.path.rsplit("/", 1)[-1]
        self.reply(404, {"message": "Book not found for hash " + self.doc})

    def do_PUT(self):
        n = int(self.headers.get("Content-Length", 0))
        body = json.loads(self.rfile.read(n) or b"{}")
        self.doc = body.get("document", "")
        if self.doc == KNOWN:
            return self.reply(200, {"document": self.doc, "timestamp": 1})
        self.reply(404, {"message": "Book not found for hash " + self.doc})


HTTPServer(("127.0.0.1", PORT), H).serve_forever()
