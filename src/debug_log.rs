//! The debug log of About → Troubleshooting → Debug Information (X19): the diagnostics report plus the
//! last lines Bookx logged, with everything personal removed.
//!
//! On top of [`crate::diagnostics::redact`] (home folder, user, e-mail, IP, tokens) the log is cleaned of
//! the library folder, the titles of the books and the exact values of the user's secrets (passwords,
//! tokens, passphrases), so a title that appears in a message such as "cannot open Dune" never leaves the
//! machine. The log lines are kept in memory only ([`Ring`]); nothing is written to disk.

use std::collections::VecDeque;
use std::io::Write;
use std::sync::{Mutex, OnceLock};

use regex::Regex;

/// Lines kept.
pub const MAX_LINES: usize = 400;
/// Longest kept line, in bytes (longer ones are cut).
pub const MAX_LINE_BYTES: usize = 600;
/// Most book titles looked for in the log (the longest ones first).
pub const MAX_TITLES: usize = 5000;

/// The most recent log lines.
#[derive(Default)]
pub struct Ring {
    lines: Mutex<VecDeque<String>>,
}

impl Ring {
    pub fn push(&self, line: &str) {
        let mut l = line.trim_end().to_string();
        if l.is_empty() {
            return;
        }
        if l.len() > MAX_LINE_BYTES {
            let mut cut = MAX_LINE_BYTES;
            while !l.is_char_boundary(cut) {
                cut -= 1;
            }
            l.truncate(cut);
            l.push('…');
        }
        let mut q = self.lines.lock().unwrap_or_else(|e| e.into_inner());
        if q.len() >= MAX_LINES {
            q.pop_front();
        }
        q.push_back(l);
    }

    pub fn lines(&self) -> Vec<String> {
        self.lines
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect()
    }
}

/// The log of this process.
pub fn ring() -> &'static Ring {
    static RING: OnceLock<Ring> = OnceLock::new();
    RING.get_or_init(Ring::default)
}

/// A writer for `tracing_subscriber` that copies every line into [`ring`] and passes it on to stderr.
#[derive(Clone, Copy, Default)]
pub struct Tee;

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        for line in text.lines() {
            let line = strip_ansi(line);
            ring().push(&line);
            crate::applog::record(&line);
        }
        std::io::stderr().write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        std::io::stderr().flush()
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Tee {
    type Writer = Tee;
    fn make_writer(&'a self) -> Tee {
        Tee
    }
}

fn strip_ansi(s: &str) -> String {
    static ANSI: OnceLock<Regex> = OnceLock::new();
    ANSI.get_or_init(|| Regex::new("\u{1b}\\[[0-9;]*m").expect("static pattern"))
        .replace_all(s, "")
        .into_owned()
}

/// What to remove from the text besides what [`crate::diagnostics::redact`] knows.
#[derive(Debug, Default, Clone)]
pub struct Scrub {
    pub home: String,
    pub user: String,
    /// The library folder: becomes `<library>`.
    pub library: String,
    /// Titles of the books: become `<title>`.
    pub titles: Vec<String>,
    /// Exact secret values (passwords, tokens): become `<secret>`.
    pub secrets: Vec<String>,
    /// Host names of the servers the user set up (sync, mail, backup, ...): become `<host>`.
    pub hosts: Vec<String>,
    /// Account names the user typed (sync user, mail login, ...): become `<account>`.
    pub accounts: Vec<String>,
}

/// A [`Scrub`] with its title pattern compiled, for text that is cleaned many times (the log file).
pub struct Scrubber {
    rules: Scrub,
    titles: Option<Regex>,
}

impl Scrubber {
    pub fn new(rules: Scrub) -> Scrubber {
        let mut titles: Vec<&String> = rules
            .titles
            .iter()
            .filter(|t| t.chars().count() >= 3)
            .collect();
        titles.sort_by_key(|t| std::cmp::Reverse(t.len()));
        titles.dedup();
        titles.truncate(MAX_TITLES);
        let titles = if titles.is_empty() {
            None
        } else {
            let alt = titles
                .iter()
                .map(|t| regex::escape(t))
                .collect::<Vec<_>>()
                .join("|");
            regex::RegexBuilder::new(&alt)
                .size_limit(32 << 20)
                .build()
                .ok()
        };
        Scrubber { rules, titles }
    }

