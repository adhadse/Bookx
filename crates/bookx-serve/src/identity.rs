//! What a mounted drive *is*, not what its folders are called (issues #57 and #58).
//!
//! A mounted volume is described by the facts the kernel knows about the hardware behind it: the
//! USB vendor and product id, the USB manufacturer and product strings, the SCSI INQUIRY vendor and
//! model, whether the disk is removable, and the filesystem UUID and label. [`SysfsIdentity`] reads
//! them from `/sys/class/block` and `/dev/disk`; [`recognise`] matches them against the small table
//! of e-readers below. A folder layout never decides whether something is an e-reader: anyone can
//! create a folder called `Books` or `.kobo`. The USB device's sysfs path is what groups the volumes
//! of one device (the internal memory and the SD card of a Nook are two LUNs of one USB device).
//!
//! Serial numbers are read only to be hashed ([`UsbFacts::private_id`]); the number itself is never
//! stored, shown or logged. Nothing here is a security boundary: a forged device can claim any id.
//!
//! # Where the table comes from
//!
//! Facts only (vendor and product ids and strings are public identifiers); no code is copied. Each
//! entry needs two independent sources:
//!
//! | Entry | Source 1 | Source 2 |
//! |---|---|---|
//! | Kobo `2237` (any product) | calibre `devices/kobo/driver.py` `KOBO_VENDOR_ID` | usb.ids / systemd hwdb `2237 Kobo Inc.` |
//! | Kobo Touch `2237:4163`, Glo `2237:4173` | calibre `KoboTouch` product ids | usb.ids `4163 Touch`, `4173 Glo` |
//! | Kindle `1949:0002`, `1949:0004` | calibre `devices/kindle/driver.py` `KINDLE2` | usb.ids `Amazon Kindle`, `Amazon Kindle 3/4/Paperwhite` |
//! | Kindle `1949` with SCSI vendor `Kindle` (incl. `1949:0324`) | calibre `VENDOR_NAME = 'KINDLE'`, `PRODUCT_ID 0x0324` | dmesg reports `Direct-Access Kindle Internal Storage` for `1949:0324` and `1949:0004` (Ask Ubuntu 541084, FreeBSD forum 66637) |
//! | Nook `2080` (any product) | calibre `devices/nook/driver.py` | usb.ids `2080 Barnes & Noble`; dmesg `Direct-Access B&N Ebook Disk` with LUN 0 and LUN 1 (Arch forum 125998) |
//! | PocketBook `fffe:0001` with `Obreey` or `PocketBook` strings | calibre `POCKETBOOK626` | dmesg `fffe:0001 Manufacturer: Obreey`, `Direct-Access USB-FS PocketBook` (abclinuxu.cz PB632); the user manual says an SD card appears as a second disk |
//! | Tolino `1f85:6052`, `1f85:6053`, `4173:8000` | calibre `devices/eb600/driver.py` `TOLINO` | dmesg / lsusb reports (retinal.dehy.de Epos; Arch forum 288853 Shine 4; AaronDewes tolinoInfo) |
//! | Sony `054c:031e` (PRS-300/505) | calibre `devices/prs505/driver.py` | usb.ids `031e PRS-300/PRS-505 eBook reader` |
//! | Boox: USB manufacturer string `Onyx` | MobileRead report (`Manufacturer: Onyx`) | KOReader `DeviceInfo.kt` (`Build.MANUFACTURER` is `ONYX`), see `device_names.rs` |
//!
//! Not in the table because there is no second source (or no mass storage mode): reMarkable (no
//! mass storage), Netronix-vendor `1f85` products other than the Tolino pairs above, Sony `05c2`,
//! Kobo-vendor Tolino model names. They can still be added by the user ("Use as an e-reader").
//! None of this has been run against real hardware by the authors.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::devices::DeviceKind;

/// Largest file read from sysfs or a test fixture; the kernel's attributes are tiny.
const MAX_ATTR: u64 = 4096;

