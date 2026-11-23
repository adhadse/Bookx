> Outdated 2026-11-23: Owner decision D10 (2026-11-20): wireless sync is KOReader only, stock firmware uses USB. Not planned; kept in case the decision is reversed. Living document: [bookx-feature-roadmap.md](../bookx-feature-roadmap.md).

# Plan: Calibre "smart device" wireless protocol (roadmap P14 / E2)

Status: **not implemented, deferred by owner decision D10 (2026-11-20): wireless sync is KOReader only (Bookx sync); the stock firmware uses USB.** Roadmap rows P5 and P13 are DEFERRED in `docs/roadmap-status.md`. This is the plan, written because the work is larger than the rest of the
device track together. Nothing here has been tried against Calibre, Calibre Companion or a real device.

## What it is

Calibre can talk to a phone, tablet or e-reader app (Calibre Companion, KOReader's Calibre plugin, Librera and
others) over Wi-Fi. The device app announces itself, calibre connects to it over TCP, and the two exchange
book lists, metadata and files. Supporting it would make Bookx a drop-in replacement for Calibre's
"Connect/share → Start wireless device connection" for those apps.

Two directions are possible and they are separate pieces of work:

1. **Bookx as the library side** (what Calibre does): find the device app, list its books, send books,
   delete books, read its reading state. This is what users of "send to device" expect.
2. **Bookx as the device side** (what the phone apps do): let Calibre send books to Bookx. Not wanted.

Only direction 1 is planned.

## Protocol (from the Calibre manual and the behaviour of the apps; confirm against traffic captures)

* **Discovery.** The device app listens for a UDP broadcast on a few well-known ports and answers with its
  TCP address; or the user types the device's IP address and port.
* **Transport.** One TCP connection. Every message is a length prefix in ASCII digits followed by a JSON
  array `[opcode, payload]`. Large files are sent as binary segments after a message that announces their
  length.
* **Session.** The library side sends `GET_INITIALIZATION_INFO` (versions, supported extensions, whether a
  password is required, the device's book path templates), the device answers; a password challenge is a hash
  of the password and a salt. After that: `GET_DEVICE_INFORMATION`, `FREE_SPACE`, `GET_BOOK_COUNT`, per-book
  metadata (`SEND_BOOKLISTS`), `SEND_BOOK` + file segments, `DELETE_BOOK`, `GET_BOOK_FILE_SEGMENT` (to
  fetch a book back), `NOOP` keep-alives.
* The opcode numbers and field names are **not** copied here on purpose (see below); they are read from
  captured traffic and the published manual when the work starts.

## Licence approach

Calibre's driver is GPL-3.0 Python. The guardrail in the roadmap applies: clean-room reimplementation in
Rust from the manual, captured traffic and the behaviour of the apps. No code is copied.

## Work breakdown

| Step | Size | Notes |
|---|---|---|
| `crates/bookx-serve/src/wireless/` message codec (length prefix + JSON array, binary segments) | S | Pure functions, fuzzable, unit tests with recorded sessions. |
| Session state machine (init, password hash, keep-alive, error handling, timeouts, size limits) | M | Must never block the UI; one thread per connection like the WebDAV client. |
| Discovery (UDP broadcast, manual address) | S | Network tests must be skippable (`NET=0`); loopback fake device for CI. |
| A fake device in tests (a small TCP server speaking the protocol) | M | The only way to test without hardware; recorded from a real session once available. |
| `DeviceDriver` adapter (`detect`, `send`, `list`) | S | Same shape as `MtpGvfsDriver` and `WebDavDriver`; `DeviceInfo.mount` carries the address. |
| Device window: a "Wireless device" entry, address and password fields | S | Reuses the Device window; books list, remove and free space already work through `Drivers`. |
| Reading state from the device (`GET_BOOK_METADATA`) | S | Feeds `plan_sync` like the Kobo database does. |
| Real-device testing with Calibre Companion and KOReader | L | Cannot be done without hardware; until then the feature is "implemented against a fake". |

Total: roughly two weeks for one developer, most of it the fake device and real-device debugging.

## Risks

* The protocol has versions; apps differ in what they support. The plan is to start with the oldest widely
  supported version and read capabilities from the initialisation reply.
* A wrong implementation can confuse a device app but cannot damage the library: Bookx only sends and
  deletes what the user selected.
* No hardware is available for automated tests, so the first release should say "experimental".

## Decision needed from the owner

Whether this is worth doing before the KOSync / OPDS / WebDAV / USB / MTP routes, which already reach most of
the same devices. KOReader (the most common target) can already use Bookx's OPDS catalogue and progress sync,
and Kobo/Kindle/Boox readers work over USB.
