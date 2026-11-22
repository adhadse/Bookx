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
/// passwords inside URLs and in query strings, `Bearer`/`Basic` credentials, IP addresses (those of the
/// local network become `<lan-ip>`, the others `<ip>`), MAC addresses, values after
/// `password:`/`token=`-like keys and long random-looking strings (keys, tokens, hashes).
pub fn redact(text: &str, home: &str, user: &str) -> String {
    static EMAIL: OnceLock<Regex> = OnceLock::new();
    static URL_CRED: OnceLock<Regex> = OnceLock::new();
    static IPV4: OnceLock<Regex> = OnceLock::new();
    static IPV6: OnceLock<Regex> = OnceLock::new();
    static MAC: OnceLock<Regex> = OnceLock::new();
    static KEYED: OnceLock<Regex> = OnceLock::new();
    static BEARER: OnceLock<Regex> = OnceLock::new();
    static QUERY: OnceLock<Regex> = OnceLock::new();
    static TOKEN: OnceLock<Regex> = OnceLock::new();
    static B64: OnceLock<Regex> = OnceLock::new();
    static HOMES: OnceLock<Regex> = OnceLock::new();
    static MEDIA: OnceLock<Regex> = OnceLock::new();

    let mut s = text.to_string();
    let home = home.trim_end_matches('/');
    if home.len() > 1 {
        s = s.replace(home, "~");
    }
    // other people's or other machines' home folders and mounted drives (labels name people and devices)
    s = rx(&HOMES, r"/(?:home|Users)/[^/\s:]+")
        .replace_all(&s, "~")
        .into_owned();
    s = rx(&MEDIA, r"/(?:run/media|media)/[^/\s:]+(?:/[^/\s:]+)?")
        .replace_all(&s, "<media>")
        .into_owned();
    // Credentials in URLs first, so the user part is not taken for a name below.
    s = rx(&URL_CRED, r"([A-Za-z][A-Za-z0-9+.\-]*://)[^/\s@]+@")
        .replace_all(&s, "${1}<redacted>@")
        .into_owned();
    s = rx(&BEARER, r"(?i)\b(bearer|basic)\s+[A-Za-z0-9._~+/=\-]{6,}")
        .replace_all(&s, "${1} <redacted>")
        .into_owned();
    s = rx(
        &QUERY,
        r"(?i)([?&;][A-Za-z0-9_\-]*(?:token|key|secret|pass(?:word)?|sig(?:nature)?|auth|code|session|credential)[A-Za-z0-9_\-]*=)[^&\s#]+",
    )
    .replace_all(&s, "${1}<redacted>")
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
    s = rx(
        &IPV6,
        r"(?:[0-9A-Fa-f]{1,4}:){7}[0-9A-Fa-f]{1,4}|(?:[0-9A-Fa-f]{1,4}:){1,7}:(?:[0-9A-Fa-f]{1,4}(?::[0-9A-Fa-f]{1,4}){0,6})?|::(?:[0-9A-Fa-f]{1,4}(?::[0-9A-Fa-f]{1,4}){0,6})?",
    )
    .replace_all(&s, |c: &regex::Captures| {
        let m = &c[0];
        let t = c.get(0).map(|m| m.start()).unwrap_or(0);
        // `a::b` in a Rust path is hex letters only: an address has a digit (or is `::1`)
        let before = if t > 0 { s.as_bytes()[t - 1] } else { b' ' };
        let word_before = before.is_ascii_alphanumeric() || before == b'_';
        if word_before || !(m.bytes().any(|b| b.is_ascii_digit())) {
            return m.to_string();
        }
        ipv6_label(m).to_string()
    })
    .into_owned();
    s = rx(&IPV4, r"\b(?:\d{1,3}\.){3}\d{1,3}\b")
        .replace_all(&s, |c: &regex::Captures| {
            let octets: Vec<u32> = c[0].split('.').filter_map(|p| p.parse().ok()).collect();
            if octets.len() != 4 || octets.iter().any(|&o| o > 255) {
                return c[0].to_string();
            }
            ipv4_label(&octets).to_string()
        })
        .into_owned();
    s = rx(&TOKEN, r"\b[A-Za-z0-9_\-]{28,}\b")
        .replace_all(&s, "<secret>")
        .into_owned();
    s = rx(&B64, r"[A-Za-z0-9+/]{20,}={0,2}")
        .replace_all(&s, |c: &regex::Captures| {
            let m = &c[0];
            let mixed = m.bytes().any(|b| b.is_ascii_uppercase())
                && m.bytes().any(|b| b.is_ascii_lowercase())
                && m.bytes().any(|b| b.is_ascii_digit());
            let marked = m.contains('+') || m.ends_with('=');
            if mixed && marked && !m.starts_with('/') {
                "<secret>".to_string()
            } else {
                m.to_string()
            }
        })
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

/// `<lan-ip>` for loopback, private and link-local IPv4 addresses, `<ip>` for the others.
fn ipv4_label(o: &[u32]) -> &'static str {
    let private = o[0] == 10
        || o[0] == 127
        || o[0] == 0
        || (o[0] == 172 && (16..=31).contains(&o[1]))
        || (o[0] == 192 && o[1] == 168)
        || (o[0] == 169 && o[1] == 254);
    if private {
        "<lan-ip>"
    } else {
        "<ip>"
    }
}

