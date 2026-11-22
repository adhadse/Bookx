//! Plugins (roadmap X3/X8/X9, decision D5): sandboxed WebAssembly extensions from `bookx-plugin`
//! and user patches. Enabled plugins add metadata sources (used by *Fetch Metadata*), file format
//! readers (used when adding books), output formats (*Convert To*), transforms (conversion, the
//! library's tool menu and the editor's Tools menu) and device drivers (*Send to*). Patches are
//! small TOML rule files applied at import, convert and open. No GTK in here, so it is unit tested.

use crate::i18n::tr;
use crate::i18n::trf;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, bail, Context, Result};
use bookx_core::contracts::{
    DeviceDriver, FormatReader, FormatWriter, MetadataQuery, MetadataSource, Registry, Transform,
};
use bookx_plugin::{Hook, Host, HostNetwork, PatchManager, PatchReport, PatchSet, PluginManager};
use bookx_sources::fetch::Fetcher;

use crate::core::archive::Archive;
use crate::core::book::OpenBook;
use crate::core::opf::Metadata;
use crate::export::Target;

/// What the enabled plugins provide right now.
#[derive(Default)]
pub struct Loaded {
    pub sources: Registry<dyn MetadataSource>,
    pub readers: Registry<dyn FormatReader>,
    pub writers: Registry<dyn FormatWriter>,
    pub transforms: Registry<dyn Transform>,
    pub devices: Registry<dyn DeviceDriver>,
    /// Actions of the enabled `editor-action` plugins (Tools → Plugins, command palette).
    pub editor_actions: Vec<EditorEntry>,
    /// `plugin id -> display name` of every installed plugin.
    pub names: HashMap<String, String>,
}

/// One action of an editor plugin.
#[derive(Clone)]
pub struct EditorEntry {
    pub plugin: Arc<bookx_plugin::Plugin>,
    pub action: bookx_plugin::Action,
}

static CURRENT: Mutex<Option<Arc<Loaded>>> = Mutex::new(None);

/// The registries of the enabled plugins (empty before the first [`publish`]).
pub fn current() -> Arc<Loaded> {
    CURRENT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_default()
}

/// Makes `loaded` what the importer and *Fetch Metadata* use.
pub fn publish(loaded: Loaded) {
    *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(loaded));
}

/// The plugin and patch folders safe mode points at (empty ones) for the rest of the process; the environment is not
/// changed for this (writing it is unsafe in Rust 2024 and other threads may be reading it).
static SAFE_DIRS: Mutex<Option<(PathBuf, PathBuf)>> = Mutex::new(None);

/// Makes [`plugins_dir`] and [`patches_dir`] answer with these folders from now on (safe mode).
pub fn override_dirs(plugins: PathBuf, patches: PathBuf) {
    *SAFE_DIRS.lock().unwrap_or_else(|e| e.into_inner()) = Some((plugins, patches));
}

/// The safe-mode folder, else `$BOOKX_PLUGINS`, else `~/.local/share/bookx/plugins`.
pub fn plugins_dir() -> PathBuf {
    if let Some((p, _)) = SAFE_DIRS.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        return p;
    }
    if let Some(d) = std::env::var_os("BOOKX_PLUGINS") {
        return PathBuf::from(d);
    }
    directories::ProjectDirs::from("com", "adhadse", "Bookx")
        .map(|d| d.data_dir().join("plugins"))
        .unwrap_or_else(|| PathBuf::from("plugins"))
}

/// Which plugins are switched on: next to the configuration file.
pub fn state_path() -> PathBuf {
    crate::config::stats_path()
        .and_then(|p| p.parent().map(|d| d.join("plugins.json")))
        .unwrap_or_else(|| PathBuf::from("plugins.json"))
}

/// Network access for plugins that were granted `network:<host>`; the plugin host checks the host
/// name, this only fetches.
struct PluginNetwork(Arc<dyn Fetcher>);

impl HostNetwork for PluginNetwork {
    fn get(&self, url: &str, max_bytes: usize) -> Result<Vec<u8>> {
        let body = self.0.get(url, &[])?;
        if body.len() > max_bytes {
            anyhow::bail!("the answer is larger than {max_bytes} bytes");
        }
        Ok(body)
    }
}

/// A manager for the plugin folder, with the folder scanned.
pub fn new_manager(fetcher: Arc<dyn Fetcher>) -> PluginManager {
    let host = Host::default().with_network(Arc::new(PluginNetwork(fetcher)));
    let mut m = PluginManager::new(plugins_dir(), state_path(), host);
    m.scan();
    m
}

/// Registries of the enabled plugins. `(id, error)` for the ones that failed to load.
pub fn build(mgr: &mut PluginManager) -> (Loaded, Vec<(String, String)>) {
    let mut loaded = Loaded::default();
    let mut errors = mgr.register_metadata_sources(&mut loaded.sources);
    errors.extend(mgr.register_readers(&mut loaded.readers));
    errors.extend(mgr.register_writers(&mut loaded.writers));
    errors.extend(mgr.register_transforms(&mut loaded.transforms));
    errors.extend(mgr.register_devices(
        &mut loaded.devices,
        Arc::new(crate::devices::candidate_mounts),
    ));
    let (editor, editor_errors) = mgr.editor_plugins();
    errors.extend(editor_errors);
    for plugin in editor {
        for action in plugin.manifest().actions.clone() {
            loaded.editor_actions.push(EditorEntry {
                plugin: plugin.clone(),
                action,
            });
        }
    }
    loaded.names = mgr.list().into_iter().map(|p| (p.id, p.name)).collect();
    (loaded, errors)
}

