#!/usr/bin/env bash
# Runs the plugin tests with lua or luajit (skips with a message when neither is installed).
set -euo pipefail
cd "$(dirname "$0")"
for l in luajit lua lua5.4 lua5.3 lua5.1; do
  if command -v "$l" >/dev/null 2>&1; then
    for f in ../bookx.koplugin/*.lua; do
      "$l" -e "assert(loadfile('$f'))"
    done
    "$l" main_load.lua
    exec "$l" run.lua
  fi
done
echo "no lua or luajit installed: plugin tests skipped"
