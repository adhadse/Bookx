//! The device zoo: popular e-readers as test fixtures. **Everything here is emulated**, nothing
//! was observed on a real device except where a field says so.
//!
//! * [`ZOO`] lists readers with what KOReader reports as `Device.model` (taken from KOReader's
//!   own device files, or from a user report where `source` says so), the name Bookx must show,
//!   and how the reader appears on a computer (USB disk layout, MTP, or not at all).
//! * [`build_usb`] and [`build_mtp`] make the folder tree a driver sees.
//! * [`Peer`] speaks the Bookx / KOSync HTTP protocol the way the KOReader plugin does on that
//!   reader (the same headers and body fields as `koreader-plugin/bookx.koplugin`), or the way
//!   stock KOReader's *Progress sync* does (no Bookx headers).
//!
//! To add a reader: one entry in [`ZOO`] (see `docs/hardware-test-plan.md`).
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use bookx_serve::activity::ActivityLog;
use bookx_serve::annotation_sync::AnnotationSync;
use bookx_serve::httpc::request;
use bookx_serve::kosync::{KoSync, MemoryStore, md5_hex};
use bookx_serve::server::{LibrarySnapshot, Server, ServerConfig, ServerHandle};
use serde_json::{Value, json};
use std::sync::Arc;

/// The brand of a reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Boox,
    Kobo,
    Kindle,
    PocketBook,
    Remarkable,
    /// A phone or tablet, or anything Bookx has no name for.
    Other,
}

/// How a reader shows up when plugged into a computer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Usb {
    /// A Kobo: `.kobo/KoboReader.sqlite` and `.kobo/version`. `product_id` is the last field of
    /// `version`; `id_seen_on_hardware` is true only where a real device was read.
    Kobo {
        product_id: &'static str,
        id_seen_on_hardware: bool,
    },
    /// `documents/` and `system/` (with `system/version.txt`).
    Kindle,
    /// `system/config` and the book folder `Books`.
    PocketBook,
    /// Plain disk with a `Books` folder.
    Disk,
    /// An Android e-reader: not a disk, it is an MTP device (gvfs folder `mtp:host=<host>`).
    Mtp { host: &'static str },
    /// Does not mount at all (only Wi-Fi).
    None,
}

#[derive(Debug, Clone, Copy)]
pub struct Reader {
    pub slug: &'static str,
    pub family: Family,
    /// The name on the box, which Bookx must show.
    pub marketing: &'static str,
    /// What KOReader's `Device.model` is on that reader (what the plugin sends as `device`).
    pub reported: &'static str,
    pub usb: Usb,
    /// Where `reported` comes from.
    pub source: &'static str,
    /// Observed on a real device by the project: always false for the zoo's own data.
    pub seen_on_hardware: bool,
}

const KO_KOBO: &str = "koreader frontend/device/kobo/device.lua";
const KO_KINDLE: &str = "koreader frontend/device/kindle/device.lua";
const KO_PB: &str = "koreader frontend/device/pocketbook/device.lua";
const KO_ANDROID: &str = "koreader android-luajit-launcher DeviceInfo.kt (Build.PRODUCT)";

const fn r(
    slug: &'static str,
    family: Family,
    marketing: &'static str,
    reported: &'static str,
    usb: Usb,
    source: &'static str,
) -> Reader {
    Reader {
        slug,
        family,
        marketing,
        reported,
        usb,
        source,
        seen_on_hardware: false,
    }
}

const fn kobo(id: &'static str) -> Usb {
    Usb::Kobo {
        product_id: id,
        id_seen_on_hardware: false,
    }
}

/// The ids of these Kobos are made up (the project has not read the `version` file of such a
/// device), so no test may claim a model from them.
const UNVERIFIED_ID: &str = "00000000-0000-0000-0000-000000000999";