/// Rebuilds the registries from the manager and publishes them.
pub fn reload(mgr: &mut PluginManager) -> Vec<(String, String)> {
    let (loaded, errors) = build(mgr);
    publish(loaded);
    errors
}

/// File extensions the plugin readers add (lower case, without the dot).
pub fn reader_extensions(loaded: &Loaded) -> Vec<String> {
    loaded
        .readers
        .iter()
        .flat_map(|r| r.extensions().iter().map(|e| e.to_ascii_lowercase()))
        .collect()
}

/// What every plugin source knows about the book: `(plugin id, metadata)`, best first. A plugin
/// that fails is skipped.
pub fn lookup_all(loaded: &Loaded, q: &MetadataQuery) -> Vec<(String, Metadata)> {
    let mut out = Vec::new();
    for s in loaded.sources.iter() {
        if let Ok(list) = s.lookup(q) {
            if let Some(m) = list.into_iter().next() {
                out.push((s.id().to_string(), m));
            }
        }
    }
    out
}

/// Reads a file with the plugin reader that claims it (by extension, else by content).
pub fn read(loaded: &Loaded, file_name: &str, bytes: &[u8]) -> Result<Archive> {
    let head = &bytes[..bytes.len().min(bookx_plugin::SNIFF_BYTES)];
    let r = bookx_core::contracts::pick_reader(&loaded.readers, file_name, head)?;
    r.read(bytes)
}

// ---- output formats and tools from plugins -------------------------------------------------

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// Every pseudo target is created once and lives for the whole run (`Target` wants `'static`).
fn interned_target(id: &str, make: impl FnOnce() -> Target) -> &'static Target {
    static T: Mutex<Option<HashMap<String, &'static Target>>> = Mutex::new(None);
    let mut g = T.lock().unwrap_or_else(|e| e.into_inner());
    let map = g.get_or_insert_with(HashMap::new);
    if let Some(t) = map.get(id) {
        return t;
    }
    let t: &'static Target = Box::leak(Box::new(make()));
    map.insert(id.to_string(), t);
    t
}

const WRITER_PREFIX: &str = "plugin:";
const TOOL_PREFIX: &str = "plugin-tool:";

fn writer_target(loaded: &Loaded, w: &dyn FormatWriter) -> &'static Target {
    let id = format!("{WRITER_PREFIX}{}", w.id());
    let name = loaded
        .names
        .get(w.id())
        .cloned()
        .unwrap_or_else(|| w.id().to_string());
    let ext = w.extension();
    interned_target(&id.clone(), || Target {
        label: leak(format!("{name} (.{ext}, plugin)")),
        id: leak(id),
        extension: leak(ext.to_string()),
        preset: None,
    })
}

fn tool_target(loaded: &Loaded, t: &dyn Transform) -> &'static Target {
    let id = format!("{TOOL_PREFIX}{}", t.id());
    let name = loaded
        .names
        .get(t.id())
        .cloned()
        .unwrap_or_else(|| t.id().to_string());
    interned_target(&id.clone(), || Target {
        label: leak(trf("Apply {} (new EPUB)", &[&name])),
        id: leak(id),
        extension: "epub",
        preset: None,
    })
}

/// Output formats of the enabled writer plugins, for the *Convert To* menu.
pub fn writer_targets() -> Vec<&'static Target> {
    let loaded = current();
    let targets = loaded
        .writers
        .iter()
        .map(|w| writer_target(&loaded, w))
        .collect();
    targets
}

/// One entry per enabled transform plugin: converting with it saves a transformed copy as EPUB.
pub fn tool_targets() -> Vec<&'static Target> {
    let loaded = current();
    let targets = loaded
        .transforms
        .iter()
        .map(|t| tool_target(&loaded, t))
        .collect();
    targets
}

/// The plugin target with this id (`plugin:<id>` or `plugin-tool:<id>`), if that plugin is enabled.
pub fn target(id: &str) -> Option<&'static Target> {
    let loaded = current();
    if let Some(wid) = id.strip_prefix(WRITER_PREFIX) {
        return loaded
            .writers
            .iter()
            .find(|w| w.id() == wid)
            .map(|w| writer_target(&loaded, w));
    }
    let tid = id.strip_prefix(TOOL_PREFIX)?;
    let found = loaded
        .transforms
        .iter()
        .find(|t| t.id() == tid)
        .map(|t| tool_target(&loaded, t));
    found
}

/// Runs the transform plugin `id` on `book`: `(files changed, notes)`.
pub fn run_transform(
    loaded: &Loaded,
    id: &str,
    book: &mut OpenBook,
) -> Result<(usize, Vec<String>)> {
    let t = loaded
        .transforms
        .iter()
        .find(|t| t.id() == id)
        .ok_or_else(|| anyhow!("the plugin `{id}` is not enabled"))?;
    let r = t.apply(book)?;
    Ok((r.files_changed, r.notes))
}

