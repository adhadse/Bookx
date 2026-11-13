//! Reading goals, the year calendar and the yearly review (R63), without GTK. The numbers come
//! from `bookx_lookup::stats`; this file turns them into what the statistics window shows.

use crate::i18n::tr;
use crate::i18n::trf;
use std::collections::BTreeMap;

use bookx_lookup::stats::{
    daily_goal_progress, daily_totals, heatmap, streaks, yearly_goal_progress, Goals, Heatmap,
    Progress, StatsStore, Totals,
};
use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc, Weekday};
use serde::{Deserialize, Serialize};

/// The goals the user set; 0 means no goal.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GoalPrefs {
    pub daily_minutes: u32,
    pub yearly_books: u32,
}

impl GoalPrefs {
    fn goals(&self) -> Goals {
        Goals {
            daily_minutes: (self.daily_minutes > 0).then_some(self.daily_minutes),
            yearly_books: (self.yearly_books > 0).then_some(self.yearly_books),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GoalStatus {
    pub daily: Option<Progress>,
    pub yearly: Option<Progress>,
}

fn today_of<Tz: TimeZone>(store: &StatsStore, now: DateTime<Utc>, tz: &Tz) -> NaiveDate {
    bookx_lookup::stats::reading_day(now, store.config.day_start_minutes, tz)
}

fn daily<Tz: TimeZone>(store: &StatsStore, tz: &Tz) -> BTreeMap<NaiveDate, Totals> {
    daily_totals(store.sessions(), &store.config, tz)
}

pub fn status<Tz: TimeZone>(
    store: &StatsStore,
    prefs: &GoalPrefs,
    now: DateTime<Utc>,
    tz: &Tz,
) -> GoalStatus {
    let today = today_of(store, now, tz);
    let goals = prefs.goals();
    GoalStatus {
        daily: daily_goal_progress(&daily(store, tz), today, &goals),
        yearly: yearly_goal_progress(store.finished(), today.year(), &goals),
    }
}

/// "12 of 30 min", "Goal reached: 35 of 30 min".
pub fn text(p: &Progress, unit: &str) -> String {
    if p.reached {
        trf("Goal reached: {} of {} {}", &[&p.done, &p.target, &unit])
    } else {
        format!("{} of {} {unit}", p.done, p.target)
    }
}

/// The calendar of the current year, weeks starting on Monday.
pub fn year_calendar<Tz: TimeZone>(
    store: &StatsStore,
    now: DateTime<Utc>,
    tz: &Tz,
) -> Option<Heatmap> {
    let today = today_of(store, now, tz);
    heatmap(&daily(store, tz), today.year(), Weekday::Mon).ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review {
    pub year: i32,
    pub total_secs: u64,
    pub days_read: u32,
    pub pages: u64,
    pub books_finished: u32,
    pub longest_streak: u32,
    /// The day with the most reading and its seconds.
    pub best_day: Option<(NaiveDate, u64)>,
}

pub fn review<Tz: TimeZone>(store: &StatsStore, now: DateTime<Utc>, tz: &Tz) -> Review {
    let today = today_of(store, now, tz);
    let year = today.year();
    let all = daily(store, tz);
    let this: BTreeMap<NaiveDate, Totals> = all
        .iter()
        .filter(|(d, _)| d.year() == year)
        .map(|(d, t)| (*d, *t))
        .collect();
    let best_day = this
        .iter()
        .filter(|(_, t)| t.seconds > 0)
        .max_by(|a, b| a.1.seconds.cmp(&b.1.seconds).then(b.0.cmp(a.0)))
        .map(|(d, t)| (*d, t.seconds));
    Review {
        year,
        total_secs: this.values().map(|t| t.seconds).sum(),
        days_read: this.values().filter(|t| t.seconds > 0).count() as u32,
        pages: this.values().map(|t| t.pages).sum(),
        books_finished: store
            .finished()
            .iter()
            .filter(|f| f.date.year() == year)
            .count() as u32,
        longest_streak: streaks(&this, today, 1).longest,
        best_day,
    }
}

/// The lines of the review as the window shows them.
pub fn review_lines(r: &Review) -> Vec<(String, String)> {
    let mut v = vec![
        (
            tr("Time reading"),
            bookx_lookup::stats::format_duration(r.total_secs),
        ),
        (tr("Days you read"), r.days_read.to_string()),
        (tr("Pages turned"), r.pages.to_string()),
        (tr("Books finished"), r.books_finished.to_string()),
        (tr("Longest streak"), format!("{} days", r.longest_streak)),
    ];
    if let Some((d, s)) = r.best_day {
        v.push((
            tr("Best day"),
            format!(
                "{} ({})",
                d.format("%-d %B"),
                bookx_lookup::stats::format_duration(s)
            ),
        ));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_lookup::stats::{Session, StatsConfig};

    fn at(m: u32, d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, m, d, h, 0, 0).unwrap()
    }

    fn session(book: &str, start: DateTime<Utc>, mins: i64, pages: u32) -> Session {
        Session {
            book_id: book.into(),
            start,
            end: start + chrono::Duration::minutes(mins),
            pages,
            words: pages as u64 * 250,
            characters: 0,
            start_pos: 0.0,
            end_pos: 0.1,
        }
    }

    fn store() -> StatsStore {
        let mut s = StatsStore::new(StatsConfig::default()).unwrap();
        s.record(session("1", at(11, 10, 20), 20, 10)).unwrap();
        s.record(session("1", at(11, 11, 20), 45, 25)).unwrap();
        s.record(session("2", at(11, 12, 20), 10, 4)).unwrap();
        s.record(session("2", at(3, 1, 20), 30, 12)).unwrap();
        s.mark_finished("1", NaiveDate::from_ymd_opt(2026, 11, 11).unwrap());
        s.mark_finished("2", NaiveDate::from_ymd_opt(2025, 5, 1).unwrap());
        s
    }

    #[test]
    fn no_goal_means_no_progress() {
        let st = status(&store(), &GoalPrefs::default(), at(11, 12, 21), &Utc);
        assert_eq!((st.daily, st.yearly), (None, None));
    }

    #[test]
    fn goals_count_today_and_this_year() {
        let prefs = GoalPrefs {
            daily_minutes: 30,
            yearly_books: 12,
        };
        let st = status(&store(), &prefs, at(11, 12, 21), &Utc);
        let d = st.daily.unwrap();
        assert_eq!((d.done, d.target, d.reached), (10, 30, false));
        assert_eq!(text(&d, "min"), "10 of 30 min");
        let y = st.yearly.unwrap();
        assert_eq!((y.done, y.target), (1, 12));
        // the day before, the 45 minutes reach the goal
        let st = status(&store(), &prefs, at(11, 11, 22), &Utc);
        let d = st.daily.unwrap();
        assert!(d.reached);
        assert_eq!(text(&d, "min"), "Goal reached: 45 of 30 min");
    }

    #[test]
    fn the_calendar_covers_the_current_year() {
        let h = year_calendar(&store(), at(11, 12, 21), &Utc).unwrap();
        assert_eq!(h.year, 2026);
        assert_eq!(h.columns.len(), h.levels.len());
        let cells: u64 = h.columns.iter().flatten().flatten().count() as u64;
        assert_eq!(cells, 365);
        let read_days = h
            .levels
            .iter()
            .flatten()
            .flatten()
            .filter(|l| **l > 0)
            .count();
        assert_eq!(read_days, 4);
        // the longest day is the darkest
        assert_eq!(h.levels.iter().flatten().flatten().max(), Some(&4));
    }

    #[test]
    fn the_review_sums_the_year_only() {
        let r = review(&store(), at(11, 12, 21), &Utc);
        assert_eq!(r.year, 2026);
        assert_eq!(r.total_secs, (20 + 45 + 10 + 30) * 60);
        assert_eq!(r.days_read, 4);
        assert_eq!(r.pages, 51);
        assert_eq!(r.books_finished, 1);
        assert_eq!(r.longest_streak, 3);
        assert_eq!(r.best_day.unwrap().1, 45 * 60);
        let lines = review_lines(&r);
        assert!(lines
            .iter()
            .any(|(k, v)| k == "Best day" && v.starts_with("11 November")));
        let empty = review(
            &StatsStore::new(StatsConfig::default()).unwrap(),
            at(1, 2, 3),
            &Utc,
        );
        assert_eq!(
            (empty.total_secs, empty.days_read, empty.best_day),
            (0, 0, None)
        );
        assert_eq!(review_lines(&empty).len(), 5);
    }
}
