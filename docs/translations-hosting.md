# Where translations are hosted and reviewed

Internal note (decision record). User-facing text is `wiki/docs/translating.md`.

Status: living | Last reviewed: 2026-11-23 | Owner: @adhadse

## Contents

- [Decision](#decision)
- [What was compared](#what-was-compared)
- [Options](#options)
- [How it fits Bookx](#how-it-fits-bookx)
- [Setting up the Weblate project (5 minutes, owner)](#setting-up-the-weblate-project-5-minutes-owner)
- [Review gate for machine translations](#review-gate-for-machine-translations)
- [CI](#ci)
- [Open items](#open-items)

## Decision

**Weblate on hosted.weblate.org (the free "libre project" hosting), project `bookx`, one gettext component `bookx`
(template `po/bookx.pot`, files `po/*.po`), connected to the GitHub repository, changes coming back as pull
requests. Pull requests that edit `po/<code>.po` directly stay welcome; Weblate picks them up through its webhook.**

The translations in the repository are written by an AI agent and carry no credit (owner decision, 2026-11-21).
Weblate's review workflow is the human gate: a string counts as *reviewed* only after a native speaker approved it.
`po/STATUS.md` records the state per language (`machine-translated` → `reviewed`) and CI keeps it in step with
`po/LINGUAS`.

## What was compared

Checked on 2026-11-21 against the repositories of apps in GNOME Circle and on Flathub:

| App | Where its translations live | Evidence |
|---|---|---|
| Mousai (Circle) | hosted.weblate.org, project `seadve`, badge and "Help translate" in README; `po/LINGUAS`, `po/POTFILES.in`, `po/meson.build` with `i18n.gettext(preset: 'glib')` | github.com/SeaDve/Mousai README, `po/` |
| Komikku (Circle) | hosted.weblate.org/engage/komikku | codeberg.org/valos/Komikku README |
| Tuba | Weblate on translate.codeberg.org | GeopJr/Tuba README |
| Foliate | plain `.po` files in the repository (`po/LINGUAS` lists ~30 languages), `po/POTFILES` lists the `.js`/`.ui` files | johnfactotum/foliate `po/` |
| Hieroglyphic (Circle) | GNOME Damned Lies (l10n.gnome.org) | FineFindus/Hieroglyphic README ("translated on the GNOME translation platform Damned Lies") |
| Apps in GNOME Core/World on gitlab.gnome.org | Damned Lies, per-language teams push to the module | l10n.gnome.org |

Flathub has **no translation requirement and no recommended platform**: the manifest only needs the app to build;
a Flatpak splits `share/locale` into an automatic `*.Locale` extension, which users must have installed to see the
translations. Bookx does not depend on that: the `.po` files are compiled into the binary (`src/i18n.rs`).
Neither GNOME Circle nor Flathub prescribes a platform (not verified against the current Circle criteria text: check
before applying); the examples above show both are common.

## Options

| | Weblate (hosted.weblate.org) | GNOME Damned Lies (l10n.gnome.org) | Transifex / Crowdin | Plain PRs only |
|---|---|---|---|---|
| Eligible | any libre project (apply once, free) | only modules in GNOME core/World/Circle on gitlab.gnome.org (Bookx is on GitHub) | free for open source on request; proprietary platforms | always |
| Translator workflow | web editor, suggestions, glossary, machine-translation hints, checks (placeholders, plurals, Rust `{}` via flag `rust-format`) | web, teams per language, a coordinator uploads `.po` files by hand | web editor | edit a `.po` locally (Poedit/Lokalize), open a PR |
| Review and ownership | built-in: suggestions, "needs editing", **approved** state with a reviewer group, per-language ACL | team coordinator reviews, per-language team | review step exists on paid/OSS plans | the maintainer reviews every PR |
| Automatic PRs | yes (GitHub PR per push, or push to a branch), webhook in both directions | no (module maintainer's CI imports) | yes via GitHub integration | – |
| Licensing | project declares a licence; contributions under it (Weblate shows it, can require a contributor agreement) | GPL-style by GNOME policy | per project | per PR |
| Effort for the owner | ~5 min to create, 1 GitHub token or webhook | cannot be done today (needs a GNOME Foundation module, a move to gitlab.gnome.org) | similar to Weblate, but closed source | none, but no progress view, no reviewers |
| Fits our setup | gettext `.po` + `.pot` are exactly its default | needs `POTFILES.in`, meson `i18n.gettext`; Bookx has neither build system nor GTK i18n | needs its own config; no gain | works today |

Rejected:
- **Damned Lies**: not available for a GitHub-hosted app outside the GNOME modules; also couples us to a meson/gettext
  build the project does not have (Bookx uses its own embedded loader).
- **Transifex / Crowdin**: proprietary services (free tier for open source only on request), no advantage over
  Weblate for a gettext project, and neither is what GNOME Circle or Flathub apps use for libre projects.
- **Plain pull requests only**: stays as the fallback and remains supported, but gives no review state, no progress
  numbers and no glossary, which matters because the only translators today are agents.
- **Self-hosted Weblate**: an operating cost for no gain at this size.

## How it fits Bookx

- The source of strings is `po/bookx.pot`, written by `build-aux/i18n.sh` (the app's own scanner, in
  `src/i18n.rs`). Weblate never extracts: it reads the committed `.pot`. A change that adds a string commits the new
  `.pot` (CI fails otherwise) and Weblate's add-on "Update PO files to match POT" brings the languages up to date.
- Weblate writes `po/<code>.po`. The loader (`parse_po_raw`) understands what Weblate/msgmerge write: wrapped
  lines, `#:` locations, `#,` flags (`fuzzy` strings are skipped, so a string "Needs editing" shows English), `#|`
  previous strings and `#~` obsolete entries (ignored).
- Languages in `po/LINGUAS` are the ones Weblate may create/keep; a language reaches the app only when it is also in
  the table in `build.rs` (it generates `LANGUAGES` in `src/i18n.rs`) and embedded with `include_str!`. CI (`ci.yml` (Rust job)) checks that both lists agree.
- Compiled `.mo` files in `/usr/share/locale`, `/app/share/locale` or `BOOKX_LOCALE_DIR` override the embedded text
  (distribution packagers, Weblate's "Generate MO files" add-on if ever wanted). Not needed for the normal flow.
- GNOME-style `po/LINGUAS` and `po/POTFILES.in` exist so any external tool can work with the tree. `POTFILES.in` is
  generated (`build-aux/gen-potfiles.sh`, checked in CI); `build-aux/i18n-xgettext.sh` extracts with GNU
  `xgettext -L Rust` and compares.

### Extraction parity (xgettext vs. the app's scanner)

Command: `xgettext -L Rust --keyword=tr:1 --keyword=trf:1 --keyword=N_:1 --keyword=ntr:1,2 --keyword=tr_c:1c,2
--files-from=po/POTFILES.in`. Result on 2026-11-21: 3328 messages from xgettext, 3340 in `po/bookx.pot`, with the
same context and plural handling. The remaining differences were not extraction bugs:

- The `.pot` was behind the source (other work was changing strings that day; about 40 on each side, e.g.
  `Cancelling` vs `Cancelling…`). CI's pot check catches this.
- The app's scanner also reads strings in comments and `#[cfg(test)]` code (a `tr("Backup")` in a doc comment is in
  the template); xgettext ignores comments. A string only in a comment ends up as an untranslatable entry:
  keep markers out of comments.
- xgettext warns about URLs inside three messages (harmless).
- `tr_c(ctx, id)` extracts correctly with `1c,2`; Weblate shows the context.

## Setting up the Weblate project (5 minutes, owner)

1. Sign in at https://hosted.weblate.org with GitHub, then **Add new translation project** → apply for the free libre
   hosting (https://hosted.weblate.org/hosting/): name `Bookx`, slug `bookx`, website https://bookx.adhadse.com,
   repository https://github.com/adhadse/Bookx, licence of the project (the one in `LICENSE`).
2. **Add component** with these settings (the same are in `weblate/component.yml`; a project admin can paste them or use
   `wlc`/the API):
   - Component name `bookx`, slug `bookx`, VCS **GitHub** (pull requests), repository
     `https://github.com/adhadse/Bookx.git`, branch `main`, push branch `weblate-translations`.
   - File format **gettext PO file**, file mask `po/*.po`, **template for new translations** `po/bookx.pot`,
     monolingual base language file *empty* (bilingual, source text is the key), new base `po/bookx.pot`, edit base
     file **off**.
   - Language code style: *POSIX style using underscore as a separator* (`de`, `pt_BR`).
   - Translation flags: `rust-format` (checks `{}`), and keep the default checks. Source language English.
   - Merge style **rebase**, commit message `i18n: update {{ language_name }} translation`, committer Weblate.
   - **Enable reviews** (dedicated reviewers), **suggestion voting** off, "Autoclean" off, licence same as the project.
   - Add-ons: *Update PO files to match POT (msgmerge)*; *Cleanup translation files* (drops obsolete entries); *Customize gettext output*:
     no line wrapping, no locations (matches the existing files); *Squash Git commits* by file; skip *Generate MO
     files* (the app embeds the `.po`).
3. In the GitHub repository add the webhook `https://hosted.weblate.org/hooks/github/` (push events), and give Weblate
   access to open PRs (Weblate shows the GitHub App/token step when you pick the GitHub VCS).
4. **Import the agent translations as translated, not approved.** The component's *Review* settings stay on, so
   nothing is *approved* until a human does it; the approved percentage is the "reviewed" number. Native speakers
   are invited with the link `https://hosted.weblate.org/engage/bookx/`.
5. Put the translation badge into the README (`https://hosted.weblate.org/widget/bookx/bookx/svg-badge.svg`) and link the
   engage page from `wiki/docs/translating.md` (section Weblate: it now says the project page "will be linked here").

Repository files for this: `.weblate` (settings for the `wlc` command-line client), `weblate/component.yml` (the
list above in machine-readable form), `po/LINGUAS`, `po/POTFILES.in`, `po/STATUS.md`, `po/GLOSSARY.md`
(import it into Weblate as the project glossary: Manage → Glossaries → Upload, convert the tables to CSV
`source,target` per language when the project exists).

## Review gate for machine translations

| State | Where recorded | Meaning | Shown to users |
|---|---|---|---|
| `machine-translated` | `po/STATUS.md`, wiki table | written by an agent, no native review | yes: wiki status table, header comment of the file |
| `reviewed` | `po/STATUS.md`; Weblate "approved" percentage | a native speaker approved the strings (or a PR from one) | yes |

Rules:
- An agent may add or update strings in any language; the state stays `machine-translated`.
- Only a pull request that names the human reviewer (or a Weblate approval at 100 % of the file) changes the state to
  `reviewed`; the owner edits `po/STATUS.md` in that change.
- A correction from a human to a `reviewed` language does not change the state; an agent change to a `reviewed`
  language sets it back to `machine-translated` for the strings it touched (note it in `docs/archive/translation-review.md`).
- The reading app does not show the state; it is a documentation fact, so a bad string is not hidden from users.

### Promoting a language to reviewed (the Flathub gate)

Flathub does not accept machine-generated translations, so its build embeds **only** the languages that a human
reviewed (cargo feature `reviewed-locales-only`, `build.rs`, list in `po/reviewed-locales.txt`, empty today = English
only). Every other build keeps all of `po/*.po`. To promote a language, for example `de`:

1. A native speaker reviews it: on Weblate every string reaches *approved* in the review workflow (100 % approved),
   or the speaker sends a pull request and says in it that they read the whole file.
2. One pull request, by the owner: set the row of `po/STATUS.md` to `reviewed` (name the reviewer in the notes) **and**
   add the code on its own line to `po/reviewed-locales.txt`. Only a complete language can be listed: a test
   (`i18n::tests::reviewed_locales_exist_and_are_complete`) fails for an unknown code, a missing `po/<code>.po` or
   missing strings, and `the_language_table_follows_the_gate` checks the reviewed-only build embeds exactly English plus
   the listed codes.
3. Release as usual; the next Flathub update carries the language. If an agent later changes strings of that
   language, set it back to `machine-translated` and remove the code from the list in the same change.

Checks: `cargo test -p bookx i18n` and `cargo test -p bookx --features reviewed-locales-only i18n` (CI runs both,
`.github/workflows/ci.yml`); `REVIEWED_LOCALES=1 build-aux/flatpak.sh` builds the Flatpak the way Flathub does.
The desktop file and metainfo contain no translated keys on purpose (a test checks it); if translations are added there
later they follow the same list.

## CI

`.github/workflows/ci.yml` (never run before this note was written): the template is up to date
(`BOOKX_UPDATE_POT=1 cargo test … the_template_lists_every_marked_string` then `git diff --exit-code po/bookx.pot`),
`po/POTFILES.in` is current, every shipped language passes `every_shipped_language_translates_every_marked_string`,
`po/LINGUAS` = the language table in `build.rs`, `po/STATUS.md` matches, each `.po` passes `msgfmt --check-format` (the only complaints of a full `--check` are the missing
`Last-Translator`/`Language-Team`/`PO-Revision-Date` header fields, which Weblate fills in).
Anything that adds a string must regenerate `po/bookx.pot` and `po/POTFILES.in` and fill de and es (the "full"
languages) in the same change.

## Open items

- Owner: create the Weblate project (above), then replace the placeholder text in the wiki page.
- `msgfmt --check-format` on the existing files is clean (they only lack the optional header fields above).
- fr (7 %) and hi (5 %) stay starter files until an agent or a person completes them; the shipped `LANGUAGES` list
  does not distinguish partial languages (a partial language shows English for the rest).
