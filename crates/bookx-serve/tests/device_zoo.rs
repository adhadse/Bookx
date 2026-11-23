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
use bookx_serve::devices::{DeviceKind, MassStorageDriver, classify};
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
        assert_eq!(classify(&mount), Some(kind), "{}", reader.slug);
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
