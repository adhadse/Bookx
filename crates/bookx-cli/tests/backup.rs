//! `library backup --to` and `library restore --from`: copies to a folder, encryption, listing,
//! restoring from a destination. Own sandbox per test; the passphrase goes through an environment
//! variable, so those tests run one after the other in this file's process via a lock.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use bookx_cli::{Env, run};

static ENV: Mutex<()> = Mutex::new(());

struct Sandbox {
    dir: PathBuf,
}

impl Sandbox {
    fn new() -> Sandbox {
        let dir = std::env::temp_dir().join(format!("bookx-cli-backup-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        Sandbox { dir }
    }
    fn cli(&self, args: &[&str]) -> (i32, String, String) {
        let mut v = vec!["bookx-cli".to_string()];
        v.extend(args.iter().map(|s| s.to_string()));
        let env = Env {
            default_library: self.dir.join("lib"),
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
    fn add(&self, title: &str) {
        let src = self.dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let p = src.join(format!("{title}.md"));
        std::fs::write(
            &p,
            format!("---\ntitle: \"{title}\"\nauthor: \"A\"\n---\n# One\n\nText.\n"),
        )
        .unwrap();
        let (code, out, err) = self.cli(&["library", "add", p.to_str().unwrap()]);
        assert_eq!(code, 0, "{out} {err}");
    }
    fn titles(&self) -> String {
        self.cli(&["library", "list"]).1
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn backup_to_a_folder_lists_and_restores_from_there() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let sb = Sandbox::new();
    sb.add("Alpha");
    let drive = sb.dir.join("usb/bookx");
    std::fs::create_dir_all(sb.dir.join("usb")).unwrap();
    let to = drive.to_str().unwrap();
    let (code, out, err) = sb.cli(&["library", "backup", "--to", to]);
    assert_eq!(code, 0, "{out} {err}");
    assert!(out.contains("checked after writing"), "{out}");
    sb.add("Beta");
    let (code, out, _) = sb.cli(&["library", "snapshots", "--from", to]);
    assert_eq!(code, 0);
    assert!(out.contains("1 books") || out.contains(" 1 books"), "{out}");
    let (code, out, err) = sb.cli(&["library", "restore", "latest", "--from", to]);
    assert_eq!(code, 0, "{out} {err}");
    let t = sb.titles();
    assert!(t.contains("Alpha") && !t.contains("Beta"), "{t}");
}

#[test]
fn encrypted_backups_need_the_passphrase_and_never_contain_the_catalogue_in_clear() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let sb = Sandbox::new();
    sb.add("Gamma Secret Title");
    let to_dir = sb.dir.join("cloud");
    let to = to_dir.to_str().unwrap();
    bookx_cli::override_var("BOOKX_BACKUP_PASSPHRASE", None);
    let (code, _, err) = sb.cli(&["library", "backup", "--to", to, "--encrypt"]);
    assert_ne!(code, 0);
    assert!(err.contains("BOOKX_BACKUP_PASSPHRASE"), "{err}");
    bookx_cli::override_var("BOOKX_BACKUP_PASSPHRASE", Some("correct horse battery"));
    let (code, out, err) = sb.cli(&["library", "backup", "--to", to, "--encrypt"]);
    assert_eq!(code, 0, "{out} {err}");
    let file = std::fs::read_dir(&to_dir)
        .unwrap()
        .flatten()
        .find(|e| e.file_name().to_string_lossy().ends_with(".bookxbackup"))
        .expect("an encrypted file");
    let bytes = std::fs::read(file.path()).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("Gamma Secret Title"));
    sb.add("Delta");
    // wrong passphrase
    bookx_cli::override_var("BOOKX_BACKUP_PASSPHRASE", Some("wrong"));
    let (code, _, err) = sb.cli(&["library", "restore", "latest", "--from", to]);
    assert_ne!(code, 0);
    assert!(err.contains("wrong passphrase"), "{err}");
    assert!(sb.titles().contains("Delta"), "nothing was restored");
    // right passphrase, from the destination and from the picked file
    bookx_cli::override_var("BOOKX_BACKUP_PASSPHRASE", Some("correct horse battery"));
    let (code, out, err) = sb.cli(&["library", "restore", "latest", "--from", to]);
    assert_eq!(code, 0, "{out} {err}");
    assert!(!sb.titles().contains("Delta"));
    sb.add("Epsilon");
    let (code, out, err) = sb.cli(&["library", "restore", file.path().to_str().unwrap()]);
    assert_eq!(code, 0, "{out} {err}");
    let t = sb.titles();
    assert!(t.contains("Gamma") && !t.contains("Epsilon"), "{t}");
    bookx_cli::override_var("BOOKX_BACKUP_PASSPHRASE", None);
}

#[test]
fn smb_addresses_are_explained_not_guessed() {
    let sb = Sandbox::new();
    sb.add("Zeta");
    let (code, _, err) = sb.cli(&["library", "backup", "--to", "smb://nas/books"]);
    assert_ne!(code, 0);
    assert!(err.contains("gio mount"), "{err}");
}

#[test]
fn an_unplugged_drive_is_reported_and_the_local_snapshot_still_exists() {
    let sb = Sandbox::new();
    sb.add("Eta");
    let (code, _, err) = sb.cli(&["library", "backup", "--to", "/definitely-not-mounted/x"]);
    assert_ne!(code, 0);
    assert!(
        err.contains("plugged in") || err.contains("mounted"),
        "{err}"
    );
    assert!(sb.cli(&["library", "snapshots"]).1.contains("Manual"));
}