    /// Cleans `text`. The order matters: exact secrets, hosts and the library folder go first, while
    /// they are still whole, then titles, then the general patterns.
    pub fn apply(&self, text: &str) -> String {
        let s = &self.rules;
        let mut out = text.to_string();
        let mut secrets: Vec<&String> = s.secrets.iter().filter(|v| v.len() >= 4).collect();
        secrets.sort_by_key(|v| std::cmp::Reverse(v.len()));
        for v in secrets {
            out = out.replace(v.as_str(), "<secret>");
        }
        let lib = s.library.trim_end_matches('/');
        if lib.len() > 1 {
            out = out.replace(lib, "<library>");
            // the folder's own name (`Alice's Ebooks`) names a person as well as the path does
            if let Some(name) = lib.rsplit('/').next() {
                const COMMON: &[&str] = &[
                    "books",
                    "library",
                    "ebooks",
                    "e-books",
                    "documents",
                    "downloads",
                    "calibre",
                    "bookx",
                    "bookx-library",
                ];
                if name.chars().count() >= 5
                    && !COMMON.contains(&name.to_ascii_lowercase().as_str())
                {
                    out = out.replace(name, "<library>");
                }
            }
        }
        for (list, label) in [(&s.hosts, "<host>"), (&s.accounts, "<account>")] {
            let mut v: Vec<&String> = list.iter().filter(|h| h.chars().count() >= 3).collect();
            v.sort_by_key(|h| std::cmp::Reverse(h.len()));
            for h in v {
                out = replace_ci(&out, h, label);
            }
        }
        if let Some(re) = &self.titles {
            out = re.replace_all(&out, "<title>").into_owned();
        }
        crate::diagnostics::redact(&out, &s.home, &s.user)
    }
}

/// Replaces every occurrence of `needle` (ASCII case-insensitive; host names are) in `hay`.
fn replace_ci(hay: &str, needle: &str, with: &str) -> String {
    match Regex::new(&format!("(?i){}", regex::escape(needle))) {
        Ok(re) => re.replace_all(hay, with).into_owned(),
        Err(_) => hay.to_string(),
    }
}

/// Cleans `text` once (see [`Scrubber::apply`]).
pub fn scrub(text: &str, s: &Scrub) -> String {
    Scrubber::new(s.clone()).apply(text)
}

/// The text of the Debug Information page: the report, then the recent log, all scrubbed.
pub fn compose(report: &str, log: &[String], s: &Scrub) -> String {
    let mut text = String::from(report.trim_end());
    text.push_str("\n\nRecent log:\n");
    if log.is_empty() {
        text.push_str("  (nothing logged)\n");
    }
    for l in log {
        text.push_str("  ");
        text.push_str(l);
        text.push('\n');
    }
    scrub(&text, s)
}

/// The scrub rules for the running app.
pub fn scrub_for(cfg: &crate::config::Config, lib: &crate::core::library::Library) -> Scrub {
    let mut copy = cfg.clone();
    let secrets: Vec<String> = copy
        .secret_fields()
        .into_iter()
        .map(|(_, v)| v.clone())
        .filter(|v| !v.is_empty())
        .collect();
    let (hosts, accounts) = configured_identifiers(cfg);
    Scrub {
        home: directories::BaseDirs::new()
            .map(|d| d.home_dir().display().to_string())
            .unwrap_or_default(),
        user: std::env::var("USER").unwrap_or_default(),
        library: lib.root.display().to_string(),
        titles: lib
            .books
            .iter()
            .flat_map(|b| {
                // authors and series name people and shelves as much as titles do
                std::iter::once(b.meta.title.clone())
                    .chain(
                        b.meta
                            .authors
                            .iter()
                            .filter(|a| !a.eq_ignore_ascii_case("unknown"))
                            .cloned(),
                    )
                    .chain((!b.meta.series.is_empty()).then(|| b.meta.series.clone()))
            })
            .collect(),
        secrets,
        hosts,
        accounts,
    }
}

