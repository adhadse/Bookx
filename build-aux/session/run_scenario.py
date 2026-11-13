#!/usr/bin/python3
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import harness  # noqa: E402

if __name__ == "__main__":
    harness.run(sys.argv[1])
