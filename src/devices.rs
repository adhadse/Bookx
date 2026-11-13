//! Sending books to a connected e-reader and looking after what is on it (roadmap P1/P5/P7/P9/
//! P14/P15): the device is found by `bookx-serve`, the book is converted to the best format the
//! device opens (falling back to the next one when a conversion fails) and copied into the folder a
//! save-path template picks. The Device window lists, removes and syncs reading state through the
//! functions here. No GTK in here, so it is unit tested.
//!
//! Devices come from three places: USB mass storage (Kobo, Kindle, PocketBook, Boox…), MTP
//! devices mounted by gvfs, and an optional WebDAV folder. The Kobo side has been checked read-only
//! against a real Libra Colour (see `docs/kobo-real-device-report.md`); tests use fake device trees (`BOOKX_DEVICE_ROOTS`, `BOOKX_MTP_ROOT`) and a fake WebDAV server.

use crate::i18n::tr;
use crate::i18n::trf;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use bookx_core::contracts::{DeviceDriver, DeviceInfo};
use bookx_serve::devfs::{self, BookFields, DeviceBook, Space, Storage};
use bookx_serve::devices::{choose_format, DeviceKind, MassStorageDriver};
use bookx_serve::mtp::MtpGvfsDriver;
use bookx_serve::reading;
#[cfg(test)]
use bookx_serve::reading::DeviceState;
use bookx_serve::webdav::{WebDav, WebDavDriver};
use serde::{Deserialize, Serialize};

use crate::export::{self, Target};
use crate::send_notes::{self, Notes};

/// Settings of the Device window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DeviceSettings {
    /// A conversion target id (`kepub`, `azw3`…); empty means the best one for the device.
    pub format: String,
    /// Save-path template such as `{author}/{title}`; empty puts books straight in the folder.
    pub template: String,
    /// Kobo shelf sent books are put on; empty leaves the Kobo database alone.
    pub kobo_shelf: String,
    pub webdav: WebDavSettings,
}

/// A WebDAV folder (Nextcloud, ownCloud…) used like a device. The password is kept in the
/// system keyring, like the other passwords (see `secrets.rs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct WebDavSettings {
    pub url: String,
    pub user: String,
    pub password: String,
}

impl WebDavSettings {
    pub fn configured(&self) -> bool {
        !self.url.trim().is_empty()
    }
}

/// How a device is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Usb,
    Mtp,
    Cloud,
    /// A device plugin (sandboxed WebAssembly) found it.
    Plugin,
}

/// All device drivers together; picks the right one from a device's id.
pub struct Drivers {
    mass: MassStorageDriver,
    mtp: Option<MtpGvfsDriver>,
    dav: Option<WebDavDriver>,
    /// Device plugins that are switched on.
    plugins: std::sync::Arc<crate::plugins::Loaded>,
}

/// Mount roots from `$BOOKX_DEVICE_ROOTS` (colon separated, no `/proc/mounts`) when set,
/// otherwise the usual removable-media locations. MTP mounts come from `$BOOKX_MTP_ROOT`, or gvfs
/// unless fake device roots are in use.
pub fn drivers(settings: &DeviceSettings) -> Drivers {
    let mass = driver_for_env();
    Drivers::from_mass(mass, settings)
}

fn driver_for_env() -> MassStorageDriver {
    match std::env::var("BOOKX_DEVICE_ROOTS") {
        Ok(v) if !v.trim().is_empty() => MassStorageDriver::with_roots(
            v.split(':')
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .collect(),
        ),
        _ => MassStorageDriver::new(),
    }
}

impl Drivers {
    /// The given USB driver plus MTP and the cloud folder as the environment and settings say.
    pub fn from_mass(mass: MassStorageDriver, settings: &DeviceSettings) -> Drivers {
        let fake_roots = std::env::var("BOOKX_DEVICE_ROOTS")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let mtp = match std::env::var("BOOKX_MTP_ROOT")
            .ok()
            .filter(|v| !v.is_empty())
        {
            Some(d) => Some(MtpGvfsDriver::with_gvfs_dir(PathBuf::from(d))),
            None if fake_roots.is_some() => None,
            None => Some(MtpGvfsDriver::new()),
        };
        let dav = settings
            .webdav
            .configured()
            .then(|| {
                WebDav::new(
                    &settings.webdav.url,
                    &settings.webdav.user,
                    &settings.webdav.password,
                )
            })
            .and_then(|r| r.ok())
            .map(WebDavDriver::new);
        Drivers {
            mass,
            mtp,
            dav,
            plugins: crate::plugins::current(),
        }
    }

    /// Only USB mass storage on the given roots (tests).
    pub fn mass_only(mass: MassStorageDriver) -> Drivers {
        Drivers {
            mass,
            mtp: None,
            dav: None,
            plugins: Default::default(),
        }
    }

    #[cfg(test)]
    pub fn with_plugins(mut self, plugins: std::sync::Arc<crate::plugins::Loaded>) -> Drivers {
        self.plugins = plugins;
        self
    }

    /// The device plugin that owns `device` (its ids start with the plugin id and a colon).
    fn plugin_of(&self, device: &DeviceInfo) -> Option<&dyn DeviceDriver> {
        self.plugins
            .devices
            .iter()
            .find(|d| device.id.starts_with(&format!("{}:", d.id())))
    }

    #[cfg(test)]
    pub fn with_mtp(mut self, mtp: MtpGvfsDriver) -> Drivers {
        self.mtp = Some(mtp);
        self
    }

    /// Everything connected right now. The cloud folder is always listed when it is set up.
    pub fn detect(&self) -> Vec<DeviceInfo> {
        let mut v = self.mass.detect();
        if let Some(m) = &self.mtp {
            v.extend(m.detect());
        }
        v.extend(self.plugins.devices.iter().flat_map(|d| d.detect()));
        if let Some(d) = &self.dav {
            v.push(d.device());
        }
        v
    }

    /// Only the devices that need a cable (what the book menu offers).
    pub fn detect_physical(&self) -> Vec<DeviceInfo> {
        let mut v = self.mass.detect();
        if let Some(m) = &self.mtp {
            v.extend(m.detect());
        }
        v.extend(self.plugins.devices.iter().flat_map(|d| d.detect()));
        v
    }

    pub fn backend(&self, device: &DeviceInfo) -> Backend {
        if self.plugin_of(device).is_some() {
            Backend::Plugin
        } else if device.id.starts_with("webdav:") {
            Backend::Cloud
        } else if device.id.starts_with("mtp:") {
            Backend::Mtp
        } else {
            Backend::Usb
        }
    }

    pub fn kind(&self, device: &DeviceInfo) -> Option<DeviceKind> {
        match self.backend(device) {
            Backend::Usb => self.mass.kind_of(device),
            Backend::Mtp => self.mtp.as_ref().map(|m| m.kind_of(device)),
            Backend::Cloud | Backend::Plugin => None,
        }
    }

    pub fn storage(&self, device: &DeviceInfo) -> Storage {
        match self.backend(device) {
            Backend::Cloud => Storage::Main,
            _ => devfs::storage_of(&device.mount),
        }
    }

    /// Free space; MTP and cloud folders often cannot say.
    pub fn space(&self, device: &DeviceInfo) -> Option<Space> {
        match self.backend(device) {
            Backend::Cloud => self.dav.as_ref().and_then(|d| d.dav().quota()),
            _ => devfs::free_space(&device.mount),
        }
    }

