# Security model, vulnerability handling, and the path to a hardened Bookx

Date: 2026-11-13, updated 2026-11-21 (gap status, release signing, GitHub settings, the P1 fixes of
section 10). Status: audit + programme plan; section 4.2 tracks what is done. Companion to `SECURITY.md` (the public disclosure policy) and `AGENTS.md` (the working
agreement).

Bookx reads files it did not create and runs network services on the user's machine. This document
states what it trusts, what it does not, what it does when something goes wrong, and how the code gets
harder to break over time.

---

## 1. The failure patterns this design avoids

Applications that open files they did not create fail in a small number of predictable ways. Each
pattern below is a rule in this repository, not a historical note.

A setuid helper that creates and removes directories on user-controlled paths, forwards user-controlled
argv to `mount`, and resolves its program through `PATH` is unsafe no matter how carefully it is
patched: the bugs are in the shape of the design, not in the lines of code. Seven patterns follow from
that, and each one maps to a rule here:

1. **Inventing a privilege boundary to bridge a platform gap.** A helper that runs as root only because
   the platform lacks a portable API is wrong by construction. Use the platform's broker
   (udisks/polkit/portals) instead. *Rule: no privilege change, ever. Devices go through gvfs/GIO/udisks
   (Bookx already does this).*
2. **Reviewing a function instead of a capability.** If nobody writes down "this component may create and
   remove directories anywhere on the system, and run arbitrary programs as root", the review has no
   unit to work with. *Rule: every component that touches the outside world gets a written capability
   statement.*
3. **Time-of-check to time-of-use.** A path validated and then re-resolved (`realpath`, `stat`, `exec`)
   can be swapped in between. *Rule: validate and use the same handle (`openat2`, `chdir` + `"."`);
   never re-open a path you already checked.*
4. **Untrusted strings at an exec boundary.** *Rule: no shell, no `PATH` lookup, no user-controlled
   element in argv.*
5. **Assumptions instead of defence in depth.** "Unprivileged users cannot create symlinks in `/dev`" is
   false (there is `/dev/shm`), and the same shape of reasoning fails everywhere. *Rule: never reason
   from "the user can't do that".*
6. **A response process that gates on proof.** Making a reporter produce a weaponised exploit before the
   report is believed, shipping patches without a regression test, and moving an issue to *fixed*
   before the test exists all turn one bug into a permanent one. *Rule: triage on impact, fix with a
   failing test, one owner per advisory, "fixed" means "the test fails before and passes after".*
7. **Convenience winning over security.** Shipping the widest possible behaviour so it "just works" is
   how a design hole survives for years. *Rule: secure default, opt-in risk, documented in the UI when
   the user widens exposure.*

## 2. Scope and adversary model

Default deployment: one user on a desktop or laptop, Flatpak or a distro package. The app is not
multi-tenant and does not run as root. Adversaries, in order of likelihood:

| # | Adversary | Entry point |
|---|---|---|
| A1 | A malicious book file (the common case: downloaded, shared, bought) | EPUB/FB2/FBZ/MOBI/AZW3/CBZ/PDF/DOCX/ODT/HTMLZ parsing, images, fonts, KOSync sidecars |
| A2 | A malicious network peer | OPDS feeds, RSS for news, metadata sources, Calibre/KOSync clients, MCP client, WebDAV server |
| A3 | A malicious plugin or patch the user installs | WebAssembly module, TOML patch set |
| A4 | Another local user, only when a LAN server is enabled | content server, KOSync, MCP HTTP |
| A5 | A compromised dependency or build | 568 crates in `Cargo.lock`, Flatpak runtime, GitHub Actions |
| A6 | A corrupted or hostile backup / library from elsewhere | `.bookxbackup`, imported library folder, Calibre `metadata.db` |

Out of scope: an attacker who already runs code as the user (that is not a boundary Bookx can hold),
kernel or GTK bugs, and physical access. It is still worth limiting the blast radius of A1–A3,
because "the damage is already done" is wrong when the user's library, keyring
and cloud credentials are what is behind the boundary.

**Trust boundaries:** process boundary (Bookx never changes uid), Flatpak sandbox, the wasm sandbox,
each network listener, and every `Command::new`. Bookx has no setuid component and no root path at all.

## 3. Assets

- The library: book files, `metadata.db`, per-book hashes, annotations, reading positions.
- Secrets in the Secret Service or `secrets.vault`: sharing password, e-mail password, KOSync password, WebDAV and
  backup passwords, MCP bearer token (`src/config.rs:522-543`, `src/mcp_host.rs`).
- The user's filesystem reachable from the app (documents, `/run/media`, gvfs).
- Integrity of backups and of what is written to e-readers.
- The user's identity against third parties (no telemetry; that is a feature, keep it).

## 4. Current state (audited 2026-11-13)

### 4.1 Controls that are already in place

| Area | Evidence |
|---|---|
| Memory safety by construction | Every library crate is `#![forbid(unsafe_code)]` (all 18 library crates, including all parsers). The `bookx` GTK crate has 20 `unsafe` uses (counted 2026-11-21: `page_area.rs` 3, `ui/editor/*` 15, `ui/audit.rs` 2), all gtk-rs object-data (`set_data`/`data`) and the `GtkAccessibleText` vtable in `src/ui/page_area.rs`; none of them touch untrusted input. |
| No privilege boundary | No setuid/setgid, no mount helper, no root path. Devices via gvfs/MTP (`crates/bookx-serve/src/mtp.rs`) and `bookx-store/src/guard.rs` for mount-point safety. |
| No JavaScript in the reader | Own Pango/cairo layout engine; `<script>`/`<style>`/`<template>` are dropped (`crates/bookx-core/src/html.rs:516`). PDF links are scheme-allowlisted (`src/ui/reader_pdf.rs:1284`). |
| Archive limits | `read_zip_limited` caps entries, per-entry and total size, and skips `..` entries (`crates/bookx-formats/src/common.rs:575-610`), with a traversal regression test. TTS model archives refuse unsafe paths and verify hashes (`crates/bookx-tts/src/download.rs:823`). |
| XML without entity expansion | `roxmltree` (no external/internal entity expansion); DTDs are parsed, never fetched. |
| Server hardening | Loopback bind by default, optional basic auth with constant-time compare, per-peer failed-auth throttle, request/body/connection limits, range parsing (`crates/bookx-serve/src/server.rs`, `net.rs`). MCP adds a ≥16-char bearer token, origin allowlist and throttling (`crates/bookx-mcp/src/http.rs`). |
| Secrets at rest | Secret Service via `oo7` (Keychain / Credential Manager on macOS / Windows), else Bookx's encrypted `secrets.vault` (Argon2id + ChaCha20-Poly1305, master password never stored), else memory only; **no secret is ever written in clear text**, plaintext of older versions is scrubbed on load (`bookx-secrets`, `src/secrets.rs`, `src/config.rs`). The sync server stores a salted Argon2id hash of the account key. SMTP refuses to send a password in clear text (`crates/bookx-serve/src/mail.rs:365`). |
| Plugin sandbox | `wasmi` interpreter with fuel, memory pages and wall-clock timeout; capabilities limited to `network:<host>` and, for `editor-action` plugins (ABI v2), `editor.read` / `editor.write` on the open book only (path rules, protected `mimetype`/`META-INF`, call and size limits, copy of the book, checkpoint; `crates/bookx-plugin/src/editor.rs`), no filesystem (`crates/bookx-plugin/src/runtime.rs`, `manifest.rs`). |
| Device safety | Library guard refuses removable/network mount points for the catalogue (`bookx-store/src/guard.rs`). |
| Book checker | Structural checks that also flag remote resources, scripts and unsafe names (`crates/bookx-editing/src/validate/`). |

