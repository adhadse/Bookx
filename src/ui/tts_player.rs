//! The read-aloud player card (enhancement 11): a compact floating player over the bottom of the
//! reader, shown while read aloud is on. It is an overlay, not a popover, so clicking the page does
//! not dismiss it. The state model is [`crate::tts_player`]; the commands are those of
//! [`super::reader_tts::Cmd`].

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use relm4::adw;
use relm4::gtk::{self, gdk, glib, prelude::*};

use super::reader_tts::{sleep_of, Cmd, SLEEP};
use crate::i18n::{tr, trf};
use crate::tts::Voice;
use crate::tts_player::{speed_label, step_speed, view, Model, Phase};

/// How long the seek bar waits for the pointer to rest before the reading jumps.
const SEEK_SETTLE_MS: u64 = 250;

pub struct PlayerCard {
    /// The widget to put in a `gtk::Overlay`.
    pub widget: adw::Clamp,
    card: gtk::Box,
    title: gtk::Label,
    chapter: gtk::Label,
    /// The sentence on one line, shown instead of the chapter in the small player.
    mini_sentence: gtk::Label,
    cover: gtk::Picture,
    sentence: gtk::Label,
    seek: gtk::Scale,
    position: gtk::Label,
    prev_chapter: gtk::Button,
    prev: gtk::Button,
    play: gtk::Button,
    next: gtk::Button,
    next_chapter: gtk::Button,
    stop: gtk::Button,
    collapse: gtk::Button,
    details: gtk::Box,
    slower: gtk::Button,
    faster: gtk::Button,
    speed: gtk::Label,
    voice_label: gtk::Label,
    voice_pop: gtk::Popover,
    voice_list: gtk::ListBox,
    voice_names: Rc<RefCell<Vec<String>>>,
    sleep_btn: gtk::MenuButton,
    volume: gtk::Scale,
    status: gtk::Box,
    status_text: gtk::Label,
    status_bar: gtk::ProgressBar,
    status_cancel: gtk::Button,
    status_retry: gtk::Button,
    status_copy: gtk::Button,
    command: Rc<RefCell<String>>,
    controls: gtk::Box,
    model: RefCell<Model>,
    collapsed: Cell<bool>,
    quiet: Rc<Cell<bool>>,
    voice_pop_open: Rc<Cell<bool>>,
    hover: Rc<Cell<bool>>,
    count: Rc<Cell<usize>>,
    /// Where the card is, as fractions of the room in the window (see `card_drag`).
    pub pos: super::card_drag::Pos,
}

fn icon_button(icon: &str, name: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.set_tooltip_text(Some(name));
    super::audit::label(&b, name);
    b
}

