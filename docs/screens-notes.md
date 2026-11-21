# Screens for engine-only features (wave 12)

Internal. Written by the screens agent. User pages: `wiki/docs/{preferences,devices-and-sharing,reverse-proxy,news-and-web,reading,library}.md`.

## Built

| Screen | Where | Logic (unit tested, no GTK) | Broadway scenarios |
|---|---|---|---|
| Library server accounts (Preferences → Sync → Advanced) | `src/ui/accounts.rs`, one line in `sync_prefs.rs` | `src/account_model.rs` | `accounts_screen`, `accounts_win11_fit` (`ui-tests-sync.sh`) |
| Hardcover (Preferences → Library) | `src/ui/hardcover.rs`, `register_group("library", 48)` | `src/hardcover_sync.rs`, `bookx_sources::hardcover` (rating, `test_connection`, `match_items`) | `hardcover_screen`, `hardcover_undo`, `hardcover_win11_fit` (`ui-tests-prefs8.sh`) |
| News: Edit Feeds and Site accounts | `src/ui/news_recipes.rs`, opened from `ui/news.rs` (press a subscription; *Sites with a login* group) | `src/news_recipe_form.rs`, `src/news_sites.rs` | `news_recipes_editor`, `news_sites_accounts`, `news_recipes_win11_fit` (`ui-tests-lib8.sh`) |
| Dictionaries: order, web sites, history | `src/ui/lookup_settings.rs` (three groups on the Dictionaries page, *Web* menu in the look-up popover) | `src/lookup_prefs.rs`, `dictionaries::load` reads the order | `lookup_settings_screen`, `lookup_win11_fit` (`ui-tests-prefs8.sh`) |

## Decisions

* **Accounts**: the screen only edits `ShareSettings::accounts` and `guest_collections`. The password is hashed with `keyhash::hash_key` on a worker thread
  (`gio::spawn_blocking`), the entries are emptied before the hash is made and nothing prints a password or hash. A change saves the settings and calls
  `share::reapply`, which restarts a running server (the server reads `ShareSettings` only at start; a live swap of `ServerConfig::accounts` would need a
  shared handle in `bookx-serve`, which is a cross-track change). Collections are **tags** (the server compares tags), so shelves and saved searches are not
  offered. *Allow guests* is a separate group, not inside the Add Account form, because guests belong to the server, not to an account. A guest with no
  collection would see everything in the engine, so an empty list means guests off.
* **Hardcover**: the import maps Want to Read → `to-read` tag, Currently Reading → Reading, Read → Finished, Did Not Finish → On Hold, and the 0-5 rating to
  the Calibre 0-10 scale. A Finished book is never taken back, a started book is never made New. It reuses `tracker_sync::apply` (ratings and tags are one
  logged step) and stores the old statuses for *Undo*. The fetch is a Jobs panel job (indeterminate, not cancellable: one request per 100 books).
* **News site accounts**: HTTP Basic only. `SiteAuth` wraps the guarded fetcher and adds the header for an `https` URL whose host equals the account's host
  (no sub-domains, no `http`); ureq's default does not forward `Authorization` on redirects. Passwords are `secret_fields` entries `news-site-<host>`; *Remove*
  calls `Vault::forget`. Add-from-the-web and the feed-name button still use the plain fetcher (only the scheduled/manual news downloads use the accounts).
* **Dictionary order** is stored in `priority.txt` inside the dictionaries folder (not in `config.json`): it travels with the dictionaries and the CLI sees it.
* **Web sites** use the engine's `{word}` placeholder (`{query}` is accepted and rewritten). The host part must be literal text.

## Caveats and not done

* Hardcover and Basic-auth sites were never tried against the real services (mock fetcher only). The GraphQL field `user_books.rating` is from the public schema.
* The a11y audit and the 900x600 / 1.5x fits are tested in Broadway; **no screen reader and no real GNOME session run** was done (`build-aux/session-tests.sh`).
* Cookie or form logins for paywalls, per-feed accounts and an account that sees *shelves* instead of tags: not built.
* The schedule is not editable in the Edit Feeds view (it stays in *Edit* of the subscription; the view shows it).
* The `{word}` and `{title}` texts in a few msgids are literal text for translators.
* `po/bookx.pot` is **not** regenerated (instruction: no po edits): `i18n::tests::the_template_lists_every_marked_string` fails until `build-aux/i18n.sh` runs.
* Wiki build (`build-aux/wiki.sh`) could not run here (no Zensical); links were checked by hand.

## Other engine features found without a screen

Found by grepping every `bookx_lookup::*`, `bookx_sources::*` and `bookx_serve::*` module against `src/`, and the roadmap and `AGENTS.md`. Nothing here was built.

* `bookx_sources::identifiers` (typed identifiers: ISBN, ASIN, DOI, ... with validation and links) is not used by `src/`; the edit dialog only has the plain identifier text field.
* `bookx-editing` CSS transforms: `AGENTS.md` says they are not in the editor UI yet.
* Roadmap rows that are partial for lack of UI: R31 (editing the text of a highlight), F6 (automatic last-position bookmark). `bookx_lookup::bookmap` is reached through other paths, not by that module name.
* The web client (`bookx_serve::pwa`) has no settings, only the server serving it.
