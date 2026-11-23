# MCP server (enhancement 28): internal notes

Status: done for the scope below. User documentation: `wiki/docs/ai-assistants.md`. Code: `crates/bookx-mcp`,
`bookx-cli mcp` (`crates/bookx-cli/src/mcp.rs`), the app glue `src/mcp_host.rs` and the page `src/ui/mcp_settings.rs`.

Status: living | Last reviewed: 2026-11-23 | Owner: area agent, reviewed by @adhadse

## Protocol and SDK choice

* Revision implemented: **2025-11-25**; older clients are answered with their own version when it is one of
  2025-06-18, 2025-03-26, 2024-11-05 (the server only uses features common to all of them). Unknown versions get the
  latest. NOTE: modelcontextprotocol.io did not answer from the build machine when this was written (timeouts), so the
  implementation follows the published revisions as known from the specification text; re-check against the live
  spec when touching it (transports page: Streamable HTTP, `Mcp-Session-Id`, `MCP-Protocol-Version`, Origin rule).
* Transports: stdio (newline-delimited JSON-RPC, stdout carries only protocol messages) and Streamable HTTP (one
  endpoint `POST /mcp`, JSON replies, 202 for notifications, optional `Mcp-Session-Id`, `DELETE` ends a session,
  `GET` is 405 because the server never starts a stream).
* Methods: `initialize`, `ping`, `tools/list|call`, `resources/list|read|templates/list`, `prompts/list|get`,
  `logging/setLevel`; notifications `initialized`, `cancelled` (in), `progress`, `message` (out). Cursor pagination on
  all list methods (opaque base64 `bx1:<offset>`). Tool results carry `content` (text) and `structuredContent`;
  every tool has an `inputSchema`, an `outputSchema` and annotations (`readOnlyHint`, `destructiveHint`,
  `idempotentHint`, `openWorldHint`). Wave 11: `resources/subscribe`/`unsubscribe` (max 200 per session) and `notifications/resources/updated|list_changed` over **stdio only** (a watcher thread compares `(id, modified)` snapshots every `Config::watch_interval`, 2 s; the HTTP endpoint cannot push, so it advertises neither and refuses subscribe). `tools.listChanged` stays false: the tool list never changes during a session. Not implemented on purpose: completions,
  sampling/elicitation/roots (server to client requests), batching (removed from the spec in 2025-06-18), OAuth
  authorization (the bearer token is the HTTP authorization; a local token is what desktop clients support).
* **Own implementation instead of `rmcp`.** The official Rust SDK (`rmcp` 3.x, Apache-2.0, maintained) is solid but
  async (tokio, plus schemars/axum-style dependency trees for its HTTP transport); this workspace is blocking code and
  already has a hardened blocking HTTP server (`bookx-serve::net`: size limits, timeouts, connection cap, auth
  throttle, TLS) that the share server uses. The subset needed (JSON-RPC 2.0 + ~10 methods) is about 400 lines on
  `serde_json`, adds no dependency, and lets tools run on plain threads with cooperative cancellation. If the surface
  grows (sampling, subscriptions, streaming), switching to `rmcp` is the way; the tools are independent of the
  protocol layer (`tools::call(app, name, args, ctx)`).
* Cancellation: stdio reads on its own thread, so `notifications/cancelled` sets the flag of a running or queued
  request; a cancelled request gets no answer (as the spec allows). Tools poll `Ctx::should_stop()` (cancel or the
  30 s `call_timeout`); `search_in_book` hands the flag to `bookx-lookup`'s search. HTTP: cancel arrives on another
  connection and finds the flag through the session id.
* Progress: only when the request has a `progressToken`; sent by `search_in_book`, `find_duplicates`,
  `get_annotations` (large libraries), `fetch_metadata`. **Not delivered over HTTP** (single JSON response, no SSE
  stream), same for log notifications.

## What the server touches

* Read side: `Catalog` follows `bookx-library.db` like the share server (`data_version`), opening the store without
  the writer lock, so it works beside a running Bookx. Text comes from the EPUB (`OpenBook`), chapter by chapter,
  canonical text = block texts joined by `\n` with whitespace collapsed (same convention as the reader's
  `canonical_text`, so highlight offsets agree in the common case); cached for the last 4 books.
* Write side (`App::with_library`): per write, load a fresh `Library` (`open_library`), apply the change, `save`,
  drop. `WritePolicy::Locked` (CLI `--allow-write`) takes the single-writer lock for the duration and refuses with
  "the library is open in …" while the app holds it. `WritePolicy::InApp` (the server inside the app) does not take
  it because the process is the app.
  * metadata edits (`set_rating`, `add_tags`, `remove_tags`, `update_metadata`, `fetch_metadata apply`) and
    `add_book_from_path` go through `Library::update_metadata` / `add_path`: file changed, operation log entry, undo
    works (tested: `undo_last` restores).
  * highlights / bookmarks / reading status / shelves are not in the operation log (the app has no undo for them
    either); they are removable in the reader, and every write tool is in `.bookx/mcp-audit.log` (JSON lines: time,
    client name, tool, redacted arguments, ok, summary).
  * there is no delete tool, no file export tool, no path argument except `add_book_from_path` (canonicalised, must be
    inside `--allow-dir`; symlink/`..` escapes tested).
