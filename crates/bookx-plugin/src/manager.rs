//! Discovery, enabling and lazy loading of installed plugins.
//!
//! Layout: `<plugins dir>/<any folder name>/plugin.toml` + the `.wasm` file. Plugins start
//! **disabled** (the user must opt in after seeing the permissions); the set of enabled ids is stored
//! in a small JSON state file chosen by the caller: `{"enabled": ["id", …]}`. A corrupt state file is
//! treated as empty and reported by [`PluginManager::state_error`].

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::{anyhow, bail, Context, Result};
use bookx_core::contracts::{
    DeviceDriver, FormatReader, FormatWriter, MetadataSource, Registry, Transform,
};
use serde::{Deserialize, Serialize};

use crate::adapters::{
    MountProvider, PluginDevice, PluginMetadataSource, PluginReader, PluginTransform, PluginWriter,
};
use crate::manifest::{Manifest, PluginKind};
use crate::runtime::{Host, Plugin};

#[derive(Debug, Default, Serialize, Deserialize)]
struct StateFile {
    #[serde(default)]
    enabled: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Stamp {
    manifest: Option<SystemTime>,
    wasm: Option<SystemTime>,
    wasm_len: Option<u64>,
}

fn stamp(dir: &Path, wasm_name: Option<&str>) -> Stamp {
    let mtime = |p: PathBuf| std::fs::metadata(p).ok();
    let m = mtime(dir.join("plugin.toml"));
    let w = wasm_name.and_then(|n| mtime(dir.join(n)));
    Stamp {
        manifest: m.and_then(|m| m.modified().ok()),
        wasm_len: w.as_ref().map(|w| w.len()),
        wasm: w.and_then(|w| w.modified().ok()),
    }
}

struct Entry {
    dir: PathBuf,
    folder: String,
    manifest: std::result::Result<Manifest, String>,
    stamp: Stamp,
    loaded: Option<std::result::Result<Arc<Plugin>, String>>,
}

impl Entry {
    fn key(&self) -> &str {
        match &self.manifest {
            Ok(m) => &m.id,
            Err(_) => &self.folder,
        }
    }
}

/// What the UI shows for one installed plugin.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginStatus {
    /// Plugin id, or the folder name when the manifest is invalid.
    pub id: String,
    pub name: String,
    pub version: String,
    pub kind: Option<PluginKind>,
    pub description: String,
    pub dir: PathBuf,
    pub permissions: Vec<String>,
    pub enabled: bool,
    /// The module is compiled and cached right now.
    pub loaded: bool,
    /// Manifest or load problem, if any.
    pub error: Option<String>,
}

/// What a [`PluginManager::scan`] changed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ScanReport {
    pub added: Vec<String>,
    /// Manifest or wasm changed on disk: the cached module was dropped and reloads on next use.
    pub changed: Vec<String>,
    pub removed: Vec<String>,
}

pub struct PluginManager {
    dir: PathBuf,
    state_path: PathBuf,
    host: Host,
    entries: Vec<Entry>,
    enabled: BTreeSet<String>,
    state_error: Option<String>,
}

impl PluginManager {
    /// Creates a manager and reads the state file; call [`PluginManager::scan`] to discover plugins.
    pub fn new(dir: impl Into<PathBuf>, state_path: impl Into<PathBuf>, host: Host) -> Self {
        let mut m = PluginManager {
            dir: dir.into(),
            state_path: state_path.into(),
            host,
            entries: Vec::new(),
            enabled: BTreeSet::new(),
            state_error: None,
        };
        m.read_state();
        m
    }

    fn read_state(&mut self) {
        self.enabled.clear();
        self.state_error = None;
        let text = match std::fs::read_to_string(&self.state_path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => {
                self.state_error = Some(format!("cannot read plugin state: {e}"));
                return;
            }
        };
        match serde_json::from_str::<StateFile>(&text) {
            Ok(s) => self.enabled = s.enabled.into_iter().collect(),
            Err(e) => self.state_error = Some(format!("plugin state is corrupt: {e}")),
        }
    }

