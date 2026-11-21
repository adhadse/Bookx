//! Metadata editing UI: the Calibre-like "Edit metadata" form, dialog and bulk editor.

use crate::i18n::{tr, trf};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gdk, gio, glib};

use crate::core::opf::Metadata;
use crate::core::util::*;

#[derive(Debug, Clone, Default)]
pub enum CoverChange {
    #[default]
    Keep,
    Set(Vec<u8>, String),
    Remove,
}

#[derive(Clone)]
pub struct MetadataForm {
    pub widget: gtk::Widget,
    orig: Metadata,
    title: adw::EntryRow,
    title_sort: adw::EntryRow,
    authors: adw::EntryRow,
    author_sort: adw::EntryRow,
    series: adw::EntryRow,
    series_index: adw::SpinRow,
    /// The number typed in any style (`III`, `Book Two`) and what it was understood as.
    series_text: adw::EntryRow,
    series_hint: adw::ActionRow,
    publisher: adw::EntryRow,
    language: adw::EntryRow,
    published: adw::EntryRow,
    tags: adw::EntryRow,
    identifiers: adw::EntryRow,
    rating: Rc<Cell<u8>>,
    description: gtk::TextView,
    cover_change: Rc<RefCell<CoverChange>>,
    picture: gtk::Picture,
    fetch_btn: gtk::Button,
    replace_chk: gtk::CheckButton,
    status: gtk::Label,
    /// Field -> where the fetched value came from, for the provenance record on save.
    fetched: Rc<RefCell<Vec<(String, String)>>>,
}

thread_local! {
    /// What the last saved edit took from online sources (read by `take_fetch_sources`).
    static LAST_FETCH: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
    /// The Save button of the open edit dialog (UI tests press it).
    static SAVE_BTN: RefCell<Option<gtk::Button>> = const { RefCell::new(None) };
    /// The form most recently built (UI tests drive it through `dev_*`).
    static ACTIVE: RefCell<Option<MetadataForm>> = const { RefCell::new(None) };
    /// The other books of the library with a series, for the series number preview of the next form.
    static SERIES_CONTEXT: RefCell<Vec<crate::series_view::SeriesBook>> = const { RefCell::new(Vec::new()) };
}

/// Tells the next edit form about the series of the library (it is used once).
pub fn set_series_context(books: Vec<crate::series_view::SeriesBook>) {
    SERIES_CONTEXT.with(|c| *c.borrow_mut() = books);
}

pub fn texture_from_bytes(data: &[u8]) -> Option<gdk::Texture> {
    gdk::Texture::from_bytes(&glib::Bytes::from(data)).ok()
}

pub fn star_row(rating: Rc<Cell<u8>>) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(tr("Rating")).build();
    let boxx = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    boxx.set_valign(gtk::Align::Center);
    let buttons: Vec<gtk::Button> = (1..=5)
        .map(|_| {
            let b = gtk::Button::new();
            b.add_css_class("flat");
            b.add_css_class("circular");
            b
        })
        .collect();
    let refresh = {
        let buttons = buttons.clone();
        let rating = rating.clone();
        Rc::new(move || {
            let stars = (rating.get() as usize).div_ceil(2);
            for (i, b) in buttons.iter().enumerate() {
                b.set_icon_name(if i < stars {
                    "starred-symbolic"
                } else {
                    "non-starred-symbolic"
                });
            }
        })
    };
    for (i, b) in buttons.iter().enumerate() {
        let rating = rating.clone();
        let refresh = refresh.clone();
        b.connect_clicked(move |_| {
            let v = ((i + 1) * 2) as u8;
            rating.set(if rating.get() == v { 0 } else { v });
            refresh();
        });
        boxx.append(b);
    }
    refresh();
    row.add_suffix(&boxx);
    row
}

fn entry(title: &str, text: &str) -> adw::EntryRow {
    let e = adw::EntryRow::builder().title(title).build();
    e.set_text(text);
    e
}

