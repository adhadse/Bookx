//! Reading aids of the reader that need its state (a child module of `reader.rs`, so it sees the
//! reader's fields): the auto-turn timer (R49), the night schedule (R60), copy with a citation
//! (R45), link hints (R43), the keys of a page turner (R61) and the Archive shelf for finished
//! books (R58). The decisions are in `bookx_text::aids` and `crate::aids_prefs`; the settings are
//! in `reader_settings_extra.rs`.

use super::*;
use bookx_text::aids::{
    AutoTurn, AutoTurnTick, CitationFields, PageAnimation, REMOTE_DEFAULTS, RemoteAction, cite,
};

#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    /// Start or stop turning pages by themselves.
    AutoTurn,
    AutoTurnTick,
    /// Copy the selection with its citation.
    CopyCitation,
    /// Show the letters on the links of the page.
    LinkHints,
    /// A key of a page turner.
    Remote(RemoteAction),
    /// The progress slider is at this fraction: show the chapter there.
    Skim(f64),
}

#[derive(Default)]
pub struct AidsCtl {
    auto: RefCell<Option<(AutoTurn, glib::SourceId)>>,
    /// Where the page was at the last tick, and whether the next change is the timer's own.
    seen: Cell<(usize, usize)>,
    own_turn: Cell<bool>,
    /// (night, warmth in twentieths) the page colours were made for.
    night_sig: Cell<(bool, i32)>,
    archived: Cell<bool>,
    /// The skim preview (popover, label) and the number of the last request (to hide it later).
    skim: RefCell<Option<(gtk::Popover, gtk::Label)>>,
    skim_gen: Rc<Cell<u64>>,
    /// The highlight whose ends are being moved with the handles.
    pub(super) adjusting: RefCell<Option<String>>,
}

thread_local! {
    /// The options of the search in the book: (whole words, match case, regular expression).
    static SEARCH_FLAGS: Cell<(bool, bool, bool)> = const { Cell::new((false, false, false)) };
}

pub fn search_flags() -> (bool, bool, bool) {
    SEARCH_FLAGS.with(|f| f.get())
}

pub fn set_search_flag(which: usize, on: bool) {
    SEARCH_FLAGS.with(|f| {
        let mut v = f.get();
        match which {
            0 => v.0 = on,
            1 => v.1 = on,
            _ => v.2 = on,
        }
        f.set(v);
    });
}