    /// Formats the device opens, best first (conversion targets only).
    pub fn formats(&self, device: &DeviceInfo) -> Vec<&'static Target> {
        let mut chain = self.plugin_formats(device);
        for t in format_chain(self.kind(device), "") {
            if !chain.iter().any(|c| c.id == t.id) {
                chain.push(t);
            }
        }
        chain
    }

    /// Targets to try for a send: the user's choice, the plugin's or the device's own, then EPUB.
    pub fn chain(&self, device: &DeviceInfo, preferred: &str) -> Vec<&'static Target> {
        let mut chain: Vec<&'static Target> = export::target(preferred).into_iter().collect();
        for t in self.formats(device) {
            if !chain.iter().any(|c| c.id == t.id) {
                chain.push(t);
            }
        }
        chain
    }

    /// The conversion targets a device plugin asks for, best first (empty for other devices).
    fn plugin_formats(&self, device: &DeviceInfo) -> Vec<&'static Target> {
        let Some(d) = self.plugin_of(device) else {
            return Vec::new();
        };
        let plugin_targets = crate::plugins::writer_targets();
        d.preferred_formats()
            .iter()
            .filter_map(|f| {
                export::TARGETS
                    .iter()
                    .chain(plugin_targets.iter().copied())
                    .find(|t| t.extension == *f)
            })
            .collect()
    }

    /// One line for the window: "Kobo · Main memory · 12.3 GB free of 32.0 GB".
    pub fn summary(&self, device: &DeviceInfo) -> String {
        let mut parts = Vec::new();
        match self.backend(device) {
            Backend::Cloud => parts.push(tr("WebDAV folder")),
            Backend::Mtp => parts.push(tr("MTP (mounted by the desktop)")),
            Backend::Plugin => parts.push(tr("Device plugin")),
            Backend::Usb => {
                if let Some(k) = self.kind(device) {
                    parts.push(k.label().to_string());
                }
                parts.push(self.storage(device).label().to_string());
            }
        }
        match self.space(device) {
            Some(s) => parts.push(s.describe()),
            None => parts.push("free space unknown".to_string()),
        }
        parts.join(" · ")
    }

    /// Books on the device.
    pub fn books(&self, device: &DeviceInfo) -> Result<Vec<DeviceBook>> {
        match self.backend(device) {
            Backend::Cloud => self
                .dav
                .as_ref()
                .ok_or_else(|| anyhow!("the cloud folder is not set up"))?
                .books(),
            _ => {
                if !device.mount.is_dir() {
                    bail!("{} is not connected any more", device.name);
                }
                Ok(devfs::list_books(&device.mount))
            }
        }
    }

    /// Removes one book (and its reading-state folder) from the device.
    pub fn remove(&self, device: &DeviceInfo, book: &DeviceBook) -> Result<()> {
        match self.backend(device) {
            Backend::Cloud => self
                .dav
                .as_ref()
                .ok_or_else(|| anyhow!("the cloud folder is not set up"))?
                .dav()
                .delete_book(&book.rel),
            _ => devfs::remove_book(&device.mount, &book.path).map(|_| ()),
        }
    }

    /// Reading state found on the device (Kobo database, KOReader sidecars).
    #[cfg(test)]
    pub fn states(&self, device: &DeviceInfo, books: &[DeviceBook]) -> Vec<(PathBuf, DeviceState)> {
        match self.backend(device) {
            Backend::Cloud => Vec::new(),
            _ => reading::collect_states(&device.mount, books),
        }
    }

    /// Shelves on a Kobo (empty for other devices or an unreadable database).
    pub fn kobo_shelves(&self, device: &DeviceInfo) -> Vec<bookx_serve::kobo::Shelf> {
        if self.kind(device) != Some(DeviceKind::Kobo) {
            return Vec::new();
        }
        bookx_serve::devices::kobo_db_path(&device.mount)
            .and_then(|db| bookx_serve::kobo::read_shelves(&db).ok())
            .unwrap_or_default()
    }

    /// Kobo shelves of each book (by file), for the book list.
    pub fn shelf_map(
        &self,
        device: &DeviceInfo,
    ) -> std::collections::HashMap<PathBuf, Vec<String>> {
        let mut map: std::collections::HashMap<PathBuf, Vec<String>> = Default::default();
        for shelf in self.kobo_shelves(device) {
            for id in &shelf.content_ids {
                if let Some(rel) = bookx_serve::kobo::rel_of_content_id(id) {
                    map.entry(device.mount.join(rel))
                        .or_default()
                        .push(shelf.name.clone());
                }
            }
        }
        map
    }

    fn send_in(
        &self,
        device: &DeviceInfo,
        dirs: &[String],
        name: &str,
        bytes: &[u8],
    ) -> Result<PathBuf> {
        match self.backend(device) {
            Backend::Usb => self.mass.send_in(device, dirs, name, bytes),
            // the plugin names the place itself; the save-path template does not apply
            Backend::Plugin => self
                .plugin_of(device)
                .ok_or_else(|| anyhow!("the device plugin is not enabled"))?
                .send(device, name, bytes),
            Backend::Mtp => self
                .mtp
                .as_ref()
                .ok_or_else(|| anyhow!("MTP is not available"))?
                .send_in(device, dirs, name, bytes),
            Backend::Cloud => self
                .dav
                .as_ref()
                .ok_or_else(|| anyhow!("the cloud folder is not set up"))?
                .send_in(dirs, name, bytes),
        }
    }
}

/// Conversion targets to try, best first: the user's choice (when valid), then what the device
/// prefers, and plain EPUB as the last resort.
pub fn format_chain(kind: Option<DeviceKind>, preferred: &str) -> Vec<&'static Target> {
    let available: Vec<&str> = export::TARGETS.iter().map(|t| t.extension).collect();
    let mut chain: Vec<&'static Target> = Vec::new();
    let mut push = |t: Option<&'static Target>| {
        if let Some(t) = t {
            if !chain.iter().any(|c| c.id == t.id) {
                chain.push(t);
            }
        }
    };
    push(export::target(preferred));
    if let Some(k) = kind {
        for f in k.preferred_formats() {
            if let Some(w) = choose_format(k, &[f]).filter(|w| available.contains(w)) {
                push(export::TARGETS.iter().find(|t| t.extension == w));
            }
        }
    }
    push(export::target("epub"));
    chain
}

/// The conversion target for a device: the first format it prefers that Bookx can write.
#[cfg(test)]
pub fn target_for(drivers: &Drivers, device: &DeviceInfo) -> Result<&'static Target> {
    drivers
        .formats(device)
        .first()
        .copied()
        .ok_or_else(|| anyhow!("{} opens no format Bookx can write", device.name))
}

/// Converts with the first target of `chain` that works. Returns the target, the bytes and the
/// labels of the targets that failed before it.
pub fn convert_with_fallback(
    chain: &[&'static Target],
    convert: &dyn Fn(&Target) -> Result<Vec<u8>>,
) -> Result<(&'static Target, Vec<u8>, Vec<String>)> {
    let mut failed = Vec::new();
    for t in chain {
        match convert(t) {
            Ok(b) => return Ok((t, b, failed)),
            Err(e) => failed.push(format!("{}: {e:#}", t.label)),
        }
    }
    bail!("could not convert the book ({})", failed.join("; "))
}

// -- what a send will do, decided once for the preview and the real send ----------------------

/// Whether a library EPUB is changed on its way to the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conversion {
    /// The file is copied as it is (plus the notes chapter when there are notes).
    AsIs,
    Converted,
}

/// How the EPUB of the library reaches target `t`. `hooked`: a plugin or patch takes part in
/// conversions to `t` (see `plugins::convert_is_hooked`), which counts as a conversion. The send
/// path and the preview in the Device window both ask this.
pub fn conversion_of(t: &Target, hooked: bool) -> Conversion {
    if t.id == "epub" && t.preset.is_none() && !hooked {
        Conversion::AsIs
    } else {
        Conversion::Converted
    }
}

/// What the Device window knows about the device when it plans a send.
pub struct SendContext<'a> {
    /// The formats to try, best first (`Drivers::chain`).
    pub chain: &'a [&'static Target],
    /// The save-path template.
    pub template: &'a str,
    /// Free bytes on the device, when known.
    pub free: Option<u64>,
    /// The books already on the device.
    pub on_device: &'a [DeviceBook],
}

