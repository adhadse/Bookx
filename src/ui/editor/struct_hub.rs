//! The structure tools of the editor (CE-028, CE-120, CE-129, CE-420, CE-421, CE-426, CE-427, CE-429): the commands
//! their widgets send, the state they keep, the Tools menu entries and the dev hooks (`editor:x:...`) the scenarios
//! drive. The dialogs themselves are in `cover_dialog.rs`, `fonts_manager.rs`, `html_transform_ui.rs`, `jacket.rs`,
//! `a11y_form.rs` and `unpack_repack.rs`; the decisions are in `bookx_editing` and `editor_structure.rs`.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;

use crate::i18n::{tr, trf};
use bookx_editing::toc_ops::DropPos;

use super::*;
use crate::editor_structure::Origin;

/// What the structure tools keep between calls.
#[derive(Default)]
pub struct State {
    /// Put the focus back on the selected Contents entry after the next rebuild (keyboard moves).
    pub toc_refocus: Cell<bool>,
    /// The file a non-EPUB book was opened from; saving offers to convert back into it.
    pub origin: RefCell<Option<Origin>>,
    /// A merge by dragging that waits for the user's answer: (dragged file, target file).
    pub pending_merge: RefCell<Option<(String, String)>>,
    /// The Manage Fonts dialog, so the dev hooks can reach it.
    pub fonts_dialog: RefCell<Option<adw::Dialog>>,
}

#[derive(Debug)]
pub enum StructCmd {
    /// A Contents entry was dropped: (dragged index, target index, where).
    TocDrop(usize, usize, DropPos),
    /// A page was dropped onto the middle of another page: (dragged, target).
    DropMerge(String, String),
    /// The result of the file chooser of "Open a Book of Another Format".
    OpenForeign(PathBuf),
    /// Save the foreign book as an EPUB at a path.
    SaveEpub(PathBuf),
    /// Convert the foreign book back to its format and replace the original.
    SaveBack,
}

/// True when a drop at height `y` of a row `h` pixels tall means "merge" (the middle of the row) and not
/// "put before or after".
pub fn is_merge_zone(y: f64, h: i32) -> bool {
    h > 0 && (0.3..=0.7).contains(&(y / f64::from(h)))
}

impl Editor {
    /// Called once from `ext_install`.
    pub(super) fn struct_install(&mut self) {
        self.toc_keys_install();
    }

    pub(super) fn struct_update(&mut self, c: StructCmd) {
        match c {
            StructCmd::TocDrop(from, target, pos) => self.toc_drop(from, target, pos),
            StructCmd::DropMerge(from, onto) => self.prompt_drop_merge(&from, &onto),
            StructCmd::OpenForeign(p) => self.open_foreign_file(&p),
            StructCmd::SaveEpub(p) => self.save_foreign_epub(&p),
            StructCmd::SaveBack => self.save_foreign_back(),
        }
    }

    /// Opens a tool of this file by its menu id; false when the id is not one of ours.
    pub(super) fn struct_open_tool(&mut self, id: &str) -> bool {
        match id {
            "cover" => self.tool_cover(),
            "fonts" => self.tool_fonts(),
            "transform-html" => self.tool_transform_html(),
            "jacket" => self.tool_jacket(),
            "a11y-form" => self.tool_a11y_form(),
            "open-foreign" => self.ask_open_foreign(),
            "open-foreign-now" => self.pick_foreign_file(),
            _ => return false,
        }
        true
    }

    // ---- CE-028: merge by dragging ---------------------------------------------------

    fn prompt_drop_merge(&mut self, from: &str, onto: &str) {
        if from == onto {
            return;
        }
        let pages = [from, onto]
            .iter()
            .all(|p| bookx_editing::markup::is_markup_path(p));
        if !pages {
            self.toast(&tr("Only pages can be merged"));
            return;
        }
        *self.ext.structure.pending_merge.borrow_mut() = Some((from.into(), onto.into()));
        let label = gtk::Label::new(Some(&trf(
            "{} is added to the end of {}. Links to it are repaired and it is removed from the book.",
            &[&file_name(from), &file_name(onto)],
        )));
        label.set_wrap(true);
        let tx = self.sender.input_sender().clone();
        let (f, o) = (from.to_string(), onto.to_string());
        self.ask(
            &tr("Merge these pages?"),
            &tr("A checkpoint is taken first, so you can undo it."),
            &label,
            &tr("Merge"),
            move || tx.emit(Input::MergeFiles(vec![o.clone(), f.clone()])),
        );
    }

    /// The accepting half of the merge prompt (the dev hook `x:drop-merge-accept`).
    fn accept_pending_merge(&mut self) {
        let p = self.ext.structure.pending_merge.borrow_mut().take();
        if let Some((from, onto)) = p {
            self.merge_files(vec![onto, from]);
        }
    }

    // ---- dev hooks -----------------------------------------------------------------------