/// The host names and account names inside the settings (read from their JSON form, so a new server
/// setting is covered without a list to keep up to date): values under keys such as `server`, `host`,
/// `url`, `user`, `email`, and the host part of every string that is a URL.
pub fn identifiers_of(settings: &serde_json::Value) -> (Vec<String>, Vec<String>) {
    fn host_of(v: &str) -> Option<String> {
        let rest = v.split_once("://").map(|(_, r)| r).unwrap_or(v);
        let rest = rest.rsplit('@').next().unwrap_or(rest);
        let h = rest.split(['/', '?', '#', ':']).next().unwrap_or("").trim();
        (h.contains('.') && !h.contains(' ') && h.len() >= 3).then(|| h.to_string())
    }
    fn walk(v: &serde_json::Value, key: &str, hosts: &mut Vec<String>, accts: &mut Vec<String>) {
        match v {
            serde_json::Value::Object(m) => {
                for (k, x) in m {
                    walk(x, k, hosts, accts);
                }
            }
            serde_json::Value::Array(a) => {
                for x in a {
                    walk(x, key, hosts, accts);
                }
            }
            serde_json::Value::String(s) if !s.is_empty() => {
                let k = key.to_ascii_lowercase();
                if s.contains("://")
                    || ["server", "host", "url", "address", "endpoint"]
                        .iter()
                        .any(|w| k.contains(w))
                {
                    // a folder path is not a host
                    if !s.starts_with('/') {
                        hosts.extend(host_of(s));
                    }
                } else if ["user", "login", "account", "email", "username"]
                    .iter()
                    .any(|w| k == *w || k.ends_with(w))
                {
                    accts.push(s.clone());
                }
            }
            _ => {}
        }
    }
    let (mut hosts, mut accts) = (Vec::new(), Vec::new());
    walk(settings, "", &mut hosts, &mut accts);
    hosts.sort();
    hosts.dedup();
    accts.sort();
    accts.dedup();
    (hosts, accts)
}

fn configured_identifiers(cfg: &crate::config::Config) -> (Vec<String>, Vec<String>) {
    serde_json::to_value(cfg)
        .map(|v| identifiers_of(&v))
        .unwrap_or_default()
}

