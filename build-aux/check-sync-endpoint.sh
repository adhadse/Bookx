#!/usr/bin/env bash
# The sync endpoint is `/bookx` only. Fails when user-facing text (wiki, README, the KOReader plugin,
# the runbook) mentions the removed `/kosync` address, or when the wiki page lacks the account section.
set -euo pipefail
cd "$(dirname "$0")/.."
if grep -rnI -- '/kosync' wiki/docs README.md koreader-plugin/bookx.koplugin koreader-plugin/README.md docs/kobo-koreader-sync-runbook.md; then
    echo "check-sync-endpoint: '/kosync' must not appear in user-facing text (use /bookx)" >&2
    exit 1
fi
page=wiki/docs/koreader-sync.md
for needle in 'Which user name and password?' 'not** the OPDS server password' 'Tools → Progress sync → Custom sync server' 'one-way hash' 'USB cable only' 'http://<host-ip>:<port>/bookx'; do
    grep -qF -- "$needle" "$page" || { echo "check-sync-endpoint: $page lacks: $needle" >&2; exit 1; }
done
echo "sync endpoint text ok"