/// The USB device behind a volume.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct UsbFacts {
    pub vendor_id: u16,
    pub product_id: u16,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    /// Hash of the serial number: a private stable key, never the serial itself.
    pub private_id: Option<String>,
    /// The USB device's sysfs path (or a test group name): volumes with the same path belong to
    /// one physical device.
    pub path: String,
}

impl std::fmt::Debug for UsbFacts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "usb {:04x}:{:04x}", self.vendor_id, self.product_id)
    }
}

/// Everything known about the hardware behind one mounted volume.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Identity {
    pub usb: Option<UsbFacts>,
    /// SCSI INQUIRY vendor and model (what `lsscsi` shows).
    pub scsi_vendor: Option<String>,
    pub scsi_model: Option<String>,
    /// The disk's `removable` flag; `None` when it could not be read.
    pub removable: Option<bool>,
    /// SCSI logical unit number of the disk (the Nook's SD card is the second one).
    pub lun: Option<u32>,
    /// Partition number, 0 for a whole-disk filesystem.
    pub partition: u32,
    pub uuid: Option<String>,
    pub label: Option<String>,
    pub size_bytes: Option<u64>,
    /// Whether any identity information could be read at all (false in a sandbox without `/sys`).
    pub readable: bool,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // no uuid, label or serial: this may end up in a log
        write!(
            f,
            "Identity {{ usb: {:?}, removable: {:?}, readable: {} }}",
            self.usb, self.removable, self.readable
        )
    }
}

/// Reads the identity of a mounted volume.
pub trait IdentitySource: Send + Sync + std::fmt::Debug {
    /// `source` is the first field of the `/proc/mounts` line (`/dev/sdd1`), empty when unknown.
    fn identify(&self, mount_point: &Path, source: &str) -> Identity;
}

/// No identity at all: every drive is "not identified" (macOS and Windows, a sandbox without sysfs).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoIdentity;

impl IdentitySource for NoIdentity {
    fn identify(&self, _mount_point: &Path, _source: &str) -> Identity {
        Identity::default()
    }
}