impl PlayerCard {
    pub fn new(on_cmd: Rc<dyn Fn(Cmd)>) -> Rc<PlayerCard> {
        let quiet = Rc::new(Cell::new(false));
        let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
        card.add_css_class("tts-player");
        card.add_css_class("card");
        card.set_accessible_role(gtk::AccessibleRole::Group);
        super::audit::label(&card, &tr("Read aloud player"));

        // title row
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let cover = gtk::Picture::new();
        cover.set_size_request(32, 44);
        cover.set_content_fit(gtk::ContentFit::Cover);
        cover.set_can_shrink(false);
        cover.set_halign(gtk::Align::Start);
        cover.set_valign(gtk::Align::Center);
        cover.add_css_class("tts-cover");
        cover.set_accessible_role(gtk::AccessibleRole::Presentation);
        head.append(&cover);
        let names = gtk::Box::new(gtk::Orientation::Vertical, 2);
        names.set_hexpand(true);
        names.set_valign(gtk::Align::Center);
        let title = gtk::Label::new(None);
        title.set_xalign(0.0);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.add_css_class("heading");
        let chapter = gtk::Label::new(None);
        chapter.set_xalign(0.0);
        chapter.set_ellipsize(gtk::pango::EllipsizeMode::End);
        chapter.add_css_class("dim-label");
        chapter.add_css_class("caption");
        let mini_sentence = gtk::Label::new(None);
        mini_sentence.set_xalign(0.0);
        mini_sentence.set_ellipsize(gtk::pango::EllipsizeMode::End);
        mini_sentence.set_max_width_chars(1);
        mini_sentence.add_css_class("caption");
        mini_sentence.set_accessible_role(gtk::AccessibleRole::Presentation);
        names.append(&title);
        names.append(&chapter);
        names.append(&mini_sentence);
        head.append(&names);
        let collapse = icon_button("pan-down-symbolic", &tr("Show the small player"));
        collapse.add_css_class("flat");
        collapse.add_css_class("circular");
        collapse.set_valign(gtk::Align::Center);
        let stop = icon_button("window-close-symbolic", &tr("Close read aloud"));
        stop.add_css_class("flat");
        stop.add_css_class("circular");
        stop.set_valign(gtk::Align::Center);
        head.append(&collapse);
        head.append(&stop);
        card.append(&head);

        // what is being read, the seek bar
        let details = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let sentence = gtk::Label::new(None);
        sentence.set_xalign(0.0);
        sentence.set_wrap(true);
        sentence.set_lines(2);
        sentence.set_ellipsize(gtk::pango::EllipsizeMode::End);
        sentence.set_max_width_chars(1);
        sentence.set_hexpand(true);
        sentence.add_css_class("tts-sentence");
        super::audit::label(&sentence, &tr("Sentence being read"));
        details.append(&sentence);
        let seek_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let seek = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.01);
        seek.set_draw_value(false);
        seek.set_hexpand(true);
        seek.set_sensitive(false);
        super::audit::label(&seek, &tr("Position in the chapter"));
        let position = gtk::Label::new(None);
        position.add_css_class("caption");
        position.add_css_class("dim-label");
        position.add_css_class("numeric");
        seek_row.append(&seek);
        seek_row.append(&position);
        details.append(&seek_row);
        card.append(&details);

        // transport
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        controls.set_halign(gtk::Align::Center);
        let prev_chapter = icon_button("media-skip-backward-symbolic", &tr("Previous chapter"));
        let prev = icon_button("media-seek-backward-symbolic", &tr("Previous sentence"));
        let play = icon_button("media-playback-start-symbolic", &tr("Read aloud from here"));
        play.add_css_class("suggested-action");
        play.add_css_class("circular");
        play.add_css_class("tts-play");
        play.set_size_request(40, 40);
        let next = icon_button("media-seek-forward-symbolic", &tr("Next sentence"));
        let next_chapter = icon_button("media-skip-forward-symbolic", &tr("Next chapter"));
        for b in [&prev_chapter, &prev, &next, &next_chapter] {
            b.add_css_class("flat");
            b.add_css_class("circular");
        }
        play.set_valign(gtk::Align::Center);
        for b in [&prev_chapter, &prev] {
            controls.append(b);
        }
        controls.append(&play);
        for b in [&next, &next_chapter] {
            controls.append(b);
        }
        card.append(&controls);

        // speed, voice, sleep timer, volume
        let extras = adw::WrapBox::new();
        extras.set_child_spacing(8);
        extras.set_line_spacing(6);
        extras.set_natural_line_length(480);
        extras.set_align(0.5);
        let speed_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        speed_box.add_css_class("linked");
        let slower = icon_button("list-remove-symbolic", &tr("Slower"));
        let faster = icon_button("list-add-symbolic", &tr("Faster"));
        let speed = gtk::Label::new(Some("1×"));
        speed.add_css_class("tts-speed");
        speed.set_width_chars(5);
        super::audit::label(&speed, &tr("Reading speed"));
        speed_box.append(&slower);
        speed_box.append(&speed);
        speed_box.append(&faster);
        extras.append(&speed_box);

