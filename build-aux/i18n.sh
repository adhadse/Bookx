#!/usr/bin/env bash
# Rewrites po/bookx.pot from the strings marked with tr("…") / N_("…") in src/, then reports what
# each language file still lacks. New language: copy po/bookx.pot to po/<code>.po, translate it, and
# add it to the table in build.rs. Compiled .mo files (msgfmt, Weblate) in a locale directory
# (BOOKX_LOCALE_DIR, /usr/share/locale, /app/share/locale) override the embedded ones.
set -euo pipefail
cd "$(dirname "$0")/.."
BOOKX_UPDATE_POT=1 cargo test -p bookx --quiet i18n::tests::the_template_lists_every_marked_string
cargo test -p bookx --quiet i18n::tests::every_shipped_language
