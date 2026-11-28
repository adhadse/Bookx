//! The device zoo (`tests/common/zoo.rs`): popular e-readers emulated against the real server and
//! the real device drivers. Nothing here ran on a real device (see `docs/hardware-test-plan.md`).
//!
//! Two things are checked per reader: how it looks when plugged in (USB disk or MTP: detection,
//! the folder and format a book gets) and what Bookx shows when its KOReader syncs (the device
//! name, progress and highlights). Every test builds its own folders and its own server, so they
//! run in parallel.

mod common;

use std::path::PathBuf;

use bookx_core::contracts::DeviceDriver;
use bookx_serve::activity::What;
use bookx_serve::devices::{DeviceKind, MassStorageDriver, layout_hint};
use bookx_serve::kobo::parse_version;
use bookx_serve::mtp::MtpGvfsDriver;
use common::zoo::{self, Bench, Family, Peer, Usb, ZOO};

const DOC: &str = "0123456789abcdef0123456789abcdef";

struct Tmp(PathBuf);
impl Tmp {
    fn new() -> Tmp {
        let p = std::env::temp_dir().join(format!("bookx-zoo-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn net_off() -> bool {
    std::env::var("NET").is_ok_and(|v| v == "0")
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

#[test]
fn the_zoo_is_well_formed() {
    let mut slugs: Vec<&str> = ZOO.iter().map(|r| r.slug).collect();
    slugs.sort();
    slugs.dedup();
    assert_eq!(slugs.len(), ZOO.len(), "slugs are unique");
    assert!(
        ZOO.iter().all(|r| !r.seen_on_hardware),
        "the zoo is emulation: nothing in it was observed on a device"
    );
    for f in [
        Family::Boox,
        Family::Kobo,
        Family::Kindle,
        Family::PocketBook,
        Family::Remarkable,
    ] {
        assert!(ZOO.iter().any(|r| r.family == f), "{f:?} is in the zoo");
    }
}

#[test]
fn every_reader_is_shown_by_the_name_on_its_box_after_a_sync() {
    if net_off() {
        return;
    }
    for reader in ZOO {
        // a server of its own per reader: the list then has exactly one row
        let bench = Bench::start();
        let peer = Peer::plugin(&bench, reader);
        assert_eq!(peer.put_progress(DOC, 0.25), 200, "{}", reader.slug);
        let (status, v) = peer.put_highlight(DOC, "h1", "A line worth keeping.", now() - 5);
        assert_eq!(status, 200, "{}: {v}", reader.slug);
        let devices = bench.log.devices();
        assert_eq!(devices.len(), 1, "{}: {devices:?}", reader.slug);
        let d = &devices[0];
        assert_eq!(
            d.name, reader.reported,
            "{}: what arrived is kept",
            reader.slug
        );
        assert_eq!(
            d.display_name(),
            reader.marketing,
            "{}: what the Sync window shows",
            reader.slug
        );
        assert_eq!(d.client, "KOReader 2026.07.2");
        assert_eq!(d.plugin, "0.3.1");
        assert!(d.last_sync.is_some(), "{}", reader.slug);
        // the activity rows name it the same way
        let events = bench.log.events(10);
        assert!(!events.is_empty(), "{}", reader.slug);
        for e in &events {
            assert_eq!(
                bench.log.name_for(&e.device_id, &e.device),
                reader.marketing,
                "{}",
                reader.slug
            );
        }
        assert!(
            events
                .iter()
                .any(|e| matches!(e.what, What::Received { highlights: 1, .. })),
            "{}: {events:?}",
            reader.slug
        );
    }
}

#[test]
fn progress_travels_with_the_device_name_as_reported() {
    if net_off() {
        return;
    }
    let bench = Bench::start();
    let leaf = zoo::by_slug("boox-leaf2-issue-39");
    let peer = Peer::plugin(&bench, leaf);
    assert_eq!(peer.put_progress(DOC, 0.4), 200);
    let v = peer.get_progress(DOC);
    assert_eq!(v["device"], "Onyx Scubapilot-IDP");
    assert_eq!(v["percentage"], 0.4);
    // another reader of the same account reads it back
    let kobo = Peer::plugin(&bench, zoo::by_slug("kobo-libra-2"));
    assert_eq!(kobo.get_progress(DOC)["device"], "Onyx Scubapilot-IDP");
    assert_eq!(kobo.put_progress(DOC, 0.6), 200);
    assert_eq!(peer.get_progress(DOC)["device"], "Kobo_io");
    assert_eq!(bench.log.devices().len(), 2, "two readers, two rows");
}

#[test]
fn stock_koreader_without_the_plugin_is_named_too() {
    if net_off() {
        return;
    }
    for reader in ZOO {
        let bench = Bench::start();
        let peer = Peer::stock(&bench, reader);
        assert_eq!(peer.put_progress(DOC, 0.1), 200, "{}", reader.slug);
        let d = bench.log.devices();
        assert_eq!(d.len(), 1, "{}", reader.slug);
        assert_eq!(d[0].display_name(), reader.marketing, "{}", reader.slug);
        assert!(
            d[0].plugin.is_empty() && d[0].client.is_empty(),
            "{}: no Bookx headers, so no plugin or version is claimed",
            reader.slug
        );
    }
}

/// Regression test of issue #39: a Boox Leaf 2 that reports `Onyx Scubapilot-IDP` was listed under
/// that name. (Fails when `friendly_name` is taken out.)
#[test]
fn issue_39_the_leaf_2_is_not_listed_as_onyx_scubapilot_idp() {
    if net_off() {
        return;
    }
    let bench = Bench::start();
    let leaf = zoo::by_slug("boox-leaf2-issue-39");
    assert_eq!(leaf.reported, "Onyx Scubapilot-IDP");
    let peer = Peer::plugin(&bench, leaf);
    assert_eq!(peer.put_progress(DOC, 0.5), 200);
    let d = &bench.log.devices()[0];
    assert_ne!(d.display_name(), "Onyx Scubapilot-IDP");
    assert_eq!(d.display_name(), "Boox Leaf 2");
}

#[test]
fn a_model_that_is_not_known_is_shown_as_reported_and_can_be_renamed() {
    if net_off() {
        return;
    }
    let bench = Bench::start();
    let phone = zoo::by_slug("android-phone-unknown");
    let peer = Peer::plugin(&bench, phone);
    assert_eq!(peer.put_progress(DOC, 0.5), 200);
    let id = bench.log.devices()[0].id.clone();
    assert_eq!(
        bench.log.devices()[0].display_name(),
        "panther",
        "never guessed"
    );
    assert!(bench.log.rename(&id, "Dad's phone"));
    assert_eq!(bench.log.devices()[0].display_name(), "Dad's phone");
    // the next sync does not undo it
    assert_eq!(peer.put_progress(DOC, 0.6), 200);
    assert_eq!(bench.log.devices()[0].display_name(), "Dad's phone");
    assert_eq!(bench.log.devices()[0].name, "panther");
}

#[test]
fn a_name_typed_in_the_plugin_is_shown_as_typed() {
    if net_off() {
        return;
    }
    let bench = Bench::start();
    let mut peer = Peer::plugin(&bench, zoo::by_slug("boox-leaf2-issue-39"));
    peer.device = "Bedroom Leaf".into();
    assert_eq!(peer.put_progress(DOC, 0.5), 200);
    assert_eq!(bench.log.devices()[0].display_name(), "Bedroom Leaf");
}

#[test]
fn usb_disks_are_detected_and_get_books_in_the_folder_their_firmware_reads() {
    for reader in ZOO {
        let (kind, folder, format) = match reader.usb {
            Usb::Kobo { .. } => (DeviceKind::Kobo, "Bookx", "kepub.epub"),
            Usb::Kindle => (DeviceKind::Kindle, "documents", "azw3"),
            Usb::PocketBook => (DeviceKind::PocketBook, "Books", "epub"),
            Usb::Disk => (DeviceKind::Generic, "Books", "epub"),
            Usb::Mtp { .. } | Usb::None => continue,
        };
        let t = Tmp::new();
        let mount = zoo::build_usb(&t.0, reader.slug, reader.usb);
        assert_eq!(layout_hint(&mount), Some(kind), "{}", reader.slug);
        let drv = MassStorageDriver::with_roots(vec![t.0.clone()]);
        let found = drv.detect();
        assert_eq!(found.len(), 1, "{}: {found:?}", reader.slug);
        assert!(
            found[0].id.starts_with(&format!("{}:", kind.slug())),
            "{}: {}",
            reader.slug,
            found[0].id
        );
        assert_eq!(drv.formats_for(&found[0])[0], format, "{}", reader.slug);
        let sent = drv.send(&found[0], "Dune.epub", b"PK-not-really").unwrap();
        assert!(
            sent.starts_with(mount.join(folder)),
            "{}: {sent:?}",
            reader.slug
        );
        assert_eq!(drv.list(&found[0]).unwrap(), vec![sent], "{}", reader.slug);
    }
}

#[test]
fn a_kobo_says_what_it_is_only_when_its_id_was_seen_on_a_real_device() {
    for reader in ZOO {
        let Usb::Kobo {
            product_id,
            id_seen_on_hardware,
        } = reader.usb
        else {
            continue;
        };
        let t = Tmp::new();
        let mount = zoo::build_usb(&t.0, reader.slug, reader.usb);
        let text = std::fs::read_to_string(mount.join(".kobo/version")).unwrap();
        let v = parse_version(&text).expect("a version file Bookx can read");
        assert_eq!(v.firmware, "4.46.23852");
        assert_eq!(v.model_id, product_id);
        let said = v.describe();
        if id_seen_on_hardware {
            assert_eq!(said, "Kobo Libra Colour, firmware 4.46.23852");
        } else {
            assert!(
                said.starts_with("Kobo (model id "),
                "{}: an id nobody read on a device is shown, not guessed: {said}",
                reader.slug
            );
        }
        assert!(
            !said.contains("N000000000000"),
            "the serial number is never kept"
        );
    }
}

#[test]
fn the_libra_colour_zoo_entry_matches_its_real_id() {
    // the one Kobo whose id was read from a real device keeps being named from it
    let r = zoo::by_slug("kobo-libra-colour");
    assert!(matches!(
        r.usb,
        Usb::Kobo {
            id_seen_on_hardware: true,
            ..
        }
    ));
}

#[test]
fn a_boox_is_an_mtp_device_and_books_go_to_its_books_folder() {
    for reader in ZOO {
        let Usb::Mtp { host } = reader.usb else {
            continue;
        };
        let t = Tmp::new();
        let storage = zoo::build_mtp(&t.0, host);
        let drv = MtpGvfsDriver::with_gvfs_dir(t.0.clone());
        let found = drv.detect();
        assert_eq!(found.len(), 1, "{}: {found:?}", reader.slug);
        assert!(found[0].id.starts_with("mtp:"), "{}", reader.slug);
        assert!(
            found[0].name.ends_with("Internal shared storage (MTP)"),
            "{}: {}",
            reader.slug,
            found[0].name
        );
        assert_eq!(drv.kind_of(&found[0]), DeviceKind::Generic);
        let sent = drv.send(&found[0], "Dune.epub", b"PK").unwrap();
        assert!(sent.starts_with(storage.join("Books")), "{sent:?}");
        assert_eq!(drv.list(&found[0]).unwrap(), vec![sent]);
        // the other direction does not exist: nothing on the device is copied into the library by
        // detecting or listing it
        assert!(
            std::fs::read_dir(&t.0).unwrap().count() == 1,
            "{}: detecting wrote nothing",
            reader.slug
        );
    }
}

#[test]
fn a_boox_is_not_seen_by_the_usb_disk_driver() {
    let t = Tmp::new();
    let _ = zoo::build_mtp(&t.0, "ONYX_Leaf2_0123456789");
    assert!(
        MassStorageDriver::with_roots(vec![t.0.clone()])
            .detect()
            .is_empty(),
        "an MTP tree has no disk layout; only the MTP driver offers it"
    );
}

#[test]
fn no_two_readers_of_one_account_share_a_row() {
    if net_off() {
        return;
    }
    let bench = Bench::start();
    for reader in ZOO {
        let peer = Peer::plugin(&bench, reader);
        assert_eq!(peer.put_progress(DOC, 0.2), 200, "{}", reader.slug);
    }
    let d = bench.log.devices();
    assert_eq!(d.len(), ZOO.len());
    let mut names: Vec<String> = d.iter().map(|d| d.display_name()).collect();
    names.sort();
    // two zoo entries are the same reader (Leaf 2, as the issue reported it and as KOReader names it)
    names.dedup();
    assert!(
        names.contains(&"Boox Leaf 2".to_string()) && names.contains(&"Kobo Libra 2".to_string())
    );
}

// ---- what a drive IS: the identity zoo (issues #57 and #58). Nothing here ran on real hardware.

fn drive(root: &std::path::Path, name: &str, json: &str, folders: &[&str]) -> PathBuf {
    let m = root.join(name);
    for f in folders {
        std::fs::create_dir_all(m.join(f)).unwrap();
    }
    std::fs::create_dir_all(&m).unwrap();
    zoo::write_identity(&m, json);
    m
}

const KOBO: &str =
    r#"{"vendor_id":"2237","product_id":"4237","manufacturer":"Rakuten Kobo Inc.","group":"k"}"#;
const KINDLE: &str = r#"{"vendor_id":"1949","product_id":"0324","scsi_vendor":"Kindle","scsi_model":"Internal Storage","group":"kin"}"#;
const NOOK_INT: &str = r#"{"vendor_id":"2080","product_id":"0003","scsi_vendor":"B&N","scsi_model":"Ebook Disk","group":"nook","lun":0,"label":"NOOK","size":220000000}"#;
const NOOK_SD: &str = r#"{"vendor_id":"2080","product_id":"0003","scsi_vendor":"B&N","scsi_model":"Ebook Disk","group":"nook","lun":1,"label":"SDHC","size":8000000000}"#;
const BOOX: &str = r#"{"vendor_id":"2207","product_id":"0001","manufacturer":"Onyx","product":"Note","group":"boox"}"#;
const POCKET: &str = r#"{"vendor_id":"fffe","product_id":"0001","manufacturer":"Obreey","scsi_vendor":"USB-FS","scsi_model":"PocketBook","group":"pb"}"#;
const STICK: &str = r#"{"vendor_id":"0781","product_id":"5581","manufacturer":"SanDisk","scsi_vendor":"SanDisk","scsi_model":"Ultra","group":"stick","uuid":"1111-2222","label":"STICK","size":16000000000}"#;
const READER: &str = r#"{"vendor_id":"058f","product_id":"6366","manufacturer":"Generic","scsi_vendor":"Generic","scsi_model":"STORAGE DEVICE","group":"reader","lun":0,"label":"SDHC","size":8000000000}"#;

#[test]
fn each_family_is_recognised_by_its_hardware_and_gets_books_where_it_reads_them() {
    for (json, folders, kind, folder) in [
        (KOBO, &[".kobo"][..], DeviceKind::Kobo, "Bookx"),
        (
            KINDLE,
            &["documents", "system"][..],
            DeviceKind::Kindle,
            "documents",
        ),
        (BOOX, &["Books"][..], DeviceKind::Generic, "Books"),
        (
            POCKET,
            &["system/config"][..],
            DeviceKind::PocketBook,
            "Books",
        ),
        (NOOK_INT, &["My Files"][..], DeviceKind::Generic, "Books"),
    ] {
        let t = Tmp::new();
        let m = drive(&t.0, "D", json, folders);
        let drv = MassStorageDriver::with_roots(vec![t.0.clone()]);
        let a = drv.scan();
        assert_eq!(a.found.len(), 1, "{json}");
        assert!(a.others.is_empty(), "{json}");
        let f = &a.found[0];
        assert!(
            f.info.id.starts_with(&format!("{}:", kind.slug())),
            "{}",
            f.info.id
        );
        assert!(!f.user_chosen && f.recognised.is_some());
        let sent = drv.send(&f.info, "x.epub", b"PK").unwrap();
        assert!(sent.starts_with(m.join(folder)), "{sent:?}");
    }
}

#[test]
fn a_nook_with_a_card_lists_the_card_under_the_nook_and_sends_to_it() {
    let t = Tmp::new();
    drive(&t.0, "NOOK", NOOK_INT, &["My Files"]);
    let sd = drive(&t.0, "SDHC", NOOK_SD, &[]);
    let drv = MassStorageDriver::with_roots(vec![t.0.clone()]);
    let a = drv.scan();
    assert!(a.others.is_empty(), "the card is never an 'other drive'");
    assert_eq!(a.found.len(), 2);
    let main = a.found.iter().find(|f| !f.is_card).unwrap();
    let card = a.found.iter().find(|f| f.is_card).unwrap();
    assert_eq!(card.info.parent.as_deref(), Some(main.info.id.as_str()));
    assert_eq!(card.info.name, "Nook, memory card (SDHC)");
    assert_eq!(drv.kind_of(&card.info), drv.kind_of(&main.info));
    let sent = drv.send(&card.info, "x.epub", b"PK").unwrap();
    assert!(sent.starts_with(sd));
}

#[test]
fn flash_drives_and_look_alikes_are_never_devices() {
    let t = Tmp::new();
    // a stick with a Books folder
    drive(&t.0, "BOOKSTICK", STICK, &["Books"]);
    // a Calibre library on a stick
    let c = drive(
        &t.0,
        "CALIBRE",
        &STICK
            .replace("stick", "cal")
            .replace("1111-2222", "3333-4444"),
        &["Books"],
    );
    std::fs::write(c.join("metadata.db"), b"SQLite").unwrap();
    // a stick labelled like a Kobo with a .kobo folder (the spoof of issue #58)
    drive(
        &t.0,
        "KOBOeReader",
        &STICK
            .replace("stick", "spoof")
            .replace("1111-2222", "5555-6666")
            .replace("STICK", "KOBOeReader"),
        &[".kobo", "Books"],
    );
    // an SD card in a USB card reader: another USB device than any reader
    drive(&t.0, "SDHC", READER, &["Books"]);
    // a device whose sysfs cannot be read: not identified, and still not guessed from its folders
    drive(
        &t.0,
        "MYSTERY",
        r#"{"unreadable":true}"#,
        &[".kobo", "documents", "system"],
    );
    let drv = MassStorageDriver::with_roots(vec![t.0.clone()]);
    assert!(drv.detect().is_empty(), "{:?}", drv.detect());
    let a = drv.scan();
    assert_eq!(a.others.len(), 5);
    use bookx_serve::devices::NotIdentified;
    assert_eq!(
        a.others
            .iter()
            .filter(|o| o.why == NotIdentified::Unreadable)
            .count(),
        1
    );
}

#[test]
fn a_hub_with_two_readers_gives_two_devices_without_cross_grouping() {
    let t = Tmp::new();
    drive(
        &t.0,
        "A",
        &KOBO.replace(r#""group":"k""#, r#""group":"hub/1""#),
        &[".kobo"],
    );
    drive(
        &t.0,
        "B",
        &KOBO.replace(r#""group":"k""#, r#""group":"hub/2""#),
        &[".kobo"],
    );
    drive(&t.0, "C", NOOK_INT, &[]);
    let a = MassStorageDriver::with_roots(vec![t.0.clone()]).scan();
    assert_eq!(a.found.len(), 3);
    assert!(
        a.found
            .iter()
            .all(|f| !f.is_card && f.info.parent.is_none())
    );
}

#[test]
fn a_drive_the_user_chose_becomes_a_device_and_stop_undoes_it() {
    use bookx_serve::devices::{UserDrive, drive_key};
    let t = Tmp::new();
    let m = drive(&t.0, "CALIBRE", STICK, &["Books"]);
    let drv = MassStorageDriver::with_roots(vec![t.0.clone()]);
    assert!(drv.detect().is_empty());
    let key = drive_key(&drv.volumes()[0]);
    assert!(!key.contains("1111") && !key.contains("STICK"));
    let chosen = drv.clone().with_user_drives(vec![UserDrive {
        key,
        kind: "generic".into(),
        folder: String::new(),
    }]);
    let a = chosen.scan();
    assert_eq!(a.found.len(), 1);
    assert!(a.found[0].user_chosen && a.others.is_empty());
    let sent = chosen.send(&a.found[0].info, "b.epub", b"PK").unwrap();
    assert!(sent.starts_with(m.join("Books")));
    // "Stop using as an e-reader" removes the entry: the drive is only an other drive again
    assert!(chosen.clone().with_user_drives(vec![]).detect().is_empty());
}