    /// `editor:x:<name>=<arg>`; false when the name is not one of ours.
    pub(super) fn struct_dev(&mut self, name: &str, arg: &str) -> bool {
        let out = |topic: &str, text: String| eprintln!("editor_x: {topic}: {text}");
        match name {
            "toc-drop" => {
                // toc-drop=<from>|<target>|before|after|into
                let p: Vec<&str> = arg.split('|').collect();
                if let [f, t, pos] = p[..] {
                    let pos = match pos {
                        "before" => DropPos::Before,
                        "into" => DropPos::Into,
                        _ => DropPos::After,
                    };
                    self.toc_drop(f.parse().unwrap_or(0), t.parse().unwrap_or(0), pos);
                }
            }
            "toc-op" => {
                let op = match arg {
                    "up" => TocOp::Up,
                    "down" => TocOp::Down,
                    "in" => TocOp::Indent,
                    "out" => TocOp::Outdent,
                    "delete" => TocOp::Delete,
                    _ => TocOp::Add,
                };
                self.toc_op(op);
            }
            "toc-problems" => out(
                "toc-problems",
                bookx_editing::toc_ops::problems(&self.toc).join("; "),
            ),
            "drop-merge" => {
                if let Some((f, o)) = arg.split_once('|') {
                    self.prompt_drop_merge(f, o);
                }
            }
            "drop-merge-accept" => self.accept_pending_merge(),
            "drop-merge-pending" => out(
                "drop-merge-pending",
                format!("{:?}", self.ext.structure.pending_merge.borrow()),
            ),
            "merge-zone" => {
                // merge-zone=<y>|<height>
                if let Some((y, h)) = arg.split_once('|') {
                    out(
                        "merge-zone",
                        is_merge_zone(y.parse().unwrap_or(0.0), h.parse().unwrap_or(0)).to_string(),
                    );
                }
            }
            "win-size" => {
                // win-size=<w>x<h>
                if let Some((w, h)) = arg.split_once('x') {
                    self.w
                        .win
                        .set_default_size(w.parse().unwrap_or(1280), h.parse().unwrap_or(800));
                }
            }
            "animations" => {
                if let Some(st) = gtk::Settings::default() {
                    st.set_gtk_enable_animations(arg != "off");
                }
            }
            "dialog-shot" => {
                // dialog-shot=<png path>: the open dialog's own content, drawn after a moment
                if let Some(d) = self.w.win.visible_dialog() {
                    let (path, win) = (arg.to_string(), self.w.win.clone());
                    glib::timeout_add_local_once(
                        std::time::Duration::from_millis(400),
                        move || shoot_dialog(&d, &win, &path),
                    );
                }
            }
            "dialog-close" => {
                if let Some(d) = self.w.win.visible_dialog() {
                    d.close();
                }
            }
            "cover" => self.dev_cover(arg),
            "cover-state" => self.dev_cover_state(),
            "fonts-manage" => self.dev_fonts(arg),
            "html-rules" => self.dev_html_rules(arg),
            "html-ruleset" => self.dev_html_ruleset(arg),
            "jacket" => self.dev_jacket(arg),
            "a11y-form" => self.dev_a11y(arg),
            "foreign-open" => self.open_foreign_file(std::path::Path::new(arg)),
            "foreign-save" => self.dev_foreign_save(arg),
            "foreign-status" => self.dev_foreign_status(),
            _ => return false,
        }
        true
    }
}

/// Draws the content of a dialog into a PNG (the screenshots of the scenarios).
fn shoot_dialog(d: &adw::Dialog, win: &adw::Window, path: &str) {
    let Some(child) = d.child() else { return };
    let (w, h) = (child.width(), child.height());
    let snapshot = gtk::Snapshot::new();
    gtk::WidgetPaintable::new(Some(&child)).snapshot(&snapshot, f64::from(w), f64::from(h));
    let node = snapshot.to_node();
    let renderer = win.native().and_then(|n| n.renderer());
    match (node, renderer) {
        (Some(n), Some(r)) => match r.render_texture(&n, None).save_to_png(path) {
            Ok(()) => eprintln!("editor_x: dialog-shot: {path} {w}x{h}"),
            Err(e) => eprintln!("editor_x: dialog-shot: failed {e}"),
        },
        _ => eprintln!(
            "editor_x: dialog-shot: nothing to draw {w}x{h} mapped={} visible={} dialog_mapped={} win_active={}",
            child.is_mapped(),
            child.is_visible(),
            d.is_mapped(),
            win.is_active()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_zone_is_the_middle_of_the_row() {
        assert!(is_merge_zone(20.0, 40));
        assert!(is_merge_zone(12.0, 40));
        assert!(!is_merge_zone(2.0, 40));
        assert!(!is_merge_zone(38.0, 40));
        assert!(!is_merge_zone(5.0, 0));
    }
}
