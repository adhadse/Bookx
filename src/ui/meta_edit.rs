//! Small editors of the library window: a star rating out of five and a tag editor with chips.
//! Both are used by the details panel and by the Properties dialog; they only call back, the
//! library window stores the change (and records that the user made it).

use std::cell::RefCell;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, glib};

use crate::i18n::{tr, trf};
use crate::tags;

/// A row of five stars for a rating stored as 0-10 (two points a star). Click a star to rate, click
/// the star that is the rating to clear it; with the keyboard: Left and Right change the rating,
/// 0-5 set it, Delete clears it.
pub fn rating_stars(value: u8, on_change: Rc<dyn Fn(u8)>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    row.add_css_class("rating-stars");
    row.set_focusable(true);
    row.set_halign(gtk::Align::Start);
    let current = Rc::new(RefCell::new(value));
    let stars: Rc<Vec<gtk::Button>> = Rc::new(
        (0..5)
            .map(|_| {
                let b = gtk::Button::new();
                b.add_css_class("flat");
                b.add_css_class("circular");
                b.set_can_focus(false);
                b
            })
            .collect(),
    );
    let paint = {
        let (stars, row) = (stars.clone(), row.clone());
        move |v: u8| {
            let n = (v as usize).div_ceil(2);
            for (i, b) in stars.iter().enumerate() {
                b.set_icon_name(if i < n {
                    "starred-symbolic"
                } else {
                    "non-starred-symbolic"
                });
                if i < n {
                    b.add_css_class("rated");
                } else {
                    b.remove_css_class("rated");
                }
                b.set_tooltip_text(Some(&if i + 1 == n {
                    tr("Click to clear the rating")
                } else {
                    trf("{} stars", &[&(i + 1)])
                }));
                super::audit::label(b, &trf("{} stars", &[&(i + 1)]));
            }
            let text = if n == 0 {
                tr("Not rated")
            } else {
                trf("Rated {} of 5 stars", &[&n])
            };
            row.update_property(&[gtk::accessible::Property::Label(&format!(
                "{} ({})",
                tr("Rating"),
                text
            ))]);
            row.set_tooltip_text(Some(&text));
        }
    };
    paint(value);
    let set: Rc<dyn Fn(u8)> = {
        let (current, on_change, paint) = (current.clone(), on_change, paint);
        Rc::new(move |v: u8| {
            if *current.borrow() == v {
                return;
            }
            *current.borrow_mut() = v;
            paint(v);
            on_change(v);
        })
    };
    for (i, b) in stars.iter().enumerate() {
        row.append(b);
        let (set, current) = (set.clone(), current.clone());
        b.connect_clicked(move |_| {
            let want = ((i + 1) * 2) as u8;
            set(if *current.borrow() == want { 0 } else { want });
        });
    }
    let keys = gtk::EventControllerKey::new();
    {
        let (set, current) = (set, current);
        keys.connect_key_pressed(move |_, key, _, _| {
            use gtk::gdk::Key;
            let now = *current.borrow();
            let new = match key {
                Key::Right | Key::plus => Some((now + 2).min(10)),
                Key::Left | Key::minus => Some(now.saturating_sub(2)),
                Key::Delete | Key::BackSpace => Some(0),
                k => k
                    .to_unicode()
                    .and_then(|c| c.to_digit(10))
                    .filter(|d| *d <= 5)
                    .map(|d| (d * 2) as u8),
            };
            match new {
                Some(v) => {
                    set(v);
                    glib::Propagation::Stop
                }
                None => glib::Propagation::Proceed,
            }
        });
    }
    row.add_controller(keys);
    row
}

/// The tags of a book as chips with a remove button each, and an entry that adds tags (commas
/// between several, Enter to add) with suggestions from `all` (the tags the library has).
/// `on_change` gets the whole new list.
pub fn tag_editor(
    initial: &[String],
    all: Vec<String>,
    on_change: Rc<dyn Fn(Vec<String>)>,
) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
    root.add_css_class("tag-editor");
    let chips = adw::WrapBox::new();
    chips.set_child_spacing(6);
    chips.set_line_spacing(6);
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some(&tr("Add a tag")));
    entry.set_hexpand(true);
    super::audit::label(&entry, &tr("Add a tag"));
    root.append(&chips);
    root.append(&entry);
    let tags_now = Rc::new(RefCell::new(initial.to_vec()));

    let rebuild: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let redraw: Rc<dyn Fn()> = {
        let (chips, tags_now, on_change, rebuild) = (
            chips.clone(),
            tags_now.clone(),
            on_change.clone(),
            rebuild.clone(),
        );
        Rc::new(move || {
            while let Some(c) = chips.first_child() {
                chips.remove(&c);
            }
            let list = tags_now.borrow().clone();
            if list.is_empty() {
                let l = gtk::Label::new(Some(&tr("No tags")));
                l.add_css_class("dim-label");
                chips.append(&l);
            }
            for t in list {
                let b = gtk::Button::new();
                let inner = gtk::Box::new(gtk::Orientation::Horizontal, 4);
                inner.append(&gtk::Label::new(Some(&t)));
                inner.append(&gtk::Image::from_icon_name("window-close-symbolic"));
                b.set_child(Some(&inner));
                b.add_css_class("pill");
                b.add_css_class("tag-chip");
                let tip = trf("Remove the tag {}", &[&t]);
                b.set_tooltip_text(Some(&tip));
                super::audit::label(&b, &tip);
                let (tags_now, on_change, rebuild, t) = (
                    tags_now.clone(),
                    on_change.clone(),
                    rebuild.clone(),
                    t.clone(),
                );
                b.connect_clicked(move |_| {
                    tags::remove(&mut tags_now.borrow_mut(), &t);
                    on_change(tags_now.borrow().clone());
                    if let Some(r) = rebuild.borrow().as_ref() {
                        r();
                    }
                });
                chips.append(&b);
            }
        })
    };
    *rebuild.borrow_mut() = Some(redraw.clone());
    redraw();

    // adding: Enter (several tags may be separated by commas)
    let add = {
        let (entry, tags_now, on_change, redraw) =
            (entry.clone(), tags_now.clone(), on_change, redraw);
        Rc::new(move || {
            let mut changed = false;
            for t in tags::parse_entry(&entry.text()) {
                changed |= tags::add(&mut tags_now.borrow_mut(), &t);
            }
            entry.set_text("");
            if changed {
                on_change(tags_now.borrow().clone());
                redraw();
            }
        })
    };
    {
        let add = add.clone();
        entry.connect_activate(move |_| add());
    }
    // a comma ends a tag
    {
        let add = add.clone();
        entry.connect_changed(move |e| {
            if e.text().ends_with(',') {
                add();
            }
        });
    }
    // suggestions from the tags the library has
    #[allow(deprecated)]
    {
        let completion = gtk::EntryCompletion::new();
        completion.set_text_column(0);
        completion.set_minimum_key_length(1);
        completion.set_inline_completion(false);
        let store = gtk::ListStore::new(&[glib::Type::STRING]);
        let have = tags_now.clone();
        for t in tags::suggestions(&all, "", &have.borrow(), 400) {
            store.set(&store.append(), &[(0, &t)]);
        }
        completion.set_model(Some(&store));
        entry.set_completion(Some(&completion));
        let (add, e) = (add, entry.clone());
        completion.connect_match_selected(move |_, model, iter| {
            if let Ok(t) = model.get_value(iter, 0).get::<String>() {
                e.set_text(&t);
                add();
            }
            glib::Propagation::Stop
        });
    }
    root
}
