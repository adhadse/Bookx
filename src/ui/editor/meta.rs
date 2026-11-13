//! The metadata tab.

use super::*;
use crate::i18n::tr;

impl Editor {
    pub(super) fn rebuild_meta(&self) {
        while let Some(c) = self.w.meta_box.first_child() {
            self.w.meta_box.remove(&c);
        }
        let (md, cover) = {
            let b = self.book.borrow();
            (b.metadata().clone(), b.cover_bytes().map(|c| c.to_vec()))
        };
        let pic = gtk::Picture::new();
        pic.set_size_request(-1, 200);
        pic.set_content_fit(gtk::ContentFit::Contain);
        pic.set_margin_top(16);
        if let Some(t) = cover.as_deref().and_then(metadata::texture_from_bytes) {
            pic.set_paintable(Some(&t));
        }
        let title = gtk::Label::new(Some(&md.title));
        title.add_css_class("title-3");
        title.set_wrap(true);
        title.set_justify(gtk::Justification::Center);
        title.set_margin_top(8);
        let authors = gtk::Label::new(Some(&md.authors_display()));
        authors.add_css_class("dim-label");
        authors.set_wrap(true);
        let info = gtk::Label::new(Some(&format!(
            "{}{}{}",
            if md.series.is_empty() {
                String::new()
            } else {
                format!(
                    "{} #{}\n",
                    md.series,
                    crate::core::opf::format_index(md.series_index)
                )
            },
            if md.publisher.is_empty() {
                String::new()
            } else {
                format!("{}\n", md.publisher)
            },
            if md.tags.is_empty() {
                String::new()
            } else {
                md.tags.join(", ")
            },
        )));
        info.add_css_class("caption");
        info.set_wrap(true);
        info.set_justify(gtk::Justification::Center);
        let edit = gtk::Button::with_label(&tr("Edit Metadata"));
        edit.add_css_class("pill");
        edit.add_css_class("suggested-action");
        edit.set_halign(gtk::Align::Center);
        edit.set_margin_top(14);
        let tx = self.sender.input_sender().clone();
        edit.connect_clicked(move |_| tx.emit(Input::EditMetadata));
        let hint = gtk::Label::new(Some(&tr("Changes are applied to the book you are editing and written to the file when you save.")));
        hint.add_css_class("dim-label");
        hint.add_css_class("caption");
        hint.set_wrap(true);
        hint.set_margin_all(16);
        hint.set_justify(gtk::Justification::Center);
        for w in [
            pic.upcast_ref::<gtk::Widget>(),
            title.upcast_ref(),
            authors.upcast_ref(),
            info.upcast_ref(),
            edit.upcast_ref(),
            hint.upcast_ref(),
        ] {
            self.w.meta_box.append(w);
        }
    }
}
