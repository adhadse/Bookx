> Archived 2026-11-23: Finished wave log; every row is addressed. Living document: [remaining.md](../remaining.md).

# Secondary windows: wave 9 notes

Internal notes for `docs/human_interface_backlog.md` §5.7 (WIN-L*, X*, XC-1/2/5/7). Not user documentation.

## What changed

- **Lint** (`src/ui/i18n_lint.rs`): raw literals in `set_description`, `set_title`, `set_subtitle`,
  `set_placeholder_text`, `set_tooltip_text` and `set_heading` fail `cargo test lint`; `tr(CONST)` needs the
  constant to be declared with `N_("…")`. `REPORT_ONLY` is empty.
- **Dialog helpers** (`src/ui/dialog_toast.rs`): `show_plain`, `show_action` (a toast with its own button),
  `overlay_of` (the overlay a widget sits in, else the window's), `app_overlay` (the window's overlay for code that
  only has the plain toast callback: the Calibre export and the annotation export use it for *Open Folder*).
- **Audit** (`src/ui/audit.rs`): `a11y-audit:` also prints `dialog_fit:` for the open dialog.
- **Windows**: Reading Trackers (preview), Audiobooks (worker thread, per-row ticks, status page), Online Catalogues
  (search bar, Cancel, Try Again), Vocabulary (header buttons, undo of removals), Import Highlights review, Book Map
  size and keyboard note card, News (undo of a deleted subscription), Duplicates (undo of a merge).

## Caveats

- Project rule for this wave: no label ends in an ellipsis. WIN-L1, X3, WIN-R1 and WIN-E6 ask for "…"; they were
  decided by default as "no ellipsis", the backlog rows say so.
- Toasts with an *Open Folder* button (`calibre_export.rs`, `annotations.rs`) are not covered by a scenario: the
  toast is not readable from a dev hook. The code path is `dialog_toast::show_action`.
- `library:` dev commands reach the library component a little later than app-level commands (`a11y-audit:`,
  `dialog:ctrlw`, `quit`): put a `wait:` after them, or the app-level command runs first.
- Text scaling in Broadway comes from `gtk-4.0/settings.ini` (`gtk-xft-dpi=147456` is 1.5x); the scenario name must
  start with `win9_scale` so `ui-tests.sh` writes it.
- The undo of a duplicate merge takes back every operation the merge wrote (`duplicates::undo_since`); a merge of
  formats (`book_formats::merge`) uses the same path and was not given its own unit test.
- A `.po` refresh is still needed: new msgids (see the wave 9 report) and the changed ones ("{} terms added, {}
  updated", "Your Definition", "Export Your Definitions as CSV/JSON") have no translation yet.

## Skipped (owned elsewhere)

WIN-L1, WIN-L8, WIN-L13, WIN-L15a, WIN-L16, WIN-L18, WIN-L19, WIN-R1/R3/R7/R8, WIN-E1-E6, XC-6, X7 and the
Statistics rows of WIN-L4: they live in `library*.rs`, `app.rs`, `health.rs`, `preferences.rs`, `stats.rs`,
`sync_window.rs`, `reader*.rs` or `editor/**`.
