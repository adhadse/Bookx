# Fuzzing Bookx

Targets (one per untrusted-input entry point): `epub`, `formats` (every reader, PDF included), `html`, `css`, `wire`
(`/bookx` JSON and routes), `lua` (KOReader sidecar), `backup`, `plugin` (manifest and WebAssembly), `comic`, `pdf`, `url`, `helpers` (the output of `unrar`, 7-Zip and `ddjvu`).
The same entry points have deterministic hostile-input tests in `crates/*/tests/evil_*.rs`, which run in `cargo test`.

```bash
cargo install cargo-fuzz                      # nightly toolchain
cargo +nightly fuzz run css -- -max_total_time=300     # from the repository root: fuzz/ is its own workspace
# or, on stable without libFuzzer, replay a corpus (also what CI does):
cargo run --manifest-path fuzz/Cargo.toml --no-default-features --bin css -- fuzz/corpus/css
cargo check --manifest-path fuzz/Cargo.toml --no-default-features   # all targets compile
```

`fuzz/corpus/<target>/` holds small seeds (keep the whole directory under 200 KB). A crash found by fuzzing is minimised,
added to the corpus and to the matching `evil_*.rs` test before the fix (docs/security.md, section 6).
