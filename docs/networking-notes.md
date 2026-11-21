# Networking notes (listeners, reverse proxy)

Internal. The user page is `wiki/docs/reverse-proxy.md`. Keep both in step when a listener changes.

## Listeners today (verified in code, 2026-11-20)

| Listener | Code | Default bind | Auth | Paths |
|---|---|---|---|---|
| Content server (app *Share Library*, `bookx-cli serve`) | `crates/bookx-serve/src/server.rs`, transport `net.rs` | `127.0.0.1` (`ServeArgs.bind`); the app maps "this computer only" / "all interfaces" to `127.0.0.1` / `0.0.0.0` (`src/sharing.rs`, no single-interface choice) | basic auth for everything except `/healthcheck` and the sync routes (own `x-auth-user`/`x-auth-key` account) | `/`, `/opds[/..]`, `/download/<id>.<ext>`, `/cover/<id>[/thumbnail]`, `/healthcheck`, `/bookx/{users,syncs,healthcheck}` and `/bookx/annotations/<md5>` |
| Sync (progress, annotations) | `kosync.rs`, `annotation_sync.rs`, mounted by `SYNC_PREFIX = "/bookx"` | same server | KOSync account | only under `/bookx`; `/kosync` and root-level routes are gone (test `legacy_kosync_and_root_routes_are_gone`) |
| MCP Streamable HTTP | `crates/bookx-mcp/src/http.rs`, `bookx-cli mcp --http ADDR:PORT` | none (stdio) unless `--http`; the docs say `127.0.0.1:8765` | bearer token (≥16 chars), `Origin` check (loopback or `--allow-origin`) | `POST /mcp` (also `/mcp/`, `/`), `DELETE` ends a session; single JSON bodies, no SSE |

`bookx-cli serve` has no annotation sync (`cfg.annotations` is only set in `src/sharing.rs`).

## Constraints a proxy must respect

* Transport limits (`net::Limits::default`): body 1 MiB (`413`), head 16 KiB, 100 headers, header 10 s, body 30 s, idle
  15 s, response base 30 s + 128 KiB/s minimum rate, 64 connections (`503`), 32 per peer, 1000 requests per connection,
  8 handler threads.
* The failed-credential throttle (`AuthThrottle`, 10 per 60 s, blocks 60 s with `429`) is keyed on the **socket peer
  address**. Behind a proxy that is the proxy for every client. `X-Forwarded-For` is believed **only** for connections whose
  socket peer is in `Limits::trusted_proxies` (`--trusted-proxy`, Preferences → Sync → Advanced → Trusted proxy addresses,
  `net::client_ip`): read from the right, trusted hops skipped, the first other address is the client, garbage stops the walk, `/0` is refused.
  Default empty = never trusted. (2026-11-21)
* OPDS/download/cover links are root-relative (`/opds/...`). Hosting them under another public prefix needs the
  proxy to rewrite bodies; **since 2026-11-21 `ServerConfig::public_prefix` (`--public-prefix`, the Sub-path row in Sync → Advanced)** puts the
  prefix in every link (`OpdsConfig::prefix = {prefix}/opds`, `OpdsConfig::base` for files and covers, `render_index_at`) and the server accepts requests with or
  without it (`strip_public_prefix`, whole segment only). `clean_public_prefix` refuses `..`, odd characters and the fixed routes (`/bookx`, `/opds`, ...).
* The sync routes depend on the `/bookx` prefix being preserved. Never document a stripped-prefix setup.
* `Cache-Control: no-store` is set when users are configured (everything but `/healthcheck`).
* `/bookx/healthcheck` exists only when sync is on (`KoSync::owns`), `/healthcheck` always.
* No upload route larger than 1 MiB exists; downloads are streamed with `Range`.
* **Host and Origin checks (2026-11-21):** every listener answers only to known `Host` names (`crates/bookx-serve/src/hostguard.rs`:
  IP literals, one-label names, `.local`, `.localhost`, `.home.arpa`, `.lan`, `.internal`, the machine name, plus `--allow-host` /
  `$BOOKX_ALLOWED_HOSTS`) and refuses a foreign `Origin` or `Sec-Fetch-Site: cross-site` on non-GET requests. A proxy that forwards a public
  name must be listed, or every request gets `403 Host not allowed`. The app has *Allowed host names* in Sync → Advanced (validated by
  `hostguard::parse_host_list`), next to the environment variable (Flatpak: `flatpak override --user --env=BOOKX_ALLOWED_HOSTS=...`).
* **Web client `/app/` (2026-11-21, `pwa.rs`)**: `/app/` (page) needs the login; manifest, icon, script, style and `sw.js` are public static text. Strict CSP
  (`default-src 'none'; script-src 'self'; ...`), no inline script, only `textContent`. The service worker only handles `/download`, `/cover`, `/opds`, `/app` of
  its own origin; `Service-Worker-Allowed` is the public prefix. Language: English only (not translated).
* **Accounts (`accounts.rs`)**: `ServerConfig::accounts` (Argon2id hash through `keyhash`, per-account collections = book tags) and `guest` (read only,
  listed collections). Filtering is on the snapshot (`access_of`), so catalogue, search, downloads and covers agree. Wrong credentials never become guest.
  The app keeps them in `ShareSettings::accounts` / `guest_collections` (hash only); no screen edits them yet, `bookx-cli serve --account/--guest-collection` does.

## Security checklist (docs/security.md §8) for this change

- [x] Untrusted bytes? No parser changed; documentation only.
- [x] Path from file/network? No.
- [x] Executes something? No.
- [x] Listener or user-supplied URL? No listener added or changed. The page recommends loopback bind, accounts on,
      no public OPDS/MCP, no body logging.
- [x] New dependency? No. Secret? No (examples use placeholders).
- [x] Widens a capability? No.
- [x] Regression test: n/a (docs). `build-aux/wiki.sh` checks links/anchors.

### Checklist to apply when a listener changes

1. Default bind stays loopback; a wider bind needs auth by default.
2. Update the table above and `wiki/docs/reverse-proxy.md` (paths, auth, limits).
3. Keep `/bookx` the only public sync prefix.
4. Test through a prefix-preserving proxy path (`crates/bookx-serve/tests/server_integration.rs` style) and the
   limits (413/431/429/503) in `tests/hardening.rs`.
5. Record the change in `docs/security.md` §10.

## Not verified

The proxy configurations (Caddy, nginx, Traefik, ufw, netplan, nmcli) were written from their documentation and not run;
no real KOReader device was tried against a proxy. Candidate test: Caddy/nginx in a container in front of
`bookx-cli serve --kosync` and `curl` the paths in the wiki page.