pub const ZOO: &[Reader] = &[
    // Boox: Android, MTP. The first one is the report of issue #39.
    r(
        "boox-leaf2-issue-39",
        Family::Boox,
        "Boox Leaf 2",
        "Onyx Scubapilot-IDP",
        Usb::Mtp {
            host: "ONYX_Leaf2_0123456789",
        },
        "Bookx issue #39 (a Boox Leaf 2 owner, KOReader through Bookx 0.3.0-beta.1)",
    ),
    r(
        "boox-leaf2-product",
        Family::Boox,
        "Boox Leaf 2",
        "leaf2",
        Usb::Mtp {
            host: "ONYX_Leaf2_0123456789",
        },
        KO_ANDROID,
    ),
    r(
        "boox-go7",
        Family::Boox,
        "Boox Go 7",
        "go7",
        Usb::Mtp {
            host: "ONYX_Go7_0123456789",
        },
        KO_ANDROID,
    ),
    r(
        "boox-palma2",
        Family::Boox,
        "Boox Palma 2",
        "palma2",
        Usb::Mtp {
            host: "ONYX_Palma2_0123456789",
        },
        KO_ANDROID,
    ),
    r(
        "boox-note-air-3c",
        Family::Boox,
        "Boox Note Air 3C",
        "noteair3c",
        Usb::Mtp {
            host: "ONYX_NoteAir3C_0123456789",
        },
        KO_ANDROID,
    ),
    r(
        "boox-tab-ultra-c",
        Family::Boox,
        "Boox Tab Ultra C",
        "tabultrac",
        Usb::Mtp {
            host: "ONYX_TabUltraC_0123456789",
        },
        KO_ANDROID,
    ),
    // Kobo: USB disk. Only the Libra Colour id was read from a real device.
    r(
        "kobo-clara-2e",
        Family::Kobo,
        "Kobo Clara 2E",
        "Kobo_goldfinch",
        kobo(UNVERIFIED_ID),
        KO_KOBO,
    ),
    r(
        "kobo-libra-2",
        Family::Kobo,
        "Kobo Libra 2",
        "Kobo_io",
        kobo(UNVERIFIED_ID),
        KO_KOBO,
    ),
    Reader {
        usb: Usb::Kobo {
            product_id: "00000000-0000-0000-0000-000000000390",
            id_seen_on_hardware: true,
        },
        ..r(
            "kobo-libra-colour",
            Family::Kobo,
            "Kobo Libra Colour",
            "Kobo_monza",
            Usb::None,
            KO_KOBO,
        )
    },
    r(
        "kobo-sage",
        Family::Kobo,
        "Kobo Sage",
        "Kobo_cadmus",
        kobo(UNVERIFIED_ID),
        KO_KOBO,
    ),
    r(
        "kobo-elipsa",
        Family::Kobo,
        "Kobo Elipsa",
        "Kobo_europa",
        kobo(UNVERIFIED_ID),
        KO_KOBO,
    ),
    r(
        "kobo-clara-bw",
        Family::Kobo,
        "Kobo Clara BW",
        "Kobo_spaBW",
        kobo(UNVERIFIED_ID),
        KO_KOBO,
    ),
    // Kindle: USB disk.
    r(
        "kindle-paperwhite-5",
        Family::Kindle,
        "Kindle Paperwhite 5",
        "KindlePaperWhite5",
        Usb::Kindle,
        KO_KINDLE,
    ),
    r(
        "kindle-oasis-3",
        Family::Kindle,
        "Kindle Oasis 3",
        "KindleOasis3",
        Usb::Kindle,
        KO_KINDLE,
    ),
    r(
        "kindle-scribe",
        Family::Kindle,
        "Kindle Scribe",
        "KindleScribe",
        Usb::Kindle,
        KO_KINDLE,
    ),
    // PocketBook: USB disk.
    r(
        "pocketbook-verse",
        Family::PocketBook,
        "PocketBook Verse",
        "PB629",
        Usb::PocketBook,
        KO_PB,
    ),
    r(
        "pocketbook-inkpad-3",
        Family::PocketBook,
        "PocketBook InkPad 3",
        "PBInkPad3",
        Usb::PocketBook,
        KO_PB,
    ),
    r(
        "pocketbook-era",
        Family::PocketBook,
        "PocketBook Era",
        "PB700",
        Usb::PocketBook,
        KO_PB,
    ),
    // reMarkable: no disk.
    r(
        "remarkable-2",
        Family::Remarkable,
        "reMarkable 2",
        "reMarkable 2.0",
        Usb::None,
        "koreader frontend/device/remarkable/device.lua",
    ),
    // Not in any table: shown as reported, never guessed.
    r(
        "android-phone-unknown",
        Family::Other,
        "panther",
        "panther",
        Usb::Mtp {
            host: "Pixel_7_ABC123",
        },
        "an Android phone's Build.PRODUCT (not in KOReader's e-reader table)",
    ),
    r(
        "kobo-unnamed-model",
        Family::Other,
        "Kobo_neverheardof",
        "Kobo_neverheardof",
        Usb::None,
        "a Kobo codename the table does not have",
    ),
];