impl MetadataForm {
    pub fn new(md: &Metadata, cover: Option<&[u8]>) -> MetadataForm {
        let page = adw::PreferencesPage::new();

        // --- cover ---------------------------------------------------------
        let cover_group = adw::PreferencesGroup::new();
        let cover_box = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        cover_box.set_halign(gtk::Align::Center);
        let picture = gtk::Picture::new();
        picture.set_size_request(120, 180);
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.add_css_class("card");
        if let Some(t) = cover.and_then(texture_from_bytes) {
            picture.set_paintable(Some(&t));
        }
        let btns = gtk::Box::new(gtk::Orientation::Vertical, 6);
        btns.set_valign(gtk::Align::Center);
        let browse = gtk::Button::with_label(&tr("Choose Cover"));
        let remove = gtk::Button::with_label(&tr("Remove Cover"));
        remove.add_css_class("destructive-action");
        let fetch_btn = gtk::Button::with_label(&tr("Fetch Metadata"));
        fetch_btn.set_tooltip_text(Some(&tr(
            "Look the book up on Open Library, Google Books and Wikidata",
        )));
        let replace_chk = gtk::CheckButton::with_label(&tr("Replace existing values"));
        btns.append(&fetch_btn);
        btns.append(&replace_chk);
        btns.append(&browse);
        btns.append(&remove);
        cover_box.append(&picture);
        cover_box.append(&btns);
        cover_group.add(&cover_box);
        let status = gtk::Label::new(None);
        status.set_wrap(true);
        status.set_xalign(0.0);
        status.add_css_class("dim-label");
        status.add_css_class("caption");
        status.set_visible(false);
        cover_group.add(&status);
        page.add(&cover_group);

        let cover_change = Rc::new(RefCell::new(CoverChange::Keep));
        {
            let picture = picture.clone();
            let cc = cover_change.clone();
            browse.connect_clicked(move |b| {
                let dialog = gtk::FileDialog::builder()
                    .title(tr("Choose a cover image"))
                    .build();
                let filter = gtk::FileFilter::new();
                filter.add_mime_type("image/*");
                filter.set_name(Some("Images"));
                dialog.set_default_filter(Some(&filter));
                let win = b.root().and_downcast::<gtk::Window>();
                let picture = picture.clone();
                let cc = cc.clone();
                dialog.open(win.as_ref(), gio::Cancellable::NONE, move |res| {
                    if let Ok(file) = res {
                        if let Some(path) = file.path() {
                            if let Ok(data) = std::fs::read(&path) {
                                if let Some(t) = texture_from_bytes(&data) {
                                    picture.set_paintable(Some(&t));
                                    let ext = path
                                        .extension()
                                        .and_then(|e| e.to_str())
                                        .unwrap_or("jpg")
                                        .to_ascii_lowercase();
                                    *cc.borrow_mut() = CoverChange::Set(data, ext);
                                }
                            }
                        }
                    }
                });
            });
        }
        {
            let picture = picture.clone();
            let cc = cover_change.clone();
            remove.connect_clicked(move |_| {
                picture.set_paintable(gdk::Paintable::NONE);
                *cc.borrow_mut() = CoverChange::Remove;
            });
        }

        // --- basic ---------------------------------------------------------
        let basic = adw::PreferencesGroup::builder().title(tr("Basics")).build();
        let title = entry(&tr("Title"), &md.title);
        let authors = entry(
            &tr("Authors (separate with &amp;)"),
            &md.authors.join(" & "),
        );
        let series = entry(&tr("Series"), &md.series);
        let series_index = adw::SpinRow::with_range(0.0, 9999.0, 0.5);
        series_index.set_title(&tr("Series number"));
        series_index.set_digits(1);
        series_index.set_value(if md.series_index > 0.0 {
            md.series_index
        } else {
            1.0
        });
        basic.add(&title);
        basic.add(&authors);
        basic.add(&series);
        basic.add(&series_index);
        let series_text = entry(&tr("Series number, your way (3, 03, III, Book Three)"), "");
        series_text.set_tooltip_text(Some(&tr(
            "Type the number as it is written on the book; it is turned into the number above",
        )));
        let series_hint = adw::ActionRow::new();
        series_hint.set_visible(false);
        series_hint.add_css_class("dim-label");
        basic.add(&series_text);
        basic.add(&series_hint);
        let others = SERIES_CONTEXT.with(|c| std::mem::take(&mut *c.borrow_mut()));
        {
            let (spin, hint, text, ser) = (
                series_index.clone(),
                series_hint.clone(),
                series_text.clone(),
                series.clone(),
            );
            let others = Rc::new(others);
            let refresh = Rc::new(move || {
                let p = crate::series_view::number_preview(&ser.text(), &text.text(), &others);
                hint.set_visible(!p.text.is_empty());
                hint.set_title(&glib::markup_escape_text(&p.text));
                if let Some(v) = p.value {
                    spin.set_value(v);
                }
            });
            let r = refresh.clone();
            series_text.connect_changed(move |_| r());
            series.connect_changed(move |_| {
                refresh();
            });
        }
        let rating = Rc::new(Cell::new(md.rating));
        basic.add(&star_row(rating.clone()));
        page.add(&basic);

        // --- details -------------------------------------------------------
        let details = adw::PreferencesGroup::builder()
            .title(tr("Details"))
            .build();
        let tags = entry(&tr("Tags (comma separated)"), &md.tags.join(", "));
        let publisher = entry(&tr("Publisher"), &md.publisher);
        let published = entry(&tr("Published (YYYY-MM-DD)"), &md.published);
        let language = entry(&tr("Language (e.g. en)"), &md.language);
        let identifiers = entry(&tr("Identifiers (isbn:…, doi:…)"), &md.identifiers_string());
        details.add(&tags);
        details.add(&publisher);
        details.add(&published);
        details.add(&language);
        details.add(&identifiers);
        page.add(&details);

        // --- sorting -------------------------------------------------------
        let sort = adw::PreferencesGroup::builder()
            .title(tr("Sorting"))
            .description(tr("Leave empty to generate automatically"))
            .build();
        let title_sort = entry(
            &tr("Title sort"),
            if md.title_sort == title_sort_of(&md.title) {
                ""
            } else {
                &md.title_sort
            },
        );
        let author_sort = entry(&tr("Author sort"), &md.author_sort);
        sort.add(&title_sort);
        sort.add(&author_sort);
        page.add(&sort);

        // --- comments ------------------------------------------------------
        let comments = adw::PreferencesGroup::builder()
            .title(tr("Comments"))
            .build();
        let description = gtk::TextView::new();
        description.set_wrap_mode(gtk::WrapMode::WordChar);
        description.set_top_margin(8);
        description.set_bottom_margin(8);
        description.set_left_margin(10);
        description.set_right_margin(10);
        let plain = crate::core::html::parse_html(&md.description, "d.html").to_text();
        description.buffer().set_text(plain.trim());
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_min_content_height(140);
        scroller.set_child(Some(&description));
        scroller.add_css_class("card");
        comments.add(&scroller);
        page.add(&comments);

        let form = MetadataForm {
            fetched: Rc::new(RefCell::new(Vec::new())),
            widget: page.upcast(),
            orig: md.clone(),
            title,
            title_sort,
            authors,
            author_sort,
            series,
            series_index,
            series_text,
            series_hint,
            publisher,
            language,
            published,
            tags,
            identifiers,
            rating,
            description,
            cover_change,
            picture,
            fetch_btn: fetch_btn.clone(),
            replace_chk,
            status,
        };
        {
            let f = form.clone();
            fetch_btn.connect_clicked(move |_| f.start_fetch());
        }
        ACTIVE.with(|a| *a.borrow_mut() = Some(form.clone()));
        form
    }

