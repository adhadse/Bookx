//! A size-capped cache of synthesized sentences, so reading a page again (or going back)
//! is instant and costs no synthesis.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use sha2::{Digest, Sha256};

use crate::synth::Pcm;

#[derive(Debug, Clone)]
pub struct SentenceCache {
    dir: PathBuf,
    cap_bytes: u64,
}

/// The key of a sentence: everything that changes how it sounds.
pub fn cache_key(engine: &str, voice: &str, language: &str, speed: f64, text: &str) -> String {
    let mut h = Sha256::new();
    for part in [engine, voice, language, &format!("{speed:.3}"), text] {
        h.update(part.as_bytes());
        h.update([0]);
    }
    format!("{:x}", h.finalize())
}

impl SentenceCache {
    pub fn new(dir: impl Into<PathBuf>, cap_mb: u32) -> SentenceCache {
        SentenceCache {
            dir: dir.into(),
            cap_bytes: cap_mb as u64 * 1024 * 1024,
        }
    }

    fn path(&self, scope: &str, key: &str) -> PathBuf {
        let scope: String = scope
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.dir.join(scope).join(format!("{key}.pcm"))
    }

    pub fn get(&self, scope: &str, key: &str) -> Option<Pcm> {
        let p = self.path(scope, key);
        let bytes = fs::read(&p).ok()?;
        if bytes.len() < 4 {
            return None;
        }
        let rate = u32::from_le_bytes(bytes[..4].try_into().ok()?);
        // reading counts as use: the oldest unused sentences go first
        if let Ok(f) = fs::File::options().write(true).open(&p) {
            let _ = f.set_modified(SystemTime::now());
        }
        let mut pcm = crate::synth::pcm_from_s16le(&bytes[4..], rate);
        if pcm.samples.is_empty() {
            pcm.rate = rate;
        }
        Some(pcm)
    }

    pub fn put(&self, scope: &str, key: &str, pcm: &Pcm) {
        if self.cap_bytes == 0 || pcm.samples.is_empty() {
            return;
        }
        let p = self.path(scope, key);
        if let Some(d) = p.parent()
            && fs::create_dir_all(d).is_err()
        {
            return;
        }
        let tmp = p.with_extension("tmp");
        let write = || -> std::io::Result<()> {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(&pcm.rate.to_le_bytes())?;
            let bytes: Vec<u8> = pcm.to_i16().iter().flat_map(|s| s.to_le_bytes()).collect();
            f.write_all(&bytes)?;
            fs::rename(&tmp, &p)
        };
        if write().is_err() {
            let _ = fs::remove_file(&tmp);
            return;
        }
        self.evict();
    }

    /// Deletes the least recently used sentences until the cache fits its cap.
    pub fn evict(&self) {
        let mut files: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
        collect(&self.dir, &mut files);
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        if total <= self.cap_bytes {
            return;
        }
        files.sort();
        for (_, size, path) in files {
            if total <= self.cap_bytes {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                total -= size;
            }
        }
    }

    pub fn size_bytes(&self) -> u64 {
        let mut files = Vec::new();
        collect(&self.dir, &mut files);
        files.iter().map(|f| f.1).sum()
    }

    /// Forgets the sentences of one book (or everything with an empty scope).
    pub fn clear(&self, scope: &str) {
        let target = if scope.is_empty() {
            self.dir.clone()
        } else {
            self.path(scope, "x").parent().unwrap().to_path_buf()
        };
        let _ = fs::remove_dir_all(target);
    }
}

fn collect(dir: &Path, out: &mut Vec<(SystemTime, u64, PathBuf)>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|x| x == "pcm")
            && let Ok(m) = e.metadata()
        {
            out.push((m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len(), p));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-cache-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn pcm(n: usize) -> Pcm {
        Pcm {
            rate: 24000,
            samples: (0..n).map(|i| (i % 100) as f32 / 200.0).collect(),
        }
    }

    #[test]
    fn keys_differ_by_everything_that_changes_the_sound() {
        let k = |e, v, l, s, t| cache_key(e, v, l, s, t);
        let base = k("kokoro", "af_heart", "en", 1.0, "Hello.");
        assert_eq!(base, k("kokoro", "af_heart", "en", 1.0, "Hello."));
        assert_ne!(base, k("piper", "af_heart", "en", 1.0, "Hello."));
        assert_ne!(base, k("kokoro", "am_adam", "en", 1.0, "Hello."));
        assert_ne!(base, k("kokoro", "af_heart", "fr", 1.0, "Hello."));
        assert_ne!(base, k("kokoro", "af_heart", "en", 1.2, "Hello."));
        assert_ne!(base, k("kokoro", "af_heart", "en", 1.0, "Hello!"));
        // "ab"+"c" is not "a"+"bc"
        assert_ne!(k("a", "bc", "", 1.0, ""), k("ab", "c", "", 1.0, ""));
    }

    #[test]
    fn sentences_come_back_as_they_went_in() {
        let d = tmp();
        let c = SentenceCache::new(&d, 10);
        assert!(c.get("book1", "k").is_none());
        let p = pcm(1000);
        c.put("book1", "k", &p);
        let got = c.get("book1", "k").unwrap();
        assert_eq!(got.rate, 24000);
        assert_eq!(got.samples.len(), 1000);
        assert!((got.samples[150] - p.samples[150]).abs() < 1e-3);
        assert!(c.get("book2", "k").is_none(), "books are kept apart");
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn the_oldest_sentences_go_when_the_cap_is_hit() {
        let d = tmp();
        // cap: 1 MB; each sentence is ~400 KB
        let c = SentenceCache::new(&d, 1);
        for i in 0..4 {
            c.put("b", &format!("k{i}"), &pcm(200_000));
            std::thread::sleep(std::time::Duration::from_millis(15));
            if i == 1 {
                // k0 is used again: k1 becomes the oldest
                assert!(c.get("b", "k0").is_some());
            }
        }
        assert!(c.size_bytes() <= 1024 * 1024, "{}", c.size_bytes());
        assert!(c.get("b", "k3").is_some(), "the newest stays");
        assert!(c.get("b", "k1").is_none(), "the least recently used goes");
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_zero_cap_stores_nothing_and_clear_forgets() {
        let d = tmp();
        let off = SentenceCache::new(&d, 0);
        off.put("b", "k", &pcm(100));
        assert_eq!(off.size_bytes(), 0);
        let c = SentenceCache::new(&d, 5);
        c.put("a", "k", &pcm(100));
        c.put("b", "k", &pcm(100));
        c.clear("a");
        assert!(c.get("a", "k").is_none());
        assert!(c.get("b", "k").is_some());
        c.clear("");
        assert_eq!(c.size_bytes(), 0);
        fs::remove_dir_all(&d).ok();
    }
}
