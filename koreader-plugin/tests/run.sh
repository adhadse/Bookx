#!/usr/bin/env bash
# Runs the Bookx Sync plugin tests with every Lua interpreter that is installed (KOReader runs LuaJIT, so
# luajit is the one that matters; plain lua 5.x catches code that only works by luck). Skips with a message
# when none is installed. `LUA=luajit run.sh` runs only that one. Each test file is independent, so they
# run in parallel.
set -euo pipefail
cd "$(dirname "$0")"
if [ -n "${LUA:-}" ]; then
  interpreters=("$LUA")
else
  interpreters=()
  for l in luajit lua5.1 lua5.4 lua5.3 lua; do
    command -v "$l" >/dev/null 2>&1 && interpreters+=("$l")
  done
fi
if [ ${#interpreters[@]} -eq 0 ]; then
  echo "no lua or luajit installed: plugin tests skipped"
  exit 0
fi
# the same interpreter twice under two names (lua -> luajit) is run once
declare -A seen
status=0
for l in "${interpreters[@]}"; do
  real=$(readlink -f "$(command -v "$l")")
  [ -n "${seen[$real]:-}" ] && continue
  seen[$real]=1
  echo "== $l ($("$l" -v 2>&1 | head -1))"
  for f in ../bookx.koplugin/*.lua; do
    "$l" -e "assert(loadfile('$f'))" || status=1
  done
  out=$(mktemp -d)
  for t in run.lua main_load.lua scenarios.lua; do
    ( "$l" "$t" >"$out/$t.log" 2>&1; echo $? >"$out/$t.rc" ) &
  done
  wait
  for t in run.lua main_load.lua scenarios.lua; do
    sed "s|^|[$t] |" "$out/$t.log"
    [ "$(cat "$out/$t.rc")" = 0 ] || { echo "FAILED: $t with $l"; status=1; }
  done
  rm -rf "$out"
done
exit $status