/// The compiled search of the popover; the error says what is wrong with a regular expression.
pub fn search_pattern(query: &str) -> Result<Option<regex::Regex>, String> {
    let (whole_word, case_sensitive, regex) = search_flags();
    bookx_lookup::search::compile(
        query,
        &bookx_lookup::search::SearchOptions {
            whole_word,
            case_sensitive,
            regex,
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())
}

/// The row of option buttons under the search entry. `changed` runs after one is toggled.
pub fn search_options_row(changed: impl Fn() + Clone + 'static) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.set_margin_start(8);
    row.set_margin_end(8);
    let flags = search_flags();
    for (i, (label, tip, on)) in [
        (tr("Words"), tr("Whole words only"), flags.0),
        (tr("Aa"), tr("Match case"), flags.1),
        (tr(".*"), tr("Regular expression"), flags.2),
    ]
    .into_iter()
    .enumerate()
    {
        let b = gtk::ToggleButton::with_label(&label);
        b.add_css_class("flat");
        b.set_tooltip_text(Some(&tip));
        b.set_active(on);
        crate::ui::audit::label(&b, &tip);
        let changed = changed.clone();
        b.connect_toggled(move |b| {
            set_search_flag(i, b.is_active());
            changed();
        });
        row.append(&b);
    }
    row
}

fn local_minute() -> u16 {
    use chrono::Timelike;
    let n = chrono::Local::now();
    (n.hour() * 60 + n.minute()) as u16
}

fn mono_ms() -> u64 {
    use std::sync::OnceLock;
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}

/// The reader action of a key of a page turner, with the user's lines over the defaults.
pub fn remote_action(shared: &SharedRef, key: gdk::Key) -> Option<RemoteAction> {
    let name = key.name()?;
    let lines = shared.cfg.borrow().aids.remote_keys.clone();
    if lines.is_empty() && !REMOTE_DEFAULTS.iter().any(|(k, _)| *k == name.as_str()) {
        return None;
    }
    bookx_text::aids::remote_map(&lines)
        .get(name.as_str())
        .copied()
}

impl Reader {
    /// True when the system is dark or when the night schedule wants the dark theme now.
    pub(super) fn night_dark(&self, prefs: &ReaderPrefs, system_dark: bool) -> bool {
        let night = self.shared.cfg.borrow().aids.night.clamped();
        system_dark
            || (night.is_night(local_minute())
                && night.dark
                && prefs.colors == ColorMode::Theme
                && prefs.theme == ReaderTheme::Auto
                && !crate::eink::is_active())
    }

    /// The warm tint of the night schedule over the page colours.
    pub(super) fn night_warm(&self, style: &mut crate::ui::canvas::CanvasStyle) {
        let night = self.shared.cfg.borrow().aids.night.clamped();
        let minute = local_minute();
        let w = if crate::eink::is_active() {
            0.0
        } else {
            night.warmth_at(minute)
        };
        self.aids
            .night_sig
            .set((night.is_night(minute), (w * 20.0).round() as i32));
        if w <= 0.0 {
            return;
        }
        let p = &mut style.palette;
        for c in [
            &mut p.bg,
            &mut p.fg,
            &mut p.link,
            &mut p.muted,
            &mut p.accent,
        ] {
            *c = bookx_text::aids::warm_tint(*c, w);
        }
    }

    /// Every half minute: the page colours follow the schedule.
    pub(super) fn night_check(&self) {
        let night = self.shared.cfg.borrow().aids.night.clamped();
        let minute = local_minute();
        let sig = (
            night.is_night(minute),
            (night.warmth_at(minute) * 20.0).round() as i32,
        );
        self.canvas
            .set_page_animation(self.shared.cfg.borrow().aids.page_animation);
        if sig != self.aids.night_sig.get() {
            self.apply_prefs();
        }
    }

    pub(super) fn aids_cmd(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::AutoTurn => self.auto_turn_toggle(),
            Cmd::AutoTurnTick => self.auto_turn_tick(mono_ms()),
            Cmd::CopyCitation => {
                if let Some(t) = self
                    .canvas
                    .selected_text()
                    .or_else(|| self.selection.as_ref().map(|s| s.text.clone()))
                {
                    let text = self.citation_for(&t);
                    self.canvas.area.clipboard().set_text(&text);
                    let _ = self
                        .sender
                        .output(Output::Toast(tr("Copied with the citation")));
                }
            }
            Cmd::LinkHints => {
                if self.canvas.hints_active() {
                    self.canvas.clear_link_hints();
                } else if self.canvas.show_link_hints() == 0 {
                    let _ = self
                        .sender
                        .output(Output::Toast(tr("There are no links on this page")));
                }
            }
            Cmd::Skim(f) => self.skim_preview(f),
            Cmd::Remote(a) => match a {
                RemoteAction::NextPage => self.canvas.turn(true),
                RemoteAction::PrevPage => self.canvas.turn(false),
                RemoteAction::NextChapter => self.sender.input(Input::Chapter(true)),
                RemoteAction::PrevChapter => self.sender.input(Input::Chapter(false)),
                RemoteAction::ToggleChrome => self.sender.input(Input::Fullscreen),
            },
        }
    }

    fn skim_preview(&self, f: f64) {
        let idx = bookx_text::aids::skim_chapter(&self.weights, f);
        let text = format!(
            "{}\n{}%",
            self.chapter_title(idx),
            (f.clamp(0.0, 1.0) * 100.0).round()
        );
        let slider = &self.w.slider;
        let mut slot = self.aids.skim.borrow_mut();
        let (pop, label) = slot.get_or_insert_with(|| {
            let pop = gtk::Popover::new();
            pop.set_parent(slider);
            pop.set_autohide(false);
            pop.set_can_target(false);
            pop.add_css_class("skim-preview");
            let label = gtk::Label::new(None);
            label.set_margin_top(6);
            label.set_margin_bottom(6);
            label.set_margin_start(10);
            label.set_margin_end(10);
            label.set_max_width_chars(36);
            label.set_wrap(true);
            pop.set_child(Some(&label));
            (pop, label)
        });
        label.set_label(&text);
        let w = slider.width().max(1);
        pop.set_pointing_to(Some(&gdk::Rectangle::new(
            (f.clamp(0.0, 1.0) * w as f64) as i32,
            0,
            1,
            slider.height(),
        )));
        if !pop.is_visible() {
            pop.popup();
        }
        let r#gen = self.aids.skim_gen.get() + 1;
        self.aids.skim_gen.set(r#gen);
        let (g, pop2) = (self.aids.skim_gen.clone(), pop.clone());
        glib::timeout_add_local_once(std::time::Duration::from_millis(900), move || {
            if g.get() == r#gen {
                pop2.popdown();
            }
        });
    }

    /// After a selection handle was let go: the highlight that was being adjusted takes the new
    /// ends and text (R29).
    pub(super) fn adjust_highlight(&self) {
        let Some(id) = self
            .aids
            .adjusting
            .borrow()
            .clone()
            .or_else(|| self.selection.as_ref().and_then(|s| s.highlight.clone()))
        else {
            return;
        };
        let Some((start, end, text)) = self.canvas.dev_selection_range() else {
            return;
        };
        let changed = {
            let mut lib = self.shared.lib.borrow_mut();
            let Some(h) = lib
                .get_mut(self.book_id)
                .and_then(|b| b.highlights.iter_mut().find(|h| h.id == id))
            else {
                return;
            };
            if h.start == start && h.end == end {
                false
            } else {
                h.start = start;
                h.end = end;
                h.text = text;
                let _ = lib.save();
                true
            }
        };
        if changed {
            self.refresh_highlights();
            self.rebuild_lists();
        }
    }

    fn auto_turn_toggle(&mut self) {
        let was = self.aids.auto.borrow_mut().take();
        if let Some((_, id)) = was {
            id.remove();
            let _ = self.sender.output(Output::Toast(tr("Auto-turn is off")));
            return;
        }
        let secs = self
            .shared
            .cfg
            .borrow()
            .aids
            .clone()
            .clamped()
            .auto_turn_secs;
        let tx = self.sender.input_sender().clone();
        let id = glib::timeout_add_seconds_local(1, move || {
            tx.emit(Input::Aids(Cmd::AutoTurnTick));
            glib::ControlFlow::Continue
        });
        *self.aids.auto.borrow_mut() = Some((AutoTurn::new(secs, mono_ms()), id));
        self.aids.seen.set((self.spine_idx, self.pos.page));
        let _ = self.sender.output(Output::Toast(crate::i18n::trf(
            "Auto-turn: a page every {} seconds",
            &[&secs.to_string()],
        )));
    }

    fn auto_turn_tick(&mut self, now: u64) {
        let at_end = self.spine_idx + 1 >= self.spine.len() && self.pos.page >= self.pos.pages;
        let busy = self.canvas.has_selection()
            || self.w.sel_pop.is_visible()
            || self.canvas.hints_active()
            || self.tts.session.state() == crate::tts::State::Playing;
        let here = (self.spine_idx, self.pos.page);
        let verdict = {
            let mut slot = self.aids.auto.borrow_mut();
            let Some((timer, _)) = slot.as_mut() else {
                return;
            };
            if here != self.aids.seen.get() {
                self.aids.seen.set(here);
                // a turn of the timer is expected; any other move starts a full interval again
                if !self.aids.own_turn.replace(false) {
                    timer.moved(now);
                }
            }
            timer.tick(now, at_end, busy)
        };
        match verdict {
            AutoTurnTick::Wait => {}
            AutoTurnTick::Turn => {
                self.aids.own_turn.set(true);
                self.canvas.turn(true);
            }
            AutoTurnTick::Stop => {
                if let Some((_, id)) = self.aids.auto.borrow_mut().take() {
                    id.remove();
                }
                let _ = self.sender.output(Output::Toast(tr(
                    "Auto-turn stopped at the end of the book",
                )));
            }
        }
    }

    pub(super) fn auto_turn_running(&self) -> bool {
        self.aids.auto.borrow().is_some()
    }

    /// The citation fields of the open book for `text`.
    fn citation_fields(&self, text: &str) -> CitationFields {
        let m = self.book.metadata();
        CitationFields {
            text: text.to_string(),
            title: m.title.clone(),
            author: m.authors.join(", "),
            chapter: self.chapter_title(self.spine_idx),
            page: self.pos.page.to_string(),
            date: chrono::Local::now().format("%Y-%m-%d").to_string(),
            isbn: m.identifier("isbn").unwrap_or_default().to_string(),
        }
    }

    pub(super) fn citation_for(&self, text: &str) -> String {
        let template = self.shared.cfg.borrow().aids.citation_template.clone();
        cite(&template, &self.citation_fields(text))
    }

    /// What Copy puts on the clipboard: the text, with its citation when the setting asks for it.
    pub(super) fn copy_text(&self, text: String) -> String {
        if self.shared.cfg.borrow().aids.copy_with_citation {
            self.citation_for(&text)
        } else {
            text
        }
    }

    /// A book that was read to the end goes to the Archive shelf, when the setting is on.
    pub(super) fn archive_if_finished(&self, frac: f64) {
        let aids = self.shared.cfg.borrow().aids.clone();
        if !crate::aids_prefs::should_archive(&aids, frac, self.aids.archived.get()) {
            return;
        }
        self.aids.archived.set(true);
        let name = crate::aids_prefs::ARCHIVE_SHELF;
        if super::super::shelves::members(&self.shared, name).is_none()
            && super::super::shelves::create(&self.shared, name).is_err()
        {
            return;
        }
        super::super::shelves::add_books(&self.shared, name, &[self.book_id]);
        let _ = self.sender.output(Output::Toast(crate::i18n::trf(
            "Added to the shelf “{}”",
            &[&name],
        )));
    }

    /// `reader:aids:...` test commands.
    pub(super) fn aids_dev(&mut self, c: &str) {
        match c {
            "auto" => self.aids_cmd(Cmd::AutoTurn),
            "tick" => {
                // a tick that finds the interval over
                let due =
                    mono_ms() + self.shared.cfg.borrow().aids.auto_turn_secs as u64 * 1000 + 1;
                self.auto_turn_tick(due);
            }
            "state" => eprintln!(
                "aids: auto={} animating={} hints={} night_sig={:?} page={}/{} spine={}",
                self.auto_turn_running(),
                self.canvas.dev_animating(),
                self.canvas.dev_hints(),
                self.aids.night_sig.get(),
                self.pos.page,
                self.pos.pages,
                self.spine_idx
            ),
            "hints" => {
                self.aids_cmd(Cmd::LinkHints);
                eprintln!("hints: {}", self.canvas.dev_hints());
            }
            "cite" => {
                let t = self
                    .canvas
                    .selected_text()
                    .unwrap_or_else(|| "Sample".into());
                eprintln!("cite: {}", self.citation_for(&t).replace('\n', "/"));
            }
            "copy" => {
                let t = self
                    .canvas
                    .selected_text()
                    .unwrap_or_else(|| "Sample".into());
                eprintln!("copy: {}", self.copy_text(t).replace('\n', "/"));
            }
            "archive" => self.archive_if_finished(1.0),
            k if k.starts_with("skim=") => {
                self.skim_preview(k[5..].parse().unwrap_or(0.5));
                let t = self
                    .aids
                    .skim
                    .borrow()
                    .as_ref()
                    .map(|(p, l)| (p.is_visible(), l.label()));
                eprintln!("skim: {:?}", t);
            }
            k if k.starts_with("sflag=") => {
                // sflag=N:on|off  (0 whole words, 1 match case, 2 regular expression)
                if let Some((n, v)) = k[6..].split_once(':') {
                    set_search_flag(n.parse().unwrap_or(0), v == "on");
                }
            }
            "adjusting" => eprintln!("adjusting: {:?}", self.aids.adjusting.borrow()),
            "turn" => {
                self.canvas.turn(true);
                eprintln!("aids_turn: animating={}", self.canvas.dev_animating());
            }
            k if k.starts_with("anim=") => {
                let kind = PageAnimation::from_key(&k[5..]);
                self.shared.cfg.borrow_mut().aids.page_animation = kind;
                self.canvas.set_page_animation(kind);
            }
            k if k.starts_with("reduced=") => {
                if let Some(st) = gtk::Settings::default() {
                    st.set_gtk_enable_animations(&k[8..] != "on");
                }
            }
            k if k.starts_with("night=") => {
                // `night=on:HH:MM:HH:MM:warmth` or `night=off`
                let mut c = self.shared.cfg.borrow_mut();
                let v: Vec<&str> = k[6..].split(':').collect();
                c.aids.night.enabled = v.first() == Some(&"on");
                if let [_, a, b, c2, d, w] = v[..] {
                    c.aids.night.from =
                        bookx_text::aids::parse_hhmm(&format!("{a}:{b}")).unwrap_or(0);
                    c.aids.night.to =
                        bookx_text::aids::parse_hhmm(&format!("{c2}:{d}")).unwrap_or(0);
                    c.aids.night.warmth = w.parse().unwrap_or(0.5);
                }
                drop(c);
                self.apply_prefs();
                let (r, g, b) = self.canvas.style().palette.bg;
                eprintln!(
                    "night: bg={:02x}{:02x}{:02x} sig={:?}",
                    (r * 255.0).round() as u8,
                    (g * 255.0).round() as u8,
                    (b * 255.0).round() as u8,
                    self.aids.night_sig.get()
                );
            }
            k if k.starts_with("key=") => {
                let key = gdk::Key::from_name(&k[4..]);
                let a = key.and_then(|key| remote_action(&self.shared, key));
                eprintln!(
                    "remote: {:?} -> {:?} (key {:?} name {:?})",
                    &k[4..],
                    a,
                    key,
                    key.and_then(|k| k.name())
                );
                if let Some(a) = a {
                    self.aids_cmd(Cmd::Remote(a));
                }
            }
            k if k.starts_with("hint=") => {
                for ch in k[5..].chars() {
                    eprintln!("hint_key: {:?}", self.canvas.hint_key(ch));
                }
            }
            _ => {}
        }
    }
}
