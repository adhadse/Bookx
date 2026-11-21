//! *Search inside books*: how many books have their text indexed and a button to build the index
//! again (roadmap L33). The index is `fulltext.rs` over the library database.

use crate::i18n::{ntr, tr, trf};
use std::rc::Rc;
use std::time::Duration;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, glib};

use super::preferences::{register_group, PrefCtx};
use super::SharedRef;

pub fn register() {
    register_group("library", 50, group);
}

/// The status line: `(indexed, books, running)`.
pub fn status(shared: &SharedRef) -> (usize, usize, bool) {
    let total = shared.lib.borrow().books.len();
    match shared.fulltext.borrow().as_ref() {
        Some(ft) => (ft.indexed_books(), total, ft.is_running()),
        None => (0, total, false),
    }
}

/// The subtitle for a status.
pub fn status_text(indexed: usize, total: usize, running: bool, available: bool) -> String {
    if !available {
        return tr("The search index could not be opened");
    }
    let base = trf(
        "{} of {} books searchable by their text",
        &[&indexed, &total],
    );
    if running {
        format!("{base} · {}", tr("Indexing"))
    } else if indexed >= total {
        base
    } else {
        format!(
            "{base} · {}",
            ntr("{} book waits", "{} books wait", total - indexed)
        )
    }
}

fn group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Search Inside Books"));
    let row = adw::ActionRow::builder().title(tr("Text index")).build();
    row.set_widget_name("index-status");
    let rebuild = gtk::Button::with_label(&tr("Rebuild"));
    rebuild.set_valign(gtk::Align::Center);
    rebuild.set_widget_name("index-rebuild");
    super::audit::label(&rebuild, &tr("Rebuild the text index"));
    row.add_suffix(&rebuild);
    g.add(&row);
    let shared = ctx.shared.clone();
    let refresh: Rc<dyn Fn()> = {
        let (shared, row, rebuild) = (shared.clone(), row.clone(), rebuild.clone());
        Rc::new(move || {
            let (i, t, r) = status(&shared);
            let available = shared.fulltext.borrow().is_some();
            row.set_subtitle(&status_text(i, t, r, available));
            rebuild.set_sensitive(available && !r);
        })
    };
    refresh();
    {
        let (shared, refresh) = (shared.clone(), refresh.clone());
        rebuild.connect_clicked(move |_| {
            let cleared = shared
                .fulltext
                .borrow()
                .as_ref()
                .map(|f| f.clear().is_ok())
                .unwrap_or(false);
            if cleared {
                shared.start_indexing();
            }
            refresh();
        });
    }
    // the numbers move while a run is going; stop when the window is gone
    let weak = row.downgrade();
    glib::timeout_add_local(Duration::from_secs(2), move || match weak.upgrade() {
        Some(r) if r.is_mapped() => {
            refresh();
            glib::ControlFlow::Continue
        }
        Some(_) => glib::ControlFlow::Continue,
        None => glib::ControlFlow::Break,
    });
    g
}

/// Test hook: the text of the row (`index_status: …`) after an optional rebuild.
pub fn dev(shared: &SharedRef, cmd: &str) -> String {
    if cmd == "rebuild" {
        let ok = shared
            .fulltext
            .borrow()
            .as_ref()
            .map(|f| f.clear().is_ok())
            .unwrap_or(false);
        if ok {
            shared.start_indexing();
        }
    }
    let (i, t, r) = status(shared);
    format!(
        "index_status: indexed={i} books={t} running={r} text=\"{}\"",
        status_text(i, t, r, shared.fulltext.borrow().is_some())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_says_what_is_waiting() {
        assert!(status_text(5, 5, false, true).starts_with("5 of 5 books"));
        assert!(status_text(3, 5, false, true).contains("2 books wait"));
        assert!(status_text(4, 5, false, true).contains("1 book waits"));
        assert!(status_text(3, 5, true, true).ends_with("Indexing"));
        assert_eq!(
            status_text(0, 0, false, false),
            "The search index could not be opened"
        );
    }
}
