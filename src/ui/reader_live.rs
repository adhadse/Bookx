//! The small banner at the top of a reader that tells of a sync while the book is open ("Your Kobo
//! Libra is at 62% (page 143)" with *Continue There*, "This note changed on your e-reader" with
//! *Use Theirs* and *Keep Mine*). The rules (one at a time, 15 seconds, a page turn) are in
//! `sync_live::Live`; this is the widget. Shared by the EPUB, PDF and comic readers: each puts
//! [`LiveBanner::widget`] into its page overlay and handles the [`Answer`].

use std::cell::RefCell;
use std::rc::Rc;

use relm4::adw::prelude::*;
use relm4::gtk::{self, glib};

use crate::sync_live::{Banner, Live};

/// Which button was pressed, with the banner it belonged to.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    /// The suggested button (*Continue There*, *Use Theirs*).
    Primary(Banner),
    /// The other one (*Dismiss*, *Keep Mine*).
    Secondary(Banner),
}

pub struct LiveBanner {
    pub widget: gtk::Revealer,
    label: gtk::Label,
    primary: gtk::Button,
    secondary: gtk::Button,
    model: RefCell<Live>,
    on_answer: Rc<dyn Fn(Answer)>,
    born: std::time::Instant,
}

impl LiveBanner {
    pub fn new(on_answer: impl Fn(Answer) + 'static) -> Rc<LiveBanner> {
        let label = gtk::Label::new(None);
        label.set_wrap(true);
        label.set_xalign(0.0);
        label.set_hexpand(true);
        label.set_max_width_chars(60);
        let primary = gtk::Button::new();
        primary.add_css_class("suggested-action");
        let secondary = gtk::Button::new();
        // named from the start: render() renames them per banner, but they exist while hidden
        for (btn, text) in [(&primary, "Continue There"), (&secondary, "Dismiss")] {
            let name = crate::i18n::tr(text);
            btn.set_label(&name);
            super::audit::label(btn, &name);
        }
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("osd");
        row.add_css_class("sync-live");
        row.set_margin_start(12);
        row.set_margin_end(12);
        row.set_margin_top(6);
        row.set_margin_bottom(6);
        row.append(&label);
        row.append(&primary);
        row.append(&secondary);
        let widget = gtk::Revealer::new();
        widget.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        widget.set_halign(gtk::Align::Center);
        widget.set_valign(gtk::Align::Start);
        widget.set_margin_top(10);
        widget.set_child(Some(&row));
        widget.set_reveal_child(false);
        super::audit::label(&widget, &crate::i18n::tr("Sync message"));
        let me = Rc::new(LiveBanner {
            widget,
            label,
            primary,
            secondary,
            model: RefCell::new(Live::default()),
            on_answer: Rc::new(on_answer),
            born: std::time::Instant::now(),
        });
        for (btn, first) in [(&me.primary, true), (&me.secondary, false)] {
            let w = Rc::downgrade(&me);
            btn.connect_clicked(move |_| {
                if let Some(me) = w.upgrade() {
                    me.answer(first)
                }
            });
        }
        let w = Rc::downgrade(&me);
        glib::timeout_add_seconds_local(1, move || match w.upgrade() {
            Some(me) => {
                if me.model.borrow_mut().tick(me.now()) {
                    me.render();
                }
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
        me
    }

    fn now(&self) -> u64 {
        self.born.elapsed().as_millis() as u64
    }

    fn answer(&self, primary: bool) {
        let Some(b) = self.model.borrow().current().cloned() else {
            return;
        };
        self.model.borrow_mut().dismiss();
        self.render();
        (self.on_answer)(if primary {
            Answer::Primary(b)
        } else {
            Answer::Secondary(b)
        });
    }

    fn render(&self) {
        let cur = self.model.borrow().current().cloned();
        match cur {
            Some(b) => {
                let buttons = b.buttons();
                self.label.set_label(&b.text());
                let (first, second) = (buttons.first(), buttons.get(1));
                // a banner with one button has it as the plain one
                match (first, second) {
                    (Some(a), Some(c)) => {
                        self.primary.set_label(a);
                        super::audit::label(&self.primary, a);
                        self.primary.set_visible(true);
                        self.secondary.set_label(c);
                        super::audit::label(&self.secondary, c);
                        self.secondary.set_visible(true);
                    }
                    (Some(a), None) => {
                        self.primary.set_visible(false);
                        self.secondary.set_label(a);
                        super::audit::label(&self.secondary, a);
                        self.secondary.set_visible(true);
                    }
                    _ => {}
                }
                self.widget.set_reveal_child(true);
            }
            None => self.widget.set_reveal_child(false),
        }
    }

    /// A device is further along. Returns whether the banner changed.
    pub fn offer_ahead(&self, device: &str, pct: f64, page: Option<usize>) -> bool {
        let changed = self
            .model
            .borrow_mut()
            .offer_ahead(device, pct, page, self.now());
        if changed {
            self.render();
        }
        changed
    }

    pub fn offer_note(&self, b: Banner) -> bool {
        let changed = self.model.borrow_mut().offer_note(b, self.now());
        if changed {
            self.render();
        }
        changed
    }

    /// The reader turned a page.
    pub fn page_turned(&self) {
        if self.model.borrow_mut().page_turned() {
            self.render();
        }
    }

    /// The note editor closed.
    pub fn editor_closed(&self) {
        if self.model.borrow_mut().editor_closed() {
            self.render();
        }
    }

    pub fn current(&self) -> Option<Banner> {
        self.model.borrow().current().cloned()
    }

    /// Test hook: `none`, or `<text> | <button> / <button>`.
    pub fn dev_state(&self) -> String {
        match self.current() {
            Some(b) => format!("{} | {}", b.text(), b.buttons().join(" / ")),
            None => "none".to_string(),
        }
    }

    /// Test hook: press a button (`primary` or `secondary`).
    pub fn dev_press(&self, primary: bool) {
        self.answer(primary)
    }
}