* Limits: `limit` ≤ 100 per page; text ≤ 20 000 chars per answer (`max_text_chars`); answers > 256 KB are refused;
  descriptions cut at 4 000 chars; rate limit 240 calls/min (`Config::rate_limit`); `call_timeout` 30 s; request bodies ≤ 1 MiB (HTTP) and lines ≤ 4 MiB (stdio).
* Secrets: tool arguments are redacted in the audit log (keys containing token/password/secret/key/authorization/
  cookie); error messages have the library path replaced by `<library>`; the library path never appears in answers
  (only file base names); the token is compared in constant time, never logged, never echoed when it was given on the
  command line (printed once on stderr only when generated); wrong tokens are throttled per peer by `bookx-serve::net`.
* DRM: books with `drm` set refuse text in every path (`read_book_text`, `get_toc`, `search_in_book`, chapter resource,
  highlights/notes creation); the full-text index hits of such books are dropped; metadata stays visible.
* Origin check on HTTP (DNS-rebinding rule of the spec): a request with an `Origin` that is not loopback (or listed with
  `--allow-origin`) gets 403 even with the token.

## Deviations from the brief

* `export_annotations` is a **read** tool (it returns text, nothing is written), so it is available read-only.
* `fetch_metadata` needs write access **and** `--allow-network` (it is the only tool with `openWorldHint`).
* There is no `fav:` / `search:` field in the library query language; `search_books` documents the fields that
  exist (`title author tag series publisher language format identifier comments rating progress size status drm weak`).
* The in-app write switch applies each change with a fresh `Library` over the same database while the app has its own
  in-memory copy: **the open window does not show an assistant's change until the book list is reloaded** (restart or
  library switch), and if the user edits the same book in the app afterwards, the app's in-memory record wins (a lost
  update, not corruption). Making the app reload on external commits (it could follow `data_version` like the share
  server does) is the clean fix; not done here to keep `src/ui/library.rs` untouched.
* The server inside the app serves the library that was open when it started; after switching libraries restart it
  (toggle) in Preferences.

## Tests

* `crates/bookx-mcp/src` unit tests (schema checker, cursors, similarity, redaction, limiter, URIs, locate/merge).
* `crates/bookx-mcp/tests/protocol.rs` (36): handshake and version negotiation, pre-initialize refusal, bad JSON,
  tools/list schema validity and annotations, pagination, every tool happy path and errors, caps, DRM, read-only
  enforcement (no write tools, protocol error when called), writes with undo, audit log, allow-list, lock refusal,
  in-app policy, fetch with the mock fetcher, progress, cancellation (deterministic and over the wire), logging level,
  rate limit, resources, prompts.
* `crates/bookx-mcp/tests/http.rs` (8): token missing/wrong/right, sessions, stateless clients, malformed requests,
  Origin, long token required, parallel clients, TLS with a self-signed certificate.
* `crates/bookx-cli/tests/mcp.rs` (4): the real binary over stdio (only protocol on stdout), `--allow-write`, option
  mistakes, `--http` with the token.
* `src/mcp_host.rs` unit tests; UI scenario `mcp_settings_page` in `build-aux/ui-tests.sh` (page, snippets, start/stop,
  real requests, token change, write switch, a11y audit, nothing leaked). Not in `session-tests.sh`: the page has no
  input behaviour that differs under a real session (switches/buttons only).

## Not tested / caveats

* No real assistant (Claude Desktop, Claude Code, Cursor, opencode) was run against it; the snippets follow their
  documented formats (`claude mcp add`, `mcpServers`, opencode's `mcp` block) and are checked as valid JSON/shell, not
  by the tools themselves. Claude Desktop's HTTP variant uses the `mcp-remote` bridge (not run).
* The session log (`SigNoz`-style telemetry) is not involved; nothing is sent anywhere.
* HTTP clients that insist on an SSE stream for notifications get none (GET → 405, which the spec allows).
* `get_reading_stats` reads the app's `stats.json`; `bookx-cli mcp` uses the default config path, so a library used
  with another config directory shows library counts only.
* Flatpak: `bookx-cli` is not in the Flatpak, so the *Command* snippets name a path that exists only for native
  builds; the in-app server works in the Flatpak (sandboxed local port; a host assistant reaches it only if the
  sandbox lets it, not tested).
