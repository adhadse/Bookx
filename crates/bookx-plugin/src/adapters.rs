//! Adapters that make a [`Plugin`] usable through the `bookx_core::contracts` traits.
//!
//! Operations and their JSON (all replies are wrapped in `{"ok": …}` / `{"err": "…"}`):
//!
//! | kind | op | input | `ok` value |
//! |---|---|---|---|
//! | metadata-source | `lookup` | `{"title","authors":[],"isbn":null\|"…"}` | array of metadata objects |
//! | metadata-source | `cover` | one metadata object | `null` or base64 image |
//! | format-reader | `sniff` | `{"head_b64"}` (first 4 KiB) | `true`/`false` |
//! | format-reader | `read` | `{"data_b64"}` | base64 EPUB (zip) |
//! | transform | `apply` | `{"book_b64"}` (EPUB zip) | `{"book_b64"?,"files_changed","notes":[]}` |
//! | format-writer | `write` | `{"book_b64"}` (EPUB zip) | base64 of the output file |
//! | device | `detect`, `send`, `list` | see [`PluginDevice`] | see [`PluginDevice`] |
//!
//! Metadata objects use the field names of `bookx_core::opf::Metadata`; every field is optional in
//! replies.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use bookx_core::archive::Archive;
use bookx_core::book::OpenBook;
use bookx_core::contracts::{
    DeviceDriver, DeviceInfo, FormatReader, FormatWriter, MetadataQuery, MetadataSource, Transform,
    TransformReport,
};
use bookx_core::opf::{Identifier, Metadata};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::manifest::PluginKind;
use crate::runtime::Plugin;

/// Bytes of the file start handed to `sniff`.
pub const SNIFF_BYTES: usize = 4096;

fn interner() -> &'static Mutex<HashMap<String, &'static str>> {
    static I: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    I.get_or_init(Default::default)
}

/// The contracts want `&'static str`; leak each distinct string once (reloads reuse it).
fn intern(s: &str) -> &'static str {
    let mut map = interner().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(v) = map.get(s) {
        return v;
    }
    let leaked: &'static str = Box::leak(s.to_string().into_boxed_str());
    map.insert(s.to_string(), leaked);
    leaked
}

fn intern_slice(items: &[String]) -> &'static [&'static str] {
    static S: OnceLock<Mutex<HashMap<String, &'static [&'static str]>>> = OnceLock::new();
    let key = items.join("\0");
    let mut map = S
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(v) = map.get(&key) {
        return v;
    }
    let v: Vec<&'static str> = items.iter().map(|s| intern(s)).collect();
    let leaked: &'static [&'static str] = Box::leak(v.into_boxed_slice());
    map.insert(key, leaked);
    leaked
}

fn expect_kind(plugin: &Plugin, kind: PluginKind) -> Result<()> {
    if plugin.manifest().kind != kind {
        bail!(
            "plugin `{}` is a {}, not a {}",
            plugin.id(),
            plugin.manifest().kind.as_str(),
            kind.as_str()
        );
    }
    Ok(())
}

fn decode_b64(what: &str, v: &Value) -> Result<Vec<u8>> {
    let s = v
        .as_str()
        .ok_or_else(|| anyhow!("{what} is not a base64 string"))?;
    B64.decode(s.trim())
        .map_err(|e| anyhow!("{what} is not valid base64: {e}"))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct MetadataDto {
    title: String,
    title_sort: String,
    authors: Vec<String>,
    author_sort: String,
    publisher: String,
    language: String,
    description: String,
    tags: Vec<String>,
    series: String,
    series_index: f64,
    identifiers: Vec<IdentifierDto>,
    published: String,
    rights: String,
    rating: u8,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct IdentifierDto {
    scheme: String,
    value: String,
}

impl From<MetadataDto> for Metadata {
    fn from(d: MetadataDto) -> Metadata {
        Metadata {
            title: d.title,
            title_sort: d.title_sort,
            authors: d.authors,
            author_sort: d.author_sort,
            publisher: d.publisher,
            language: d.language,
            description: d.description,
            tags: d.tags,
            series: d.series,
            series_index: d.series_index,
            identifiers: d
                .identifiers
                .into_iter()
                .map(|i| Identifier {
                    scheme: i.scheme,
                    value: i.value,
                    xml_id: None,
                })
                .collect(),
            published: d.published,
            rights: d.rights,
            rating: d.rating.min(10),
        }
    }
}

/// Decodes a plugin's metadata array (missing fields default; rating is clamped to 0..=10).
pub fn decode_metadata_list(v: Value) -> Result<Vec<Metadata>> {
    let items: Vec<MetadataDto> =
        serde_json::from_value(v).context("reply is not an array of metadata objects")?;
    Ok(items.into_iter().map(Metadata::from).collect())
}

/// `MetadataSource` backed by a plugin of kind `metadata-source`.
pub struct PluginMetadataSource {
    plugin: Arc<Plugin>,
    id: &'static str,
}

impl PluginMetadataSource {
    pub fn new(plugin: Arc<Plugin>) -> Result<Self> {
        expect_kind(&plugin, PluginKind::MetadataSource)?;
        let id = intern(plugin.id());
        Ok(Self { plugin, id })
    }
}

impl MetadataSource for PluginMetadataSource {
    fn id(&self) -> &'static str {
        self.id
    }

    fn lookup(&self, query: &MetadataQuery) -> Result<Vec<Metadata>> {
        let input = json!({
            "title": query.title,
            "authors": query.authors,
            "isbn": query.isbn,
        });
        decode_metadata_list(self.plugin.call_json("lookup", &input)?)
            .with_context(|| format!("plugin `{}`", self.id))
    }

    fn cover(&self, meta: &Metadata) -> Result<Option<Vec<u8>>> {
        let v = self
            .plugin
            .call_json("cover", &serde_json::to_value(meta)?)?;
        if v.is_null() {
            return Ok(None);
        }
        decode_b64("cover", &v)
            .map(Some)
            .with_context(|| format!("plugin `{}`", self.id))
    }
}

/// `FormatReader` backed by a plugin of kind `format-reader`.
pub struct PluginReader {
    plugin: Arc<Plugin>,
    id: &'static str,
    extensions: &'static [&'static str],
}

