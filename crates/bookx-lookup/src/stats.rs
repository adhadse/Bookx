//! Reading statistics: sessions, daily/weekly/monthly totals, streaks, speed,
//! time-remaining estimates, goals and a calendar heatmap.
//!
//! Instants are stored in UTC; everything calendar-related takes a `chrono::TimeZone`
//! and honours a configurable "day starts at" offset (reading at 01:30 with a 04:00
//! day start belongs to the previous day).

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Datelike, Duration, LocalResult, NaiveDate, TimeZone, Utc, Weekday};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

/// Speeds above this (words per minute) are treated as skimming and ignored.
pub const MAX_PLAUSIBLE_WPM: f64 = 1500.0;
/// Sessions shorter than this do not contribute to the speed estimate.
pub const MIN_SPEED_SESSION_SECS: i64 = 30;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatsConfig {
    /// Sessions shorter than this are dropped.
    pub min_session_secs: i64,
    /// Sessions of the same book separated by at most this gap are merged.
    pub merge_gap_secs: i64,
    /// Minutes after local midnight at which a new reading day starts (0..=720).
    pub day_start_minutes: u32,
}

impl Default for StatsConfig {
    fn default() -> Self {
        Self {
            min_session_secs: 5,
            merge_gap_secs: 60,
            day_start_minutes: 0,
        }
    }
}

impl StatsConfig {
    pub fn validate(&self) -> Result<()> {
        if self.min_session_secs < 0 {
            bail!("min_session_secs must not be negative");
        }
        if self.merge_gap_secs < 0 {
            bail!("merge_gap_secs must not be negative");
        }
        if self.day_start_minutes > 720 {
            bail!("day_start_minutes must be at most 720 (12:00)");
        }
        Ok(())
    }
}

/// One uninterrupted stretch of reading. Positions are book fractions in `0.0..=1.0`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub book_id: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub pages: u32,
    pub words: u64,
    pub characters: u64,
    pub start_pos: f64,
    pub end_pos: f64,
}

impl Session {
    pub fn duration_secs(&self) -> i64 {
        (self.end - self.start).num_seconds()
    }
}

/// A session being recorded right now.
#[derive(Debug, Clone)]
pub struct ActiveSession {
    book_id: String,
    start: DateTime<Utc>,
    start_pos: f64,
}

impl ActiveSession {
    pub fn begin(book_id: &str, now: DateTime<Utc>, pos: f64) -> Self {
        Self {
            book_id: book_id.to_string(),
            start: now,
            start_pos: pos,
        }
    }

