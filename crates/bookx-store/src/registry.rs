//! A small JSON file listing named libraries and which one is current.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryEntry {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Data {
    #[serde(default)]
    current: Option<String>,
    #[serde(default)]
    libraries: Vec<LibraryEntry>,
}

/// Registry of libraries; every change is written to disk immediately.
#[derive(Debug)]
pub struct LibraryRegistry {
    file: PathBuf,
    data: Data,
}

impl LibraryRegistry {
    /// Loads the registry; a missing file gives an empty registry.
    pub fn open(file: &Path) -> Result<LibraryRegistry> {
        let data = match std::fs::read_to_string(file) {
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("corrupt library registry {}", file.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Data::default(),
            Err(e) => return Err(e).with_context(|| format!("cannot read {}", file.display())),
        };
        Ok(LibraryRegistry {
            file: file.to_path_buf(),
            data,
        })
    }

    fn save(&self) -> Result<()> {
        if let Some(dir) = self.file.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)?;
            }
        }
        let mut tmp = self.file.clone().into_os_string();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        std::fs::write(&tmp, serde_json::to_string_pretty(&self.data)?)?;
        std::fs::rename(&tmp, &self.file)?;
        Ok(())
    }

    pub fn list(&self) -> &[LibraryEntry] {
        &self.data.libraries
    }

    pub fn get(&self, name: &str) -> Option<&LibraryEntry> {
        self.data.libraries.iter().find(|l| l.name == name)
    }

    pub fn current(&self) -> Option<&LibraryEntry> {
        self.data.current.as_deref().and_then(|n| self.get(n))
    }

    /// Adds a library; the first one added becomes current.
    pub fn add(&mut self, name: &str, path: &Path) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            bail!("library name is empty");
        }
        if self.get(name).is_some() {
            bail!("library `{name}` already exists");
        }
        self.data.libraries.push(LibraryEntry {
            name: name.to_string(),
            path: path.to_path_buf(),
        });
        if self.data.current.is_none() {
            self.data.current = Some(name.to_string());
        }
        self.save()
    }

    /// Removes the entry (never the files). The current library falls back to the first remaining.
    pub fn remove(&mut self, name: &str) -> Result<bool> {
        let Some(pos) = self.data.libraries.iter().position(|l| l.name == name) else {
            return Ok(false);
        };
        self.data.libraries.remove(pos);
        if self.data.current.as_deref() == Some(name) {
            self.data.current = self.data.libraries.first().map(|l| l.name.clone());
        }
        self.save()?;
        Ok(true)
    }

    pub fn rename(&mut self, old: &str, new: &str) -> Result<()> {
        let new = new.trim();
        if new.is_empty() {
            bail!("library name is empty");
        }
        if self.get(old).is_none() {
            bail!("no library named `{old}`");
        }
        if old != new && self.get(new).is_some() {
            bail!("library `{new}` already exists");
        }
        for l in &mut self.data.libraries {
            if l.name == old {
                l.name = new.to_string();
            }
        }
        if self.data.current.as_deref() == Some(old) {
            self.data.current = Some(new.to_string());
        }
        self.save()
    }

    pub fn set_current(&mut self, name: &str) -> Result<()> {
        if self.get(name).is_none() {
            bail!("no library named `{name}`");
        }
        self.data.current = Some(name.to_string());
        self.save()
    }
}
