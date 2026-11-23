//! Recent problems: the warnings and errors of the log, grouped, counted and without the noise.
//!
//! Two real reports (#39, #49) were mostly the same harmless desktop-portal warning repeated hundreds of
//! times, which pushed the useful lines out of the 60 KB the report may hold. [`analyse`] turns the log
//! into a short list (one row per kind of message, with a count and the last time), [`NOISE`] names the
//! known harmless lines (they are counted, never listed), and [`fit_log`] chooses the log lines that
//! fit a byte budget by importance (errors, then breadcrumbs, then warnings; newest first), not only by
//! age.
//!
//! The text of a log line is cleaned by the caller (the scrubber), not here: nothing in this module
//! decides what may be shown, only what is worth showing.
//!
//! No GTK, no I/O.

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;

/// Problems listed.
pub const MAX_PROBLEMS: usize = 30;
/// Longest message shown in the list.
pub const MESSAGE_BYTES: usize = 260;

/// A known harmless line: `all` of the words must be in the line.
pub struct Noise {
    pub name: &'static str,
    pub all: &'static [&'static str],
    pub why: &'static str,
}

/// Lines that say nothing about a bug. They are counted ("N harmless lines left out") and never listed.
pub const NOISE: &[Noise] = &[
    Noise {
        name: "desktop portal property cache",
        all: &["Failed to populate properties cache"],
        why: "zbus warns when a short-lived portal request ends before its properties are read (seen in every report from GNOME)",
    },
    Noise {
        name: "desktop portal request proxy",
        all: &["org.freedesktop.portal.Request", "proxy"],
        why: "the same portal request proxy notice in another wording",
    },
    Noise {
        name: "portal property change streams",
        all: &["Property change streams will not produce values"],
        why: "second half of the portal property cache warning",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Warn,
    Error,
}

/// One line of the log, taken apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed<'a> {
    pub time: &'a str,
    pub level: &'a str,
    pub target: &'a str,
    pub message: String,
}

fn rx(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).expect("static pattern"))
}

/// Skips the `span{field=..}:` prefixes `tracing` puts before the target.
fn skip_spans(mut s: &str) -> &str {
    loop {
        let name_end = s
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == ':'))
            .unwrap_or(s.len());
        if !s[name_end..].starts_with('{') {
            return s;
        }
        let mut depth = 0usize;
        let mut end = None;
        for (i, c) in s[name_end..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(name_end + i + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        match end {
            Some(e) => s = s[e..].trim_start_matches(':').trim_start(),
            None => return s,
        }
    }
}

/// `2026-11-22T22:54:49.1Z  WARN <spans> target: message`.
pub fn parse(line: &str) -> Option<Parsed<'_>> {
    let mut it = line.splitn(2, char::is_whitespace);
    let time = it.next()?;
    let rest = it.next()?.trim_start();
    let (level, rest) = rest.split_once(char::is_whitespace)?;
    if !matches!(level, "WARN" | "ERROR" | "INFO" | "DEBUG" | "TRACE") {
        return None;
    }
    let rest = skip_spans(rest.trim_start());
    let (target, message) = match rest.split_once(": ") {
        Some((t, m)) if !t.contains(' ') && !t.is_empty() => (t, m),
        _ => ("", rest),
    };
    Some(Parsed {
        time,
        level,
        target,
        message: message.trim().to_string(),
    })
}

/// The message with what varies between two occurrences taken out: numbers, hashes, random names.
pub fn key_of(message: &str) -> String {
    static HEX: OnceLock<Regex> = OnceLock::new();
    static RAND: OnceLock<Regex> = OnceLock::new();
    static NUM: OnceLock<Regex> = OnceLock::new();
    let s = rx(&HEX, r"\b[0-9a-fA-F]{8,}\b").replace_all(message, "#");
    // `ashpd_Yy1MyeskkQ`, `request/1_12989/…`: a part after `_` or `/` with mixed letters and digits
    let s = rx(&RAND, r"[A-Za-z0-9]*_[A-Za-z0-9]{6,}\b").replace_all(&s, "#");
    rx(&NUM, r"\d+").replace_all(&s, "#").to_ascii_lowercase()
}

/// The known noise this line is, if any.
pub fn noise_of(line: &str) -> Option<&'static Noise> {
    NOISE
        .iter()
        .find(|n| n.all.iter().all(|w| line.contains(w)))
}

fn cut(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut c = max;
    while !s.is_char_boundary(c) {
        c -= 1;
    }
    format!("{}…", &s[..c])
}

/// One kind of problem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub level: Level,
    pub target: String,
    /// The first message of the kind (not yet cleaned by the scrubber).
    pub message: String,
    pub count: usize,
    pub first: String,
    pub last: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Analysis {
    /// Newest kind last; at most [`MAX_PROBLEMS`].
    pub problems: Vec<Problem>,
    /// `(name, lines)` of the noise left out.
    pub noise: Vec<(&'static str, &'static str, usize)>,
}

