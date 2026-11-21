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
            ring().push(&strip_ansi(line));
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
}

/// Cleans `text`. The order matters: exact secrets and the library folder go first, while they are still
/// whole, then titles, then the general patterns.
pub fn scrub(text: &str, s: &Scrub) -> String {
    let mut out = text.to_string();
    let mut secrets: Vec<&String> = s.secrets.iter().filter(|v| v.len() >= 4).collect();
    secrets.sort_by_key(|v| std::cmp::Reverse(v.len()));
    for v in secrets {
        out = out.replace(v.as_str(), "<secret>");
    }
    let lib = s.library.trim_end_matches('/');
    if lib.len() > 1 {
        out = out.replace(lib, "<library>");
    }
    let mut titles: Vec<&String> = s.titles.iter().filter(|t| t.chars().count() >= 3).collect();
    titles.sort_by_key(|t| std::cmp::Reverse(t.len()));
    titles.dedup();
    titles.truncate(MAX_TITLES);
    if !titles.is_empty() {
        let alt = titles
            .iter()
            .map(|t| regex::escape(t))
            .collect::<Vec<_>>()
            .join("|");
        if let Ok(re) = regex::RegexBuilder::new(&alt).size_limit(32 << 20).build() {
            out = re.replace_all(&out, "<title>").into_owned();
        }
    }
    crate::diagnostics::redact(&out, &s.home, &s.user)
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
    Scrub {
        home: directories::BaseDirs::new()
            .map(|d| d.home_dir().display().to_string())
            .unwrap_or_default(),
        user: std::env::var("USER").unwrap_or_default(),
        library: lib.root.display().to_string(),
        titles: lib.books.iter().map(|b| b.meta.title.clone()).collect(),
        secrets,
    }
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
}