    /// Looks the book up on a worker thread and fills the form with the answer.
    fn start_fetch(&self) {
        use crate::metadata_fetch as mf;
        let current = self.read();
        self.fetch_btn.set_sensitive(false);
        self.status.set_visible(true);
        self.status.set_label(&tr("Looking the book up…"));
        let (tx, rx) = std::sync::mpsc::channel();
        let md = current.clone();
        std::thread::spawn(move || {
            let r = mf::fetch(&md, mf::fetcher()).map_err(|e| format!("{e:#}"));
            mf::persist_health();
            let _ = tx.send(r);
        });
        let form = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(60), move || {
            match rx.try_recv() {
                Ok(res) => {
                    form.fetch_btn.set_sensitive(true);
                    match res {
                        Ok(Some(f)) => form.apply_fetched(&f),
                        Ok(None) => form.status.set_label(&tr(
                            "No match found. Check the title and author and try again.",
                        )),
                        Err(e) => form
                            .status
                            .set_label(&trf("Could not look the book up: {}", &[&e])),
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    form.fetch_btn.set_sensitive(true);
                    glib::ControlFlow::Break
                }
            }
        });
    }

    fn apply_fetched(&self, f: &crate::metadata_fetch::Fetched) {
        use crate::metadata_fetch as mf;
        let before = self.read();
        let mode = if self.replace_chk.is_active() {
            mf::Merge::Replace
        } else {
            mf::Merge::FillEmpty
        };
        let after = mf::apply(&before, &f.meta, mode);
        let changed = mf::changed_fields(&before, &after);
        {
            let mut seen = self.fetched.borrow_mut();
            for c in &changed {
                seen.retain(|(f, _)| f != c);
                seen.push((c.to_string(), f.sources.join(", ")));
            }
        }
        self.title.set_text(&after.title);
        self.authors.set_text(&after.authors.join(" & "));
        self.series.set_text(&after.series);
        if after.series_index > 0.0 {
            self.series_index.set_value(after.series_index);
        }
        self.publisher.set_text(&after.publisher);
        self.published.set_text(&after.published);
        self.language.set_text(&after.language);
        self.tags.set_text(&after.tags.join(", "));
        self.identifiers.set_text(&after.identifiers_string());
        if before.title != after.title {
            self.title_sort.set_text("");
        }
        if before.authors != after.authors {
            self.author_sort.set_text("");
        }
        if before.description != after.description {
            let plain = crate::core::html::parse_html(&after.description, "d.html").to_text();
            self.description.buffer().set_text(plain.trim());
        }
        let mut cover_note = "";
        if let Some(bytes) = &f.cover {
            let had_cover = !matches!(*self.cover_change.borrow(), CoverChange::Keep)
                || self.picture.paintable().is_some();
            if (mode == mf::Merge::Replace || !had_cover) && texture_from_bytes(bytes).is_some() {
                if let Some(t) = texture_from_bytes(bytes) {
                    self.picture.set_paintable(Some(&t));
                }
                let ext = if bytes.starts_with(b"\x89PNG") {
                    "png"
                } else {
                    "jpg"
                };
                *self.cover_change.borrow_mut() = CoverChange::Set(bytes.clone(), ext.into());
                cover_note = " Cover added.";
            }
        }
        let mut text = format!("{}{cover_note}", mf::summary(f, &changed));
        for w in &f.warnings {
            text.push_str(&format!("\n({w})"));
        }
        self.status.set_label(&text);
    }