/// Groups the WARN and ERROR lines of the log.
pub fn analyse(log: &[String]) -> Analysis {
    let mut order: Vec<(Level, String)> = Vec::new();
    let mut map: HashMap<(Level, String), Problem> = HashMap::new();
    let mut noise: Vec<(&'static str, &'static str, usize)> = Vec::new();
    for l in log {
        let Some(p) = parse(l) else { continue };
        let level = match p.level {
            "WARN" => Level::Warn,
            "ERROR" => Level::Error,
            _ => continue,
        };
        if let Some(n) = noise_of(l) {
            match noise.iter_mut().find(|(name, _, _)| *name == n.name) {
                Some((_, _, c)) => *c += 1,
                None => noise.push((n.name, n.why, 1)),
            }
            continue;
        }
        let key = (level, format!("{}|{}", p.target, key_of(&p.message)));
        match map.get_mut(&key) {
            Some(e) => {
                e.count += 1;
                e.last = p.time.to_string();
            }
            None => {
                order.push(key.clone());
                map.insert(
                    key,
                    Problem {
                        level,
                        target: p.target.to_string(),
                        message: cut(&p.message, MESSAGE_BYTES),
                        count: 1,
                        first: p.time.to_string(),
                        last: p.time.to_string(),
                    },
                );
            }
        }
    }
    let mut problems: Vec<Problem> = order.into_iter().filter_map(|k| map.remove(&k)).collect();
    // newest kinds last; when too many, the oldest warnings go first, errors stay
    if problems.len() > MAX_PROBLEMS {
        let mut drop = problems.len() - MAX_PROBLEMS;
        problems.retain(|p| {
            if drop > 0 && p.level == Level::Warn {
                drop -= 1;
                false
            } else {
                true
            }
        });
        let excess = problems.len().saturating_sub(MAX_PROBLEMS);
        problems.drain(..excess);
    }
    Analysis { problems, noise }
}

/// What [`fit_log`] chose.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogPlan {
    pub lines: Vec<String>,
    /// Harmless lines left out.
    pub noise: usize,
    /// Lines left out to fit.
    pub dropped: usize,
    /// Repeats folded into the first occurrence.
    pub folded: usize,
}

fn score(line: &str) -> u8 {
    match parse(line).map(|p| p.level) {
        Some("ERROR") => 3,
        Some("INFO") => 2,
        Some("WARN") => 1,
        _ => 0,
    }
}

