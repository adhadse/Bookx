//! Fills a models folder with packs that look installed (files of the right size, sparse, plus the
//! install marker), for the UI tests that must not download anything:
//!
//!     fake_install <dir> <pack-id> [pack-id ...]

use bookx_tts::models::{Store, find_pack};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let store = Store::new(a.get(1).expect("usage: fake_install <dir> <pack-id>..."));
    for id in &a[2..] {
        let p = find_pack(id).unwrap_or_else(|| panic!("unknown pack {id}"));
        for f in &p.files {
            let path = store.dir_of(&p).join(&f.rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::File::create(&path)
                .unwrap()
                .set_len(f.size)
                .unwrap();
        }
        store.mark_installed(&p).unwrap();
    }
    // the runtime library is a file name, not a pack marker
    let lib = store.root.join("onnxruntime");
    if a[2..].iter().any(|i| i == "onnxruntime") {
        std::fs::create_dir_all(&lib).unwrap();
    }
}
