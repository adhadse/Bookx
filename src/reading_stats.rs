//! Reading statistics glue (roadmap R11/R40): turns reader events into recorded sessions and
//! sessions into the numbers the statistics window shows. No GTK in here, so it is unit tested.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};

use bookx_lookup::stats::{
    book_totals, daily_totals, reading_day, reading_speed_wpm, streaks, ActiveSession, Session,
    StatsConfig, StatsStore, Totals,
};

/// No page turn for this long ends the session (the book was probably put down).
pub const IDLE_SECS: i64 = 300;
/// A page turn that moves the position by more than this fraction of the book is a jump
/// (slider, search result, table of contents) and does not count as reading.
const MAX_STEP: f64 = 0.2;
/// Rough characters per word, for the character counter.
const CHARS_PER_WORD: u64 = 6;

#[derive(Debug, Clone)]
struct Active {
    book_id: String,
    session: ActiveSession,
    last_event: DateTime<Utc>,
    last_key: (usize, usize),
    last_pos: f64,
    pages: u32,
    words: u64,
}

/// Follows the reader and cuts its events into sessions.
#[derive(Debug, Default)]
pub struct Tracker {
    active: Option<Active>,
}

impl Tracker {
    /// The reader reports where it is. `key` identifies the page (chapter, page in chapter),
    /// `pos` is the position as a fraction of the book and `words_per_page` the density of the
    /// current chapter. Returns the sessions that ended because of this event.
    pub fn on_position(
        &mut self,
        book_id: &str,
        now: DateTime<Utc>,
        key: (usize, usize),
        pos: f64,
        words_per_page: u64,
    ) -> Vec<Session> {
        let mut done = Vec::new();
        let stale = self.active.as_ref().is_some_and(|a| {
            a.book_id != book_id || (now - a.last_event).num_seconds() > IDLE_SECS
        });
        if stale {
            done.extend(self.finish(now));
        }
        match self.active.as_mut() {
            None => {
                self.active = Some(Active {
                    book_id: book_id.to_string(),
                    session: ActiveSession::begin(book_id, now, pos),
                    last_event: now,
                    last_key: key,
                    last_pos: pos,
                    pages: 0,
                    words: 0,
                });
            }
            Some(a) => {
                if key != a.last_key {
                    let step = pos - a.last_pos;
                    if step > 0.0 && step <= MAX_STEP {
                        a.pages += 1;
                        a.words += words_per_page;
                    }
                    a.last_key = key;
                    a.last_pos = pos;
                }
                a.last_event = now;
            }
        }
        done
    }

    /// Ends the running session (book closed, window closed, app quit). A session that sat idle
    /// ends at its last page turn rather than now.
    pub fn finish(&mut self, now: DateTime<Utc>) -> Option<Session> {
        let a = self.active.take()?;
        let end = if (now - a.last_event).num_seconds() > IDLE_SECS {
            a.last_event
        } else {
            now
        };
        Some(
            a.session
                .finish(end, a.last_pos, a.pages, a.words, a.words * CHARS_PER_WORD),
        )
    }

    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }
}

/// The statistics store plus the tracker and where to save them.
pub struct StatsHub {
    pub store: StatsStore,
    tracker: Tracker,
    path: Option<PathBuf>,
}

impl StatsHub {
    /// Loads (or starts) the store at `path`. A corrupt file is kept aside and replaced by an
    /// empty store, so the app never fails to start because of statistics.
    pub fn load(path: Option<PathBuf>) -> StatsHub {
        let store = match path.as_deref().map(StatsStore::load) {
            Some(Ok(s)) => s,
            Some(Err(_)) => {
                if let Some(p) = &path {
                    let _ = std::fs::rename(p, p.with_extension("json.corrupt"));
                }
                StatsStore::default()
            }
            None => StatsStore::default(),
        };
        StatsHub {
            store,
            tracker: Tracker::default(),
            path,
        }
    }

