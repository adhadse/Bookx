//! Device driver used through the `bookx_core::contracts` registry, with real EPUB bytes and fake
//! mount trees.

use std::path::{Path, PathBuf};

use bookx_core::archive::Archive;
use bookx_core::book::OpenBook;
use bookx_core::contracts::{DeviceDriver, Registry};
use bookx_core::convert::{Chapter, create_epub};
use bookx_core::opf::Metadata;
use bookx_serve::devices::{DeviceKind, MassStorageDriver, classify, kobo_db_path};
use bookx_serve::mail::{Address, Attachment, Message, build};

struct Tmp(PathBuf);
impl Tmp {
    fn new() -> Tmp {
        let p = std::env::temp_dir().join(format!("bookx-devit-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn epub_bytes(title: &str) -> Vec<u8> {
    let md = Metadata {
        title: title.into(),
        authors: vec!["Tester".into()],
        ..Default::default()
    };
    let ch = Chapter {
        title: "One".into(),
        body: "<p>hello</p>".into(),
    };
    create_epub(&md, &[ch], None).to_bytes().unwrap()
}

fn make_device(root: &Path, name: &str, kind: DeviceKind) -> PathBuf {
    let m = root.join(name);
    match kind {
        DeviceKind::Kobo => {
            std::fs::create_dir_all(m.join(".kobo")).unwrap();
            std::fs::write(m.join(".kobo/KoboReader.sqlite"), b"SQLite format 3\0").unwrap();
        }
        DeviceKind::Kindle => {
            std::fs::create_dir_all(m.join("documents")).unwrap();
            std::fs::create_dir_all(m.join("system")).unwrap();
        }
        DeviceKind::PocketBook => std::fs::create_dir_all(m.join("system/config")).unwrap(),
        DeviceKind::Generic => std::fs::create_dir_all(m.join("Books")).unwrap(),
    }
    m
}

#[test]
fn send_a_real_epub_to_every_kind_and_read_it_back() {
    let t = Tmp::new();
    let media = t.0.join("media");
    for (name, kind) in [
        ("KOBOeReader", DeviceKind::Kobo),
        ("Kindle", DeviceKind::Kindle),
        ("PB740", DeviceKind::PocketBook),
        ("BOOX", DeviceKind::Generic),
    ] {
        make_device(&media, name, kind);
    }
    let mut reg: Registry<dyn DeviceDriver> = Registry::new();
    reg.register(Box::new(MassStorageDriver::with_roots(vec![media.clone()])));
    assert_eq!(reg.len(), 1);
    let driver = reg.iter().next().unwrap();
    let devices = driver.detect();
    assert_eq!(devices.len(), 4);

    let bytes = epub_bytes("On Device");
    for dev in &devices {
        let path = driver.send(dev, "On Device.epub", &bytes).unwrap();
        assert!(
            path.starts_with(std::fs::canonicalize(&dev.mount).unwrap()),
            "{path:?} in {:?}",
            dev.mount
        );
        let on_disk = std::fs::read(&path).unwrap();
        assert_eq!(on_disk, bytes, "byte-exact copy for {}", dev.name);
        let book = OpenBook::from_archive(Archive::from_bytes(on_disk).unwrap()).unwrap();
        assert_eq!(book.metadata().title, "On Device");
        let listed = driver.list(dev).unwrap();
        assert_eq!(listed, vec![path]);
    }
    // folders per kind
    assert!(media.join("KOBOeReader/Bookx/On Device.epub").exists());
    assert!(media.join("Kindle/documents/On Device.epub").exists());
    assert!(media.join("PB740/Books/On Device.epub").exists());
    assert!(media.join("BOOX/Books/On Device.epub").exists());
}

#[test]
fn sending_the_same_book_twice_keeps_both_copies() {
    let t = Tmp::new();
    let media = t.0.join("media");
    make_device(&media, "K", DeviceKind::Kobo);
    let d = MassStorageDriver::with_roots(vec![media]);
    let dev = d.detect().remove(0);
    let a = epub_bytes("A");
    let b = epub_bytes("B");
    let p1 = d.send(&dev, "same.kepub.epub", &a).unwrap();
    let p2 = d.send(&dev, "same.kepub.epub", &b).unwrap();
    assert_ne!(p1, p2);
    assert!(p2.to_string_lossy().ends_with("same (1).kepub.epub"));
    assert_eq!(std::fs::read(p1).unwrap(), a);
    assert_eq!(std::fs::read(p2).unwrap(), b);
}

#[test]
fn kobo_database_is_detected_but_never_touched() {
    let t = Tmp::new();
    let media = t.0.join("media");
    let k = make_device(&media, "KOBO", DeviceKind::Kobo);
    let db = kobo_db_path(&k).unwrap();
    let before = std::fs::read(&db).unwrap();
    let d = MassStorageDriver::with_roots(vec![media]);
    let dev = d.detect().remove(0);
    d.send(&dev, "x.epub", &epub_bytes("x")).unwrap();
    assert_eq!(std::fs::read(&db).unwrap(), before);
    assert_eq!(classify(&k), Some(DeviceKind::Kobo));
}

#[test]
fn proc_mounts_text_drives_detection() {
    let t = Tmp::new();
    let pb = make_device(&t.0.join("anywhere"), "My Reader", DeviceKind::PocketBook);
    let mounts = t.0.join("proc-mounts");
    // the mount point has a space, which /proc/mounts escapes as \040
    let line = format!(
        "/dev/sdb1 {} vfat rw,nosuid 0 0\ntmpfs /tmp tmpfs rw 0 0\n",
        pb.display().to_string().replace(' ', "\\040")
    );
    std::fs::write(&mounts, line).unwrap();
    let d = MassStorageDriver::with_roots(vec![]).with_proc_mounts(Some(mounts));
    let found = d.detect();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].mount, pb);
    assert!(found[0].id.starts_with("pocketbook:"));
}

#[test]
fn send_by_mail_message_carries_the_same_epub() {
    let bytes = epub_bytes("Mail Me");
    let msg = Message::new(
        Address::parse("Reader <me@example.com>").unwrap(),
        vec![Address::new("name_xyz@kindle.com").unwrap()],
        "Mail Me",
        "Sent from Bookx",
    )
    .attach(Attachment::new("Mail Me.epub", bytes.clone()));
    msg.check_kindle_limit().unwrap();
    let raw = build(&msg).unwrap();
    assert!(raw.is_ascii());
    let text = String::from_utf8(raw).unwrap();
    assert!(text.contains("Content-Type: application/epub+zip"));
    assert!(text.contains("To: name_xyz@kindle.com"));
    // decode the attachment payload with the standard decoder and open it as a book
    let marker = "Content-Disposition: attachment";
    let after = &text[text.find(marker).unwrap()..];
    let payload = after.split("\r\n\r\n").nth(1).unwrap();
    let b64: String = payload
        .lines()
        .take_while(|l| !l.starts_with("--"))
        .collect::<Vec<_>>()
        .join("");
    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .unwrap();
    assert_eq!(decoded, bytes);
    let book = OpenBook::from_archive(Archive::from_bytes(decoded).unwrap()).unwrap();
    assert_eq!(book.metadata().title, "Mail Me");
}