        let voice_btn = gtk::MenuButton::new();
        let voice_label = gtk::Label::new(Some(&tr("Default voice")));
        voice_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        voice_label.set_max_width_chars(16);
        let vb = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        vb.append(&gtk::Image::from_icon_name("audio-speakers-symbolic"));
        vb.append(&voice_label);
        voice_btn.set_child(Some(&vb));
        voice_btn.set_tooltip_text(Some(&tr("Choose the voice")));
        super::audit::label(&voice_btn, &tr("Choose the voice"));
        let voice_pop = gtk::Popover::new();
        let pop_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        pop_box.set_margin_top(6);
        pop_box.set_margin_bottom(6);
        pop_box.set_margin_start(6);
        pop_box.set_margin_end(6);
        let voice_list = gtk::ListBox::new();
        voice_list.add_css_class("navigation-sidebar");
        voice_list.set_selection_mode(gtk::SelectionMode::Single);
        super::audit::label(&voice_list, &tr("Voices"));
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_child(Some(&voice_list));
        scroll.set_min_content_width(240);
        scroll.set_max_content_height(240);
        scroll.set_propagate_natural_height(true);
        pop_box.append(&scroll);
        let settings = gtk::Button::with_label(&tr("Voice settings"));
        settings.add_css_class("flat");
        pop_box.append(&settings);
        voice_pop.set_child(Some(&pop_box));
        voice_btn.set_popover(Some(&voice_pop));
        extras.append(&voice_btn);

