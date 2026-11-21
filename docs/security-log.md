# Security log

Monthly record (docs/security.md, section 6): open advisories, their age, dependency advisories acted on, fuzz hours,
the last threat-model review. Newest first. The owner of advisories is @adhadse.

## 2026-11

* **Advisories:** none open, none published. Private vulnerability reporting enabled on 2026-11-21; `security` label created.
* **Dependencies:** `cargo deny` clean on 2026-11-20 (lettre 0.11.24 fixed six RUSTSEC advisories, seven "unmaintained"
  notices listed in `deny.toml`). Dependabot alerts and security updates enabled 2026-11-21. `cargo deny` was not installed on the
  machine that did the 2026-11-21 work, so it was not re-run.
* **Fixed (found by the P1 audit and the new evil-input tests, all with regression tests; details in docs/security.md section 10):**
  link schemes, shell in the player override, SSRF guard, Host/Origin checks, attribute and nesting floods in XML/HTML,
  CSS `:not(` recursion, plugin permission flood, unbounded key derivation in backup files.
* **Fuzz hours:** 0 (targets exist, seeds replayed on stable only; no nightly job yet).
* **Threat-model review:** 2026-11-21 (this pass). Next: 2027-02.
* **Metrics:** time-to-triage and time-to-fix: no reports yet. Parsers with budget test and fuzz target: the readers, EPUB
  core, CSS, sync wire, Lua sidecar, backup, plugin, comic and PDF entry points; not yet audiobook tags, StarDict, Calibre import.
* **Open items:** `// SAFETY:` comments (G5), plugin signing (G10), non-Flatpak sandbox (G11), SBOM and first signed release
  (G13), trusted-proxy setting, a nightly fuzz job, a shared `Limits` type, a UI field for allowed host names.