/// The finished Debug Information text of the running app.
pub fn export(cfg: &crate::config::Config, lib: &crate::core::library::Library) -> String {
    let report = crate::diagnostics::gather(cfg, lib.books.len());
    // the report is rendered with the same rules; `compose` scrubs the whole once more
    let s = scrub_for(cfg, lib);
    compose(
        &crate::diagnostics::render(&report, &s.home, &s.user),
        &ring().lines(),
        &s,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> Scrub {
        Scrub {
            home: "/home/alice".into(),
            user: "alice".into(),
            library: "/home/alice/Books/My Library".into(),
            titles: vec![
                "Dune".into(),
                "The Left Hand of Darkness".into(),
                "A".into(),
            ],
            secrets: vec!["hunter2-pass".into(), "xy".into()],
            ..Scrub::default()
        }
    }

    #[test]
    fn secrets_titles_and_paths_never_appear() {
        let log = vec![
            "ERROR cannot open The Left Hand of Darkness in /home/alice/Books/My Library/x.epub"
                .to_string(),
            "WARN login failed with hunter2-pass for alice@example.org".to_string(),
            "INFO sync to https://alice:hunter2-pass@srv.example/path".to_string(),
            "INFO Dune finished".to_string(),
            "DEBUG token=abcDEF123 from 192.168.1.20".to_string(),
        ];
        let out = compose("Bookx 1.0", &log, &rules());
        for bad in [
            "The Left Hand",
            "hunter2-pass",
            "/home/alice",
            "alice@example.org",
            "Dune",
            "My Library",
            "192.168.1.20",
            "abcDEF123",
        ] {
            assert!(!out.contains(bad), "{bad} leaked:\n{out}");
        }
        assert!(out.contains("<title>") && out.contains("<secret>") && out.contains("<library>"));
        assert!(out.starts_with("Bookx 1.0"));
    }

    #[test]
    fn short_titles_and_secrets_are_left_alone_not_mangled() {
        let out = scrub("A xy A", &rules());
        assert_eq!(out, "A xy A");
    }

    #[test]
    fn titles_with_regex_characters_are_matched_literally() {
        let s = Scrub {
            titles: vec!["C++ (2nd ed.) [draft]".into()],
            ..Scrub::default()
        };
        assert_eq!(
            scrub("opened C++ (2nd ed.) [draft] ok", &s),
            "opened <title> ok"
        );
        assert_eq!(scrub("opened C (2nd ed)", &s), "opened C (2nd ed)");
    }

    #[test]
    fn the_ring_keeps_the_last_lines_and_cuts_long_ones() {
        let r = Ring::default();
        for i in 0..(MAX_LINES + 10) {
            r.push(&format!("line {i}"));
        }
        let l = r.lines();
        assert_eq!(l.len(), MAX_LINES);
        assert_eq!(l[0], "line 10");
        r.push(&"é".repeat(MAX_LINE_BYTES));
        assert!(r.lines().last().unwrap().len() <= MAX_LINE_BYTES + 3);
        r.push("   ");
        assert!(!r.lines().last().unwrap().trim().is_empty());
    }

    #[test]
    fn colour_codes_are_dropped() {
        assert_eq!(strip_ansi("\u{1b}[32m INFO\u{1b}[0m ok"), " INFO ok");
    }

    #[test]
    fn an_empty_log_says_so() {
        assert!(compose("r", &[], &Scrub::default()).contains("(nothing logged)"));
    }

    #[test]
    fn configured_servers_and_accounts_are_found_in_the_settings_and_removed() {
        let v = serde_json::json!({
            "kosync": {"server": "https://sync.example.org:8443/bookx", "user": "alice_reads", "password": "x"},
            "email": {"host": "smtp.mail.example", "port": 587, "user": "me@mail.example"},
            "backup": {"destinations": [{"kind": "webdav", "location": "https://cloud.example.net/dav", "user": "dav-user"},
                                          {"kind": "folder", "location": "/mnt/usb/backup"}]},
            "title": "not a host.example",
        });
        let (hosts, accts) = identifiers_of(&v);
        assert_eq!(
            hosts,
            ["cloud.example.net", "smtp.mail.example", "sync.example.org"]
        );
        assert_eq!(accts, ["alice_reads", "dav-user", "me@mail.example"]);
        let s = Scrub {
            hosts,
            accounts: accts,
            ..Scrub::default()
        };
        let out = scrub(
            "login Alice_Reads at SYNC.example.org failed; mail smtp.mail.example, dav-user@cloud.example.net",
            &s,
        );
        for bad in [
            "alice_reads",
            "sync.example",
            "smtp.mail",
            "dav-user",
            "cloud.example",
        ] {
            assert!(!out.to_lowercase().contains(bad), "{bad}: {out}");
        }
        assert!(out.contains("<host>") && out.contains("<account>"));
    }

    #[test]
    fn a_compiled_scrubber_gives_the_same_text_as_the_one_shot_function() {
        let s = Scrubber::new(rules());
        let t = "ERROR cannot open Dune for alice at /home/alice/Books/My Library/x";
        assert_eq!(s.apply(t), scrub(t, &rules()));
    }

    #[test]
    fn the_rules_for_the_running_app_come_from_the_settings_and_the_library() {
        let d = std::env::temp_dir().join(format!("bookx-dl-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        let mut lib = crate::core::library::Library::in_memory(&d.join("lib")).unwrap();
        let p = d.join("b.md");
        std::fs::write(&p, "---\ntitle: \"Zanzibar Nights\"\n---\n# A\n\nHi.\n").unwrap();
        lib.add_path(&p, true).unwrap();
        let mut cfg = crate::config::Config::default();
        cfg.share.kosync = true;
        let s = scrub_for(&cfg, &lib);
        assert_eq!(s.library, lib.root.display().to_string());
        assert_eq!(s.titles, vec!["Zanzibar Nights".to_string()]);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_tee_copies_lines_into_the_ring_without_colour() {
        use std::io::Write;
        let mut t = Tee;
        let line = "\u{1b}[32m INFO\u{1b}[0m teetest-marker-7f3a";
        t.write_all(format!("{line}\n").as_bytes()).unwrap();
        t.flush().unwrap();
        assert!(
            ring()
                .lines()
                .iter()
                .any(|l| l == " INFO teetest-marker-7f3a")
        );
        let _ = tracing_subscriber::fmt::MakeWriter::make_writer(&Tee);
    }
}
