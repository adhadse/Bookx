//! `bookx-cli device-check` against fake mounts. Each test builds its own temp mount, so they run in parallel.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use bookx_cli::{Env, run};
use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

fn mount(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bookx-devcheck-{tag}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn check(args: &[&str]) -> (i32, String) {
    let mut v = vec![
        "bookx-cli".to_string(),
        "device-check".to_string(),
        "--fixture-identity".to_string(),
    ];
    v.extend(args.iter().map(|s| s.to_string()));
    let env = Env {
        default_library: std::env::temp_dir().join("bookx-devcheck-unused-lib"),
        stop: Arc::new(AtomicBool::new(false)),
    };
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run(&v, &env, &mut out, &mut err);
    (
        code,
        String::from_utf8(out).unwrap() + &String::from_utf8(err).unwrap(),
    )
}

fn epub_bytes(kobo: bool, compress_mimetype: bool) -> Vec<u8> {
    let body = if kobo {
        r#"<p><span class="koboSpan" id="kobo.1.1">Hi</span></p>"#
    } else {
        "<p>Hi</p>"
    };
    epub_with_page(
        &format!("<html><body>{body}</body></html>"),
        compress_mimetype,
    )
}

fn epub_with_page(page: &str, compress_mimetype: bool) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    z.start_file(
        "mimetype",
        if compress_mimetype { deflated } else { stored },
    )
    .unwrap();
    z.write_all(b"application/epub+zip").unwrap();
    z.start_file("META-INF/container.xml", deflated).unwrap();
    z.write_all(br#"<container><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#).unwrap();
    z.start_file("OEBPS/content.opf", deflated).unwrap();
    z.write_all(b"<package/>").unwrap();
    z.start_file("OEBPS/ch1.xhtml", deflated).unwrap();
    z.write_all(page.as_bytes()).unwrap();
    z.finish().unwrap().into_inner()
}

fn mobi_bytes(version: u32) -> Vec<u8> {
    let mut b = vec![0u8; 200];
    b[60..68].copy_from_slice(b"BOOKMOBI");
    b[78..82].copy_from_slice(&100u32.to_be_bytes());
    b[116..120].copy_from_slice(b"MOBI");
    b[136..140].copy_from_slice(&version.to_be_bytes());
    b
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn kobo(tag: &str) -> PathBuf {
    let m = mount(tag);
    write(&m.join(".kobo/KoboReader.sqlite"), b"sqlite");
    write(&m.join(".kobo/version"), b"1,2,3");
    identify(
        &m,
        r#"{"vendor_id":"2237","product_id":"4237","manufacturer":"Rakuten Kobo Inc."}"#,
    );
    m
}

fn kindle(tag: &str) -> PathBuf {
    let m = mount(tag);
    std::fs::create_dir_all(m.join("system")).unwrap();
    std::fs::create_dir_all(m.join("documents")).unwrap();
    identify(
        &m,
        r#"{"vendor_id":"1949","product_id":"0324","scsi_vendor":"Kindle","scsi_model":"Internal Storage"}"#,
    );
    m
}

const KOBO: &str = r#"{"vendor_id":"2237","product_id":"4237"}"#;
const GEN: &str = r#"{"vendor_id":"2207","product_id":"0001","manufacturer":"Onyx"}"#;

fn identify(m: &Path, json: &str) {
    write(&m.join(".bookx-test-identity"), json.as_bytes());
}

#[test]
fn healthy_kobo_with_a_kepub_passes() {
    let m = kobo("kobo-ok");
    write(&m.join("Bookx/Dune.kepub.epub"), &epub_bytes(true, false));
    let (code, out) = check(&[m.to_str().unwrap()]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("device kind: Kobo"), "{out}");
    assert!(out.contains("KEPUB markup present"), "{out}");
    assert!(out.contains("1 book(s) checked, 0 failure(s)"), "{out}");
    assert!(out.contains("PASS"));
}

#[test]
fn kobo_plain_epub_is_a_warning_and_a_fake_kepub_fails() {
    let m = kobo("kobo-plain");
    write(&m.join("Bookx/Plain.epub"), &epub_bytes(false, false));
    let (code, out) = check(&[m.to_str().unwrap()]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("send KEPUB"), "{out}");
    write(&m.join("Bookx/Fake.kepub.epub"), &epub_bytes(false, false));
    let (code, out) = check(&[m.to_str().unwrap()]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("not a real KEPUB"), "{out}");
}

#[test]
fn kobo_without_its_database_warns() {
    let m = mount("kobo-nodb");
    identify(&m, KOBO);
    std::fs::create_dir_all(m.join(".kobo")).unwrap();
    let (code, out) = check(&[m.to_str().unwrap()]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("no library database"), "{out}");
    assert!(out.contains("books folder does not exist yet"), "{out}");
}

#[test]
fn kindle_checks_azw3_and_mobi_headers() {
    let m = kindle("kindle");
    write(&m.join("documents/Good.azw3"), &mobi_bytes(8));
    write(&m.join("documents/Old.mobi"), &mobi_bytes(6));
    let (code, out) = check(&[m.to_str().unwrap()]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("device kind: Kindle"), "{out}");
    assert!(out.contains("2 book(s) checked, 0 failure(s)"), "{out}");

    write(&m.join("documents/Bad.azw3"), &mobi_bytes(6));
    let (code, out) = check(&[m.to_str().unwrap()]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("no KF8 (version 8) header"), "{out}");
}

#[test]
fn truncated_and_empty_files_fail() {
    let m = kindle("truncated");
    let full = mobi_bytes(8);
    write(&m.join("documents/Cut.azw3"), &full[..90]);
    write(&m.join("documents/Empty.mobi"), b"");
    write(&m.join("documents/NotMobi.azw3"), &vec![7u8; 300]);
    let (code, out) = check(&[m.to_str().unwrap()]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("file is empty"), "{out}");
    assert!(out.contains("does not say BOOKMOBI"), "{out}");
    assert_eq!(out.matches("FAIL ").count(), 3, "{out}");
}

#[test]
fn broken_epub_containers_are_reported() {
    let m = mount("epub-broken");
    identify(&m, GEN);
    std::fs::create_dir_all(m.join("Books")).unwrap();
    write(&m.join("Books/compressed.epub"), &epub_bytes(false, true));
    let good = epub_bytes(false, false);
    write(&m.join("Books/cut.epub"), &good[..good.len() / 2]);
    write(&m.join("Books/text.epub"), b"hello this is not a zip");
    write(&m.join("Books/ok.epub"), &good);
    let (code, out) = check(&[m.to_str().unwrap()]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("device kind: E-reader"), "{out}");
    assert!(out.contains("`mimetype` is compressed"), "{out}");
    assert!(out.contains("broken ZIP"), "{out}");
    assert!(out.contains("not a ZIP file"), "{out}");
    assert!(out.contains("4 book(s) checked, 3 failure(s)"), "{out}");
}

#[test]
fn file_names_fat_cannot_store_fail() {
    let m = mount("names");
    identify(&m, GEN);
    std::fs::create_dir_all(m.join("Books")).unwrap();
    write(
        &m.join("Books/What? A Book.epub"),
        &epub_bytes(false, false),
    );
    let (code, out) = check(&[m.to_str().unwrap()]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("characters FAT cannot store"), "{out}");
}

#[test]
fn a_single_sent_book_can_be_checked_and_a_missing_one_fails() {
    let m = kobo("one");
    write(&m.join("Bookx/A.kepub.epub"), &epub_bytes(true, false));
    write(&m.join("Bookx/Broken.epub"), b"junk");
    let (code, out) = check(&[m.to_str().unwrap(), "--book", "A.kepub.epub"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("1 book(s) checked"), "{out}");
    let (code, out) = check(&[m.to_str().unwrap(), "--book", "Nope.epub"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("not on the device"), "{out}");
}

#[test]
fn unknown_layouts_missing_mounts_and_bad_kinds() {
    let m = mount("empty");
    let (code, out) = check(&[m.to_str().unwrap()]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("not an identified e-reader"), "{out}");
    let (code, out) = check(&["/definitely/not/mounted"]);
    assert_eq!(code, 2, "{out}");
    let (code, out) = check(&[m.to_str().unwrap(), "--kind", "nook"]);
    assert_eq!(code, 2, "{out}");
    // forcing a kind on a non-matching layout warns and then fails on the missing folders
    let (code, out) = check(&[m.to_str().unwrap(), "--kind", "kindle"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("checking as Kindle"), "{out}");
}

#[test]
fn json_report_is_machine_readable() {
    let m = kindle("json");
    write(&m.join("documents/Bad.pdf"), b"nope");
    let (code, out) = check(&[m.to_str().unwrap(), "--json"]);
    assert_eq!(code, 1);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["device"], "kindle");
    assert_eq!(v["failures"], 1);
    assert_eq!(v["books_checked"], 1);
}

#[test]
fn kepub_markup_is_checked_for_duplicate_ids_and_broken_xml() {
    let m = kobo("kobo-spans");
    let good = r#"<html><body><div id="book-columns"><div id="book-inner"><p><span class="koboSpan" id="kobo.1.1">A</span><span class="koboSpan" id="kobo.1.2">B</span></p></div></div></body></html>"#;
    write(
        &m.join("Bookx/Good.kepub.epub"),
        &epub_with_page(good, false),
    );
    let (code, out) = check(&[m.to_str().unwrap()]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("KEPUB markup present (2 spans)"), "{out}");
    assert!(!out.contains("wrapper"), "{out}");

    let dup = good.replace("kobo.1.2", "kobo.1.1");
    write(
        &m.join("Bookx/Dup.kepub.epub"),
        &epub_with_page(&dup, false),
    );
    let (code, out) = check(&[m.to_str().unwrap()]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("appears twice"), "{out}");
    std::fs::remove_file(m.join("Bookx/Dup.kepub.epub")).unwrap();

    let broken = good.replace("</p>", "");
    write(
        &m.join("Bookx/Broken.kepub.epub"),
        &epub_with_page(&broken, false),
    );
    let (code, out) = check(&[m.to_str().unwrap()]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("not well-formed XML"), "{out}");
    std::fs::remove_file(m.join("Bookx/Broken.kepub.epub")).unwrap();

    let odd = good.replace("kobo.1.2", "x7");
    write(
        &m.join("Bookx/Odd.kepub.epub"),
        &epub_with_page(&odd, false),
    );
    let (code, out) = check(&[m.to_str().unwrap()]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("is not kobo.<paragraph>.<sentence>"), "{out}");
}

#[test]
fn a_kobo_with_the_real_schema_reports_model_firmware_and_database_facts() {
    let m = mount("kobo-real");
    identify(&m, KOBO);
    write(
        &m.join(".kobo/version"),
        b"N0000000000000,4.9.77,4.46.23852,4.9.77,4.9.77,00000000-0000-0000-0000-000000000390\n",
    );
    let db = m.join(".kobo/KoboReader.sqlite");
    bookx_serve::kobo::create_real_schema_database(&db).unwrap();
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute_batch(
            "INSERT INTO content (ContentID, ContentType, MimeType, Title, ReadStatus, ___PercentRead, IsDownloaded, ___UserID) VALUES ('file:///mnt/onboard/Known.epub','6','application/epub+zip','Known',0,0,'true','');
             INSERT INTO content (ContentID, ContentType, MimeType, Title, ReadStatus, ___PercentRead, IsDownloaded, ___UserID) VALUES ('file:///mnt/onboard/.adds/k/a.png','6','image/png','a',0,0,'true','');",
        )
        .unwrap();
    // books at the root, as Calibre and KOReader users have them; one is unknown to the firmware
    write(&m.join("Known.epub"), &epub_bytes(false, false));
    write(&m.join("Unknown.epub"), &epub_bytes(false, false));
    let before: Vec<_> = {
        let mut v: Vec<_> = std::fs::read_dir(m.join(".kobo"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        v.sort();
        v
    };
    let (code, out) = check(&[m.to_str().unwrap(), "--all"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("Kobo Libra Colour, firmware 4.46.23852"),
        "{out}"
    );
    assert!(
        !out.contains("N0000000000000"),
        "the serial number is not printed: {out}"
    );
    assert!(
        out.contains("schema version 176, WAL journal, 1 book row(s) (+1 files below .adds)"),
        "{out}"
    );
    assert!(
        out.contains("1 of 2 book file(s) are not in the Kobo database"),
        "{out}"
    );
    assert!(out.contains("2 book(s) checked"), "{out}");
    let after: Vec<_> = {
        let mut v: Vec<_> = std::fs::read_dir(m.join(".kobo"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        v.sort();
        v
    };
    assert_eq!(
        before, after,
        "device-check leaves .kobo exactly as it was (no -shm/-wal)"
    );
    // without --all only the folder Bookx sends to is looked at
    let (_, out) = check(&[m.to_str().unwrap()]);
    assert!(
        out.contains("0 book(s) checked") || out.contains("books folder does not exist"),
        "{out}"
    );
}

#[test]
fn a_look_alike_stick_is_not_an_identified_e_reader() {
    // issue #58: .kobo and Books folders on a flash drive do not make a Kobo
    let m = mount("spoof");
    write(&m.join(".kobo/KoboReader.sqlite"), b"sqlite");
    std::fs::create_dir_all(m.join("Books")).unwrap();
    identify(
        &m,
        r#"{"vendor_id":"0781","product_id":"5581","manufacturer":"SanDisk","serial":"SERIAL-SENTINEL-1","uuid":"UUID-SENTINEL-1","label":"LABEL-SENTINEL-1"}"#,
    );
    let (code, out) = check(&[m.to_str().unwrap()]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("USB vendor id 0781, product id 5581"), "{out}");
    assert!(out.contains("not an identified e-reader"), "{out}");
    assert!(out.contains("a hint only"), "{out}");
    assert!(
        !out.contains("SENTINEL"),
        "no serial, uuid or label is printed: {out}"
    );
}

#[test]
fn the_report_names_how_a_device_was_recognised_and_its_other_storage() {
    let t = mount("nook");
    let int = t.join("NOOK");
    let sd = t.join("SDHC");
    for (m, lun) in [(&int, 0), (&sd, 1)] {
        std::fs::create_dir_all(m.join("Books")).unwrap();
        identify(
            m,
            &format!(
                r#"{{"vendor_id":"2080","product_id":"0003","scsi_vendor":"B&N","scsi_model":"Ebook Disk","group":"nook","lun":{lun}}}"#
            ),
        );
    }
    let (code, out) = check(&[sd.to_str().unwrap()]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("identified by its hardware as Nook (USB 2080:0003)"),
        "{out}"
    );
    assert!(out.contains("USB vendor id 2080, product id 0003"), "{out}");
}

#[test]
fn a_forced_kind_checks_an_unidentified_drive_and_says_so() {
    let m = mount("forced");
    std::fs::create_dir_all(m.join("Books")).unwrap();
    let (_, out) = check(&[m.to_str().unwrap(), "--kind", "generic"]);
    assert!(
        out.contains("not identified, checking as E-reader because you asked"),
        "{out}"
    );
}
