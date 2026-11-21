//! The reading statistics window (roadmap R11/R40).

use crate::i18n::ntr;
use crate::i18n::{tr, trf};
use relm4::adw::{self, prelude::*};
use relm4::gtk;

use super::SharedRef;
use crate::reading_stats::{duration_text, summarize, Summary};

/// Plain-text lines of the summary, used by the window and by the UI tests.
pub fn summary_lines(s: &Summary) -> Vec<(String, String)> {
    let mut v = vec![
        ("Today".to_string(), duration_text(s.today_secs)),
        ("Last 7 days".to_string(), duration_text(s.week_secs)),
        ("Last 30 days".to_string(), duration_text(s.month_secs)),
        ("All time".to_string(), duration_text(s.total_secs)),
        (
            "Streak".to_string(),
            trf(
                "{} (longest {})",
                &[
                    &ntr("{} day", "{} days", s.streak_current),
                    &s.streak_longest,
                ],
            ),
        ),
        (
            "Books finished this year".to_string(),
            s.books_finished_this_year.to_string(),
        ),
    ];
    v.push((
        "Reading speed".to_string(),
        match s.wpm {
            Some(w) => format!("{w:.0} words per minute"),
            None => "not enough data yet".to_string(),
        },
    ));
    v
}

/// No reading recorded at all: the window shows a hint instead of seven zero rows.
pub fn is_empty(s: &Summary) -> bool {
    s.total_secs == 0 && s.books_finished_this_year == 0 && s.streak_longest == 0
}

/// `summary_lines` for the window: the same rows, translated.
fn window_lines(s: &Summary) -> Vec<(String, String)> {
    let mut lines = summary_lines(s);
    for (title, _) in lines.iter_mut() {
        *title = match title.as_str() {
            "Today" => tr("Today"),
            "Last 7 days" => tr("Last 7 Days"),
            "Last 30 days" => tr("Last 30 Days"),
            "All time" => tr("All Time"),
            "Streak" => tr("Streak"),
            "Books finished this year" => tr("Books Finished This Year"),
            _ => tr("Reading Speed"),
        };
    }
    if let Some((_, v)) = lines.last_mut() {
        *v = match s.wpm {
            Some(w) => trf("{} words per minute", &[&format!("{w:.0}")]),
            None => tr("Not enough data yet"),
        };
    }
    lines
}

pub fn show(parent: &adw::ApplicationWindow, shared: &SharedRef) {
    let now = shared.now();
    let summary = {
        let hub = shared.stats.borrow();
        summarize(&hub.store, now, &chrono::Local)
    };

    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(&tr("Reading Statistics"));
    dialog.set_content_width(460);
    dialog.set_content_height(560);

    let page = adw::PreferencesPage::new();

    let numbers = adw::PreferencesGroup::new();
    if !is_empty(&summary) {
        numbers.set_description(Some(&trf(
            "Read {} today",
            &[&duration_text(summary.today_secs)],
        )));
    }
    if is_empty(&summary) {
        let hint = adw::StatusPage::new();
        hint.set_icon_name(Some("view-statistics-symbolic"));
        hint.set_title(&tr("No Reading Yet"));
        hint.set_description(Some(&tr(
            "Read a book and your time, streak and speed appear here",
        )));
        hint.add_css_class("compact");
        numbers.add(&hint);
    }
    for (title, value) in window_lines(&summary)
        .into_iter()
        .filter(|_| !is_empty(&summary))
    {
        numbers.add(&property_row(&title, &value));
    }
    page.add(&numbers);

    page.add(&goals_group(shared));

    let chart_group = adw::PreferencesGroup::new();
    chart_group.set_title(&tr("Last 7 Days"));
    chart_group.add(&week_chart(&summary));
    page.add(&chart_group);

    if !summary.top_books.is_empty() {
        let top = adw::PreferencesGroup::new();
        top.set_title(&tr("Most read"));
        let lib = shared.lib.borrow();
        for (id, secs) in &summary.top_books {
            let title = lib
                .get(*id)
                .map(|b| b.meta.title.clone())
                .unwrap_or_else(|| tr("(removed book)"));
            top.add(&property_row(&glib_escape(&title), &duration_text(*secs)));
        }
        page.add(&top);
    }

    if let Some(g) = super::audiobook_player::listening_group(shared) {
        page.add(&g);
    }

    if let Some(cal) =
        crate::goals::year_calendar(&shared.stats.borrow().store, now, &chrono::Local)
    {
        let g = adw::PreferencesGroup::new();
        g.set_title(&trf("Reading in {}", &[&cal.year]));
        g.add(&year_calendar(&cal));
        page.add(&g);
    }
    let review = crate::goals::review(&shared.stats.borrow().store, now, &chrono::Local);
    let rg = adw::PreferencesGroup::new();
    rg.set_title(&trf("{} in review", &[&review.year]));
    for (title, value) in crate::goals::review_lines(&review) {
        rg.add(&property_row(&title, &value));
    }
    page.add(&rg);

    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&page));
    dialog.set_child(Some(&view));
    dialog.present(Some(parent));
}

