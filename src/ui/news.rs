//! The *News Subscriptions* window (roadmap S4/S5): the list of subscriptions with their schedule
//! and last result, and a form to add or edit one. Edits are saved at once; running is done by the
//! library component (it owns the library and the worker threads).

use crate::i18n::ntr;
use crate::i18n::tr;
use crate::i18n::trf;
use std::cell::RefCell;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, glib};

use super::alert;
use crate::news::{
    feed_lines, recipe_from_form, schedule_text, status_text, NewsStore, ScheduleChoice,
    Subscription, DEFAULT_KEEP, WEEKDAYS,
};

/// Lets the owner redraw the list of an open window after a download finished.
pub type RefreshSlot = Rc<RefCell<Option<Rc<dyn Fn()>>>>;

#[derive(Clone)]
pub struct Ctx {
    pub store: Rc<RefCell<NewsStore>>,
    /// Download these subscriptions now.
    pub run_keys: Rc<dyn Fn(Vec<String>)>,
    /// Download every subscription that is due.
    pub run_due: Rc<dyn Fn()>,
    pub slot: RefreshSlot,
}

fn offset() -> chrono::FixedOffset {
    *chrono::Local::now().offset()
}

pub fn show(parent: &impl IsA<gtk::Widget>, ctx: Ctx) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&tr("News Subscriptions"));
    dialog.set_content_width(560);
    dialog.set_content_height(560);

    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    group.set_description(Some(
        "Each subscription makes a new book from its feeds on its schedule. Old issues are removed.",
    ));
    let add_btn = gtk::Button::with_label(&tr("Add"));
    add_btn.add_css_class("suggested-action");
    add_btn.set_valign(gtk::Align::Center);
    add_btn.set_widget_name("news-add");
    let due_btn = gtk::Button::with_label(&tr("Download All Due"));
    due_btn.set_valign(gtk::Align::Center);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    head.append(&due_btn);
    head.append(&add_btn);
    group.set_header_suffix(Some(&head));
    page.add(&group);

    let rows: Rc<RefCell<Vec<adw::ActionRow>>> = Rc::new(RefCell::new(Vec::new()));
    let refresh: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));

    let build = {
        let group = group.clone();
        let rows = rows.clone();
        let ctx = ctx.clone();
        let refresh = refresh.clone();
        let parent = dialog.clone();
        move || {
            for r in rows.borrow_mut().drain(..) {
                group.remove(&r);
            }
            let off = offset();
            let subs: Vec<Subscription> = ctx.store.borrow().subs.clone();
            if subs.is_empty() {
                let row = adw::ActionRow::new();
                row.set_title(&tr("No subscriptions yet"));
                row.set_subtitle(&tr("Press Add to subscribe to a news feed."));
                row.set_activatable(false);
                group.add(&row);
                rows.borrow_mut().push(row);
            }
            for s in subs {
                let row = adw::ActionRow::new();
                row.set_title(&glib::markup_escape_text(&s.recipe.title));
                let feeds = s.recipe.feeds.len();
                row.set_subtitle(&glib::markup_escape_text(&trf(
                    "{} · {}\n{}",
                    &[
                        &ntr("{} feed", "{} feeds", feeds),
                        &schedule_text(&s, off),
                        &status_text(&s, off),
                    ],
                )));
                row.set_subtitle_lines(2);

                let again = refresh.clone();
                let again = move || {
                    if let Some(f) = again.borrow().clone() {
                        f();
                    }
                };

                let dl = gtk::Button::from_icon_name("folder-download-symbolic");
                dl.set_tooltip_text(Some(&tr("Download Now")));
                dl.set_valign(gtk::Align::Center);
                dl.add_css_class("flat");
                {
                    let key = s.key.clone();
                    let run = ctx.run_keys.clone();
                    dl.connect_clicked(move |_| run(vec![key.clone()]));
                }
                let edit_btn = gtk::Button::from_icon_name("document-edit-symbolic");
                edit_btn.set_tooltip_text(Some(&tr("Edit")));
                edit_btn.set_valign(gtk::Align::Center);
                edit_btn.add_css_class("flat");
                {
                    let ctx = ctx.clone();
                    let sub = s.clone();
                    let parent = parent.clone();
                    let again = again.clone();
                    edit_btn.connect_clicked(move |_| {
                        edit(
                            &parent,
                            ctx.clone(),
                            Some(sub.clone()),
                            Rc::new(again.clone()),
                        )
                    });
                }
                let del = gtk::Button::from_icon_name("user-trash-symbolic");
                del.set_tooltip_text(Some(&tr("Delete")));
                del.set_valign(gtk::Align::Center);
                del.add_css_class("flat");
                {
                    let store = ctx.store.clone();
                    let key = s.key.clone();
                    let name = s.recipe.title.clone();
                    let parent = parent.clone();
                    let again = again.clone();
                    del.connect_clicked(move |_| {
                        let store = store.clone();
                        let key = key.clone();
                        let again = again.clone();
                        alert(
                            &parent,
                            &trf("Delete “{}”?", &[&name]),
                            "The subscription is removed. Issues already in the library stay.",
                            "Delete",
                            true,
                            move || {
                                let _ = store.borrow_mut().remove(&key);
                                again();
                            },
                        );
                    });
                }
                row.add_suffix(&dl);
                row.add_suffix(&edit_btn);
                row.add_suffix(&del);
                group.add(&row);
                rows.borrow_mut().push(row);
            }
        }
    };
    let build = Rc::new(build);
    *refresh.borrow_mut() = Some(build.clone());
    build();

    {
        let ctx = ctx.clone();
        let build = build.clone();
        let d = dialog.clone();
        add_btn.connect_clicked(move |_| {
            let build = build.clone();
            edit(&d, ctx.clone(), None, Rc::new(move || build()));
        });
    }
    {
        let run = ctx.run_due.clone();
        due_btn.connect_clicked(move |_| run());
    }

    *ctx.slot.borrow_mut() = Some(build.clone());
    {
        let slot = ctx.slot.clone();
        let refresh = refresh.clone();
        dialog.connect_closed(move |_| {
            *slot.borrow_mut() = None;
            // break the reference cycle between the window and its own refresh closure
            *refresh.borrow_mut() = None;
        });
    }

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&page));
    dialog.set_child(Some(&toolbar));
    dialog.present(Some(parent));
}