    pub fn on_position(
        &mut self,
        book_id: u64,
        now: DateTime<Utc>,
        key: (usize, usize),
        pos: f64,
        words_per_page: u64,
    ) {
        let ended = self
            .tracker
            .on_position(&book_id.to_string(), now, key, pos, words_per_page);
        for s in ended {
            let _ = self.store.record(s);
        }
        if self.tracker.is_active() {
            // cheap: a single small JSON file, written at most once per page turn
            self.save();
        }
    }

    /// Ends the running session and writes the file.
    pub fn finish(&mut self, now: DateTime<Utc>) {
        if let Some(s) = self.tracker.finish(now) {
            let _ = self.store.record(s);
        }
        self.save();
    }

    pub fn mark_finished(&mut self, book_id: u64, date: NaiveDate) {
        self.store.mark_finished(&book_id.to_string(), date);
    }

    pub fn save(&self) {
        if let Some(p) = &self.path {
            let _ = self.store.save(p);
        }
    }
}

/// Numbers shown in the statistics window.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Summary {
    pub today_secs: u64,
    pub week_secs: u64,
    pub month_secs: u64,
    pub total_secs: u64,
    pub total_pages: u64,
    pub streak_current: u32,
    pub streak_longest: u32,
    pub wpm: Option<f64>,
    pub books_finished_this_year: u32,
    /// Oldest first, today last: the last seven reading days.
    pub last_week: Vec<(NaiveDate, u64)>,
    /// Most read first (book id, seconds), at most five.
    pub top_books: Vec<(u64, u64)>,
}

/// Computes the [`Summary`] as of `now` in time zone `tz`.
pub fn summarize<Tz: TimeZone>(store: &StatsStore, now: DateTime<Utc>, tz: &Tz) -> Summary {
    let cfg: &StatsConfig = &store.config;
    let today = reading_day(now, cfg.day_start_minutes, tz);
    let daily: BTreeMap<NaiveDate, Totals> = daily_totals(store.sessions(), cfg, tz);
    let secs_since = |days: i64| -> u64 {
        let from = today - Duration::days(days - 1);
        daily
            .range(from..=today)
            .map(|(_, t)| t.seconds)
            .sum::<u64>()
    };
    let last_week = (0..7)
        .rev()
        .map(|i| {
            let d = today - Duration::days(i);
            (d, daily.get(&d).map_or(0, |t| t.seconds))
        })
        .collect();
    let st = streaks(&daily, today, 1);
    let mut top: Vec<(u64, u64)> = book_totals(store.sessions())
        .into_iter()
        .filter_map(|(id, t)| id.parse().ok().map(|id| (id, t.seconds)))
        .collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    top.truncate(5);
    let year = chrono::Datelike::year(&today);
    Summary {
        today_secs: secs_since(1),
        week_secs: secs_since(7),
        month_secs: secs_since(30),
        total_secs: daily.values().map(|t| t.seconds).sum(),
        total_pages: daily.values().map(|t| t.pages).sum(),
        streak_current: st.current,
        streak_longest: st.longest,
        wpm: reading_speed_wpm(store.sessions()),
        books_finished_this_year: store
            .finished()
            .iter()
            .filter(|f| chrono::Datelike::year(&f.date) == year)
            .count() as u32,
        last_week,
        top_books: top,
    }
}