        let sleep_btn = gtk::MenuButton::new();
        sleep_btn.set_icon_name("preferences-system-time-symbolic");
        sleep_btn.set_tooltip_text(Some(&tr("Sleep timer")));
        super::audit::label(&sleep_btn, &tr("Sleep timer"));
        let sleep_pop = gtk::Popover::new();
        let sleep_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
        sleep_box.set_margin_top(6);
        sleep_box.set_margin_bottom(6);
        sleep_box.set_margin_start(6);
        sleep_box.set_margin_end(6);
        for (i, entry) in SLEEP.iter().enumerate() {
            let b = gtk::Button::with_label(&tr(entry.0));
            b.add_css_class("flat");
            b.add_css_class("menu-item");
            if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                l.set_xalign(0.0);
            }
            let (f, p, sb) = (on_cmd.clone(), sleep_pop.clone(), sleep_btn.clone());
            b.connect_clicked(move |_| {
                p.popdown();
                sb.set_tooltip_text(Some(&if i == 0 {
                    tr("Sleep timer")
                } else {
                    trf("Sleep timer: {}", &[&tr(SLEEP[i].0)])
                }));
                f(Cmd::Sleep(sleep_of(i)));
            });
            sleep_box.append(&b);
        }
        sleep_pop.set_child(Some(&sleep_box));
        sleep_btn.set_popover(Some(&sleep_pop));
        extras.append(&sleep_btn);

        let vol_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        vol_box.append(&gtk::Image::from_icon_name("audio-volume-high-symbolic"));
        let volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.05);
        volume.set_draw_value(false);
        volume.set_value(1.0);
        volume.set_size_request(96, -1);
        super::audit::label(&volume, &tr("Volume"));
        vol_box.append(&volume);
        extras.append(&vol_box);
        details.append(&extras);

        // download / problem line
        let status = gtk::Box::new(gtk::Orientation::Vertical, 6);
        status.set_visible(false);
        let status_text = gtk::Label::new(None);
        status_text.set_wrap(true);
        status_text.set_xalign(0.0);
        status_text.set_max_width_chars(1);
        status_text.set_hexpand(true);
        status_text.set_selectable(false);
        status.append(&status_text);
        let status_bar = gtk::ProgressBar::new();
        super::audit::label(&status_bar, &tr("Voice download progress"));
        status.append(&status_bar);
        let status_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        status_row.set_halign(gtk::Align::End);
        let status_cancel = gtk::Button::with_label(&tr("Cancel"));
        let status_retry = gtk::Button::with_label(&tr("Try again"));
        status_retry.add_css_class("suggested-action");
        let status_copy = gtk::Button::with_label(&tr("Copy install command"));
        status_row.append(&status_copy);
        status_row.append(&status_retry);
        status_row.append(&status_cancel);
        status.append(&status_row);
        card.append(&status);

        let clamp = adw::Clamp::new();
        clamp.set_maximum_size(560);
        clamp.set_tightening_threshold(420);
        clamp.set_child(Some(&card));
        clamp.set_halign(gtk::Align::Center);
        clamp.set_valign(gtk::Align::End);
        clamp.set_visible(false);

        let command = Rc::new(RefCell::new(String::new()));
        let voice_names = Rc::new(RefCell::new(vec![String::new()]));
        let voice_pop_open = Rc::new(Cell::new(false));
        let hover = Rc::new(Cell::new(false));
        let count = Rc::new(Cell::new(0usize));
        {
            let m = gtk::EventControllerMotion::new();
            let h = hover.clone();
            m.connect_enter(move |_, _, _| h.set(true));
            let h = hover.clone();
            m.connect_leave(move |_| h.set(false));
            card.add_controller(m);
        }

        // wiring
        {
            let f = on_cmd.clone();
            play.connect_clicked(move |_| f(Cmd::PlayPause));
            let f = on_cmd.clone();
            stop.connect_clicked(move |_| f(Cmd::Stop));
            let f = on_cmd.clone();
            prev.connect_clicked(move |_| f(Cmd::Skip(-1)));
            let f = on_cmd.clone();
            next.connect_clicked(move |_| f(Cmd::Skip(1)));
            let f = on_cmd.clone();
            prev_chapter.connect_clicked(move |_| f(Cmd::Chapter(-1)));
            let f = on_cmd.clone();
            next_chapter.connect_clicked(move |_| f(Cmd::Chapter(1)));
            let f = on_cmd.clone();
            status_cancel.connect_clicked(move |_| f(Cmd::CancelPrep));
            let f = on_cmd.clone();
            status_retry.connect_clicked(move |_| f(Cmd::PlayPause));
            let cmdline = command.clone();
            status_copy.connect_clicked(move |b| {
                b.display().clipboard().set_text(&cmdline.borrow());
            });
            let (f, p) = (on_cmd.clone(), voice_pop.clone());
            settings.connect_clicked(move |_| {
                p.popdown();
                f(Cmd::Settings)
            });
            let f = on_cmd.clone();
            voice_pop.connect_show(move |_| f(Cmd::Opened));
            let (f, q, n, p) = (
                on_cmd.clone(),
                quiet.clone(),
                voice_names.clone(),
                voice_pop.clone(),
            );
            voice_list.connect_row_activated(move |_, row| {
                if q.get() {
                    return;
                }
                if let Some(name) = n.borrow().get(row.index() as usize) {
                    f(Cmd::Voice(name.clone()));
                }
                p.popdown();
            });
            let (f, q) = (on_cmd.clone(), quiet.clone());
            volume.connect_value_changed(move |s| {
                if !q.get() {
                    f(Cmd::Volume((s.value() * 100.0).round() / 100.0));
                }
            });
            // the speed buttons step through the offered speeds from what is shown
            let (f, l) = (on_cmd.clone(), speed.clone());
            slower.connect_clicked(move |_| f(Cmd::Speed(step_speed(parse_speed(&l), -1))));
            let (f, l) = (on_cmd.clone(), speed.clone());
            faster.connect_clicked(move |_| f(Cmd::Speed(step_speed(parse_speed(&l), 1))));
            // the seek bar: user moves only, and the jump waits until the pointer rests
            let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
            let f = on_cmd.clone();
            let mc = count.clone();
            seek.connect_change_value(move |s, _, v| {
                if let Some(id) = pending.borrow_mut().take() {
                    id.remove();
                }
                let count = mc.get();
                let idx = crate::tts_player::seek_index(v, count);
                let f = f.clone();
                let slot = pending.clone();
                let id = glib::timeout_add_local_once(
                    std::time::Duration::from_millis(SEEK_SETTLE_MS),
                    move || {
                        slot.borrow_mut().take();
                        f(Cmd::Seek(idx));
                    },
                );
                *pending.borrow_mut() = Some(id);
                s.set_value(v.clamp(0.0, 1.0));
                glib::Propagation::Stop
            });
        }

        let this = PlayerCard {
            widget: clamp,
            card,
            title,
            chapter,
            mini_sentence,
            cover,
            sentence,
            seek,
            position,
            prev_chapter,
            prev,
            play,
            next,
            next_chapter,
            stop,
            collapse: collapse.clone(),
            details,
            slower,
            faster,
            speed,
            voice_label,
            voice_pop,
            voice_list,
            voice_names,
            sleep_btn,
            volume,
            status,
            status_text,
            status_bar,
            status_cancel,
            status_retry,
            status_copy,
            command,
            controls,
            model: RefCell::new(Model::default()),
            collapsed: Cell::new(true),
            quiet,
            voice_pop_open,
            hover,
            count,
            pos: Rc::new(Cell::new(crate::reader_cards::TTS_PLACE)),
        };
        let (a, b) = (this.voice_pop_open.clone(), this.voice_pop_open.clone());
        this.voice_pop.connect_show(move |_| a.set(true));
        this.voice_pop.connect_closed(move |_| b.set(false));
        let this = Rc::new(this);
        // any part of the card that is not a control drags it; a double click puts it back
        super::card_drag::make_draggable(
            this.card.upcast_ref(),
            this.widget.upcast_ref(),
            this.pos.clone(),
            crate::reader_cards::TTS_PLACE,
            0,
            {
                let f = on_cmd.clone();
                Rc::new(move || f(Cmd::Layout))
            },
        );
        // Escape closes the card when the keyboard focus is in it
        {
            let k = gtk::EventControllerKey::new();
            let f = on_cmd.clone();
            k.connect_key_pressed(move |_, key, _, _| {
                if key == gtk::gdk::Key::Escape {
                    f(Cmd::Stop);
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            });
            this.card.add_controller(k);
        }
        // the small player is the default; the full one is opt-in
        this.set_collapsed(true);
        let me = Rc::downgrade(&this);
        let f = on_cmd.clone();
        this.collapse.connect_clicked(move |_| {
            if let Some(c) = me.upgrade() {
                c.set_collapsed(!c.collapsed.get());
                f(Cmd::Layout);
            }
        });
        this
    }

    /// Puts the card at `frac` (fractions of the free room in the window).
    pub fn set_pos(&self, frac: (f64, f64)) {
        super::card_drag::set_pos(self.widget.upcast_ref(), &self.pos, frac);
    }

    /// The play button takes the keyboard focus (a restored card waits for it).
    pub fn focus_play(&self) -> bool {
        self.play.grab_focus()
    }

    pub fn collapsed(&self) -> bool {
        self.collapsed.get()
    }

    /// Switches between the full card and the small player (title, transport, close).
    pub fn set_collapsed(&self, collapsed: bool) {
        self.collapsed.set(collapsed);
        self.model.borrow_mut().collapsed = collapsed;
        self.card.set_css_classes(&if collapsed {
            vec!["tts-player", "card", "mini"]
        } else {
            vec!["tts-player", "card"]
        });
        let name = if collapsed {
            tr("Show the full player")
        } else {
            tr("Show the small player")
        };
        self.collapse.set_icon_name(if collapsed {
            "pan-up-symbolic"
        } else {
            "pan-down-symbolic"
        });
        self.collapse.set_tooltip_text(Some(&name));
        super::audit::label(&self.collapse, &name);
        self.relayout();
    }

    fn relayout(&self) {
        let m = self.model.borrow().clone();
        let v = view(&m);
        let working = matches!(m.phase, Phase::Playing | Phase::Paused | Phase::Idle);
        let message = !working;
        self.details.set_visible(!self.collapsed.get() && working);
        self.controls.set_visible(!message || v.play_enabled);
        self.prev_chapter.set_visible(!self.collapsed.get());
        self.next_chapter.set_visible(!self.collapsed.get());
        self.chapter.set_visible(!self.collapsed.get());
        self.mini_sentence
            .set_visible(self.collapsed.get() && working);
        self.widget
            .set_maximum_size(if self.collapsed.get() { 360 } else { 560 });
        self.status.set_visible(message);
    }

    /// Shows the card for `m`: phase, enabled buttons, position, speed.
    pub fn apply(&self, m: &Model) {
        let mut model = m.clone();
        model.collapsed = self.collapsed.get();
        let v = view(&model);
        self.quiet.set(true);
        self.widget.set_visible(v.visible);
        self.play.set_sensitive(v.play_enabled);
        let (icon, tip) = if v.shows_pause {
            ("media-playback-pause-symbolic", tr("Pause reading"))
        } else if matches!(model.phase, Phase::Paused | Phase::Idle) {
            ("media-playback-start-symbolic", tr("Resume reading"))
        } else {
            ("media-playback-start-symbolic", tr("Read aloud from here"))
        };
        self.play.set_icon_name(icon);
        self.play.set_tooltip_text(Some(&tip));
        super::audit::label(&self.play, &tip);
        self.prev.set_sensitive(v.prev_sentence);
        self.next.set_sensitive(v.next_sentence);
        self.prev_chapter.set_sensitive(v.prev_chapter);
        self.next_chapter.set_sensitive(v.next_chapter);
        self.seek.set_sensitive(v.seek_enabled);
        if !self.seek.has_focus() {
            self.seek.set_value(v.progress);
        }
        self.count.set(model.sentences);
        self.position.set_text(&v.progress_text);
        let sp = speed_label(model.speed);
        self.speed.set_text(&sp);
        self.slower
            .set_sensitive(step_speed(model.speed, -1) < model.speed - 0.005);
        self.faster
            .set_sensitive(step_speed(model.speed, 1) > model.speed + 0.005);
        match &model.phase {
            Phase::Preparing(p) => {
                self.status_text
                    .set_text(&trf("Preparing voice {}%", &[&p]));
                self.status_bar.set_visible(true);
                self.status_bar.set_fraction(*p as f64 / 100.0);
                self.status_cancel.set_visible(true);
                self.status_retry.set_visible(false);
                self.status_copy.set_visible(false);
            }
            Phase::Failed(msg) => {
                self.status_text.set_text(msg);
                self.status_bar.set_visible(false);
                self.status_cancel.set_visible(false);
                self.status_retry.set_visible(true);
                self.status_copy.set_visible(false);
            }
            Phase::NeedsEspeak => {
                let cmd = self.command.borrow().clone();
                self.status_text.set_text(&if cmd.is_empty() {
                    tr("Read aloud needs the speech tool espeak-ng. Install the package espeak-ng with your system's software manager, then press play.")
                } else {
                    trf("Read aloud needs the speech tool espeak-ng. Install it with:\n{}\nthen press play.", &[&cmd])
                });
                self.status_bar.set_visible(false);
                self.status_cancel.set_visible(false);
                self.status_retry.set_visible(true);
                self.status_copy.set_visible(!cmd.is_empty());
            }
            _ => {}
        }
        *self.model.borrow_mut() = model;
        self.relayout();
        self.quiet.set(false);
    }

    /// The command that installs espeak-ng here ("" when unknown).
    pub fn set_install_command(&self, cmd: &str) {
        *self.command.borrow_mut() = cmd.to_string();
    }

    pub fn set_sentence(&self, text: &str) {
        let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
        self.sentence.set_text(&one);
        self.mini_sentence.set_text(&one);
    }

    pub fn set_book(&self, title: &str, chapter: &str, cover: Option<&gdk::Texture>) {
        self.title.set_text(title);
        self.chapter.set_text(chapter);
        self.cover.set_paintable(cover);
        self.cover.set_visible(cover.is_some());
    }

    pub fn book_title(&self) -> String {
        self.title.text().to_string()
    }

    pub fn set_voice_name(&self, name: &str) {
        self.voice_label.set_text(&if name.is_empty() {
            tr("Default voice")
        } else {
            name.to_string()
        });
    }

    /// Shows the voices (the default first) and marks `current`.
    pub fn set_voices(&self, voices: &[Voice], current: &str) {
        self.quiet.set(true);
        while let Some(r) = self.voice_list.first_child() {
            self.voice_list.remove(&r);
        }
        let mut names = vec![String::new()];
        let add = |text: &str| {
            let l = gtk::Label::new(Some(text));
            l.set_xalign(0.0);
            l.set_margin_top(4);
            l.set_margin_bottom(4);
            self.voice_list.append(&l);
        };
        add(&tr("Default voice"));
        for v in voices {
            add(&if v.language.is_empty() {
                v.name.clone()
            } else {
                format!("{} ({})", v.name, v.language)
            });
            names.push(v.name.clone());
        }
        let at = names.iter().position(|n| n == current).unwrap_or(0);
        if let Some(r) = self.voice_list.row_at_index(at as i32) {
            self.voice_list.select_row(Some(&r));
        }
        *self.voice_names.borrow_mut() = names;
        self.set_voice_name(current);
        self.quiet.set(false);
    }

    pub fn set_volume(&self, v: f64) {
        self.quiet.set(true);
        self.volume.set_value(v);
        self.quiet.set(false);
    }

    pub fn set_sleep_off(&self) {
        self.sleep_btn.set_tooltip_text(Some(&tr("Sleep timer")));
    }

    /// The pointer is over the card, the keyboard focus is in it or its voice list is open: the
    /// reader's bars must not fade under the user's hand.
    pub fn in_use(&self) -> bool {
        if !self.widget.is_visible() {
            return false;
        }
        let hovered = self.hover.get();
        let focus = self.widget.root().and_then(|r| r.focus()).is_some_and(|f| {
            f.is_ancestor(&self.card) && f.state_flags().contains(gtk::StateFlags::FOCUS_VISIBLE)
        });
        hovered || focus || self.voice_pop_open.get()
    }

    // ---- test helpers ---------------------------------------------------------------------

    pub fn dev_click(&self, what: &str) {
        let b: Option<&gtk::Button> = match what {
            "play" => Some(&self.play),
            "stop" => Some(&self.stop),
            "next" => Some(&self.next),
            "prev" => Some(&self.prev),
            "next-chapter" => Some(&self.next_chapter),
            "prev-chapter" => Some(&self.prev_chapter),
            "faster" => Some(&self.faster),
            "slower" => Some(&self.slower),
            "collapse" => Some(&self.collapse),
            "cancel" => Some(&self.status_cancel),
            "retry" => Some(&self.status_retry),
            "copy" => Some(&self.status_copy),
            _ => None,
        };
        if let Some(b) = b {
            if b.is_sensitive() && b.is_visible() {
                b.emit_clicked();
            }
        }
    }

    pub fn dev_seek(&self, fraction: f64) {
        // the same path the pointer takes
        self.seek.emit_by_name::<bool>(
            "change-value",
            &[&gtk::ScrollType::Jump, &fraction.clamp(0.0, 1.0)],
        );
    }

    pub fn dev_dump(&self) -> String {
        let v = view(&self.model.borrow());
        let on = |b: &gtk::Button| b.is_sensitive() as u8;
        format!(
            "visible={} collapsed={} icon={} play={} prev={} next={} pch={} nch={} seek={} pos={:.2} text={:?} speed={} voice={:?} status={:?} bar={:.2} preparing={:?} sentence={:?} title={:?} chapter={:?} details={} pos=({:.2},{:.2}) size={}x{} cmd={:?}",
            self.widget.is_visible() as u8,
            self.collapsed.get() as u8,
            self.play.icon_name().unwrap_or_default(),
            on(&self.play),
            on(&self.prev),
            on(&self.next),
            on(&self.prev_chapter),
            on(&self.next_chapter),
            self.seek.is_sensitive() as u8,
            self.seek.value(),
            self.position.text(),
            self.speed.text(),
            self.voice_label.text(),
            if self.status.is_visible() { self.status_text.text().to_string() } else { String::new() },
            self.status_bar.fraction(),
            v.preparing,
            self.sentence.text(),
            self.title.text(),
            self.chapter.text(),
            self.dev_details_visible() as u8,
            self.pos.get().0,
            self.pos.get().1,
            self.card.width(),
            self.card.height(),
            self.command.borrow(),
        )
    }

    /// Test helper: is the details block (sentence, seek bar, extras) shown?
    pub fn dev_details_visible(&self) -> bool {
        self.details.is_visible()
    }
}

fn parse_speed(l: &gtk::Label) -> f64 {
    l.text().trim_end_matches('×').parse::<f64>().unwrap_or(1.0)
}
