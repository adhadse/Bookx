#![forbid(unsafe_code)]
//! Sandboxed WebAssembly plugin host for Bookx (roadmap X3/X8, decision D5).
//!
//! Plugins are third-party extensions written in any language that compiles to WebAssembly. They run
//! in the pure-Rust `wasmi` interpreter (no JIT, no C, no WASI): no file system, no sockets, no clock
//! except what the host lends them.
//!
//! # Layout of a plugin
//!
//! A folder containing `plugin.toml` (see [`manifest`] for every key) and the `.wasm` file it names
//! (default `plugin.wasm`). The manifest declares `id`, `name`, `version`, `api_version = 1`, `kind`
//! (`metadata-source`, `format-reader`, `transform`, `format-writer`, `device`), `permissions`
//! (`network:<host>`; `fs:read` / `fs:write` are never granted in v1) and limits (`memory_pages`,
//! `fuel`, `timeout_ms`).
//!
//! # ABI v1
//!
//! All integers are WebAssembly `i32` unless noted; pointers are offsets into the exported memory.
//!
//! **Guest exports (required)**
//!
//! * `memory`: the linear memory.
//! * `alloc(len: i32) -> i32`: returns a buffer of `len` bytes (`len` may be 0). The host writes
//!   inputs there.
//! * `dealloc(ptr: i32, len: i32)`: releases a buffer from `alloc`.
//! * `bookx_call(op_ptr: i32, op_len: i32, in_ptr: i32, in_len: i32) -> i64`: runs operation `op`
//!   (UTF-8 name) on the UTF-8 JSON input. Returns `(reply_ptr << 32) | reply_len` pointing at a
//!   UTF-8 JSON reply in guest memory, or a **negative** error code: `-1` generic failure, `-2`
//!   unsupported operation, `-3` invalid input, `-4` out of memory.
//!
//! **Guest exports (optional)**
//!
//! * `bookx_info() -> i64`: packed pointer/length of a JSON description (free-form object).
//!
//! **Reply envelope.** The JSON reply is `{"ok": <value>}` or `{"err": "message"}`. Anything else
//! (non-JSON, non-object, neither key) is a protocol error. The per-operation inputs and values
//! are tabulated in [`adapters`].
//!
//! **Host imports** (module `bookx_host`; any other import, or a wrong signature, makes the plugin
//! fail to load: deny by default)
//!
//! * `log(level: i32, ptr: i32, len: i32)`: level 0 trace .. 4 error; 256 lines per call, 4 KiB per
//!   line.
//! * `http_get(url_ptr: i32, url_len: i32, out: i32) -> i32`: GET an `http(s)` URL. The host name must
//!   be listed in a `network:` permission (exact match, no wildcards). On success (`0`) the host
//!   calls the guest's `alloc`, stores the body there and writes `ptr` and `len` as two
//!   little-endian `u32` at `out` (8 bytes); the guest owns the buffer. Errors: `-1` bad arguments or
//!   URL, `-2` host not permitted, `-3` network failure, `-4` body over 8 MiB, `-5` guest allocation
//!   failed, `-6` more than 16 requests in one call, `-7` timed out. Network goes through the
//!   injected [`HostNetwork`] trait.
//! * `now_ms() -> i64`: Unix time in milliseconds.
//!
//! # Sandbox guarantees
//!
//! * Every call runs in a **fresh instance** (fresh memory, globals, tables): no state survives and
//!   a hostile plugin cannot poison later calls or leak memory in the host.
//! * **Fuel** limits instructions (`fuel`); the `timeout_ms` is also converted to fuel
//!   ([`runtime::FUEL_PER_MS`] per ms) so endless loops are cut off deterministically, and it is
//!   re-checked after the call and before every `http_get`.
//! * **Memory** is capped at `memory_pages` (64 KiB each): growing past it fails inside the guest, a
//!   module declaring more fails to instantiate.
//! * Replies over [`runtime::MAX_REPLY_BYTES`] and pointers outside the memory are rejected.
//! * Traps, bad exports, garbage replies all surface as `anyhow` errors, never as panics.
//!
//! # Writing a plugin in Rust (no SDK crate needed)
//!
//! Build with `cargo build --release --target wasm32-unknown-unknown` in a `crate-type = ["cdylib"]`
//! crate and copy the `.wasm` next to a `plugin.toml`:
//!
//! ```ignore
//! use std::alloc::{alloc as rs_alloc, dealloc as rs_dealloc, Layout};
//!
//! #[no_mangle]
//! pub extern "C" fn alloc(len: i32) -> i32 {
//!     unsafe { rs_alloc(Layout::from_size_align(len.max(1) as usize, 1).unwrap()) as i32 }
//! }
//!
//! #[no_mangle]
//! pub extern "C" fn dealloc(ptr: i32, len: i32) {
//!     unsafe { rs_dealloc(ptr as *mut u8, Layout::from_size_align(len.max(1) as usize, 1).unwrap()) }
//! }
//!
//! #[no_mangle]
//! pub extern "C" fn bookx_call(op_ptr: i32, op_len: i32, in_ptr: i32, in_len: i32) -> i64 {
//!     let op = unsafe { std::slice::from_raw_parts(op_ptr as *const u8, op_len as usize) };
//!     let input = unsafe { std::slice::from_raw_parts(in_ptr as *const u8, in_len as usize) };
//!     if op != b"lookup" {
//!         return -2;
//!     }
//!     let _ = input; // parse with serde_json if you like
//!     let reply: &'static str = Box::leak(
//!         r#"{"ok":[{"title":"Found it","authors":["A. Writer"]}]}"#.to_string().into_boxed_str(),
//!     );
//!     ((reply.as_ptr() as i64) << 32) | reply.len() as i64
//! }
//! ```
//!
//! # Using the host
//!
//! ```ignore
//! let mut mgr = PluginManager::new(plugins_dir, state_file, Host::default());
//! mgr.scan();
//! mgr.set_enabled("org.example.openlib", true)?;
//! let mut sources: Registry<dyn MetadataSource> = Registry::new();
//! mgr.register_metadata_sources(&mut sources);
//! ```
//!
//! Every kind has an adapter ([`PluginDevice`] for `device`). The full guide with a worked example
//! is `docs/plugin-development.md`; user patches (TOML, no WebAssembly) are in [`patch`].

pub mod adapters;
pub mod manager;
pub mod manifest;
pub mod patch;
pub mod runtime;

pub use adapters::{
    MountProvider, PluginDevice, PluginMetadataSource, PluginReader, PluginTransform, PluginWriter,
    SNIFF_BYTES,
};
pub use manager::{PluginManager, PluginStatus};
pub use manifest::{Limits, Manifest, Permission, PluginKind, API_VERSION};
pub use patch::{Hook, Patch, PatchManager, PatchReport, PatchSet, PatchStatus};
pub use runtime::{CallStats, Host, HostLog, HostNetwork, NoNetwork, NullLog, Plugin};
