# Serve and sources: what is built, what is not (2026-11-21)

Internal. Written by the T6/T8 agent. User pages: `wiki/docs/{reverse-proxy,devices-and-sharing,news-and-web,command-line,preferences}.md`.

## Built

| Item | Where | Tests |
|---|---|---|
| Trusted proxy addresses | `net.rs` (`ProxyNet`, `client_ip`, `Limits::trusted_proxies`), `--trusted-proxy`, `ui/proxy_settings.rs` | unit, `tests/hardening.rs`, cli, Broadway `sync_proxy_settings` |
| `OpdsConfig::prefix` used, `public_prefix` | `server.rs`, `opds.rs` (`base`) | unit, cli `serve_accounts_guest_prefix_and_web_client` |
| Allowed host names UI | `ui/proxy_settings.rs`, `ShareSettings::allowed_hosts`, `hostguard::parse_host_list` | unit, Broadway |
| QR code of the sync / OPDS address | `bookx-serve/src/qr.rs` (own encoder, versions 1-40, L/M/Q/H), `ui/qr_dialog.rs` | format/version words and capacity table from the standard, RS vector from the standard, recorded symbols decoded by zxing-cpp 3.1.1 (v1, 3, 5, 7, 10, 18, 26, 40 also decoded when written); Broadway `sync_qr_code` |
| Wallabag import (P19) | `bookx-sources/src/wallabag.rs`, `web_import::import_wallabag`, `ui/wallabag.rs` (opened from Add from the Web), `Fetcher::post` | unit, `tests/wallabag_live.rs` (loopback stub, SSRF), Broadway `wallabag_import` |
| Per-feed full article (S5) | `FeedSpec::full_article`, `news::split_feed_line` (`url full|summary`) | unit |
| Per-field source priority (S6), name order | `source_priority.rs`, `ui/source_settings.rs`, `sources::{Field::ALL, FieldPriority::overlay}`, `authors::NameOrder` | unit, Broadway `sources_priority_ui` |
| Wikipedia as a book (S8) | `bookx-sources/src/wikipedia.rs` (`wikipedia:Title` in Add from the Web) | unit |
| Hardcover (read only) | `bookx-sources/src/hardcover.rs`, token in `Config::secret_fields` | unit with a stub fetcher; **not** tried against the real service; **no screen, nothing applied to the library yet** (`hardcover::plan` only says what would change) |
| Web client (PP-G-26/27) | `bookx-serve/src/pwa.rs` + `pwa/` | unit (CSP, no inline/foreign content), cli; **not run in a real browser** (JS only syntax-checked with node) |
| Accounts and guest (PP-C-10) | `bookx-serve/src/accounts.rs`, `--account`, `--guest-collection`, `ShareSettings::{accounts, guest_collections}` | unit, cli |
| Check before send in the Device window | `ui/devices.rs::send_checked` | Broadway `meta_preflight_device` |

Collections are the tags of a book (the library has no other group a server could name). Accounts have no screen yet: the settings file holds
`accounts: [{name, hash, collections}]` (hash from `keyhash::hash_key`); the CLI hashes for you.

## Not built, with reasons

* **S4 recipe editor UI / accounts for news** (`ui/news_recipes.rs`): not started. The subscription form already edits the common fields; a recipe editor needs
  fields for selectors, include/exclude, image limits and, for accounts, a login flow for paywalled feeds (cookies/headers), which needs a design decision
  (where the secret lives, what is sent to which host).
* **P7 direct MTP**: gvfs mounts work (`devices` accept `/run/user/.../gvfs/mtp:...`); libmtp would add a C dependency and `unsafe` FFI, against
  `#![forbid(unsafe_code)]` in the library crates. Documented as "through gvfs only" on the wiki.
* **Kindle native reading state**: the position of a Kindle book lives in binary sidecar files whose layout is neither public nor stable between
  firmware versions; nothing here can be verified without a device. Not feasible without a real Kindle and a format reference. KOReader on a Kindle works.
* **Unifying `devices::full_sync_plan` with the `device_sync` plan**: not done. The KOReader state is written through the safe paths that exist
  (`look_sync`, `kosync_sync`, `annotation_sync_ui`); merging two planners touches device files and has no hardware to try on. Needs a design of its own.
* Hardcover screen, apply step and a real-service check; accounts screen; the web client in a real browser (install prompt, offline); translation of the web client.
* The wiki build (`build-aux/wiki.sh`) could not be run here (the environment has no Zensical and `wiki/site` assets were missing); links were written by hand.

## Open checks

* Real Wallabag server (the API paths and the OAuth password grant are from its documentation).
* Real Hardcover GraphQL schema (`me.user_books`), status ids 1, 2, 3, 5.
* `/app/` in Firefox and Chromium: service worker registration under a sub-path, `Service-Worker-Allowed`, the Basic-auth prompt on first load.