pub fn by_slug(slug: &str) -> &'static Reader {
    ZOO.iter().find(|r| r.slug == slug).expect("a zoo reader")
}

/// A `.kobo/version` line: serial, kernel, firmware, kernel, kernel, product id.
pub fn kobo_version_line(product_id: &str) -> String {
    format!("N000000000000,4.19.14123,4.46.23852,4.19.14123,4.19.14123,{product_id}\n")
}

/// Builds the folder tree the reader shows as a USB disk, under `root/<name>`. Returns the mount
/// directory. Panics for a reader that is not a USB disk.
pub fn build_usb(root: &Path, name: &str, usb: Usb) -> PathBuf {
    let m = root.join(name);
    match usb {
        Usb::Kobo { product_id, .. } => {
            std::fs::create_dir_all(m.join(".kobo")).unwrap();
            bookx_serve::kobo::create_real_schema_database(&m.join(".kobo/KoboReader.sqlite"))
                .unwrap();
            std::fs::write(m.join(".kobo/version"), kobo_version_line(product_id)).unwrap();
        }
        Usb::Kindle => {
            std::fs::create_dir_all(m.join("documents")).unwrap();
            std::fs::create_dir_all(m.join("system")).unwrap();
            std::fs::write(m.join("system/version.txt"), "Kindle 5.16.2.1.1\n").unwrap();
        }
        Usb::PocketBook => {
            std::fs::create_dir_all(m.join("system/config")).unwrap();
            std::fs::create_dir_all(m.join("Books")).unwrap();
        }
        Usb::Disk => std::fs::create_dir_all(m.join("Books")).unwrap(),
        Usb::Mtp { .. } | Usb::None => panic!("{usb:?} is not a USB disk"),
    }
    m
}

/// Builds the gvfs tree of an MTP device under `gvfs`: `mtp:host=<host>/Internal shared storage`
/// with the folders an Android e-reader has. Returns the storage directory.
pub fn build_mtp(gvfs: &Path, host: &str) -> PathBuf {
    let storage = gvfs
        .join(format!("mtp:host={host}"))
        .join("Internal shared storage");
    for d in ["Android", "Download", "DCIM", "Books"] {
        std::fs::create_dir_all(storage.join(d)).unwrap();
    }
    storage
}

/// A running Bookx sync server with its activity log, and the account every [`Peer`] uses.
pub struct Bench {
    pub handle: ServerHandle,
    pub log: Arc<ActivityLog>,
    pub ann: Arc<AnnotationSync>,
    pub user: &'static str,
    pub key: String,
}

impl Bench {
    pub fn start() -> Bench {
        Bench::start_with(Arc::new(LibrarySnapshot::default))
    }

    /// A server that serves `provider`'s library (tests of what a device can do to it).
    pub fn start_with(provider: bookx_serve::server::LibraryProvider) -> Bench {
        let mut cfg = ServerConfig::new(provider);
        let log = Arc::new(ActivityLog::memory());
        cfg.kosync = Some(Arc::new(
            KoSync::new(Box::new(MemoryStore::new()), true).with_activity(log.clone()),
        ));
        let ann = Arc::new(AnnotationSync::memory().with_activity(log.clone()));
        cfg.annotations = Some(ann.clone());
        let handle = Server::start(cfg).expect("server starts");
        let b = Bench {
            handle,
            log,
            ann,
            user: "zoo",
            key: md5_hex("zoo-password"),
        };
        let r = request(
            "POST",
            &format!("{}/bookx/users/create", b.base()).parse().unwrap(),
            &[("Content-Type", "application/json")],
            json!({"username": b.user, "password": b.key})
                .to_string()
                .as_bytes(),
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(r.status, 201, "the account is created");
        b
    }

    pub fn base(&self) -> String {
        self.handle.base_url()
    }
}

/// What the plugin puts in a header: ASCII only.
fn ascii(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii() && !c.is_control() {
                c
            } else {
                '?'
            }
        })
        .collect()
}

