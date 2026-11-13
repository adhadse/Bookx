//! Speed reading (R48, enhancement 23): a small card floating over the bottom of the reader (like the
//! read-aloud player card, movable by dragging its title row, with an X to stop) and the reading itself
//! on the page: the word being read has a broad highlight and its recognition letter a strong one, and
//! the page turns by itself to keep the word in view. The word timing and the focus letter come from
//! `crate::rsvp_view` / `bookx_lookup::rsvp`; the card geometry is shared with the read-aloud card
//! (`crate::tts_player::{rect_of, frac_of}`, `card_drag`).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use relm4::adw;
use relm4::gtk::{self, gdk, glib, prelude::*};

use super::canvas::BookCanvas;
use crate::i18n::{tr, trf};
use crate::rsvp_view::{Player, STEP_WPM};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cmd {
    PlayPause,
    /// Words forward (positive) or back.
    Step(i64),
    SentenceBack,
    SentenceForward,
    /// Change the speed by this many words per minute.
    Speed(i32),
    /// The X button: stop and hide the card.
    Close,
    /// The card was moved: remember where it is.
    Layout,
}

/// What the reader has to do after a command or tick.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The card was closed; keep this speed for next time.
    Closed(u32),
    /// The chapter's words are used up: go on with the next chapter.
    TextEnd,
    /// Play was pressed on a card that waits at a saved place: load that chapter and run from here.
    Unpark(Parked),
}

/// The command a key stands for while speed reading is on (the keys are taken from the page then).
pub fn key_cmd(key: gdk::Key) -> Option<Cmd> {
    use gdk::Key;
    Some(match key {
        Key::space => Cmd::PlayPause,
        Key::Left => Cmd::Step(-1),
        Key::Right => Cmd::Step(1),
        Key::Up => Cmd::Speed(STEP_WPM as i32),
        Key::Down => Cmd::Speed(-(STEP_WPM as i32)),
        Key::Escape => Cmd::Close,
        _ => return None,
    })
}

struct Ui {
    card: gtk::Box,
    progress: gtk::ProgressBar,
    wpm: gtk::Label,
    play: gtk::Button,
    word: gtk::Label,
}

/// Where the card waits when it is open but nothing runs: restored with the book, or left behind
/// when the user went to another chapter. Play continues from here.
#[derive(Debug, Clone, PartialEq)]
pub struct Parked {
    pub chapter: usize,
    /// Char offset of the word in the chapter text.
    pub offset: i32,
    /// Number of the word in the chapter and the number of words of the chapter.
    pub word: usize,
    pub words: usize,
    pub text: String,
    pub wpm: u32,
}

#[derive(Default)]
struct State {
    parked: Option<Parked>,
    /// The chapter the player's text belongs to.
    chapter: usize,
    player: Option<Player>,
    playing: bool,
    timer: Option<glib::SourceId>,
}

pub struct RsvpCtl {
    /// Put this in the reader's overlay.
    pub widget: adw::Clamp,
    ui: Ui,
    st: RefCell<State>,
    tick: Rc<dyn Fn()>,
    /// True while speed reading is on (the reader gives the keys to it).
    pub active: Rc<Cell<bool>>,
    hover: Rc<Cell<bool>>,
    /// Where the card is, as fractions of the room in the window (see `card_drag`).
    pub pos: super::card_drag::Pos,
}

fn icon_button(icon: &str, name: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.set_tooltip_text(Some(name));
    super::audit::label(&b, name);
    b
}

