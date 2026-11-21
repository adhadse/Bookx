//! Small decisions of the reader that need no GTK: the auto-turn timer (R49), the warmth and
//! auto-night schedule (R60), copy with a citation (R45), link hints (R43), the page-turn
//! animation choice (R65/R59) and the key map of Bluetooth page turners (R61).

use std::collections::BTreeMap;

// ---------------------------------------------------------------------------------------------
// R49: auto-turn timer
// ---------------------------------------------------------------------------------------------

pub const AUTO_TURN_RANGE: (u32, u32) = (2, 3600);

/// An interval in seconds inside the allowed range.
pub fn clamp_auto_turn(secs: u32) -> u32 {
    secs.clamp(AUTO_TURN_RANGE.0, AUTO_TURN_RANGE.1)
}

/// What the timer wants the reader to do at a tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoTurnTick {
    Wait,
    Turn,
    /// The book ended: the timer is switched off.
    Stop,
}

/// Turns the page every `interval_ms`. The reader calls [`AutoTurn::tick`] about once a second.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoTurn {
    interval_ms: u64,
    last_ms: u64,
}

impl AutoTurn {
    pub fn new(secs: u32, now_ms: u64) -> Self {
        AutoTurn {
            interval_ms: clamp_auto_turn(secs) as u64 * 1000,
            last_ms: now_ms,
        }
    }

    pub fn secs(&self) -> u32 {
        (self.interval_ms / 1000) as u32
    }

    /// The reader moved on its own (key, tap, scroll): a full interval starts over.
    pub fn moved(&mut self, now_ms: u64) {
        self.last_ms = now_ms;
    }

    /// `busy`: a menu, a dialog or a selection is open; the time does not run then.
    pub fn tick(&mut self, now_ms: u64, at_end: bool, busy: bool) -> AutoTurnTick {
        if at_end {
            return AutoTurnTick::Stop;
        }
        if busy || now_ms < self.last_ms {
            self.last_ms = now_ms;
            return AutoTurnTick::Wait;
        }
        if now_ms - self.last_ms >= self.interval_ms {
            self.last_ms = now_ms;
            AutoTurnTick::Turn
        } else {
            AutoTurnTick::Wait
        }
    }
}

// ---------------------------------------------------------------------------------------------
// R60: warmth and auto-night schedule
// ---------------------------------------------------------------------------------------------

pub type Rgb = (f64, f64, f64);

