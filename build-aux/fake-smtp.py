#!/usr/bin/env python3
"""A one-shot fake SMTP server for the UI tests: fake-smtp.py PORT_FILE OUT_FILE.
Binds a free loopback port, writes it to PORT_FILE, and appends every received message to OUT_FILE."""
import socket
import sys

port_file, out_file = sys.argv[1], sys.argv[2]
srv = socket.socket()
srv.bind(("127.0.0.1", 0))
srv.listen(4)
with open(port_file, "w") as f:
    f.write(str(srv.getsockname()[1]))
while True:
    conn, _ = srv.accept()
    f = conn.makefile("rwb")
    f.write(b"220 fake ESMTP\r\n"); f.flush()
    in_data = False
    while True:
        line = f.readline()
        if not line:
            break
        if in_data:
            if line == b".\r\n":
                in_data = False
                f.write(b"250 queued\r\n"); f.flush()
            else:
                with open(out_file, "ab") as o:
                    o.write(line)
            continue
        up = line.strip().upper()
        with open(out_file, "ab") as o:
            o.write(b"CMD " + line)
        if up.startswith(b"EHLO"):
            f.write(b"250-fake\r\n250 AUTH PLAIN LOGIN\r\n")
        elif up.startswith(b"DATA"):
            in_data = True
            f.write(b"354 go\r\n")
        elif up.startswith(b"QUIT"):
            f.write(b"221 bye\r\n"); f.flush()
            break
        else:
            f.write(b"250 ok\r\n")
        f.flush()
    conn.close()