### 4.2 Gaps

Status as of 2026-11-21. "Done" rows stay for the record.

| # | Gap | Status |
|---|---|---|
| G1 | No `SECURITY.md`, no private channel. | **Done.** `SECURITY.md`; GitHub private vulnerability reporting enabled 2026-11-21 (section 11). |
| G2 | No threat model. | **Done.** This document. |
| G3 | No dependency audit in CI. | **Mostly done.** `deny.toml`, `cargo deny` job in `ci.yml`, `.github/dependabot.yml`; Dependabot alerts and security updates enabled on GitHub 2026-11-21. Open: SBOM, scheduled run (workflows have never run on GitHub). |
| G4 | No fuzzing, no hostile-input corpus. | **Done (first version), 2026-11-21.** `fuzz/` (11 targets, section 12) and `crates/*/tests/evil_*.rs` (hostile inputs in normal `cargo test`). Open: nightly fuzz job, hours of real fuzzing, a shared `Limits` type (P1.3). |
| G5 | The GTK crate cannot `forbid(unsafe_code)`; its 20 `unsafe` uses have no `// SAFETY:` comment. | **Open (S4).** All are gtk-rs `set_data`/`data` calls and the accessibility vtable; none touches untrusted input. The comments touch `src/ui/**` files other work is editing; do them in a quiet window. |
| G6 | `is_external` accepts `ftp://` and `data:`. | **Done 2026-11-21.** `is_external` is kept as a classification (converters need `data:` images to count as "not a book file"); every place that *opens* a link goes through `bookx_core::util::safe_external_url` (http, https, mailto only; control characters, spaces, backslash, bidi and zero-width characters, non-ASCII scheme, `mailto:?attach=` refused). Reader, PDF reader, editor help links and `ui::open_uri` use it. |
| G7 | `BOOKX_TTS_PLAYER` runs through `sh -c`. | **Done 2026-11-21**, and the same pattern in `BOOKX_AUDIO_PLAYER`: words are split with quoting only, the program must be an absolute path, `{rate}` is replaced per argument after splitting; built-in players are found only in `/usr/bin`, `/bin`, `/usr/local/bin`, `/app/bin`, never through `PATH`. |
| G8 | No SSRF guard. | **Done 2026-11-21** for everything that fetches through `bookx_sources::fetch::UreqFetcher` (metadata, covers, Add from the Web, news and read-later, online catalogues, link checks, plugin `http_get`, MCP fetch): a resolver plugged into ureq (`netguard.rs`) drops loopback, private, link-local (169.254.169.254), 100.64/10, fc00::/7, IPv4-in-IPv6, 6to4, NAT64, Teredo, documentation, multicast and reserved addresses on every hop; max 5 redirects; http and https only; body and time caps as before. The host of an OPDS catalogue you added, or of an address you typed in Add from the Web, is trusted (`trust_host`). WebDAV, KOSync, SMTP and the TTS voice server are addresses you configure and use their own clients (no redirects followed). Not covered: a configured HTTP(S) proxy (its own DNS), Add-from-the-Web typed host rebinding (user typed it, accepted). |
| G9 | No `Origin`/`Host` check. | **Done 2026-11-21.** `bookx_serve::hostguard` runs before every route of the content server, OPDS, `/bookx` sync and in the MCP endpoint: `Host` must be an IP literal, a one-label name, `*.local`, `*.localhost`, `*.home.arpa`, `*.lan`, `*.internal`, the machine name or listed (`--allow-host`, `BOOKX_ALLOWED_HOSTS`); a request with a foreign `Origin`, or `Sec-Fetch-Site: cross-site` on a non-GET, gets 403. Native clients (KOReader, OPDS apps) send no `Origin`. MCP keeps its own origin allow-list as well. |
| G10 | Plugins are not signed; the install dialog does not show capabilities in plain language. | **Open (S2).** Capabilities are declared and listed in Preferences; no signature or pin-by-hash yet. 2026-11-21: manifests are capped at 64 permission lines and `url_host` rejects ports above 65535. |
| G11 | Tarball installs are unsandboxed. | **Open (P2).** |
| G12 | No security step in `AGENTS.md`, no label, no owner. | **Done.** Section 6 of `AGENTS.md`; `security` label created and owner named (section 11). |
| G13 | Release artifacts not signed. | **Decided and wired 2026-11-21, not yet run.** Section 13: Sigstore cosign keyless + build-provenance attestation in `release.yml`. SBOM and Flatpak provenance still open. |
| G14 | Quadratic or recursive parsing of hostile markup (found by the new evil tests). | **Done 2026-11-21.** roxmltree and html5ever compare each attribute with the earlier ones (30 000 attributes took 7 s in debug, a million would take minutes): `parse_xml` refuses more than 5000 attributes on an element and `parse_html_document`/`parse_html_fragment` return an empty document. html5ever overflowed the stack on 100 000 nested `<div>`: nesting above 2000 is treated the same way. `:not(` nesting in the CSS selector parser overflowed the stack: limited to 8. All untrusted HTML parsing in the workspace now goes through the guarded functions. |
| G16 | Without a keyring, passwords and tokens stayed **in clear text in `config.json`** (and the MCP token in `mcp.json`); the sync server kept the raw MD5 account key. | **Done 2026-11-21.** See the change record below. |
| G15 | Backup and repository files could request 1 GiB and 64 passes of Argon2. | **Done 2026-11-21.** `crypto::kdf_is_reasonable` (at most 256 MiB, 10 passes, 8 lanes; Bookx writes 19 MiB, 2, 1) used by `.bookxbackup` and the repository. |

## 5. The rule for each vulnerability class

Each row is the rule new code must follow, and the test that proves it.

