//! `BOOKX_DUMP_CORPUS=dir cargo test --test dump_corpus` writes the synthetic corpus to disk (for
//! manual runs of Calibre or other tools on the same files).
mod common;
mod corpus;

#[test]
fn dump_corpus() {
    let Ok(dir) = std::env::var("BOOKX_DUMP_CORPUS") else {
        return;
    };
    std::fs::create_dir_all(&dir).unwrap();
    for s in corpus::all() {
        std::fs::write(std::path::Path::new(&dir).join(&s.file), &s.bytes).unwrap();
    }
}
