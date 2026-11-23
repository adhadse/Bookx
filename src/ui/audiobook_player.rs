//! The audiobook player (PP-G-25): a page over the library with the cover, the chapter, a seek
//! bar for the whole book, play/pause, previous/next chapter, back 15 s and forward 30 s, speed
//! (pitch kept), a sleep timer, bookmarks with notes and the chapter list. Media keys come from
//! MPRIS. The logic is `crate::audiobook::Listening` (unit tested); this file is the widgets.
//!
//! Dev hooks: see `dev` (`audio:...` commands of `BOOKX_DEV_SCRIPT`).

use crate::i18n::{N_, tr, trf};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use bookx_audio::model::clock;
use bookx_tts::SleepTimer;
use relm4::RelmWidgetExt;
use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, glib};

use super::{SharedRef, audit, covers};
use crate::audiobook::{self as ab, Listening, Sleep};
use crate::core::formats::AUDIOBOOK;
use crate::core::library::Book;

/// Speeds offered as buttons.
const PRESETS: [f32; 7] = [0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 3.0];
/// Sleep timer entries: label, setting.
const SLEEPS: [(&str, Sleep); 8] = [
    (N_("Off"), Sleep::Off),
    (N_("5 minutes"), Sleep::Minutes(5)),
    (N_("10 minutes"), Sleep::Minutes(10)),
    (N_("15 minutes"), Sleep::Minutes(15)),
    (N_("30 minutes"), Sleep::Minutes(30)),
    (N_("45 minutes"), Sleep::Minutes(45)),
    (N_("1 hour"), Sleep::Minutes(60)),
    (N_("End of chapter"), Sleep::EndOfChapter),
];
/// How long the seek bar waits for the pointer to rest before the audio jumps.
const SEEK_SETTLE_MS: u64 = 250;

pub struct Page {
    pub page: adw::NavigationPage,
    inner: Rc<Inner>,
}

thread_local! {
    static LAST_DELETE: std::cell::RefCell<Option<Rc<dyn Fn()>>> = const { std::cell::RefCell::new(None) };
}

struct Inner {
    shared: SharedRef,
    l: RefCell<Listening>,
    mpris: RefCell<Option<crate::mpris::Mpris>>,
    chapter: gtk::Label,
    chapter_no: gtk::Label,
    seek: gtk::Scale,
    elapsed: gtk::Label,
    remaining: gtk::Label,
    progress: gtk::Label,
    play: gtk::Button,
    speed_btn: gtk::MenuButton,
    speed_scale: gtk::Scale,
    sleep_label: gtk::Label,
    volume: gtk::Scale,
    error: gtk::Label,
    chapters: gtk::ListBox,
    bookmarks: gtk::ListBox,
    note: gtk::Entry,
    shown_chapter: Cell<usize>,
    updating: Cell<bool>,
    pending_seek: Cell<Option<u64>>,
    closed: Cell<bool>,
    toast: Rc<dyn Fn(String)>,
    fit: Rc<dyn Fn()>,
}

fn icon_button(icon: &str, name: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.set_tooltip_text(Some(name));
    audit::label(&b, name);
    b.add_css_class("flat");
    b.add_css_class("circular");
    b
}

fn text_button(text: &str, name: &str) -> gtk::Button {
    let b = gtk::Button::with_label(text);
    b.set_tooltip_text(Some(name));
    audit::label(&b, name);
    // the visible text is the name a screen reader gets; this says what it means
    b.update_property(&[gtk::accessible::Property::Description(name)]);
    b.add_css_class("flat");
    b.add_css_class("circular");
    b
}

fn speed_text(s: f32) -> String {
    let t = format!("{s:.2}");
    let t = t.trim_end_matches('0').trim_end_matches('.');
    format!("{t}×")
}

