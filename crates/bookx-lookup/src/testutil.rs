//! Test helpers: unique temp dirs and hand-assembled StarDict files.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct TmpDir(PathBuf);

impl TmpDir {
    pub fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!(
            "bookx-lookup-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&p).expect("tmp dir");
        Self(p)
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub struct DictSpec<'a> {
    pub name: &'a str,
    pub entries: Vec<(&'a str, Vec<u8>)>,
    pub bits: u8,
    pub sts: Option<&'a str>,
    pub dz: bool,
    pub syn: Vec<(&'a str, u32)>,
}

impl<'a> DictSpec<'a> {
    pub fn new(name: &'a str, entries: Vec<(&'a str, Vec<u8>)>) -> Self {
        Self {
            name,
            entries,
            bits: 32,
            sts: Some("m"),
            dz: false,
            syn: Vec::new(),
        }
    }
}

fn order(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_ascii_lowercase()
        .cmp(&b.to_ascii_lowercase())
        .then(a.cmp(b))
}

/// Writes `<name>.ifo/.idx/.dict[.dz]/.syn` into `dir`, returns the ifo path.
pub fn write_dict(dir: &Path, spec: &DictSpec) -> PathBuf {
    let mut entries = spec.entries.clone();
    entries.sort_by(|a, b| order(a.0, b.0));
    let mut idx = Vec::new();
    let mut dict = Vec::new();
    for (w, data) in &entries {
        idx.extend_from_slice(w.as_bytes());
        idx.push(0);
        if spec.bits == 64 {
            idx.extend_from_slice(&(dict.len() as u64).to_be_bytes());
        } else {
            idx.extend_from_slice(&(dict.len() as u32).to_be_bytes());
        }
        idx.extend_from_slice(&(data.len() as u32).to_be_bytes());
        dict.extend_from_slice(data);
    }
    let mut ifo = format!(
        "StarDict's dict ifo file\nversion=2.4.2\nbookname={}\nwordcount={}\nidxfilesize={}\n",
        spec.name,
        entries.len(),
        idx.len()
    );
    if let Some(s) = spec.sts {
        ifo.push_str(&format!("sametypesequence={s}\n"));
    }
    if spec.bits == 64 {
        ifo.push_str("idxoffsetbits=64\n");
    }
    if !spec.syn.is_empty() {
        ifo.push_str(&format!("synwordcount={}\n", spec.syn.len()));
    }
    let ifo_path = dir.join(format!("{}.ifo", spec.name));
    fs::write(&ifo_path, ifo).unwrap();
    fs::write(dir.join(format!("{}.idx", spec.name)), idx).unwrap();
    if spec.dz {
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(&dict).unwrap();
        fs::write(
            dir.join(format!("{}.dict.dz", spec.name)),
            enc.finish().unwrap(),
        )
        .unwrap();
    } else {
        fs::write(dir.join(format!("{}.dict", spec.name)), dict).unwrap();
    }
    if !spec.syn.is_empty() {
        let mut syn = spec.syn.clone();
        syn.sort_by(|a, b| order(a.0, b.0));
        let mut bytes = Vec::new();
        for (w, i) in syn {
            bytes.extend_from_slice(w.as_bytes());
            bytes.push(0);
            bytes.extend_from_slice(&i.to_be_bytes());
        }
        fs::write(dir.join(format!("{}.syn", spec.name)), bytes).unwrap();
    }
    ifo_path
}