/// `"22:30"` as minutes after midnight.
pub fn parse_hhmm(s: &str) -> Option<u16> {
    let (h, m) = s.trim().split_once(':')?;
    let (h, m): (u16, u16) = (h.trim().parse().ok()?, m.trim().parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

pub fn format_hhmm(minutes: u16) -> String {
    let m = minutes % (24 * 60);
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// Minutes over which the warmth grows before the night starts and fades after it ends.
pub const NIGHT_RAMP_MIN: u32 = 30;

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct NightSchedule {
    pub enabled: bool,
    /// Minutes after midnight at which the night starts and ends (the span may cross midnight).
    pub from: u16,
    pub to: u16,
    /// Strength of the warm tint at night, 0 (none) to 1.
    pub warmth: f64,
    /// Switch to the dark theme at night.
    pub dark: bool,
}

impl Default for NightSchedule {
    fn default() -> Self {
        NightSchedule {
            enabled: false,
            from: 21 * 60,
            to: 6 * 60,
            warmth: 0.5,
            dark: true,
        }
    }
}

impl NightSchedule {
    pub fn clamped(mut self) -> Self {
        self.from %= 24 * 60;
        self.to %= 24 * 60;
        self.warmth = if self.warmth.is_finite() {
            self.warmth.clamp(0.0, 1.0)
        } else {
            0.5
        };
        self
    }

    /// Whether `minute` (of the day) lies inside the night. `from == to` is never night.
    pub fn is_night(&self, minute: u16) -> bool {
        if !self.enabled || self.from == self.to {
            return false;
        }
        let m = minute % (24 * 60);
        if self.from < self.to {
            m >= self.from && m < self.to
        } else {
            m >= self.from || m < self.to
        }
    }

    /// The tint strength at `minute`: full at night, growing over [`NIGHT_RAMP_MIN`] before it and
    /// fading over the same time after it.
    pub fn warmth_at(&self, minute: u16) -> f64 {
        if !self.enabled || self.from == self.to {
            return 0.0;
        }
        let day = 24 * 60u32;
        let m = (minute as u32) % day;
        if self.is_night(m as u16) {
            return self.warmth.clamp(0.0, 1.0);
        }
        let until_start = (self.from as u32 + day - m) % day;
        let since_end = (m + day - self.to as u32) % day;
        let ramp = NIGHT_RAMP_MIN as f64;
        let f = if until_start < NIGHT_RAMP_MIN {
            1.0 - until_start as f64 / ramp
        } else if since_end < NIGHT_RAMP_MIN {
            1.0 - since_end as f64 / ramp
        } else {
            0.0
        };
        self.warmth.clamp(0.0, 1.0) * f
    }
}

/// A colour with the blue (and a little of the green) taken out, `amount` 0 to 1.
pub fn warm_tint(c: Rgb, amount: f64) -> Rgb {
    let a = if amount.is_finite() {
        amount.clamp(0.0, 1.0)
    } else {
        0.0
    };
    (c.0, c.1 * (1.0 - 0.25 * a), c.2 * (1.0 - 0.6 * a))
}

// ---------------------------------------------------------------------------------------------
// R45: copy with a citation
// ---------------------------------------------------------------------------------------------

pub const DEFAULT_CITATION: &str = "“{text}”\n— {author}, {title}, {chapter}";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CitationFields {
    pub text: String,
    pub title: String,
    pub author: String,
    pub chapter: String,
    pub page: String,
    pub date: String,
    pub isbn: String,
}

/// Fills `{text}`, `{title}`, `{author}`, `{chapter}`, `{page}`, `{date}` and `{isbn}` of a
/// template. A separator before an empty field goes with it (`, {page}` costs nothing when there
/// is no page); unknown `{names}` stay as they are; `{{` and `}}` are braces. The result is capped.
pub fn cite(template: &str, f: &CitationFields) -> String {
    const MAX: usize = 100_000;
    let t: &str = if template.trim().is_empty() {
        DEFAULT_CITATION
    } else {
        template
    };
    let get = |name: &str| -> Option<&str> {
        Some(match name {
            "text" => &f.text,
            "title" => &f.title,
            "author" => &f.author,
            "chapter" => &f.chapter,
            "page" => &f.page,
            "date" => &f.date,
            "isbn" => &f.isbn,
            _ => return None,
        })
    };
    let mut out = String::new();
    let cs: Vec<char> = t.chars().collect();
    let mut i = 0;
    while i < cs.len() && out.len() < MAX {
        let c = cs[i];
        if c == '{' && cs.get(i + 1) == Some(&'{') {
            out.push('{');
            i += 2;
        } else if c == '}' && cs.get(i + 1) == Some(&'}') {
            out.push('}');
            i += 2;
        } else if c == '{' {
            if let Some(end) = cs[i..].iter().position(|&x| x == '}') {
                let name: String = cs[i + 1..i + end].iter().collect();
                match get(name.trim()) {
                    Some(v) if v.trim().is_empty() => {
                        // drop the separator in front of an empty field
                        while out.ends_with(' ') {
                            out.pop();
                        }
                        if out.ends_with(',') || out.ends_with(';') || out.ends_with('—') {
                            out.pop();
                            while out.ends_with(' ') {
                                out.pop();
                            }
                        }
                        i += end + 1;
                        continue;
                    }
                    Some(v) => out.push_str(v),
                    None => out.extend(&cs[i..=i + end]),
                }
                i += end + 1;
            } else {
                out.push(c);
                i += 1;
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    out.trim_end().to_string()
}

// ---------------------------------------------------------------------------------------------
// R43: link hints
// ---------------------------------------------------------------------------------------------

/// Letters of the hints, the home row first.
pub const HINT_KEYS: &str = "asdfghjklqwertyuiopzxcvbnm";

/// `n` hints that no other hint starts with: single letters while they suffice, else all two letter
/// ones (`aa`, `as`, …) so that a typed prefix is never ambiguous. More than 676 links get none for
/// the rest.
pub fn hint_labels(n: usize) -> Vec<String> {
    let keys: Vec<char> = HINT_KEYS.chars().collect();
    if n <= keys.len() {
        return keys.iter().take(n).map(|c| c.to_string()).collect();
    }
    let mut v = Vec::new();
    for a in &keys {
        for b in &keys {
            if v.len() == n {
                return v;
            }
            v.push(format!("{a}{b}"));
        }
    }
    v
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HintMatch {
    /// No hint starts with what was typed.
    None,
    /// These hint indexes still match.
    Some(Vec<usize>),
    /// Exactly this link is chosen.
    Hit(usize),
}

/// What typing `typed` selects among `labels`.
pub fn match_hint(labels: &[String], typed: &str) -> HintMatch {
    let t = typed.to_lowercase();
    if t.is_empty() {
        return HintMatch::Some((0..labels.len()).collect());
    }
    if let Some(i) = labels.iter().position(|l| *l == t) {
        return HintMatch::Hit(i);
    }
    let m: Vec<usize> = labels
        .iter()
        .enumerate()
        .filter(|(_, l)| l.starts_with(&t))
        .map(|(i, _)| i)
        .collect();
    if m.is_empty() {
        HintMatch::None
    } else {
        HintMatch::Some(m)
    }
}

// ---------------------------------------------------------------------------------------------
// R25: skim preview
// ---------------------------------------------------------------------------------------------

/// The chapter (index into `weights`, the share of the book each one has) that the position
/// `frac` (0 to 1) of the progress slider falls in.
pub fn skim_chapter(weights: &[f64], frac: f64) -> usize {
    let total: f64 = weights.iter().filter(|w| **w > 0.0).sum();
    if weights.is_empty() || total <= 0.0 || !frac.is_finite() {
        return 0;
    }
    let target = frac.clamp(0.0, 1.0) * total;
    let mut acc = 0.0;
    for (i, w) in weights.iter().enumerate() {
        acc += w.max(0.0);
        if target < acc {
            return i;
        }
    }
    weights.len() - 1
}

// ---------------------------------------------------------------------------------------------
// R65 / R59: page-turn animation
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum PageAnimation {
    #[default]
    None,
    Slide,
    Fade,
}

impl PageAnimation {
    pub fn key(self) -> &'static str {
        match self {
            PageAnimation::None => "none",
            PageAnimation::Slide => "slide",
            PageAnimation::Fade => "fade",
        }
    }

    pub fn from_key(s: &str) -> PageAnimation {
        match s {
            "slide" => PageAnimation::Slide,
            "fade" => PageAnimation::Fade,
            _ => PageAnimation::None,
        }
    }
}

/// The animation that is really played: none in e-ink mode and when the system asks for reduced
/// motion, whatever the setting says.
pub fn effective_animation(
    choice: PageAnimation,
    reduced_motion: bool,
    eink: bool,
) -> PageAnimation {
    if reduced_motion || eink {
        PageAnimation::None
    } else {
        choice
    }
}

pub const PAGE_ANIMATION_MS: u64 = 220;

/// Ease-out progress, 0 to 1, after `elapsed_ms` of an animation of `total_ms`.
pub fn ease_out(elapsed_ms: u64, total_ms: u64) -> f64 {
    if total_ms == 0 || elapsed_ms >= total_ms {
        return 1.0;
    }
    let t = elapsed_ms as f64 / total_ms as f64;
    1.0 - (1.0 - t) * (1.0 - t) * (1.0 - t)
}

// ---------------------------------------------------------------------------------------------
// R61: keys of a Bluetooth page turner
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RemoteAction {
    NextPage,
    PrevPage,
    NextChapter,
    PrevChapter,
    ToggleChrome,
}

impl RemoteAction {
    pub fn key(self) -> &'static str {
        match self {
            RemoteAction::NextPage => "next-page",
            RemoteAction::PrevPage => "prev-page",
            RemoteAction::NextChapter => "next-chapter",
            RemoteAction::PrevChapter => "prev-chapter",
            RemoteAction::ToggleChrome => "toggle-chrome",
        }
    }

    pub fn from_key(s: &str) -> Option<RemoteAction> {
        Some(match s.trim() {
            "next-page" => RemoteAction::NextPage,
            "prev-page" => RemoteAction::PrevPage,
            "next-chapter" => RemoteAction::NextChapter,
            "prev-chapter" => RemoteAction::PrevChapter,
            "toggle-chrome" => RemoteAction::ToggleChrome,
            _ => return None,
        })
    }
}

/// Keys that page turners and presentation remotes send (GDK key names). Arrow keys, Page Up/Down
/// and Space are the reader's own and are not listed here.
pub const REMOTE_DEFAULTS: &[(&str, RemoteAction)] = &[
    ("AudioNext", RemoteAction::NextPage),
    ("AudioPrev", RemoteAction::PrevPage),
    ("AudioForward", RemoteAction::NextPage),
    ("AudioRewind", RemoteAction::PrevPage),
    ("KP_Enter", RemoteAction::NextPage),
    ("KP_Add", RemoteAction::NextPage),
    ("KP_Subtract", RemoteAction::PrevPage),
    ("KP_Page_Down", RemoteAction::NextPage),
    ("KP_Page_Up", RemoteAction::PrevPage),
    ("F7", RemoteAction::PrevPage),
    ("F8", RemoteAction::NextPage),
    ("Forward", RemoteAction::NextPage),
    ("Back", RemoteAction::PrevPage),
];

/// The remote key map: the defaults with the user's lines laid over them. A line is
/// `KeyName=action`; `KeyName=` (no action) frees a default key. Bad lines are skipped.
pub fn remote_map(user: &[String]) -> BTreeMap<String, RemoteAction> {
    let mut m: BTreeMap<String, RemoteAction> = REMOTE_DEFAULTS
        .iter()
        .map(|(k, a)| (k.to_string(), *a))
        .collect();
    for line in user.iter().take(64) {
        let Some((k, a)) = line.split_once('=') else {
            continue;
        };
        // GDK names the media keys without the X11 prefix
        let k = k.trim();
        let k = k.strip_prefix("XF86").unwrap_or(k);
        if k.is_empty() || k.len() > 40 || k.chars().any(|c| c.is_control() || c.is_whitespace()) {
            continue;
        }
        if a.trim().is_empty() {
            m.remove(k);
        } else if let Some(act) = RemoteAction::from_key(a) {
            m.insert(k.to_string(), act);
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_turn_waits_turns_and_stops() {
        let mut t = AutoTurn::new(10, 0);
        assert_eq!(t.tick(5_000, false, false), AutoTurnTick::Wait);
        assert_eq!(t.tick(10_000, false, false), AutoTurnTick::Turn);
        assert_eq!(t.tick(12_000, false, false), AutoTurnTick::Wait);
        assert_eq!(t.tick(20_000, false, false), AutoTurnTick::Turn);
        assert_eq!(t.tick(21_000, true, false), AutoTurnTick::Stop);
    }

    #[test]
    fn auto_turn_does_not_run_while_busy_or_after_a_manual_move() {
        let mut t = AutoTurn::new(10, 0);
        assert_eq!(t.tick(9_000, false, true), AutoTurnTick::Wait);
        // the busy time did not count: a full interval from the last busy tick
        assert_eq!(t.tick(18_000, false, false), AutoTurnTick::Wait);
        assert_eq!(t.tick(19_000, false, false), AutoTurnTick::Turn);
        t.moved(25_000);
        assert_eq!(t.tick(30_000, false, false), AutoTurnTick::Wait);
        assert_eq!(t.tick(35_000, false, false), AutoTurnTick::Turn);
        // a clock that goes back does not turn pages
        assert_eq!(t.tick(1_000, false, false), AutoTurnTick::Wait);
    }

    #[test]
    fn auto_turn_interval_is_clamped() {
        assert_eq!(AutoTurn::new(0, 0).secs(), 2);
        assert_eq!(AutoTurn::new(u32::MAX, 0).secs(), 3600);
        assert_eq!(clamp_auto_turn(45), 45);
    }

    #[test]
    fn skimming_finds_the_chapter_under_the_slider() {
        let w = [1.0, 2.0, 1.0];
        assert_eq!(skim_chapter(&w, 0.0), 0);
        assert_eq!(skim_chapter(&w, 0.24), 0);
        assert_eq!(skim_chapter(&w, 0.26), 1);
        assert_eq!(skim_chapter(&w, 0.74), 1);
        assert_eq!(skim_chapter(&w, 0.76), 2);
        assert_eq!(skim_chapter(&w, 1.0), 2);
        assert_eq!(skim_chapter(&w, 7.0), 2);
        assert_eq!(skim_chapter(&w, f64::NAN), 0);
        assert_eq!(skim_chapter(&[], 0.5), 0);
        assert_eq!(skim_chapter(&[0.0, 0.0], 0.5), 0);
    }

    #[test]
    fn times_parse_and_print() {
        assert_eq!(parse_hhmm("22:30"), Some(22 * 60 + 30));
        assert_eq!(parse_hhmm(" 6:05 "), Some(365));
        assert_eq!(parse_hhmm("24:00"), None);
        assert_eq!(parse_hhmm("12:60"), None);
        assert_eq!(parse_hhmm("noon"), None);
        assert_eq!(parse_hhmm("-1:00"), None);
        assert_eq!(format_hhmm(365), "06:05");
        assert_eq!(format_hhmm(24 * 60 + 5), "00:05");
    }

    fn night(from: &str, to: &str) -> NightSchedule {
        NightSchedule {
            enabled: true,
            from: parse_hhmm(from).unwrap(),
            to: parse_hhmm(to).unwrap(),
            warmth: 0.8,
            dark: true,
        }
    }

    #[test]
    fn a_night_that_crosses_midnight() {
        let n = night("21:00", "06:00");
        assert!(n.is_night(22 * 60));
        assert!(n.is_night(0));
        assert!(n.is_night(5 * 60 + 59));
        assert!(!n.is_night(6 * 60));
        assert!(!n.is_night(12 * 60));
        assert!(n.is_night(21 * 60));
    }

    #[test]
    fn a_night_inside_one_day_and_the_empty_ones() {
        let n = night("13:00", "15:00");
        assert!(n.is_night(14 * 60));
        assert!(!n.is_night(16 * 60));
        assert!(!night("07:00", "07:00").is_night(7 * 60));
        let mut off = night("21:00", "06:00");
        off.enabled = false;
        assert!(!off.is_night(23 * 60));
        assert_eq!(off.warmth_at(23 * 60), 0.0);
    }

    #[test]
    fn the_warmth_ramps_before_and_after_the_night() {
        let n = night("21:00", "06:00");
        assert_eq!(n.warmth_at(23 * 60), 0.8);
        assert_eq!(n.warmth_at(12 * 60), 0.0);
        let before = n.warmth_at(20 * 60 + 45);
        assert!(before > 0.0 && before < 0.8, "{before}");
        let after = n.warmth_at(6 * 60 + 15);
        assert!(after > 0.0 && after < 0.8, "{after}");
        assert!(n.warmth_at(20 * 60 + 59) > n.warmth_at(20 * 60 + 35));
        assert_eq!(n.warmth_at(6 * 60 + 30), 0.0);
    }

    #[test]
    fn schedule_values_are_clamped() {
        let n = NightSchedule {
            enabled: true,
            from: 5000,
            to: 3000,
            warmth: f64::NAN,
            dark: false,
        }
        .clamped();
        assert!(n.from < 1440 && n.to < 1440);
        assert_eq!(n.warmth, 0.5);
        assert_eq!(NightSchedule { warmth: 7.0, ..n }.clamped().warmth, 1.0);
    }

    #[test]
    fn schedule_survives_a_json_round_trip_and_old_files() {
        let n = night("20:00", "07:30");
        let s = serde_json::to_string(&n).unwrap();
        assert_eq!(serde_json::from_str::<NightSchedule>(&s).unwrap(), n);
        let d: NightSchedule = serde_json::from_str("{}").unwrap();
        assert!(!d.enabled);
    }

    #[test]
    fn warm_tint_removes_blue_first() {
        let c = warm_tint((1.0, 1.0, 1.0), 1.0);
        assert_eq!(c.0, 1.0);
        assert!(c.1 < 1.0 && c.2 < c.1);
        assert_eq!(warm_tint((0.5, 0.5, 0.5), 0.0), (0.5, 0.5, 0.5));
        assert_eq!(warm_tint((1.0, 1.0, 1.0), f64::NAN), (1.0, 1.0, 1.0));
    }

    fn fields() -> CitationFields {
        CitationFields {
            text: "Call me Ishmael.".into(),
            title: "Moby-Dick".into(),
            author: "Herman Melville".into(),
            chapter: "Loomings".into(),
            page: "12".into(),
            date: "2026-11-21".into(),
            isbn: String::new(),
        }
    }

    #[test]
    fn the_default_citation() {
        assert_eq!(
            cite("", &fields()),
            "“Call me Ishmael.”\n— Herman Melville, Moby-Dick, Loomings"
        );
    }

    #[test]
    fn a_template_fills_every_field() {
        let t = "{text} ({author}, {title}, p. {page}, {date})";
        assert_eq!(
            cite(t, &fields()),
            "Call me Ishmael. (Herman Melville, Moby-Dick, p. 12, 2026-11-21)"
        );
    }

    #[test]
    fn an_empty_field_takes_its_separator_along() {
        let mut f = fields();
        f.chapter.clear();
        assert_eq!(
            cite("{text} — {author}, {title}, {chapter}", &f),
            "Call me Ishmael. — Herman Melville, Moby-Dick"
        );
        assert_eq!(cite("{text} {isbn}", &f), "Call me Ishmael.");
    }

    #[test]
    fn unknown_names_and_braces_stay() {
        assert_eq!(cite("{nope} {{x}} {text", &fields()), "{nope} {x} {text");
    }

    #[test]
    fn a_citation_is_capped() {
        let mut f = fields();
        f.text = "x".repeat(1_000_000);
        let t = "{text}{text}{text}";
        assert!(cite(t, &f).len() <= 1_000_001 * 3);
        let many = "{text}".repeat(1000);
        assert!(cite(&many, &f).len() < 3_000_000);
    }

    #[test]
    fn hint_labels_are_prefix_free() {
        for n in [0, 1, 5, 26, 27, 100, 676, 900] {
            let l = hint_labels(n);
            assert_eq!(l.len(), n.min(if n <= 26 { n } else { 676 }));
            for (i, a) in l.iter().enumerate() {
                for (j, b) in l.iter().enumerate() {
                    assert!(i == j || !b.starts_with(a.as_str()), "{a} {b}");
                }
            }
        }
        assert_eq!(hint_labels(3), vec!["a", "s", "d"]);
        assert_eq!(hint_labels(30)[0], "aa");
    }

    #[test]
    fn typing_narrows_the_hints_and_picks_one() {
        let l = hint_labels(30);
        assert_eq!(match_hint(&l, ""), HintMatch::Some((0..30).collect()));
        match match_hint(&l, "a") {
            HintMatch::Some(v) => assert_eq!(v.len(), 26),
            m => panic!("{m:?}"),
        }
        assert_eq!(match_hint(&l, "as"), HintMatch::Hit(1));
        assert_eq!(match_hint(&l, "AS"), HintMatch::Hit(1));
        assert_eq!(match_hint(&l, "1"), HintMatch::None);
        let s = hint_labels(4);
        assert_eq!(match_hint(&s, "f"), HintMatch::Hit(3));
    }

    #[test]
    fn animation_is_off_for_reduced_motion_and_eink() {
        assert_eq!(
            effective_animation(PageAnimation::Slide, false, false),
            PageAnimation::Slide
        );
        assert_eq!(
            effective_animation(PageAnimation::Slide, true, false),
            PageAnimation::None
        );
        assert_eq!(
            effective_animation(PageAnimation::Fade, false, true),
            PageAnimation::None
        );
        assert_eq!(PageAnimation::from_key("slide"), PageAnimation::Slide);
        assert_eq!(PageAnimation::from_key("junk"), PageAnimation::None);
        for a in [
            PageAnimation::None,
            PageAnimation::Slide,
            PageAnimation::Fade,
        ] {
            assert_eq!(PageAnimation::from_key(a.key()), a);
        }
    }

    #[test]
    fn easing_goes_from_zero_to_one() {
        assert_eq!(ease_out(0, 200), 0.0);
        assert_eq!(ease_out(200, 200), 1.0);
        assert_eq!(ease_out(500, 200), 1.0);
        assert_eq!(ease_out(1, 0), 1.0);
        let mut last = 0.0;
        for t in (0..=200).step_by(10) {
            let v = ease_out(t, 200);
            assert!(v >= last);
            last = v;
        }
        assert!(ease_out(100, 200) > 0.5, "starts fast");
    }

    #[test]
    fn remote_defaults_and_user_lines() {
        let m = remote_map(&[]);
        assert_eq!(m.get("AudioNext"), Some(&RemoteAction::NextPage));
        assert_eq!(m.get("F7"), Some(&RemoteAction::PrevPage));
        let m = remote_map(&[
            "F7=next-chapter".into(),
            "F8=".into(),
            "Menu=toggle-chrome".into(),
            "bad line".into(),
            "X=unknown".into(),
            "=next-page".into(),
            "a b=next-page".into(),
            "XF86AudioRewind=next-chapter".into(),
        ]);
        assert_eq!(m.get("AudioRewind"), Some(&RemoteAction::NextChapter));
        assert_eq!(m.get("F7"), Some(&RemoteAction::NextChapter));
        assert!(!m.contains_key("F8"));
        assert_eq!(m.get("Menu"), Some(&RemoteAction::ToggleChrome));
        assert!(!m.contains_key("X"));
        assert!(!m.contains_key("a b"));
    }

    #[test]
    fn remote_actions_round_trip() {
        for a in [
            RemoteAction::NextPage,
            RemoteAction::PrevPage,
            RemoteAction::NextChapter,
            RemoteAction::PrevChapter,
            RemoteAction::ToggleChrome,
        ] {
            assert_eq!(RemoteAction::from_key(a.key()), Some(a));
        }
        let many: Vec<String> = (0..1000).map(|i| format!("K{i}=next-page")).collect();
        assert!(remote_map(&many).len() < 100, "only 64 user lines count");
    }
}
