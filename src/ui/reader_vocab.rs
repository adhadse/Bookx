//! The reader's side of your own vocabulary: saving the selection as a term with your definition, marking
//! the saved terms in the text, the popover of a saved term, and the glossary of the book. Included
//! from `reader.rs` as a child module (it works on the reader's own state).

use super::*;
use crate::ui::vocab_own as marks_ui;
use crate::vocab_marks::{sentence_around, term_from_selection};

impl Reader {
    /// The series of the book ("" when none), for terms that apply to a whole series.
    fn vocab_series(&self) -> String {
        self.shared
            .lib
            .borrow()
            .get(self.book_id)
            .map(|b| b.meta.series.clone())
            .unwrap_or_default()
    }

    /// Marks the saved terms on the chapter (a dotted underline), when the setting is on. The page may
    /// not be laid out yet (then there is no text): it is tried again at the next position report.
    pub(super) fn refresh_vocab_marks(&self) {
        self.vocab_key.set(None);
        self.vocab_ensure_marks();
    }

    /// Marks the chapter again when the chapter, the vocabulary or the setting changed since the last time.
    pub(super) fn vocab_ensure_marks(&self) {
        let on = self.shared.cfg.borrow().mark_vocabulary;
        let want = (self.spine_idx, self.shared.vocab.borrow().revision(), on);
        if self.vocab_key.get() == Some(want) {
            return;
        }
        let series = self.vocab_series();
        let marks = {
            let hub = self.shared.vocab.borrow();
            if on && hub.has_terms_for(self.book_id, &series) {
                let text = self.canvas.canonical_text();
                if text.is_empty() {
                    return;
                }
                hub.marks_for(self.book_id, &series, &text)
            } else {
                Vec::new()
            }
        };
        self.canvas.set_vocab_marks(marks);
        self.vocab_key.set(Some(want));
    }

    /// The block "Your definition" of the look-up popover, for the selection (or `term`, the word that
    /// the dictionaries were asked for).
    pub(super) fn show_your_definition(&self, term: &str) {
        let series = self.vocab_series();
        let phrase = self
            .selection
            .as_ref()
            .and_then(|s| term_from_selection(&s.text))
            .unwrap_or_else(|| term.to_string());
        let book = self.book_id.to_string();
        let hub = self.shared.vocab.borrow();
        let found = hub
            .store
            .own_definition(&phrase, &book, &series)
            .or_else(|| hub.store.own_definition(term, &book, &series));
        self.w.vocab_def.set(found, &phrase);
    }

    /// Opens the dialog that saves `term` with your own definition (or edits it). The sentence the
    /// selection stands in is kept with the term.
    pub(super) fn vocab_dialog(&self, term: &str) {
        let (context, position) = match &self.selection {
            Some(sel) if term_from_selection(&sel.text).is_some() => {
                let text = self.canvas.canonical_text();
                (
                    sentence_around(&text, sel.start.max(0) as usize, sel.end.max(0) as usize),
                    Some(self.overall_fraction()),
                )
            }
            _ => (String::new(), None),
        };
        let ctx = marks_ui::Ctx::new(
            &self.shared,
            Some(self.book_id),
            term,
            &context,
            position,
            &self.dict_language(),
        );
        self.w.sel_pop.popdown();
        let tx = self.sender.input_sender().clone();
        marks_ui::open_dialog(&self.w.page, ctx, move |t| tx.emit(Input::VocabSaved(t)));
    }

    /// "Add to Vocabulary" for the selection.
    pub(super) fn vocab_add(&self) {
        let term = self
            .selection
            .as_ref()
            .and_then(|s| term_from_selection(&s.text));
        match term {
            Some(t) => self.vocab_dialog(&t),
            None => {
                let _ = self
                    .sender
                    .output(Output::Toast(tr("Select a word or phrase first.")));
            }
        }
    }

    /// A saved term was clicked on the page: its definition in a small popover.
    pub(super) fn vocab_mark_click(&self, info: &SelectionInfo) {
        let series = self.vocab_series();
        let entry = {
            let hub = self.shared.vocab.borrow();
            hub.word_for_mark(self.book_id, &series, &info.text)
                .and_then(|w| hub.store.find(&w).cloned())
        };
        let Some(entry) = entry else { return };
        let tx = self.sender.input_sender().clone();
        let (a, b, c) = (tx.clone(), tx.clone(), tx);
        marks_ui::show_mark_popover(
            &self.canvas.area,
            info.rect,
            &entry,
            marks_ui::MarkActions {
                edit: Rc::new(move |w| a.emit(Input::VocabEdit(w))),
                remove: Rc::new(move |w| b.emit(Input::VocabRemove(w))),
                open: Rc::new(move |w| c.emit(Input::VocabOpen(w))),
            },
        );
    }

    /// The glossary of this book: what you defined for it and the words you met in it.
    pub(super) fn vocab_terms(&self) {
        let (title, language) = {
            let lib = self.shared.lib.borrow();
            lib.get(self.book_id)
                .map(|b| (b.meta.title.clone(), b.meta.language.clone()))
                .unwrap_or_default()
        };
        let tx = self.sender.input_sender().clone();
        let goto: Rc<dyn Fn(f64)> = Rc::new(move |f| tx.emit(Input::Seek(f)));
        marks_ui::show_book_terms(
            &self.w.page,
            &self.shared,
            marks_ui::BookView {
                id: self.book_id,
                title,
                language,
                goto: Some(goto),
            },
        );
    }

    /// Something changed in your vocabulary: the marks follow.
    pub(super) fn vocab_changed(&self) {
        self.refresh_vocab_marks();
    }

    /// Tells the user and the screen reader what was done.
    pub(super) fn vocab_say(&self, text: String) {
        self.canvas
            .area
            .announce(&text, gtk::AccessibleAnnouncementPriority::Medium);
        let _ = self.sender.output(Output::Toast(text));
    }

    /// Dev hooks of the reader (`reader:ui-vocab-*`).
    pub(super) fn vocab_dev(&self, c: &str) -> bool {
        match c {
            "ui-vocab-add" => self.vocab_add(),
            "ui-vocab-click" => {
                eprintln!("vocab_click: {}", self.canvas.dev_click_vocab());
            }
            "ui-vocab-terms" => self.vocab_terms(),
            "vocab-dump" => {
                let (n, first) = self.canvas.dev_visible_vocab();
                let marks = self.canvas.dev_vocab_marks();
                eprintln!(
                    "vocab_marks: total={} on_page={} first={:?} texts={:?}",
                    marks.len(),
                    n,
                    first,
                    marks.iter().map(|m| m.2.clone()).collect::<Vec<_>>()
                );
                eprintln!("vocab_def: {}", self.w.vocab_def.dump());
                eprintln!("{}", marks_ui::dev_mark(""));
                eprintln!("{}", marks_ui::dev(""));
            }
            "ui-vocab-def-btn" => self.w.vocab_def.button.emit_clicked(),
            "ui-vocab-btn" => self.w.vocab_btn.emit_clicked(),
            k if k.starts_with("select-text=") => {
                let ok = self.canvas.dev_select_text(&k["select-text=".len()..]);
                eprintln!("select_text: {ok}");
            }
            k if k.starts_with("ui-vocab-dialog=") => {
                eprintln!("{}", marks_ui::dev(&k["ui-vocab-dialog=".len()..]));
            }
            k if k.starts_with("ui-vocab-mark=") => {
                eprintln!("{}", marks_ui::dev_mark(&k["ui-vocab-mark=".len()..]));
            }
            _ => return false,
        }
        true
    }
}
