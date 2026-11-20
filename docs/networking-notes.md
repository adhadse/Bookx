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
  address**. Behind a proxy that is the proxy for every client. Bookx deliberately does not trust `X-Forwarded-For`
  (a spoofable header would let anyone dodge the throttle). If this becomes a problem, the fix is an explicit
  "trusted proxy addresses" setting, not trusting the header by default. Not done; this is a security change (§8).
* OPDS/download/cover links are root-relative (`/opds/...`). Hosting them under another public prefix needs the
  proxy to rewrite bodies; documented as unsupported. `OpdsConfig::prefix` exists but the server always uses `/opds`.
* The sync routes depend on the `/bookx` prefix being preserved. Never document a stripped-prefix setup.
* `Cache-Control: no-store` is set when users are configured (everything but `/healthcheck`).
* `/bookx/healthcheck` exists only when sync is on (`KoSync::owns`), `/healthcheck` always.
* No upload route larger than 1 MiB exists; downloads are streamed with `Range`.
* No `Host` check on the content server (the proxy should serve one name; the wiki shows a default-server catch-all).
  An `Origin`/`Host` check for mutations on the content server is the §8 expectation for listeners; sync PUT/POST use
  header accounts, not cookies, so browser CSRF does not apply. Revisit if cookies/sessions are ever added.

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