impl RsvpCtl {
    /// `on_cmd` receives the card's commands; `tick` is called when the next word is due.
    pub fn new(on_cmd: Rc<dyn Fn(Cmd)>, tick: Rc<dyn Fn()>) -> Rc<RsvpCtl> {
        let card = gtk::Box::new(gtk::Orientation::Vertical, 4);
        card.add_css_class("tts-player");
        card.add_css_class("rsvp-card");
        card.add_css_class("card");
        card.set_accessible_role(gtk::AccessibleRole::Group);
        super::audit::label(&card, &tr("Speed reading controls"));

        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let icon = gtk::Image::from_icon_name("view-reveal-symbolic");
        let title = gtk::Label::new(Some(&tr("Speed reading")));
        title.add_css_class("heading");
        title.set_xalign(0.0);
        title.set_hexpand(true);
        let word = gtk::Label::new(None);
        word.add_css_class("dim-label");
        word.set_ellipsize(gtk::pango::EllipsizeMode::End);
        word.set_max_width_chars(18);
        let close = icon_button("window-close-symbolic", &tr("Close speed reading"));
        close.add_css_class("flat");
        close.add_css_class("circular");
        head.append(&icon);
        head.append(&title);
        head.append(&word);
        head.append(&close);
        card.append(&head);

        let progress = gtk::ProgressBar::new();
        super::audit::label(&progress, &tr("Progress through the chapter"));
        card.append(&progress);

        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        controls.set_halign(gtk::Align::Center);
        let back = icon_button("media-skip-backward-symbolic", &tr("Previous sentence"));
        let play = icon_button("media-playback-start-symbolic", &tr("Start"));
        play.add_css_class("suggested-action");
        play.add_css_class("circular");
        play.add_css_class("tts-play");
        play.set_valign(gtk::Align::Center);
        let next = icon_button("media-skip-forward-symbolic", &tr("Next sentence"));
        let slower = icon_button("list-remove-symbolic", &tr("Slower"));
        let faster = icon_button("list-add-symbolic", &tr("Faster"));
        let wpm = gtk::Label::new(None);
        wpm.set_width_chars(9);
        wpm.add_css_class("tts-speed");
        super::audit::label(&wpm, &tr("Reading speed"));
        for b in [&back, &next] {
            b.add_css_class("flat");
            b.add_css_class("circular");
        }
        let speed = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        speed.add_css_class("linked");
        speed.set_margin_start(8);
        speed.append(&slower);
        speed.append(&wpm);
        speed.append(&faster);
        controls.append(&back);
        controls.append(&play);
        controls.append(&next);
        controls.append(&speed);
        card.append(&controls);

        let clamp = adw::Clamp::new();
        clamp.set_maximum_size(360);
        clamp.set_tightening_threshold(300);
        clamp.set_child(Some(&card));
        clamp.set_halign(gtk::Align::Center);
        clamp.set_valign(gtk::Align::End);
        clamp.set_visible(false);

        let hover = Rc::new(Cell::new(false));
        let m = gtk::EventControllerMotion::new();
        let h = hover.clone();
        m.connect_enter(move |_, _, _| h.set(true));
        let h = hover.clone();
        m.connect_leave(move |_| h.set(false));
        card.add_controller(m);

        for (b, c) in [
            (&play, Cmd::PlayPause),
            (&back, Cmd::SentenceBack),
            (&next, Cmd::SentenceForward),
            (&slower, Cmd::Speed(-(STEP_WPM as i32))),
            (&faster, Cmd::Speed(STEP_WPM as i32)),
            (&close, Cmd::Close),
        ] {
            let f = on_cmd.clone();
            b.connect_clicked(move |_| f(c));
        }

        let this = Rc::new(RsvpCtl {
            widget: clamp,
            ui: Ui {
                card,
                progress,
                wpm,
                play,
                word,
            },
            st: RefCell::new(State::default()),
            tick,
            active: Rc::new(Cell::new(false)),
            hover,
            pos: Rc::new(Cell::new(crate::reader_cards::RSVP_PLACE)),
        });

        // Escape closes the card when the keyboard focus is in it
        {
            let k = gtk::EventControllerKey::new();
            let f = on_cmd.clone();
            k.connect_key_pressed(move |_, key, _, _| {
                if key == gdk::Key::Escape {
                    f(Cmd::Close);
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            });
            this.ui.card.add_controller(k);
        }
        // any part of the card that is not a control drags it; a double click puts it back
        super::card_drag::make_draggable(
            this.ui.card.upcast_ref(),
            this.widget.upcast_ref(),
            this.pos.clone(),
            crate::reader_cards::RSVP_PLACE,
            1,
            {
                let f = on_cmd.clone();
                Rc::new(move || f(Cmd::Layout))
            },
        );
        this
    }

    /// Puts the card at `frac` (fractions of the free room in the window).
    pub fn set_pos(&self, frac: (f64, f64)) {
        super::card_drag::set_pos(self.widget.upcast_ref(), &self.pos, frac);
    }

    /// The play button takes the keyboard focus (a restored card waits for it).
    pub fn focus_play(&self) -> bool {
        self.ui.play.grab_focus()
    }

    /// The pointer is over the card or the keyboard focus is in it: the reader's bars must stay.
    pub fn in_use(&self) -> bool {
        if !self.active.get() {
            return false;
        }
        let focus = self.widget.root().and_then(|r| r.focus()).is_some_and(|f| {
            f.is_ancestor(&self.ui.card) && f.state_flags().contains(gtk::StateFlags::FOCUS_VISIBLE)
        });
        self.hover.get() || focus
    }

    /// Starts on `text` (the chapter) at char `from`, paused, with the first word marked.
    pub fn start(&self, canvas: &BookCanvas, text: &str, from: usize, wpm: u32, chapter: usize) {
        self.stop_timer();
        let player = Player::new(text, from, wpm);
        {
            let mut st = self.st.borrow_mut();
            st.player = Some(player);
            st.parked = None;
            st.chapter = chapter;
            st.playing = false;
        }
        self.active.set(true);
        self.widget.set_visible(true);
        self.show(canvas);
    }

    /// Goes on with another text (the next chapter) at its start, keeping the play state.
    pub fn continue_with(&self, canvas: &BookCanvas, text: &str, chapter: usize) {
        let wpm = self.wpm();
        let playing = self.st.borrow().playing;
        self.stop_timer();
        {
            let mut st = self.st.borrow_mut();
            st.player = Some(Player::new(text, 0, wpm));
            st.chapter = chapter;
            st.playing = playing;
        }
        self.show(canvas);
        if playing && !self.is_empty() {
            self.schedule();
        } else if playing {
            self.set_playing(false);
        }
    }

    fn is_empty(&self) -> bool {
        self.st
            .borrow()
            .player
            .as_ref()
            .is_none_or(|p| p.is_empty())
    }

    pub fn wpm(&self) -> u32 {
        let st = self.st.borrow();
        st.player.as_ref().map_or(
            st.parked
                .as_ref()
                .map_or(crate::rsvp_view::DEFAULT_WPM, |p| p.wpm),
            |p| p.wpm(),
        )
    }

    /// Opens the card waiting at `p` (nothing runs, nothing is marked on the page).
    pub fn park(&self, canvas: &BookCanvas, p: Parked) {
        self.stop_timer();
        {
            let mut st = self.st.borrow_mut();
            st.player = None;
            st.playing = false;
            st.chapter = p.chapter;
            st.parked = Some(p);
        }
        self.set_playing(false);
        self.active.set(true);
        self.widget.set_visible(true);
        canvas.set_focus_word(None);
        self.show_parked();
    }

    /// The running player becomes a waiting card (the chapter was left): the place is kept.
    pub fn park_current(&self, canvas: &BookCanvas) {
        let Some(p) = self.snapshot() else {
            return;
        };
        if self.st.borrow().player.is_some() {
            self.park(canvas, p);
        }
    }

    /// Where the card is: the running word, or the place it waits at.
    pub fn snapshot(&self) -> Option<Parked> {
        if !self.active.get() {
            return None;
        }
        let st = self.st.borrow();
        match (&st.player, &st.parked) {
            (Some(p), _) => Some(Parked {
                chapter: st.chapter,
                offset: p.word_range().map_or(0, |r| r.0),
                word: p.absolute_index(),
                words: p.absolute_len(),
                text: p.current().map(|w| w.text.clone()).unwrap_or_default(),
                wpm: p.wpm(),
            }),
            (None, Some(k)) => Some(k.clone()),
            _ => None,
        }
    }

    fn show_parked(&self) {
        let st = self.st.borrow();
        let Some(k) = st.parked.as_ref() else {
            return;
        };
        self.ui.word.set_text(&k.text);
        self.ui.progress.set_fraction(if k.words > 1 {
            k.word as f64 / (k.words - 1) as f64
        } else {
            0.0
        });
        self.ui.wpm.set_text(&trf("{} wpm", &[&k.wpm]));
        self.ui.progress.set_tooltip_text(Some(&trf(
            "Word {} of {}",
            &[&(k.word + 1), &k.words.max(1)],
        )));
    }

    fn is_parked(&self) -> bool {
        let st = self.st.borrow();
        st.player.is_none() && st.parked.is_some()
    }

    /// Speed reading is on (the card is shown).
    pub fn is_on(&self) -> bool {
        self.active.get()
    }

    pub fn playing(&self) -> bool {
        self.st.borrow().playing
    }

    fn stop_timer(&self) {
        if let Some(id) = self.st.borrow_mut().timer.take() {
            id.remove();
        }
    }

    fn set_playing(&self, on: bool) {
        self.st.borrow_mut().playing = on;
        let (icon, name) = if on {
            ("media-playback-pause-symbolic", tr("Pause"))
        } else {
            ("media-playback-start-symbolic", tr("Start"))
        };
        self.ui.play.set_icon_name(icon);
        self.ui.play.set_tooltip_text(Some(&name));
        super::audit::label(&self.ui.play, &name);
        if !on {
            self.stop_timer();
        }
    }

    fn schedule(&self) {
        self.stop_timer();
        let ms = self
            .st
            .borrow()
            .player
            .as_ref()
            .map_or(1000, |p| p.delay_ms()) as u64;
        let tick = self.tick.clone();
        let id = glib::timeout_add_local_once(std::time::Duration::from_millis(ms), move || tick());
        self.st.borrow_mut().timer = Some(id);
    }

    /// Marks the word on the page and updates the card.
    fn show(&self, canvas: &BookCanvas) {
        let st = self.st.borrow();
        let Some(p) = st.player.as_ref() else {
            return;
        };
        match (p.word_range(), p.focus_range()) {
            (Some(w), Some(f)) => canvas.set_focus_word(Some((w, f))),
            _ => canvas.set_focus_word(None),
        }
        self.ui.word.set_text(p.current().map_or("", |w| &w.text));
        self.ui.progress.set_fraction(p.fraction());
        self.ui.wpm.set_text(&trf("{} wpm", &[&p.wpm()]));
        let left = p.minutes_left();
        let left_text = if p.is_empty() {
            tr("Nothing to read here")
        } else if left < 1.0 {
            trf(
                "{} s left in the chapter",
                &[&((left * 60.0).round() as u32)],
            )
        } else {
            trf("{} min left in the chapter", &[&format!("{left:.0}")])
        };
        // the card is small: the time left is on the progress bar's tooltip
        self.ui.progress.set_tooltip_text(Some(&left_text));
    }

    /// The next word is due.
    pub fn on_tick(&self, canvas: &BookCanvas) -> Option<Event> {
        // the source that called us is spent: forget it (removing it again would be an error)
        self.st.borrow_mut().timer = None;
        if !self.playing() {
            return None;
        }
        let moved = self
            .st
            .borrow_mut()
            .player
            .as_mut()
            .is_some_and(|p| p.advance());
        if moved {
            self.show(canvas);
            self.schedule();
            None
        } else {
            self.set_playing(false);
            Some(Event::TextEnd)
        }
    }

    pub fn command(&self, cmd: Cmd, canvas: &BookCanvas) -> Option<Event> {
        if self.is_parked() {
            // a card waiting at a saved place: play starts from there, the speed can change
            match cmd {
                Cmd::PlayPause => return self.st.borrow().parked.clone().map(Event::Unpark),
                Cmd::Speed(d) => {
                    let w = (self.wpm() as i32 + d).clamp(50, 1000) as u32;
                    if let Some(k) = self.st.borrow_mut().parked.as_mut() {
                        k.wpm = w;
                    }
                    self.show_parked();
                    return None;
                }
                Cmd::Close => return self.close(canvas),
                _ => return None,
            }
        }
        match cmd {
            Cmd::PlayPause => {
                if self.playing() {
                    self.set_playing(false);
                } else if !self.is_empty() {
                    let finished = self
                        .st
                        .borrow()
                        .player
                        .as_ref()
                        .is_some_and(|p| p.finished());
                    if finished {
                        if let Some(p) = self.st.borrow_mut().player.as_mut() {
                            p.step(i64::MIN / 2);
                        }
                        self.show(canvas);
                    }
                    self.set_playing(true);
                    self.schedule();
                }
            }
            Cmd::Step(n) => {
                if let Some(p) = self.st.borrow_mut().player.as_mut() {
                    p.step(n);
                }
                self.show(canvas);
                if self.playing() {
                    self.schedule();
                }
            }
            Cmd::SentenceBack | Cmd::SentenceForward => {
                if let Some(p) = self.st.borrow_mut().player.as_mut() {
                    if cmd == Cmd::SentenceBack {
                        p.sentence_back();
                    } else {
                        p.sentence_forward();
                    }
                }
                self.show(canvas);
                if self.playing() {
                    self.schedule();
                }
            }
            Cmd::Speed(d) => {
                let w = (self.wpm() as i32 + d).max(0) as u32;
                if let Some(p) = self.st.borrow_mut().player.as_mut() {
                    p.set_wpm(w);
                }
                self.show(canvas);
            }
            Cmd::Close => return self.close(canvas),
            Cmd::Layout => {}
        }
        None
    }

    /// Stops and hides the card; returns the speed to remember.
    pub fn close(&self, canvas: &BookCanvas) -> Option<Event> {
        if !self.active.get() {
            return None;
        }
        let wpm = self.wpm();
        self.set_playing(false);
        {
            let mut st = self.st.borrow_mut();
            st.player = None;
            st.parked = None;
        }
        self.active.set(false);
        self.widget.set_visible(false);
        canvas.set_focus_word(None);
        Some(Event::Closed(wpm))
    }

    /// Test hook (`reader:rsvp-…`).
    pub fn dump(&self, canvas: &BookCanvas) -> String {
        let st = self.st.borrow();
        let Some(p) = st.player.as_ref() else {
            return match &st.parked {
                Some(k) => format!(
                    "parked word={} index={} of={} wpm={} chapter={} offset={} card={} pos=({:.2},{:.2}) size={}x{}",
                    k.text,
                    k.word,
                    k.words,
                    k.wpm,
                    k.chapter,
                    k.offset,
                    self.widget.is_visible() as u8,
                    self.pos.get().0,
                    self.pos.get().1,
                    self.ui.card.width(),
                    self.ui.card.height(),
                ),
                None => "closed".into(),
            };
        };
        format!(
            "word={} index={} of={} chapter={} playing={} wpm={} upcoming={} range={:?} marks={:?} card={} icon={} pos=({:.2},{:.2}) size={}x{}",
            p.current().map(|w| w.text.as_str()).unwrap_or(""),
            p.absolute_index(),
            p.absolute_len(),
            st.chapter,
            st.playing,
            p.wpm(),
            p.upcoming(3),
            p.word_range(),
            canvas.focus_word(),
            self.widget.is_visible() as u8,
            self.ui.play.icon_name().unwrap_or_default(),
            self.pos.get().0,
            self.pos.get().1,
            self.ui.card.width(),
            self.ui.card.height(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map_to_commands() {
        assert_eq!(key_cmd(gdk::Key::space), Some(Cmd::PlayPause));
        assert_eq!(key_cmd(gdk::Key::Left), Some(Cmd::Step(-1)));
        assert_eq!(key_cmd(gdk::Key::Right), Some(Cmd::Step(1)));
        assert_eq!(key_cmd(gdk::Key::Up), Some(Cmd::Speed(25)));
        assert_eq!(key_cmd(gdk::Key::Down), Some(Cmd::Speed(-25)));
        assert_eq!(key_cmd(gdk::Key::Escape), Some(Cmd::Close));
        assert_eq!(key_cmd(gdk::Key::a), None);
    }
}