/// Transform plugins that also run in every conversion (the user's choice, off by default).
pub fn auto_transforms_path() -> PathBuf {
    state_path().with_file_name("plugin-transforms.json")
}

pub fn load_auto_transforms(path: &Path) -> BTreeSet<String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn set_auto_transform(path: &Path, id: &str, on: bool) -> Result<()> {
    let mut set = load_auto_transforms(path);
    if on {
        set.insert(id.to_string());
    } else {
        set.remove(id);
    }
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(path, serde_json::to_string(&set)?)?;
    Ok(())
}

// ---- user patches (X9) -------------------------------------------------------------------

/// The safe-mode folder, else `$BOOKX_PATCHES`, else `~/.local/share/bookx/patches`.
pub fn patches_dir() -> PathBuf {
    if let Some((_, p)) = SAFE_DIRS.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        return p;
    }
    if let Some(d) = std::env::var_os("BOOKX_PATCHES") {
        return PathBuf::from(d);
    }
    directories::ProjectDirs::from("com", "adhadse", "Bookx")
        .map(|d| d.data_dir().join("patches"))
        .unwrap_or_else(|| PathBuf::from("patches"))
}

pub fn patches_state_path() -> PathBuf {
    state_path().with_file_name("patches.json")
}

/// The patch folder as the manager UI works on it.
pub fn patch_manager() -> PatchManager {
    PatchManager::new(patches_dir(), patches_state_path())
}

static PATCHES: Mutex<Option<Arc<PatchSet>>> = Mutex::new(None);

/// The enabled patches (read from disk on first use, then until [`reload_patches`]).
pub fn current_patches() -> Arc<PatchSet> {
    if cfg!(test) {
        // unit tests must not depend on the patches of whoever runs them
        return Arc::default();
    }
    let mut g = PATCHES.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(|| Arc::new(patch_manager().enabled_set()))
        .clone()
}

/// Re-reads the patch folder and makes it the active set.
pub fn reload_patches() -> Arc<PatchSet> {
    let set = Arc::new(patch_manager().enabled_set());
    *PATCHES.lock().unwrap_or_else(|e| e.into_inner()) = Some(set.clone());
    set
}

/// A small book for dry runs when the library is empty.
pub fn sample_book() -> Archive {
    use crate::core::convert::{create_epub, Chapter};
    create_epub(
        &Metadata {
            title: "Sample".into(),
            authors: vec!["Bookx".into()],
            language: "en".into(),
            ..Default::default()
        },
        &[Chapter {
            title: "One".into(),
            body: "<p>\u{201c}Hello,\u{201d} she said -- and left.</p><p>Second   paragraph...</p>"
                .into(),
        }],
        None,
    )
}

/// Dry run of patch `id` on the EPUB at `epub` (the sample book when `None`).
pub fn preview_patch(mgr: &PatchManager, id: &str, epub: Option<&Path>) -> Result<PatchReport> {
    let archive = match epub {
        Some(p) => Archive::open(p)?,
        None => sample_book(),
    };
    mgr.preview(id, &archive)
}

/// Applies the `open-before` patches to a book the reader has just loaded (the stored file stays
/// as it is).
pub fn patch_on_open(archive: &mut Archive) {
    current_patches().apply(Hook::OpenBefore, archive);
}

/// Applies the `import-after` patches to a freshly prepared import; metadata and cover are read
/// again when something changed. Used by the tests; the importer calls [`patch_prepared_noted`].
#[cfg(test)]
pub fn patch_prepared_with(
    set: &PatchSet,
    prepared: crate::core::library::Prepared,
    origin: &Path,
) -> Result<crate::core::library::Prepared> {
    patch_prepared_noted(set, prepared, origin).map(|(p, _)| p)
}

/// [`patch_prepared_with`] that also says which patches changed the book (their ids).
pub fn patch_prepared_noted(
    set: &PatchSet,
    prepared: crate::core::library::Prepared,
    origin: &Path,
) -> Result<(crate::core::library::Prepared, Vec<String>)> {
    if !set.has_hook(Hook::ImportAfter) {
        return Ok((prepared, Vec::new()));
    }
    let mut archive = Archive::from_bytes(prepared.bytes.clone())?;
    let applied = set.apply(Hook::ImportAfter, &mut archive);
    if applied.is_empty() {
        return Ok((prepared, Vec::new()));
    }
    let names = applied.into_iter().map(|(id, _)| id).collect();
    Ok((
        crate::core::library::prepare_from_archive(archive, origin)?,
        names,
    ))
}

// ---- conversion with hooks and plugin writers/transforms ---------------------------------

/// Converts the EPUB `bytes` when plugins or patches take part, `None` when the plain converter
/// does the job.
pub fn convert_hooked(bytes: &[u8], name: &str, t: &Target) -> Option<Result<Vec<u8>>> {
    if !convert_is_hooked(t) {
        return None;
    }
    let loaded = current();
    let patches = current_patches();
    let auto = load_auto_transforms(&auto_transforms_path());
    Some(convert_with(&loaded, &patches, &auto, bytes, name, t))
}