impl PluginReader {
    pub fn new(plugin: Arc<Plugin>) -> Result<Self> {
        expect_kind(&plugin, PluginKind::FormatReader)?;
        let id = intern(plugin.id());
        let extensions = intern_slice(&plugin.manifest().extensions);
        Ok(Self {
            plugin,
            id,
            extensions,
        })
    }
}

impl FormatReader for PluginReader {
    fn id(&self) -> &'static str {
        self.id
    }

    fn extensions(&self) -> &'static [&'static str] {
        self.extensions
    }

    fn sniff(&self, head: &[u8]) -> bool {
        let head = &head[..head.len().min(SNIFF_BYTES)];
        self.plugin
            .call_json("sniff", &json!({ "head_b64": B64.encode(head) }))
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    fn read(&self, bytes: &[u8]) -> Result<Archive> {
        let v = self
            .plugin
            .call_json("read", &json!({ "data_b64": B64.encode(bytes) }))?;
        let zip = decode_b64("reply", &v).with_context(|| format!("plugin `{}`", self.id))?;
        Archive::from_bytes(zip)
            .with_context(|| format!("plugin `{}` returned an invalid EPUB", self.id))
    }
}

/// `FormatWriter` backed by a plugin of kind `format-writer`.
pub struct PluginWriter {
    plugin: Arc<Plugin>,
    id: &'static str,
    extension: &'static str,
}

impl PluginWriter {
    pub fn new(plugin: Arc<Plugin>) -> Result<Self> {
        expect_kind(&plugin, PluginKind::FormatWriter)?;
        let id = intern(plugin.id());
        let extension = intern(plugin.manifest().extension.as_deref().unwrap_or(""));
        Ok(Self {
            plugin,
            id,
            extension,
        })
    }
}

impl FormatWriter for PluginWriter {
    fn id(&self) -> &'static str {
        self.id
    }

    fn extension(&self) -> &'static str {
        self.extension
    }

    fn write(&self, book: &OpenBook) -> Result<Vec<u8>> {
        let zip = book.archive.to_bytes()?;
        let v = self
            .plugin
            .call_json("write", &json!({ "book_b64": B64.encode(zip) }))?;
        decode_b64("reply", &v).with_context(|| format!("plugin `{}`", self.id))
    }
}

/// `Transform` backed by a plugin of kind `transform`.
pub struct PluginTransform {
    plugin: Arc<Plugin>,
    id: &'static str,
}

impl PluginTransform {
    pub fn new(plugin: Arc<Plugin>) -> Result<Self> {
        expect_kind(&plugin, PluginKind::Transform)?;
        let id = intern(plugin.id());
        Ok(Self { plugin, id })
    }
}