/// Opens the player for book `id`. Fails when the audio is gone or cannot be read.
pub fn open(shared: &SharedRef, id: u64, toast: Rc<dyn Fn(String)>) -> anyhow::Result<Page> {
    let data = ab::data_dir().ok_or_else(|| anyhow::anyhow!("no settings folder"))?;
    let (loaded, title, authors, cover_path, pos) = {
        let lib = shared.lib.borrow();
        let b = lib.get(id).ok_or_else(|| anyhow::anyhow!("no such book"))?;
        let loaded = ab::load(&lib.root, b, &data)?;
        (
            loaded,
            b.meta.title.clone(),
            b.meta.authors.join(" & "),
            lib.cover_path(b),
            b.position,
        )
    };
    let book = loaded.book.clone();
    let own = shared.lib.borrow().get(id).is_some_and(ab::is_own);
    let sh = shared.clone();
    let on_progress: Box<dyn FnMut(ab::Progress)> = Box::new(move |p| {
        if !own {
            return;
        }
        let mut lib = sh.lib.borrow_mut();
        lib.set_progress(id, p.read_pos(), p.fraction);
        let _ = lib.save();
    });
    let sink = bookx_audio::player::sink_from_env();
    let listening = Listening::open(
        loaded,
        id,
        &data,
        sink,
        on_progress,
        if own { pos } else { None },
    );

    let narrator = book.narrator.clone();
    let subtitle = if narrator.is_empty() {
        authors.clone()
    } else {
        trf("{} · narrated by {}", &[&authors, &narrator])
    };

    // ---- widgets ----
    let cover = gtk::Picture::new();
    cover.set_content_fit(gtk::ContentFit::Cover);
    cover.set_can_shrink(true);
    cover.set_size_request(180, 270);
    cover.set_accessible_role(gtk::AccessibleRole::Presentation);
    // without a picture the cover is a coloured card with the title, like in the library grid
    let cover_box = gtk::Overlay::new();
    cover_box.set_halign(gtk::Align::Center);
    cover_box.set_valign(gtk::Align::Start);
    cover_box.add_css_class("card");
    cover_box.set_overflow(gtk::Overflow::Hidden);
    let fallback = gtk::CenterBox::new();
    fallback.set_size_request(180, 270);
    fallback.add_css_class(&covers::placeholder_class(&title));
    let fallback_title = gtk::Label::new(Some(&title));
    fallback_title.set_wrap(true);
    fallback_title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    fallback_title.set_justify(gtk::Justification::Center);
    fallback_title.set_margin_all(12);
    fallback_title.set_max_width_chars(14);
    fallback_title.add_css_class("cover-title");
    fallback.set_center_widget(Some(&fallback_title));
    cover_box.set_child(Some(&fallback));
    cover_box.set_accessible_role(gtk::AccessibleRole::Presentation);
    if let Some(t) = cover_path
        .as_deref()
        .and_then(|p| covers::thumbnail(p, 360))
    {
        cover.set_paintable(Some(&t));
        cover_box.add_overlay(&cover);
    }

    let chapter = gtk::Label::new(None);
    chapter.add_css_class("title-3");
    chapter.set_wrap(true);
    chapter.set_justify(gtk::Justification::Center);
    chapter.set_max_width_chars(36);
    let chapter_no = gtk::Label::new(None);
    chapter_no.add_css_class("dim-label");
    chapter_no.add_css_class("caption");

    let seek = gtk::Scale::with_range(
        gtk::Orientation::Horizontal,
        0.0,
        (book.total_ms / 1000).max(1) as f64,
        1.0,
    );
    seek.set_draw_value(false);
    seek.set_hexpand(true);
    audit::label(&seek, &tr("Position in the audiobook"));
    let elapsed = gtk::Label::new(Some("0:00"));
    let remaining = gtk::Label::new(Some("0:00"));
    for l in [&elapsed, &remaining] {
        l.add_css_class("caption");
        l.add_css_class("numeric");
        l.add_css_class("dim-label");
    }
    let seek_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    seek_row.append(&elapsed);
    seek_row.append(&seek);
    seek_row.append(&remaining);

    let progress = gtk::Label::new(None);
    progress.add_css_class("caption");
    progress.add_css_class("dim-label");

    let prev_ch = icon_button("media-skip-backward-symbolic", &tr("Previous Chapter"));
    let back = text_button("−15", &tr("Back 15 seconds"));
    let play = icon_button("media-playback-start-symbolic", &tr("Play"));
    play.remove_css_class("flat");
    play.add_css_class("suggested-action");
    play.set_size_request(56, 56);
    let fwd = text_button("+30", &tr("Forward 30 seconds"));
    let next_ch = icon_button("media-skip-forward-symbolic", &tr("Next Chapter"));
    for b in [&back, &fwd] {
        b.set_size_request(44, 44);
    }
    let transport = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    transport.set_halign(gtk::Align::Center);
    transport.set_valign(gtk::Align::Center);
    for b in [&prev_ch, &back, &play, &fwd, &next_ch] {
        transport.append(b);
    }

    // speed
    let speed_btn = gtk::MenuButton::new();
    speed_btn.set_label(&speed_text(listening.state.speed));
    speed_btn.set_tooltip_text(Some(&tr("Playback Speed")));
    audit::label(&speed_btn, &tr("Playback Speed"));
    let speed_pop = gtk::Popover::new();
    let sbox = gtk::Box::new(gtk::Orientation::Vertical, 8);
    sbox.set_margin_all(10);
    let speed_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.5, 3.0, 0.05);
    speed_scale.set_draw_value(false);
    speed_scale.set_size_request(240, -1);
    speed_scale.set_value(listening.state.speed as f64);
    audit::label(&speed_scale, &tr("Speed"));
    sbox.append(&speed_scale);
    let presets = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    presets.add_css_class("linked");
    for p in PRESETS {
        let b = gtk::Button::with_label(&speed_text(p));
        audit::label(&b, &trf("Speed {}", &[&speed_text(p)]));
        presets.append(&b);
    }
    sbox.append(&presets);
    let note = gtk::Label::new(Some(&tr("The pitch of the voice stays the same.")));
    note.add_css_class("caption");
    note.add_css_class("dim-label");
    sbox.append(&note);
    speed_pop.set_child(Some(&sbox));
    speed_btn.set_popover(Some(&speed_pop));

    // sleep timer
    let sleep_btn = gtk::MenuButton::new();
    let sleep_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    sleep_box.append(&gtk::Image::from_icon_name("alarm-symbolic"));
    let sleep_label = gtk::Label::new(None);
    sleep_label.add_css_class("numeric");
    sleep_label.set_visible(false);
    sleep_box.append(&sleep_label);
    sleep_btn.set_child(Some(&sleep_box));
    sleep_btn.set_tooltip_text(Some(&tr("Sleep Timer")));
    audit::label(&sleep_btn, &tr("Sleep Timer"));
    let sleep_pop = gtk::Popover::new();
    let slist = gtk::Box::new(gtk::Orientation::Vertical, 2);
    slist.set_margin_all(6);
    sleep_pop.set_child(Some(&slist));
    sleep_btn.set_popover(Some(&sleep_pop));

    // volume
    let volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.05);
    volume.set_draw_value(false);
    volume.set_value(1.0);
    volume.set_size_request(110, -1);
    audit::label(&volume, &tr("Volume"));
    let vol_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    vol_box.append(&gtk::Image::from_icon_name("audio-volume-high-symbolic"));
    vol_box.append(&volume);

    let extras = adw::WrapBox::new();
    extras.set_child_spacing(10);
    extras.set_line_spacing(6);
    extras.set_align(0.5);
    extras.append(&speed_btn);
    extras.append(&sleep_btn);
    extras.append(&vol_box);

    let error = gtk::Label::new(None);
    error.set_wrap(true);
    error.add_css_class("error");
    error.set_visible(false);
    error.set_selectable(true);

    // chapters
    let chapters = gtk::ListBox::new();
    chapters.add_css_class("navigation-sidebar");
    chapters.set_selection_mode(gtk::SelectionMode::Single);
    audit::label(&chapters, &tr("Chapters"));
    for (i, c) in book.chapters.iter().enumerate() {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.set_margin_top(4);
        row.set_margin_bottom(4);
        let n = gtk::Label::new(Some(&(i + 1).to_string()));
        n.add_css_class("dim-label");
        n.add_css_class("numeric");
        n.set_width_chars(3);
        let t = gtk::Label::new(Some(&c.title));
        t.set_xalign(0.0);
        t.set_hexpand(true);
        t.set_ellipsize(gtk::pango::EllipsizeMode::End);
        t.set_max_width_chars(34);
        let d = gtk::Label::new(Some(&clock(c.duration_ms())));
        d.add_css_class("dim-label");
        d.add_css_class("numeric");
        row.append(&n);
        row.append(&t);
        row.append(&d);
        let r = gtk::ListBoxRow::new();
        r.set_child(Some(&row));
        audit::label(&r, &trf("Chapter {}: {}", &[&(i + 1), &c.title]));
        chapters.append(&r);
    }
    let chapters_btn = gtk::MenuButton::new();
    chapters_btn.set_icon_name("view-list-symbolic");
    chapters_btn.set_tooltip_text(Some(&tr("Chapters")));
    audit::label(&chapters_btn, &tr("Chapters"));
    let chapters_pop = gtk::Popover::new();
    let cs = gtk::ScrolledWindow::new();
    cs.set_child(Some(&chapters));
    cs.set_min_content_width(340);
    cs.set_min_content_height(160);
    cs.set_max_content_height(380);
    cs.set_propagate_natural_height(true);
    chapters_pop.set_child(Some(&cs));
    chapters_btn.set_popover(Some(&chapters_pop));

    // bookmarks
    let bookmarks = gtk::ListBox::new();
    bookmarks.add_css_class("navigation-sidebar");
    bookmarks.set_selection_mode(gtk::SelectionMode::None);
    audit::label(&bookmarks, &tr("Bookmarks"));
    let note_entry = gtk::Entry::new();
    note_entry.set_placeholder_text(Some(&tr("Note (optional)")));
    note_entry.set_hexpand(true);
    audit::label(&note_entry, &tr("Bookmark note"));
    let add_bm = gtk::Button::with_label(&tr("Add"));
    add_bm.add_css_class("suggested-action");
    audit::label(&add_bm, &tr("Add a bookmark at this position"));
    let add_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    add_row.append(&note_entry);
    add_row.append(&add_bm);
    let bm_scroll = gtk::ScrolledWindow::new();
    bm_scroll.set_child(Some(&bookmarks));
    bm_scroll.set_min_content_width(340);
    bm_scroll.set_min_content_height(80);
    bm_scroll.set_max_content_height(300);
    bm_scroll.set_propagate_natural_height(true);
    let bm_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
    bm_box.set_margin_all(10);
    bm_box.append(&add_row);
    bm_box.append(&bm_scroll);
    let bm_btn = gtk::MenuButton::new();
    bm_btn.set_icon_name("bookmark-new-symbolic");
    bm_btn.set_tooltip_text(Some(&tr("Bookmarks")));
    audit::label(&bm_btn, &tr("Bookmarks"));
    let bm_pop = gtk::Popover::new();
    bm_pop.set_child(Some(&bm_box));
    bm_btn.set_popover(Some(&bm_pop));

    // ---- layout ----
    let header = adw::HeaderBar::new();
    let wt = adw::WindowTitle::new(&title, &subtitle);
    header.set_title_widget(Some(&wt));
    header.pack_end(&bm_btn);
    header.pack_end(&chapters_btn);
    let card = gtk::Box::new(gtk::Orientation::Vertical, 14);
    card.set_margin_top(18);
    card.set_margin_bottom(18);
    card.set_margin_start(18);
    card.set_margin_end(18);
    card.append(&cover_box);
    let names = gtk::Box::new(gtk::Orientation::Vertical, 2);
    names.append(&chapter);
    names.append(&chapter_no);
    card.append(&names);
    card.append(&seek_row);
    card.append(&progress);
    card.append(&transport);
    card.append(&extras);
    card.append(&error);
    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(560);
    clamp.set_child(Some(&card));
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&clamp));
    scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    // the cover takes what the height leaves after the controls (about 360 px); GTK has no height
    // notification, so the clock of the page calls this
    let fit: Rc<dyn Fn()> = {
        let (cover, fallback, scroll, cover_box) = (
            cover.clone(),
            fallback.clone(),
            scroll.clone(),
            cover_box.clone(),
        );
        let last = Cell::new(0);
        Rc::new(move || {
            let h = (scroll.height() - 360).clamp(110, 270);
            if h == last.get() {
                return;
            }
            last.set(h);
            let w = h * 2 / 3;
            for wd in [
                cover.upcast_ref::<gtk::Widget>(),
                fallback.upcast_ref(),
                cover_box.upcast_ref(),
            ] {
                wd.set_size_request(w, h);
            }
        })
    };
    let tb = adw::ToolbarView::new();
    tb.add_top_bar(&header);
    tb.set_content(Some(&scroll));
    let page = adw::NavigationPage::builder()
        .title(&title)
        .tag(unique_tag())
        .build();
    page.set_child(Some(&tb));
    audit::label(&card, &tr("Audiobook player"));

    let inner = Rc::new(Inner {
        shared: shared.clone(),
        l: RefCell::new(listening),
        mpris: RefCell::new(None),
        chapter,
        chapter_no,
        seek: seek.clone(),
        elapsed,
        remaining,
        progress,
        play: play.clone(),
        speed_btn: speed_btn.clone(),
        speed_scale: speed_scale.clone(),
        sleep_label,
        volume: volume.clone(),
        error,
        chapters: chapters.clone(),
        bookmarks,
        note: note_entry.clone(),
        shown_chapter: Cell::new(usize::MAX),
        updating: Cell::new(false),
        pending_seek: Cell::new(None),
        closed: Cell::new(false),
        toast,
        fit,
    });

    // sleep entries
    for (label, setting) in SLEEPS {
        let b = gtk::Button::with_label(&tr(label));
        b.add_css_class("flat");
        if let Some(l) = b.child().and_downcast::<gtk::Label>() {
            l.set_xalign(0.0);
        }
        let (w, pop) = (Rc::downgrade(&inner), sleep_pop.clone());
        b.connect_clicked(move |_| {
            pop.popdown();
            if let Some(i) = w.upgrade() {
                i.set_sleep(setting);
            }
        });
        slist.append(&b);
    }

    // transport
    let w = Rc::downgrade(&inner);
    let call = move |f: fn(&Inner)| {
        let w = w.clone();
        move |_: &gtk::Button| {
            if let Some(i) = w.upgrade() {
                f(&i);
            }
        }
    };
    play.connect_clicked(call(|i| i.toggle()));
    prev_ch.connect_clicked(call(|i| i.l.borrow_mut().previous_chapter()));
    next_ch.connect_clicked(call(|i| i.l.borrow_mut().next_chapter()));
    back.connect_clicked(call(|i| i.l.borrow_mut().skip(ab::SKIP_BACK_MS)));
    fwd.connect_clicked(call(|i| i.l.borrow_mut().skip(ab::SKIP_FORWARD_MS)));

    // seek bar: the audio jumps when the pointer has rested
    {
        let w = Rc::downgrade(&inner);
        seek.connect_change_value(move |_, _, v| {
            if let Some(i) = w.upgrade() {
                if !i.updating.get() {
                    i.schedule_seek((v.max(0.0) * 1000.0) as u64);
                }
            }
            glib::Propagation::Proceed
        });
    }
    {
        let w = Rc::downgrade(&inner);
        speed_scale.connect_value_changed(move |s| {
            if let Some(i) = w.upgrade() {
                if !i.updating.get() {
                    i.set_speed(((s.value() * 20.0).round() / 20.0) as f32);
                }
            }
        });
    }
    let mut c = presets.first_child();
    while let Some(b) = c {
        if let Some(btn) = b.downcast_ref::<gtk::Button>() {
            let sp = btn
                .label()
                .map(|l| l.trim_end_matches('×').parse::<f32>().unwrap_or(1.0))
                .unwrap_or(1.0);
            let w = Rc::downgrade(&inner);
            btn.connect_clicked(move |_| {
                if let Some(i) = w.upgrade() {
                    i.set_speed(sp);
                }
            });
        }
        c = b.next_sibling();
    }
    {
        let w = Rc::downgrade(&inner);
        volume.connect_value_changed(move |v| {
            if let Some(i) = w.upgrade() {
                i.l.borrow().player.set_volume(v.value() as f32);
            }
        });
    }
    {
        let w = Rc::downgrade(&inner);
        chapters.connect_row_activated(move |_, row| {
            if let Some(i) = w.upgrade() {
                i.l.borrow_mut().goto_chapter(row.index() as usize);
                chapters_pop.popdown();
            }
        });
    }
    {
        let w = Rc::downgrade(&inner);
        let add = move || {
            if let Some(i) = w.upgrade() {
                i.add_bookmark();
            }
        };
        let a2 = add.clone();
        add_bm.connect_clicked(move |_| add());
        note_entry.connect_activate(move |_| a2());
    }
    // keyboard: space, arrows, page keys (a focused slider keeps its own arrows)
    {
        let key = gtk::EventControllerKey::new();
        let w = Rc::downgrade(&inner);
        key.connect_key_pressed(move |_, k, _, mods| {
            use relm4::gtk::gdk::Key;
            let Some(i) = w.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let shift = mods.contains(relm4::gtk::gdk::ModifierType::SHIFT_MASK);
            match k {
                Key::space => i.toggle(),
                Key::Left if shift => i.l.borrow_mut().skip(-30_000),
                Key::Left => i.l.borrow_mut().skip(ab::SKIP_BACK_MS),
                Key::Right if shift => i.l.borrow_mut().skip(30_000),
                Key::Right => i.l.borrow_mut().skip(15_000),
                Key::Page_Up => i.l.borrow_mut().previous_chapter(),
                Key::Page_Down => i.l.borrow_mut().next_chapter(),
                Key::Escape => {
                    let _ = relm4::gtk::prelude::WidgetExt::activate_action(
                        &i.progress,
                        "navigation.pop",
                        None,
                    );
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        key.set_propagation_phase(gtk::PropagationPhase::Capture);
        page.add_controller(key);
    }

    inner.fill_bookmarks();
    inner.refresh();
    if std::env::var_os("BOOKX_NO_MPRIS").is_none() {
        *inner.mpris.borrow_mut() = crate::mpris::Mpris::start(None);
    }
    // the clock
    {
        let w = Rc::downgrade(&inner);
        glib::timeout_add_local(Duration::from_millis(200), move || match w.upgrade() {
            Some(i) if !i.closed.get() => {
                i.tick();
                glib::ControlFlow::Continue
            }
            _ => glib::ControlFlow::Break,
        });
    }
    Ok(Page { page, inner })
}

impl Inner {
    fn toggle(&self) {
        self.l.borrow_mut().play_pause();
    }

    fn set_speed(&self, s: f32) {
        self.l.borrow_mut().set_speed(s);
        self.refresh();
    }

    fn set_sleep(&self, s: Sleep) {
        self.l.borrow_mut().set_sleep(s, Instant::now());
        self.refresh();
    }

    fn schedule_seek(self: &Rc<Self>, ms: u64) {
        let first = self.pending_seek.replace(Some(ms)).is_none();
        // the labels follow the pointer at once
        let total = self.l.borrow().book.total_ms;
        self.elapsed.set_label(&clock(ms));
        self.remaining
            .set_label(&format!("-{}", clock(total.saturating_sub(ms))));
        if first {
            let w = Rc::downgrade(self);
            glib::timeout_add_local_once(Duration::from_millis(SEEK_SETTLE_MS), move || {
                if let Some(i) = w.upgrade() {
                    if let Some(ms) = i.pending_seek.take() {
                        i.l.borrow_mut().seek(ms);
                        i.refresh();
                    }
                }
            });
        }
    }

    fn add_bookmark(self: &Rc<Self>) {
        let text = self.note.text().to_string();
        let ok = self
            .l
            .borrow_mut()
            .add_bookmark(&text, chrono::Utc::now())
            .is_some();
        if ok {
            self.note.set_text("");
            self.fill_bookmarks();
            (self.toast)(tr("Bookmark added"));
        } else {
            (self.toast)(tr("Too many bookmarks"));
        }
    }

    /// Deletes a bookmark and offers to take it back with a toast.
    fn delete_bookmark(self: &Rc<Self>, id: &str) {
        let Some(m) = self
            .l
            .borrow()
            .state
            .bookmarks
            .iter()
            .find(|b| b.id == id)
            .cloned()
        else {
            return;
        };
        self.l.borrow_mut().remove_bookmark(id, chrono::Utc::now());
        self.fill_bookmarks();
        let w = Rc::downgrade(self);
        let restore: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(i) = w.upgrade() {
                i.l.borrow_mut()
                    .restore_bookmark(m.position_ms, &m.note, chrono::Utc::now());
                i.fill_bookmarks();
            }
        });
        LAST_DELETE.with(|l| *l.borrow_mut() = Some(restore.clone()));
        let overlay = self
            .bookmarks
            .root()
            .and_then(|r| super::dialog_toast::find_overlay(&r));
        match overlay {
            Some(o) => {
                super::dialog_toast::show_undo(&o, &tr("Bookmark deleted"), move || restore())
            }
            None => (self.toast)(tr("Bookmark deleted")),
        }
    }

    fn fill_bookmarks(self: &Rc<Self>) {
        while let Some(c) = self.bookmarks.first_child() {
            self.bookmarks.remove(&c);
        }
        let marks = self.l.borrow().state.bookmarks.clone();
        if marks.is_empty() {
            let l = gtk::Label::new(Some(&tr("No bookmarks yet")));
            l.add_css_class("dim-label");
            l.set_margin_all(8);
            self.bookmarks.append(&l);
        }
        for m in marks {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.set_margin_top(2);
            row.set_margin_bottom(2);
            let go = gtk::Button::new();
            go.add_css_class("flat");
            go.set_hexpand(true);
            let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let t = gtk::Label::new(Some(&clock(m.position_ms)));
            t.set_xalign(0.0);
            t.add_css_class("numeric");
            let n = gtk::Label::new(Some(&m.note));
            n.set_xalign(0.0);
            n.set_ellipsize(gtk::pango::EllipsizeMode::End);
            n.set_max_width_chars(30);
            n.add_css_class("dim-label");
            n.add_css_class("caption");
            n.set_visible(!m.note.is_empty());
            text.append(&t);
            text.append(&n);
            go.set_child(Some(&text));
            audit::label(&go, &trf("Go to {} {}", &[&clock(m.position_ms), &m.note]));
            let del = icon_button("edit-delete-symbolic", &tr("Delete Bookmark"));
            row.append(&go);
            row.append(&del);
            self.bookmarks.append(&row);
            let (id, id2) = (m.id.clone(), m.id.clone());
            let w = Rc::downgrade(self);
            go.connect_clicked(move |_| {
                if let Some(i) = w.upgrade() {
                    i.l.borrow_mut().go_to_bookmark(&id);
                }
            });
            let w = Rc::downgrade(self);
            del.connect_clicked(move |_| {
                if let Some(i) = w.upgrade() {
                    i.delete_bookmark(&id2);
                }
            });
        }
    }

    fn refresh(&self) {
        let (
            st,
            total,
            chapter_idx,
            chapter_title,
            nchap,
            speed,
            sleep_left,
            sleep_kind,
            playing,
            marks,
        ) = {
            let l = self.l.borrow();
            let st = l.player.status();
            let idx = l.book.chapter_index_at(st.position_ms);
            let title = l
                .book
                .chapters
                .get(idx)
                .map(|c| c.title.clone())
                .unwrap_or_default();
            let left = match l.sleep {
                SleepTimer::At(t) => Some(t.saturating_duration_since(Instant::now())),
                _ => None,
            };
            let kind = l.sleep;
            (
                st.clone(),
                l.book.total_ms,
                idx,
                title,
                l.book.chapters.len(),
                l.state.speed,
                left,
                kind,
                st.state == bookx_audio::player::State::Playing,
                l.state.bookmarks.len(),
            )
        };
        let _ = marks;
        self.updating.set(true);
        let pos = self.pending_seek.get().unwrap_or(st.position_ms);
        if self.pending_seek.get().is_none() {
            self.seek.set_value(pos as f64 / 1000.0);
            self.elapsed.set_label(&clock(pos));
            self.remaining
                .set_label(&format!("-{}", clock(total.saturating_sub(pos))));
        }
        self.chapter.set_label(&chapter_title);
        self.chapter_no
            .set_label(&trf("Chapter {} of {}", &[&(chapter_idx + 1), &nchap]));
        let (icon, name) = if playing {
            ("media-playback-pause-symbolic", tr("Pause"))
        } else {
            ("media-playback-start-symbolic", tr("Play"))
        };
        if self.play.icon_name().as_deref() != Some(icon) {
            self.play.set_icon_name(icon);
            self.play.set_tooltip_text(Some(&name));
            audit::label(&self.play, &name);
        }
        self.speed_btn.set_label(&speed_text(speed));
        if (self.speed_scale.value() - speed as f64).abs() > 0.001 {
            self.speed_scale.set_value(speed as f64);
        }
        if (self.volume.value() - st.volume as f64).abs() > 0.001 {
            self.volume.set_value(st.volume as f64);
        }
        let text = {
            let l = self.l.borrow();
            ab::progress_text(&l.book, pos, speed)
        };
        self.progress.set_label(&text);
        match (sleep_kind, sleep_left) {
            (SleepTimer::At(_), Some(d)) => {
                self.sleep_label.set_label(&clock(d.as_millis() as u64));
                self.sleep_label.set_visible(true);
            }
            (SleepTimer::EndOfChapter, _) => {
                self.sleep_label.set_label(&tr("Chapter"));
                self.sleep_label.set_visible(true);
            }
            _ => self.sleep_label.set_visible(false),
        }
        match &st.error {
            Some(e) => {
                self.error.set_label(e);
                self.error.set_visible(true);
            }
            None => self.error.set_visible(false),
        }
        if self.shown_chapter.get() != chapter_idx {
            self.shown_chapter.set(chapter_idx);
            if let Some(row) = self.chapters.row_at_index(chapter_idx as i32) {
                self.chapters.select_row(Some(&row));
            }
        }
        self.updating.set(false);
    }

    fn tick(&self) {
        (self.fit)();
        {
            let mut l = self.l.borrow_mut();
            l.tick(Instant::now(), self.shared.now());
            if l.take_slept() {
                (self.toast)(tr("Sleep timer: playback stopped"));
            }
        }
        self.refresh();
        self.mpris_sync();
    }

    fn mpris_sync(&self) {
        let m = self.mpris.borrow();
        let Some(m) = m.as_ref() else { return };
        let (status, title, chapter) = {
            let l = self.l.borrow();
            let st = l.player.status();
            let status = match st.state {
                bookx_audio::player::State::Playing => "Playing",
                bookx_audio::player::State::Paused => "Paused",
                bookx_audio::player::State::Ended => "Stopped",
            };
            let ch = l
                .book
                .chapter_at(st.position_ms)
                .map(|c| c.title.clone())
                .unwrap_or_default();
            (status, l.book.title.clone(), ch)
        };
        m.update(crate::mpris::Snapshot {
            status,
            title,
            chapter,
            can_next: true,
            can_previous: true,
        });
        for c in m.controls() {
            use crate::mpris::Control;
            let mut l = self.l.borrow_mut();
            match c {
                Control::Play => l.player.play(),
                Control::Pause | Control::Stop => l.player.pause(),
                Control::PlayPause => l.play_pause(),
                Control::Next => l.next_chapter(),
                Control::Previous => l.previous_chapter(),
            }
        }
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        self.close();
    }
}

impl Page {
    /// Stops playing and saves the position, bookmarks and the listening session.
    pub fn close(&self) {
        if self.inner.closed.replace(true) {
            return;
        }
        self.inner.l.borrow_mut().close(self.inner.shared.now());
        *self.inner.mpris.borrow_mut() = None;
    }

    /// `audio:` dev commands. Returns the lines to print.
    pub fn dev(&self, cmd: &str) -> String {
        let i = &self.inner;
        let (name, arg) = cmd.split_once('=').unwrap_or((cmd, ""));
        match name {
            "play" => i.l.borrow().player.play(),
            "pause" => i.l.borrow().player.pause(),
            "toggle" => i.toggle(),
            "skip" => i.l.borrow_mut().skip(arg.parse().unwrap_or(0) * 1000),
            "seek" => i.l.borrow_mut().seek(arg.parse().unwrap_or(0)),
            "chapter" => i.l.borrow_mut().goto_chapter(arg.parse().unwrap_or(0)),
            "next" => i.l.borrow_mut().next_chapter(),
            "prev" => i.l.borrow_mut().previous_chapter(),
            "speed" => i.set_speed(arg.parse().unwrap_or(1.0)),
            "volume" => i.volume.set_value(arg.parse().unwrap_or(1.0)),
            "sleep" => i.set_sleep(match arg {
                "chapter" => Sleep::EndOfChapter,
                "off" | "" => Sleep::Off,
                m => Sleep::Minutes(m.parse().unwrap_or(0)),
            }),
            // a sleep timer of this many seconds (the minutes of the menu are too long for a test)
            "sleep-secs" => {
                let s: u64 = arg.parse().unwrap_or(1);
                let mut l = i.l.borrow_mut();
                l.set_sleep(Sleep::Off, Instant::now());
                l.sleep = SleepTimer::At(Instant::now() + Duration::from_secs(s));
            }
            "bookmark" => {
                i.note.set_text(arg);
                i.add_bookmark();
            }
            "rmbookmark" => {
                let id =
                    i.l.borrow()
                        .state
                        .bookmarks
                        .get(arg.parse::<usize>().unwrap_or(0))
                        .map(|b| b.id.clone());
                if let Some(id) = id {
                    i.delete_bookmark(&id);
                }
            }
            // `audio:undo`: presses Undo of the last "Bookmark deleted" toast
            "undo" => {
                let f = LAST_DELETE.with(|l| l.borrow_mut().take());
                if let Some(f) = f {
                    f();
                }
            }
            "gobookmark" => {
                let id =
                    i.l.borrow()
                        .state
                        .bookmarks
                        .get(arg.parse::<usize>().unwrap_or(0))
                        .map(|b| b.id.clone());
                if let Some(id) = id {
                    i.l.borrow_mut().go_to_bookmark(&id);
                }
            }
            "tick" => i.tick(),
            _ => {}
        }
        i.refresh();
        if name == "dump" {
            i.dump()
        } else {
            String::new()
        }
    }
}

impl Inner {
    fn dump(&self) -> String {
        let l = self.l.borrow();
        let st = l.player.status();
        let idx = l.book.chapter_index_at(st.position_ms);
        let sleep = match l.sleep {
            SleepTimer::Off => "off".to_string(),
            SleepTimer::At(_) => "timer".to_string(),
            SleepTimer::EndOfChapter => "chapter".to_string(),
        };
        format!(
            "audio: state={:?} pos={} chapter={}/{} speed={} sleep={} bookmarks={} total={} mpris={} play_icon={} progress=\"{}\" error={}",
            st.state,
            st.position_ms,
            idx + 1,
            l.book.chapters.len(),
            l.state.speed,
            sleep,
            l.state.bookmarks.len(),
            l.book.total_ms,
            self.mpris
                .borrow()
                .as_ref()
                .map(|m| m.name.clone())
                .unwrap_or_default(),
            self.play.icon_name().unwrap_or_default(),
            self.progress.label(),
            st.error.unwrap_or_default(),
        )
    }
}

/// The "Audiobook" group of the details panel: length, narrator, progress and a Play button.
/// `None` when the book has no audiobook.
pub fn details_group(
    shared: &SharedRef,
    b: &Book,
    play: Rc<dyn Fn(u64)>,
    read: Rc<dyn Fn(u64)>,
    toast: Rc<dyn Fn(String)>,
) -> Option<adw::PreferencesGroup> {
    if !b.resources.iter().any(|r| r.kind == AUDIOBOOK) {
        return None;
    }
    let data = ab::data_dir()?;
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Listen"));
    g.set_margin_top(10);
    let loaded = {
        let lib = shared.lib.borrow();
        ab::load(&lib.root, b, &data)
    };
    let row = |title: &str, value: &str| {
        let r = adw::ActionRow::builder()
            .title(title)
            .subtitle(gtk::glib::markup_escape_text(value))
            .subtitle_selectable(true)
            .build();
        r.add_css_class("property");
        r
    };
    let id = b.id;
    match loaded {
        Ok(l) => {
            if !l.book.narrator.is_empty() {
                g.add(&row(&tr("Narrator"), &l.book.narrator));
            }
            g.add(&row(
                &tr("Length"),
                &trf(
                    "{} · {} chapters",
                    &[&ab::length_text(l.book.total_ms), &l.book.chapters.len()],
                ),
            ));
            if b.progress > 0.0 {
                let st = bookx_audio::state::State::load(
                    &data.join(format!("{}.state.json", ab::key_of(&l.source))),
                );
                let pos = if ab::is_own(b) {
                    b.position
                        .map(|p| l.book.from_readpos(p.spine, p.frac))
                        .unwrap_or(st.position_ms)
                } else {
                    st.position_ms
                };
                g.add(&row(
                    &tr("Progress"),
                    &ab::progress_text(&l.book, pos, st.speed),
                ));
            }
            let play_row = adw::ActionRow::builder()
                .title(tr("Play in Bookx"))
                .activatable(true)
                .build();
            let btn = gtk::Button::from_icon_name("media-playback-start-symbolic");
            btn.set_valign(gtk::Align::Center);
            btn.add_css_class("flat");
            btn.add_css_class("circular");
            audit::label(&btn, &tr("Play in Bookx"));
            btn.set_tooltip_text(Some(&tr("Play in Bookx")));
            play_row.add_suffix(&btn);
            let (p1, p2) = (play.clone(), play.clone());
            btn.connect_clicked(move |_| p1(id));
            play_row.connect_activated(move |_| p2(id));
            g.add(&play_row);
            // an e-book with audio linked: one place for both
            if !ab::is_own(b) {
                let mk = |title: String, subtitle: String, name: &str| {
                    let r = adw::ActionRow::builder()
                        .title(title)
                        .subtitle(subtitle)
                        .activatable(true)
                        .build();
                    r.set_widget_name(name);
                    r
                };
                let listen = mk(
                    tr("Continue in Audiobook"),
                    tr("Start listening where you stopped reading"),
                    "audio-continue-listening",
                );
                listen.set_sensitive(b.position.is_some());
                let (sh, p, t) = (shared.clone(), play.clone(), toast.clone());
                listen.connect_activated(move |_| {
                    let Some(data) = ab::data_dir() else { return };
                    let r = ab::continue_in_audiobook(&sh.lib.borrow(), id, &data);
                    match r {
                        Ok(_) => p(id),
                        Err(e) => t(trf(
                            "Could not continue in the audiobook: {}",
                            &[&format!("{e:#}")],
                        )),
                    }
                });
                g.add(&listen);
                let reading = mk(
                    tr("Continue Reading"),
                    tr("Open the e-book where you stopped listening"),
                    "audio-continue-reading",
                );
                let (sh, r2, t) = (shared.clone(), read.clone(), toast.clone());
                reading.connect_activated(move |_| {
                    let Some(data) = ab::data_dir() else { return };
                    let res = ab::continue_reading(&mut sh.lib.borrow_mut(), id, &data);
                    match res {
                        Ok(_) => {
                            let _ = sh.lib.borrow().save();
                            r2(id)
                        }
                        Err(e) => t(trf("Could not continue reading: {}", &[&format!("{e:#}")])),
                    }
                });
                g.add(&reading);
            }
        }
        Err(e) => {
            g.add(&row(&tr("Audio"), &format!("{e:#}")));
        }
    }
    Some(g)
}

/// The small headphones badge on the cover of a book that has audio.
pub fn cover_badge() -> gtk::Image {
    let i = gtk::Image::from_icon_name("audio-headphones-symbolic");
    i.add_css_class("audio-badge");
    i.set_halign(gtk::Align::Start);
    i.set_valign(gtk::Align::End);
    i.set_margin_start(6);
    i.set_margin_bottom(6);
    i.set_pixel_size(14);
    i.set_visible(false);
    i.set_tooltip_text(Some(&tr("Audiobook")));
    audit::label(&i, &tr("Audiobook"));
    i
}

/// The *Audiobooks* entry of the sidebar (shows the books that have audio); `None` without any.
pub fn sidebar_row(
    lib: &crate::core::library::Library,
    go: Rc<dyn Fn(String)>,
) -> Option<adw::ActionRow> {
    let n = lib.books.iter().filter(|b| ab::is_audiobook(b)).count();
    if n == 0 {
        return None;
    }
    let row = adw::ActionRow::builder()
        .title(tr("Audiobooks"))
        .activatable(true)
        .build();
    row.add_prefix(&gtk::Image::from_icon_name("audio-headphones-symbolic"));
    let count = gtk::Label::new(Some(&n.to_string()));
    count.add_css_class("dim-label");
    count.add_css_class("numeric");
    row.add_suffix(&count);
    row.set_widget_name("audiobooks-row");
    row.connect_activated(move |_| go("audiobook:yes".to_string()));
    Some(row)
}

/// *Add Audiobook*: a folder of parts, or one audio file or zip.
pub fn pick(parent: Option<gtk::Window>, done: Rc<dyn Fn(Vec<std::path::PathBuf>)>) {
    let Some(parent) = parent else { return };
    let dlg = adw::AlertDialog::new(
        Some(&tr("Add Audiobook")),
        Some(&tr(
            "Choose the folder of an audiobook, or one audio file (MP3, M4B, OGG, FLAC, WAV) or zip. The audio stays where it is.",
        )),
    );
    dlg.add_response("cancel", &tr("Cancel"));
    dlg.add_response("file", &tr("Choose File"));
    dlg.add_response("folder", &tr("Choose Folder"));
    dlg.set_response_appearance("folder", adw::ResponseAppearance::Suggested);
    dlg.set_default_response(Some("folder"));
    dlg.set_close_response("cancel");
    let p2 = parent.clone();
    dlg.choose(Some(&parent), gtk::gio::Cancellable::NONE, move |r| {
        let done = done.clone();
        match r.as_str() {
            "folder" => {
                let d = gtk::FileDialog::builder()
                    .title(tr("Audiobook Folder"))
                    .build();
                d.select_folder(Some(&p2), gtk::gio::Cancellable::NONE, move |res| {
                    if let Some(p) = res.ok().and_then(|f| f.path()) {
                        done(vec![p]);
                    }
                });
            }
            "file" => {
                let d = gtk::FileDialog::builder()
                    .title(tr("Audiobook File"))
                    .build();
                let filter = gtk::FileFilter::new();
                filter.set_name(Some(&tr("Audio files and zips")));
                for e in bookx_audio::format::EXTENSIONS {
                    filter.add_suffix(e);
                }
                filter.add_suffix("zip");
                let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
                filters.append(&filter);
                d.set_filters(Some(&filters));
                d.open(Some(&p2), gtk::gio::Cancellable::NONE, move |res| {
                    if let Some(p) = res.ok().and_then(|f| f.path()) {
                        done(vec![p]);
                    }
                });
            }
            _ => {}
        }
    });
}

/// The *Listening* group of the statistics window; `None` before the first listening session.
pub fn listening_group(shared: &SharedRef) -> Option<adw::PreferencesGroup> {
    let data = ab::data_dir()?;
    let sum = ab::listening_summary(&data, shared.now())?;
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Listening"));
    let row = |title: String, value: String| {
        let r = adw::ActionRow::new();
        r.set_title(&title);
        let l = gtk::Label::new(Some(&value));
        l.add_css_class("dim-label");
        r.add_suffix(&l);
        r
    };
    g.add(&row(
        tr("Today"),
        crate::reading_stats::duration_text(sum.today_secs),
    ));
    g.add(&row(
        tr("Last 7 days"),
        crate::reading_stats::duration_text(sum.week_secs),
    ));
    g.add(&row(
        tr("Total"),
        crate::reading_stats::duration_text(sum.total_secs),
    ));
    g.add(&row(
        tr("Audiobooks finished this year"),
        sum.books_finished_this_year.to_string(),
    ));
    let lib = shared.lib.borrow();
    for (id, secs) in &sum.top_books {
        let title = lib
            .get(*id)
            .map(|b| b.meta.title.clone())
            .unwrap_or_else(|| tr("(removed book)"));
        g.add(&row(
            glib::markup_escape_text(&title).to_string(),
            crate::reading_stats::duration_text(*secs),
        ));
    }
    Some(g)
}

/// `audio:stats` of the dev script: the numbers of the Listening group on one line.
pub fn dev_stats(shared: &SharedRef) -> String {
    let Some(data) = ab::data_dir() else {
        return "audio_stats: no data folder".into();
    };
    match ab::listening_summary(&data, shared.now()) {
        None => "audio_stats: none".into(),
        Some(s) => format!(
            "audio_stats: today={} total={} finished_year={} top={:?}",
            s.today_secs, s.total_secs, s.books_finished_this_year, s.top_books
        ),
    }
}

/// `library:audiobadges` of the dev script: how many cover badges show.
pub fn audio_badges(w: &gtk::Widget, n: &mut usize) {
    if w.has_css_class("audio-badge") && w.is_visible() {
        *n += 1;
    }
    let mut c = w.first_child();
    while let Some(ch) = c {
        audio_badges(&ch, n);
        c = ch.next_sibling();
    }
}

/// Is there a widget with this name below `w`?
pub fn has_named(w: &gtk::Widget, name: &str) -> bool {
    if w.widget_name() == name {
        return true;
    }
    let mut c = w.first_child();
    while let Some(ch) = c {
        if has_named(&ch, name) {
            return true;
        }
        c = ch.next_sibling();
    }
    false
}

/// A new tag for every page: a closed page keeps its tag until its transition has ended, and
/// opening a book at once again must not meet it.
fn unique_tag() -> String {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    format!("audiobook-{}", N.fetch_add(1, Ordering::Relaxed))
}
