# Security policy

Bookx reads files it did not create and, when you enable it, talks to the network. Bugs in those paths
are security bugs, and they are treated as such.

The full model, the severity rubric and the process are in [`docs/security.md`](docs/security.md).
This page is what you need to report a problem.

## Reporting a vulnerability

**Please do not open a public issue for an unfixed vulnerability.**

Use GitHub's private vulnerability reporting on this repository:
**Security → Advisories → Report a vulnerability**
(<https://github.com/adhadse/Bookx/security/advisories/new>).

If that is unavailable, open a normal issue that says only "I have a security report and need a
private channel" — with no details — and a maintainer will contact you.

Helpful in a report: affected version (`bookx --version` or the About dialog), distribution (Flatpak,
tarball, distro package), what the attacker controls (a book file, an OPDS feed, a plugin, the LAN),
and the smallest file or request that shows the problem. A proof of concept is welcome but not
required — we triage on impact, not on effort. You will never be asked to write a weaponised exploit.

## What to expect

| Step | Target |
|---|---|
| Acknowledge the report | 3 business days |
| Severity decision and plan | 10 business days |
| Fix for critical issues (code execution, secret leak, library corruption) | 7 days |
| Fix for issues that need an opt-in setting (LAN server, plugins, import) | 30 days |
| Fix for denial of service or local information disclosure | 90 days |
| Public advisory (GitHub Security Advisory, CVE where users need one) | with the release that fixes it |

If a deadline cannot be met you will be told why and when to expect the next update. You will be
credited in the advisory unless you ask not to be.

## Scope

In scope: anything reachable from a file Bookx opens, a feed or server it talks to, a plugin the user
installs, or a network service it runs (content server, KOSync, MCP). That includes memory safety in
the parsers, path traversal, decompression bombs, script or URI injection through book content, SSRF,
authentication and CSRF in the servers, secrets leaking into config, logs or backups, and supply-chain
problems in the crates we ship.

Out of scope: bugs that require an attacker who already runs code as your user, kernel or GTK bugs,
and physical access to the machine.

## Supported versions

The latest release and the previous minor version receive fixes. Anything older gets a fix only for
critical issues, if the change is small enough to backport.

## No bug bounty, no telemetry

Bookx has no paid bounty programme; we will credit you publicly instead. Bookx collects no telemetry,
no usage statistics and no installation identifier — there is nothing to leak.
