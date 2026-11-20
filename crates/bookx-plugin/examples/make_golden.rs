//! Regenerates `tests/golden/v1-rawcopy/plugin.wasm` from the worked example. Only run this on purpose:
//! the golden binaries exist to stay the same while the host changes (see `tests/abi_compat.rs`).
//!
//!     cargo run -p bookx-plugin --example make_golden

use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let wat =
        std::fs::read_to_string(root.join("../../examples/plugins/rawcopy-writer/plugin.wat"))
            .unwrap();
    let wasm = wat::parse_str(wat).unwrap();
    std::fs::write(root.join("tests/golden/v1-rawcopy/plugin.wasm"), wasm).unwrap();
}
