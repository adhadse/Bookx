# Translation status

Review state of every shipped language. `build-aux/i18n-status.sh --check` (CI) fails when a language in
`po/LINGUAS` has no row here, has an unknown state, or is marked `full` while strings are missing.

- **machine-translated**: written by an AI agent. Nobody who speaks the language has read it yet. Shown as such
  on the wiki page *Translating Bookx*.
- **reviewed**: a native speaker went through the strings (on Weblate: *approved* in the review workflow, or a pull
  request from a native speaker). Change the state here in the same pull request that records the review.

Scope `full` = every marked string is translated (a test enforces it); `starter` = the most visible strings only,
the rest shows in English.

| Language | Code | Scope | Strings (snapshot 2026-11-22, 3735 total) | Review state |
|---|---|---|---|---|
| Deutsch | `de` | full | 3735 (100 %) | machine-translated |
| Español | `es` | full | 3735 (100 %) | machine-translated |
| Français | `fr` | full | 3735 (100 %) | machine-translated |
| हिन्दी | `hi` | full | 3735 (100 %) | machine-translated |

The snapshot count is informative; run `build-aux/i18n-status.sh` for the current numbers. Known problems found by
the last agent review are in `docs/archive/translation-review.md`; the glossary is `po/GLOSSARY.md`.

**Français (fr), 2026-11-21:** completed from `po/bookx.pot` (3725 messages, 100 % at the time; the template has 3735 messages on 2026-11-22 and `build-aux/i18n-status.sh` reports 100 %) by an AI agent; review state:
machine-translated (agent), unreviewed (the table keeps the state word `machine-translated` that `i18n-status.sh`
checks). Style: vous-form, sentence case, `’` apostrophes, `« »` quotes, narrow no-break space before `? ! ; :`,
key names (Ctrl, Shift, Alt, Esc, Enter, Backspace) kept as in the source, no ellipsis at the end of a label.
Two source entries (`...` and `... becomes …`) are literal examples of the autocorrect and stay as typed.

**हिन्दी (hi), 2026-11-21:** completed from `po/bookx.pot` (3725 messages, 100 % at the time; the template has 3735 messages on 2026-11-22 and `build-aux/i18n-status.sh` reports 100 %) by an AI agent; review state:
machine-translated (agent), unreviewed (the table keeps the state word `machine-translated` that `i18n-status.sh`
checks). Style: आप-form with polite imperatives, GNOME-style loanwords in Devanagari (लाइब्रेरी, हाइलाइट, बुकमार्क,
सिंक, डिवाइस, कन्वर्ट, शेल्फ़) following `po/GLOSSARY.md`, `{} मिलान` for "match", `रीडर` for the reading window,
no ellipsis at the end of a label, product names and key names kept. Plural rule kept as `n > 1` (the pot header says
`n != 1`; the two differ for 0). `translator-credits` is translated as itself. The entries `...` and `... becomes …`
are literal autocorrect examples and keep their dots. Needs a native-speaker read, especially the long help texts.