/// Chooses the lines of the log that fit `max_lines` and `max_bytes` (each line counts its length and a
/// newline): noise out, repeated warnings and errors folded into the first one with `[×N]`, and then, if
/// still too much, the least important lines (warnings before breadcrumbs before errors, oldest first)
/// out. The order of what stays is the order of the log.
pub fn fit_log(log: &[String], max_lines: usize, max_bytes: usize) -> LogPlan {
    let mut plan = LogPlan::default();
    // (line, score, repeats)
    let mut kept: Vec<(String, u8, usize)> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for l in log {
        if noise_of(l).is_some() {
            plan.noise += 1;
            continue;
        }
        let sc = score(l);
        if sc == 1 || sc == 3 {
            if let Some(p) = parse(l) {
                let key = format!("{}|{}|{}", p.level, p.target, key_of(&p.message));
                if let Some(&i) = seen.get(&key) {
                    kept[i].2 += 1;
                    plan.folded += 1;
                    continue;
                }
                seen.insert(key, kept.len());
            }
        }
        kept.push((l.clone(), sc, 1));
    }
    let render = |(l, _, n): &(String, u8, usize)| {
        if *n > 1 {
            format!("{l}  [×{n}]")
        } else {
            l.clone()
        }
    };
    let mut alive = vec![true; kept.len()];
    let mut lines = kept.len();
    let mut bytes: usize = kept.iter().map(|k| render(k).len() + 1).sum();
    for want in 0..=3u8 {
        if lines <= max_lines && bytes <= max_bytes {
            break;
        }
        for (i, k) in kept.iter().enumerate() {
            if lines <= max_lines && bytes <= max_bytes {
                break;
            }
            if alive[i] && k.1 == want {
                alive[i] = false;
                lines -= 1;
                bytes -= render(k).len() + 1;
                plan.dropped += 1;
            }
        }
    }
    plan.lines = kept
        .iter()
        .zip(&alive)
        .filter(|(_, a)| **a)
        .map(|(k, _)| render(k))
        .collect();
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPAM: &str = "2026-11-22T22:54:49.161170Z  WARN {}{task_name=\"org.freedesktop.portal.Request proxy caching\"}: zbus::proxy: Failed to populate properties cache via GetAll: org.freedesktop.DBus.Error.UnknownMethod: Object does not exist at path “/org/freedesktop/portal/desktop/request/1_12989/ashpd_Yy1MyeskkQ”. Property change streams will not produce values.";

    fn spam(n: usize) -> Vec<String> {
        (0..n)
            .map(|i| SPAM.replace("Yy1MyeskkQ", &format!("Ab{i:08}x")))
            .collect()
    }

    #[test]
    fn a_line_is_taken_apart_through_its_spans() {
        let p = parse(SPAM).unwrap();
        assert_eq!((p.level, p.target), ("WARN", "zbus::proxy"));
        assert!(p.message.starts_with("Failed to populate"));
        let p = parse("2026-11-22T23:00:39.8Z  WARN update_with_view{input=ShowSyncPrefs component=\"bookx::app::App\" id=\"0x7f\"}:{}{task_name=\"x y\"}: zbus::proxy: boom").unwrap();
        assert_eq!((p.target, p.message.as_str()), ("zbus::proxy", "boom"));
        let p = parse("2026-11-22T22:54:49Z  INFO bookx_crumb: start Bookx").unwrap();
        assert_eq!(
            (p.level, p.target, p.message.as_str()),
            ("INFO", "bookx_crumb", "start Bookx")
        );
        assert!(parse("not a log line").is_none());
        assert!(parse("").is_none());
    }

    #[test]
    fn the_portal_spam_of_issue_39_and_49_is_counted_not_listed() {
        let mut log = spam(300);
        log.push(
            "2026-11-23T12:00:00Z  ERROR bookx::kosync: push failed: the server answered 404"
                .into(),
        );
        let a = analyse(&log);
        assert_eq!(a.problems.len(), 1, "{:?}", a.problems);
        assert_eq!(a.problems[0].level, Level::Error);
        assert_eq!(a.noise.iter().map(|(_, _, n)| *n).sum::<usize>(), 300);
    }

    #[test]
    fn the_same_problem_with_other_numbers_is_one_row_with_a_count() {
        let log: Vec<String> = (0..5)
            .map(|i| format!("2026-11-23T12:0{i}:00Z  WARN bookx: could not read page {i} of 0123456789abcdef{i}"))
            .collect();
        let a = analyse(&log);
        assert_eq!(a.problems.len(), 1);
        assert_eq!(a.problems[0].count, 5);
        assert_eq!(a.problems[0].first, "2026-11-23T12:00:00Z");
        assert_eq!(a.problems[0].last, "2026-11-23T12:04:00Z");
    }

    #[test]
    fn different_levels_and_targets_stay_apart() {
        let log = vec![
            "t1  WARN a: same".to_string(),
            "t2  ERROR a: same".to_string(),
            "t3  WARN b: same".to_string(),
        ];
        assert_eq!(analyse(&log).problems.len(), 3);
    }

    #[test]
    fn the_list_is_bounded_and_keeps_errors() {
        let mut log: Vec<String> = (0..80)
            .map(|i| format!("t{i}  WARN w: kind-{}", "x".repeat(i + 1)))
            .collect();
        log.insert(0, "t  ERROR e: the one that matters".into());
        let a = analyse(&log);
        assert!(a.problems.len() <= MAX_PROBLEMS);
        assert!(a.problems.iter().any(|p| p.level == Level::Error));
    }

    #[test]
    fn the_log_choice_drops_noise_folds_repeats_and_keeps_errors_when_short_of_room() {
        let mut log = spam(500);
        log.push("t0  INFO bookx_crumb: start Bookx".into());
        for _ in 0..40 {
            log.push("t1  WARN bookx::x: slow thing 12".into());
        }
        log.push("t2  ERROR bookx::kosync: push failed: the server answered 404".into());
        log.push("t3  INFO bookx_crumb: sync kosync push fail http=404 class=status".into());
        let p = fit_log(&log, 200, 60_000);
        assert_eq!(p.noise, 500);
        assert_eq!(p.folded, 39);
        assert_eq!(p.lines.len(), 4);
        assert!(p.lines.iter().any(|l| l.ends_with("[×40]")));
        // not enough room: the warning goes first, then the breadcrumbs, the error stays
        let p = fit_log(&log, 2, 60_000);
        assert_eq!(p.lines.len(), 2);
        assert!(p.lines.iter().any(|l| l.contains("ERROR")));
        assert!(p.lines.iter().all(|l| !l.contains("slow thing")));
        let p = fit_log(&log, 1, 60_000);
        assert_eq!(p.lines.len(), 1);
        assert!(p.lines[0].contains("ERROR"));
        // the byte budget works too
        let p = fit_log(&log, 200, 90);
        assert!(p.lines.iter().map(|l| l.len() + 1).sum::<usize>() <= 90);
    }

    #[test]
    fn what_stays_keeps_the_order_of_the_log() {
        let log: Vec<String> = vec![
            "t1  INFO bookx_crumb: a".into(),
            "t2  ERROR x: b".into(),
            "t3  INFO bookx_crumb: c".into(),
        ];
        let p = fit_log(&log, 10, 1000);
        assert_eq!(p.lines, log);
    }

    #[test]
    fn every_noise_filter_has_a_reason_and_matches_its_own_example() {
        for n in NOISE {
            assert!(!n.why.is_empty() && !n.name.is_empty() && !n.all.is_empty());
        }
        assert_eq!(
            noise_of(SPAM).unwrap().name,
            "desktop portal property cache"
        );
        assert!(noise_of("t  WARN bookx: the library is on a network folder").is_none());
    }
}
