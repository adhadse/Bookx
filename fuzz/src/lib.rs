//! `fuzz_target!` that works with and without libFuzzer.
//!
//! With the `libfuzzer` feature it is `libfuzzer_sys::fuzz_target!`. Without it, the target is a
//! program that runs the closure on every file named on the command line (a directory is walked
//! one level), so a corpus can be replayed on stable: `cargo run -p ... --bin css -- fuzz/corpus/css/*`.

#[cfg(feature = "libfuzzer")]
pub use libfuzzer_sys::fuzz_target;

#[cfg(not(feature = "libfuzzer"))]
#[macro_export]
macro_rules! fuzz_target {
    (|$data:ident: &[u8]| $body:block) => {
        fn main() {
            let run = |$data: &[u8]| $body;
            let mut files = Vec::new();
            for a in std::env::args().skip(1) {
                let p = std::path::PathBuf::from(a);
                if p.is_dir() {
                    if let Ok(rd) = std::fs::read_dir(&p) {
                        files.extend(rd.flatten().map(|e| e.path()).filter(|p| p.is_file()));
                    }
                } else {
                    files.push(p);
                }
            }
            for f in files {
                if let Ok(bytes) = std::fs::read(&f) {
                    run(&bytes);
                }
            }
        }
    };
}