/// One number of the window: the value is the primary text (libadwaita `property` style), the name sits under it.
fn property_row(name: &str, value: &str) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.add_css_class("property");
    row.set_title(name);
    row.set_subtitle(value);
    row.set_subtitle_selectable(true);
    row
}

/// The week chart said in words, for screen readers: `Mon: 12 min, Tue: 0 min, …`.
pub fn week_description(days: &[(String, u64)]) -> String {
    days.iter()
        .map(|(d, secs)| trf("{}: {}", &[d, &duration_text(*secs)]))
        .collect::<Vec<_>>()
        .join(", ")
}

/// What a goal switch writes to the settings: off is 0, on keeps the remembered amount (at least 1).
pub fn goal_value(on: bool, remembered: u32, default: u32) -> u32 {
    if !on {
        0
    } else if remembered == 0 {
        default
    } else {
        remembered
    }
}

fn glib_escape(s: &str) -> String {
    relm4::gtk::glib::markup_escape_text(s).to_string()
}

fn week_chart(s: &Summary) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_height(120);
    area.set_hexpand(true);
    let data: Vec<(String, u64)> = s
        .last_week
        .iter()
        .map(|(d, secs)| (d.format("%a").to_string(), *secs))
        .collect();
    area.update_property(&[
        gtk::accessible::Property::Label(&tr("Reading time, last 7 days")),
        gtk::accessible::Property::Description(&week_description(&data)),
    ]);
    CHART_TEXT
        .with(|t| *t.borrow_mut() = (tr("Reading time, last 7 days"), week_description(&data)));
    area.set_draw_func(move |a, cr, w, h| {
        let fg = a.color();
        let max = data.iter().map(|d| d.1).max().unwrap_or(0).max(60) as f64;
        let n = data.len().max(1) as f64;
        let slot = w as f64 / n;
        let bar_w = slot * 0.55;
        let label_h = 18.0;
        let chart_h = h as f64 - label_h;
        for (i, (label, secs)) in data.iter().enumerate() {
            let x = i as f64 * slot + (slot - bar_w) / 2.0;
            let bh = (*secs as f64 / max * (chart_h - 4.0)).max(if *secs > 0 { 2.0 } else { 0.0 });
            cr.set_source_rgba(
                fg.red() as f64,
                fg.green() as f64,
                fg.blue() as f64,
                if *secs > 0 { 0.75 } else { 0.12 },
            );
            let y = chart_h - bh.max(2.0);
            cr.rectangle(x, y, bar_w, bh.max(2.0));
            let _ = cr.fill();
            cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.6);
            cr.set_font_size(11.0);
            if let Ok(ext) = cr.text_extents(label) {
                cr.move_to(x + (bar_w - ext.width()) / 2.0, h as f64 - 4.0);
                let _ = cr.show_text(label);
            }
        }
    });
    area
}

