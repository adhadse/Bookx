# Security model, vulnerability handling, and the path to a hardened Bookx

Date: 2026-11-13. Status: current-state audit + programme plan. No behaviour was changed by this
document. Companion to `SECURITY.md` (the public disclosure policy) and `AGENTS.md` (the working
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
- Secrets in the Secret Service: sharing password, e-mail password, KOSync password, WebDAV and
  backup passwords, MCP bearer token (`src/config.rs:522-543`, `src/mcp_host.rs`).
- The user's filesystem reachable from the app (documents, `/run/media`, gvfs).
- Integrity of backups and of what is written to e-readers.
- The user's identity against third parties (no telemetry; that is a feature, keep it).

## 4. Current state (audited 2026-11-13)

### 4.1 Controls that are already in place

| Area | Evidence |
|---|---|
| Memory safety by construction | Every library crate is `#![forbid(unsafe_code)]` (15 crates, including all parsers). The `bookx` GTK crate has 20 `unsafe` uses, all gtk-rs object-data (`set_data`/`data`) and the `GtkAccessibleText` vtable in `src/ui/page_area.rs`; none of them touch untrusted input. |
| No privilege boundary | No setuid/setgid, no mount helper, no root path. Devices via gvfs/MTP (`crates/bookx-serve/src/mtp.rs`) and `bookx-store/src/guard.rs` for mount-point safety. |
| No JavaScript in the reader | Own Pango/cairo layout engine; `<script>`/`<style>`/`<template>` are dropped (`crates/bookx-core/src/html.rs:516`). PDF links are scheme-allowlisted (`src/ui/reader_pdf.rs:1284`). |
| Archive limits | `read_zip_limited` caps entries, per-entry and total size, and skips `..` entries (`crates/bookx-formats/src/common.rs:575-610`), with a traversal regression test. TTS model archives refuse unsafe paths and verify hashes (`crates/bookx-tts/src/download.rs:823`). |
| XML without entity expansion | `roxmltree` (no external/internal entity expansion); DTDs are parsed, never fetched. |
| Server hardening | Loopback bind by default, optional basic auth with constant-time compare, per-peer failed-auth throttle, request/body/connection limits, range parsing (`crates/bookx-serve/src/server.rs`, `net.rs`). MCP adds a ≥16-char bearer token, origin allowlist and throttling (`crates/bookx-mcp/src/http.rs`). |
| Secrets at rest | Secret Service via `oo7`, plaintext migration on load, memory backend in tests (`bookx-secrets`, `src/secrets.rs`). SMTP refuses to send a password in clear text (`crates/bookx-serve/src/mail.rs:365`). |
| Plugin sandbox | `wasmi` interpreter with fuel, memory pages and wall-clock timeout; capabilities limited to `network:<host>` and, for `editor-action` plugins (ABI v2), `editor.read` / `editor.write` on the open book only (path rules, protected `mimetype`/`META-INF`, call and size limits, copy of the book, checkpoint; `crates/bookx-plugin/src/editor.rs`), no filesystem (`crates/bookx-plugin/src/runtime.rs`, `manifest.rs`). |
| Device safety | Library guard refuses removable/network mount points for the catalogue (`bookx-store/src/guard.rs`). |
| Book checker | Structural checks that also flag remote resources, scripts and unsafe names (`crates/bookx-editing/src/validate/`). |

### 4.2 Gaps

| # | Gap | Why it matters |
|---|---|---|
| G1 | No `SECURITY.md`, no private reporting channel, no CVE/GHSA process, no security contact. | A reporter has nowhere good to go, so the report arrives in public or not at all. |
| G2 | No threat model in the repository (this document is the first). | Nobody can review a boundary that was never written down. |
| G3 | No dependency audit in CI, no `cargo deny`, no Dependabot/Renovate, no SBOM. | 568 crates, zero automated advisory signal. |
| G4 | No fuzzing and no hostile-input corpus. Parser limits exist but are not exercised adversarially, and not uniformly: `bookx-core`, `bookx-editing` and PDF parsing rely on the caller. | A1 is the most likely adversary and the least tested. |
| G5 | The GTK app crate cannot `forbid(unsafe_code)` (gtk-rs FFI). Its 20 `unsafe` blocks are uncommented and not covered by a rule, and new ones can appear without review. | Small, but it is the one place where memory safety is a promise rather than a proof. |
| G6 | `is_external` accepts `ftp://` and `data:` (`crates/bookx-core/src/util.rs:106`). | An untrusted book can hand weird schemes to the desktop's URI handler on one click. Narrow to `http(s)`/`mailto`, as the PDF reader already does. |
| G7 | `BOOKX_TTS_PLAYER` is run through `sh -c` with `{rate}` substitution (`crates/bookx-tts/src/sink.rs:129`). | Environment-driven shell. It is the user's own env var, so severity is low, but it is the exact anti-pattern from lesson #4. Use argv, not a shell. |
| G8 | User-supplied URLs (metadata fetch, OPDS, news) are fetched without a private-address guard. | SSRF against the user's LAN and cloud metadata endpoints, from a hostile feed. |
| G9 | Content server has no `Origin`/`Host` check on state-changing routes. | CSRF/DNS-rebinding once the user sets `0.0.0.0`. |
| G10 | Plugins are not signed and the install dialog does not show the capabilities in plain language. | A3 becomes "install this to get metadata" and silently gain a network capability. |
| G11 | Tarball installs are unsandboxed; only Flatpak has a sandbox. | A1 blast radius is the whole home directory outside Flatpak. |
| G12 | No security review step in `AGENTS.md` or the PR flow; no "security" label; no owner. | Processes decay unless they are written down and checked. |
| G13 | Release artifacts are checksummed but not signed or attested; Flatpak is not verified against a builder identity. | A5. |

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
| Secrets | Keyring only. Never in config, logs, error messages, crash dumps, backups or MCP resources. Test backend is memory-only. | `config.rs` round-trip tests, log grep in UI tests. |
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
2. Sign releases (`cosign`/minisign) and generate an SBOM (`cargo cyclonedx`) per release; publish
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
- [ ] **Secret?** Keyring only; not in config, logs, errors, backups, resources or tests.
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