/// `"1 h 5 min"`-style text for a duration.
pub fn duration_text(secs: u64) -> String {
    bookx_lookup::stats::format_duration(secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(h: u32, m: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 12, h, m, s).unwrap()
    }

    #[test]
    fn first_event_only_starts_a_session() {
        let mut tr = Tracker::default();
        assert!(tr.on_position("1", t(9, 0, 0), (0, 0), 0.0, 300).is_empty());
        assert!(tr.is_active());
        let s = tr.finish(t(9, 0, 40)).unwrap();
        assert_eq!((s.pages, s.words), (0, 0));
        assert_eq!(s.duration_secs(), 40);
    }

    #[test]
    fn forward_page_turns_count_pages_and_words() {
        let mut tr = Tracker::default();
        tr.on_position("1", t(9, 0, 0), (0, 0), 0.00, 300);
        tr.on_position("1", t(9, 0, 30), (0, 1), 0.01, 300);
        tr.on_position("1", t(9, 1, 0), (0, 2), 0.02, 300);
        let s = tr.finish(t(9, 1, 10)).unwrap();
        assert_eq!(s.pages, 2);
        assert_eq!(s.words, 600);
        assert_eq!(s.characters, 3600);
        assert_eq!(s.duration_secs(), 70);
        assert!((s.start_pos - 0.0).abs() < 1e-9 && (s.end_pos - 0.02).abs() < 1e-9);
    }

    #[test]
    fn jumps_and_backwards_moves_are_not_reading() {
        let mut tr = Tracker::default();
        tr.on_position("1", t(9, 0, 0), (0, 0), 0.0, 300);
        tr.on_position("1", t(9, 0, 10), (5, 0), 0.5, 300); // slider jump
        tr.on_position("1", t(9, 0, 20), (5, 1), 0.51, 300); // one real page
        tr.on_position("1", t(9, 0, 30), (5, 0), 0.50, 300); // went back
        let s = tr.finish(t(9, 0, 40)).unwrap();
        assert_eq!(s.pages, 1);
    }

    #[test]
    fn same_page_events_do_not_count() {
        let mut tr = Tracker::default();
        tr.on_position("1", t(9, 0, 0), (0, 3), 0.1, 300);
        for i in 1..5 {
            tr.on_position("1", t(9, 0, i * 5), (0, 3), 0.1, 300); // resize, relayout
        }
        assert_eq!(tr.finish(t(9, 0, 30)).unwrap().pages, 0);
    }

    #[test]
    fn idle_time_ends_the_session_at_the_last_page_turn() {
        let mut tr = Tracker::default();
        tr.on_position("1", t(9, 0, 0), (0, 0), 0.0, 300);
        tr.on_position("1", t(9, 2, 0), (0, 1), 0.01, 300);
        // 20 minutes later the next page turn starts a new session
        let ended = tr.on_position("1", t(9, 22, 0), (0, 2), 0.02, 300);
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].end, t(9, 2, 0));
        assert_eq!(ended[0].pages, 1);
        // and finishing after a long idle also ends at the last event
        let s = tr.finish(t(10, 30, 0)).unwrap();
        assert_eq!(s.end, t(9, 22, 0));
    }

    #[test]
    fn switching_books_ends_the_session() {
        let mut tr = Tracker::default();
        tr.on_position("1", t(9, 0, 0), (0, 0), 0.0, 300);
        let ended = tr.on_position("2", t(9, 1, 0), (0, 0), 0.0, 300);
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].book_id, "1");
        assert_eq!(tr.finish(t(9, 1, 30)).unwrap().book_id, "2");
    }

    #[test]
    fn finish_without_a_session_is_none() {
        let mut tr = Tracker::default();
        assert!(tr.finish(t(9, 0, 0)).is_none());
        tr.on_position("1", t(9, 0, 0), (0, 0), 0.0, 1);
        assert!(tr.finish(t(9, 0, 10)).is_some());
        assert!(tr.finish(t(9, 0, 20)).is_none());
    }

    #[test]
    fn clock_going_backwards_never_panics() {
        let mut tr = Tracker::default();
        tr.on_position("1", t(9, 0, 10), (0, 0), 0.0, 100);
        let s = tr.finish(t(9, 0, 0)).unwrap();
        assert_eq!(s.duration_secs(), 0);
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "bookx-stats-{tag}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d.join("stats.json")
    }

    #[test]
    fn hub_records_saves_and_reloads() {
        let path = tmp("hub");
        let mut hub = StatsHub::load(Some(path.clone()));
        hub.on_position(7, t(9, 0, 0), (0, 0), 0.0, 250);
        hub.on_position(7, t(9, 0, 45), (0, 1), 0.01, 250);
        hub.finish(t(9, 1, 30));
        assert_eq!(hub.store.sessions().len(), 1);
        let again = StatsHub::load(Some(path.clone()));
        assert_eq!(again.store.sessions().len(), 1);
        assert_eq!(again.store.sessions()[0].book_id, "7");
        assert_eq!(again.store.sessions()[0].pages, 1);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn hub_short_sessions_are_ignored_by_the_store() {
        let mut hub = StatsHub::load(None);
        hub.on_position(1, t(9, 0, 0), (0, 0), 0.0, 250);
        hub.finish(t(9, 0, 2)); // below min_session_secs
        assert!(hub.store.sessions().is_empty());
    }

    #[test]
    fn hub_survives_a_corrupt_file() {
        let path = tmp("corrupt");
        std::fs::write(&path, "{ not json").unwrap();
        let hub = StatsHub::load(Some(path.clone()));
        assert!(hub.store.sessions().is_empty());
        assert!(path.with_extension("json.corrupt").exists());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn hub_without_a_path_never_touches_disk() {
        let mut hub = StatsHub::load(None);
        hub.on_position(1, t(9, 0, 0), (0, 0), 0.0, 100);
        hub.on_position(1, t(9, 0, 30), (0, 1), 0.01, 100);
        hub.finish(t(9, 0, 50));
        assert_eq!(hub.store.sessions().len(), 1);
    }

    fn store_with(sessions: &[(&str, DateTime<Utc>, i64, u64)]) -> StatsStore {
        let mut s = StatsStore::default();
        for (book, start, secs, words) in sessions {
            s.record(Session {
                book_id: book.to_string(),
                start: *start,
                end: *start + Duration::seconds(*secs),
                pages: 5,
                words: *words,
                characters: *words * 6,
                start_pos: 0.0,
                end_pos: 0.1,
            })
            .unwrap();
        }
        s
    }

    #[test]
    fn summary_of_an_empty_store_is_all_zero() {
        let sum = summarize(&StatsStore::default(), t(12, 0, 0), &Utc);
        assert_eq!(sum.today_secs, 0);
        assert_eq!(sum.total_secs, 0);
        assert_eq!((sum.streak_current, sum.streak_longest), (0, 0));
        assert_eq!(sum.wpm, None);
        assert!(sum.top_books.is_empty());
        assert_eq!(sum.last_week.len(), 7);
        assert!(sum.last_week.iter().all(|(_, s)| *s == 0));
        assert_eq!(sum.last_week[6].0, t(12, 0, 0).date_naive());
    }

    #[test]
    fn summary_adds_up_days_weeks_and_months() {
        let day = |d: u32| Utc.with_ymd_and_hms(2026, 11, d, 10, 0, 0).unwrap();
        let s = store_with(&[
            ("1", day(12), 600, 2000), // today
            ("1", day(11), 300, 900),
            ("2", day(6), 1200, 4000), // 6 days ago = still in the week
            ("2", day(5), 100, 300),   // 7 days ago = outside the week, inside the month
            (
                "3",
                Utc.with_ymd_and_hms(2026, 9, 1, 10, 0, 0).unwrap(),
                60,
                100,
            ),
        ]);
        let sum = summarize(&s, day(12) + Duration::hours(5), &Utc);
        assert_eq!(sum.today_secs, 600);
        assert_eq!(sum.week_secs, 600 + 300 + 1200);
        assert_eq!(sum.month_secs, 600 + 300 + 1200 + 100);
        assert_eq!(sum.total_secs, 600 + 300 + 1200 + 100 + 60);
        assert_eq!(sum.total_pages, 25);
        assert_eq!(sum.top_books, vec![(2, 1300), (1, 900), (3, 60)]);
        assert_eq!(sum.last_week[6], (day(12).date_naive(), 600));
        assert_eq!(sum.last_week[5], (day(11).date_naive(), 300));
        assert_eq!(sum.last_week[0], (day(6).date_naive(), 1200));
    }

    #[test]
    fn summary_counts_streaks_and_speed() {
        let day = |d: u32| Utc.with_ymd_and_hms(2026, 11, d, 10, 0, 0).unwrap();
        let s = store_with(&[
            ("1", day(12), 600, 2400),
            ("1", day(11), 600, 2400),
            ("1", day(10), 600, 2400),
            ("1", day(5), 600, 2400),
            ("1", day(4), 600, 2400),
        ]);
        let sum = summarize(&s, day(12) + Duration::hours(2), &Utc);
        assert_eq!(sum.streak_current, 3);
        assert_eq!(sum.streak_longest, 3);
        let wpm = sum.wpm.unwrap();
        assert!((wpm - 240.0).abs() < 1.0, "{wpm}");
    }

    #[test]
    fn current_streak_survives_until_the_day_ends() {
        let day = |d: u32| Utc.with_ymd_and_hms(2026, 11, d, 10, 0, 0).unwrap();
        let s = store_with(&[("1", day(11), 600, 100), ("1", day(10), 600, 100)]);
        // nothing read yet today: the streak still counts back from yesterday
        let sum = summarize(&s, day(12), &Utc);
        assert_eq!(sum.streak_current, 2);
        // a whole day without reading breaks it
        let sum = summarize(&s, day(13), &Utc);
        assert_eq!(sum.streak_current, 0);
        assert_eq!(sum.streak_longest, 2);
    }

    #[test]
    fn day_start_offset_moves_late_night_reading_to_the_previous_day() {
        let mut s = StatsStore::default();
        s.config.day_start_minutes = 240; // the day starts at 04:00
        let late = Utc.with_ymd_and_hms(2026, 11, 12, 1, 0, 0).unwrap();
        s.record(Session {
            book_id: "1".into(),
            start: late,
            end: late + Duration::seconds(900),
            pages: 3,
            words: 600,
            characters: 3600,
            start_pos: 0.0,
            end_pos: 0.1,
        })
        .unwrap();
        // at 09:00 on the 12th that reading belongs to the 11th
        let sum = summarize(
            &s,
            Utc.with_ymd_and_hms(2026, 11, 12, 9, 0, 0).unwrap(),
            &Utc,
        );
        assert_eq!(sum.today_secs, 0);
        assert_eq!(sum.last_week[5].1, 900);
        // at 02:00 on the 12th the same reading is "today"
        let sum = summarize(
            &s,
            Utc.with_ymd_and_hms(2026, 11, 12, 2, 0, 0).unwrap(),
            &Utc,
        );
        assert_eq!(sum.today_secs, 900);
    }

    #[test]
    fn finished_books_count_per_year() {
        let mut s = StatsStore::default();
        s.mark_finished("1", NaiveDate::from_ymd_opt(2026, 3, 1).unwrap());
        s.mark_finished("2", NaiveDate::from_ymd_opt(2026, 9, 1).unwrap());
        s.mark_finished("3", NaiveDate::from_ymd_opt(2025, 12, 31).unwrap());
        let sum = summarize(&s, t(12, 0, 0), &Utc);
        assert_eq!(sum.books_finished_this_year, 2);
    }

    #[test]
    fn non_numeric_book_ids_are_left_out_of_the_top_list() {
        let day = Utc.with_ymd_and_hms(2026, 11, 12, 10, 0, 0).unwrap();
        let s = store_with(&[("abc", day, 600, 100), ("4", day, 300, 100)]);
        let sum = summarize(&s, day, &Utc);
        assert_eq!(sum.top_books, vec![(4, 300)]);
    }

    #[test]
    fn duration_text_is_human() {
        assert_eq!(duration_text(0), "0 min");
        assert_eq!(duration_text(3900), "1 h 05 min");
    }
}