impl Transform for PluginTransform {
    fn id(&self) -> &'static str {
        self.id
    }

    fn apply(&self, book: &mut OpenBook) -> Result<TransformReport> {
        let zip = book.archive.to_bytes()?;
        let v = self
            .plugin
            .call_json("apply", &json!({ "book_b64": B64.encode(zip) }))?;
        let obj = v
            .as_object()
            .ok_or_else(|| anyhow!("plugin `{}`: reply is not an object", self.id))?;
        let files_changed = match obj.get("files_changed") {
            None | Some(Value::Null) => 0,
            Some(n) => n
                .as_u64()
                .ok_or_else(|| anyhow!("plugin `{}`: files_changed is not a number", self.id))?
                as usize,
        };
        let notes = match obj.get("notes") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(a)) => a
                .iter()
                .map(|n| {
                    n.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| anyhow!("plugin `{}`: a note is not a string", self.id))
                })
                .collect::<Result<Vec<_>>>()?,
            Some(_) => bail!("plugin `{}`: notes is not an array", self.id),
        };
        match obj.get("book_b64") {
            None | Some(Value::Null) => {}
            Some(b) => {
                let bytes =
                    decode_b64("book_b64", b).with_context(|| format!("plugin `{}`", self.id))?;
                let archive = Archive::from_bytes(bytes)
                    .with_context(|| format!("plugin `{}` returned an invalid EPUB", self.id))?;
                *book = OpenBook::from_archive(archive)
                    .with_context(|| format!("plugin `{}` returned an invalid EPUB", self.id))?;
            }
        }
        Ok(TransformReport {
            files_changed,
            notes,
        })
    }
}

/// Folders (mount points) a device plugin may look at. Called on every `detect`.
pub type MountProvider = Arc<dyn Fn() -> Vec<PathBuf> + Send + Sync>;

/// Most entries the host lists per call to a device plugin.
pub const MAX_LISTED: usize = 2000;

/// `DeviceDriver` backed by a plugin of kind `device`.
///
/// The plugin has no file system. For `detect` the host lists the top-level entries of every
/// candidate mount point and the plugin recognises its device from those names; to `send` it names
/// the path (relative to the mount point) a book goes to, and the host does the writing after
/// checking that the path stays inside the mount point.
///
/// | op | input | `ok` value |
/// |---|---|---|
/// | `detect` | `{"mounts":[{"path","name","entries":[…]}]}` | `[{"id","name","mount"}]`, `mount` is one of the given paths |
/// | `send` | `{"device":{"id","name","mount"},"file_name","data_b64"}` | `{"path":"rel/dir/file","data_b64"?}` (optional replacement bytes) |
/// | `list` | `{"device":{…},"files":["rel/path",…]}` | array of the relative paths that are books |
///
/// `preferred_formats` are the manifest's `extensions`.
pub struct PluginDevice {
    plugin: Arc<Plugin>,
    id: &'static str,
    formats: &'static [&'static str],
    mounts: MountProvider,
}

impl PluginDevice {
    pub fn new(plugin: Arc<Plugin>, mounts: MountProvider) -> Result<Self> {
        expect_kind(&plugin, PluginKind::Device)?;
        let id = intern(plugin.id());
        let formats = intern_slice(&plugin.manifest().extensions);
        Ok(Self {
            plugin,
            id,
            formats,
            mounts,
        })
    }

    fn device_json(d: &DeviceInfo) -> Value {
        json!({ "id": d.id, "name": d.name, "mount": d.mount.to_string_lossy() })
    }

    fn own<'a>(&self, device: &'a DeviceInfo) -> Result<&'a str> {
        device
            .id
            .strip_prefix(self.id)
            .and_then(|r| r.strip_prefix(':'))
            .ok_or_else(|| anyhow!("`{}` is not a device of plugin `{}`", device.id, self.id))
    }
}

/// A path the plugin named, made safe: relative, no `..`, no empty parts, no hidden-root tricks.
pub fn safe_relative(p: &str) -> Result<PathBuf> {
    let path = Path::new(p);
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Normal(s) if !s.is_empty() => out.push(s),
            Component::CurDir => {}
            _ => bail!("the path `{p}` leaves the device"),
        }
    }
    if out.as_os_str().is_empty() {
        bail!("the plugin gave an empty path");
    }
    Ok(out)
}

fn list_top(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v.truncate(200);
    v
}