/// Whether a plugin or patch takes part when converting to `t` (so even an EPUB is rewritten).
pub fn convert_is_hooked(t: &Target) -> bool {
    let loaded = current();
    let patches = current_patches();
    let auto = load_auto_transforms(&auto_transforms_path());
    let special = t.id.starts_with(WRITER_PREFIX) || t.id.starts_with(TOOL_PREFIX);
    let has_auto = loaded.transforms.iter().any(|x| auto.contains(x.id()));
    special
        || has_auto
        || patches.has_hook(Hook::ConvertBefore)
        || patches.has_hook(Hook::ConvertAfter)
}

pub fn convert_with(
    loaded: &Loaded,
    patches: &PatchSet,
    auto: &BTreeSet<String>,
    bytes: &[u8],
    name: &str,
    t: &Target,
) -> Result<Vec<u8>> {
    let options = crate::export::options_for(t)?;
    let mut book = bookx_convert::read_input(&bookx_formats::default_readers(), bytes, name)?;
    patches.apply(Hook::ConvertBefore, &mut book.archive);
    let tool = t.id.strip_prefix(TOOL_PREFIX);
    if tool.is_none() {
        for tr in options.transforms().iter() {
            tr.apply(&mut book)
                .with_context(|| format!("transform `{}` failed", tr.id()))?;
        }
    }
    for tr in loaded.transforms.iter() {
        if tool == Some(tr.id()) || (tool.is_none() && auto.contains(tr.id())) {
            tr.apply(&mut book)
                .with_context(|| format!("plugin transform `{}` failed", tr.id()))?;
        }
    }
    patches.apply(Hook::ConvertAfter, &mut book.archive);
    if let Some(wid) = t.id.strip_prefix(WRITER_PREFIX) {
        let w = loaded
            .writers
            .iter()
            .find(|w| w.id() == wid)
            .ok_or_else(|| anyhow!("the plugin `{wid}` is not enabled"))?;
        return w
            .write(&book)
            .with_context(|| format!("writing {} with plugin `{wid}`", t.extension));
    }
    let epub = book.archive.to_bytes()?;
    if tool.is_some() {
        return Ok(epub);
    }
    // the transforms ran above; the writer only needs its own settings
    let write_opts = bookx_convert::ConvertOptions {
        epub_version: options.epub_version,
        mobi_inline_toc: options.mobi_inline_toc,
        docx: options.docx.clone(),
        pdb_compression: options.pdb_compression,
        ..Default::default()
    };
    let cancel = AtomicBool::new(false);
    let out = bookx_convert::convert(
        &bookx_formats::default_readers(),
        &epub,
        "book.epub",
        t.id,
        &write_opts,
        &|_| {},
        &cancel,
    )?;
    if out.bytes.is_empty() {
        bail!("the converter returned nothing");
    }
    Ok(out.bytes)
}

/// What a plugin may do, in words for the Preferences list. `permissions` are the manifest's
/// strings (`network:<host>`).
pub fn permission_summary(permissions: &[String]) -> String {
    let has = |p: &str| permissions.iter().any(|x| x == p);
    if has("editor.write") {
        return tr("Permissions: may read and change the book open in the editor. Every action is one checkpoint you can restore. It cannot reach other files or the network.");
    }
    if has("editor.read") {
        return tr("Permissions: may read the book open in the editor. It cannot change anything, reach other files or the network.");
    }
    let hosts: Vec<&str> = permissions
        .iter()
        .filter_map(|p| p.strip_prefix("network:"))
        .collect();
    if hosts.is_empty() {
        tr("Permissions: none. It cannot read your files or reach the network.")
    } else {
        trf(
            "Permissions: may connect to {}. It cannot read your files.",
            &[&hosts.join(", ")],
        )
    }
}

/// What a plugin kind adds to Bookx, in words.
pub fn kind_summary(kind: bookx_plugin::PluginKind) -> &'static str {
    use bookx_plugin::PluginKind::*;
    match kind {
        MetadataSource => "Metadata source (Fetch Metadata)",
        FormatReader => "File reader (adding books)",
        Transform => "Transform (Convert To, library and editor tools)",
        FormatWriter => "Output format (Convert To)",
        Device => "Device (Send to)",
        EditorAction => "Editor action (Tools → Plugins)",
    }
}

