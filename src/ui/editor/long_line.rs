//! Files with one enormous line (an EPUB written without line breaks: one chapter is one 500 KB line).
//! GtkTextView lays a line out as one paragraph, so such a file freezes the editor for seconds (7 s for a
//! 1.2 MB line, and every scroll or cursor move repeats part of it). Instead of freezing, the code view shows
//! a page that says so and offers to add line breaks (the Reformat tool: only the white space between tags
//! changes) or to open the file as it is. The decision is `codeview::has_huge_line`.

use crate::i18n::{tr, trf};

use super::struct_hub::StructCmd;
use super::tools_ui::Cmd;
use super::*;

/// The page of the code stack that explains it; its buttons send `StructCmd::LongLine`.
pub(super) fn long_line_page(
    tx: relm4::Sender<Input>,
    st: &super::struct_hub::State,
) -> adw::StatusPage {
    let page = adw::StatusPage::builder()
        .icon_name("text-x-generic-symbolic")
        .title(tr("This File Is One Very Long Line"))
        .build();
    let reformat = gtk::Button::with_label(&tr("Add Line Breaks and Open"));
    reformat.add_css_class("suggested-action");
    reformat.add_css_class("pill");
    reformat.set_tooltip_text(Some(&tr(
        "Puts the tags on separate lines. Only the white space between tags changes",
    )));
    let anyway = gtk::Button::with_label(&tr("Open Anyway"));
    anyway.add_css_class("pill");
    anyway.set_tooltip_text(Some(&tr(
        "The editor may stop responding for several seconds",
    )));
    for (b, reformat_it) in [(&reformat, true), (&anyway, false)] {
        let tx = tx.clone();
        b.connect_clicked(move |_| {
            tx.emit(Input::Ext(Cmd::Struct(StructCmd::LongLine(reformat_it))))
        });
    }
    let row = gtk::Box::new(gtk::Orientation::Vertical, 8);
    row.set_halign(gtk::Align::Center);
    row.append(&reformat);
    row.append(&anyway);
    page.set_child(Some(&row));
    *st.long_page.borrow_mut() = Some(page.clone());
    page
}

impl Editor {
    /// True (and the explaining page is shown) when `path` has a line too long for the text view and the user
    /// has not chosen yet what to do about it.
    pub(super) fn long_line_gate(&self, path: &str, buf: &sv::Buffer) -> bool {
        let st = &self.ext.structure;
        if st.long_ok.borrow().contains(path) || !codeview::is_plain(buf) {
            return false;
        }
        let text = buffer_text(buf);
        let Some(longest) = codeview::huge_line(&text) else {
            return false;
        };
        if let Some(p) = st.long_page.borrow().as_ref() {
            p.set_description(Some(&trf(
                "{} has a line of {} KB. A line this long makes the editor stop responding for seconds; with line breaks it works normally.",
                &[&file_name(path), &(longest / 1000)],
            )));
        }
        let blank = st
            .long_blank
            .borrow_mut()
            .get_or_insert_with(|| codeview::new_buffer(system_is_dark()))
            .clone();
        self.w.view.set_buffer(Some(&blank));
        self.w
            .status
            .set_label(&trf("{}   ·   one very long line", &[&path]));
        self.w.center.set_visible_child_name("longline");
        true
    }

    /// The user's answer: add line breaks (when the file can be reformatted) or open it as it is.
    pub(super) fn long_line_choose(&mut self, reformat: bool) {
        let Some(path) = self.current.clone() else {
            return;
        };
        let buf = self.buffer_for(&path);
        if reformat {
            let text = buffer_text(&buf);
            match pretty_print_xml(&text) {
                Ok(new) if new != text => {
                    buf.begin_user_action();
                    let (mut s, mut e) = buf.bounds();
                    buf.delete(&mut s, &mut e);
                    let mut s = buf.start_iter();
                    buf.insert(&mut s, &new);
                    buf.end_user_action();
                    if !codeview::too_big_to_colour(&new) {
                        codeview::unmark_plain(&buf, &path);
                    }
                    self.toast(&tr("Line breaks added"));
                }
                Ok(_) => {}
                Err(e) => self.toast(&trf("Cannot reformat: {}", &[&e])),
            }
        }
        self.ext.structure.long_ok.borrow_mut().insert(path.clone());
        self.show_code(&path, &buf);
    }
}