fn read_attr(path: &Path) -> Option<String> {
    use std::io::Read;
    let f = std::fs::File::open(path).ok()?;
    let mut buf = String::new();
    f.take(MAX_ATTR).read_to_string(&mut buf).ok()?;
    let t = buf.trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn read_hex16(path: &Path) -> Option<u16> {
    u16::from_str_radix(read_attr(path)?.trim_start_matches("0x"), 16).ok()
}

fn hash_hex(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update([0]);
    }
    h.finalize()[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `\x20` escapes of `/dev/disk/by-label` names.
fn unescape_by_label(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1] == b'x' {
            if let Ok(v) = u8::from_str_radix(&s[i + 2..i + 4], 16) {
                out.push(v);
                i += 4;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Reads the identity from the kernel's sysfs and udev's `/dev/disk` links. Both roots can be
/// replaced so tests can build fake trees.
#[derive(Debug, Clone)]
pub struct SysfsIdentity {
    sys: PathBuf,
    dev: PathBuf,
}

impl Default for SysfsIdentity {
    fn default() -> Self {
        SysfsIdentity::with_roots(PathBuf::from("/sys"), PathBuf::from("/dev"))
    }
}

impl SysfsIdentity {
    pub fn with_roots(sys: PathBuf, dev: PathBuf) -> SysfsIdentity {
        SysfsIdentity { sys, dev }
    }

    fn by_link(&self, dir: &str, name: &str) -> Option<String> {
        let rd = std::fs::read_dir(self.dev.join("disk").join(dir)).ok()?;
        for e in rd.flatten() {
            let target = std::fs::read_link(e.path()).ok()?;
            if target.file_name().is_some_and(|n| n == name) {
                return Some(unescape_by_label(&e.file_name().to_string_lossy()));
            }
        }
        None
    }
}

fn is_scsi_address(name: &str) -> Option<u32> {
    let parts: Vec<&str> = name.split(':').collect();
    if parts.len() == 4 && parts.iter().all(|p| p.parse::<u32>().is_ok()) {
        parts[3].parse().ok()
    } else {
        None
    }
}

impl IdentitySource for SysfsIdentity {
    fn identify(&self, _mount_point: &Path, source: &str) -> Identity {
        let mut id = Identity::default();
        let Some(name) = source.strip_prefix("/dev/").filter(|n| !n.contains('/')) else {
            return id;
        };
        let Ok(block) = std::fs::canonicalize(self.sys.join("class/block").join(name)) else {
            return id;
        };
        id.readable = true;
        id.partition = read_attr(&block.join("partition"))
            .and_then(|p| p.parse().ok())
            .unwrap_or(0);
        id.size_bytes = read_attr(&block.join("size"))
            .and_then(|s| s.parse::<u64>().ok())
            .map(|s| s.saturating_mul(512));
        let disk = if id.partition > 0 {
            block
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or(block.clone())
        } else {
            block.clone()
        };
        id.removable = read_attr(&disk.join("removable")).map(|r| r == "1");
        let mut usb: Option<UsbFacts> = None;
        for dir in block.ancestors().skip(1) {
            let fname = dir.file_name().map(|n| n.to_string_lossy().into_owned());
            if let Some(n) = fname.as_deref() {
                if let Some(lun) = is_scsi_address(n) {
                    if id.lun.is_none() {
                        id.lun = Some(lun);
                        id.scsi_vendor = read_attr(&dir.join("vendor"));
                        id.scsi_model = read_attr(&dir.join("model"));
                    }
                }
            }
            if let Some(vendor_id) = read_hex16(&dir.join("idVendor")) {
                let rel = dir.strip_prefix(&self.sys).unwrap_or(dir);
                usb = Some(UsbFacts {
                    vendor_id,
                    product_id: read_hex16(&dir.join("idProduct")).unwrap_or(0),
                    manufacturer: read_attr(&dir.join("manufacturer")),
                    product: read_attr(&dir.join("product")),
                    private_id: read_attr(&dir.join("serial")).map(|s| hash_hex(&["serial", &s])),
                    path: rel.display().to_string(),
                });
                break;
            }
        }
        id.usb = usb;
        id.uuid = self.by_link("by-uuid", name);
        id.label = self.by_link("by-label", name);
        id
    }
}

/// The JSON a test puts in `<mount>/.bookx-test-identity` (only read by [`TestIdentity`]).
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TestFile {
    vendor_id: Option<String>,
    product_id: Option<String>,
    manufacturer: Option<String>,
    product: Option<String>,
    serial: Option<String>,
    scsi_vendor: Option<String>,
    scsi_model: Option<String>,
    /// Names the physical device: volumes with the same group are one device.
    group: Option<String>,
    lun: Option<u32>,
    partition: Option<u32>,
    removable: Option<bool>,
    uuid: Option<String>,
    label: Option<String>,
    size: Option<u64>,
    /// Pretends sysfs could not be read (a sandbox).
    unreadable: bool,
}

/// Test-only identity from a JSON file inside the mount, for fake mount trees (`BOOKX_DEVICE_ROOTS`
/// and [`crate::devices::MassStorageDriver::with_roots`]). The driver built for the real
/// `/proc/mounts` never uses it, so a USB stick cannot claim to be a Kobo by carrying this file.
#[derive(Debug, Clone, Copy, Default)]
pub struct TestIdentity;

/// The file name [`TestIdentity`] reads.
pub const TEST_IDENTITY_FILE: &str = ".bookx-test-identity";

impl IdentitySource for TestIdentity {
    fn identify(&self, mount_point: &Path, _source: &str) -> Identity {
        let mut id = Identity {
            readable: true,
            removable: Some(true),
            ..Default::default()
        };
        let Some(text) = read_attr(&mount_point.join(TEST_IDENTITY_FILE)) else {
            return id;
        };
        let Ok(t) = serde_json::from_str::<TestFile>(&text) else {
            return id;
        };
        if t.unreadable {
            return Identity::default();
        }
        let hex = |s: &Option<String>| {
            s.as_deref()
                .and_then(|v| u16::from_str_radix(v.trim_start_matches("0x"), 16).ok())
        };
        if let Some(vendor_id) = hex(&t.vendor_id) {
            id.usb = Some(UsbFacts {
                vendor_id,
                product_id: hex(&t.product_id).unwrap_or(0),
                manufacturer: t.manufacturer.clone(),
                product: t.product.clone(),
                private_id: t.serial.as_deref().map(|s| hash_hex(&["serial", s])),
                path: t
                    .group
                    .clone()
                    .unwrap_or_else(|| format!("test/{}", mount_point.display())),
            });
        }
        id.scsi_vendor = t.scsi_vendor;
        id.scsi_model = t.scsi_model;
        id.lun = t.lun;
        id.partition = t.partition.unwrap_or(1);
        id.removable = t.removable.or(id.removable);
        id.uuid = t.uuid;
        id.label = t.label;
        id.size_bytes = t.size;
        id
    }
}

/// How a drive was recognised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recognised {
    pub kind: DeviceKind,
    /// "Kobo", "Nook", "Boox Leaf 2"…
    pub model: String,
    /// The facts the decision rests on, for people ("USB 2237:4163").
    pub how: String,
}

struct Known {
    vendor: Option<u16>,
    /// Empty: any product of the vendor.
    products: &'static [u16],
    /// Lower-case words; one must be contained in the USB manufacturer string.
    manufacturer: &'static [&'static str],
    /// Lower-case words; one must be contained in a USB string or the SCSI vendor or model.
    text: &'static [&'static str],
    /// Lower-case SCSI vendor that must be present (trimmed, case-insensitive).
    scsi_vendor: Option<&'static str>,
    kind: DeviceKind,
    model: &'static str,
}

const K: Known = Known {
    vendor: None,
    products: &[],
    manufacturer: &[],
    text: &[],
    scsi_vendor: None,
    kind: DeviceKind::Generic,
    model: "",
};

const TABLE: &[Known] = &[
    Known {
        vendor: Some(0x2237),
        kind: DeviceKind::Kobo,
        model: "Kobo",
        ..K
    },
    Known {
        vendor: Some(0x1949),
        products: &[0x0002, 0x0004],
        kind: DeviceKind::Kindle,
        model: "Kindle",
        ..K
    },
    Known {
        vendor: Some(0x1949),
        scsi_vendor: Some("kindle"),
        kind: DeviceKind::Kindle,
        model: "Kindle",
        ..K
    },
    Known {
        vendor: Some(0x2080),
        kind: DeviceKind::Generic,
        model: "Nook",
        ..K
    },
    Known {
        vendor: Some(0xfffe),
        products: &[0x0001],
        text: &["obreey", "pocketbook"],
        kind: DeviceKind::PocketBook,
        model: "PocketBook",
        ..K
    },
    Known {
        vendor: Some(0x1f85),
        products: &[0x6052, 0x6053],
        kind: DeviceKind::Generic,
        model: "Tolino",
        ..K
    },
    Known {
        vendor: Some(0x4173),
        products: &[0x8000],
        kind: DeviceKind::Generic,
        model: "Tolino",
        ..K
    },
    Known {
        vendor: Some(0x054c),
        products: &[0x031e],
        kind: DeviceKind::Generic,
        model: "Sony Reader",
        ..K
    },
    Known {
        manufacturer: &["onyx"],
        kind: DeviceKind::Generic,
        model: "Boox",
        ..K
    },
];

/// Model names of products inside a vendor entry (each seen in two sources, see the header).
fn product_name(vendor: u16, product: u16) -> Option<&'static str> {
    match (vendor, product) {
        (0x2237, 0x4163) => Some("Kobo Touch"),
        (0x2237, 0x4173) => Some("Kobo Glo"),
        _ => None,
    }
}