    pub fn cover_change(&self) -> CoverChange {
        self.cover_change.borrow().clone()
    }

    pub fn read(&self) -> Metadata {
        let mut m = self.orig.clone();
        m.title = self.title.text().trim().to_string();
        m.authors = split_list(&self.authors.text(), &['&', ';']);
        m.series = self.series.text().trim().to_string();
        m.series_index = self.series_index.value();
        m.rating = self.rating.get();
        m.tags = split_list(&self.tags.text(), &[',']);
        m.publisher = self.publisher.text().trim().to_string();
        m.published = self.published.text().trim().to_string();
        m.language = self.language.text().trim().to_string();
        m.set_identifiers_from_string(&self.identifiers.text());
        let buf = self.description.buffer();
        let (s, e) = buf.bounds();
        let text = buf.text(&s, &e, false).to_string();
        let orig_plain = crate::core::html::parse_html(&self.orig.description, "d.html").to_text();
        // keep the original markup if the user did not touch the text
        if text.trim() != orig_plain.trim() {
            m.description = text.trim().to_string();
        }
        let ts = self.title_sort.text().trim().to_string();
        m.title_sort = if ts.is_empty() {
            title_sort_of(&m.title)
        } else {
            ts
        };
        let asort = self.author_sort.text().trim().to_string();
        m.author_sort = if asort.is_empty() {
            m.authors
                .iter()
                .map(|a| author_sort(a))
                .collect::<Vec<_>>()
                .join(" & ")
        } else {
            asort
        };
        m
    }
}

/// Test hook: clicks *Fetch Metadata* in the dialog that is open.
pub fn dev_fetch(replace: bool) -> bool {
    ACTIVE.with(|a| match a.borrow().as_ref() {
        Some(f) if f.fetch_btn.is_mapped() => {
            f.replace_chk.set_active(replace);
            f.fetch_btn.emit_clicked();
            true
        }
        _ => false,
    })
}

/// Test hook: types `text` into the series number field of the open dialog.
pub fn dev_series_text(text: &str) -> bool {
    ACTIVE.with(|a| match a.borrow().as_ref() {
        Some(f) if f.widget.is_mapped() => {
            f.series_text.set_text(text);
            true
        }
        _ => false,
    })
}