/// A KOReader on one reader, talking to the server.
pub struct Peer<'a> {
    pub bench: &'a Bench,
    pub reader: &'static Reader,
    /// A name typed in the plugin's *Device name* field, else KOReader's `Device.model`.
    pub device: String,
    pub device_id: String,
    /// `false` is stock KOReader (*Progress sync* only): no Bookx headers, no annotations.
    pub plugin: bool,
}

impl<'a> Peer<'a> {
    /// The Bookx plugin on `reader`, which sends `Device.model`.
    pub fn plugin(bench: &'a Bench, reader: &'static Reader) -> Peer<'a> {
        Peer {
            bench,
            reader,
            device: reader.reported.to_string(),
            device_id: format!("{:016x}", fnv(reader.slug)),
            plugin: true,
        }
    }

    /// Stock KOReader with *Progress sync* set to Bookx's address.
    pub fn stock(bench: &'a Bench, reader: &'static Reader) -> Peer<'a> {
        Peer {
            plugin: false,
            ..Peer::plugin(bench, reader)
        }
    }

    fn headers(&self) -> Vec<(String, String)> {
        let mut h = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("x-auth-user".to_string(), self.bench.user.to_string()),
            ("x-auth-key".to_string(), self.bench.key.clone()),
        ];
        if self.plugin {
            h.push(("X-Bookx-Device".into(), ascii(&self.device)));
            h.push(("X-Bookx-Device-Id".into(), self.device_id.clone()));
            h.push((
                "X-Bookx-Client".into(),
                "KOReader/2026.07.2 bookx-plugin/0.3.1".into(),
            ));
            h.push(("X-Bookx-Protocol".into(), "1".into()));
        }
        h
    }

    fn call(&self, method: &str, path: &str, body: &Value) -> (u16, Value) {
        let owned = self.headers();
        let refs: Vec<(&str, &str)> = owned
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let r = request(
            method,
            &format!("{}/bookx{path}", self.bench.base())
                .parse()
                .unwrap(),
            &refs,
            body.to_string().as_bytes(),
            Duration::from_secs(10),
        )
        .unwrap();
        (
            r.status,
            serde_json::from_slice(&r.body).unwrap_or(Value::Null),
        )
    }

    /// KOReader's progress sync: `PUT /syncs/progress`.
    pub fn put_progress(&self, doc: &str, percentage: f64) -> u16 {
        self.call(
            "PUT",
            "/syncs/progress",
            &json!({
                "document": doc, "progress": "/body/DocFragment[3]/body",
                "percentage": percentage, "device": self.device, "device_id": self.device_id,
            }),
        )
        .0
    }

    pub fn get_progress(&self, doc: &str) -> Value {
        self.call("GET", &format!("/syncs/progress/{doc}"), &Value::Null)
            .1
    }

    /// The plugin's annotation sync: one highlight.
    pub fn put_highlight(&self, doc: &str, id: &str, text: &str, updated: i64) -> (u16, Value) {
        assert!(self.plugin, "stock KOReader has no annotation sync");
        self.call(
            "PUT",
            &format!("/annotations/{doc}"),
            &json!({
                "origin": "koreader", "device": self.device, "device_id": self.device_id,
                "title": "Zoo Book", "authors": "Zoo Author",
                "annotations": [{
                    "id": id, "kind": "highlight", "text": text, "note": "",
                    "color": "yellow", "drawer": "lighten", "chapter": "One", "page": 3,
                    "created": updated - 5, "updated": updated,
                }],
            }),
        )
    }
}

fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)
    })
}
