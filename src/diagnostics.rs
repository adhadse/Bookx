//! "Copy diagnostics" (X19): the text a user pastes into a bug report.
//!
//! It names versions, the desktop, the enabled features and where Bookx keeps its files. It never
//! holds book titles, account names, passwords, tokens, addresses or the user's home folder:
//! [`redact`] runs over the finished text as a second safety net, after the report was built from
//! settings that are already free of secrets.

use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;

/// What the report says; built by [`gather`], rendered by [`render`] (so tests need no GTK).
#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub version: String,
    pub app_id: String,
    pub os: String,
    pub flatpak: bool,
    pub desktop: String,
    pub session: String,
    pub gtk: String,
    pub adw: String,
    pub language: String,
    pub locale: String,
    pub library_dir: String,
    pub config_file: String,
    pub books: usize,
    pub features: Vec<(String, String)>,
    pub plugins: usize,
    pub patches: usize,
}

/// The finished report for these facts. `home` and `user` are removed from it.
pub fn render(f: &Facts, home: &str, user: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!("Bookx {} ({})\n", f.version, f.app_id));
    s.push_str(&format!(
        "Installed as: {}\n",
        if f.flatpak { "Flatpak" } else { "native" }
    ));
    s.push_str(&format!("System: {}\n", f.os));
    s.push_str(&format!("Desktop: {} ({})\n", f.desktop, f.session));
    s.push_str(&format!("GTK {}, libadwaita {}\n", f.gtk, f.adw));
    s.push_str(&format!("Language: {} (locale {})\n", f.language, f.locale));
    s.push_str(&format!("Library: {} ({} books)\n", f.library_dir, f.books));
    s.push_str(&format!("Settings file: {}\n", f.config_file));
    s.push_str(&format!("Plugins: {}, patches: {}\n", f.plugins, f.patches));
    s.push_str("Features:\n");
    for (name, state) in &f.features {
        s.push_str(&format!("  {name}: {state}\n"));
    }
    redact(&s, home, user)
}

fn rx(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("static pattern"))
}

/// Removes personal data from a text: the home folder (`~`), the user name, e-mail addresses,
/// passwords inside URLs, IP and MAC addresses, values after `password:`/`token=`-like keys and
/// long random-looking strings (keys, tokens).
pub fn redact(text: &str, home: &str, user: &str) -> String {
    static EMAIL: OnceLock<Regex> = OnceLock::new();
    static URL_CRED: OnceLock<Regex> = OnceLock::new();
    static IPV4: OnceLock<Regex> = OnceLock::new();
    static MAC: OnceLock<Regex> = OnceLock::new();
    static KEYED: OnceLock<Regex> = OnceLock::new();
    static TOKEN: OnceLock<Regex> = OnceLock::new();

    let mut s = text.to_string();
    let home = home.trim_end_matches('/');
    if home.len() > 1 {
        s = s.replace(home, "~");
    }
    // Credentials in URLs first, so the user part is not taken for a name below.
    s = rx(&URL_CRED, r"([A-Za-z][A-Za-z0-9+.\-]*://)[^/\s@]+@")
        .replace_all(&s, "${1}<redacted>@")
        .into_owned();
    s = rx(
        &KEYED,
        r"(?i)\b(pass(?:word|phrase|wd)?|secret|token|api[_\- ]?key|authorization|cookie)(\s*[:=]\s*)\S+",
    )
    .replace_all(&s, "${1}${2}<redacted>")
    .into_owned();
    s = rx(&EMAIL, r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}")
        .replace_all(&s, "<email>")
        .into_owned();
    s = rx(&MAC, r"\b(?:[0-9A-Fa-f]{2}[:\-]){5}[0-9A-Fa-f]{2}\b")
        .replace_all(&s, "<mac>")
        .into_owned();
    s = rx(&IPV4, r"\b(?:\d{1,3}\.){3}\d{1,3}\b")
        .replace_all(&s, "<ip>")
        .into_owned();
    s = rx(&TOKEN, r"\b[A-Za-z0-9_\-]{28,}\b")
        .replace_all(&s, "<secret>")
        .into_owned();
    let user = user.trim();
    if user.len() >= 3 && user != "root" {
        // The application id is not personal even when the user is called like the developer.
        const KEEP: &str = "\u{1}app-id\u{1}";
        s = s.replace(crate::config::APP_ID, KEEP);
        let word = Regex::new(&format!(r"\b{}\b", regex::escape(user))).expect("escaped");
        s = word.replace_all(&s, "<user>").into_owned();
        s = s.replace(KEEP, crate::config::APP_ID);
    }
    s
}

fn os_name() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|t| {
            t.lines()
                .find_map(|l| l.strip_prefix("PRETTY_NAME="))
                .map(|v| v.trim_matches('"').to_string())
        })
        .unwrap_or_else(|| std::env::consts::OS.to_string())
}

fn count_dirs(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|rd| rd.flatten().count())
        .unwrap_or(0)
}

fn on_off(b: bool) -> String {
    if b { "on" } else { "off" }.to_string()
}

