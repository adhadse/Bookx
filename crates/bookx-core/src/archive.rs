//! An in-memory EPUB (zip) container that can be read, modified and written back.

use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use std::path::Path;

use anyhow::{Context, Result};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

#[derive(Debug, Clone, Default)]
pub struct Archive {
    order: Vec<String>,
    files: HashMap<String, Vec<u8>>,
}

impl Archive {
    pub fn open(path: &Path) -> Result<Self> {
        let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        Self::from_bytes(data)
    }

    /// Largest size one member may expand to.
    pub const MAX_MEMBER: u64 = crate::limits::ARCHIVE_MEMBER_BYTES;
    /// Largest total size of all members.
    pub const MAX_TOTAL: u64 = crate::limits::ARCHIVE_TOTAL_BYTES;

    pub fn from_bytes(data: Vec<u8>) -> Result<Self> {
        let mut zip = ZipArchive::new(Cursor::new(data)).context("not a valid zip/epub file")?;
        if zip.len() > 200_000 {
            anyhow::bail!(
                "the zip file has {} members (corrupt or hostile file)",
                zip.len()
            );
        }
        let mut arc = Archive::default();
        let mut total = 0u64;
        for i in 0..zip.len() {
            let mut f = zip.by_index(i)?;
            if f.is_dir() {
                continue;
            }
            let name = f.name()?.to_string();
            // the declared size is only a hint: never allocate more than a sane first guess
            let mut buf = Vec::with_capacity((f.size() as usize).min(16 << 20));
            (&mut f).take(Self::MAX_MEMBER + 1).read_to_end(&mut buf)?;
            if buf.len() as u64 > Self::MAX_MEMBER {
                anyhow::bail!(
                    "`{name}` expands to more than {} MB (zip bomb?)",
                    Self::MAX_MEMBER >> 20
                );
            }
            total += buf.len() as u64;
            if total > Self::MAX_TOTAL {
                anyhow::bail!(
                    "the zip file expands to more than {} GB (zip bomb?)",
                    Self::MAX_TOTAL >> 30
                );
            }
            arc.set(&name, buf);
        }
        Ok(arc)
    }

    pub fn names(&self) -> &[String] {
        &self.order
    }

    pub fn contains(&self, name: &str) -> bool {
        self.files.contains_key(name)
    }

    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.files.get(name).map(|v| v.as_slice())
    }

    pub fn get_string(&self, name: &str) -> Option<String> {
        self.get(name).map(|b| {
            String::from_utf8_lossy(b)
                .trim_start_matches('\u{feff}')
                .to_string()
        })
    }

    pub fn size_of(&self, name: &str) -> usize {
        self.files.get(name).map(|v| v.len()).unwrap_or(0)
    }

    pub fn set(&mut self, name: &str, data: Vec<u8>) {
        if self.files.insert(name.to_string(), data).is_none() {
            self.order.push(name.to_string());
        }
    }

    pub fn set_string(&mut self, name: &str, text: &str) {
        self.set(name, text.as_bytes().to_vec());
    }

    pub fn remove(&mut self, name: &str) -> bool {
        if self.files.remove(name).is_some() {
            self.order.retain(|n| n != name);
            true
        } else {
            false
        }
    }

    pub fn rename(&mut self, from: &str, to: &str) -> bool {
        if from == to || self.files.contains_key(to) {
            return false;
        }
        if let Some(data) = self.files.remove(from) {
            for n in self.order.iter_mut() {
                if n == from {
                    *n = to.to_string();
                }
            }
            self.files.insert(to.to_string(), data);
            true
        } else {
            false
        }
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut out = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut out);
            let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            let deflated =
                SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

            // The EPUB spec requires `mimetype` to be the first, uncompressed entry.
            let mimetype = self
                .get("mimetype")
                .map(|m| m.to_vec())
                .unwrap_or_else(|| b"application/epub+zip".to_vec());
            zip.start_file("mimetype", stored)?;
            zip.write_all(&mimetype)?;

            for name in &self.order {
                if name == "mimetype" {
                    continue;
                }
                let data = &self.files[name];
                let opts = if is_already_compressed(name) {
                    stored
                } else {
                    deflated
                };
                zip.start_file(name, opts)?;
                zip.write_all(data)?;
            }
            zip.finish()?;
        }
        Ok(out.into_inner())
    }

    /// Atomically writes the archive to `path`.
    pub fn save(&self, path: &Path) -> Result<()> {
        let bytes = self.to_bytes()?;
        let tmp = path.with_extension("epub.bookx-tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

fn is_already_compressed(name: &str) -> bool {
    matches!(
        super::util::extension(name).as_str(),
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "woff" | "woff2" | "mp3" | "mp4"
    )
}
