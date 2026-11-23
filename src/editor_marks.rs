//! What the editor's gutter marks, as plain data: lines changed since the last checkpoint,
//! findings of the Check book run and misspelled-word counts. The gutter renderer in
//! `ui/editor/gutter.rs` only draws it. No GTK in here.

use crate::i18n::tr;
use std::collections::{BTreeMap, HashMap};

use bookx_editing::compare::{LineChangeKind, changed_lines};

use crate::core::check::Severity;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Change {
    Added,
    Modified,
    Removed,
}

/// Everything shown for one line (0-based).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineInfo {
    pub change: Option<Change>,
    /// The worst finding on the line.
    pub severity: Option<Severity>,
    pub findings: Vec<String>,
    pub misspelled: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Marks {
    lines: BTreeMap<u32, LineInfo>,
}

fn rank(s: Severity) -> u8 {
    match s {
        Severity::Error => 3,
        Severity::Warning => 2,
        Severity::Info => 1,
    }
}

impl Marks {
    /// Combines the three sources. `findings` are `(line, severity, text)` with 1-based lines as the
    /// checker reports them; `misspelled` counts per 0-based line.
    pub fn build(
        baseline: Option<&str>,
        current: &str,
        findings: &[(u32, Severity, String)],
        misspelled: &HashMap<u32, usize>,
    ) -> Marks {
        let mut m = Marks::default();
        if let Some(old) = baseline {
            for c in changed_lines(old, current) {
                m.lines.entry(c.line as u32).or_default().change = Some(match c.kind {
                    LineChangeKind::Added => Change::Added,
                    LineChangeKind::Modified => Change::Modified,
                    LineChangeKind::Removed => Change::Removed,
                });
            }
        }
        for (line, sev, text) in findings {
            let e = m.lines.entry(line.saturating_sub(1)).or_default();
            if e.severity.is_none_or(|s| rank(s) < rank(*sev)) {
                e.severity = Some(*sev);
            }
            e.findings.push(text.clone());
        }
        for (line, n) in misspelled {
            if *n > 0 {
                m.lines.entry(*line).or_default().misspelled = *n;
            }
        }
        m
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn get(&self, line: u32) -> Option<&LineInfo> {
        self.lines.get(&line)
    }

    pub fn changed_count(&self) -> usize {
        self.lines.values().filter(|l| l.change.is_some()).count()
    }

    pub fn finding_count(&self) -> usize {
        self.lines.values().map(|l| l.findings.len()).sum()
    }

    pub fn misspelled_total(&self) -> usize {
        self.lines.values().map(|l| l.misspelled).sum()
    }

    /// The hover text of a line, or `None` when there is nothing to say.
    pub fn tooltip(&self, line: u32) -> Option<String> {
        let l = self.lines.get(&line)?;
        let mut parts: Vec<String> = Vec::new();
        for f in &l.findings {
            parts.push(f.clone());
        }
        if l.misspelled > 0 {
            parts.push(format!(
                "{} possible spelling mistake{}",
                l.misspelled,
                if l.misspelled == 1 { "" } else { "s" }
            ));
        }
        match l.change {
            Some(Change::Added) => parts.push(tr("New since the last checkpoint")),
            Some(Change::Modified) => parts.push(tr("Changed since the last checkpoint")),
            Some(Change::Removed) => {
                parts.push(tr("Lines were removed here since the last checkpoint"))
            }
            None => {}
        }
        (!parts.is_empty()).then(|| parts.join("\n"))
    }

    /// Lines with a mark, in order (for tests and the dev hook).
    pub fn lines(&self) -> impl Iterator<Item = (u32, &LineInfo)> {
        self.lines.iter().map(|(l, i)| (*l, i))
    }
}

/// Number of misspelled words on each 0-based line, from byte offsets into `text`.
pub fn misspelled_per_line(text: &str, spots: &[(usize, usize)]) -> HashMap<u32, usize> {
    let mut starts = vec![0usize];
    starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
    let mut m: HashMap<u32, usize> = HashMap::new();
    for (s, _) in spots {
        let line = starts.partition_point(|x| x <= s) - 1;
        *m.entry(line as u32).or_default() += 1;
    }
    m
}

/// What a screen reader is told about the code view: the status line, the problems and the changed lines.
/// Nothing is said about zero problems or changes.
pub fn view_description(status: &str, findings: usize, changed: usize) -> String {
    use crate::i18n::ntr;
    let mut parts = vec![status.to_string()];
    if findings > 0 {
        parts.push(ntr("{} problem", "{} problems", findings));
    }
    if changed > 0 {
        parts.push(ntr("{} changed line", "{} changed lines", changed));
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_combine_changes_findings_and_spelling() {
        let old = "<p>a</p>\n<p>b</p>\n<p>c</p>\n";
        let new = "<p>a</p>\n<p>B</p>\n<p>c</p>\n<p>d</p>\n";
        let findings = vec![
            (2, Severity::Warning, "w".to_string()),
            (2, Severity::Error, "e".to_string()),
            (4, Severity::Info, "i".to_string()),
        ];
        let mut sp = HashMap::new();
        sp.insert(3u32, 2usize);
        sp.insert(0, 0);
        let m = Marks::build(Some(old), new, &findings, &sp);
        assert_eq!(m.get(0), None, "zero misspellings leave no mark");
        let l1 = m.get(1).unwrap();
        assert_eq!(l1.change, Some(Change::Modified));
        assert_eq!(l1.severity, Some(Severity::Error), "the worst finding wins");
        assert_eq!(l1.findings, ["w", "e"]);
        let l3 = m.get(3).unwrap();
        assert_eq!(l3.change, Some(Change::Added));
        assert_eq!(l3.misspelled, 2);
        assert_eq!(l3.severity, Some(Severity::Info));
        assert_eq!(m.changed_count(), 2);
        assert_eq!(m.finding_count(), 3);
        assert_eq!(m.misspelled_total(), 2);
        let t = m.tooltip(3).unwrap();
        assert!(
            t.contains('i')
                && t.contains("2 possible spelling mistakes")
                && t.contains("New since")
        );
        assert_eq!(m.tooltip(2), None);
    }

    #[test]
    fn no_baseline_means_no_change_marks() {
        let m = Marks::build(None, "a\nb\n", &[], &HashMap::new());
        assert!(m.is_empty());
        let same = Marks::build(Some("a\n"), "a\n", &[], &HashMap::new());
        assert!(same.is_empty());
    }

    #[test]
    fn spelling_counts_are_per_line() {
        let text = "ab cd\nef\n\ngh ij";
        let spots = [(0, 2), (3, 5), (6, 8), (12, 14)];
        let m = misspelled_per_line(text, &spots);
        assert_eq!(m.get(&0), Some(&2));
        assert_eq!(m.get(&1), Some(&1));
        assert_eq!(m.get(&2), None);
        assert_eq!(m.get(&3), Some(&1));
    }

    #[test]
    fn the_view_description_counts_problems_and_changes() {
        assert_eq!(view_description("Ln 1", 0, 0), "Ln 1");
        assert_eq!(
            view_description("Ln 1", 3, 2),
            "Ln 1, 3 problems, 2 changed lines"
        );
        assert_eq!(
            view_description("Ln 1", 1, 1),
            "Ln 1, 1 problem, 1 changed line"
        );
    }
}