/// A library book about to be sent.
pub struct SendCandidate<'a> {
    pub fields: &'a BookFields,
    pub size: u64,
    /// Highlights and bookmarks that would be appended (0 when the notes are switched off).
    pub notes: usize,
    pub file_missing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendWarning {
    /// The book file is not in the library folder any more.
    FileMissing,
    /// More bytes than the device has free.
    TooBig { size: u64, free: u64 },
    /// A book of that name is on the device already (a send never overwrites: it would add a copy).
    AlreadyOnDevice,
    /// The save-path template cannot be used.
    BadTemplate(String),
    /// The device opens no format Bookx can write.
    NoFormat,
}

impl SendWarning {
    pub fn text(&self) -> String {
        match self {
            SendWarning::FileMissing => tr("The book file is missing from the library."),
            SendWarning::TooBig { size, free } => trf(
                "Too big: {} needed, {} free on the device.",
                &[&devfs::human_size(*size), &devfs::human_size(*free)],
            ),
            SendWarning::AlreadyOnDevice => {
                "Already on the device; sending adds a second copy (skipped unless you tick it)."
                    .into()
            }
            SendWarning::BadTemplate(p) => trf("The save path cannot be used: {}", &[&p]),
            SendWarning::NoFormat => tr("The device opens no format Bookx can write."),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendPlan {
    /// The format that is tried first; later ones are fall-backs when a conversion fails.
    pub target: Option<&'static Target>,
    pub conversion: Conversion,
    /// Highlights and bookmarks that are appended.
    pub notes: usize,
    /// The file name on the device (without the folders).
    pub file_name: String,
    pub warnings: Vec<SendWarning>,
}

impl SendPlan {
    /// `EPUB (sent as is)`, `EPUB → KEPUB (converted)`.
    pub fn format_line(&self) -> String {
        let Some(t) = self.target else {
            return tr("No format");
        };
        match self.conversion {
            Conversion::AsIs => format!("{} (sent as is)", t.id.to_uppercase()),
            Conversion::Converted => trf("EPUB → {} (converted)", &[&t.id.to_uppercase()]),
        }
    }

    /// `+ 3 notes added as a last section`, empty without notes.
    pub fn notes_line(&self) -> String {
        match self.notes {
            0 => String::new(),
            1 => "+ 1 note added as a last section".into(),
            n => format!("+ {n} notes added as a last section"),
        }
    }

    /// A book with a warning is left out unless the user ticks it.
    pub fn default_included(&self) -> bool {
        self.warnings.is_empty()
    }
}

/// Decides how one book will be sent. Pure: the Device window shows this, and `send_full` takes
/// the same decisions with the same helpers ([`Drivers::chain`], [`conversion_of`], the template).
pub fn plan_send(
    ctx: &SendContext,
    hooked: &dyn Fn(&Target) -> bool,
    c: &SendCandidate,
) -> SendPlan {
    let mut warnings = Vec::new();
    let target = ctx.chain.first().copied();
    if target.is_none() {
        warnings.push(SendWarning::NoFormat);
    }
    let conversion = target.map_or(Conversion::Converted, |t| conversion_of(t, hooked(t)));
    if c.file_missing {
        warnings.push(SendWarning::FileMissing);
    }
    let mut file_name = String::new();
    match devfs::render_template(ctx.template, c.fields) {
        Ok(save) => {
            file_name = format!("{}.{}", save.stem, target.map_or("epub", |t| t.extension));
            let key = |s: &str| {
                s.chars()
                    .filter(|c| c.is_alphanumeric())
                    .flat_map(|c| c.to_lowercase())
                    .collect::<String>()
            };
            let mine = key(&save.stem);
            if ctx.on_device.iter().any(|b| key(&b.stem()) == mine) {
                warnings.push(SendWarning::AlreadyOnDevice);
            }
        }
        Err(e) => warnings.push(SendWarning::BadTemplate(format!("{e:#}"))),
    }
    if let Some(free) = ctx.free {
        if c.size > free {
            warnings.push(SendWarning::TooBig { size: c.size, free });
        }
    }
    SendPlan {
        target,
        conversion,
        notes: c.notes,
        file_name,
        warnings,
    }
}

/// Options of one send.
#[derive(Debug, Clone, Default)]
pub struct SendOptions {
    pub format: String,
    pub template: String,
    pub kobo_shelf: String,
}

impl From<&DeviceSettings> for SendOptions {
    fn from(s: &DeviceSettings) -> Self {
        SendOptions {
            format: s.format.clone(),
            template: s.template.clone(),
            kobo_shelf: s.kobo_shelf.clone(),
        }
    }
}

/// Result of a send.
#[derive(Debug, PartialEq, Eq)]
pub struct Sent {
    pub path: PathBuf,
    pub format: &'static str,
    /// The KOReader sidecar written next to the book when it was sent with its notes.
    pub sidecar: Option<PathBuf>,
    /// Formats that failed before this one worked (empty when the first choice worked).
    pub fell_back_from: Vec<String>,
    /// Something that did not stop the send but the user should know (shelf not written…).
    pub note: Option<String>,
}

/// Readers that open EPUB and keep KOReader sidecars; the Kindle's own firmware does not.
fn takes_sidecar(kind: Option<DeviceKind>) -> bool {
    kind != Some(DeviceKind::Kindle)
}

/// The full send: the book (with its notes when `notes` has any) is converted with the first format
/// of the chain that works, named by the save-path template, copied to the device, put on the
/// Kobo shelf and given a KOReader sidecar.
pub fn send_full(
    drivers: &Drivers,
    device: &DeviceInfo,
    epub: &Path,
    fields: &BookFields,
    opts: &SendOptions,
    notes: Option<&Notes>,
) -> Result<Sent> {
    let mut bytes = std::fs::read(epub).with_context(|| format!("reading {}", epub.display()))?;
    let src_name = epub
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("book.epub")
        .to_string();
    let notes = notes.filter(|n| !n.is_empty());
    if let Some(n) = notes {
        bytes = send_notes::embed_appendix(&bytes, n)?;
    }
    let kind = drivers.kind(device);
    let chain = drivers.chain(device, &opts.format);
    let (target, converted, failed) = convert_with_fallback(&chain, &|t| match conversion_of(
        t,
        crate::plugins::convert_is_hooked(t),
    ) {
        Conversion::AsIs => Ok(bytes.clone()),
        Conversion::Converted => export::convert_epub_bytes(&bytes, &src_name, t),
    })?;
    let save = devfs::render_template(&opts.template, fields)?;
    let name = format!("{}.{}", save.stem, target.extension);
    let path = drivers.send_in(device, &save.dirs, &name, &converted)?;
    let mut note = None;
    if kind == Some(DeviceKind::Kobo) && !opts.kobo_shelf.trim().is_empty() {
        note = shelve(device, &path, &opts.kobo_shelf).err().map(|e| {
            trf(
                "Sent, but the shelf “{}” was not updated: {}",
                &[&opts.kobo_shelf, &format!("{:#}", e)],
            )
        });
    }
    let mut sidecar = None;
    if let Some(n) = notes {
        if matches!(drivers.backend(device), Backend::Usb | Backend::Mtp) && takes_sidecar(kind) {
            let chapters = crate::core::book::OpenBook::from_archive(
                crate::core::archive::Archive::from_bytes(bytes.clone())?,
            )
            .map(|b| send_notes::chapter_titles(&b))
            .unwrap_or_default();
            let file = reading::sidecar_file(&path);
            if let Some(dir) = file.parent() {
                if std::fs::create_dir_all(dir).is_ok() && !file.exists() {
                    let lua = send_notes::koreader_sidecar(&fields.title, &chapters, n);
                    if std::fs::write(&file, lua).is_ok() {
                        sidecar = Some(file);
                    }
                }
            }
        }
    }
    Ok(Sent {
        path,
        format: target.label,
        sidecar,
        fell_back_from: failed,
        note,
    })
}

/// Anything that can stand for the set of drivers: the set itself, or a bare USB driver.
pub trait AsDrivers {
    fn with_drivers<R>(&self, f: impl FnOnce(&Drivers) -> R) -> R;
}

impl AsDrivers for Drivers {
    fn with_drivers<R>(&self, f: impl FnOnce(&Drivers) -> R) -> R {
        f(self)
    }
}

impl AsDrivers for MassStorageDriver {
    fn with_drivers<R>(&self, f: impl FnOnce(&Drivers) -> R) -> R {
        f(&Drivers::mass_only(self.clone()))
    }
}

/// Converts the EPUB at `epub` for `device` and copies it there.
#[cfg(test)]
pub fn send_book(
    drv: &impl AsDrivers,
    device: &DeviceInfo,
    epub: &Path,
    title: &str,
) -> Result<Sent> {
    send_book_with(drv, device, epub, title, None)
}

/// Like [`send_book`], optionally together with the reader's notes.
pub fn send_book_with(
    drv: &impl AsDrivers,
    device: &DeviceInfo,
    epub: &Path,
    title: &str,
    notes: Option<&Notes>,
) -> Result<Sent> {
    let fields = BookFields {
        title: title.to_string(),
        ..Default::default()
    };
    drv.with_drivers(|d| send_full(d, device, epub, &fields, &SendOptions::default(), notes))
}

/// Mount points that could hold a device a plugin knows (the children of the device roots).
pub fn candidate_mounts() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in driver().roots() {
        if let Ok(rd) = std::fs::read_dir(root) {
            let mut dirs: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            dirs.sort();
            out.extend(dirs);
        }
    }
    out
}

/// Every connected device: USB, MTP, the cloud folder, and the devices plugins find.
pub fn detect_all() -> Vec<DeviceInfo> {
    dedupe(drivers(&settings()).detect())
}

/// Devices that need a cable (what the book menu offers): no cloud folder.
pub fn detect_physical() -> Vec<DeviceInfo> {
    dedupe(drivers(&settings()).detect_physical())
}

fn dedupe(mut v: Vec<DeviceInfo>) -> Vec<DeviceInfo> {
    let mut seen = std::collections::HashSet::new();
    v.retain(|d| seen.insert(d.id.clone()));
    v
}

/// The USB driver on the roots from `$BOOKX_DEVICE_ROOTS`, else the usual locations.
pub fn driver() -> MassStorageDriver {
    drivers(&DeviceSettings::default()).mass
}

static SETTINGS: std::sync::Mutex<Option<DeviceSettings>> = std::sync::Mutex::new(None);

/// The device settings in force (loaded from the configuration on first use).
pub fn settings() -> DeviceSettings {
    let mut g = SETTINGS.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(|| crate::config::Config::load().device)
        .clone()
}

/// Makes `s` the settings used by sends from now on.
pub fn set_settings(s: DeviceSettings) {
    *SETTINGS.lock().unwrap_or_else(|e| e.into_inner()) = Some(s);
}

fn shelve(device: &DeviceInfo, path: &Path, shelf: &str) -> Result<()> {
    let db = bookx_serve::devices::kobo_db_path(&device.mount)
        .ok_or_else(|| anyhow!("this Kobo has no library database yet"))?;
    let real_mount = std::fs::canonicalize(&device.mount)?;
    let rel = path
        .strip_prefix(&real_mount)
        .or_else(|_| path.strip_prefix(&device.mount))
        .context("the book is not on the device")?;
    let rel = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    bookx_serve::kobo::add_to_shelf(&db, shelf, &[bookx_serve::kobo::content_id(&rel)], &now)?;
    Ok(())
}

/// Metadata fields a save-path template can use.
pub fn fields_of(meta: &crate::core::opf::Metadata) -> BookFields {
    BookFields {
        title: meta.title.clone(),
        authors: meta.authors.clone(),
        series: Some(meta.series.clone()).filter(|s| !s.is_empty()),
        series_index: Some(meta.series_index).filter(|_| !meta.series.is_empty()),
        language: meta.language.clone(),
    }
}

/// Removes the books from the device. Returns how many went and why the others did not.
pub fn remove_books(
    drivers: &Drivers,
    device: &DeviceInfo,
    books: &[DeviceBook],
) -> (usize, Vec<String>) {
    let (mut ok, mut errors) = (0, Vec::new());
    for b in books {
        match drivers.remove(device, b) {
            Ok(()) => ok += 1,
            Err(e) => errors.push(format!("{}: {e:#}", b.rel)),
        }
    }
    (ok, errors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::book::OpenBook;
    use std::fs;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "bookx-dev-{tag}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn epub(dir: &Path) -> PathBuf {
        let md = crate::core::opf::Metadata {
            title: "Sent Book".into(),
            authors: vec!["Ann".into()],
            ..Default::default()
        };
        let ch = crate::core::convert::Chapter {
            title: "One".into(),
            body: "<p>Words for the device.</p>".into(),
        };
        let p = dir.join("book.epub");
        fs::write(
            &p,
            crate::core::convert::create_epub(&md, &[ch], None)
                .to_bytes()
                .unwrap(),
        )
        .unwrap();
        p
    }

    /// Fake mounts: a Kobo, a Kindle and a plain reader with a Books folder.
    fn mounts(root: &Path) {
        fs::create_dir_all(root.join("KOBOeReader/.kobo")).unwrap();
        fs::create_dir_all(root.join("Kindle/documents")).unwrap();
        fs::create_dir_all(root.join("Kindle/system")).unwrap();
        fs::create_dir_all(root.join("Boox/Books")).unwrap();
    }

    fn find(drv: &Drivers, part: &str) -> DeviceInfo {
        drv.detect()
            .into_iter()
            .find(|d| d.mount.to_string_lossy().contains(part))
            .unwrap_or_else(|| panic!("no device {part}"))
    }

    #[test]
    fn detects_the_fake_devices() {
        let root = tmp("detect");
        mounts(&root);
        let drv = Drivers::mass_only(MassStorageDriver::with_roots(vec![root.clone()]));
        assert_eq!(drv.detect().len(), 3);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn each_device_gets_the_best_format() {
        let root = tmp("targets");
        mounts(&root);
        let drv = Drivers::mass_only(MassStorageDriver::with_roots(vec![root.clone()]));
        assert_eq!(target_for(&drv, &find(&drv, "KOBO")).unwrap().id, "kepub");
        assert_eq!(target_for(&drv, &find(&drv, "Kindle")).unwrap().id, "azw3");
        assert_eq!(target_for(&drv, &find(&drv, "Boox")).unwrap().id, "epub");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sends_a_kepub_to_a_kobo() {
        let root = tmp("kobo");
        mounts(&root);
        let src = tmp("src");
        let drv = Drivers::mass_only(MassStorageDriver::with_roots(vec![root.clone()]));
        let dev = find(&drv, "KOBO");
        let sent = send_book(&drv, &dev, &epub(&src), "Sent Book").unwrap();
        assert!(sent.path.starts_with(&root), "{:?}", sent.path);
        assert!(sent
            .path
            .to_string_lossy()
            .ends_with("Sent Book.kepub.epub"));
        assert_eq!(sent.format, "KEPUB (Kobo)");
        let bytes = fs::read(&sent.path).unwrap();
        assert!(bytes.starts_with(b"PK"));
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(src);
    }

    #[test]
    fn sends_an_azw3_to_a_kindle_and_an_epub_to_a_plain_reader() {
        let root = tmp("others");
        mounts(&root);
        let src = tmp("src2");
        let drv = Drivers::mass_only(MassStorageDriver::with_roots(vec![root.clone()]));
        let book = epub(&src);
        let k = send_book(&drv, &find(&drv, "Kindle"), &book, "Sent Book").unwrap();
        assert!(k.path.to_string_lossy().contains("documents"));
        let bytes = fs::read(&k.path).unwrap();
        assert_eq!(&bytes[60..68], b"BOOKMOBI");
        let g = send_book(&drv, &find(&drv, "Boox"), &book, "Sent Book").unwrap();
        assert!(g.path.to_string_lossy().contains("Books"));
        assert!(g.path.to_string_lossy().ends_with(".epub"));
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(src);
    }

    fn candidate_fields(title: &str) -> BookFields {
        BookFields {
            title: title.into(),
            ..Default::default()
        }
    }

    #[test]
    fn the_plan_says_which_format_and_whether_it_is_converted() {
        let f = candidate_fields("Dune");
        let cand = SendCandidate {
            fields: &f,
            size: 1000,
            notes: 3,
            file_missing: false,
        };
        let none = |_: &Target| false;
        let kepub = format_chain(Some(DeviceKind::Kobo), "");
        let ctx = SendContext {
            chain: &kepub,
            template: "{title}",
            free: Some(1_000_000),
            on_device: &[],
        };
        let p = plan_send(&ctx, &none, &cand);
        assert_eq!(p.format_line(), "EPUB → KEPUB (converted)");
        assert_eq!(p.notes_line(), "+ 3 notes added as a last section");
        assert_eq!(p.file_name, "Dune.kepub.epub");
        assert!(p.warnings.is_empty() && p.default_included());
        let epub = format_chain(Some(DeviceKind::Generic), "");
        let ctx2 = SendContext {
            chain: &epub,
            ..ctx
        };
        let p = plan_send(&ctx2, &none, &SendCandidate { notes: 1, ..cand });
        assert_eq!(p.format_line(), "EPUB (sent as is)");
        assert_eq!(p.notes_line(), "+ 1 note added as a last section");
        assert_eq!(p.conversion, Conversion::AsIs);
        // a plugin or patch that takes part makes even an EPUB a conversion
        let hooked = |_: &Target| true;
        let p = plan_send(&ctx2, &hooked, &SendCandidate { notes: 0, ..cand });
        assert_eq!(p.format_line(), "EPUB → EPUB (converted)");
        assert_eq!(p.notes_line(), "");
        let kindle = format_chain(Some(DeviceKind::Kindle), "");
        let ctx3 = SendContext {
            chain: &kindle,
            ..ctx2
        };
        assert_eq!(
            plan_send(&ctx3, &none, &cand).format_line(),
            "EPUB → AZW3 (converted)"
        );
    }

    #[test]
    fn the_plan_warns_about_big_missing_and_duplicate_books() {
        let f = candidate_fields("Dune");
        let chain = format_chain(None, "");
        let on = DeviceBook {
            path: PathBuf::from("/m/Bookx/dune.epub"),
            rel: "Bookx/dune.epub".into(),
            size: 5,
            modified: None,
        };
        let ctx = SendContext {
            chain: &chain,
            template: "{title}",
            free: Some(100),
            on_device: std::slice::from_ref(&on),
        };
        let cand = SendCandidate {
            fields: &f,
            size: 500,
            notes: 0,
            file_missing: true,
        };
        let p = plan_send(&ctx, &|_| false, &cand);
        assert!(p.warnings.contains(&SendWarning::FileMissing));
        assert!(p.warnings.contains(&SendWarning::AlreadyOnDevice));
        assert!(p.warnings.iter().any(|w| matches!(
            w,
            SendWarning::TooBig {
                size: 500,
                free: 100
            }
        )));
        assert!(!p.default_included());
        let bad = SendContext {
            template: "{nonsense}",
            ..ctx
        };
        let p = plan_send(&bad, &|_| false, &cand);
        assert!(p
            .warnings
            .iter()
            .any(|w| matches!(w, SendWarning::BadTemplate(_))));
        let empty = SendContext { chain: &[], ..bad };
        assert!(plan_send(&empty, &|_| false, &cand)
            .warnings
            .contains(&SendWarning::NoFormat));
    }

    #[test]
    fn the_real_send_does_what_the_plan_said() {
        let root = tmp("plan-vs-send");
        mounts(&root);
        let src = tmp("plan-vs-send-src");
        let drv = Drivers::mass_only(MassStorageDriver::with_roots(vec![root.clone()]));
        let book = epub(&src);
        for (name, ext_expected, as_is) in [("Kindle", "azw3", false), ("Boox", "epub", true)] {
            let dev = find(&drv, name);
            let fields = candidate_fields("Planned");
            let opts = SendOptions {
                template: "{title}".into(),
                ..Default::default()
            };
            let chain = drv.chain(&dev, &opts.format);
            let ctx = SendContext {
                chain: &chain,
                template: &opts.template,
                free: None,
                on_device: &[],
            };
            let plan = plan_send(
                &ctx,
                &|t| crate::plugins::convert_is_hooked(t),
                &SendCandidate {
                    fields: &fields,
                    size: 1,
                    notes: 0,
                    file_missing: false,
                },
            );
            let sent = send_full(&drv, &dev, &book, &fields, &opts, None).unwrap();
            assert_eq!(sent.format, plan.target.unwrap().label);
            assert_eq!(
                sent.path.file_name().unwrap().to_string_lossy(),
                plan.file_name
            );
            assert!(plan.file_name.ends_with(ext_expected));
            assert_eq!(plan.conversion == Conversion::AsIs, as_is);
            if as_is {
                assert_eq!(fs::read(&sent.path).unwrap(), fs::read(&book).unwrap());
            } else {
                assert_ne!(fs::read(&sent.path).unwrap(), fs::read(&book).unwrap());
            }
        }
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(src);
    }

    #[test]
    fn sending_twice_never_overwrites() {
        let root = tmp("twice");
        mounts(&root);
        let src = tmp("src3");
        let drv = Drivers::mass_only(MassStorageDriver::with_roots(vec![root.clone()]));
        let dev = find(&drv, "Boox");
        let book = epub(&src);
        let a = send_book(&drv, &dev, &book, "Sent Book").unwrap();
        let b = send_book(&drv, &dev, &book, "Sent Book").unwrap();
        assert_ne!(a.path, b.path);
        assert!(a.path.exists() && b.path.exists());
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(src);
    }

    #[test]
    fn a_hostile_title_stays_inside_the_device() {
        let root = tmp("evil");
        mounts(&root);
        let src = tmp("src4");
        let drv = Drivers::mass_only(MassStorageDriver::with_roots(vec![root.clone()]));
        let dev = find(&drv, "Boox");
        let sent = send_book(&drv, &dev, &epub(&src), "../../escape/../x").unwrap();
        assert!(sent.path.starts_with(root.join("Boox")), "{:?}", sent.path);
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(src);
    }

    #[test]
    fn unplugging_is_an_error_not_a_crash() {
        let root = tmp("gone");
        mounts(&root);
        let src = tmp("src5");
        let drv = Drivers::mass_only(MassStorageDriver::with_roots(vec![root.clone()]));
        let dev = find(&drv, "Boox");
        fs::remove_dir_all(&dev.mount).unwrap();
        assert!(send_book(&drv, &dev, &epub(&src), "X").is_err());
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(src);
    }

    #[test]
    fn no_roots_means_no_devices() {
        let drv = Drivers::mass_only(MassStorageDriver::with_roots(vec![PathBuf::from(
            "/nonexistent/mounts",
        )]));
        assert!(drv.detect().is_empty());
    }

    fn notes() -> Notes {
        let created = chrono::Utc::now();
        Notes {
            highlights: vec![bookx_core::library::Highlight {
                id: "h".into(),
                spine: 0,
                start: 0,
                end: 5,
                color: "yellow".into(),
                style: String::new(),
                note: "remember".into(),
                text: "Words for".into(),
                created,
            }],
            bookmarks: vec![],
        }
    }

    fn zip_has(bytes: &[u8], needle: &str) -> bool {
        let z = bookx_core::archive::Archive::from_bytes(bytes.to_vec()).unwrap();
        z.names()
            .iter()
            .filter_map(|n| z.get_string(n))
            .any(|t| t.contains(needle))
    }

    #[test]
    fn notes_go_to_a_kobo_as_an_appendix_and_a_koreader_sidecar() {
        let root = tmp("notes-kobo");
        mounts(&root);
        let src = tmp("src6");
        let drv = Drivers::mass_only(MassStorageDriver::with_roots(vec![root.clone()]));
        let dev = find(&drv, "KOBO");
        let sent = send_book_with(&drv, &dev, &epub(&src), "Sent Book", Some(&notes())).unwrap();
        assert!(zip_has(&fs::read(&sent.path).unwrap(), "remember"));
        let side = sent.sidecar.expect("sidecar");
        assert!(side.starts_with(&root));
        assert!(
            side.ends_with("Sent Book.kepub.sdr/metadata.epub.lua"),
            "{side:?}"
        );
        assert!(fs::read_to_string(side).unwrap().contains("remember"));
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(src);
    }

    #[test]
    fn a_kindle_gets_the_appendix_but_no_sidecar() {
        let root = tmp("notes-kindle");
        mounts(&root);
        let src = tmp("src7");
        let drv = Drivers::mass_only(MassStorageDriver::with_roots(vec![root.clone()]));
        let sent = send_book_with(
            &drv,
            &find(&drv, "Kindle"),
            &epub(&src),
            "Sent Book",
            Some(&notes()),
        )
        .unwrap();
        assert!(sent.sidecar.is_none());
        let bytes = fs::read(&sent.path).unwrap();
        let back =
            OpenBook::from_archive(bookx_formats::read_any("x.azw3", &bytes).unwrap()).unwrap();
        let text: String = back
            .spine_paths()
            .iter()
            .filter_map(|p| back.chapter_source(p))
            .collect();
        assert!(text.contains("remember"), "{text}");
        assert!(!sent.path.with_extension("sdr").exists());
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(src);
    }

    #[test]
    fn without_annotations_the_book_is_sent_as_it_is() {
        let root = tmp("notes-none");
        mounts(&root);
        let src = tmp("src8");
        let drv = Drivers::mass_only(MassStorageDriver::with_roots(vec![root.clone()]));
        let dev = find(&drv, "Boox");
        let book = epub(&src);
        let plain = send_book(&drv, &dev, &book, "A").unwrap();
        let empty = send_book_with(&drv, &dev, &book, "B", Some(&Notes::default())).unwrap();
        assert!(empty.sidecar.is_none());
        assert_eq!(
            fs::read(&plain.path).unwrap().len(),
            fs::read(&empty.path).unwrap().len()
        );
        let sent = send_book_with(&drv, &dev, &book, "C", Some(&notes())).unwrap();
        assert!(sent.sidecar.is_some());
        assert!(zip_has(&fs::read(&sent.path).unwrap(), "remember"));
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(src);
    }

    #[test]
    fn a_second_send_gets_its_own_sidecar_and_keeps_the_first() {
        let root = tmp("notes-twice");
        mounts(&root);
        let src = tmp("src9");
        let drv = Drivers::mass_only(MassStorageDriver::with_roots(vec![root.clone()]));
        let dev = find(&drv, "Boox");
        let book = epub(&src);
        let a = send_book_with(&drv, &dev, &book, "T", Some(&notes())).unwrap();
        let b = send_book_with(&drv, &dev, &book, "T", Some(&notes())).unwrap();
        assert_ne!(a.sidecar, b.sidecar);
        assert!(a.sidecar.unwrap().exists() && b.sidecar.unwrap().exists());
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(src);
    }

    fn plugin_drivers(root: &Path, ext: &str) -> (Drivers, PathBuf) {
        use crate::plugins::test_support as ts;
        let mount = root.join("mounts/PluginReader");
        fs::create_dir_all(&mount).unwrap();
        let mj = serde_json::to_string(&mount.to_string_lossy()).unwrap();
        ts::install(
            &root.join("plugins"),
            "dev",
            "test.device",
            "device",
            &format!("extensions = [{ext}]"),
            &ts::canned(&[
                (
                    b'd',
                    format!(r#"{{"ok":[{{"id":"r1","name":"Plugin Reader","mount":{mj}}}]}}"#),
                ),
                (b's', r#"{"ok":{"path":"books/last.bin"}}"#.to_string()),
            ]),
        );
        let mut m = ts::manager(&root.join("plugins"));
        assert!(!m.is_enabled("test.device"), "disabled by default");
        m.set_enabled("test.device", true).unwrap();
        let mut loaded = crate::plugins::Loaded::default();
        let m2 = mount.clone();
        let errs = m.register_devices(
            &mut loaded.devices,
            std::sync::Arc::new(move || vec![m2.clone()]),
        );
        assert!(errs.is_empty(), "{errs:?}");
        let d = Drivers::mass_only(MassStorageDriver::with_roots(vec![]))
            .with_plugins(std::sync::Arc::new(loaded));
        (d, mount)
    }

    #[test]
    fn a_device_plugin_appears_as_a_device_and_gets_its_preferred_format() {
        let root = std::env::temp_dir().join(format!("bookx-devplug-{}", uuid::Uuid::new_v4()));
        let (drivers, mount) = plugin_drivers(&root, "\"fb2\", \"epub\"");
        let found = drivers.detect();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "test.device:r1");
        assert_eq!(drivers.backend(&found[0]), Backend::Plugin);
        assert_eq!(drivers.formats(&found[0])[0].id, "fb2");
        assert!(drivers.summary(&found[0]).contains("Device plugin"));
        let src = root.join("src");
        fs::create_dir_all(&src).unwrap();
        let sent = send_book(&drivers, &found[0], &epub(&src), "My Book").unwrap();
        assert_eq!(sent.path, mount.join("books/last.bin"));
        assert!(sent.format.contains("FictionBook"), "{}", sent.format);
        assert!(fs::read(&sent.path)
            .unwrap()
            .windows(11)
            .any(|w| w == b"FictionBook"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_device_plugin_asking_for_an_unknown_format_gets_epub() {
        let root = std::env::temp_dir().join(format!("bookx-devplug2-{}", uuid::Uuid::new_v4()));
        let (drivers, mount) = plugin_drivers(&root, "\"lit\"");
        let dev = drivers.detect().remove(0);
        let src = root.join("src");
        fs::create_dir_all(&src).unwrap();
        let sent = send_book(&drivers, &dev, &epub(&src), "T").unwrap();
        assert!(sent.format.contains("EPUB"), "{}", sent.format);
        assert_eq!(&fs::read(mount.join("books/last.bin")).unwrap()[..2], b"PK");
        let _ = fs::remove_dir_all(root);
    }

    // -- Device window logic ---------------------------------------------------------------

    fn meta_fields() -> BookFields {
        BookFields {
            title: "Sent Book".into(),
            authors: vec!["Ann Author".into()],
            series: Some("Saga".into()),
            series_index: Some(2.0),
            language: "en".into(),
        }
    }

    #[test]
    fn the_format_chain_starts_with_the_choice_and_ends_with_epub() {
        let ids = |c: Vec<&Target>| c.iter().map(|t| t.id).collect::<Vec<_>>();
        assert_eq!(
            ids(format_chain(Some(DeviceKind::Kobo), "")),
            ["kepub", "epub"]
        );
        assert_eq!(
            ids(format_chain(Some(DeviceKind::Kindle), "")),
            ["azw3", "mobi", "epub"]
        );
        assert_eq!(
            ids(format_chain(Some(DeviceKind::Kobo), "fb2")),
            ["fb2", "kepub", "epub"]
        );
        assert_eq!(
            ids(format_chain(Some(DeviceKind::Kobo), "kepub")),
            ["kepub", "epub"]
        );
        assert_eq!(ids(format_chain(None, "bogus")), ["epub"]);
        assert_eq!(ids(format_chain(Some(DeviceKind::Generic), "")), ["epub"]);
    }

    #[test]
    fn a_failing_conversion_falls_back_to_the_next_format() {
        let chain = format_chain(Some(DeviceKind::Kobo), "");
        let (t, bytes, failed) = convert_with_fallback(&chain, &|t| {
            if t.id == "kepub" {
                Err(anyhow!("boom"))
            } else {
                Ok(vec![1, 2, 3])
            }
        })
        .unwrap();
        assert_eq!((t.id, bytes), ("epub", vec![1, 2, 3]));
        assert_eq!(failed.len(), 1);
        assert!(failed[0].contains("KEPUB") && failed[0].contains("boom"));
        let e = convert_with_fallback(&chain, &|_| Err(anyhow!("nope"))).unwrap_err();
        assert!(e.to_string().contains("could not convert"), "{e}");
    }

    #[test]
    fn the_chosen_format_and_template_decide_name_and_folder() {
        let root = tmp("tpl");
        mounts(&root);
        let src = tmp("tpl-src");
        let drv = Drivers::mass_only(MassStorageDriver::with_roots(vec![root.clone()]));
        let dev = find(&drv, "KOBO");
        let opts = SendOptions {
            format: "epub".into(),
            template: "{author}/{series}/{series_index} - {title}".into(),
            kobo_shelf: String::new(),
        };
        let sent = send_full(&drv, &dev, &epub(&src), &meta_fields(), &opts, None).unwrap();
        assert!(
            sent.path
                .ends_with("Bookx/Ann Author/Saga/02 - Sent Book.epub"),
            "{:?}",
            sent.path
        );
        assert!(sent.fell_back_from.is_empty());
        assert_eq!(sent.format, "EPUB (cleaned up)");
        let bad = SendOptions {
            template: "{nope}".into(),
            ..Default::default()
        };
        assert!(send_full(&drv, &dev, &epub(&src), &meta_fields(), &bad, None).is_err());
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(src);
    }

    #[test]
    fn sent_books_land_on_the_kobo_shelf_and_a_broken_database_only_warns() {
        let root = tmp("shelf");
        mounts(&root);
        let src = tmp("shelf-src");
        let drv = Drivers::mass_only(MassStorageDriver::with_roots(vec![root.clone()]));
        let dev = find(&drv, "KOBO");
        let db = dev.mount.join(".kobo/KoboReader.sqlite");
        bookx_serve::kobo::create_test_database(&db).unwrap();
        let opts = SendOptions {
            kobo_shelf: "Bookx".into(),
            ..Default::default()
        };
        let sent = send_full(&drv, &dev, &epub(&src), &meta_fields(), &opts, None).unwrap();
        assert!(sent.note.is_none(), "{:?}", sent.note);
        let shelves = drv.kobo_shelves(&dev);
        assert_eq!(shelves.len(), 1);
        assert_eq!(
            shelves[0].content_ids,
            vec!["file:///mnt/onboard/Bookx/Sent Book.kepub.epub"]
        );
        let map = drv.shelf_map(&dev);
        assert_eq!(
            map[&dev.mount.join("Bookx/Sent Book.kepub.epub")],
            vec!["Bookx"]
        );
        // a database that is not a Kobo's: the book is still sent, the user is told
        fs::write(&db, b"not a database").unwrap();
        let again = send_full(&drv, &dev, &epub(&src), &meta_fields(), &opts, None).unwrap();
        assert!(again.path.exists());
        assert!(again.note.unwrap().contains("shelf"));
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(src);
    }

    #[test]
    fn books_on_a_device_can_be_listed_and_removed() {
        let root = tmp("list");
        mounts(&root);
        let src = tmp("list-src");
        let drv = Drivers::mass_only(MassStorageDriver::with_roots(vec![root.clone()]));
        let dev = find(&drv, "Boox");
        let a = send_book(&drv, &dev, &epub(&src), "Alpha").unwrap();
        send_book(&drv, &dev, &epub(&src), "Beta").unwrap();
        let books = drv.books(&dev).unwrap();
        assert_eq!(books.len(), 2);
        let alpha = books.iter().find(|b| b.path == a.path).unwrap();
        drv.remove(&dev, alpha).unwrap();
        assert_eq!(drv.books(&dev).unwrap().len(), 1);
        assert!(!a.path.exists());
        let summary = drv.summary(&dev);
        assert!(
            summary.contains("Main memory") && summary.contains("free"),
            "{summary}"
        );
        fs::remove_dir_all(&dev.mount).unwrap();
        assert!(drv.books(&dev).is_err());
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(src);
    }

    #[test]
    fn mtp_storages_are_devices_and_take_books_without_fsync() {
        let root = tmp("mtp");
        let gvfs = root.join("gvfs");
        fs::create_dir_all(gvfs.join("mtp:host=Tab_1/Internal storage")).unwrap();
        let src = tmp("mtp-src");
        let drv = Drivers::mass_only(MassStorageDriver::with_roots(vec![root.join("none")]))
            .with_mtp(MtpGvfsDriver::with_gvfs_dir(gvfs));
        let dev = drv.detect().remove(0);
        assert_eq!(drv.backend(&dev), Backend::Mtp);
        assert!(drv.summary(&dev).contains("MTP"));
        let opts = SendOptions {
            template: "{author}/{title}".into(),
            ..Default::default()
        };
        let sent = send_full(&drv, &dev, &epub(&src), &meta_fields(), &opts, None).unwrap();
        assert!(
            sent.path.ends_with("Books/Ann Author/Sent Book.epub"),
            "{:?}",
            sent.path
        );
        assert_eq!(drv.books(&dev).unwrap().len(), 1);
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(src);
    }

    /// Manual hardware test, never part of a normal run: sends public-domain test books through the
    /// real send path into a sandbox folder on a mounted Kobo. Needs `BOOKX_KOBO_SANDBOX` (a folder
    /// named `BookxSandbox` on the device) and `BOOKX_TEST_BOOKS` (a folder with `alice.epub`).
    /// Run with `--ignored`; it writes only below the sandbox and leaves the files for inspection.
    #[test]
    #[ignore]
    fn sandbox_send_to_a_real_kobo() {
        let sandbox =
            PathBuf::from(std::env::var("BOOKX_KOBO_SANDBOX").expect("BOOKX_KOBO_SANDBOX"));
        assert_eq!(
            sandbox.file_name().unwrap(),
            "BookxSandbox",
            "refusing any other folder"
        );
        let books = PathBuf::from(std::env::var("BOOKX_TEST_BOOKS").expect("BOOKX_TEST_BOOKS"));
        fs::create_dir_all(&sandbox).unwrap();
        let device = DeviceInfo {
            id: format!("kobo:{}", sandbox.display()),
            name: "Kobo sandbox".into(),
            mount: sandbox.clone(),
        };
        let drv = Drivers::mass_only(MassStorageDriver::with_roots(vec![]));
        // the shelf option stays off: the database on the device is never written
        let opts = SendOptions {
            format: "kepub".into(),
            template: "{title}".into(),
            kobo_shelf: String::new(),
        };
        let alice = books.join("alice.epub");
        let fields = BookFields {
            title: "Alice's Adventures in Wonderland".into(),
            authors: vec!["Lewis Carroll".into()],
            ..Default::default()
        };
        let sent = send_full(&drv, &device, &alice, &fields, &opts, None).unwrap();
        assert_eq!(sent.format, "KEPUB (Kobo)");
        assert!(sent.path.starts_with(&sandbox));

        let created = chrono::Utc::now();
        let notes = Notes {
            highlights: vec![crate::core::library::Highlight {
                id: "h1".into(),
                spine: 0,
                start: 0,
                end: 20,
                color: "yellow".into(),
                style: String::new(),
                note: "a note: with \"quotes\" & <markup>".into(),
                text: "Down the Rabbit-Hole".into(),
                created,
            }],
            bookmarks: vec![],
        };
        let fields2 = BookFields {
            title: "The Masque of the Red Death".into(),
            authors: vec!["Edgar Allan Poe".into()],
            ..Default::default()
        };
        let with_notes = send_full(
            &drv,
            &device,
            &books.join("tiny.epub"),
            &fields2,
            &opts,
            Some(&notes),
        )
        .unwrap();
        assert!(with_notes.sidecar.is_some());

        // byte identity: what is read back from the device is what was handed to the driver
        let bytes = fs::read(&alice).unwrap();
        let (_, expected, _) = convert_with_fallback(&drv.chain(&device, "kepub"), &|t| {
            export::convert_epub_bytes(&bytes, "alice.epub", t)
        })
        .unwrap();
        let path = drv
            .send_in(&device, &[], "Identity Check.kepub.epub", &expected)
            .unwrap();
        assert_eq!(fs::read(&path).unwrap(), expected);
    }

    #[test]
    fn the_full_sync_plan_reads_a_real_schema_kobo_and_a_sidecar_without_writing() {
        let root = tmp("fullplan");
        let dir = tmp("fullplan-lib");
        let src = tmp("fullplan-src");
        let book = epub(&src);
        let mut lib = bookx_store::open_library(&dir.join("lib")).unwrap().library;
        lib.add_path(&book, true).unwrap();
        let id = lib.books[0].id;
        lib.get_mut(id).unwrap().progress = 0.1;
        lib.get_mut(id)
            .unwrap()
            .highlights
            .push(crate::core::library::Highlight {
                id: "h".into(),
                spine: 0,
                start: 0,
                end: 5,
                color: "yellow".into(),
                style: String::new(),
                note: String::new(),
                text: "Words for the library only".into(),
                created: chrono::Utc::now(),
            });
        // a Kobo as the owner's: the book was read in KOReader, the firmware never imported it
        fs::create_dir_all(root.join("KOBOeReader/.kobo")).unwrap();
        fs::create_dir_all(root.join("KOBOeReader/Sent Book.sdr")).unwrap();
        fs::write(root.join("KOBOeReader/Sent Book.epub"), b"x").unwrap();
        fs::write(
            root.join("KOBOeReader/Sent Book.sdr/metadata.epub.lua"),
            "return {\n [\"annotations\"] = {\n  [1] = { [\"text\"] = \"Words for the device only\", [\"datetime\"] = \"2026-08-17 11:22:15\" },\n },\n [\"doc_props\"] = { [\"title\"] = \"Sent Book\" },\n [\"percent_finished\"] = 0.6,\n [\"summary\"] = { [\"modified\"] = \"2026-08-12\", [\"status\"] = \"reading\" },\n}\n",
        )
        .unwrap();
        bookx_serve::kobo::create_real_schema_database(
            &root.join("KOBOeReader/.kobo/KoboReader.sqlite"),
        )
        .unwrap();
        let drv = Drivers::mass_only(MassStorageDriver::with_roots(vec![root.clone()]));
        let dev = find(&drv, "KOBO");
        let books = drv.books(&dev).unwrap();
        let list = |r: &Path| -> Vec<String> {
            let mut v: Vec<String> = Vec::new();
            fn w(d: &Path, v: &mut Vec<String>) {
                for e in fs::read_dir(d).unwrap().flatten() {
                    let m = e.metadata().unwrap();
                    v.push(format!("{} {}", e.path().display(), m.len()));
                    if m.is_dir() {
                        w(&e.path(), v);
                    }
                }
            }
            w(r, &mut v);
            v.sort();
            v
        };
        let before = list(&root);
        let _ = &books;
        struct Anywhere;
        impl crate::device_sync::Locator for Anywhere {
            fn locate(&self, _: u64, _: &str) -> Option<(usize, i32, i32)> {
                Some((0, 0, 5))
            }
        }
        let readings = crate::device_sync::read_device(&drv, &dev).unwrap();
        let plan = crate::device_sync::plan(
            &readings,
            &crate::device_sync::library_books(&lib.books),
            &Anywhere,
        );
        assert_eq!(list(&root), before, "planning writes nothing to the device");
        let s = plan.summary();
        assert_eq!(s.progress.len(), 1);
        assert_eq!(s.highlights_to_library, 1);
        assert_eq!(
            s.highlights_to_device, 0,
            "the copy keeps its own notes file"
        );
        assert_eq!(s.held_back, 1);
        assert!(s.text().contains("Nothing is deleted"));
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(src);
    }

    #[test]
    fn koreader_sidecars_and_the_kobo_database_feed_the_sync() {
        let root = tmp("states");
        mounts(&root);
        let src = tmp("states-src");
        let drv = Drivers::mass_only(MassStorageDriver::with_roots(vec![root.clone()]));
        let dev = find(&drv, "Boox");
        let sent = send_book(&drv, &dev, &epub(&src), "Alpha").unwrap();
        let side = bookx_serve::reading::sidecar_file(&sent.path);
        fs::create_dir_all(side.parent().unwrap()).unwrap();
        fs::write(&side, "return { [\"percent_finished\"] = 0.75 }").unwrap();
        let books = drv.books(&dev).unwrap();
        let st = drv.states(&dev, &books);
        assert_eq!(st.len(), 1);
        assert_eq!(st[0].1.progress, 0.75);
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(src);
    }

    #[test]
    fn settings_survive_old_configuration_files() {
        let s: DeviceSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(s, DeviceSettings::default());
        assert!(!s.webdav.configured());
        let s: DeviceSettings =
            serde_json::from_str(r#"{"template":"{title}","webdav":{"url":"https://x/dav"}}"#)
                .unwrap();
        assert_eq!(s.template, "{title}");
        assert!(s.webdav.configured());
    }

    #[test]
    fn cloud_folders_are_devices_only_when_set_up() {
        let off = drivers(&DeviceSettings::default());
        assert!(off.detect().iter().all(|d| !d.id.starts_with("webdav:")));
        let on = drivers(&DeviceSettings {
            webdav: WebDavSettings {
                url: "https://cloud.example/dav/books".into(),
                user: "ann".into(),
                password: "pw".into(),
            },
            ..Default::default()
        });
        let cloud: Vec<_> = on
            .detect()
            .into_iter()
            .filter(|d| d.id.starts_with("webdav:"))
            .collect();
        assert_eq!(cloud.len(), 1);
        assert!(!on
            .detect_physical()
            .iter()
            .any(|d| d.id.starts_with("webdav:")));
        assert_eq!(on.backend(&cloud[0]), Backend::Cloud);
    }

    #[test]
    fn removing_several_books_reports_each_failure() {
        let root = tmp("rm-many");
        mounts(&root);
        let src = tmp("rm-many-src");
        let drv = Drivers::mass_only(MassStorageDriver::with_roots(vec![root.clone()]));
        let dev = find(&drv, "Boox");
        send_book(&drv, &dev, &epub(&src), "One").unwrap();
        send_book(&drv, &dev, &epub(&src), "Two").unwrap();
        let mut books = drv.books(&dev).unwrap();
        books.push(DeviceBook {
            path: dev.mount.join("Books/ghost.epub"),
            rel: "Books/ghost.epub".into(),
            size: 0,
            modified: None,
        });
        let (ok, errors) = remove_books(&drv, &dev, &books);
        assert_eq!(ok, 2);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("ghost.epub"));
        assert!(drv.books(&dev).unwrap().is_empty());
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(src);
    }
}
