# Spelling panel, Reports window, special characters (internal notes)

Date: 2026-11-13. Rows CE-270..CE-281, CE-300..CE-311 of `docs/calibre-editor-gap-analysis.md`. Public docs: `wiki/docs/editing.md`.

## Where things are

| Piece | File | Notes |
|---|---|---|
| dictionaries by full tag, `canonical_tag` | `crates/bookx-editing/src/spell.rs` | `SpellChecker` is `Clone` (dictionaries in `Arc`), so a scan can run on a worker thread |
| book-wide word index, exclusions, Change All | `spell_session.rs` | `scan_sources` takes owned text, not the `OpenBook` |
| named user dictionaries | `spell_user.rs` | Personal = old `spelling-words.txt` |
| Hunspell / `.oxt` import | `spell_import.rs` | only UTF-8 `.aff`/`.dic` (spellbook limit) |
| report tables, CSV | `reports_tables.rs` | rows carry places (`Loc`), capped at 200 per row |
| Unicode names, blocks, Alt+X, entities | `charinfo.rs` | `unicode_names2` (MIT/Apache, pure Rust) because names must work offline without a data file |
| glue | `src/editor_spell.rs`, `editor_spell_dicts.rs` (folders, per-book prefs), `editor_chars.rs` (favourites) | |
| UI | `src/ui/editor/{spelling,spelling_dicts,reports,special_chars}.rs` | dev hooks in each file (`dev_*`) |

## Decisions and caveats

* **F8 conflict:** the Problems panel owns F8 / Shift+F8 (CE-163). Spelling uses Ctrl+F8 / Ctrl+Shift+F8. Calibre uses F8 for spelling and Ctrl+F7 for problems.
* Ctrl-click in the special character window keeps it open (owner brief); a plain click inserts and closes.
* Entity typing knows only the names in `markup::named_entity` (about 30) and numeric references. Setting is off by default.
* Ignore (session) is lost when dictionaries are imported (the checker is rebuilt); *Ignore in Book* survives.
* Per-book data is keyed by the library book id (`spelling-books/<id>.json`).
* Scans copy the text of the book once (worker thread); results are dropped when a newer scan was started.
* Reports tabs are built on first view, synchronously on the UI thread (budget test: all tabs of a 5 MB book under 1.2 s each in release; measured well below).
* Not done: grammar (CE-281, not planned); report window has no per-row context menu; words with `lang` regions are shown with the full tag; the spelling panel has no keyboard shortcut to toggle (menu only).
* Not tested on real hardware or with real system dictionaries (the tests use tiny fixtures); the Alt+X real-session test used Mutter's headless session.

## Tests

* Rust: `charinfo` (5), `spell` (+2), `spell_session` (8), `spell_user` (4), `spell_import` (5), `reports_tables` (10), `editor_spell` (+4), `editor_spell_dicts` (2), `editor_chars` (3), `editor_prefs`, perf budgets (3).
* UI scenarios: `editor_live_spellpanel`, `editor_live_spellpanel_scope`, `editor_live_spelldicts`, `editor_live_a11y_spelling`, `editor_reports`, `editor_chars`.
* Real session: `editor_reports_double_click` (real pointer: Links tab click, double-click on a link row and on a file row), `editor_alt_x` (real keys).