/// Collects the facts of this run; the settings are looked at, never copied (no passwords).
pub fn gather(cfg: &crate::config::Config, books: usize) -> Facts {
    use relm4::{adw, gtk};
    let var = |k: &str| std::env::var(k).unwrap_or_default();
    let enabled = |b: bool| on_off(b);
    let features = vec![
        ("E-ink mode".to_string(), enabled(cfg.eink)),
        (
            "Library sharing server".to_string(),
            enabled(cfg.share.lan || cfg.share.kosync),
        ),
        (
            "Automatic backups".to_string(),
            enabled(cfg.health.auto_backup),
        ),
        ("Footnote popups".to_string(), enabled(cfg.footnote_popups)),
        ("Read aloud engine".to_string(), cfg.tts.engine.clone()),
        (
            "Network (Flatpak sandbox)".to_string(),
            "granted by the manifest".to_string(),
        ),
    ];
    Facts {
        version: crate::config::VERSION.to_string(),
        app_id: crate::config::APP_ID.to_string(),
        os: os_name(),
        flatpak: Path::new("/.flatpak-info").exists(),
        desktop: var("XDG_CURRENT_DESKTOP"),
        session: var("XDG_SESSION_TYPE"),
        gtk: format!(
            "{}.{}.{}",
            gtk::major_version(),
            gtk::minor_version(),
            gtk::micro_version()
        ),
        adw: format!(
            "{}.{}.{}",
            adw::major_version(),
            adw::minor_version(),
            adw::micro_version()
        ),
        language: crate::i18n::current(),
        locale: var("LANG"),
        library_dir: cfg.library_dir.display().to_string(),
        config_file: crate::config::config_file_path_text(),
        books,
        features,
        plugins: count_dirs(&crate::plugins::plugins_dir()),
        patches: count_dirs(&crate::plugins::patches_dir()),
    }
}

/// The report for the running app, redacted.
pub fn report(cfg: &crate::config::Config, books: usize) -> String {
    let home = directories::BaseDirs::new()
        .map(|d| d.home_dir().display().to_string())
        .unwrap_or_default();
    let user = std::env::var("USER").unwrap_or_default();
    render(&gather(cfg, books), &home, &user)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_home_folder_and_user_name_are_removed() {
        let t = "Library: /home/alice/Documents/Bookx Library\nowner alice";
        let r = redact(t, "/home/alice", "alice");
        assert_eq!(r, "Library: ~/Documents/Bookx Library\nowner <user>");
        assert!(!r.contains("alice"));
    }

    #[test]
    fn the_application_id_survives_a_user_with_the_same_name() {
        let r = redact(
            "Bookx 0.2.0 (com.adhadse.Bookx) in /home/adhadse/.var/app/com.adhadse.Bookx",
            "/home/adhadse",
            "adhadse",
        );
        assert_eq!(
            r,
            "Bookx 0.2.0 (com.adhadse.Bookx) in ~/.var/app/com.adhadse.Bookx"
        );
    }

    #[test]
    fn addresses_and_credentials_are_removed() {
        let t = "mail me@example.org via smtp://bob:hunter2@mail.example.org:587 \
                 from 192.168.1.20 mac aa:bb:cc:dd:ee:ff";
        let r = redact(t, "/home/x", "xyz");
        assert!(!r.contains("me@example.org"), "{r}");
        assert!(!r.contains("hunter2") && !r.contains("bob"), "{r}");
        assert!(!r.contains("192.168.1.20"), "{r}");
        assert!(!r.contains("aa:bb:cc"), "{r}");
        assert!(r.contains("smtp://<redacted>@mail.example.org:587"), "{r}");
    }

    #[test]
    fn secrets_after_keys_and_long_tokens_are_removed() {
        let r = redact(
            "password: s3cret!\nToken=abc123\napi_key = ZZZ\nid 0123456789abcdef0123456789abcdef01",
            "/home/x",
            "xyz",
        );
        assert!(
            !r.contains("s3cret") && !r.contains("abc123") && !r.contains("ZZZ"),
            "{r}"
        );
        assert!(!r.contains("0123456789abcdef0123456789abcdef01"), "{r}");
        assert!(r.contains("password: <redacted>"), "{r}");
    }

    #[test]
    fn harmless_text_is_kept() {
        let t = "Bookx 0.2.0\nGTK 4.20.1, libadwaita 1.8.0\nLanguage: de (locale de_DE.UTF-8)\n";
        assert_eq!(redact(t, "/home/alice", "alice"), t);
        // short or root user names are not hunted down inside other words
        assert_eq!(redact("rootfs ok", "/root", "root"), "rootfs ok");
        assert_eq!(redact("an ab", "/home/an", "an"), "an ab");
    }

    #[test]
    fn the_report_lists_versions_and_features_without_personal_data() {
        let facts = Facts {
            version: "0.2.0".into(),
            app_id: "com.adhadse.Bookx".into(),
            os: "Fedora Linux 43".into(),
            flatpak: true,
            desktop: "GNOME".into(),
            session: "wayland".into(),
            gtk: "4.20.1".into(),
            adw: "1.8.0".into(),
            language: "en".into(),
            locale: "en_US.UTF-8".into(),
            library_dir: "/home/alice/Documents/Bookx Library".into(),
            config_file: "/home/alice/.var/app/com.adhadse.Bookx/config/bookx/config.json".into(),
            books: 12,
            features: vec![("E-ink mode".into(), "off".into())],
            plugins: 1,
            patches: 2,
        };
        let r = render(&facts, "/home/alice", "alice");
        assert!(r.contains("Bookx 0.2.0 (com.adhadse.Bookx)"));
        assert!(r.contains("Installed as: Flatpak"));
        assert!(r.contains("GTK 4.20.1, libadwaita 1.8.0"));
        assert!(r.contains("Library: ~/Documents/Bookx Library (12 books)"));
        assert!(r.contains("E-ink mode: off"));
        assert!(!r.contains("alice"));
    }
}
