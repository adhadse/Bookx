//! "Summary and suspected area": the rule table that reads the report and says what probably went wrong.
//!
//! The rules are data, `build-aux/triage-rules.json`, shared with `build-aux/triage-report.py` (which a
//! maintainer runs on a report pasted into an issue): **one table, two readers**. A rule is a set of
//! regular expressions over the *text* of the report (the plain subset that Rust's `regex` and Python's
//! `re` both read), so a finding can never depend on something the maintainer cannot see in the report.
//!
//! A rule matches when every `all` pattern matches, one of the `any` patterns matches (if there are any)
//! and no `none` pattern matches. Named groups of the patterns fill `{name}` in the cause. The headline
//! is the first matching rule about something that *happened* (an operation failed, a crash, a database
//! message) before the rules about *state* (the library is on a network disk): a sync bug on a network
//! library is still a sync bug first. Each rule carries the evidence lines to quote, a hint, and five
//! things to ask the user when the report cannot tell.
//!
//! No GTK, no I/O.

use std::sync::OnceLock;

use regex::Regex;
use serde::Deserialize;

const RULES_JSON: &str = include_str!("../build-aux/triage-rules.json");

#[derive(Debug, Deserialize)]
struct RuleSet {
    area_labels: std::collections::BTreeMap<String, String>,
    generic_asks: Vec<String>,
    rules: Vec<RuleDef>,
}

#[derive(Debug, Deserialize)]
struct RuleDef {
    id: String,
    area: String,
    severity: String,
    #[serde(default)]
    state: bool,
    #[serde(default)]
    all: Vec<String>,
    #[serde(default)]
    any: Vec<String>,
    #[serde(default)]
    none: Vec<String>,
    cause: String,
    hint: String,
    #[serde(default)]
    asks: Vec<String>,
    #[serde(default)]
    evidence: Vec<String>,
}

struct Rule {
    def: RuleDef,
    all: Vec<Regex>,
    any: Vec<Regex>,
    none: Vec<Regex>,
    evidence: Vec<Regex>,
}

struct Compiled {
    area_labels: std::collections::BTreeMap<String, String>,
    generic_asks: Vec<String>,
    rules: Vec<Rule>,
}

fn compiled() -> &'static Compiled {
    static C: OnceLock<Compiled> = OnceLock::new();
    C.get_or_init(|| {
        let set: RuleSet = serde_json::from_str(RULES_JSON).expect("triage-rules.json is valid");
        let re = |s: &String| Regex::new(s).unwrap_or_else(|e| panic!("bad rule pattern {s}: {e}"));
        Compiled {
            area_labels: set.area_labels,
            generic_asks: set.generic_asks,
            rules: set
                .rules
                .into_iter()
                .map(|d| Rule {
                    all: d.all.iter().map(re).collect(),
                    any: d.any.iter().map(re).collect(),
                    none: d.none.iter().map(re).collect(),
                    evidence: d.evidence.iter().map(re).collect(),
                    def: d,
                })
                .collect(),
        }
    })
}

/// How serious a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Crash,
    DataRisk,
    Annoyance,
}

