#!/usr/bin/env python3
"""A fake OpenAI-compatible speech server for the UI tests of read aloud.

    tts-fake-server.py PORTFILE LOGFILE [SECONDS_OF_AUDIO]

Answers POST /v1/audio/speech with a WAV tone (22.05 kHz, 16 bit, mono, 0.6 s unless told), GET
/v1/audio/voices with two voices; every request is logged to LOGFILE as `METHOD PATH {json}`. The port
it listens on (a free one) is written to PORTFILE. Used by build-aux/ui-tests.sh (scenario tts_server);
needs no network and no model.
"""
import json
import math
import struct
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORTFILE, LOGFILE = sys.argv[1], sys.argv[2]
SECONDS = float(sys.argv[3]) if len(sys.argv) > 3 else 0.6
RATE = 22050


def wav():
    n = int(RATE * SECONDS)
    data = b"".join(struct.pack("<h", int(9000 * math.sin(i / RATE * 2 * math.pi * 220))) for i in range(n))
    head = b"RIFF" + struct.pack("<I", 36 + len(data)) + b"WAVEfmt " + struct.pack("<IHHIIHH", 16, 1, 1, RATE, RATE * 2, 2, 16)
    return head + b"data" + struct.pack("<I", len(data)) + data


class H(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def _log(self, body=b""):
        with open(LOGFILE, "a") as f:
            f.write(f"{self.command} {self.path} {body.decode('utf8', 'replace')}\n")

    def do_GET(self):
        self._log()
        if self.path.rstrip("/") == "/v1/audio/voices":
            out = json.dumps({"voices": ["fake-a", "fake-b"]}).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
        else:
            out = b"not found"
            self.send_response(404)
        self.send_header("Content-Length", str(len(out)))
        self.end_headers()
        self.wfile.write(out)

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        self._log(body)
        if self.path == "/v1/audio/speech":
            out = wav()
            self.send_response(200)
            self.send_header("Content-Type", "audio/wav")
        else:
            out = b"not found"
            self.send_response(404)
        self.send_header("Content-Length", str(len(out)))
        self.end_headers()
        self.wfile.write(out)


srv = ThreadingHTTPServer(("127.0.0.1", 0), H)
open(PORTFILE, "w").write(str(srv.server_address[1]))
srv.serve_forever()