fn lower(s: &Option<String>) -> String {
    s.as_deref().unwrap_or("").trim().to_ascii_lowercase()
}

/// Matches the identity against the table of known e-readers. `None` means: not an e-reader we can
/// tell from its hardware (a flash drive, an SD card in a reader, or a sandbox that hides sysfs).
pub fn recognise(id: &Identity) -> Option<Recognised> {
    let usb = id.usb.as_ref()?;
    let manufacturer = lower(&usb.manufacturer);
    let strings = [
        manufacturer.clone(),
        lower(&usb.product),
        lower(&id.scsi_vendor),
        lower(&id.scsi_model),
    ];
    for k in TABLE {
        if k.vendor.is_some_and(|v| v != usb.vendor_id) {
            continue;
        }
        if !k.products.is_empty() && !k.products.contains(&usb.product_id) {
            continue;
        }
        if !k.manufacturer.is_empty() && !k.manufacturer.iter().any(|w| manufacturer.contains(w)) {
            continue;
        }
        if !k.text.is_empty() && !k.text.iter().any(|w| strings.iter().any(|s| s.contains(w))) {
            continue;
        }
        if k.scsi_vendor.is_some_and(|v| lower(&id.scsi_vendor) != v) {
            continue;
        }
        let mut model = product_name(usb.vendor_id, usb.product_id)
            .unwrap_or(k.model)
            .to_string();
        if k.model == "Boox" {
            if let Some(named) = usb
                .product
                .as_deref()
                .and_then(|p| crate::device_names::known_name(&format!("Onyx {p}")))
            {
                model = named;
            }
        }
        let how = if k.vendor.is_some() {
            format!("USB {:04x}:{:04x}", usb.vendor_id, usb.product_id)
        } else {
            format!(
                "USB {:04x}:{:04x}, manufacturer string",
                usb.vendor_id, usb.product_id
            )
        };
        return Some(Recognised {
            kind: k.kind,
            model,
            how,
        });
    }
    None
}