const KINDS: [&str; 4] = ["Manual only", "Every hour", "Every day", "Every week"];

/// The form for a new subscription (`existing` is `None`) or an existing one.
fn edit(parent: &adw::Dialog, ctx: Ctx, existing: Option<Subscription>, done: Rc<dyn Fn()>) {
    let off = offset();
    let dialog = adw::Dialog::new();
    dialog.set_title(
        &(if existing.is_some() {
            tr("Edit Subscription")
        } else {
            tr("New Subscription")
        }),
    );
    dialog.set_content_width(520);
    dialog.set_content_height(640);

    let choice = ScheduleChoice::from_schedule(
        existing.as_ref().and_then(|s| s.recipe.schedule.as_ref()),
        off,
    );

    let page = adw::PreferencesPage::new();
    let basics = adw::PreferencesGroup::new();
    let title = adw::EntryRow::new();
    title.set_title(&tr("Name"));
    if let Some(s) = &existing {
        title.set_text(&s.recipe.title);
    }
    let fetch_title = gtk::Button::from_icon_name("view-refresh-symbolic");
    fetch_title.set_tooltip_text(Some(&tr("Take the name from the feed")));
    fetch_title.set_valign(gtk::Align::Center);
    fetch_title.add_css_class("flat");
    title.add_suffix(&fetch_title);
    basics.add(&title);

    let feeds_view = gtk::TextView::new();
    feeds_view.set_wrap_mode(gtk::WrapMode::WordChar);
    feeds_view.set_top_margin(8);
    feeds_view.set_bottom_margin(8);
    feeds_view.set_left_margin(12);
    feeds_view.set_right_margin(12);
    if let Some(s) = &existing {
        feeds_view.buffer().set_text(&s.feeds_text());
    }
    let feeds_scroll = gtk::ScrolledWindow::new();
    feeds_scroll.set_min_content_height(90);
    feeds_scroll.set_child(Some(&feeds_view));
    feeds_scroll.add_css_class("card");
    let feeds_group = adw::PreferencesGroup::new();
    feeds_group.set_title(&tr("Feed addresses"));
    feeds_group.set_description(Some(
        "One address per line. Start a line with # to switch that feed off.",
    ));
    let opml_btn = gtk::Button::with_label(&tr("Import OPML"));
    opml_btn.set_valign(gtk::Align::Center);
    opml_btn.set_widget_name("news-opml");
    feeds_group.set_header_suffix(Some(&opml_btn));
    feeds_group.add(&feeds_scroll);

    let content = adw::PreferencesGroup::new();
    let articles = adw::SpinRow::with_range(1.0, 100.0, 1.0);
    articles.set_title(&tr("Articles per feed"));
    articles.set_value(existing.as_ref().map_or(25, |s| s.recipe.max_articles) as f64);
    let full = adw::SwitchRow::new();
    full.set_title(&tr("Full articles"));
    full.set_subtitle(&tr(
        "Download each article's web page instead of using the feed text",
    ));
    full.set_active(existing.as_ref().is_some_and(|s| s.recipe.use_full_article));
    content.add(&articles);
    content.add(&full);

    let when = adw::PreferencesGroup::new();
    when.set_title(&tr("Schedule"));
    let kind = adw::ComboRow::new();
    kind.set_title(&tr("Download"));
    kind.set_model(Some(&gtk::StringList::new(&KINDS)));
    let day = adw::ComboRow::new();
    day.set_title(&tr("On"));
    let names: Vec<&str> = WEEKDAYS
        .iter()
        .map(|d| crate::news::weekday_name(*d))
        .collect();
    day.set_model(Some(&gtk::StringList::new(&names)));
    let time = adw::EntryRow::new();
    time.set_title(&tr("At (HH:MM, local time)"));
    time.set_text("07:00");
    match &choice {
        ScheduleChoice::Manual | ScheduleChoice::Custom(_) => kind.set_selected(0),
        ScheduleChoice::Hourly => kind.set_selected(1),
        ScheduleChoice::Daily { hour, minute } => {
            kind.set_selected(2);
            time.set_text(&format!("{hour:02}:{minute:02}"));
        }
        ScheduleChoice::Weekly {
            weekday,
            hour,
            minute,
        } => {
            kind.set_selected(3);
            day.set_selected(weekday.num_days_from_monday());
            time.set_text(&format!("{hour:02}:{minute:02}"));
        }
    }
    let sync = {
        let (kind, day, time) = (kind.clone(), day.clone(), time.clone());
        move || {
            day.set_visible(kind.selected() == 3);
            time.set_visible(kind.selected() >= 2);
        }
    };
    sync();
    kind.connect_selected_notify(move |_| sync());
    when.add(&kind);
    when.add(&day);
    when.add(&time);

    let keep_group = adw::PreferencesGroup::new();
    let keep = adw::SpinRow::with_range(1.0, 50.0, 1.0);
    keep.set_title(&tr("Issues to keep"));
    keep.set_subtitle(&tr(
        "Older issues are removed from the library after a download",
    ));
    keep.set_value(existing.as_ref().map_or(DEFAULT_KEEP, |s| s.keep_issues) as f64);
    let enabled = adw::SwitchRow::new();
    enabled.set_title(&tr("Download automatically"));
    enabled.set_active(existing.as_ref().is_none_or(|s| s.enabled));
    keep_group.add(&keep);
    keep_group.add(&enabled);

    page.add(&basics);
    page.add(&feeds_group);
    page.add(&content);
    page.add(&when);
    page.add(&keep_group);

    let banner = adw::Banner::new("");
    banner.set_revealed(false);

    // take the name from the feed on a worker thread
    {
        let title = title.clone();
        let feeds_view = feeds_view.clone();
        let banner = banner.clone();
        fetch_title.connect_clicked(move |b| {
            let buf = feeds_view.buffer();
            let text = buf.text(&buf.start_iter(), &buf.end_iter(), false);
            let first = text.lines().next().unwrap_or("").trim().to_string();
            b.set_sensitive(false);
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(
                    crate::news::feed_title(&first, &*crate::web_import::fetcher())
                        .map_err(|e| format!("{e:#}")),
                );
            });
            let (title, banner, b) = (title.clone(), banner.clone(), b.clone());
            glib::timeout_add_local(std::time::Duration::from_millis(60), move || {
                match rx.try_recv() {
                    Ok(r) => {
                        match r {
                            Ok(t) => title.set_text(&t),
                            Err(e) => {
                                banner.set_title(&e);
                                banner.set_revealed(true);
                            }
                        }
                        b.set_sensitive(true);
                        glib::ControlFlow::Break
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(_) => {
                        b.set_sensitive(true);
                        glib::ControlFlow::Break
                    }
                }
            });
        });
    }

    // add the feeds of an OPML file to the box
    {
        let feeds_view = feeds_view.clone();
        let banner = banner.clone();
        let dialog = dialog.clone();
        opml_btn.connect_clicked(move |_| {
            let chooser = gtk::FileDialog::new();
            chooser.set_title(&tr("Import OPML"));
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("OPML files"));
            filter.add_pattern("*.opml");
            filter.add_pattern("*.xml");
            let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            chooser.set_filters(Some(&filters));
            let (feeds_view, banner) = (feeds_view.clone(), banner.clone());
            let win = dialog.root().and_downcast::<gtk::Window>();
            chooser.open(win.as_ref(), gtk::gio::Cancellable::NONE, move |r| {
                let Ok(file) = r else { return };
                let Some(path) = file.path() else { return };
                match std::fs::read_to_string(&path)
                    .map_err(anyhow::Error::from)
                    .and_then(|t| crate::news::parse_opml(&t))
                {
                    Ok(found) => {
                        let buf = feeds_view.buffer();
                        let mut text = buf
                            .text(&buf.start_iter(), &buf.end_iter(), false)
                            .to_string();
                        for (_, url) in found {
                            if !text.is_empty() && !text.ends_with('\n') {
                                text.push('\n');
                            }
                            text.push_str(&url);
                        }
                        buf.set_text(&text);
                    }
                    Err(e) => {
                        banner.set_title(&format!("{e:#}"));
                        banner.set_revealed(true);
                    }
                }
            });
        });
    }

    let save = gtk::Button::with_label(&tr("Save"));
    save.add_css_class("suggested-action");
    {
        let dialog = dialog.clone();
        let banner = banner.clone();
        save.connect_clicked(move |_| {
            let buf = feeds_view.buffer();
            let feeds = buf.text(&buf.start_iter(), &buf.end_iter(), false);
            let sched = match kind.selected() {
                0 => Ok(ScheduleChoice::Manual),
                1 => Ok(ScheduleChoice::Hourly),
                2 => ScheduleChoice::parse(&format!("daily {}", time.text().trim())),
                _ => ScheduleChoice::parse(&format!(
                    "weekly {} {}",
                    WEEKDAYS[day.selected() as usize % 7],
                    time.text().trim()
                )),
            };
            // a schedule written by hand (cron) stays as it was when "Manual" is not touched
            let sched = match (&choice, kind.selected()) {
                (ScheduleChoice::Custom(_), 0) => Ok(choice.clone()),
                _ => sched,
            };
            let result = sched.and_then(|c| {
                let schedule = c.to_schedule(off);
                let (feeds, disabled) = feed_lines(&feeds)?;
                let recipe = recipe_from_form(
                    title.text().as_str(),
                    &feeds,
                    articles.value() as usize,
                    full.is_active(),
                    schedule,
                    existing.as_ref().map(|s| &s.recipe),
                )?;
                let mut store = ctx.store.borrow_mut();
                match &existing {
                    Some(old) => {
                        let mut s = old.clone();
                        s.recipe = recipe;
                        s.disabled_feeds = disabled;
                        s.keep_issues = keep.value() as usize;
                        s.enabled = enabled.is_active();
                        store.update(s)
                    }
                    None => {
                        let mut s = Subscription::new(recipe);
                        s.disabled_feeds = disabled;
                        s.keep_issues = keep.value() as usize;
                        s.enabled = enabled.is_active();
                        store.add(s).map(|_| ())
                    }
                }
            });
            match result {
                Ok(()) => {
                    dialog.close();
                    done();
                }
                Err(e) => {
                    banner.set_title(&format!("{e:#}"));
                    banner.set_revealed(true);
                }
            }
        });
    }

    let header = adw::HeaderBar::new();
    header.pack_end(&save);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.add_top_bar(&banner);
    toolbar.set_content(Some(&page));
    dialog.set_child(Some(&toolbar));
    dialog.present(Some(parent));
}