| Class | Bookx rule | Test |
|---|---|---|
| Memory safety | No `unsafe` in library crates: `#![forbid(unsafe_code)]` is set and must stay. In `bookx` (GTK) the only allowed `unsafe` is gtk-rs object-data and FFI vtable glue, each with a `// SAFETY:` comment; pointer arithmetic on data from a file or the network is never allowed. | Compile time for the crates; review for the app crate. |
| Path traversal / zip slip | Never write or read by a name from an archive. `read_zip_limited`-style limits, reject absolute paths and `..` components, then join and verify containment on the canonical path. | `common.rs::read_zip_skips_path_traversal_entries`, `download.rs::an_unsafe_oversized_or_linked_member_is_not_unpacked`. |
| Decompression and parse bombs | Every parser that takes untrusted bytes takes an explicit budget (entries, bytes in, bytes out, nesting depth, time). No unbounded `read_to_end`. | A budget test per parser: the bomb is rejected with an error, not an OOM. |
| XML entities | `roxmltree` only; never enable entity resolution, never fetch a DTD. | A test feeding an XXE and a billion-laughs document. |
| Script and markup injection | Reader renders text; it never executes markup. No web view, ever. Server pages escape all metadata (`server.rs` tests). | A book with `<script>` renders as text. |
| URI handlers | Allowlist `http`, `https`, `mailto`; confirm before opening. Never `file:` or a scheme that can launch a local handler. | Unit test on `is_external` / the PDF equivalent. |
| SSRF | Any URL that came from a book, feed or remote server is resolved and checked: no loopback, link-local, private, or metadata addresses unless the user typed it. | `Fetcher` test with a fake resolver. |
| Auth, CSRF, rebinding | Servers default to loopback, auth optional but warned, constant-time compare, throttle, `Origin`/`Host` checks on mutations. | `server.rs` auth/CSRF scenarios. |
| Secrets | Keyring, else the encrypted `secrets.vault` (master password), else memory only. **Never** in config, other plain files, logs, error messages, crash dumps, backups or MCP resources, whatever the state of the stores (`Vault::persist` blanks every secret in the copy it is given). Test backend is memory-only. | `config.rs` `no_vault_state_ever_puts_a_secret_in_the_file`, `no_serialization_path_carries_a_secret`, `bookx-secrets` file tests, UI scenarios `secrets_*`. |
| Crypto | AEAD with a reviewed construction, KDF parameters documented, never invent a format. | Vector tests; restore of a tampered backup must fail. |
| Command execution | Fixed argv, absolute program or a user-chosen program from settings; no `sh -c`, no `PATH` search for anything security-relevant. | A test that a crafted book title never reaches an argv element. |
| Untrusted wasm | Fuel, memory and time limits; capabilities declared and shown to the user; host imports reviewed like public API. | `runtime.rs` limit tests; a plugin that loops and one that allocates must fail cleanly. |
| Supply chain | New dependency needs a reason, an audit, a license check and a `NOTICE.md` entry. Lockfile changes are reviewed, not rubber-stamped. | `cargo deny` in CI. |
| DoS | Budgets in the parser layer, limits in the transport layer, perf tests for hostile input. | `perf_budgets.rs` cases for pathological input. |

## 6. Vulnerability handling: how Bookx keeps the number small

The goal is not "no bugs"; it is "few, found early, fixed fast, never twice".

**Intake.** `SECURITY.md` points to GitHub private vulnerability reporting (Security Advisories). No
public issue for an unfixed report. Anything that arrives in a public issue is moved to a private
advisory within one working day, with a neutral public reply.

**Triage.** Acknowledge in 3 business days, decide severity in 10. Severity is impact × exposure ×
preconditions, not reporter volume:

| Severity | Meaning | Examples | Fix target |
|---|---|---|---|
| S0 | Code execution, secret exfiltration, or library corruption in the default configuration, reachable from A1–A3. | Malicious EPUB writes outside the library and runs a program; MCP token leaks to any web page. | 7 days |
| S1 | The same, but the user must opt in (LAN server, plugin install, import a hostile library). | Content-server CSRF when bound to `0.0.0.0`. | 30 days |
| S2 | Denial of service, local information disclosure, server-side XSS in a page the user opens. | A book that OOMs the app; OPDS feed reading `~/.ssh`. | 90 days |
| S3 | Requires an attacker who already controls the process or the account. | `BOOKX_TTS_PLAYER` shell usage. | Next release |
| S4 | Hardening, no demonstrated impact. | Missing `forbid(unsafe_code)`. | Backlog |

**Fixing.** In this order, always: (1) a test that reproduces the attack and fails, (2) the fix, (3)
the test passes, (4) a second person reviews the diff adversarially — "what else can this input
reach?", (5) the advisory. A fix without a regression test is not a fix. If the fix is in a parser, the
same input goes into the fuzz corpus.

**Advisory.** GHSA (and a CVE when it crosses a trust boundary for users) for S0–S2, published after
the release. Reporter credited unless they ask otherwise. Supported versions: the latest release and
the previous minor. Backports only for S0/S1.

**Tracking.** A `security` label, a milestone per advisory, and a monthly `docs/security-log.md`
entry: open advisories, their age, dependency advisories acted on, fuzz hours, and the last
threat-model review. That log is how "very few vulnerabilities" becomes a measurable claim instead of
a hope.

**Metrics to watch:** time-to-triage, time-to-fix by severity, advisories reopened (should be zero),
percent of untrusted-input parsers with a budget test and a fuzz target, count of crates older than 24
months, and secrets found in logs/config during tests (must be zero).

## 7. Roadmap

### P0 — Foundations (about a week)