/// Size in GiB rounded up to a power of two (8 GB cards are 7.4 GiB): stable across formatting.
pub fn size_class(bytes: u64) -> u64 {
    let gib = bytes.div_ceil(1 << 30).max(1);
    gib.next_power_of_two()
}

/// A stable key for a drive, remembered in the settings: a hash of the filesystem UUID, else of the
/// label and the size class. `None` when neither exists. Never contains a serial number, and the
/// hash keeps the label and UUID out of the settings file and out of logs.
pub fn stable_key(id: &Identity) -> Option<String> {
    if let Some(u) = id.uuid.as_deref().filter(|u| !u.is_empty()) {
        return Some(format!(
            "drv1-{}",
            hash_hex(&["uuid", &u.to_ascii_lowercase()])
        ));
    }
    let label = id.label.as_deref().filter(|l| !l.is_empty())?;
    let class = size_class(id.size_bytes?);
    Some(format!(
        "drv1-{}",
        hash_hex(&["label", label, &class.to_string()])
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usb(v: u16, p: u16, m: &str, prod: &str) -> UsbFacts {
        UsbFacts {
            vendor_id: v,
            product_id: p,
            manufacturer: Some(m.into()),
            product: Some(prod.into()),
            private_id: None,
            path: "x".into(),
        }
    }

    fn id(v: u16, p: u16, m: &str, prod: &str, sv: &str, sm: &str) -> Identity {
        Identity {
            usb: Some(usb(v, p, m, prod)),
            scsi_vendor: Some(sv.into()),
            scsi_model: Some(sm.into()),
            readable: true,
            ..Default::default()
        }
    }

    fn kind(i: &Identity) -> Option<(DeviceKind, String)> {
        recognise(i).map(|r| (r.kind, r.model))
    }

    #[test]
    fn the_table_recognises_each_family() {
        assert_eq!(
            kind(&id(
                0x2237,
                0x4163,
                "Kobo Inc.",
                "eReader",
                "KOBO",
                "eReader"
            )),
            Some((DeviceKind::Kobo, "Kobo Touch".into()))
        );
        assert_eq!(
            kind(&id(
                0x2237,
                0x4237,
                "Rakuten Kobo Inc.",
                "Libra Colour",
                "",
                ""
            )),
            Some((DeviceKind::Kobo, "Kobo".into()))
        );
        assert_eq!(
            kind(&id(
                0x1949,
                0x0324,
                "",
                "Internal Storage",
                "Kindle",
                "Internal Storage"
            )),
            Some((DeviceKind::Kindle, "Kindle".into()))
        );
        assert_eq!(
            kind(&id(0x1949, 0x0004, "Amazon", "Amazon Kindle", "", "")),
            Some((DeviceKind::Kindle, "Kindle".into()))
        );
        assert_eq!(
            kind(&id(0x2080, 0x0003, "", "NOOK", "B&N", "Ebook Disk")),
            Some((DeviceKind::Generic, "Nook".into()))
        );
        assert_eq!(
            kind(&id(
                0xfffe,
                0x0001,
                "Obreey",
                "PB632",
                "USB-FS",
                "PocketBook"
            )),
            Some((DeviceKind::PocketBook, "PocketBook".into()))
        );
        assert_eq!(
            kind(&id(0x1f85, 0x6053, "Rakuten Kobo Inc.", "tolino", "", "")),
            Some((DeviceKind::Generic, "Tolino".into()))
        );
        assert_eq!(
            kind(&id(0x054c, 0x031e, "Sony", "PRS-505", "Sony", "PRS-505")),
            Some((DeviceKind::Generic, "Sony Reader".into()))
        );
        assert_eq!(
            kind(&id(0x2207, 0x0001, "Onyx", "Note", "", "")),
            Some((DeviceKind::Generic, "Boox Note".into()))
        );
    }

    #[test]
    fn the_boox_model_comes_from_the_shared_name_resolver() {
        let r = recognise(&id(0x2207, 0x0001, "ONYX", "Scubapilot-IDP", "", "")).unwrap();
        assert_eq!(r.model, "Boox Leaf 2");
    }

    #[test]
    fn things_that_are_not_e_readers_are_not_recognised() {
        // a flash drive
        assert_eq!(
            recognise(&id(0x0781, 0x5581, "SanDisk", "Ultra", "SanDisk", "Ultra")),
            None
        );
        // an SD card in a card reader
        assert_eq!(
            recognise(&id(
                0x058f,
                0x6366,
                "Generic",
                "Mass Storage",
                "Generic",
                "STORAGE DEVICE"
            )),
            None
        );
        // Kindle Fire is a tablet, other Lab126 products are not readers
        assert_eq!(
            recognise(&id(
                0x1949,
                0x0006,
                "Amazon",
                "Kindle Fire",
                "AMAZON",
                "Kindle Fire"
            )),
            None
        );
        assert_eq!(
            recognise(&id(0x1949, 0x0417, "Yubico", "Zukey", "", "")),
            None
        );
        // Sony cameras, other PocketBook-looking vendor ids
        assert_eq!(
            recognise(&id(0x054c, 0x0854, "Sony", "DSC", "Sony", "DSC")),
            None
        );
        assert_eq!(
            recognise(&id(0xfffe, 0x0001, "Acme", "Thing", "ACME", "Stick")),
            None
        );
        // no USB facts at all
        assert_eq!(recognise(&Identity::default()), None);
    }

    #[test]
    fn a_volume_label_never_makes_a_device() {
        let mut i = id(0x0781, 0x5581, "SanDisk", "Ultra", "SanDisk", "Ultra");
        i.label = Some("KOBOeReader".into());
        i.uuid = Some("1234-ABCD".into());
        assert_eq!(recognise(&i), None);
    }

    #[test]
    fn size_classes_absorb_formatting_overhead() {
        assert_eq!(size_class(7_900_000_000), 8);
        assert_eq!(size_class(8_000_000_000), 8);
        assert_eq!(size_class(220_000_000), 1);
        assert_eq!(size_class(31_900_000_000), 32);
    }

    #[test]
    fn the_stable_key_uses_uuid_then_label_and_size_and_hides_both() {
        let mut i = Identity {
            uuid: Some("ABCD-1234".into()),
            label: Some("MYSTICK".into()),
            size_bytes: Some(8_000_000_000),
            ..Default::default()
        };
        let k = stable_key(&i).unwrap();
        assert!(k.starts_with("drv1-") && !k.contains("ABCD") && !k.contains("MYSTICK"));
        // same uuid, other label: same key
        i.label = Some("RENAMED".into());
        assert_eq!(stable_key(&i), Some(k));
        // no uuid: label and size class
        i.uuid = None;
        let a = stable_key(&i).unwrap();
        i.size_bytes = Some(7_900_000_000);
        assert_eq!(stable_key(&i), Some(a.clone()));
        i.size_bytes = Some(16_000_000_000);
        assert_ne!(stable_key(&i), Some(a));
        i.label = None;
        assert_eq!(stable_key(&i), None);
    }

    #[test]
    fn debug_output_hides_serial_uuid_and_label() {
        let mut i = id(0x2237, 0x4163, "Kobo", "eReader", "KOBO", "eReader");
        i.uuid = Some("SECRET-UUID".into());
        i.label = Some("SECRET-LABEL".into());
        i.usb.as_mut().unwrap().private_id = Some("deadbeef".into());
        let s = format!("{i:?}");
        assert!(!s.contains("SECRET") && !s.contains("deadbeef"), "{s}");
    }

    struct Fake(PathBuf);
    impl Fake {
        fn new() -> Fake {
            let p = std::env::temp_dir().join(format!("bookx-id-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&p).unwrap();
            Fake(p)
        }
        fn write(&self, rel: &str, text: &str) {
            let p = self.0.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        /// A USB device `usbpath` with one SCSI LUN and one partition `part` of disk `disk`.
        fn volume(
            &self,
            usbpath: &str,
            vid: &str,
            pid: &str,
            strings: (&str, &str),
            lun: u32,
            disk: &str,
            part: &str,
        ) {
            let usb = format!("sys/devices/pci0/{usbpath}");
            self.write(&format!("{usb}/idVendor"), vid);
            self.write(&format!("{usb}/idProduct"), pid);
            self.write(&format!("{usb}/manufacturer"), strings.0);
            self.write(&format!("{usb}/product"), strings.1);
            self.write(&format!("{usb}/serial"), "SERIAL-0001");
            let scsi = format!("{usb}/1-1:1.0/host9/target9:0:0/9:0:0:{lun}");
            self.write(&format!("{scsi}/vendor"), "B&N     ");
            self.write(&format!("{scsi}/model"), "Ebook Disk      ");
            let disk_dir = format!("{scsi}/block/{disk}");
            self.write(&format!("{disk_dir}/removable"), "1");
            self.write(&format!("{disk_dir}/{part}/partition"), "1");
            self.write(&format!("{disk_dir}/{part}/size"), "15625000");
            std::fs::create_dir_all(self.0.join("sys/class/block")).unwrap();
            std::os::unix::fs::symlink(
                self.0.join(&disk_dir).join(part),
                self.0.join("sys/class/block").join(part),
            )
            .unwrap();
        }
        fn link(&self, kind: &str, name: &str, to: &str) {
            let d = self.0.join("dev/disk").join(kind);
            std::fs::create_dir_all(&d).unwrap();
            std::os::unix::fs::symlink(format!("../../{to}"), d.join(name)).unwrap();
        }
        fn source(&self) -> SysfsIdentity {
            SysfsIdentity::with_roots(self.0.join("sys"), self.0.join("dev"))
        }
    }
    impl Drop for Fake {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    #[test]
    fn sysfs_identity_reads_usb_scsi_and_filesystem_facts() {
        let f = Fake::new();
        f.volume(
            "usb1/1-4",
            "2080\n",
            "0003\n",
            ("Barnes & Noble", "NOOK"),
            1,
            "sdf",
            "sdf1",
        );
        f.link("by-uuid", "AAAA-0001", "sdf1");
        f.link("by-label", "SD\\x20CARD", "sdf1");
        let i = f
            .source()
            .identify(Path::new("/run/media/u/SDCARD"), "/dev/sdf1");
        let u = i.usb.as_ref().unwrap();
        assert_eq!((u.vendor_id, u.product_id), (0x2080, 3));
        assert_eq!(u.manufacturer.as_deref(), Some("Barnes & Noble"));
        assert_eq!(u.path, "devices/pci0/usb1/1-4");
        assert!(
            u.private_id
                .as_deref()
                .is_some_and(|p| !p.contains("SERIAL"))
        );
        assert_eq!(i.scsi_vendor.as_deref(), Some("B&N"));
        assert_eq!(i.scsi_model.as_deref(), Some("Ebook Disk"));
        assert_eq!((i.lun, i.partition), (Some(1), 1));
        assert_eq!(i.removable, Some(true));
        assert_eq!(i.size_bytes, Some(8_000_000_000));
        assert_eq!(i.uuid.as_deref(), Some("AAAA-0001"));
        assert_eq!(i.label.as_deref(), Some("SD CARD"));
        assert!(i.readable);
        assert_eq!(recognise(&i).unwrap().model, "Nook");
    }

    #[test]
    fn sysfs_identity_without_sysfs_is_unreadable_not_guessed() {
        let f = Fake::new();
        let i = f
            .source()
            .identify(Path::new("/run/media/u/X"), "/dev/sdz1");
        assert!(!i.readable && i.usb.is_none() && recognise(&i).is_none());
        // a network or loop source is not a block device in /sys/class/block
        let j = f.source().identify(Path::new("/mnt/x"), "server:/share");
        assert!(!j.readable);
    }

    #[test]
    fn two_volumes_of_one_usb_device_share_a_path_and_two_devices_do_not() {
        let f = Fake::new();
        f.volume(
            "usb1/1-4",
            "2080",
            "0003",
            ("B&N", "NOOK"),
            0,
            "sdd",
            "sdd1",
        );
        f.volume(
            "usb1/1-4",
            "2080",
            "0003",
            ("B&N", "NOOK"),
            1,
            "sde",
            "sde1",
        );
        f.volume(
            "usb1/1-5",
            "2080",
            "0003",
            ("B&N", "NOOK"),
            0,
            "sdf",
            "sdf1",
        );
        let s = f.source();
        let m = Path::new("/mnt");
        let (a, b, c) = (
            s.identify(m, "/dev/sdd1"),
            s.identify(m, "/dev/sde1"),
            s.identify(m, "/dev/sdf1"),
        );
        assert_eq!(a.usb.as_ref().unwrap().path, b.usb.as_ref().unwrap().path);
        assert_ne!(a.usb.as_ref().unwrap().path, c.usb.as_ref().unwrap().path);
        assert_eq!((a.lun, b.lun), (Some(0), Some(1)));
    }

    #[test]
    fn the_test_identity_reads_its_file_and_ignores_garbage() {
        let f = Fake::new();
        let m = f.0.join("m");
        std::fs::create_dir_all(&m).unwrap();
        // no file: a removable drive nobody knows
        let i = TestIdentity.identify(&m, "");
        assert!(i.usb.is_none() && i.removable == Some(true));
        f.write(
            "m/.bookx-test-identity",
            r#"{"vendor_id":"2237","product_id":"4163","group":"g1","label":"KOBOeReader"}"#,
        );
        let i = TestIdentity.identify(&m, "");
        assert_eq!(recognise(&i).unwrap().kind, DeviceKind::Kobo);
        assert_eq!(i.usb.unwrap().path, "g1");
        f.write("m/.bookx-test-identity", "not json");
        assert!(TestIdentity.identify(&m, "").usb.is_none());
    }
}