/// Test hook: the series number and the preview line of the open dialog.
pub fn dev_series_dump() -> String {
    ACTIVE.with(|a| match a.borrow().as_ref() {
        Some(f) => format!(
            "series={:?} number={} hint={:?} hint_visible={}",
            f.series.text(),
            f.series_index.value(),
            f.series_hint.title(),
            f.series_hint.is_visible()
        ),
        None => "no form".into(),
    })
}

/// Test hook: what the open dialog shows.
pub fn dev_dump() -> String {
    ACTIVE.with(|a| match a.borrow().as_ref() {
        Some(f) => format!(
            "title={:?} publisher={:?} published={:?} series={:?} tags={:?} ids={:?} cover={} status={:?} busy={}",
            f.title.text(),
            f.publisher.text(),
            f.published.text(),
            f.series.text(),
            f.tags.text(),
            f.identifiers.text(),
            matches!(*f.cover_change.borrow(), CoverChange::Set(..)),
            f.status.label(),
            !f.fetch_btn.is_sensitive(),
        ),
        None => "no form".into(),
    })
}

fn title_sort_of(t: &str) -> String {
    title_sort(t)
}

pub fn split_list(s: &str, seps: &[char]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in s.split(|c| seps.contains(&c)) {
        let p = p.trim();
        if !p.is_empty() && !out.iter().any(|o| o.eq_ignore_ascii_case(p)) {
            out.push(p.to_string());
        }
    }
    out
}

/// Shows the "Edit metadata" dialog. `on_save` is called with the new metadata and cover change.
pub fn edit_dialog(
    parent: &impl IsA<gtk::Widget>,
    md: &Metadata,
    cover: Option<Vec<u8>>,
    on_save: impl Fn(Metadata, CoverChange) + 'static,
) {
    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(&tr("Edit Metadata"));
    dialog.set_content_width(620);
    dialog.set_content_height(760);
    let form = Rc::new(MetadataForm::new(md, cover.as_deref()));

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.set_show_start_title_buttons(false);
    header.set_show_end_title_buttons(false);
    let cancel = gtk::Button::with_label(&tr("Cancel"));
    let save = gtk::Button::with_label(&tr("Save"));
    save.add_css_class("suggested-action");
    header.pack_start(&cancel);
    header.pack_end(&save);
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&form.widget));
    dialog.set_child(Some(&toolbar));
    dialog.set_default_widget(Some(&save));

    {
        let d = dialog.clone();
        cancel.connect_clicked(move |_| {
            d.close();
        });
    }
    {
        let d = dialog.clone();
        let form = form.clone();
        save.connect_clicked(move |_| {
            LAST_FETCH.with(|l| *l.borrow_mut() = form.fetched.borrow().clone());
            on_save(form.read(), form.cover_change());
            d.close();
        });
    }
    SAVE_BTN.with(|b| *b.borrow_mut() = Some(save.clone()));
    dialog.present(Some(parent));
}

/// What the edit that was just saved took from online sources: `(field, source)`.
pub fn take_fetch_sources() -> Vec<(String, String)> {
    LAST_FETCH.with(|l| std::mem::take(&mut *l.borrow_mut()))
}

/// Test hook: presses Save in the open edit dialog.
pub fn dev_save() -> bool {
    SAVE_BTN.with(|b| match b.borrow().as_ref() {
        Some(s) if s.is_mapped() => {
            s.emit_clicked();
            true
        }
        _ => false,
    })
}

/// Bulk editor for several books.
#[derive(Debug, Clone, Default)]
pub struct BulkEdit {
    pub add_tags: Vec<String>,
    pub remove_tags: Vec<String>,
    pub author: Option<String>,
    pub series: Option<String>,
    pub publisher: Option<String>,
    pub language: Option<String>,
    pub rating: Option<u8>,
}

impl BulkEdit {
    pub fn apply(&self, m: &mut Metadata) {
        for t in &self.add_tags {
            if !m.tags.iter().any(|x| x.eq_ignore_ascii_case(t)) {
                m.tags.push(t.clone());
            }
        }
        m.tags
            .retain(|t| !self.remove_tags.iter().any(|r| r.eq_ignore_ascii_case(t)));
        if let Some(a) = &self.author {
            m.authors = split_list(a, &['&', ';']);
            m.author_sort = m
                .authors
                .iter()
                .map(|a| author_sort(a))
                .collect::<Vec<_>>()
                .join(" & ");
        }
        if let Some(s) = &self.series {
            m.series = s.clone();
        }
        if let Some(p) = &self.publisher {
            m.publisher = p.clone();
        }
        if let Some(l) = &self.language {
            m.language = l.clone();
        }
        if let Some(r) = self.rating {
            m.rating = r;
        }
    }
}

