# Validator notes

`bookx_editing::validate` is checked against **epubveri** (<https://github.com/veripublica/epubveri>, a pure-Rust
EPUB validator, AGPL-3.0, used only as an external test tool, see `NOTICE.md`) in
`crates/bookx-editing/tests/epubveri_oracle.rs`. No Java is involved.

Install (user-local, no system changes):

```bash
cargo install epubveri --root ~/.cache/epubveri
```

The test looks for the binary in `$BOOKX_EPUBVERI`, then `epubveri` on `PATH`, then `~/.cache/epubveri/bin/epubveri`.
When none is found the comparison is skipped cleanly (the fixture-verdict tests still run). `BOOKX_ORACLE=0` skips it
explicitly; `BOOKX_ORACLE_KEEP=1` keeps the generated EPUBs in the temp dir for inspection. The cases run in parallel.

Last run: epubveri 0.40.1, 61 corpus cases (valid books and single-fault books). 58 agree.
Three differ, and in all three Bookx is **stricter** (Bookx fails the book, epubveri passes it). There is no case where
epubveri fails and Bookx passes.

| Case | epubveri 0.40.1 | Bookx | Decision |
|---|---|---|---|
| HTM-902 document without `<title>` | warning RSC-017 only | error | keep: the title is what reading systems show |
| NCX-902 navPoint without `playOrder` | no message | error | keep: required by the NCX 2005-1 DTD |
| NCX-904 empty navPoint label | no message | error | keep: an empty TOC entry is unusable |

These are listed in `DIFFERENCES_FROM_EPUBVERI` in the test and skipped in the comparison. Everything else agrees,
including HTM-907 (`<img>` without `alt`) and PKG-022 (deflated `mimetype`).

epubveri documents its own departures from W3C's epubcheck in its `AGENTS.md` ("Known differences"). If a retailer
requires a pass by that specific tool, run it as well; passing epubveri is not identical to passing it.

## Extended checks (`BKX-xxx`, `ACC-9xx`)

`validate_with(archive, &Options { extended, accessibility })` adds the editor's checks (`validate/extra.rs`, rule table
`EXTRA_RULES`). `validate()` alone is unchanged, so the corpus above is not affected. They are ERRORs only where
epubveri 0.40.1 also fails the book for the same fault (found with `the_extended_rules_do_not_contradict_epubveri` in
`validate/tests_extra.rs`, which compares verdicts on one fixture per rule): `BKX-003` (RSC-028), `BKX-004` for
documents without an XHTML 1.x document type (RSC-016; a warning where the DTD defines the entity), `BKX-010`
(CSS-008), `BKX-018` (RSC-020), `BKX-020` (RSC-030), `BKX-023` (RSC-010), `BKX-027`. All others are warnings or
usage hints. The editor lists them with the quick checks of `bookx-core` in `bookx_editing::problems`; the
quick-check messages are mapped to validator ids and dropped when the validator reports the same id for the file
(extra ids `CHK-001..003` for author, cover and unreadable package).
