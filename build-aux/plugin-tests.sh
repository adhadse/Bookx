#!/usr/bin/env bash
# The Bookx Sync plugin for KOReader: Lua tests (fake KOReader, scripted server) and, when cargo is available, the
# Rust test that runs the real plugin against the real sync server on localhost.
#   build-aux/plugin-tests.sh            all of it
#   NET=0 build-aux/plugin-tests.sh      no localhost server test
#   LUA=luajit build-aux/plugin-tests.sh only that interpreter
set -euo pipefail
cd "$(dirname "$0")/.."
koreader-plugin/tests/run.sh
build-aux/check-sync-endpoint.sh
if [ "${NET:-1}" != 0 ] && command -v cargo >/dev/null 2>&1; then
  cargo test -p bookx-serve --test koreader_annotations
fi
