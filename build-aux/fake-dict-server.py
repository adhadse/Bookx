#!/usr/bin/env python3
"""A fake dictionary server for the UI tests of the dictionary downloads.

  fake-dict-server.py PORTFILE DIR [DELAY_MS] [LANGS] [MODE] [WORDS_FILE]

Builds in DIR a StarDict archive per language (tar.gz; words of the test books with the definition
"A mock definition of <word>." and some padding so a download takes a moment), a catalogue
`DIR/catalog.json` (the format of data/dictionaries.json, addresses on this server) and serves them on a free
port. The port is written to PORTFILE when the server is ready. DELAY_MS pauses after every 8 KB sent.
LANGS is a comma list (default `en,de`). MODE `down` writes a catalogue whose address nobody listens on.
WORDS_FILE: more headwords, one per line (the real-session test uses the words of its book).
"""
import hashlib
import http.server
import io
import json
import os
import struct
import sys
import tarfile
import threading
import time

port_file, root = sys.argv[1], sys.argv[2]
delay = float(sys.argv[3]) / 1000 if len(sys.argv) > 3 else 0
langs = (sys.argv[4] if len(sys.argv) > 4 else "en,de").split(",")
mode = sys.argv[5] if len(sys.argv) > 5 else ""
os.makedirs(root, exist_ok=True)
words_file = sys.argv[6] if len(sys.argv) > 6 else ""

WORDS = ["chapter", "paragraph", "functionality", "readers", "world", "plain", "words", "talks", "around", "them",
         "alice", "wonderland", "rabbit", "hole", "ein", "buch", "text", "eins", "styled", "slanted", "heavy"]
NAMES = {"en": "English", "de": "German", "fr": "French"}


def stardict(lang):
    extra = open(words_file).read().split() if words_file and os.path.exists(words_file) else []
    words = sorted(set(WORDS) | set(extra))
    dict_data, idx = b"", b""
    for w in words:
        d = f"A mock definition of {w} ({lang}).".encode()
        idx += w.encode() + b"\0" + struct.pack(">II", len(dict_data), len(d))
        dict_data += d
    ifo = (f"StarDict's dict ifo file\nversion=2.4.2\nbookname=Mock {NAMES.get(lang, lang)} dictionary ({lang})\n"
           f"wordcount={len(words)}\nidxfilesize={len(idx)}\nsametypesequence=m\n").encode()
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as t:
        for name, data in ((f"mock-{lang}/mock.ifo", ifo), (f"mock-{lang}/mock.idx", idx), (f"mock-{lang}/mock.dict", dict_data),
                           (f"mock-{lang}/padding.bin", os.urandom(240_000))):
            ti = tarfile.TarInfo(name)
            ti.size = len(data)
            t.addfile(ti, io.BytesIO(data))
    return buf.getvalue()


blobs = {f"/{l}.tgz": stardict(l) for l in langs}


class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_GET(self):
        body = blobs.get(self.path)
        if body is None:
            self.send_response(404)
            self.end_headers()
            return
        start = 0
        r = self.headers.get("Range")
        if r and r.startswith("bytes="):
            start = int(r[6:].split("-")[0] or 0)
        part = body[start:]
        if start:
            self.send_response(206)
            self.send_header("Content-Range", f"bytes {start}-{len(body) - 1}/{len(body)}")
        else:
            self.send_response(200)
        self.send_header("Content-Length", str(len(part)))
        self.end_headers()
        try:
            for i in range(0, len(part), 8192):
                self.wfile.write(part[i:i + 8192])
                self.wfile.flush()
                if delay:
                    time.sleep(delay)
        except (BrokenPipeError, ConnectionResetError):
            pass


srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
port = srv.server_address[1]
base = f"http://127.0.0.1:{1 if mode == 'down' else port}"
cat = {"version": 1, "languages": []}
for l in langs:
    b = blobs[f"/{l}.tgz"]
    cat["languages"].append({
        "code": l, "name": NAMES.get(l, l),
        "dictionaries": [{
            "id": f"mock-{l}", "title": f"Mock {NAMES.get(l, l)} dictionary", "kind": "monolingual", "gloss": l,
            "source": "the test server", "licence": "CC0", "licence_url": "https://creativecommons.org/publicdomain/zero/1.0/",
            "size": len(b),
            "origins": [{"url": f"{base}/{l}.tgz", "format": "tar.gz", "sha256": hashlib.sha256(b).hexdigest(), "size": len(b)}],
        }],
    })
with open(os.path.join(root, "catalog.json"), "w") as f:
    json.dump(cat, f)
with open(port_file + ".tmp", "w") as f:
    f.write(str(port))
os.rename(port_file + ".tmp", port_file)
threading.Thread(target=srv.serve_forever, daemon=True).start()
while True:
    time.sleep(3600)