/// The current goal texts, `(daily, yearly)`; empty when no goal is set.
pub fn goal_texts(shared: &SharedRef) -> (String, String) {
    let prefs = shared.cfg.borrow().goals;
    let st = crate::goals::status(
        &shared.stats.borrow().store,
        &prefs,
        shared.now(),
        &chrono::Local,
    );
    (
        st.daily
            .map(|p| crate::goals::text(&p, "min today"))
            .unwrap_or_default(),
        st.yearly
            .map(|p| crate::goals::text(&p, "books this year"))
            .unwrap_or_default(),
    )
}

thread_local! {
    static CHART_TEXT: std::cell::RefCell<(String, String)> =
        const { std::cell::RefCell::new((String::new(), String::new())) };
    static GOALS: std::cell::RefCell<Option<GoalRows>> = const { std::cell::RefCell::new(None) };
}

struct GoalRows {
    daily: (adw::SwitchRow, adw::SpinRow),
    yearly: (adw::SwitchRow, adw::SpinRow),
}

const DEFAULT_DAILY: u32 = 30;
const DEFAULT_YEARLY: u32 = 12;

fn goals_group(shared: &SharedRef) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Goals"));
    g.set_description(Some(&tr("Switch a goal on to see how far you are")));
    let prefs = shared.cfg.borrow().goals;
    let make = |title: &str, value: u32, default: u32, max: f64, step: f64| {
        let sw = adw::SwitchRow::new();
        sw.set_title(title);
        sw.set_active(value > 0);
        let r = adw::SpinRow::with_range(1.0, max, step);
        r.set_title(&tr("Amount"));
        r.set_value(goal_value(true, value, default) as f64);
        r.set_visible(value > 0);
        (sw, r)
    };
    let daily = make(
        &tr("Daily goal"),
        prefs.daily_minutes,
        DEFAULT_DAILY,
        720.0,
        5.0,
    );
    let yearly = make(
        &tr("Yearly goal"),
        prefs.yearly_books,
        DEFAULT_YEARLY,
        1000.0,
        1.0,
    );
    daily.1.set_subtitle(&tr("Minutes a day"));
    yearly.1.set_subtitle(&tr("Books a year"));
    let bars = (gtk::ProgressBar::new(), gtk::ProgressBar::new());
    for ((_, row), bar) in [(&daily, &bars.0), (&yearly, &bars.1)] {
        bar.set_valign(gtk::Align::Center);
        bar.set_width_request(110);
        bar.update_property(&[gtk::accessible::Property::Label(&trf(
            "Progress of the goal {}",
            &[&row.subtitle().unwrap_or_default()],
        ))]);
        row.add_suffix(bar);
    }
    let refresh = {
        let (shared, daily, yearly, b0, b1) = (
            shared.clone(),
            daily.1.clone(),
            yearly.1.clone(),
            bars.0.clone(),
            bars.1.clone(),
        );
        move || {
            let prefs = shared.cfg.borrow().goals;
            let st = crate::goals::status(
                &shared.stats.borrow().store,
                &prefs,
                shared.now(),
                &chrono::Local,
            );
            for (row, bar, p, unit) in [
                (&daily, &b0, st.daily, "min today"),
                (&yearly, &b1, st.yearly, "books this year"),
            ] {
                bar.set_visible(p.is_some());
                let text = p.map(|p| crate::goals::text(&p, unit)).unwrap_or_default();
                if !text.is_empty() {
                    row.set_tooltip_text(Some(&text));
                }
                bar.set_fraction(p.map_or(0.0, |p| p.fraction.min(1.0)));
            }
        }
    };
    refresh();
    let refresh = std::rc::Rc::new(refresh);
    for ((sw, spin), is_daily) in [(&daily, true), (&yearly, false)] {
        let default = if is_daily {
            DEFAULT_DAILY
        } else {
            DEFAULT_YEARLY
        };
        let write = {
            let (shared, refresh, sw, spin) =
                (shared.clone(), refresh.clone(), sw.clone(), spin.clone());
            move || {
                let v = goal_value(sw.is_active(), spin.value() as u32, default);
                spin.set_visible(sw.is_active());
                {
                    let mut c = shared.cfg.borrow_mut();
                    if is_daily {
                        c.goals.daily_minutes = v;
                    } else {
                        c.goals.yearly_books = v;
                    }
                    c.save();
                }
                refresh();
            }
        };
        let w1 = write.clone();
        sw.connect_active_notify(move |_| w1());
        spin.connect_value_notify(move |_| write());
    }
    for (sw, spin) in [&daily, &yearly] {
        g.add(sw);
        g.add(spin);
    }
    GOALS.with(|c| {
        *c.borrow_mut() = Some(GoalRows {
            daily: daily.clone(),
            yearly: yearly.clone(),
        })
    });
    g
}