/// Helpers for tests (here and in `devices`): mock plugins assembled from WebAssembly text.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::fmt::Write as _;

    pub fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "bookx-plug-{tag}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A plugin that answers by the first byte of the operation name with a fixed reply.
    pub fn canned(replies: &[(u8, String)]) -> Vec<u8> {
        let (mut data, mut checks, mut off) = (String::new(), String::new(), 1024usize);
        for (c, reply) in replies {
            let esc: String = reply.bytes().map(|b| format!("\\{b:02x}")).collect();
            let _ = writeln!(data, "(data (i32.const {off}) \"{esc}\")");
            let packed = ((off as i64) << 32) | reply.len() as i64;
            let _ = writeln!(
                checks,
                "(if (i32.eq (local.get $c) (i32.const {c})) (then (return (i64.const {packed}))))"
            );
            off += reply.len() + 16;
        }
        let heap = (off + 4095) & !4095;
        wat::parse_str(format!(
            r#"(module (memory (export "memory") 2) (global $h (mut i32) (i32.const {heap}))
              (func (export "alloc") (param $n i32) (result i32) (local $p i32)
                (local.set $p (global.get $h))
                (global.set $h (i32.add (local.get $p) (i32.and (i32.add (local.get $n) (i32.const 7)) (i32.const -8))))
                (local.get $p))
              (func (export "dealloc") (param i32 i32))
              {data}
              (func (export "bookx_call") (param $op i32) (param $ol i32) (param $ip i32) (param $il i32) (result i64)
                (local $c i32) (local.set $c (i32.load8_u (local.get $op)))
                {checks}
                (i64.const -2)))"#
        ))
        .unwrap()
    }

    pub fn install(dir: &Path, folder: &str, id: &str, kind: &str, extra: &str, wasm: &[u8]) {
        let d = dir.join(folder);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("plugin.toml"),
            format!("id = \"{id}\"\nname = \"Name of {id}\"\nversion = \"1.0.0\"\napi_version = 1\nkind = \"{kind}\"\n{extra}\n"),
        )
        .unwrap();
        std::fs::write(d.join("plugin.wasm"), wasm).unwrap();
    }

    pub fn manager(dir: &Path) -> PluginManager {
        let mut m = PluginManager::new(dir, dir.join("state.json"), Host::default());
        m.scan();
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_plugin::PluginKind;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "bookx-plug-{tag}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn esc(b: &[u8]) -> String {
        b.iter().map(|x| format!("\\{x:02x}")).collect()
    }

    /// A metadata-source plugin that answers `lookup` with `reply` (`{"ok": [...]}`).
    fn source_wasm(reply: &str) -> Vec<u8> {
        let packed = ((1024i64) << 32) | reply.len() as i64;
        wat::parse_str(format!(
            r#"(module (memory (export "memory") 2) (global $h (mut i32) (i32.const 4096))
              (func (export "alloc") (param $n i32) (result i32) (local $p i32)
                (local.set $p (global.get $h))
                (global.set $h (i32.add (local.get $p) (i32.and (i32.add (local.get $n) (i32.const 7)) (i32.const -8))))
                (local.get $p))
              (func (export "dealloc") (param i32 i32))
              (data (i32.const 1024) "{}")
              (func (export "bookx_call") (param $op i32) (param $ol i32) (param $ip i32) (param $il i32) (result i64)
                (if (i32.eq (i32.load8_u (local.get $op)) (i32.const 108)) (then (return (i64.const {packed}))))
                (i64.const -2)))"#,
            esc(reply.as_bytes())
        ))
        .unwrap()
    }

    fn install(dir: &std::path::Path, folder: &str, id: &str, kind: &str, wasm: &[u8]) {
        let d = dir.join(folder);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("plugin.toml"),
            format!("id = \"{id}\"\nname = \"{id}\"\nversion = \"1.0.0\"\napi_version = 1\nkind = \"{kind}\"\n"),
        )
        .unwrap();
        std::fs::write(d.join("plugin.wasm"), wasm).unwrap();
    }

    fn manager(dir: &std::path::Path) -> PluginManager {
        let mut m = PluginManager::new(dir, dir.join("state.json"), Host::default());
        m.scan();
        m
    }

    #[test]
    fn plugins_start_disabled_and_provide_nothing_until_enabled() {
        let d = tmp("off");
        install(
            &d,
            "a",
            "pub.source",
            "metadata-source",
            &source_wasm(r#"{"ok":[{"publisher":"P"}]}"#),
        );
        let mut m = manager(&d);
        let (loaded, errs) = build(&mut m);
        assert!(errs.is_empty());
        assert_eq!(loaded.sources.len(), 0);
        assert!(lookup_all(&loaded, &MetadataQuery::default()).is_empty());
        m.set_enabled("pub.source", true).unwrap();
        let (loaded, errs) = build(&mut m);
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(loaded.sources.len(), 1);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn an_enabled_source_answers_lookups() {
        let d = tmp("on");
        install(
            &d,
            "a",
            "pub.source",
            "metadata-source",
            &source_wasm(r#"{"ok":[{"title":"T","publisher":"Plugin Press","tags":["x"]}]}"#),
        );
        let mut m = manager(&d);
        m.set_enabled("pub.source", true).unwrap();
        let (loaded, _) = build(&mut m);
        let q = MetadataQuery {
            title: "anything".into(),
            ..Default::default()
        };
        let hits = lookup_all(&loaded, &q);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, "pub.source");
        assert_eq!(hits[0].1.publisher, "Plugin Press");
        assert_eq!(hits[0].1.tags, vec!["x"]);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_failing_or_empty_plugin_is_skipped_and_the_others_still_answer() {
        let d = tmp("mixed");
        install(
            &d,
            "bad",
            "bad.source",
            "metadata-source",
            &source_wasm(r#"{"err":"boom"}"#),
        );
        install(
            &d,
            "empty",
            "empty.source",
            "metadata-source",
            &source_wasm(r#"{"ok":[]}"#),
        );
        install(
            &d,
            "good",
            "good.source",
            "metadata-source",
            &source_wasm(r#"{"ok":[{"publisher":"Good"}]}"#),
        );
        let mut m = manager(&d);
        for id in ["bad.source", "empty.source", "good.source"] {
            m.set_enabled(id, true).unwrap();
        }
        let (loaded, _) = build(&mut m);
        let hits = lookup_all(&loaded, &MetadataQuery::default());
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].1.publisher, "Good");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_broken_plugin_folder_is_listed_with_its_error() {
        let d = tmp("broken");
        std::fs::create_dir_all(d.join("junk")).unwrap();
        std::fs::write(d.join("junk/plugin.toml"), "this is not toml = = =").unwrap();
        install(
            &d,
            "ok",
            "ok.source",
            "metadata-source",
            &source_wasm(r#"{"ok":[]}"#),
        );
        let m = manager(&d);
        let list = m.list();
        assert_eq!(list.len(), 2);
        assert!(list.iter().any(|p| p.error.is_some()), "{list:?}");
        assert!(list
            .iter()
            .any(|p| p.id == "ok.source" && p.error.is_none()));
        assert!(list
            .iter()
            .any(|p| p.kind == Some(PluginKind::MetadataSource)));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_enabled_state_survives_a_new_manager() {
        let d = tmp("state");
        install(
            &d,
            "a",
            "pub.source",
            "metadata-source",
            &source_wasm(r#"{"ok":[]}"#),
        );
        {
            let mut m = manager(&d);
            m.set_enabled("pub.source", true).unwrap();
        }
        let m = manager(&d);
        assert!(m.is_enabled("pub.source"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn no_plugin_readers_means_no_extra_extensions() {
        let loaded = Loaded::default();
        assert!(reader_extensions(&loaded).is_empty());
        assert!(read(&loaded, "x.abc", b"data").is_err());
    }

    #[test]
    fn publishing_replaces_what_the_app_uses() {
        // the only test that touches the global registry
        assert_eq!(current().sources.len(), 0);
        let d = tmp("publish");
        install(
            &d,
            "a",
            "pub.source",
            "metadata-source",
            &source_wasm(r#"{"ok":[]}"#),
        );
        let mut m = manager(&d);
        m.set_enabled("pub.source", true).unwrap();
        let errs = reload(&mut m);
        assert!(errs.is_empty());
        assert_eq!(current().sources.len(), 1);
        m.set_enabled("pub.source", false).unwrap();
        reload(&mut m);
        assert_eq!(current().sources.len(), 0);
        let _ = std::fs::remove_dir_all(d);
    }

    use super::test_support as ts;

    fn epub_bytes() -> Vec<u8> {
        sample_book().to_bytes().unwrap()
    }

    fn patch_set(dir: &Path, files: &[(&str, &str)]) -> PatchSet {
        for (name, text) in files {
            std::fs::write(dir.join(name), text).unwrap();
        }
        let mut m = PatchManager::new(dir, dir.join("state/patches.json"));
        for (name, _) in files {
            m.set_enabled(name.trim_end_matches(".toml"), true).unwrap();
        }
        m.enabled_set()
    }

    fn chapter_text(epub: &[u8]) -> String {
        let a = Archive::from_bytes(epub.to_vec()).unwrap();
        let n = a
            .names()
            .iter()
            .find(|n| n.ends_with(".xhtml") && !n.contains("nav"))
            .unwrap()
            .clone();
        a.get_string(&n).unwrap()
    }

    #[test]
    fn permissions_are_described_in_words() {
        assert!(permission_summary(&[]).contains("none"));
        let s = permission_summary(&["network:openlibrary.org".into(), "network:a.b".into()]);
        assert!(s.contains("openlibrary.org, a.b") && s.contains("cannot read your files"));
        assert!(kind_summary(bookx_plugin::PluginKind::FormatWriter).contains("Convert To"));
        let r = permission_summary(&["editor.read".into()]);
        assert!(r.contains("read the book") && r.contains("cannot change anything"));
        let w = permission_summary(&["editor.read".into(), "editor.write".into()]);
        assert!(w.contains("change the book") && w.contains("checkpoint"));
        assert!(kind_summary(bookx_plugin::PluginKind::EditorAction).contains("Tools"));
    }

    #[test]
    fn editor_plugins_are_disabled_until_switched_on_and_then_offer_their_actions() {
        let d = ts::tmp("editor-actions");
        let example = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples/plugins/trim-whitespace");
        let dest = d.join("trim");
        std::fs::create_dir_all(&dest).unwrap();
        for f in ["plugin.toml", "plugin.wasm"] {
            std::fs::copy(example.join(f), dest.join(f)).unwrap();
        }
        let mut m = ts::manager(&d);
        let (loaded, errors) = build(&mut m);
        assert!(
            errors.is_empty() && loaded.editor_actions.is_empty(),
            "off by default"
        );
        let st = m.list().into_iter().next().unwrap();
        assert_eq!(st.kind, Some(bookx_plugin::PluginKind::EditorAction));
        assert_eq!(st.permissions, vec!["editor.read", "editor.write"]);
        m.set_enabled(&st.id, true).unwrap();
        let (loaded, errors) = build(&mut m);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(loaded.editor_actions.len(), 1);
        assert_eq!(loaded.editor_actions[0].action.id, "trim");
        // run it on a book through the same adapter the editor uses
        let mut a = crate::core::archive::Archive::default();
        a.set_string("OEBPS/a.xhtml", "<p>x</p>   \n");
        let copy = crate::ui::editor::plugin_host::for_tests::new_copy(a);
        let session = bookx_plugin::EditorSession::new(
            Box::new(crate::ui::editor::plugin_host::for_tests::adapter(
                copy.clone(),
            )),
            &loaded.editor_actions[0].plugin.manifest().permissions,
            "Trim",
            bookx_plugin::EditorLimits::default(),
        );
        let r = loaded.editor_actions[0]
            .plugin
            .run_editor_action("trim", &session)
            .unwrap();
        assert_eq!(r.changes, 1);
        let c = copy.lock().unwrap();
        assert!(c.changed);
        assert_eq!(c.archive.get_string("OEBPS/a.xhtml").unwrap(), "<p>x</p>\n");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_conversion_option_of_a_transform_is_remembered() {
        let d = ts::tmp("auto");
        let f = d.join("sub/plugin-transforms.json");
        assert!(load_auto_transforms(&f).is_empty());
        set_auto_transform(&f, "a.b", true).unwrap();
        set_auto_transform(&f, "c.d", true).unwrap();
        set_auto_transform(&f, "a.b", false).unwrap();
        assert_eq!(
            load_auto_transforms(&f).into_iter().collect::<Vec<_>>(),
            ["c.d"]
        );
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn enabled_writers_become_convert_targets_and_write_files() {
        let d = ts::tmp("writer");
        let out = "QUJDLU9VVA==";
        ts::install(
            &d,
            "w",
            "test.writer",
            "format-writer",
            "extension = \"abc\"",
            &ts::canned(&[(b'w', format!("{{\"ok\":\"{out}\"}}"))]),
        );
        let mut m = ts::manager(&d);
        let (off, _) = build(&mut m);
        assert_eq!(off.writers.len(), 0, "disabled by default");
        m.set_enabled("test.writer", true).unwrap();
        let (loaded, errs) = build(&mut m);
        assert!(errs.is_empty(), "{errs:?}");
        let w = loaded.writers.iter().next().unwrap();
        let t = writer_target(&loaded, w);
        assert_eq!(t.id, "plugin:test.writer");
        assert_eq!(t.extension, "abc");
        assert!(t.label.contains("Name of test.writer") && t.label.contains("plugin"));
        assert!(
            std::ptr::eq(t, writer_target(&loaded, w)),
            "one target per plugin"
        );
        let bytes = convert_with(
            &loaded,
            &PatchSet::default(),
            &BTreeSet::new(),
            &epub_bytes(),
            "b.epub",
            t,
        )
        .unwrap();
        assert_eq!(bytes, b"ABC-OUT");
        assert_eq!(crate::export::file_name("Title", t), "Title.abc");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_missing_writer_plugin_is_an_error_not_a_panic() {
        let t = Target {
            label: "x",
            id: "plugin:gone",
            extension: "x",
            preset: None,
        };
        let e = convert_with(
            &Loaded::default(),
            &PatchSet::default(),
            &BTreeSet::new(),
            &epub_bytes(),
            "b.epub",
            &t,
        )
        .unwrap_err();
        assert!(e.to_string().contains("not enabled"));
    }

    #[test]
    fn transform_plugins_run_as_one_off_tools_and_report() {
        let d = ts::tmp("transform");
        ts::install(
            &d,
            "t",
            "test.transform",
            "transform",
            "",
            &ts::canned(&[(
                b'a',
                r#"{"ok":{"files_changed":3,"notes":["did it"]}}"#.into(),
            )]),
        );
        let mut m = ts::manager(&d);
        m.set_enabled("test.transform", true).unwrap();
        let (loaded, errs) = build(&mut m);
        assert!(errs.is_empty(), "{errs:?}");
        let mut book = OpenBook::from_archive(sample_book()).unwrap();
        let (files, notes) = run_transform(&loaded, "test.transform", &mut book).unwrap();
        assert_eq!((files, notes), (3, vec!["did it".to_string()]));
        assert!(run_transform(&loaded, "nope", &mut book).is_err());

        let t = tool_target(&loaded, loaded.transforms.iter().next().unwrap());
        assert_eq!((t.id, t.extension), ("plugin-tool:test.transform", "epub"));
        let out = convert_with(
            &loaded,
            &PatchSet::default(),
            &BTreeSet::new(),
            &epub_bytes(),
            "b.epub",
            t,
        )
        .unwrap();
        assert!(Archive::from_bytes(out)
            .unwrap()
            .contains("META-INF/container.xml"));

        // as a conversion option it also runs for ordinary targets
        let auto: BTreeSet<String> = ["test.transform".to_string()].into();
        let fb2 = crate::export::target("fb2").unwrap();
        let out = convert_with(
            &loaded,
            &PatchSet::default(),
            &auto,
            &epub_bytes(),
            "b.epub",
            fb2,
        )
        .unwrap();
        assert!(String::from_utf8_lossy(&out).contains("FictionBook"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_failing_transform_plugin_stops_the_conversion_with_its_name() {
        let d = ts::tmp("badtransform");
        ts::install(
            &d,
            "t",
            "bad.transform",
            "transform",
            "",
            &ts::canned(&[(b'a', r#"{"err":"boom"}"#.into())]),
        );
        let mut m = ts::manager(&d);
        m.set_enabled("bad.transform", true).unwrap();
        let (loaded, _) = build(&mut m);
        let auto: BTreeSet<String> = ["bad.transform".to_string()].into();
        let e = convert_with(
            &loaded,
            &PatchSet::default(),
            &auto,
            &epub_bytes(),
            "b.epub",
            crate::export::target("epub").unwrap(),
        )
        .unwrap_err();
        assert!(format!("{e:#}").contains("bad.transform") && format!("{e:#}").contains("boom"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn patches_run_before_and_after_in_their_order() {
        let d = ts::tmp("patch-convert");
        let set = patch_set(
            &d,
            &[
                (
                    "a-before.toml",
                    "hook = \"convert-before\"\n[[regex]]\nfind = 'Hello'\nreplace = 'Howdy'\n",
                ),
                (
                    "b-after.toml",
                    "hook = \"convert-after\"\n[[regex]]\nfind = 'Howdy'\nreplace = 'Greetings'\n",
                ),
                (
                    "c-open.toml",
                    "hook = \"open-before\"\n[[regex]]\nfind = 'Greetings'\nreplace = 'NEVER'\n",
                ),
            ],
        );
        let t = crate::export::target("epub").unwrap();
        let out = convert_with(
            &Loaded::default(),
            &set,
            &BTreeSet::new(),
            &epub_bytes(),
            "b.epub",
            t,
        )
        .unwrap();
        let text = chapter_text(&out);
        assert!(text.contains("Greetings"), "{text}");
        assert!(
            !text.contains("NEVER"),
            "open-before does not run on convert"
        );
        // other formats get the patched book too
        let fb2 = crate::export::target("fb2").unwrap();
        let out = convert_with(
            &Loaded::default(),
            &set,
            &BTreeSet::new(),
            &epub_bytes(),
            "b.epub",
            fb2,
        )
        .unwrap();
        assert!(String::from_utf8_lossy(&out).contains("Greetings"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn convert_hooked_steps_aside_when_nothing_takes_part() {
        let t = crate::export::target("epub").unwrap();
        assert!(convert_hooked(&epub_bytes(), "b.epub", t).is_none());
    }

    #[test]
    fn import_patches_change_the_stored_book_but_keep_its_metadata() {
        let d = ts::tmp("patch-import");
        let set = patch_set(
            &d,
            &[(
                "imp.toml",
                "hook = \"import-after\"\n[[regex]]\nfind = 'Second'\nreplace = '2nd'\n",
            )],
        );
        let prepared =
            crate::core::library::prepare_from_archive(sample_book(), Path::new("x.epub")).unwrap();
        let before = prepared.bytes.clone();
        let patched = patch_prepared_with(&set, prepared, Path::new("x.epub")).unwrap();
        assert_ne!(patched.bytes, before);
        assert!(chapter_text(&patched.bytes).contains("2nd "));
        assert_eq!(patched.meta.title, "Sample");
        // no import hook: untouched, byte for byte
        let none = patch_set(&ts::tmp("patch-import-none"), &[]);
        let prepared =
            crate::core::library::prepare_from_archive(sample_book(), Path::new("x.epub")).unwrap();
        let b = prepared.bytes.clone();
        assert_eq!(
            patch_prepared_with(&none, prepared, Path::new("x.epub"))
                .unwrap()
                .bytes,
            b
        );
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn import_patches_say_which_patches_changed_the_book() {
        let d = ts::tmp("patch-noted");
        let set = patch_set(
            &d,
            &[
                (
                    "a-imp.toml",
                    "hook = \"import-after\"\n[[regex]]\nfind = 'Second'\nreplace = '2nd'\n",
                ),
                (
                    "b-none.toml",
                    "hook = \"import-after\"\n[[regex]]\nfind = 'NOWHERE'\nreplace = 'x'\n",
                ),
            ],
        );
        let prepared =
            crate::core::library::prepare_from_archive(sample_book(), Path::new("x.epub")).unwrap();
        let (_, names) = patch_prepared_noted(&set, prepared, Path::new("x.epub")).unwrap();
        assert_eq!(
            names.len(),
            1,
            "only the patch that changed something: {names:?}"
        );
        assert!(names[0].contains("a-imp"));
        let none = patch_set(&ts::tmp("patch-noted-none"), &[]);
        let prepared =
            crate::core::library::prepare_from_archive(sample_book(), Path::new("x.epub")).unwrap();
        assert!(patch_prepared_noted(&none, prepared, Path::new("x.epub"))
            .unwrap()
            .1
            .is_empty());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn dry_run_uses_the_sample_book_and_changes_nothing_on_disk() {
        let d = ts::tmp("patch-dry");
        std::fs::write(
            d.join("q.toml"),
            "hook = \"convert-before\"\n[[regex]]\nfind = '[\u{201c}\u{201d}]'\nreplace = '\"'\n",
        )
        .unwrap();
        let m = PatchManager::new(&d, d.join("s.json"));
        let r = preview_patch(&m, "q", None).unwrap();
        assert_eq!(r.total_changes(), 2);
        assert!(preview_patch(&m, "missing", None).is_err());
        let epub = d.join("b.epub");
        let original = epub_bytes();
        std::fs::write(&epub, &original).unwrap();
        assert_eq!(
            preview_patch(&m, "q", Some(&epub)).unwrap().total_changes(),
            2
        );
        assert!(std::fs::read(&epub).unwrap() == original);
        let _ = std::fs::remove_dir_all(d);
    }
}
