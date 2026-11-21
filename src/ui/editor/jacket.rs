//! Tools → Structure and Accessibility → Book Jacket (CE-426): add, update or remove the metadata page near the
//! front of the book. The page is regenerated when the metadata is saved from the editor
//! (`Editor::struct_after_metadata`). The page and its marker are `bookx_editing::jacket`.

use crate::editor_structure as es;
use crate::i18n::{tr, trf};
use bookx_editing::jacket::{self, JacketOptions};

use super::tools_ui::*;
use super::*;

/// The primary button of the Book Jacket dialog for the chosen action (0 = add or update, 1 = remove).
pub(super) fn apply_label(existing: bool, action: u32) -> &'static str {
    match (action, existing) {
        (1, _) => crate::i18n::N_("Remove Jacket"),
        (_, true) => crate::i18n::N_("Update Jacket"),
        (_, false) => crate::i18n::N_("Add Jacket"),
    }
}

impl Editor {
    pub(super) fn tool_jacket(&self) {
        let existing = jacket::options_of(&self.book.borrow());
        let o = existing.clone().unwrap_or_default();
        let action = combo_row(
            &tr("Action"),
            &[
                &tr(if existing.is_some() {
                    crate::i18n::N_("Update the jacket")
                } else {
                    crate::i18n::N_("Add a jacket")
                }),
                &tr("Remove the jacket"),
            ],
            0,
        );
        let sw = |t: &str, on: bool| switch_row(&tr(t), "", on);
        let cover = sw("Cover", o.cover);
        let series = sw("Series", o.series);
        let rating = sw("Rating", o.rating);
        let tags = sw("Tags", o.tags);
        let comments = sw("Comments", o.comments);
        let publisher = sw("Publisher", o.publisher);
        let g = group("");
        let action_row = action.clone();
        let has_jacket = existing.is_some();
        for w in [
            action.upcast_ref::<gtk::Widget>(),
            cover.upcast_ref(),
            series.upcast_ref(),
            rating.upcast_ref(),
            tags.upcast_ref(),
            comments.upcast_ref(),
            publisher.upcast_ref(),
        ] {
            g.add(w);
        }
        let make: MakeOp = Rc::new(move || {
            if action.selected() == 1 {
                return Ok((
                    "Remove Jacket".to_string(),
                    Rc::new(|b: &mut OpenBook, _: Option<&str>| es::jacket_remove(b)) as Op,
                ));
            }
            let o = JacketOptions {
                cover: cover.is_active(),
                series: series.is_active(),
                tags: tags.is_active(),
                rating: rating.is_active(),
                comments: comments.is_active(),
                publisher: publisher.is_active(),
            };
            Ok((
                "Book Jacket".to_string(),
                Rc::new(move |b: &mut OpenBook, _: Option<&str>| es::jacket_add(b, &o)) as Op,
            ))
        });
        let ui = self.tool_dialog(
            Spec::new(
                crate::i18n::N_("Book Jacket"),
                tr("A page after the cover that lists the title, authors, series, tags, rating and comments from the book's metadata. It is marked, so it can be updated when the metadata changes and removed without a trace."),
                apply_label(has_jacket, 0),
            ),
            Some(g.upcast()),
            make,
            None,
        );
        let apply = ui.apply.clone();
        action_row.connect_selected_notify(move |r| {
            apply.set_label(&tr(apply_label(has_jacket, r.selected())));
        });
    }

    /// Called after the metadata was applied: a jacket page shows the new values.
    pub(super) fn struct_after_metadata(&mut self) {
        let has = jacket::find_jacket(&self.book.borrow()).is_some();
        if !has {
            return;
        }
        let res = es::jacket_refresh(&mut self.book.borrow_mut());
        match res {
            Ok(Some(p)) => {
                self.after_archive_changed();
                self.structure_dirty = true;
                self.update_title();
                self.toast(&trf("Updated the book jacket {}", &[&p]));
            }
            Ok(None) => {}
            Err(e) => self.toast(&trf(
                "Could not update the book jacket: {}",
                &[&format!("{e:#}")],
            )),
        }
    }

    /// `x:jacket=add[|cover=0|tags=0...]` | `remove` | `refresh` | `state`.
    pub(super) fn dev_jacket(&mut self, arg: &str) {
        let mut it = arg.split('|');
        match it.next().unwrap_or("") {
            "add" => {
                let mut o = JacketOptions::default();
                for kv in it {
                    if let Some((k, v)) = kv.split_once('=') {
                        let on = v == "1";
                        match k {
                            "cover" => o.cover = on,
                            "series" => o.series = on,
                            "tags" => o.tags = on,
                            "rating" => o.rating = on,
                            "comments" => o.comments = on,
                            "publisher" => o.publisher = on,
                            _ => {}
                        }
                    }
                }
                self.run_op("Book Jacket", move |b, _| es::jacket_add(b, &o));
            }
            "remove" => self.run_op("Remove Jacket", |b, _| es::jacket_remove(b)),
            "refresh" => self.struct_after_metadata(),
            "set-title" => {
                let t = it.next().unwrap_or("").to_string();
                let mut md = self.book.borrow().metadata().clone();
                md.title = t;
                self.sender
                    .input(Input::ApplyMetadata(md, CoverChange::Keep));
            }
            _ => {
                let b = self.book.borrow();
                let p = jacket::find_jacket(&b);
                eprintln!(
                    "editor_x: jacket: page={} first-spine={} stale={} options={}",
                    p.clone().unwrap_or_default(),
                    b.spine_paths().first().cloned().unwrap_or_default(),
                    jacket::options_of(&b)
                        .map(|o| jacket::is_stale(&b, &o))
                        .unwrap_or(false),
                    jacket::options_of(&b)
                        .map(|o| o.to_marker_text())
                        .unwrap_or_default()
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::apply_label;

    #[test]
    fn jacket_button_names_the_action() {
        assert_eq!(apply_label(false, 0), "Add Jacket");
        assert_eq!(apply_label(true, 0), "Update Jacket");
        assert_eq!(apply_label(true, 1), "Remove Jacket");
        assert_eq!(apply_label(false, 1), "Remove Jacket");
    }
}