    fn write_state(&self) -> Result<()> {
        let state = StateFile {
            enabled: self.enabled.iter().cloned().collect(),
        };
        if let Some(parent) = self.state_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.state_path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&state)?)
            .with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &self.state_path)
            .with_context(|| format!("writing {}", self.state_path.display()))?;
        Ok(())
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Set when the state file could not be read or parsed (all plugins then count as disabled).
    pub fn state_error(&self) -> Option<&str> {
        self.state_error.as_deref()
    }

    /// (Re)reads the plugins folder. Entries whose files did not change keep their cached module;
    /// changed ones are dropped (hot reload), vanished ones forgotten.
    pub fn scan(&mut self) -> ScanReport {
        let mut folders: Vec<(String, PathBuf)> = std::fs::read_dir(&self.dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| e.path().is_dir())
                    .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
                    .collect()
            })
            .unwrap_or_default();
        folders.sort();
        let mut old: Vec<Entry> = std::mem::take(&mut self.entries);
        let mut report = ScanReport::default();
        let mut seen_ids: Vec<(String, String)> = Vec::new();
        let mut entries = Vec::new();
        for (folder, path) in folders {
            let manifest = Manifest::load(&path).map_err(|e| format!("{e:#}"));
            let manifest = match manifest {
                Ok(m) => match seen_ids.iter().find(|(id, _)| *id == m.id) {
                    Some((_, first)) => Err(format!(
                        "duplicate plugin id `{}` (already provided by folder `{first}`)",
                        m.id
                    )),
                    None => {
                        seen_ids.push((m.id.clone(), folder.clone()));
                        Ok(m)
                    }
                },
                Err(e) => Err(e),
            };
            let wasm_name = manifest.as_ref().ok().map(|m| m.wasm.clone());
            let st = stamp(&path, wasm_name.as_deref());
            let prev = old
                .iter()
                .position(|o| o.folder == folder)
                .map(|i| old.remove(i));
            let mut entry = Entry {
                dir: path,
                folder,
                manifest,
                stamp: st,
                loaded: None,
            };
            match prev {
                Some(p) => {
                    let same = p.stamp == entry.stamp
                        && match (&p.manifest, &entry.manifest) {
                            (Ok(a), Ok(b)) => a == b,
                            (Err(a), Err(b)) => a == b,
                            _ => false,
                        };
                    if same {
                        entry.loaded = p.loaded;
                    } else {
                        report.changed.push(entry.key().to_string());
                    }
                }
                None => report.added.push(entry.key().to_string()),
            }
            entries.push(entry);
        }
        report.removed = old.iter().map(|o| o.key().to_string()).collect();
        self.entries = entries;
        report
    }

    /// Alias of [`PluginManager::scan`]: hot reload by modification time.
    pub fn reload_changed(&mut self) -> ScanReport {
        self.scan()
    }

    pub fn is_enabled(&self, id: &str) -> bool {
        self.enabled.contains(id)
    }

    fn find(&self, id: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.key() == id)
    }

    /// Enables or disables a plugin and saves the state. Enabling checks that the module loads and
    /// refuses (leaving it disabled) when it does not.
    pub fn set_enabled(&mut self, id: &str, on: bool) -> Result<()> {
        let idx = self
            .find(id)
            .ok_or_else(|| anyhow!("no plugin with id `{id}`"))?;
        if on {
            self.load_entry(idx)
                .with_context(|| format!("cannot enable `{id}`"))?;
            self.enabled.insert(id.to_string());
        } else {
            self.enabled.remove(id);
        }
        self.write_state()?;
        self.state_error = None;
        Ok(())
    }

    fn load_entry(&mut self, idx: usize) -> Result<Arc<Plugin>> {
        let host = self.host.clone();
        let e = &mut self.entries[idx];
        let manifest = match &e.manifest {
            Ok(m) => m.clone(),
            Err(msg) => bail!("{msg}"),
        };
        if e.loaded.is_none() {
            e.loaded = Some(
                Plugin::load(&e.dir, manifest, host)
                    .map(Arc::new)
                    .map_err(|err| format!("{err:#}")),
            );
        }
        match e.loaded.as_ref() {
            Some(Ok(p)) => Ok(p.clone()),
            Some(Err(msg)) => bail!("{msg}"),
            None => unreachable!("set above"),
        }
    }

    /// The loaded plugin (compiled on first use). Fails when unknown, invalid, disabled or broken.
    pub fn plugin(&mut self, id: &str) -> Result<Arc<Plugin>> {
        let idx = self
            .find(id)
            .ok_or_else(|| anyhow!("no plugin with id `{id}`"))?;
        if !self.is_enabled(id) && self.entries[idx].manifest.is_ok() {
            bail!("plugin `{id}` is disabled");
        }
        self.load_entry(idx)
    }

    /// Everything installed, valid or not, sorted by folder name.
    pub fn list(&self) -> Vec<PluginStatus> {
        self.entries
            .iter()
            .map(|e| {
                let (name, version, kind, description, permissions, mut error) = match &e.manifest {
                    Ok(m) => (
                        m.name.clone(),
                        m.version.clone(),
                        Some(m.kind),
                        m.description.clone(),
                        m.permissions
                            .iter()
                            .map(|p| p.to_manifest_string())
                            .collect(),
                        None,
                    ),
                    Err(msg) => (
                        e.folder.clone(),
                        String::new(),
                        None,
                        String::new(),
                        Vec::new(),
                        Some(msg.clone()),
                    ),
                };
                if let Some(Err(msg)) = &e.loaded {
                    error = Some(msg.clone());
                }
                PluginStatus {
                    id: e.key().to_string(),
                    name,
                    version,
                    kind,
                    description,
                    dir: e.dir.clone(),
                    permissions,
                    enabled: e.manifest.is_ok() && self.enabled.contains(e.key()),
                    loaded: matches!(e.loaded, Some(Ok(_))),
                    error,
                }
            })
            .collect()
    }

    fn enabled_of_kind(&mut self, kind: PluginKind) -> (Vec<Arc<Plugin>>, Vec<(String, String)>) {
        let ids: Vec<String> = self
            .entries
            .iter()
            .filter_map(|e| e.manifest.as_ref().ok())
            .filter(|m| m.kind == kind && self.enabled.contains(&m.id))
            .map(|m| m.id.clone())
            .collect();
        let mut ok = Vec::new();
        let mut errors = Vec::new();
        for id in ids {
            match self.plugin(&id) {
                Ok(p) => ok.push(p),
                Err(e) => errors.push((id, format!("{e:#}"))),
            }
        }
        (ok, errors)
    }

    /// Registers every enabled `metadata-source` plugin; returns `(id, error)` for those that failed.
    pub fn register_metadata_sources(
        &mut self,
        reg: &mut Registry<dyn MetadataSource>,
    ) -> Vec<(String, String)> {
        let (plugins, mut errors) = self.enabled_of_kind(PluginKind::MetadataSource);
        for p in plugins {
            let id = p.id().to_string();
            match PluginMetadataSource::new(p) {
                Ok(a) => reg.register(Box::new(a)),
                Err(e) => errors.push((id, format!("{e:#}"))),
            }
        }
        errors
    }

    pub fn register_readers(
        &mut self,
        reg: &mut Registry<dyn FormatReader>,
    ) -> Vec<(String, String)> {
        let (plugins, mut errors) = self.enabled_of_kind(PluginKind::FormatReader);
        for p in plugins {
            let id = p.id().to_string();
            match PluginReader::new(p) {
                Ok(a) => reg.register(Box::new(a)),
                Err(e) => errors.push((id, format!("{e:#}"))),
            }
        }
        errors
    }

    pub fn register_writers(
        &mut self,
        reg: &mut Registry<dyn FormatWriter>,
    ) -> Vec<(String, String)> {
        let (plugins, mut errors) = self.enabled_of_kind(PluginKind::FormatWriter);
        for p in plugins {
            let id = p.id().to_string();
            match PluginWriter::new(p) {
                Ok(a) => reg.register(Box::new(a)),
                Err(e) => errors.push((id, format!("{e:#}"))),
            }
        }
        errors
    }

    pub fn register_transforms(
        &mut self,
        reg: &mut Registry<dyn Transform>,
    ) -> Vec<(String, String)> {
        let (plugins, mut errors) = self.enabled_of_kind(PluginKind::Transform);
        for p in plugins {
            let id = p.id().to_string();
            match PluginTransform::new(p) {
                Ok(a) => reg.register(Box::new(a)),
                Err(e) => errors.push((id, format!("{e:#}"))),
            }
        }
        errors
    }

    /// Registers every enabled `device` plugin. `mounts` tells them which folders to look at.
    pub fn register_devices(
        &mut self,
        reg: &mut Registry<dyn DeviceDriver>,
        mounts: MountProvider,
    ) -> Vec<(String, String)> {
        let (plugins, mut errors) = self.enabled_of_kind(PluginKind::Device);
        for p in plugins {
            let id = p.id().to_string();
            match PluginDevice::new(p, mounts.clone()) {
                Ok(a) => reg.register(Box::new(a)),
                Err(e) => errors.push((id, format!("{e:#}"))),
            }
        }
        errors
    }
}
