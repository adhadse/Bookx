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

pub fn show(parent: &adw::ApplicationWindow, shared: &SharedRef) {
    let now = shared.now();
    let summary = {
        let hub = shared.stats.borrow();
        summarize(&hub.store, now, &chrono::Local)
    };

    let dialog = adw::Dialog::new();
    dialog.set_title(&tr("Reading Statistics"));
    dialog.set_content_width(460);
    dialog.set_content_height(640);

    let page = adw::PreferencesPage::new();

    let numbers = adw::PreferencesGroup::new();
    for (title, value) in summary_lines(&summary) {
        let row = adw::ActionRow::new();
        row.set_title(&title);
        let l = gtk::Label::new(Some(&value));
        l.add_css_class("dim-label");
        row.add_suffix(&l);
        numbers.add(&row);
    }
    page.add(&numbers);

    page.add(&goals_group(shared));

    let chart_group = adw::PreferencesGroup::new();
    chart_group.set_title(&tr("Last 7 days"));
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
                .unwrap_or_else(|| "(removed book)".to_string());
            let row = adw::ActionRow::new();
            row.set_title(&glib_escape(&title));
            let l = gtk::Label::new(Some(&duration_text(*secs)));
            l.add_css_class("dim-label");
            row.add_suffix(&l);
            top.add(&row);
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
        let row = adw::ActionRow::new();
        row.set_title(&title);
        let l = gtk::Label::new(Some(&value));
        l.add_css_class("dim-label");
        row.add_suffix(&l);
        rg.add(&row);
    }
    page.add(&rg);

    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&page));
    dialog.set_child(Some(&view));
    dialog.present(Some(parent));
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

fn goals_group(shared: &SharedRef) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Goals"));
    g.set_description(Some(&tr(
        "Set a goal to see how far you are. 0 means no goal.",
    )));
    let prefs = shared.cfg.borrow().goals;
    let make = |title: &str, value: u32, max: f64, step: f64| {
        let r = adw::SpinRow::with_range(0.0, max, step);
        r.set_title(title);
        r.set_value(value as f64);
        r
    };
    let daily = make("Minutes a day", prefs.daily_minutes, 720.0, 5.0);
    let yearly = make("Books a year", prefs.yearly_books, 1000.0, 1.0);
    let bars = (gtk::ProgressBar::new(), gtk::ProgressBar::new());
    for (row, bar) in [(&daily, &bars.0), (&yearly, &bars.1)] {
        bar.set_valign(gtk::Align::Center);
        bar.set_width_request(110);
        bar.update_property(&[gtk::accessible::Property::Label(&trf(
            "Progress of the goal {}",
            &[&row.title()],
        ))]);
        row.add_suffix(bar);
    }
    let refresh = {
        let (shared, daily, yearly, b0, b1) = (
            shared.clone(),
            daily.clone(),
            yearly.clone(),
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
                row.set_subtitle(&p.map(|p| crate::goals::text(&p, unit)).unwrap_or_default());
                bar.set_fraction(p.map_or(0.0, |p| p.fraction.min(1.0)));
            }
        }
    };
    refresh();
    let refresh = std::rc::Rc::new(refresh);
    for (row, is_daily) in [(&daily, true), (&yearly, false)] {
        let (shared, refresh) = (shared.clone(), refresh.clone());
        row.connect_value_notify(move |r| {
            {
                let mut c = shared.cfg.borrow_mut();
                if is_daily {
                    c.goals.daily_minutes = r.value() as u32;
                } else {
                    c.goals.yearly_books = r.value() as u32;
                }
                c.save();
            }
            refresh();
        });
    }
    g.add(&daily);
    g.add(&yearly);
    g
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