fn walk_files(root: &Path, dir: &Path, out: &mut Vec<String>, depth: usize) {
    if depth > 6 || out.len() >= MAX_LISTED {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut items: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    items.sort();
    for p in items {
        if out.len() >= MAX_LISTED {
            return;
        }
        let Ok(meta) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        if meta.is_dir() {
            walk_files(root, &p, out, depth + 1);
        } else if meta.is_file() {
            if let Ok(rel) = p.strip_prefix(root) {
                out.push(rel.to_string_lossy().into_owned());
            }
        }
    }
}

impl DeviceDriver for PluginDevice {
    fn id(&self) -> &'static str {
        self.id
    }

    fn detect(&self) -> Vec<DeviceInfo> {
        let mounts: Vec<PathBuf> = (self.mounts)();
        let given: Vec<Value> = mounts
            .iter()
            .map(|m| {
                json!({
                    "path": m.to_string_lossy(),
                    "name": m.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                    "entries": list_top(m),
                })
            })
            .collect();
        let Ok(v) = self.plugin.call_json("detect", &json!({ "mounts": given })) else {
            return Vec::new();
        };
        let Some(items) = v.as_array() else {
            return Vec::new();
        };
        items
            .iter()
            .filter_map(|d| {
                let id = d.get("id")?.as_str()?;
                let name = d.get("name")?.as_str()?;
                let mount = PathBuf::from(d.get("mount")?.as_str()?);
                if id.is_empty() || !mounts.contains(&mount) {
                    return None;
                }
                Some(DeviceInfo {
                    id: format!("{}:{id}", self.id),
                    name: name.chars().take(80).collect(),
                    mount,
                    parent: None,
                })
            })
            .collect()
    }

    fn preferred_formats(&self) -> &'static [&'static str] {
        self.formats
    }

    fn send(&self, device: &DeviceInfo, file_name: &str, bytes: &[u8]) -> Result<PathBuf> {
        self.own(device)?;
        if !(self.mounts)().contains(&device.mount) {
            bail!("{} is no longer connected", device.name);
        }
        let v = self.plugin.call_json(
            "send",
            &json!({
                "device": Self::device_json(device),
                "file_name": file_name,
                "data_b64": B64.encode(bytes),
            }),
        )?;
        let rel = v
            .get("path")
            .and_then(|p| p.as_str())
            .ok_or_else(|| anyhow!("plugin `{}`: reply has no path", self.id))?;
        let rel = safe_relative(rel)?;
        let data = match v.get("data_b64") {
            None | Some(Value::Null) => bytes.to_vec(),
            Some(d) => decode_b64("data_b64", d)?,
        };
        let dest = device.mount.join(rel);
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&dest, data).with_context(|| format!("writing {}", dest.display()))?;
        Ok(dest)
    }

    fn list(&self, device: &DeviceInfo) -> Result<Vec<PathBuf>> {
        self.own(device)?;
        let mut files = Vec::new();
        walk_files(&device.mount, &device.mount, &mut files, 0);
        let v = self.plugin.call_json(
            "list",
            &json!({ "device": Self::device_json(device), "files": files }),
        )?;
        let arr = v
            .as_array()
            .ok_or_else(|| anyhow!("plugin `{}`: list reply is not an array", self.id))?;
        arr.iter()
            .map(|p| {
                let rel = safe_relative(p.as_str().ok_or_else(|| anyhow!("a path is not text"))?)?;
                Ok(device.mount.join(rel))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_relative_paths() {
        assert_eq!(
            safe_relative("a/b.epub").unwrap(),
            PathBuf::from("a/b.epub")
        );
        assert_eq!(safe_relative("./a.epub").unwrap(), PathBuf::from("a.epub"));
        for bad in ["", "/etc/passwd", "../x", "a/../../x", "a/.."] {
            assert!(safe_relative(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn listing_helpers_are_bounded_and_relative() {
        let d = std::env::temp_dir().join(format!("bookx-adapt-{}", std::process::id()));
        std::fs::create_dir_all(d.join("a/b")).unwrap();
        std::fs::write(d.join("top.txt"), "x").unwrap();
        std::fs::write(d.join("a/b/deep.epub"), "x").unwrap();
        assert_eq!(list_top(&d), ["a", "top.txt"]);
        let mut files = Vec::new();
        walk_files(&d, &d, &mut files, 0);
        assert_eq!(files, ["a/b/deep.epub", "top.txt"]);
        assert!(list_top(&d.join("missing")).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn intern_returns_the_same_pointer() {
        let a = intern("unit.test.id");
        let b = intern(&String::from("unit.test.id"));
        assert!(std::ptr::eq(a, b));
        assert_eq!(a, "unit.test.id");
        assert_ne!(intern("other.id"), a);
    }

    #[test]
    fn intern_slice_dedups_and_keeps_order() {
        let v = vec!["u1".to_string(), "u2".to_string()];
        let a = intern_slice(&v);
        let b = intern_slice(&v);
        assert!(std::ptr::eq(a, b));
        assert_eq!(a, &["u1", "u2"]);
        assert!(intern_slice(&[]).is_empty());
    }

    #[test]
    fn base64_decoding_errors_name_the_field() {
        assert_eq!(decode_b64("f", &json!("aGk=")).unwrap(), b"hi");
        assert_eq!(decode_b64("f", &json!(" aGk=\n")).unwrap(), b"hi");
        assert!(
            decode_b64("cover", &json!(1))
                .unwrap_err()
                .to_string()
                .contains("cover")
        );
        assert!(decode_b64("f", &json!("a")).is_err());
    }

    #[test]
    fn metadata_dto_defaults() {
        let m = decode_metadata_list(json!([{}])).unwrap();
        assert_eq!(m, vec![Metadata::default()]);
    }
}
