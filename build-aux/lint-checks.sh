#!/usr/bin/env bash
# Fails when a UI scenario reads `$?` after a command substitution in the label of `check`:
#     cmd; check "label $(other)" $?      # $? is the status of $(other), always 0: the check is vacuous
# Capture the status first:  cmd; rc=$?; check "label $(other)" $rc
cd "$(dirname "$0")" || exit 2
bad=$(grep -nE 'check "[^"]*\$\(.*" \$\?' ui-tests*.sh session-tests.sh 2>/dev/null)
if [ -n "$bad" ]; then
    echo "lint-checks: \`check \"…\$(…)…\" \$?\` is always true; use rc=\$? before check:" >&2
    echo "$bad" | cut -c1-200 >&2
    exit 1
fi
echo "lint-checks: ok"