Status 2026-11-21: items 1, 2, 4 and 5 done (item 4's scheduled run and SBOM are open); item 3 still lacks the `// SAFETY:` comments (G5).

1. `SECURITY.md` with scope, private reporting, targets, credit, supported versions.
2. This document, linked from `AGENTS.md`; the per-change checklist (below) added there.
3. Keep `#![forbid(unsafe_code)]` on all 15 library crates; add `// SAFETY:` comments to the 20
   GTK `unsafe` blocks in `bookx` and a review rule that no new one parses untrusted data.
4. CI job: `cargo deny check advisories bans licenses sources` with a committed `deny.toml`;
   Dependabot for `cargo` and `github-actions`, weekly.
5. A `security` label, and one named owner for advisories.

Acceptance: CI fails when a shipped crate matches a published advisory; `cargo deny` is clean today;
the checklist is in `AGENTS.md`.

### P1 — Make the parsers prove themselves (2–4 weeks)

Status 2026-11-21: items 1 and 2 have a first version (`fuzz/`, `crates/*/tests/evil_*.rs`; the nightly job and real
fuzzing hours are open), item 4 is done (G6 to G9), item 3 (one shared `Limits` type) is open.

1. `cargo fuzz` targets for the six entry points: zip container, OPF/XML, XHTML, CSS, PDF, and the
   KOSync sidecar reader. Seed corpus from `data/` test books; nightly job with a time budget.
2. A `tests/evil/` corpus checked into the repository and run by a normal test: zip bomb, 100k-entry
   zip, deep nesting, billion laughs, truncated everything, `..` and absolute names, a PDF with a
   link loop, a 1 GB declared image. Each must fail cleanly within a time and memory budget.
3. One `Limits` type shared by all parsers, threaded through the app path, not just `bookx-formats`.
4. SSRF guard in `Fetcher` (G8); `Origin`/`Host` check on content-server mutations (G9); narrow
   `is_external` to `http`/`https`/`mailto` (G6); replace `sh -c` in the TTS sink with argv (G7).

Acceptance: a new untrusted-input parser cannot be merged without a budget test and a fuzz target
(checked in review); the evil corpus runs in under 30 s in CI.

### P2 — Reduce blast radius (a quarter)

1. Sandbox the non-Flatpak install too: a `bwrap`/Landlock wrapper or portal-only file access; make
   the Flatpak the recommended install and document the difference.
2. Sign releases (decided: Sigstore cosign keyless, section 13; wired in `release.yml`) and generate an SBOM (`cargo cyclonedx`) per release; publish
   both next to the tarball. Verify the Flatpak build provenance.
3. Plugin trust: signature or pin-by-hash, a plain-language capability sheet at install
   ("can talk to openlibrary.org"), and an "unverified plugin" banner.
4. Backup crypto review: Argon2id KDF, documented parameters, tamper tests, key rotation, and a
   restore-from-untrusted-file test.
5. Dependency policy: maximum age, maintenance check, one-in-one-out for large crates.

Acceptance: an S1 in the parser or server layer is contained by the sandbox; a user can tell what a
plugin may do before installing it.

### P3 — Continuous

- Quarterly threat-model review (this document), with the diff of the last quarter's trust-boundary
  changes as the input.
- Annual external review of the parsers and the network surface, or a paid bug-bounty window if the
  project can afford it.
- An incident runbook (who does what in the first hour) and one tabletop exercise a year.
- "Red team Thursday": for each feature shipped, someone writes the attack and adds it to the evil
  corpus.
- Keep the `docs/security-log.md` monthly entry.

### P4 — When servers and MCP become first class

- Authenticated by default, per-user scoping, real TLS guidance instead of self-signed by default,
  and an audit-log view for MCP writes (the log already exists in `bookx-mcp`).
- Treat every remote peer as A4 even on loopback: a browser on the same machine is a remote peer.

## 8. Per-change security checklist

Paste into the PR description for any change that touches untrusted input, a path, a network, a
process, a secret or a dependency.

- [ ] **Untrusted bytes?** Which parser, what budget (entries/bytes/depth/time), which bad-input test,
      which fuzz target?
- [ ] **Path from a file or the network?** Joined *and* containment-checked after canonicalisation;
      opened by handle, not re-resolved after the check.
- [ ] **Executes something?** No shell, no `PATH` search, no untrusted argv element.
- [ ] **Listener or user-supplied URL?** Loopback default, auth default where it makes sense, SSRF
      guard, `Origin`/`Host` check on mutations.
- [ ] **New dependency?** Audit, license, `NOTICE.md`, pinned, justified.
- [ ] **Secret?** Keyring, encrypted store or memory only; not in config or any plain file, logs, errors, backups, resources or tests. A new secret field goes into `Config::secret_fields` (the scan tests then cover it).
- [ ] **Widens a capability?** The capability statement in this document is updated, and someone else
      reviewed it.
- [ ] **Regression test** fails before the fix and passes after. No exceptions.

## 9. Anti-patterns Bookx will not adopt

- No setuid/setgid helper, no root path, no "just shell out to mount".
- No shell-string interpolation, no `PATH` lookup for a program that matters.
- No "an unprivileged user cannot do that" as a mitigation.
- No fix without a failing test; no "Fix Released" that only means "committed".
- No silent security fix: if it crosses a trust boundary, it gets an advisory.
- No telemetry, no install UUID, no phone-home. Bookx collects nothing to leak.

## 10. Change records

### 2026-11-21: trusted proxies, sub-path, web client, accounts, QR code, Wallabag, Wikipedia, Hardcover (T6/T8)

Checklist (§8):

* **Untrusted bytes?** Yes: Wallabag/Hardcover/Wikipedia JSON and Wallabag HTML (`wallabag::parse_page`, `hardcover::parse_page`, `wikipedia::parse_response`), the
  OPDS XML read by the web client script. HTML goes through `clean_fragment` (whitelist) plus, for Wikipedia, a noise filter; the script uses `textContent`
  only. Tests: `wallabag::tests::article_text_goes_through_the_whitelist`, `pwa::tests::no_file_loads_anything_from_another_site_or_runs_inline`.
* **Path from file/network?** None used as a file path. The sub-path is validated (`clean_public_prefix`, test `public_prefix_cleaning`).
* **Executes something?** No.
* **Listener / user URL?** No new listener; new routes `/app/*` pass `hostguard`, the login (page) and the limits. The Wallabag address is user typed: its host is
  trusted for private addresses (`netguard::trust_url`), pictures and links inside articles are **not**: `tests/wallabag_live.rs` proves a picture on another
  private address is never contacted. Wikipedia only asks `*.wikipedia.org` (`requests_only_go_to_wikipedia`). Hardcover has a fixed endpoint.
  The new `Fetcher::post` has the same offline switch, scheme check and guarded resolver as `get`.
* **Secret?** Wallabag client secret, password, token and the Hardcover token are `Config::secret_fields` (keyring/vault, never the settings file; test
  `web_accounts::tests::old_settings_without_accounts_load_and_the_secrets_stay_out_of_the_file`). Tokens travel in headers, never in URLs or error texts
  (`the_token_goes_in_the_header_only…`, `errors_name_the_problem_and_never_the_token`). Account passwords are Argon2id hashes (`keyhash`), a plain value in the settings is not a login (`accounts_and_guest_reach_the_server_config_and_plain_passwords_do_not`).
* **Widens a capability?** `X-Forwarded-For` is believed only from addresses the user listed (default none; `/0` refused): regression tests
  `x_forwarded_for_is_ignored_without_a_trusted_proxy` (a rotating header cannot dodge the throttle), `a_trusted_proxy_makes_the_throttle_count_each_client`,
  `forwarded_for_is_believed_only_from_a_trusted_proxy`. A guest can read the listed collections without a login (opt-in, read only, empty list = no guest);
  wrong credentials are never downgraded to guest (`a_guest_reads_some_collections_and_nothing_else`); an account sees only its collections in every route
  (`accounts_see_only_their_collections`). The public static files of `/app/` contain nothing of the library.
* **Dependency?** None (own QR encoder, `qr.rs`, checked against an independent decoder).


### 2026-11-21: no secret in clear text, ever (G16)

Owner decision: "under no circumstances the passwords/tokens should be saved unencrypted". Checklist (§8):

* **Secret?** Yes, this is the change. Design: `Vault::persist` empties **every** secret in the serialized copy whatever the state
  of the stores (the old "kept: value that cannot be stored stays in the copy" path is gone), and `Config::file_text` clears them
  once more afterwards. Stores, in order: system keyring; else `FileStore` (`crates/bookx-secrets/src/file.rs`, `secrets.vault`
  next to `config.json`): header `BOOKXVLT` + version + Argon2id parameters (default 64 MiB, 3 passes, 1 lane, checked with
  `kdf_is_reasonable`: at most 256 MiB, 10 passes, 8 lanes) + salt + sequence number; per item ChaCha20-Poly1305 with a random
  nonce and the header plus the item name as associated data; HMAC-SHA256 over the whole file, **checked first, in constant
  time**, so a wrong master password and a damaged file give one answer and no partial data; temp file 0600, fsync, rename,
  fsync of the folder; file at most 1 MiB, 512 items, 128-byte names, 16 KiB values; keys and the unlocked values are `Zeroizing`.
  The master password is never stored. Else: memory only for the session (`Not saved: it is forgotten when Bookx closes`).
  UI: `src/ui/secrets_unlock.rs` (create / unlock / forgot dialogs, asked at most once per session; "Not Now" / "Don't Save" means
  memory only). Duplicated crypto instead of a dependency on `bookx-backup` (that crate pulls in the whole server and store); the
  KDF bounds are the same.
* **Migration:** plain text secrets found in `config.json` are moved into the keyring, or kept in memory when there is none, and
  the file is rewritten at once without them (0600, folder 0700); `config.json.tmp` is removed and `.old`, `.bak`, `~`, `.orig`
  copies that parse as settings are rewritten without secrets (`config::scrub_copies`). A one-time toast says
  "Saved passwords were removed from the settings file". `mcp.json` no longer has a writable token field (`token_fallback` is
  read only, to move an old value); without a store the MCP token is made again at each start.
* **Audit of other places:** settings export (`settings_io`, secrets blanked, import never reads them), Debug Information
  (`debug_log::scrub` with the live secrets), diagnostics (`diagnostics::gather` reads no secret field), dev hooks (`kosync:dump`
  prints booleans only; `secrets:has` prints an equality, never a value), `bookx-cli` (reads `saved_searches` from the settings, prints
  no setting), backups (catalogue and books; the passphrase is not in them), MCP resources (library only).
* **Sync server:** the KOSync account key (MD5 hex of the password, which KOReader sends as `x-auth-key`; **a protocol limit**: the
  key is the credential and MD5 is fast) is stored as a salted Argon2id hash (`bookx-serve/src/keyhash.rs`, 19 MiB, 2 passes),
  verified in constant time with a unknown-user dummy hash; successes are cached in memory as HMAC under a per-process key;
  raw keys of older `kosync.json` files are hashed when the file is opened; the file is written 0600. The MCP bearer token was
  already compared with `constant_time_eq`.
* **KOReader plugin:** stores only the MD5 key in `settings/bookx_sync.lua`, never the password (checked in `main.lua`,
  `bookxsync.lua`); documented in the wiki (`koreader-sync.md`, "What is stored on the e-reader"). **Future (not done):** per-device
  revocable keys so one lost e-reader can be cut off without changing the account password.
* **Regression tests** (fail on the old code, whose `Vault::persist` left the value in the file): `config::secret_tests::
  without_a_keyring_the_file_never_has_a_password`, `no_vault_state_ever_puts_a_secret_in_the_file`,
  `no_serialization_path_carries_a_secret`, `plain_text_in_the_file_is_scrubbed_even_without_a_keyring`,
  `scrubbing_removes_secrets_from_old_copies_and_makes_the_folder_private`; `bookx-secrets` file tests (round trip, wrong password,
  every byte flip and every truncation, hostile KDF/count/size, rollback, item swap, 0600/0700, create never overwrites);
  `bookx-serve` `the_account_key_is_never_on_disk_and_old_files_are_upgraded`; UI scenarios `secrets_no_keyring_not_in_file`,
  `secrets_master_password_vault`, `secrets_migrate_plaintext`, `secrets_no_keyring`; `crates/bookx-secrets/tests/evil_vault.rs`
  and the `vault` fuzz target.
* **Residual risks:** malware running as the user while the keyring or the store is unlocked can read the secrets (out of scope,
  §2). A weak master password can be attacked offline by someone who copies `secrets.vault` (Argon2id slows it, length is enforced
  at 10 characters, no strength estimator). Rollback of `secrets.vault` to an older copy between sessions is not detectable. Key
  names in the vault are readable (they say which services have a password, not what it is). Secrets live in ordinary `String`s
  in memory while Bookx runs (GTK entries and the keyring client are not zeroized); swap and core dumps are not addressed.
  The first-run dialog and unlock dialog were exercised only through the dev hooks, not by a person (no session test).
  Old copies of `config.json` made by *other* tools (backups of the home folder, snapshots) cannot be reached by Bookx: existing
  users should rotate passwords they care about.

### 2026-11-21: P1 fixes (G6 to G9, G14, G15), fuzzing, GitHub setup, release signing

Checklist (§8). Each fix has a test that fails on the old code; new user strings: none (no `po` change).

* **G6 URI handlers.** `bookx_core::util::safe_external_url`; tests `only_http_https_and_mailto_are_ever_opened` (60 refused
  forms: case, tabs/newlines in the scheme, NUL and control characters, fullwidth scheme letters, bidi/zero-width, backslash,
  userinfo-less empty hosts, `file:`, `data:`, `javascript:`, `ftp:`, `smb:`, relative and protocol-relative links,
  `mailto:?attach=` in plain and percent-encoded form) and `is_external_stays_a_classification_not_permission`. Wired into
  `Input::FollowLink` (reader), `reader_pdf::activate_link`, `format_actions::open_url`, and `ui::open_uri` (which now accepts
  only `file:///` addresses built by the app, plus what the function accepts).
* **G7 command execution.** `bookx-tts/src/command.rs` and the copy in `bookx-audio`; tests `words_split_with_quotes_and_no_expansion`,
  `the_program_must_be_an_absolute_path`, `substitution_cannot_add_words`, `system_players_are_not_looked_up_through_path`,
  `a_custom_player_is_never_run_through_a_shell` (a `; touch` / `$(touch)` / `sh -c` value creates no file). UI tests now set
  `BOOKX_TTS_PLAYER=/bin/cat`.
* **G8 SSRF.** `bookx-sources/src/netguard.rs`, `fetch.rs`; tests: address tables (blocked and allowed), `loopback_and_metadata_addresses_are_refused_before_any_connection`
  (127.1, decimal and hex forms, `localhost`, `[::1]`, 169.254.169.254, fd00:ec2::254, `::ffff:127.0.0.1`: the local listener
  sees zero connections), `a_host_the_user_configured_may_be_local`, `a_redirect_to_a_private_address_is_not_followed`,
  `redirect_loops_stop_at_the_cap`, `only_http_and_https_are_fetched`, `the_body_cap_is_enforced`.
* **G9 Host/Origin.** `bookx-serve/src/hostguard.rs`, `server::handle`, `bookx-mcp/src/http.rs`; tests in `hostguard`,
  `a_rebinding_host_and_a_cross_origin_post_never_reach_a_route`, `a_dns_rebinding_host_is_refused_even_with_the_token`,
  `request_heads_with_hostile_hosts_never_reach_the_library`. Reviewed in the same pass, no change needed: bearer and basic
  credentials are compared in constant time, failed credentials are throttled per peer address (10 per minute, 429 for a
  minute, KOSync `x-auth-key` included, `failed_kosync_logins_count_too`), request line 8 KiB, head 16 KiB, 100 headers, body
  1 MiB, header 10 s, body 30 s, idle 15 s, 64 connections and 32 per peer. Known weak point: behind a reverse proxy every client
  shares the proxy's address, so one wrong password throttles everyone (documented in the wiki; trusted-proxy setting still open).
  `POST /bookx/users/create` is open registration by design (KOReader registers itself); with the Origin check a web page can
  no longer call it.
* **G14/G15.** `util::has_attribute_flood`, `has_excessive_nesting`, `parse_html_document/_fragment`; tests
  `an_attribute_flood_is_refused_in_linear_time`, `absurd_html_nesting_is_refused_before_the_parser_can_overflow`,
  `deeply_nested_not_is_refused_instead_of_overflowing_the_stack`, `key_settings_beyond_what_bookx_writes_are_refused`, and the
  evil corpus. Plugin manifest: `MAX_PERMISSIONS` (64; 50 000 permission lines took 23 s) and port range in `url_host`.
* **Untrusted bytes / fuzz.** Section 12. **New dependency:** none in the app (the `fuzz/` workspace pulls `libfuzzer-sys`,
  not shipped). **Secret:** none. **Widens a capability:** no, narrows several.
* **Behaviour changes users may notice:** a feed or book link to a LAN address is no longer fetched; a catalogue on the LAN
  works because you added it; a reverse proxy needs `--allow-host`/`BOOKX_ALLOWED_HOSTS` for dotted public names; a custom
  player command must start with an absolute path.

### 2026-11-20: editor plugin API (ABI v2) and EPUB 3 narration (CE-019, CE-422)

* Untrusted bytes: SMIL documents of a book are parsed by `bookx_editing::media_overlay::parse_smil` (16 MiB, 200 000 `par`,
  depth 32, DTD refused, clock values bounded; references to other schemes, absolute paths and paths that leave the book are dropped,
  never followed). WAV headers are bounds-checked in `overlay_play::wav_info`; clips are cut to 10 minutes; audio in memory is capped.
  Tests: `hostile_references_are_dropped`, `malformed_smil_is_an_error_not_a_panic`, `size_depth_and_count_limits`,
  `a_hostile_smil_is_survived`, `wav_headers_are_checked`.
* Path from a file: plugin paths go through `editor::valid_book_path` / `is_protected` before the host touches the archive; tests
  `escape_attempts_never_reach_the_book` (Rust and through a real wasm module).
* Executes something: no. Listener / URL: no. New dependency: none. Secret: none.
* Widens a capability: yes, plugins of the new kind `editor-action` may read and change the open book after the user switched them
  on and read the permission text; the capability statement above is updated. Needs a second reviewer.

### 2026-11-20: audiobooks (wave 6, PP-G-25)

Checklist (§8) for `crates/bookx-audio`, `src/audiobook.rs`, `src/ui/audiobook_player.rs`:

* **Untrusted bytes:** ID3v2 (v2.2-2.4 frames, unsynchronisation, `CHAP`, `APIC`), MP4 atoms (`moov`, `chpl`, `ilst`, chapter
  text track with `stbl` tables), Vorbis comments and Ogg pages, FLAC blocks, WAV chunks, Symphonia decoding of whole files,
  zip entries, the per-book state and cache JSON. Budgets in `bookx_audio::limits` (see `docs/audiobook-notes.md`); tests
  that feed every prefix and byte flips of generated and real files and absurd sizes; no `unsafe`; no cargo-fuzz target yet.
* **Paths:** zip entry names go through `scan::safe_rel` (absolute, `..`, backslash, drive letter, NUL, hidden and over-long
  names are skipped) and the unpacked path is checked to stay under the destination; folders are walked without following
  symlinks; part paths in the cache are relative to the audio folder and joined on load (a cache written by hand could name
  `../x`: parts are only opened for reading by the decoder, never written or executed, and a part that does not exist makes the
  cache invalid).
* **Executes something:** only the sound player (`pacat`, `pw-cat`, `aplay`) by name with rate/channel arguments of our own;
  the shell override `BOOKX_AUDIO_PLAYER` is an environment setting, like `BOOKX_TTS_PLAYER`.
* **Listener / URL:** none (MPRIS is a D-Bus service on the user's own session bus, as read aloud has).
* **New dependency:** `symphonia` 0.5.5 and codec/format crates (MPL-2.0), in `NOTICE.md`.
* **Secret:** none. **Capability:** a book can now make Bookx read audio files from the folder it links to, with the same
  rights as the user; nothing is written there.
* **Regression tests:** hostile-input tests per parser, `safe_names`, `zip_entries_are_checked_and_unpacked`,
  `symlinks_are_not_followed`, `the_list_and_notes_are_bounded`.

### 2026-11-20: comic reader (wave 6, PP-G-24)

Checklist (§8) for `crates/bookx-comic`, `crates/bookx-formats/src/comic.rs`, `src/comic.rs`, `src/ui/reader_comic.rs`:

* **Untrusted bytes:** the page pictures of a CBZ/CBT (JPEG, PNG, GIF, WebP through the `image` crate) and ComicInfo.xml. Budgets in
  `bookx_comic::decode`: a header that claims more than 160 megapixels or 40 000 pixels on a side is refused **before** anything is
  allocated (`dimensions` reads the header only; the decoder also gets `Limits` for width, height and 1 GiB of allocation); the decoded
  page is reduced to at most 4096 px on its longest side and 12 megapixels (2400 px on e-ink) on a worker thread, and the cache of
  decoded pages has a byte budget (320 MiB, pages on screen are never evicted). The converter drops a page whose header claims an
  absurd size (`decode::is_bomb`), so it never enters the library. The archive readers already bound entry size, total size and entry
  count. Tests: `image_bombs_are_refused_from_the_header` (a 70-byte PNG claiming 900 megapixels), `an_image_bomb_page_is_left_out`,
  `a_small_header_with_missing_data_fails_cleanly`, `garbage_is_an_error_not_a_panic`, the UI scenarios `comic_bomb` and
  `comic_corrupt`. No cargo-fuzz target for the page decoder yet (the `image` crate is the exposed parser).
* **Path from a file:** page names from the archive are never used as paths: the converter renames every page to
  `images/page-NNNN.ext`, entries with `..` are skipped (`archive_entry_names_cannot_reach_outside_the_book`). Per-book settings are
  keyed by the numeric book id.
* **Executes something:** no. **Listener / URL:** no. **New dependency:** none new (`image`, `zip`, `serde` were already in the tree;
  the new crate `bookx-comic` is ours). **Secret:** none.
* **Widens a capability:** no. The settings file `comics.json` holds layout choices only.
* **Regression test:** the real-session swipe test (`comic_manga`) failed before the fix of a drag that also counted as a click
  (an interface bug, not a security one).

### 2026-11-20: release hardening pass (wave 6, PP-C-19)

* Dependency audit: `cargo deny` 0.21 (installed under `~/.cache/tools`, advisory database fetched from GitHub) found six
  vulnerabilities in `lettre` 0.11.23 (RUSTSEC-2026-0560 to 0565: STARTTLS reply injection, SMTP command injection by dot-stuffing,
  quoted local part delivered to another mailbox, two panics, unbounded reply buffering). `lettre` is used by the *Send by e-mail*
  feature of `bookx-serve`. **Fixed** by `cargo update -p lettre` to 0.11.24 (Cargo.lock and `build-aux/cargo-sources.json`
  regenerated); regression test `release_consistency::lettre_is_past_the_smtp_advisories`. This is an advisory-worthy change if
  0.2.0 binaries are in use: the issue was present in the 0.2.0 tree.
* Seven "unmaintained" notices remain (no known vulnerability): `chrono`, `ttf-parser`, `smartstring` (via `rhai`), `zvariant`,
  `zbus_names`, `zvariant_derive` (via `oo7`), `rustybuzz` (dev-dependency only). They are listed with reasons in `deny.toml`;
  `cargo deny check advisories bans licenses sources` is clean with them. Gap G3 is now half closed: a committed policy and a CI job
  (`deny` in `ci.yml`, never run on GitHub), but no scheduled run, no Dependabot and no SBOM yet.
* Licences: every crate in the lock file has a licence the policy allows (MIT, Apache-2.0, ISC, BSD, Zlib, Unicode, CC0, 0BSD,
  bzip2-1.0.6, CDLA-Permissive-2.0, MPL-2.0, LGPL-2.1-or-later as an alternative, GPL-3.0-or-later for our own crates). New
  since the 0.2.0 files: `symphonia` (MPL-2.0) and its codec crates, `extended`, `bitflags` 1.3 (audiobooks, uncommitted at the
  time of writing); `NOTICE.md` now lists them and the remaining direct dependencies. Wave 5 itself changed no dependencies.
* Narration / plugins / overlays (wave 5): re-read against the checklist, nothing new; the plugin ABI is now frozen by the golden
  tests (`crates/bookx-plugin/tests/abi_compat.rs`), so a host change cannot silently widen what an old plugin may do.
* Flatpak permissions re-checked against the manifest: network, IPC, Wayland/X11 fallback, DRI, PulseAudio, Documents, `/run/media`,
  `xdg-run/gvfs` and `gvfsd`, `org.gtk.vfs.*`; no home/host/all-devices/session-bus/ssh-agent. Guarded by `release_consistency::
  the_flatpak_manifest_stays_minimal`, `desktop_integration` and `release-check.sh`. `--share=network` and `/run/media` are the
  widest and are needed (see `docs/packaging-notes.md`).
* Release pipeline: the release workflow builds with `--locked`, runs `release-check.sh` first, never pushes tags, and publishes
  `SHA256SUMS`. Artefacts are not signed (P2 item 2 stays open). Workflows have never run on GitHub.

### 2026-11-20: Sync UX, KOReader plugin and wave-6 regressions

Per-change checklist (§8) for the wireless sync and editor fixes in the final wave:

- **Untrusted bytes?** Annotation JSON from `/bookx` is still parsed by the existing size-capped readers in
  `bookx-serve::annotation_sync`; the colour merge adds no new parse path. `bookxmd5.lua` hashes only a local password string
  with bounded length; vectors cover 55/56/57-byte and 1000-byte inputs (`koreader-plugin/tests/run.sh`).
- **Path from a file or the network?** No new path join. The plugin installer copies a fixed list of seven files into KOReader's
  `plugins` directory; it does not use a path supplied by a file or the network.
- **Executes something?** No shell or `PATH` lookup added. The plugin's KOReader-facing callbacks are wrapped in `pcall` so a
  malformed settings file or dialog error cannot become a device crash; the error reporter itself is fail-safe.
- **Listener or user-supplied URL?** No new listener. `/bookx` replaces `/kosync` with no alias; the server's existing auth and
  loopback defaults are unchanged. The plugin's server URL comes from the device's own settings form.
- **New dependency?** None. `bookxmd5.lua` is original, dependency-free Lua (MIT, listed in `NOTICE.md`); it replaces the
  optional KOReader `ffi/sha2` module so a missing FFI library cannot panic.
- **Secret?** Still only the one-way hash in the plugin settings; the plaintext password is never stored, sent or logged, and an
  empty password field means “keep the saved hash”. No secret appears in the new tests.
- **Widens a capability?** No. The sync plugin is unchanged in scope; the UI edits only configuration.
- **Regression tests?** `docs/bugs.md` rows 17–21: plugin exceptions and MD5, device colour preservation, preview follow race,
  external-editor events, and the virtual Contents list scroll. Each has a test that fails without its fix.

### 2026-11-21: incremental backup repository with book files (D9)

Checklist (§8) for `crates/bookx-backup/src/repo/`, `bookx-serve` streaming WebDAV, `bookx-cli backup`; the full text with
test names is in `docs/backup-notes.md` (D9).

* **Untrusted bytes:** repository config, manifests, journal records, blobs and WebDAV answers; budgets and bad-input tests
  as listed there (manifest 128 MiB / 1 000 000 files, bounded blob reads, Argon2 limits, no decompression).
* **Paths:** manifest paths validated twice (`validate_rel`), restored component by component without following links,
  temp-name-then-rename; repository paths limited to `[A-Za-z0-9._-]`, three segments.
* **Executes something:** no. **Listener / URL:** no listener; user-typed WebDAV address as before.
* **New dependency:** `hmac`, `subtle`, `zeroize` (already in the lock file). **Secret:** passphrase from the keyring only,
  zeroized keys, never in state/logs/manifests.
* **Capability:** Bookx now copies book files to user-chosen places and can write book files into a chosen folder on
  restore; the capability statement above still needs updating and a second reviewer.
* **Regression tests:** `a_hostile_manifest_cannot_write_outside_the_target`, `a_link_in_the_target_is_never_followed_on_restore`,
  `an_old_snapshot_renamed_to_look_new_is_refused`, `tampered_truncated_and_missing_blobs_are_caught_for_both_kinds_of_place`.


## 11. GitHub settings and ownership (applied 2026-11-21)

Applied with the `gh` CLI as `adhadse` (admin on `adhadse/Bookx`); branch protection and code were not touched.

| Setting | State | Command |
|---|---|---|
| Private vulnerability reporting | enabled | `gh api -X PUT repos/adhadse/Bookx/private-vulnerability-reporting` |
| `security` label | created (`#B60205`) | `gh label create security --color B60205 --description "Security issue or hardening (advisories go through private reporting)" -R adhadse/Bookx` |
| Dependabot alerts | enabled | `gh api -X PUT repos/adhadse/Bookx/vulnerability-alerts` |
| Dependabot security updates | enabled | `gh api -X PUT repos/adhadse/Bookx/automated-security-fixes` |
| Secret scanning and push protection | enabled | `gh api -X PATCH repos/adhadse/Bookx` with `{"security_and_analysis":{"secret_scanning":{"status":"enabled"},"secret_scanning_push_protection":{"status":"enabled"}}}` |
| Advisory owner | **@adhadse** (repository owner), named in `SECURITY.md` | |

Not enabled (not available on this plan or not wanted): secret-scanning validity checks and non-provider patterns; code
scanning (CodeQL) is a possible next step. To switch a setting on by hand: repository **Settings → Code security**. To check:
`gh api repos/adhadse/Bookx --jq .security_and_analysis` and `gh api repos/adhadse/Bookx/private-vulnerability-reporting`.

## 12. Fuzzing and hostile inputs

* `fuzz/` is its own Cargo workspace (excluded from the main one, so `cargo test --workspace` never builds it). Targets:
  `epub`, `formats` (all readers incl. PDF; first byte picks the extension), `html`, `css`, `wire` (annotation JSON, KOSync routes,
  Host check), `lua` (KOReader sidecar), `backup` (`.bookxbackup` header, manifest JSON, paths), `plugin` (manifest and wasm),
  `comic` (page images), `pdf`, `url` (link gate, host and origin checks, blocked addresses).
* With nightly and `cargo install cargo-fuzz`: `cargo +nightly fuzz run css`. Without them, the targets are replay programs and
  compile-check on stable: `cargo check --manifest-path fuzz/Cargo.toml --no-default-features`;
  `cargo run --manifest-path fuzz/Cargo.toml --no-default-features --bin css -- fuzz/corpus/css`. Seeds are under `fuzz/corpus/`
  (about 100 KB; keep under 200 KB). `fuzz/README.md` has the commands. No libFuzzer run was possible on the machine that wrote
  this (no `cargo-fuzz`), so the targets have only been replayed on their seeds.
* Deterministic corpus tests (run in `cargo test`, about 30 s together, no network): `bookx-formats/tests/evil_inputs.rs`
  (garbage and magic prefixes for every extension, truncation of valid files, XML entity bombs, XXE, nesting, RTF braces,
  zip names, zip bombs, entry-count floods, lying EOCD and sizes, overlapping entries, MOBI size lies, image bombs, PDF loops),
  `bookx-core/tests/evil_epub.rs` (nesting, floods, zip layouts, OPF), `bookx-text/tests/evil_css.rs`,
  `bookx-serve/tests/evil_wire.rs`, `bookx-backup/tests/evil_backup.rs`, `bookx-plugin/tests/evil_plugin.rs`,
  `bookx-comic/tests/evil_images.rs`. They found three real bugs (G14) and two limits worth tightening (G15, plugin permissions).
* Not covered yet: the Bookx sync server under network-level fuzzing (the HTTP layer is hyper), `bookx-audio` tag parsers (they have
  their own prefix/bit-flip tests), `bookx-store` (SQLite import of a hostile Calibre `metadata.db`), `bookx-lookup` StarDict, and a
  memory ceiling for the evil tests (time is bounded, RSS is not). The 512 MiB `Archive::MAX_MEMBER` bomb is not run in tests because it
  needs that much memory; the budgets are tested with the explicit limits of `read_zip_limited`.

## 13. Release signing and key custody (decision 2026-11-21)

**Decision: Sigstore cosign, keyless, from GitHub Actions; plus GitHub build-provenance attestations. No long-lived key exists.**

* Why: a signing key held by one person is the most likely thing to leak or be lost; keyless signing ties a signature to the
  identity of the workflow (`https://github.com/adhadse/Bookx/.github/workflows/release.yml@refs/tags/v…`) and records it in the
  public Rekor transparency log. Nothing to rotate, back up or protect. Cost: verifying needs `cosign` (or `gh`) and the signatures only prove
  "built by this workflow for a tag", not that the tag's code is good; a compromised repository account or workflow can still sign.
* **Key custody: none.** There is no private key, no secret in GitHub, no hardware token. The only trust anchors are GitHub's OIDC
  issuer, Sigstore's Fulcio/Rekor, and the repository's tag protection (the owner pushes tags; branch protection was left alone).
* What `release.yml` does (publish job, never run yet): `SHA256SUMS` of every file, `actions/attest-build-provenance` over
  `dist/*`, `cosign sign-blob --yes --bundle FILE.sigstore.json FILE` for every file including `SHA256SUMS`, then `cosign verify-blob`
  with the identity regexp above, then the GitHub release. `src/release_consistency.rs` checks that these steps stay in the file.
* User steps: wiki `install.md`, "Verify your download" (`sha256sum -c`, `cosign verify-blob`, `gh attestation verify`).
* Open: SBOM (`cargo cyclonedx`) next to the tarball; pin third-party actions by commit SHA (Dependabot keeps them current);
  verify the Flatpak build provenance; reproducible binaries; first real run and a check that the verify step passes
  with the installed cosign version (`cosign-installer@v3` installs the latest 2.x/3.x; the `--bundle` form works in both).