impl BulkEdit {
    /// `tag:+a|tag:-b|author:X|series:S|publisher:P|language:l|rating:6` (the developer script).
    pub fn from_spec(spec: &str) -> BulkEdit {
        let mut e = BulkEdit::default();
        for part in spec.split('|') {
            match part.split_once(':') {
                Some(("tag", v)) if v.starts_with('+') => e.add_tags.push(v[1..].to_string()),
                Some(("tag", v)) if v.starts_with('-') => e.remove_tags.push(v[1..].to_string()),
                Some(("author", v)) => e.author = Some(v.to_string()),
                Some(("series", v)) => e.series = Some(v.to_string()),
                Some(("publisher", v)) => e.publisher = Some(v.to_string()),
                Some(("language", v)) => e.language = Some(v.to_string()),
                Some(("rating", v)) => e.rating = v.parse().ok(),
                _ => {}
            }
        }
        e
    }
}

/// Applies a bulk edit to `ids`: one logged edit per changed book, and the changed fields record
/// `bulk edit` as their source. The dialog and the developer script use this one function.
pub fn apply_bulk(
    shared: &super::SharedRef,
    ids: &[u64],
    edit: &BulkEdit,
) -> anyhow::Result<usize> {
    let before = crate::library_health::snapshot_meta(&shared.lib.borrow(), ids);
    let res = shared.lib.borrow_mut().bulk_edit(ids, |m| edit.apply(m));
    crate::library_health::record_changes(
        &shared.guard.borrow().store,
        &shared.lib.borrow(),
        &before,
        crate::library_health::BULK_EDIT,
        chrono::Utc::now().timestamp(),
    );
    res
}

pub fn bulk_dialog(
    parent: &impl IsA<gtk::Widget>,
    count: usize,
    on_apply: impl Fn(BulkEdit) + 'static,
) {
    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(&trf("Edit {} Books", &[&count]));
    dialog.set_content_width(520);
    dialog.set_content_height(560);

    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::builder()
        .description(tr("Only the fields you fill in are changed"))
        .build();
    let add_tags = entry(&tr("Add tags (comma separated)"), "");
    let remove_tags = entry(&tr("Remove tags (comma separated)"), "");
    let author = entry(&tr("Set authors"), "");
    let series = entry(&tr("Set series"), "");
    let publisher = entry(&tr("Set publisher"), "");
    let language = entry(&tr("Set language"), "");
    for r in [
        &add_tags,
        &remove_tags,
        &author,
        &series,
        &publisher,
        &language,
    ] {
        group.add(r);
    }
    let rating = Rc::new(Cell::new(0u8));
    let rating_row = star_row(rating.clone());
    rating_row.set_title(&tr("Set rating (click to choose)"));
    group.add(&rating_row);
    page.add(&group);

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.set_show_start_title_buttons(false);
    header.set_show_end_title_buttons(false);
    let cancel = gtk::Button::with_label(&tr("Cancel"));
    let apply = gtk::Button::with_label(&tr("Apply"));
    apply.add_css_class("suggested-action");
    header.pack_start(&cancel);
    header.pack_end(&apply);
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&page));
    dialog.set_child(Some(&toolbar));

    {
        let d = dialog.clone();
        cancel.connect_clicked(move |_| {
            d.close();
        });
    }
    {
        let d = dialog.clone();
        apply.connect_clicked(move |_| {
            let opt = |e: &adw::EntryRow| {
                let t = e.text().trim().to_string();
                if t.is_empty() {
                    None
                } else {
                    Some(t)
                }
            };
            on_apply(BulkEdit {
                add_tags: split_list(&add_tags.text(), &[',']),
                remove_tags: split_list(&remove_tags.text(), &[',']),
                author: opt(&author),
                series: opt(&series),
                publisher: opt(&publisher),
                language: opt(&language),
                rating: if rating.get() > 0 {
                    Some(rating.get())
                } else {
                    None
                },
            });
            d.close();
        });
    }
    dialog.present(Some(parent));
}
