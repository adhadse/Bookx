//! `bookx-cli backup …`: plan, run, list, restore, verify, gc against a folder, an encrypted
//! folder and a WebDAV stand-in. Temp folders only; the passphrase and the cheap key settings go
//! through variables the CLI reads with `bookx_cli::override_var` (per test thread, nothing global).

#[path = "../../bookx-backup/tests/common/mod.rs"]
mod common;

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use bookx_cli::{Env, run};
use common::{Lib, dav, tmp, tree};

static ENV: Mutex<()> = Mutex::new(());

fn cli(lib: &Path, args: &[&str]) -> (i32, String, String) {
    let mut v = vec![
        "bookx-cli".to_string(),
        "--library".into(),
        lib.display().to_string(),
    ];
    v.extend(args.iter().map(|s| s.to_string()));
    let env = Env {
        default_library: lib.to_path_buf(),
        stop: Arc::new(AtomicBool::new(false)),
    };
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run(&v, &env, &mut out, &mut err);
    (
        code,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

fn env_guard() -> std::sync::MutexGuard<'static, ()> {
    let g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    bookx_cli::override_var("BOOKX_BACKUP_KDF", Some("test"));
    bookx_cli::override_var("BOOKX_BACKUP_PASSPHRASE", None);
    bookx_cli::override_var("BOOKX_WEBDAV_PASSWORD", None);
    g
}

#[test]
fn plan_run_list_verify_restore_gc_on_a_folder() {
    let _g = env_guard();
    let mut lib = Lib::new("clibk");
    let a = lib.add("Alpha", 300);
    lib.add("Beta", 300);
    lib.highlight(a, "a note");
    let place = tmp("cli-place");
    let p = place.to_str().unwrap();

    let (code, out, err) = cli(&lib.root, &["backup", "plan", "--to", p]);
    assert_eq!(code, 0, "{out} {err}");
    assert!(out.contains("would copy 2 new file(s)"), "{out}");
    assert!(out.contains("estimate"), "{out}");

    let (code, out, err) = cli(&lib.root, &["backup", "run", "--to", p]);
    assert_eq!(code, 0, "{out} {err}");
    assert!(
        out.contains("backed up 2 file(s)") && out.contains("2 new"),
        "{out}"
    );
    assert!(place.join("Bookx Backup/README.txt").exists());

    let (_, out, _) = cli(&lib.root, &["backup", "run", "--to", p]);
    assert!(out.contains("up to date"), "{out}");

    lib.add("Gamma", 300);
    let (_, out, _) = cli(&lib.root, &["backup", "plan", "--to", p, "--json"]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        (v["new_files"].as_u64(), v["reused_files"].as_u64()),
        (Some(1), Some(2)),
        "{out}"
    );
    let (_, out, _) = cli(
        &lib.root,
        &["backup", "run", "--to", p, "--manual", "--json"],
    );
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        (
            v["new_files"].as_u64(),
            v["reused_files"].as_u64(),
            v["seq"].as_u64()
        ),
        (Some(1), Some(2), Some(2)),
        "{out}"
    );

    let (_, out, _) = cli(&lib.root, &["backup", "list", "--from", p, "--json"]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["snapshots"].as_array().unwrap().len(), 2);
    assert_eq!(v["snapshots"][0]["kind"], "manual");
    assert_eq!(v["snapshots"][0]["files"], 3);

    let (code, out, _) = cli(&lib.root, &["backup", "verify", "--from", p, "--deep"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("all good"));

    // dry run, then the real restore into a new folder
    let to = tmp("cli-restore").join("lib");
    let t = to.to_str().unwrap();
    let (code, out, _) = cli(
        &lib.root,
        &["backup", "restore", "--from", p, "--into", t, "--dry-run"],
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("dry run") && !to.exists(), "{out}");
    let (code, out, err) = cli(
        &lib.root,
        &["backup", "restore", "latest", "--from", p, "--into", t],
    );
    assert_eq!(code, 0, "{out} {err}");
    assert_eq!(tree(&lib.root), tree(&to));
    let (_, titles, _) = cli(&to, &["library", "list"]);
    assert!(
        titles.contains("Gamma") && titles.contains("Alpha"),
        "{titles}"
    );
    let o = bookx_store::open_library(&to).unwrap();
    assert_eq!(o.library.get(a).unwrap().highlights[0].note, "a note");

    // by snapshot number: the first one has two books
    let to1 = tmp("cli-restore1").join("lib");
    let (code, _, err) = cli(
        &lib.root,
        &[
            "backup",
            "restore",
            "1",
            "--from",
            p,
            "--into",
            to1.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(tree(&to1).len(), 2);

    // damage a file: verify says so, --remove-bad drops it and the next run puts it back
    let blobs = place.join("Bookx Backup/blobs");
    let big = walk(&blobs)
        .into_iter()
        .max_by_key(|p| std::fs::metadata(p).unwrap().len())
        .unwrap();
    let mut bytes = std::fs::read(&big).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0xff;
    std::fs::write(&big, bytes).unwrap();
    let (code, out, _) = cli(&lib.root, &["backup", "verify", "--from", p, "--deep"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("PROBLEMS") && out.contains("damaged"), "{out}");
    let (_, out, _) = cli(
        &lib.root,
        &["backup", "verify", "--from", p, "--deep", "--remove-bad"],
    );
    assert!(out.contains("removed 1"), "{out}");
    let (code, out, _) = cli(&lib.root, &["backup", "run", "--to", p, "--force"]);
    assert_eq!(code, 0, "{out}");
    // the newest snapshot is whole again (an older snapshot that needed the damaged file cannot be)
    let to2 = tmp("cli-restore2").join("lib");
    let (code, out, err) = cli(
        &lib.root,
        &[
            "backup",
            "restore",
            "--from",
            p,
            "--into",
            to2.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "{out} {err}");
    assert_eq!(tree(&lib.root), tree(&to2));

    let (code, out, _) = cli(&lib.root, &["backup", "gc", "--from", p]);
    assert_eq!(code, 0);
    assert!(out.contains("in use"), "{out}");
}

fn walk(d: &Path) -> Vec<std::path::PathBuf> {
    let mut v = Vec::new();
    for e in std::fs::read_dir(d).unwrap().flatten() {
        if e.file_type().unwrap().is_dir() {
            v.extend(walk(&e.path()));
        } else {
            v.push(e.path());
        }
    }
    v
}

#[test]
fn an_encrypted_place_needs_the_passphrase_and_never_holds_titles_in_clear() {
    let _g = env_guard();
    let mut lib = Lib::new("clienc");
    lib.add("Hidden Title", 200);
    let place = tmp("cli-enc-place");
    let p = place.to_str().unwrap();

    let (code, _, err) = cli(&lib.root, &["backup", "run", "--to", p, "--encrypt"]);
    assert_eq!(code, 2);
    assert!(err.contains("BOOKX_BACKUP_PASSPHRASE"), "{err}");
    assert!(
        !place.join("Bookx Backup").exists()
            || walk(&place)
                .iter()
                .all(|f| !f.to_string_lossy().contains("blobs"))
    );

    bookx_cli::override_var("BOOKX_BACKUP_PASSPHRASE", Some("correct horse battery"));
    let (code, out, err) = cli(&lib.root, &["backup", "run", "--to", p, "--encrypt"]);
    assert_eq!(code, 0, "{out} {err}");
    assert!(out.contains("encrypted"));
    for f in walk(&place) {
        if f.ends_with("README.txt") || f.ends_with("bookx-repo.json") {
            continue;
        }
        let b = std::fs::read(&f).unwrap();
        assert!(
            !b.windows(6).any(|w| w == b"Hidden"),
            "{} is readable",
            f.display()
        );
        assert!(!b.starts_with(b"PK"), "{} is a plain zip", f.display());
    }
    // wrong and missing passphrase
    bookx_cli::override_var("BOOKX_BACKUP_PASSPHRASE", Some("wrong"));
    let (code, _, err) = cli(&lib.root, &["backup", "list", "--from", p]);
    assert_eq!(code, 2);
    assert!(err.contains("passphrase"), "{err}");
    bookx_cli::override_var("BOOKX_BACKUP_PASSPHRASE", None);
    let (code, _, err) = cli(&lib.root, &["backup", "list", "--from", p]);
    assert_eq!(code, 2);
    assert!(err.contains("encrypted"), "{err}");
    // the right one restores
    bookx_cli::override_var("BOOKX_BACKUP_PASSPHRASE", Some("correct horse battery"));
    let to = tmp("cli-enc-restore").join("lib");
    let (code, out, err) = cli(
        &lib.root,
        &[
            "backup",
            "restore",
            "--from",
            p,
            "--into",
            to.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "{out} {err}");
    assert_eq!(tree(&lib.root), tree(&to));
    bookx_cli::override_var("BOOKX_BACKUP_PASSPHRASE", None);
}

#[test]
fn a_webdav_place_works_end_to_end() {
    let _g = env_guard();
    let d = dav();
    bookx_cli::override_var("BOOKX_WEBDAV_PASSWORD", Some("secret"));
    bookx_cli::override_var("BOOKX_BACKUP_PASSPHRASE", Some("pw for dav"));
    let mut lib = Lib::new("clidav");
    lib.add("Alpha", 300);
    let url = d.url();
    let (code, out, err) = cli(
        &lib.root,
        &[
            "backup",
            "run",
            "--to",
            &url,
            "--user",
            "u",
            "--folder",
            "cli",
            "--encrypt",
        ],
    );
    assert_eq!(code, 0, "{out} {err}");
    assert!(
        d.files
            .lock()
            .unwrap()
            .keys()
            .any(|k| k.contains("/dav/cli/Bookx Backup/blobs/"))
    );
    let (_, out, _) = cli(
        &lib.root,
        &[
            "backup", "run", "--to", &url, "--user", "u", "--folder", "cli",
        ],
    );
    assert!(out.contains("up to date"), "{out}");
    let to = tmp("cli-dav-restore").join("lib");
    let (code, out, err) = cli(
        &lib.root,
        &[
            "backup",
            "restore",
            "--from",
            &url,
            "--user",
            "u",
            "--folder",
            "cli",
            "--into",
            to.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "{out} {err}");
    assert_eq!(tree(&lib.root), tree(&to));
    let (code, out, _) = cli(
        &lib.root,
        &[
            "backup", "verify", "--from", &url, "--user", "u", "--folder", "cli", "--deep",
        ],
    );
    assert_eq!(code, 0, "{out}");
    bookx_cli::override_var("BOOKX_WEBDAV_PASSWORD", None);
    bookx_cli::override_var("BOOKX_BACKUP_PASSPHRASE", None);
}

#[test]
fn the_journal_command_protects_notes_between_snapshots_and_restore_replays_it() {
    let _g = env_guard();
    let mut lib = Lib::new("clijrn");
    let a = lib.add("Alpha", 100);
    let place = tmp("cli-jrn");
    let p = place.to_str().unwrap();
    assert_eq!(cli(&lib.root, &["backup", "run", "--to", p]).0, 0);
    lib.highlight(a, "written after the backup");
    let (code, out, _) = cli(&lib.root, &["backup", "journal", "--to", p]);
    assert_eq!(code, 0);
    assert!(out.contains("journal record 1"), "{out}");
    let (_, out, _) = cli(&lib.root, &["backup", "journal", "--to", p]);
    assert!(out.contains("no notes changed"), "{out}");
    let (_, out, _) = cli(&lib.root, &["backup", "list", "--from", p, "--journal"]);
    assert!(out.contains("journal 1"), "{out}");
    let to = tmp("cli-jrn-restore").join("lib");
    let (code, out, _) = cli(
        &lib.root,
        &[
            "backup",
            "restore",
            "--from",
            p,
            "--into",
            to.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0);
    assert!(out.contains("1 journal record"), "{out}");
    let o = bookx_store::open_library(&to).unwrap();
    assert_eq!(
        o.library.get(a).unwrap().highlights[0].note,
        "written after the backup"
    );
}

#[test]
fn usage_errors_are_clear() {
    let _g = env_guard();
    let lib = Lib::new("cliusage");
    let place = tmp("cli-usage");
    let p = place.to_str().unwrap();
    let (code, _, err) = cli(&lib.root, &["backup", "list", "--from", p]);
    assert_eq!(code, 2);
    assert!(err.contains("no Bookx backup"), "{err}");
    let (code, _, err) = cli(&lib.root, &["backup", "run", "--to", "smb://nas/books"]);
    assert_eq!(code, 2);
    assert!(err.contains("mount it first"), "{err}");
    let (code, _, err) = cli(
        &lib.root,
        &[
            "backup", "restore", "--from", p, "--into", "/tmp/x", "--mode", "nonsense",
        ],
    );
    assert_eq!(code, 2, "{err}");
    let (code, _, err) = cli(
        &lib.root,
        &["backup", "run", "--to", "/definitely-not-mounted/x"],
    );
    assert_eq!(code, 2);
    assert!(err.contains("mounted") || err.contains("plugged"), "{err}");
}