/// The same for IPv6: loopback, unique-local (`fc00::/7`) and link-local (`fe80::/10`).
fn ipv6_label(a: &str) -> &'static str {
    let l = a.to_ascii_lowercase();
    if l == "::1"
        || l.starts_with("fc")
        || l.starts_with("fd")
        || l.starts_with("fe8")
        || l.starts_with("fe9")
        || l.starts_with("fea")
        || l.starts_with("feb")
    {
        "<lan-ip>"
    } else {
        "<ip>"
    }
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

    #[test]
    fn private_addresses_are_labelled_and_public_ones_removed() {
        let r = redact(
            "a 192.168.1.20 b 10.0.0.5 c 172.16.4.4 d 172.32.0.1 e 8.8.8.8 f 127.0.0.1 g 999.1.1.1 h 1.2.3",
            "/home/x",
            "xyz",
        );
        assert_eq!(
            r,
            "a <lan-ip> b <lan-ip> c <lan-ip> d <ip> e <ip> f <lan-ip> g 999.1.1.1 h 1.2.3"
        );
    }

    #[test]
    fn ipv6_addresses_are_removed_but_times_and_paths_are_kept() {
        let r = redact(
            "v6 2001:db8::8a2e:370:7334 lan fd12:3456::1 ll fe80::1 loop ::1 full 2001:0db8:85a3:0000:0000:8a2e:0370:7334",
            "/home/x",
            "xyz",
        );
        assert_eq!(
            r,
            "v6 <ip> lan <lan-ip> ll <lan-ip> loop <lan-ip> full <ip>"
        );
        let keep = "12:30:45 bookx_core::add::dec and fe::ad at 10:00";
        assert_eq!(redact(keep, "/home/x", "xyz"), keep);
    }

    #[test]
    fn bearer_headers_and_query_tokens_are_removed() {
        let r = redact(
            "Authorization: Bearer abc.def-123_xyz and https://h.example/p?user=1&access_token=SENTINEL9&x=2 and X-Api-Key=SENTINEL8",
            "/home/x",
            "xyz",
        );
        assert!(!r.contains("abc.def") && !r.contains("SENTINEL"), "{r}");
        assert!(r.contains("x=2"), "{r}");
        let b = redact("sent Bearer abcdef123456 here", "/home/x", "xyz");
        assert_eq!(b, "sent Bearer <redacted> here");
    }

    #[test]
    fn other_home_folders_and_drives_are_removed() {
        let r = redact(
            "/home/bob/x.epub /Users/carol/y /run/media/dave/KOBOeReader/books /media/erin/USB",
            "/home/alice",
            "alice",
        );
        assert_eq!(r, "~/x.epub ~/y <media>/books <media>");
    }

    #[test]
    fn long_hex_and_base64_strings_are_removed_but_words_are_not() {
        let r = redact(
            "k 0123456789abcdef0123456789abcdef b QWxhZGRpbjpvcGVuIHNlc2FtZQ== p /usr/share/applications/org.gnome.Bookx.desktop",
            "/home/x",
            "xyz",
        );
        assert!(
            !r.contains("0123456789abcdef") && !r.contains("QWxhZGRp"),
            "{r}"
        );
        assert!(
            r.contains("/usr/share/applications/org.gnome.Bookx.desktop"),
            "{r}"
        );
    }
}