/// Dev hook (`prefs:stats-dump`, `prefs:stats-daily=on|off`, `prefs:stats-daily-amount=N`): the window's goal rows
/// and chart, as `stats_ui: chart_label=… chart_desc=… daily_on=B daily_amount_visible=B daily_amount=N yearly_on=B`.
pub fn dev(arg: &str) {
    GOALS.with(|c| {
        let c = c.borrow();
        let Some(g) = c.as_ref() else {
            eprintln!("stats_ui: not open");
            return;
        };
        if let Some(v) = arg.strip_prefix("daily=") {
            g.daily.0.set_active(v == "on");
        }
        if let Some(v) = arg.strip_prefix("daily-amount=") {
            g.daily.1.set_value(v.parse().unwrap_or(1.0));
        }
        if arg == "dump" {
            let (l, d) = CHART_TEXT.with(|t| t.borrow().clone());
            eprintln!(
                "stats_ui: chart_label={l:?} chart_desc={} daily_on={} daily_amount_visible={} daily_amount={} yearly_on={} yearly_amount_visible={}",
                !d.is_empty(),
                g.daily.0.is_active(),
                g.daily.1.is_visible(),
                g.daily.1.value(),
                g.yearly.0.is_active(),
                g.yearly.1.is_visible()
            );
        }
    });
}

fn year_calendar(cal: &bookx_lookup::stats::Heatmap) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_height(7 * 11 + 4);
    area.set_hexpand(true);
    area.update_property(&[gtk::accessible::Property::Label(&tr(
        "Calendar of the days you read this year",
    ))]);
    let levels = cal.levels.clone();
    area.set_draw_func(move |a, cr, w, _h| {
        let fg = a.color();
        let cols = levels.len().max(1) as f64;
        let cell = (w as f64 / cols).min(14.0);
        for (c, col) in levels.iter().enumerate() {
            for (r, lvl) in col.iter().enumerate() {
                let Some(l) = lvl else { continue };
                let alpha = 0.10 + 0.22 * (*l as f64);
                cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, alpha);
                cr.rectangle(c as f64 * cell, r as f64 * cell, cell - 2.0, cell - 2.0);
                let _ = cr.fill();
            }
        }
    });
    area
}

/// Test hook (`stats:goal:MINUTES:BOOKS`): sets the goals as the window's rows do.
pub fn dev_goals(shared: &SharedRef, arg: &str) {
    let mut it = arg.split(':').map(|x| x.parse::<u32>().unwrap_or(0));
    {
        let mut c = shared.cfg.borrow_mut();
        c.goals.daily_minutes = it.next().unwrap_or(0);
        c.goals.yearly_books = it.next().unwrap_or(0);
        c.save();
    }
    let (d, y) = goal_texts(shared);
    eprintln!("stats_goal_daily: {d}");
    eprintln!("stats_goal_yearly: {y}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goal_switch_maps_to_the_stored_amount() {
        assert_eq!(goal_value(false, 45, 30), 0);
        assert_eq!(goal_value(true, 0, 30), 30);
        assert_eq!(goal_value(true, 45, 30), 45);
    }

    #[test]
    fn the_week_chart_is_described_day_by_day() {
        let d = week_description(&[("Mon".into(), 600), ("Tue".into(), 0)]);
        assert!(d.starts_with("Mon: "));
        assert!(d.contains(", Tue: "));
    }

    #[test]
    fn a_default_summary_is_empty_and_reading_makes_it_not() {
        let mut s = Summary::default();
        assert!(is_empty(&s));
        s.total_secs = 60;
        assert!(!is_empty(&s));
    }
}