impl Severity {
    pub fn word(self) -> &'static str {
        match self {
            Severity::Crash => "crash",
            Severity::DataRisk => "data risk",
            Severity::Annoyance => "annoyance",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub id: String,
    pub area: String,
    pub severity: Severity,
    pub state: bool,
    pub cause: String,
    pub hint: String,
    pub evidence: Vec<String>,
    pub asks: Vec<String>,
}

/// The part of a report the rules read: without an earlier summary block and without the user's own
/// words (which can say anything and must not steer the rules).
pub fn rule_input(report: &str) -> String {
    let mut text = report.to_string();
    if let Some(i) = text.find("### What happened") {
        text.truncate(i);
    }
    if let Some(i) = text.find("### Summary and suspected area") {
        let rest = &text[i + 3..];
        let end = rest.find("\n### ").map_or(text.len(), |j| i + 3 + j + 1);
        text.replace_range(i..end, "");
    }
    text
}

fn fill(template: &str, caps: &std::collections::BTreeMap<String, String>) -> String {
    let mut out = template.to_string();
    for (k, v) in caps {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    // a placeholder nobody filled is dropped, not shown
    static LEFT: OnceLock<Regex> = OnceLock::new();
    let left = LEFT.get_or_init(|| Regex::new(r" ?\{[a-z_]+\}").expect("static pattern"));
    left.replace_all(&out, "").into_owned()
}

/// Every rule that matches the report, in the order of the table.
pub fn analyse(report: &str) -> Vec<Finding> {
    let text = rule_input(report);
    let c = compiled();
    let mut out = Vec::new();
    for r in &c.rules {
        let mut caps = std::collections::BTreeMap::new();
        let mut take = |re: &Regex| -> bool {
            match re.captures(&text) {
                Some(m) => {
                    for n in re.capture_names().flatten() {
                        if let Some(v) = m.name(n) {
                            caps.entry(n.to_string())
                                .or_insert_with(|| v.as_str().trim().to_string());
                        }
                    }
                    true
                }
                None => false,
            }
        };
        if !r.all.iter().all(&mut take) {
            continue;
        }
        if !r.any.is_empty() && !r.any.iter().any(&mut take) {
            continue;
        }
        if r.none.iter().any(|n| n.is_match(&text)) {
            continue;
        }
        let evidence: Vec<String> = r
            .evidence
            .iter()
            .filter_map(|e| e.find(&text).map(|m| m.as_str().trim().to_string()))
            .filter(|s| !s.is_empty())
            .take(3)
            .collect();
        let mut asks = r.def.asks.clone();
        for a in &c.generic_asks {
            if asks.len() >= 5 {
                break;
            }
            if !asks.contains(a) {
                asks.push(a.clone());
            }
        }
        out.push(Finding {
            id: r.def.id.clone(),
            area: if r.def.area == "@area" {
                caps.get("area")
                    .and_then(|w| c.area_labels.get(w))
                    .cloned()
                    .unwrap_or_else(|| "Network".to_string())
            } else {
                r.def.area.clone()
            },
            severity: match r.def.severity.as_str() {
                "crash" => Severity::Crash,
                "data" => Severity::DataRisk,
                _ => Severity::Annoyance,
            },
            state: r.def.state,
            cause: fill(&r.def.cause, &caps),
            hint: r.def.hint.clone(),
            evidence,
            asks,
        });
    }
    out
}

/// The finding to put first: what happened before what is merely the state of things.
pub fn headline(findings: &[Finding]) -> Option<&Finding> {
    findings
        .iter()
        .find(|f| !f.state)
        .or_else(|| findings.first())
}

/// `### Summary and suspected area` for the top of the report. `entry` is the area the user pressed
/// Report in (if known); the text of the findings is cleaned by the caller before it is shown.
pub fn summary_block(findings: &[Finding], entry: Option<&str>) -> String {
    let mut s = String::from("### Summary and suspected area\n\n");
    match headline(findings) {
        Some(h) => {
            s.push_str(&format!("- Suspected area: {}\n", h.area));
            s.push_str(&format!("- Severity: {}\n", h.severity.word()));
            s.push_str(&format!("- What went wrong: {}\n", h.cause));
            s.push_str(&format!("- Hint: {}\n", h.hint));
            for e in &h.evidence {
                s.push_str(&format!("- Evidence: `{}`\n", e.replace('`', "'")));
            }
            let mut seen = vec![h.area.clone()];
            let also: Vec<String> = findings
                .iter()
                .filter(|f| {
                    if seen.contains(&f.area) {
                        false
                    } else {
                        seen.push(f.area.clone());
                        true
                    }
                })
                .take(4)
                .map(|f| format!("{} ({})", f.area, f.cause.trim_end_matches('.')))
                .collect();
            if !also.is_empty() {
                s.push_str(&format!("- Also noticed: {}\n", also.join("; ")));
            }
        }
        None => {
            s.push_str("- Suspected area: not clear from the facts below (nothing failed that Bookx could see)\n");
            s.push_str("- Hint: the description under \"What happened\" is the main clue\n");
        }
    }
    if let Some(a) = entry {
        s.push_str(&format!("- Reported from: {}\n", a.replace('|', ", ")));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area_of(report: &str) -> String {
        headline(&analyse(report))
            .map(|f| f.area.clone())
            .unwrap_or_default()
    }

    #[test]
    fn the_table_loads_and_every_rule_is_complete() {
        let c = compiled();
        assert!(c.rules.len() >= 40);
        let mut ids = std::collections::BTreeSet::new();
        for r in &c.rules {
            assert!(ids.insert(r.def.id.clone()), "duplicate {}", r.def.id);
            assert!(
                !r.all.is_empty() || !r.any.is_empty(),
                "{} matches everything",
                r.def.id
            );
            assert!(
                !r.def.cause.is_empty() && !r.def.hint.is_empty(),
                "{}",
                r.def.id
            );
            assert!(r.def.asks.len() >= 3, "{} needs questions", r.def.id);
            assert!(!r.evidence.is_empty(), "{} quotes nothing", r.def.id);
            assert!(["crash", "data", "annoyance"].contains(&r.def.severity.as_str()));
        }
        assert_eq!(c.generic_asks.len(), 5);
    }

    #[test]
    fn the_area_labels_of_the_table_are_the_areas_of_the_recorder() {
        let c = compiled();
        for a in crate::trace::Area::ALL {
            assert_eq!(
                c.area_labels.get(a.word()).map(String::as_str),
                Some(a.label()),
                "{}",
                a.word()
            );
        }
        assert_eq!(c.area_labels.len(), crate::trace::Area::ALL.len());
    }

    #[test]
    fn a_generic_rule_names_the_area_that_failed() {
        let f = analyse("- 12:00:00Z backup/job FAIL class=timeout 30000 ms\n");
        assert_eq!(headline(&f).unwrap().area, "Backups");
        let f = analyse("- 12:00:00Z opds/request FAIL class=dns 5 ms host=domain\n");
        assert_eq!(headline(&f).unwrap().area, "OPDS and web import");
    }

    #[test]
    fn a_404_on_push_names_sync_and_quotes_the_line() {
        let r = "### Sync\n\n- Server address: http://<ip:private>:30260/api/koreader\n- Document matching: partial-md5\n\n### Recent operations\n\n- 12:03:11Z sync/push FAIL http=404 class=status 120 ms fmt=pdf doc=0123456789abcdef0123456789abcdef err=\"the server answered 404\"\n";
        let f = analyse(r);
        let h = headline(&f).unwrap();
        assert_eq!(h.id, "sync-404-push");
        assert_eq!(h.area, "Sync (KOSync client)");
        assert!(
            h.evidence
                .iter()
                .any(|e| e.contains("sync/push FAIL http=404"))
        );
        assert!(h.evidence.iter().any(|e| e.contains("/api/koreader")));
        assert_eq!(h.asks.len(), 5);
    }

    #[test]
    fn an_event_beats_a_state() {
        let r = "- File system kind: NETWORK file system\n- 12:00:00Z sync/pull FAIL class=timeout 10000 ms\n";
        let f = analyse(r);
        assert!(f.iter().any(|f| f.id == "library-network-fs"));
        assert_eq!(headline(&f).unwrap().id, "network-timeout");
        assert_eq!(
            area_of("- File system kind: NETWORK file system\n"),
            "Library folder"
        );
    }

    #[test]
    fn named_groups_fill_the_cause_and_unfilled_ones_vanish() {
        let f =
            analyse("- 12:00:00Z devices/copy FAIL class=not_mounted 5 ms device=Boox_Leaf_2\n");
        let h = headline(&f).unwrap();
        assert_eq!(h.id, "device-not-mounted");
        assert!(!h.cause.contains('{'), "{}", h.cause);
        let f = analyse("- 12:00:00Z network/x FAIL class=dns\n");
        assert!(headline(&f).unwrap().cause.starts_with("network/x:"));
    }

    #[test]
    fn the_users_words_and_an_older_summary_do_not_steer_the_rules() {
        let r = "### Summary and suspected area\n\n- Suspected area: X\n- Evidence: sync/push FAIL http=404\n\n### Bookx\n\n- Version: 1\n\n### What happened\n\nsync/push FAIL http=404 class=status\n";
        assert!(analyse(r).is_empty(), "{:?}", analyse(r));
        let text = rule_input(r);
        assert!(text.contains("### Bookx") && !text.contains("Suspected"));
    }

    #[test]
    fn nothing_failed_says_so_and_still_names_where_the_report_was_opened() {
        let s = summary_block(&[], Some("Sync window"));
        assert!(s.contains("not clear from the facts") && s.contains("Reported from: Sync window"));
    }

    #[test]
    fn the_summary_block_has_area_severity_cause_hint_evidence() {
        let f = analyse("- 12:00:00Z server/start FAIL class=port_in_use 3 ms port=30260\n");
        let s = summary_block(&f, None);
        for want in [
            "Suspected area: Built-in server",
            "Severity: annoyance",
            "What went wrong:",
            "Hint:",
            "Evidence: `",
        ] {
            assert!(s.contains(want), "{want} in {s}");
        }
    }

    #[test]
    fn a_pasted_terminal_error_names_a_package_built_for_another_release() {
        let r = "bookx: /lib64/libc.so.6: version `GLIBC_2.43' not found (required by bookx)\n";
        let f = analyse(r);
        assert_eq!(headline(&f).unwrap().id, "package-binary");
        assert_eq!(headline(&f).unwrap().severity, Severity::Crash);
    }
}
