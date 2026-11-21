#!/usr/bin/env python3
"""A tiny WebDAV server for the UI tests: fake-webdav.py PORT_FILE ROOT_DIR USER:PASSWORD.
Binds a free loopback port, writes it to PORT_FILE, and serves ROOT_DIR with basic authentication.
Supports PROPFIND (depth 0/1), MKCOL, PUT, GET and DELETE, which is all Bookx's backups use."""
import base64
import os
import sys
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

port_file, root, creds = sys.argv[1], os.path.abspath(sys.argv[2]), sys.argv[3]
os.makedirs(root, exist_ok=True)
EXPECTED = "Basic " + base64.b64encode(creds.encode()).decode()


class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def local(self):
        p = urllib.parse.unquote(urllib.parse.urlparse(self.path).path)
        full = os.path.abspath(os.path.join(root, p.lstrip("/")))
        return full if full.startswith(root) else None

    def reply(self, code, body=b"", ctype="text/plain"):
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)

    def auth(self):
        if self.headers.get("Authorization") != EXPECTED:
            self.reply(401, b"no")
            return False
        return True

    def body(self):
        n = int(self.headers.get("Content-Length") or 0)
        return self.rfile.read(n) if n else b""

    def do_PROPFIND(self):
        self.body()
        if not self.auth():
            return
        p = self.local()
        if p is None or not os.path.exists(p):
            return self.reply(404)
        href = urllib.parse.urlparse(self.path).path
        entries = [(href, p)]
        if os.path.isdir(p) and self.headers.get("Depth", "1") != "0":
            for n in sorted(os.listdir(p)):
                entries.append((href.rstrip("/") + "/" + urllib.parse.quote(n), os.path.join(p, n)))
        x = '<?xml version="1.0"?><d:multistatus xmlns:d="DAV:">'
        for h, f in entries:
            if os.path.isdir(f):
                x += f"<d:response><d:href>{h.rstrip('/')}/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response>"
            else:
                x += f"<d:response><d:href>{h}</d:href><d:propstat><d:prop><d:resourcetype/><d:getcontentlength>{os.path.getsize(f)}</d:getcontentlength></d:prop></d:propstat></d:response>"
        x += "</d:multistatus>"
        self.reply(207, x.encode(), "application/xml")

    def do_MKCOL(self):
        self.body()
        if not self.auth():
            return
        p = self.local()
        if p is None:
            return self.reply(403)
        if os.path.exists(p):
            return self.reply(405)
        if not os.path.isdir(os.path.dirname(p)):
            return self.reply(409)
        os.mkdir(p)
        self.reply(201)

    def do_PUT(self):
        data = self.body()
        if not self.auth():
            return
        p = self.local()
        if p is None or not os.path.isdir(os.path.dirname(p)):
            return self.reply(409)
        with open(p, "wb") as f:
            f.write(data)
        self.reply(201)

    def do_GET(self):
        if not self.auth():
            return
        p = self.local()
        if p is None or not os.path.isfile(p):
            return self.reply(404)
        with open(p, "rb") as f:
            self.reply(200, f.read(), "application/octet-stream")

    def do_DELETE(self):
        if not self.auth():
            return
        p = self.local()
        if p and os.path.isfile(p):
            os.remove(p)
            return self.reply(204)
        self.reply(404)


ThreadingHTTPServer.request_queue_size = 64  # a real server queues more than 5 connections
srv = ThreadingHTTPServer(("127.0.0.1", 0), H)
with open(port_file, "w") as f:
    f.write(str(srv.server_address[1]))
srv.serve_forever()