    /// Ends the session; a clock that went backwards yields a zero-length session.
    pub fn finish(
        self,
        now: DateTime<Utc>,
        pos: f64,
        pages: u32,
        words: u64,
        characters: u64,
    ) -> Session {
        Session {
            book_id: self.book_id,
            start: self.start,
            end: now.max(self.start),
            pages,
            words,
            characters,
            start_pos: self.start_pos,
            end_pos: pos,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Totals {
    pub seconds: u64,
    pub pages: u64,
    pub words: u64,
    pub characters: u64,
    pub sessions: u32,
}

impl Totals {
    fn add(&mut self, other: &Totals) {
        self.seconds += other.seconds;
        self.pages += other.pages;
        self.words += other.words;
        self.characters += other.characters;
        self.sessions += other.sessions;
    }
}

/// What [`StatsStore::record`] did with a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordOutcome {
    Recorded,
    Merged,
    Ignored,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Goals {
    pub daily_minutes: Option<u32>,
    pub yearly_books: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinishedBook {
    pub book_id: String,
    pub date: NaiveDate,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StatsStore {
    pub config: StatsConfig,
    pub goals: Goals,
    sessions: Vec<Session>,
    finished: Vec<FinishedBook>,
}

impl StatsStore {
    pub fn new(config: StatsConfig) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            config,
            ..Self::default()
        })
    }

    pub fn sessions(&self) -> &[Session] {
        &self.sessions
    }

    pub fn finished(&self) -> &[FinishedBook] {
        &self.finished
    }

    /// Records a session: short ones are ignored, ones close after the previous
    /// session of the same book are merged into it.
    pub fn record(&mut self, s: Session) -> Result<RecordOutcome> {
        if s.end < s.start {
            bail!("session ends before it starts");
        }
        if !s.start_pos.is_finite() || !s.end_pos.is_finite() {
            bail!("session positions must be finite numbers");
        }
        if s.duration_secs() < self.config.min_session_secs {
            return Ok(RecordOutcome::Ignored);
        }
        let prev = self
            .sessions
            .iter_mut()
            .filter(|p| p.book_id == s.book_id)
            .max_by_key(|p| p.start);
        if let Some(p) = prev {
            let gap = (s.start - p.end).num_seconds();
            if s.start >= p.start && gap <= self.config.merge_gap_secs {
                p.end = p.end.max(s.end);
                p.pages += s.pages;
                p.words += s.words;
                p.characters += s.characters;
                p.end_pos = s.end_pos;
                return Ok(RecordOutcome::Merged);
            }
        }
        let at = self.sessions.partition_point(|p| p.start <= s.start);
        self.sessions.insert(at, s);
        Ok(RecordOutcome::Recorded)
    }

    /// Marks a book finished on a date (one entry per book; a later call updates the date).
    pub fn mark_finished(&mut self, book_id: &str, date: NaiveDate) {
        match self.finished.iter_mut().find(|f| f.book_id == book_id) {
            Some(f) => f.date = date,
            None => self.finished.push(FinishedBook {
                book_id: book_id.to_string(),
                date,
            }),
        }
    }

    pub fn daily<Tz: TimeZone>(&self, tz: &Tz) -> BTreeMap<NaiveDate, Totals> {
        daily_totals(&self.sessions, &self.config, tz)
    }

    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn from_json(s: &str) -> Result<Self> {
        let st: Self = serde_json::from_str(s).context("parsing statistics JSON")?;
        st.config.validate()?;
        Ok(st)
    }

    /// Loads a store; a missing file yields an empty store.
    pub fn load(path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(s) => Self::from_json(&s).with_context(|| format!("loading {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, self.to_json()?)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }
}

/// The reading day an instant belongs to.
pub fn reading_day<Tz: TimeZone>(t: DateTime<Utc>, day_start_minutes: u32, tz: &Tz) -> NaiveDate {
    (t.with_timezone(tz).naive_local() - Duration::minutes(day_start_minutes as i64)).date()
}

/// The instant at which reading day `date` begins.
pub fn day_start_instant<Tz: TimeZone>(
    date: NaiveDate,
    day_start_minutes: u32,
    tz: &Tz,
) -> DateTime<Utc> {
    let naive =
        date.and_hms_opt(0, 0, 0).unwrap_or_default() + Duration::minutes(day_start_minutes as i64);
    for extra in [0, 1, 2] {
        match tz.from_local_datetime(&(naive + Duration::hours(extra))) {
            LocalResult::Single(t) => return t.with_timezone(&Utc),
            LocalResult::Ambiguous(first, _) => return first.with_timezone(&Utc),
            LocalResult::None => continue,
        }
    }
    naive.and_utc()
}

fn split_by_day<Tz: TimeZone>(s: &Session, cfg: &StatsConfig, tz: &Tz) -> Vec<(NaiveDate, i64)> {
    let mut out = Vec::new();
    let mut cur = s.start;
    for _ in 0..400 {
        let day = reading_day(cur, cfg.day_start_minutes, tz);
        let next = day
            .succ_opt()
            .map(|d| day_start_instant(d, cfg.day_start_minutes, tz))
            .unwrap_or(s.end);
        if next <= cur || next >= s.end {
            out.push((day, (s.end - cur).num_seconds()));
            return out;
        }
        out.push((day, (next - cur).num_seconds()));
        cur = next;
    }
    out.push((
        reading_day(cur, cfg.day_start_minutes, tz),
        (s.end - cur).num_seconds(),
    ));
    out
}

fn share(total: u64, part: i64, whole: i64) -> u64 {
    if whole <= 0 {
        return 0;
    }
    (total as u128 * part.max(0) as u128 / whole as u128) as u64
}

/// Per reading-day totals. A session crossing a day boundary is split by time
/// (pages/words/characters proportionally); it counts as one session on its first day.
pub fn daily_totals<Tz: TimeZone>(
    sessions: &[Session],
    cfg: &StatsConfig,
    tz: &Tz,
) -> BTreeMap<NaiveDate, Totals> {
    let mut map: BTreeMap<NaiveDate, Totals> = BTreeMap::new();
    for s in sessions {
        let whole = s.duration_secs();
        let pieces = split_by_day(s, cfg, tz);
        let (mut p, mut w, mut c) = (0u64, 0u64, 0u64);
        let n = pieces.len();
        for (i, (day, secs)) in pieces.into_iter().enumerate() {
            let last = i + 1 == n;
            let (pp, ww, cc) = if last {
                (s.pages as u64 - p, s.words - w, s.characters - c)
            } else {
                (
                    share(s.pages as u64, secs, whole),
                    share(s.words, secs, whole),
                    share(s.characters, secs, whole),
                )
            };
            p += pp;
            w += ww;
            c += cc;
            map.entry(day).or_default().add(&Totals {
                seconds: secs.max(0) as u64,
                pages: pp,
                words: ww,
                characters: cc,
                sessions: u32::from(i == 0),
            });
        }
    }
    map
}

/// Totals per book.
pub fn book_totals(sessions: &[Session]) -> BTreeMap<String, Totals> {
    let mut map: BTreeMap<String, Totals> = BTreeMap::new();
    for s in sessions {
        map.entry(s.book_id.clone()).or_default().add(&Totals {
            seconds: s.duration_secs().max(0) as u64,
            pages: s.pages as u64,
            words: s.words,
            characters: s.characters,
            sessions: 1,
        });
    }
    map
}

/// First day of the week containing `date`.
pub fn week_start_of(date: NaiveDate, week_start: Weekday) -> NaiveDate {
    let back = (7 + date.weekday().num_days_from_monday() as i64
        - week_start.num_days_from_monday() as i64)
        % 7;
    date - Duration::days(back)
}

/// Weekly totals keyed by the first day of the week.
pub fn weekly_totals(
    daily: &BTreeMap<NaiveDate, Totals>,
    week_start: Weekday,
) -> BTreeMap<NaiveDate, Totals> {
    let mut map: BTreeMap<NaiveDate, Totals> = BTreeMap::new();
    for (d, t) in daily {
        map.entry(week_start_of(*d, week_start)).or_default().add(t);
    }
    map
}

/// Monthly totals keyed by `(year, month)`.
pub fn monthly_totals(daily: &BTreeMap<NaiveDate, Totals>) -> BTreeMap<(i32, u32), Totals> {
    let mut map: BTreeMap<(i32, u32), Totals> = BTreeMap::new();
    for (d, t) in daily {
        map.entry((d.year(), d.month())).or_default().add(t);
    }
    map
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Streaks {
    pub current: u32,
    pub longest: u32,
}

/// Reading streaks. A day counts when it has at least `min_seconds` of reading.
/// The current streak survives until the end of today: if today has no reading yet,
/// it counts back from yesterday. Days after `today` are ignored.
pub fn streaks(daily: &BTreeMap<NaiveDate, Totals>, today: NaiveDate, min_seconds: u64) -> Streaks {
    let days: BTreeSet<NaiveDate> = daily
        .iter()
        .filter(|(d, t)| **d <= today && t.seconds >= min_seconds.max(1))
        .map(|(d, _)| *d)
        .collect();
    let mut longest = 0u32;
    let mut run = 0u32;
    let mut prev: Option<NaiveDate> = None;
    for d in &days {
        run = match prev {
            Some(p) if p.succ_opt() == Some(*d) => run + 1,
            _ => 1,
        };
        longest = longest.max(run);
        prev = Some(*d);
    }
    let anchor = if days.contains(&today) {
        Some(today)
    } else {
        today.pred_opt().filter(|y| days.contains(y))
    };
    let mut current = 0u32;
    let mut d = anchor;
    while let Some(day) = d {
        if !days.contains(&day) {
            break;
        }
        current += 1;
        d = day.pred_opt();
    }
    Streaks { current, longest }
}

fn median(sorted: &[f64]) -> f64 {
    let n = sorted.len();
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    }
}

/// Reading speed in words per minute: median of per-session speeds after dropping
/// implausible values and outliers (more than 3 scaled MADs from the median).
pub fn reading_speed_wpm(sessions: &[Session]) -> Option<f64> {
    let mut speeds: Vec<f64> = sessions
        .iter()
        .filter(|s| s.words > 0 && s.duration_secs() >= MIN_SPEED_SESSION_SECS)
        .map(|s| s.words as f64 * 60.0 / s.duration_secs() as f64)
        .filter(|w| w.is_finite() && *w > 0.0 && *w <= MAX_PLAUSIBLE_WPM)
        .collect();
    if speeds.is_empty() {
        return None;
    }
    speeds.sort_by(|a, b| a.total_cmp(b));
    let m = median(&speeds);
    let mut dev: Vec<f64> = speeds.iter().map(|s| (s - m).abs()).collect();
    dev.sort_by(|a, b| a.total_cmp(b));
    let mad = median(&dev);
    let limit = if mad > 0.0 {
        3.0 * 1.4826 * mad
    } else {
        m * 0.5
    };
    let mut kept: Vec<f64> = speeds
        .into_iter()
        .filter(|s| (s - m).abs() <= limit)
        .collect();
    kept.sort_by(|a, b| a.total_cmp(b));
    Some(median(&kept))
}

/// Seconds needed to read `words_left` words at `wpm`; `None` without a usable speed.
pub fn estimate_remaining_secs(words_left: u64, wpm: f64) -> Option<u64> {
    if !wpm.is_finite() || wpm <= 0.0 {
        return None;
    }
    Some((words_left as f64 * 60.0 / wpm).ceil() as u64)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Remaining {
    pub chapter_secs: Option<u64>,
    pub book_secs: Option<u64>,
}

/// Time left in the current chapter and in the book.
pub fn remaining_estimates(
    chapter_words_left: u64,
    book_words_left: u64,
    wpm: Option<f64>,
) -> Remaining {
    let f = |w| wpm.and_then(|s| estimate_remaining_secs(w, s));
    Remaining {
        chapter_secs: f(chapter_words_left),
        book_secs: f(book_words_left),
    }
}

/// `"<1 min"`, `"42 min"`, `"1 h 05 min"`, `"3 h"`.
pub fn format_duration(secs: u64) -> String {
    if secs == 0 {
        return "0 min".into();
    }
    if secs < 30 {
        return "<1 min".into();
    }
    let mins = (secs + 30) / 60;
    match (mins / 60, mins % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m:02} min"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Progress {
    pub done: u64,
    pub target: u64,
    /// `done / target` (may exceed 1.0).
    pub fraction: f64,
    pub reached: bool,
}

fn progress(done: u64, target: u64) -> Progress {
    Progress {
        done,
        target,
        fraction: if target == 0 {
            1.0
        } else {
            done as f64 / target as f64
        },
        reached: done >= target,
    }
}

/// Today's reading minutes against the daily goal (`None` when no goal is set).
pub fn daily_goal_progress(
    daily: &BTreeMap<NaiveDate, Totals>,
    today: NaiveDate,
    goals: &Goals,
) -> Option<Progress> {
    let target = goals.daily_minutes?;
    let secs = daily.get(&today).map_or(0, |t| t.seconds);
    Some(progress(secs / 60, target as u64))
}

/// Books finished in `year` against the yearly goal (`None` when no goal is set).
pub fn yearly_goal_progress(
    finished: &[FinishedBook],
    year: i32,
    goals: &Goals,
) -> Option<Progress> {
    let target = goals.yearly_books?;
    let done = finished.iter().filter(|f| f.date.year() == year).count() as u64;
    Some(progress(done, target as u64))
}

/// Calendar heatmap: `columns[week][row]` where `row` counts days from `week_start`.
/// Cells outside the year are `None`. The grid has 53 columns, or 54 in the rare
/// leap years whose 366 days need an extra partial week.
#[derive(Debug, Clone, PartialEq)]
pub struct Heatmap {
    pub year: i32,
    pub week_start: Weekday,
    pub columns: Vec<[Option<u64>; 7]>,
    /// Intensity 0..=4 per cell (0 = no reading).
    pub levels: Vec<[Option<u8>; 7]>,
}

pub fn heatmap(
    daily: &BTreeMap<NaiveDate, Totals>,
    year: i32,
    week_start: Weekday,
) -> Result<Heatmap> {
    let jan1 = NaiveDate::from_ymd_opt(year, 1, 1).context("year out of range")?;
    let offset = (week_start_offset(jan1, week_start)) as usize;
    let days = if jan1.leap_year() { 366 } else { 365 };
    let cols = ((offset + days).div_ceil(7)).max(53);
    let mut columns = vec![[None; 7]; cols];
    let mut max = 0u64;
    for i in 0..days {
        let d = jan1 + Duration::days(i as i64);
        let secs = daily.get(&d).map_or(0, |t| t.seconds);
        max = max.max(secs);
        columns[(offset + i) / 7][(offset + i) % 7] = Some(secs);
    }
    let levels = columns
        .iter()
        .map(|col| {
            let mut row = [None; 7];
            for (r, cell) in col.iter().enumerate() {
                row[r] = cell.map(|s| {
                    if s == 0 || max == 0 {
                        0
                    } else {
                        (1 + (s - 1) * 4 / max).min(4) as u8
                    }
                });
            }
            row
        })
        .collect();
    Ok(Heatmap {
        year,
        week_start,
        columns,
        levels,
    })
}

fn week_start_offset(d: NaiveDate, week_start: Weekday) -> i64 {
    (d - week_start_of(d, week_start)).num_days()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, NaiveDateTime, Offset};

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, s).unwrap()
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn sess(book: &str, start: DateTime<Utc>, secs: i64, words: u64) -> Session {
        Session {
            book_id: book.into(),
            start,
            end: start + Duration::seconds(secs),
            pages: (words / 250) as u32,
            words,
            characters: words * 6,
            start_pos: 0.1,
            end_pos: 0.2,
        }
    }

    /// America/New_York for 2023..=2030: DST from the 2nd Sunday of March 02:00 local
    /// to the 1st Sunday of November 02:00 local (hand-written test zone).
    #[derive(Clone, Copy, Debug)]
    struct Ny;
    #[derive(Clone, Copy, Debug)]
    struct NyOff(i32);
    impl Offset for NyOff {
        fn fix(&self) -> FixedOffset {
            FixedOffset::east_opt(self.0).unwrap()
        }
    }
    impl std::fmt::Display for NyOff {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.0)
        }
    }
    fn nth_sunday(y: i32, m: u32, n: u32) -> NaiveDate {
        let first = date(y, m, 1);
        let to_sun = (7 - first.weekday().num_days_from_sunday()) % 7;
        first + Duration::days((to_sun + 7 * (n - 1)) as i64)
    }
    fn ny_off_utc(u: &NaiveDateTime) -> i32 {
        let y = u.year();
        let start = nth_sunday(y, 3, 2).and_hms_opt(7, 0, 0).unwrap();
        let end = nth_sunday(y, 11, 1).and_hms_opt(6, 0, 0).unwrap();
        if *u >= start && *u < end {
            -4 * 3600
        } else {
            -5 * 3600
        }
    }
    impl TimeZone for Ny {
        type Offset = NyOff;
        fn from_offset(_: &NyOff) -> Self {
            Ny
        }
        fn offset_from_local_date(&self, l: &NaiveDate) -> LocalResult<NyOff> {
            self.offset_from_local_datetime(&l.and_hms_opt(12, 0, 0).unwrap())
        }
        fn offset_from_local_datetime(&self, l: &NaiveDateTime) -> LocalResult<NyOff> {
            let mut ok = Vec::new();
            for off in [-4 * 3600, -5 * 3600] {
                let u = *l - Duration::seconds(off as i64);
                if ny_off_utc(&u) == off {
                    ok.push(NyOff(off));
                }
            }
            match ok.len() {
                0 => LocalResult::None,
                1 => LocalResult::Single(ok[0]),
                _ => LocalResult::Ambiguous(ok[0], ok[1]),
            }
        }
        fn offset_from_utc_date(&self, u: &NaiveDate) -> NyOff {
            NyOff(ny_off_utc(&u.and_hms_opt(12, 0, 0).unwrap()))
        }
        fn offset_from_utc_datetime(&self, u: &NaiveDateTime) -> NyOff {
            NyOff(ny_off_utc(u))
        }
    }

    fn cfg() -> StatsConfig {
        StatsConfig::default()
    }

    #[test]
    fn test_zone_sanity() {
        // 2024-03-10 06:59 UTC is still EST, 07:00 UTC is EDT
        let a = utc(2024, 3, 10, 6, 59, 0).with_timezone(&Ny);
        let b = utc(2024, 3, 10, 7, 0, 0).with_timezone(&Ny);
        assert_eq!(a.naive_local().to_string(), "2024-03-10 01:59:00");
        assert_eq!(b.naive_local().to_string(), "2024-03-10 03:00:00");
        assert!(matches!(
            Ny.from_local_datetime(&date(2024, 3, 10).and_hms_opt(2, 30, 0).unwrap()),
            LocalResult::None
        ));
        assert!(matches!(
            Ny.from_local_datetime(&date(2024, 11, 3).and_hms_opt(1, 30, 0).unwrap()),
            LocalResult::Ambiguous(_, _)
        ));
    }

    #[test]
    fn config_validation() {
        assert!(cfg().validate().is_ok());
        let bad = |c: StatsConfig| c.validate().unwrap_err().to_string();
        assert!(bad(StatsConfig {
            min_session_secs: -1,
            ..cfg()
        })
        .contains("min_session"));
        assert!(bad(StatsConfig {
            merge_gap_secs: -1,
            ..cfg()
        })
        .contains("merge_gap"));
        assert!(bad(StatsConfig {
            day_start_minutes: 721,
            ..cfg()
        })
        .contains("720"));
        assert!(StatsStore::new(StatsConfig {
            day_start_minutes: 999,
            ..cfg()
        })
        .is_err());
    }

    #[test]
    fn record_ignores_short_sessions() {
        let mut st = StatsStore::new(StatsConfig {
            min_session_secs: 10,
            ..cfg()
        })
        .unwrap();
        let t = utc(2025, 5, 1, 10, 0, 0);
        assert_eq!(
            st.record(sess("a", t, 9, 10)).unwrap(),
            RecordOutcome::Ignored
        );
        assert_eq!(
            st.record(sess("a", t, 10, 10)).unwrap(),
            RecordOutcome::Recorded
        );
        assert_eq!(st.sessions().len(), 1);
    }

    #[test]
    fn record_merges_within_gap_only_for_same_book() {
        let mut st = StatsStore::new(cfg()).unwrap();
        let t = utc(2025, 5, 1, 10, 0, 0);
        st.record(sess("a", t, 600, 1000)).unwrap();
        // gap of exactly 60 s merges
        let mut second = sess("a", t + Duration::seconds(660), 300, 500);
        second.start_pos = 0.2;
        second.end_pos = 0.35;
        assert_eq!(st.record(second).unwrap(), RecordOutcome::Merged);
        assert_eq!(st.sessions().len(), 1);
        let m = &st.sessions()[0];
        assert_eq!(m.duration_secs(), 960);
        assert_eq!((m.words, m.pages, m.characters), (1500, 4 + 2, 9000));
        assert_eq!((m.start_pos, m.end_pos), (0.1, 0.35));
        // gap 61 s does not merge
        let t3 = m.end + Duration::seconds(61);
        assert_eq!(
            st.record(sess("a", t3, 100, 10)).unwrap(),
            RecordOutcome::Recorded
        );
        // other book never merges
        let t4 = st.sessions()[1].end + Duration::seconds(1);
        assert_eq!(
            st.record(sess("b", t4, 100, 10)).unwrap(),
            RecordOutcome::Recorded
        );
        assert_eq!(st.sessions().len(), 3);
    }

    #[test]
    fn record_overlapping_merge_keeps_longest_end() {
        let mut st = StatsStore::new(cfg()).unwrap();
        let t = utc(2025, 5, 1, 10, 0, 0);
        st.record(sess("a", t, 1000, 100)).unwrap();
        // fully inside the first one
        assert_eq!(
            st.record(sess("a", t + Duration::seconds(100), 50, 5))
                .unwrap(),
            RecordOutcome::Merged
        );
        assert_eq!(st.sessions()[0].duration_secs(), 1000);
    }

    #[test]
    fn record_keeps_sessions_sorted_and_does_not_merge_older_session() {
        let mut st = StatsStore::new(cfg()).unwrap();
        let late = utc(2025, 5, 2, 10, 0, 0);
        let early = utc(2025, 5, 1, 10, 0, 0);
        st.record(sess("a", late, 600, 10)).unwrap();
        // an earlier session recorded afterwards (sync/import) must not merge backwards
        assert_eq!(
            st.record(sess("a", early, 600, 10)).unwrap(),
            RecordOutcome::Recorded
        );
        let starts: Vec<_> = st.sessions().iter().map(|s| s.start).collect();
        assert_eq!(starts, vec![early, late]);
    }

    #[test]
    fn record_rejects_invalid_sessions() {
        let mut st = StatsStore::new(cfg()).unwrap();
        let t = utc(2025, 5, 1, 10, 0, 0);
        let mut s = sess("a", t, 100, 1);
        s.end = t - Duration::seconds(1);
        assert!(st.record(s).unwrap_err().to_string().contains("before"));
        let mut s = sess("a", t, 100, 1);
        s.end_pos = f64::NAN;
        assert!(st.record(s).is_err());
        assert!(st.sessions().is_empty());
    }

    #[test]
    fn active_session_flow() {
        let t = utc(2025, 5, 1, 10, 0, 0);
        let a = ActiveSession::begin("book", t, 0.25);
        let s = a
            .clone()
            .finish(t + Duration::seconds(90), 0.3, 2, 400, 2400);
        assert_eq!(s.duration_secs(), 90);
        assert_eq!(
            (s.start_pos, s.end_pos, s.pages, s.words, s.characters),
            (0.25, 0.3, 2, 400, 2400)
        );
        let back = a.finish(t - Duration::seconds(5), 0.25, 0, 0, 0);
        assert_eq!(
            back.duration_secs(),
            0,
            "clock going backwards gives zero length"
        );
    }

    #[test]
    fn reading_day_with_offset() {
        let utc_tz = Utc;
        let t = utc(2025, 5, 2, 1, 30, 0);
        assert_eq!(reading_day(t, 0, &utc_tz), date(2025, 5, 2));
        assert_eq!(reading_day(t, 240, &utc_tz), date(2025, 5, 1));
        assert_eq!(
            reading_day(utc(2025, 5, 2, 4, 0, 0), 240, &utc_tz),
            date(2025, 5, 2)
        );
        assert_eq!(
            reading_day(utc(2025, 5, 2, 3, 59, 59), 240, &utc_tz),
            date(2025, 5, 1)
        );
        // year boundary
        assert_eq!(
            reading_day(utc(2025, 1, 1, 2, 0, 0), 240, &utc_tz),
            date(2024, 12, 31)
        );
        // fixed offset zone
        let tz = FixedOffset::east_opt(9 * 3600).unwrap();
        assert_eq!(
            reading_day(utc(2025, 5, 1, 15, 30, 0), 0, &tz),
            date(2025, 5, 2)
        );
    }

    #[test]
    fn daily_totals_split_across_midnight() {
        let c = cfg();
        // 23:30 -> 00:30 UTC: 3600 s, 600 words
        let s = sess("a", utc(2025, 5, 1, 23, 30, 0), 3600, 600);
        let d = daily_totals(std::slice::from_ref(&s), &c, &Utc);
        assert_eq!(d.len(), 2);
        let a = d[&date(2025, 5, 1)];
        let b = d[&date(2025, 5, 2)];
        assert_eq!((a.seconds, b.seconds), (1800, 1800));
        assert_eq!(a.words + b.words, 600);
        assert_eq!(a.pages + b.pages, s.pages as u64);
        assert_eq!(a.characters + b.characters, s.characters);
        assert_eq!((a.sessions, b.sessions), (1, 0));
    }

    #[test]
    fn daily_totals_with_day_start_offset() {
        let c = StatsConfig {
            day_start_minutes: 240,
            ..cfg()
        };
        let s = sess("a", utc(2025, 5, 2, 1, 0, 0), 3600, 100);
        let d = daily_totals(&[s], &c, &Utc);
        assert_eq!(d.len(), 1);
        assert_eq!(
            d[&date(2025, 5, 1)].seconds,
            3600,
            "01:00 counts for the previous day"
        );
        // crossing the 04:00 boundary
        let s = sess("a", utc(2025, 5, 2, 3, 30, 0), 3600, 100);
        let d = daily_totals(&[s], &c, &Utc);
        assert_eq!(d[&date(2025, 5, 1)].seconds, 1800);
        assert_eq!(d[&date(2025, 5, 2)].seconds, 1800);
    }

    #[test]
    fn daily_totals_multi_day_session_and_zero_length() {
        let c = cfg();
        let s = sess("a", utc(2025, 12, 30, 12, 0, 0), 3 * 86400, 3000);
        let d = daily_totals(&[s], &c, &Utc);
        assert_eq!(d.len(), 4);
        assert_eq!(d.values().map(|t| t.seconds).sum::<u64>(), 3 * 86400);
        assert_eq!(d.values().map(|t| t.words).sum::<u64>(), 3000);
        assert!(
            d.contains_key(&date(2026, 1, 2)),
            "crosses the year boundary"
        );
        let z = sess("a", utc(2025, 5, 1, 12, 0, 0), 0, 0);
        let d = daily_totals(&[z], &c, &Utc);
        assert_eq!(d[&date(2025, 5, 1)].seconds, 0);
    }

    #[test]
    fn dst_spring_forward_day_has_23_hours() {
        // reading the whole of 2024-03-10 local: 05:00Z to 04:00Z next day = 23 h
        let c = cfg();
        let start = utc(2024, 3, 10, 5, 0, 0);
        let s = sess("a", start, 23 * 3600, 2300);
        let d = daily_totals(&[s], &c, &Ny);
        assert_eq!(d.len(), 1, "{d:?}");
        assert_eq!(d[&date(2024, 3, 10)].seconds, 23 * 3600);
        // one more hour spills into 03-11
        let s = sess("a", start, 24 * 3600, 2400);
        let d = daily_totals(&[s], &c, &Ny);
        assert_eq!(d[&date(2024, 3, 10)].seconds, 23 * 3600);
        assert_eq!(d[&date(2024, 3, 11)].seconds, 3600);
    }

    #[test]
    fn dst_fall_back_day_has_25_hours() {
        let c = cfg();
        let start = utc(2024, 11, 3, 4, 0, 0); // local midnight EDT
        let s = sess("a", start, 25 * 3600, 100);
        let d = daily_totals(&[s], &c, &Ny);
        assert_eq!(d.len(), 1, "{d:?}");
        assert_eq!(d[&date(2024, 11, 3)].seconds, 25 * 3600);
        let s = sess("a", start, 26 * 3600, 100);
        let d = daily_totals(&[s], &c, &Ny);
        assert_eq!(d[&date(2024, 11, 4)].seconds, 3600);
    }

    #[test]
    fn dst_day_start_inside_the_gap() {
        // day starts at 02:30; on 2024-03-10 that local time does not exist
        let c = StatsConfig {
            day_start_minutes: 150,
            ..cfg()
        };
        let t = day_start_instant(date(2024, 3, 10), 150, &Ny);
        assert_eq!(
            t.with_timezone(&Ny).naive_local().to_string(),
            "2024-03-10 03:30:00"
        );
        let d = daily_totals(&[sess("a", utc(2024, 3, 10, 12, 0, 0), 600, 10)], &c, &Ny);
        assert_eq!(d.len(), 1);
        // ambiguous: day starts 01:30 on fall-back day picks the first occurrence
        let t = day_start_instant(date(2024, 11, 3), 90, &Ny);
        assert_eq!(t, utc(2024, 11, 3, 5, 30, 0));
    }

    #[test]
    fn day_start_instant_basic() {
        assert_eq!(
            day_start_instant(date(2025, 5, 1), 0, &Utc),
            utc(2025, 5, 1, 0, 0, 0)
        );
        assert_eq!(
            day_start_instant(date(2025, 5, 1), 270, &Utc),
            utc(2025, 5, 1, 4, 30, 0)
        );
        let tz = FixedOffset::west_opt(5 * 3600).unwrap();
        assert_eq!(
            day_start_instant(date(2025, 1, 1), 0, &tz),
            utc(2025, 1, 1, 5, 0, 0)
        );
    }

    #[test]
    fn book_totals_aggregate() {
        let t = utc(2025, 5, 1, 10, 0, 0);
        let v = vec![
            sess("a", t, 100, 10),
            sess("b", t, 200, 20),
            sess("a", t, 300, 30),
        ];
        let m = book_totals(&v);
        assert_eq!(m["a"].seconds, 400);
        assert_eq!(m["a"].words, 40);
        assert_eq!(m["a"].sessions, 2);
        assert_eq!(m["b"].seconds, 200);
        assert!(book_totals(&[]).is_empty());
    }

    #[test]
    fn week_and_month_aggregation() {
        let c = cfg();
        let v = vec![
            sess("a", utc(2025, 12, 28, 10, 0, 0), 600, 10), // Sunday
            sess("a", utc(2025, 12, 29, 10, 0, 0), 600, 10), // Monday
            sess("a", utc(2026, 1, 1, 10, 0, 0), 600, 10),   // Thursday
            sess("a", utc(2026, 1, 5, 10, 0, 0), 600, 10),   // Monday
        ];
        let daily = daily_totals(&v, &c, &Utc);
        let w = weekly_totals(&daily, Weekday::Mon);
        assert_eq!(w.len(), 3);
        assert_eq!(w[&date(2025, 12, 22)].seconds, 600);
        assert_eq!(
            w[&date(2025, 12, 29)].seconds,
            1200,
            "week crosses the new year"
        );
        assert_eq!(w[&date(2026, 1, 5)].seconds, 600);
        let w = weekly_totals(&daily, Weekday::Sun);
        assert_eq!(w[&date(2025, 12, 28)].seconds, 1800);
        let m = monthly_totals(&daily);
        assert_eq!(m[&(2025, 12)].seconds, 1200);
        assert_eq!(m[&(2026, 1)].seconds, 1200);
        assert_eq!(m[&(2026, 1)].sessions, 2);
    }

    #[test]
    fn week_start_of_all_weekdays() {
        for i in 0..14 {
            let d = date(2025, 3, 1) + Duration::days(i);
            for ws in [Weekday::Mon, Weekday::Sun, Weekday::Sat] {
                let s = week_start_of(d, ws);
                assert_eq!(s.weekday(), ws);
                assert!((d - s).num_days() < 7 && s <= d);
            }
        }
    }

    fn days_map(days: &[NaiveDate]) -> BTreeMap<NaiveDate, Totals> {
        days.iter()
            .map(|d| {
                (
                    *d,
                    Totals {
                        seconds: 600,
                        ..Totals::default()
                    },
                )
            })
            .collect()
    }

    #[test]
    fn streaks_current_and_longest() {
        let today = date(2025, 5, 10);
        let d = days_map(&[
            date(2025, 5, 1),
            date(2025, 5, 2),
            date(2025, 5, 3),
            date(2025, 5, 4),
            date(2025, 5, 8),
            date(2025, 5, 9),
            date(2025, 5, 10),
        ]);
        assert_eq!(
            streaks(&d, today, 1),
            Streaks {
                current: 3,
                longest: 4
            }
        );
        // today not read yet: streak survives from yesterday
        assert_eq!(
            streaks(&d, date(2025, 5, 11), 1),
            Streaks {
                current: 3,
                longest: 4
            }
        );
        // a missed day breaks it
        assert_eq!(
            streaks(&d, date(2025, 5, 12), 1),
            Streaks {
                current: 0,
                longest: 4
            }
        );
        assert_eq!(
            streaks(&BTreeMap::new(), today, 1),
            Streaks {
                current: 0,
                longest: 0
            }
        );
    }

    #[test]
    fn streaks_threshold_future_days_and_year_boundary() {
        let d = days_map(&[
            date(2025, 12, 30),
            date(2025, 12, 31),
            date(2026, 1, 1),
            date(2026, 1, 2),
        ]);
        assert_eq!(streaks(&d, date(2026, 1, 2), 600).current, 4);
        assert_eq!(
            streaks(&d, date(2026, 1, 2), 601),
            Streaks {
                current: 0,
                longest: 0
            }
        );
        // days after "today" are ignored
        assert_eq!(
            streaks(&d, date(2025, 12, 31), 1),
            Streaks {
                current: 2,
                longest: 2
            }
        );
        // min 0 behaves like 1: empty days do not count
        let mut z = d.clone();
        z.insert(date(2026, 1, 3), Totals::default());
        assert_eq!(streaks(&z, date(2026, 1, 3), 0).current, 4);
        // leap day continuity
        let l = days_map(&[date(2024, 2, 28), date(2024, 2, 29), date(2024, 3, 1)]);
        assert_eq!(streaks(&l, date(2024, 3, 1), 1).current, 3);
    }

    #[test]
    fn streak_respects_day_start_offset_end_to_end() {
        // night owl: reads at 00:30 on three consecutive nights with a 04:00 day start
        let c = StatsConfig {
            day_start_minutes: 240,
            ..cfg()
        };
        let v: Vec<Session> = (2..5)
            .map(|d| sess("a", utc(2025, 5, d, 0, 30, 0), 600, 10))
            .collect();
        let daily = daily_totals(&v, &c, &Utc);
        assert_eq!(
            daily.keys().cloned().collect::<Vec<_>>(),
            vec![date(2025, 5, 1), date(2025, 5, 2), date(2025, 5, 3)]
        );
        assert_eq!(streaks(&daily, date(2025, 5, 3), 1).current, 3);
    }

    #[test]
    fn speed_median_and_outliers() {
        let t = utc(2025, 5, 1, 10, 0, 0);
        // 200 wpm sessions of 5 minutes, with noise and one skim at 5000 wpm
        let mut v = vec![
            sess("a", t, 300, 1000),
            sess("a", t, 300, 1010),
            sess("a", t, 300, 990),
            sess("a", t, 300, 1020),
            sess("a", t, 300, 980),
        ];
        v.push(sess("a", t, 300, 25000)); // implausible, dropped
        v.push(sess("a", t, 300, 2500)); // 500 wpm outlier by MAD
        let wpm = reading_speed_wpm(&v).unwrap();
        assert!((wpm - 200.0).abs() < 1.0, "{wpm}");
    }

    #[test]
    fn speed_edge_cases() {
        let t = utc(2025, 5, 1, 10, 0, 0);
        assert!(reading_speed_wpm(&[]).is_none());
        assert!(
            reading_speed_wpm(&[sess("a", t, 10, 100)]).is_none(),
            "too short"
        );
        assert!(
            reading_speed_wpm(&[sess("a", t, 600, 0)]).is_none(),
            "no words"
        );
        let one = reading_speed_wpm(&[sess("a", t, 600, 2000)]).unwrap();
        assert!((one - 200.0).abs() < 1e-9);
        // identical speeds: MAD = 0 path
        let same = vec![sess("a", t, 600, 2000); 4];
        assert!((reading_speed_wpm(&same).unwrap() - 200.0).abs() < 1e-9);
        // even count median
        let two = vec![sess("a", t, 600, 2000), sess("a", t, 600, 2200)];
        assert!((reading_speed_wpm(&two).unwrap() - 210.0).abs() < 1e-9);
    }

    #[test]
    fn remaining_estimates_and_format() {
        assert_eq!(estimate_remaining_secs(500, 250.0), Some(120));
        assert_eq!(estimate_remaining_secs(1, 250.0), Some(1));
        assert_eq!(estimate_remaining_secs(0, 250.0), Some(0));
        assert_eq!(estimate_remaining_secs(10, 0.0), None);
        assert_eq!(estimate_remaining_secs(10, -3.0), None);
        assert_eq!(estimate_remaining_secs(10, f64::NAN), None);
        let r = remaining_estimates(250, 25000, Some(250.0));
        assert_eq!((r.chapter_secs, r.book_secs), (Some(60), Some(6000)));
        let r = remaining_estimates(250, 25000, None);
        assert_eq!((r.chapter_secs, r.book_secs), (None, None));
        for (s, out) in [
            (0, "0 min"),
            (10, "<1 min"),
            (29, "<1 min"),
            (30, "1 min"),
            (60, "1 min"),
            (2520, "42 min"),
            (3600, "1 h"),
            (3900, "1 h 05 min"),
            (10800, "3 h"),
            (36000 + 29 * 60, "10 h 29 min"),
        ] {
            assert_eq!(format_duration(s), out, "{s}");
        }
    }

    #[test]
    fn goals_progress() {
        let today = date(2025, 5, 10);
        let mut daily = BTreeMap::new();
        daily.insert(
            today,
            Totals {
                seconds: 25 * 60 + 59,
                ..Totals::default()
            },
        );
        let none = Goals::default();
        assert!(daily_goal_progress(&daily, today, &none).is_none());
        assert!(yearly_goal_progress(&[], 2025, &none).is_none());
        let g = Goals {
            daily_minutes: Some(30),
            yearly_books: Some(12),
        };
        let p = daily_goal_progress(&daily, today, &g).unwrap();
        assert_eq!((p.done, p.target, p.reached), (25, 30, false));
        assert!((p.fraction - 25.0 / 30.0).abs() < 1e-9);
        daily.insert(
            today,
            Totals {
                seconds: 45 * 60,
                ..Totals::default()
            },
        );
        let p = daily_goal_progress(&daily, today, &g).unwrap();
        assert!(p.reached && p.fraction > 1.0);
        let p = daily_goal_progress(&daily, date(2025, 5, 11), &g).unwrap();
        assert_eq!(p.done, 0);
        let fin = vec![
            FinishedBook {
                book_id: "a".into(),
                date: date(2024, 12, 31),
            },
            FinishedBook {
                book_id: "b".into(),
                date: date(2025, 1, 1),
            },
            FinishedBook {
                book_id: "c".into(),
                date: date(2025, 12, 31),
            },
        ];
        let p = yearly_goal_progress(&fin, 2025, &g).unwrap();
        assert_eq!((p.done, p.target), (2, 12));
        let zero = Goals {
            daily_minutes: Some(0),
            yearly_books: Some(0),
        };
        let p = daily_goal_progress(&daily, today, &zero).unwrap();
        assert!(p.reached && p.fraction == 1.0);
    }

    #[test]
    fn mark_finished_dedups_books() {
        let mut st = StatsStore::new(cfg()).unwrap();
        st.mark_finished("a", date(2025, 1, 1));
        st.mark_finished("b", date(2025, 2, 1));
        st.mark_finished("a", date(2025, 3, 1));
        assert_eq!(st.finished().len(), 2);
        assert_eq!(st.finished()[0].date, date(2025, 3, 1));
    }

    #[test]
    fn heatmap_shape_and_placement() {
        let mut daily = BTreeMap::new();
        daily.insert(
            date(2025, 1, 1),
            Totals {
                seconds: 100,
                ..Totals::default()
            },
        );
        daily.insert(
            date(2025, 12, 31),
            Totals {
                seconds: 400,
                ..Totals::default()
            },
        );
        daily.insert(
            date(2025, 6, 15),
            Totals {
                seconds: 200,
                ..Totals::default()
            },
        );
        daily.insert(
            date(2024, 12, 31),
            Totals {
                seconds: 999,
                ..Totals::default()
            },
        ); // other year
        let h = heatmap(&daily, 2025, Weekday::Sun).unwrap();
        assert_eq!(h.columns.len(), 53);
        assert_eq!(h.levels.len(), 53);
        // 2025-01-01 is a Wednesday: row 3 in a Sunday-first week
        assert_eq!(h.columns[0][3], Some(100));
        assert_eq!(h.columns[0][0], None);
        assert_eq!(h.columns[0][2], None);
        assert_eq!(h.columns[0][4], Some(0));
        // 2025-12-31 is a Wednesday, last column
        let filled: usize = h.columns.iter().flatten().flatten().count();
        assert_eq!(filled, 365);
        assert_eq!(h.columns[52][3], Some(400));
        assert_eq!(h.columns[52][4], None);
        // levels: 400 = max -> 4; 200 -> 2 ; 100 -> 1 ; empty 0
        assert_eq!(h.levels[52][3], Some(4));
        assert_eq!(h.levels[0][3], Some(1));
        assert_eq!(h.levels[0][4], Some(0));
        let jun = (date(2025, 6, 15) - date(2025, 1, 1)).num_days() as usize + 3;
        assert_eq!(h.columns[jun / 7][jun % 7], Some(200));
        assert_eq!(h.levels[jun / 7][jun % 7], Some(2));
        assert_eq!(h.year, 2025);
        assert_eq!(h.week_start, Weekday::Sun);
    }

    #[test]
    fn heatmap_week_start_and_leap_years() {
        let empty = BTreeMap::new();
        // 2024 leap year, Jan 1 is Monday
        let h = heatmap(&empty, 2024, Weekday::Mon).unwrap();
        assert_eq!(h.columns.len(), 53);
        assert_eq!(h.columns.iter().flatten().flatten().count(), 366);
        assert_eq!(h.columns[0][0], Some(0));
        // 2000: leap, Jan 1 is Saturday: Sunday-first needs a 54th column
        let h = heatmap(&empty, 2000, Weekday::Sun).unwrap();
        assert_eq!(h.columns.len(), 54);
        assert_eq!(h.columns.iter().flatten().flatten().count(), 366);
        assert_eq!(h.columns[0][6], Some(0));
        // same year Monday-first fits in 53
        let h = heatmap(&empty, 2000, Weekday::Mon).unwrap();
        assert_eq!(h.columns.len(), 53);
        // all levels are zero for an empty year
        assert!(h.levels.iter().flatten().flatten().all(|l| *l == 0));
        assert!(heatmap(&empty, i32::MAX, Weekday::Mon).is_err());
    }

    #[test]
    fn heatmap_every_year_has_every_day_exactly_once() {
        let empty = BTreeMap::new();
        for year in 1990..2060 {
            for ws in [Weekday::Mon, Weekday::Sun] {
                let h = heatmap(&empty, year, ws).unwrap();
                let n = if NaiveDate::from_ymd_opt(year, 1, 1).unwrap().leap_year() {
                    366
                } else {
                    365
                };
                assert_eq!(
                    h.columns.iter().flatten().flatten().count(),
                    n,
                    "{year} {ws:?}"
                );
                assert!(h.columns.len() == 53 || h.columns.len() == 54);
            }
        }
    }

    #[test]
    fn json_round_trip_and_files() {
        let mut st = StatsStore::new(StatsConfig {
            day_start_minutes: 120,
            ..cfg()
        })
        .unwrap();
        st.goals = Goals {
            daily_minutes: Some(20),
            yearly_books: Some(5),
        };
        st.record(sess("a", utc(2025, 5, 1, 10, 0, 0), 600, 1000))
            .unwrap();
        st.mark_finished("a", date(2025, 5, 2));
        let back = StatsStore::from_json(&st.to_json().unwrap()).unwrap();
        assert_eq!(back, st);

        let dir = crate::testutil::TmpDir::new("stats");
        let p = dir.path().join("x/stats.json");
        st.save(&p).unwrap();
        assert_eq!(StatsStore::load(&p).unwrap(), st);
        assert!(StatsStore::load(&dir.path().join("missing.json"))
            .unwrap()
            .sessions()
            .is_empty());
        std::fs::write(&p, "oops").unwrap();
        assert!(StatsStore::load(&p).is_err());
        let bad = st
            .to_json()
            .unwrap()
            .replace("\"day_start_minutes\": 120", "\"day_start_minutes\": 5000");
        assert!(StatsStore::from_json(&bad).is_err());
    }

    #[test]
    fn store_daily_matches_free_function() {
        let mut st = StatsStore::new(cfg()).unwrap();
        st.record(sess("a", utc(2025, 5, 1, 10, 0, 0), 600, 100))
            .unwrap();
        assert_eq!(
            st.daily(&Utc),
            daily_totals(st.sessions(), &st.config, &Utc)
        );
    }

    #[test]
    fn property_daily_totals_conserve_time_and_words() {
        // deterministic pseudo-random sessions in two zones and with day offsets
        let mut x = 99u64;
        let mut next = |m: u64| {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (x >> 33) % m
        };
        for offset in [0u32, 90, 240, 720] {
            let c = StatsConfig {
                day_start_minutes: offset,
                ..cfg()
            };
            let mut v = Vec::new();
            for _ in 0..200 {
                let start = utc(2024, 1, 1, 0, 0, 0) + Duration::seconds(next(400 * 86400) as i64);
                let secs = next(3 * 86400) as i64;
                v.push(sess("a", start, secs, next(100000)));
            }
            let total_secs: i64 = v.iter().map(|s| s.duration_secs()).sum();
            let total_words: u64 = v.iter().map(|s| s.words).sum();
            let total_pages: u64 = v.iter().map(|s| s.pages as u64).sum();
            for d in [daily_totals(&v, &c, &Utc), daily_totals(&v, &c, &Ny)] {
                assert_eq!(
                    d.values().map(|t| t.seconds as i64).sum::<i64>(),
                    total_secs
                );
                assert_eq!(d.values().map(|t| t.words).sum::<u64>(), total_words);
                assert_eq!(d.values().map(|t| t.pages).sum::<u64>(), total_pages);
                assert_eq!(d.values().map(|t| t.sessions).sum::<u32>(), v.len() as u32);
            }
        }
    }
}
