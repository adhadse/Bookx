#!/usr/bin/env python3
"""A second KOSync client for the UI tests (it plays "KOReader on a Kobo"), written independently
of the Rust client from the public protocol description, so the two check each other.

  kosync-peer.py partial-md5 FILE
  kosync-peer.py filename-md5 FILE
  kosync-peer.py wait BASE            wait until the server answers /healthcheck
  kosync-peer.py register BASE USER PASSWORD
  kosync-peer.py put BASE USER PASSWORD DOC PERCENTAGE DEVICE DEVICE_ID
  kosync-peer.py get BASE USER PASSWORD DOC     prints "<percentage> <device> <progress>" or "none"
"""
import hashlib
import json
import os
import sys
import time
import urllib.error
import urllib.request


def partial_md5(path):
    # KOReader util.partialMD5: 1024 bytes at offsets 0 and 1024 << 2i for i = 0..10
    h = hashlib.md5()
    with open(path, "rb") as f:
        for off in [0] + [1024 << (2 * i) for i in range(11)]:
            f.seek(off)
            chunk = f.read(1024)
            if not chunk:
                break
            h.update(chunk)
    return h.hexdigest()


def call(base, method, path, user=None, password=None, body=None):
    req = urllib.request.Request(base + path, method=method)
    req.add_header("Accept", "application/vnd.koreader.v1+json")
    if user is not None:
        req.add_header("x-auth-user", user)
        req.add_header("x-auth-key", hashlib.md5(password.encode()).hexdigest())
    data = None
    if body is not None:
        data = json.dumps(body).encode()
        req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, data=data, timeout=10) as r:
            return r.status, json.loads(r.read() or b"{}")
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read() or b"{}")


def main(a):
    cmd = a[0]
    if cmd == "partial-md5":
        print(partial_md5(a[1]))
    elif cmd == "filename-md5":
        print(hashlib.md5(os.path.basename(a[1]).encode()).hexdigest())
    elif cmd == "wait":
        for _ in range(100):
            try:
                if call(a[1], "GET", "/healthcheck")[0] == 200:
                    return 0
            except Exception:
                pass
            time.sleep(0.1)
        return 1
    elif cmd == "register":
        st, _ = call(
            a[1],
            "POST",
            "/users/create",
            body={"username": a[2], "password": hashlib.md5(a[3].encode()).hexdigest()},
        )
        print(st)
        return 0 if st in (201, 402) else 1
    elif cmd == "put":
        st, _ = call(
            a[1],
            "PUT",
            "/syncs/progress",
            a[2],
            a[3],
            {
                "document": a[4],
                "progress": "/body/DocFragment[10]/body/p[3]/text().0",
                "percentage": float(a[5]),
                "device": a[6],
                "device_id": a[7],
            },
        )
        print(st)
        return 0 if st == 200 else 1
    elif cmd == "get":
        st, v = call(a[1], "GET", "/syncs/progress/" + a[4], a[2], a[3])
        if st == 200 and "percentage" in v:
            print(v["percentage"], v["device"].replace(" ", "_"), v["progress"])
        else:
            print("none")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
