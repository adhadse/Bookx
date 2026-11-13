//! The "Read aloud" button of the reader (roadmap R10): play/pause, stop, speed and voice.
//! The reading itself is [`crate::tts`].

use crate::i18n::tr;
use crate::i18n::trf;
use std::cell::RefCell;
use std::rc::Rc;

use relm4::adw;
use relm4::gtk::{self, glib, prelude::*};

use crate::tts::{SleepTimer, TtsPrefs, Voice};

#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    PlayPause,
    Stop,
    /// The popover was opened: load the voices.
    Opened,
    Speed(f64),
    Pitch(f64),
    /// Previous (-1) or next (1) sentence.
    Skip(i32),
    /// `""` = the default voice.
    Voice(String),
    Sleep(SleepTimer),
    /// Open the settings page of read aloud.
    Settings,
    /// Save the chapter (`false`) or the whole book (`true`) as WAV files.
    SaveAudio(bool),
    /// The player card: jump to sentence number N of the chapter.
    Seek(usize),
    /// The player card: previous (-1) or next (1) chapter.
    Chapter(i32),
    /// The player card: volume 0..=1.
    Volume(f64),
    /// The player card: cancel the download of the voice.
    CancelPrep,
    /// The player card was moved or switched between small and full: remember it.
    Layout,
    /// Download the default voice and read when it is ready (`true`: always, do not ask again).
    DownloadVoice(bool),
}

/// The entries of the sleep timer: label and minutes (`None`: end of the chapter, 0: off).
pub const SLEEP: [(&str, Option<u32>); 6] = [
    ("Off", Some(0)),
    ("After 15 minutes", Some(15)),
    ("After 30 minutes", Some(30)),
    ("After 45 minutes", Some(45)),
    ("After 60 minutes", Some(60)),
    ("At the end of the chapter", None),
];

/// The timer an entry of the dropdown stands for.
pub fn sleep_of(entry: usize) -> SleepTimer {
    match SLEEP.get(entry).map(|e| e.1) {
        Some(Some(m)) => SleepTimer::after_minutes(m),
        Some(None) => SleepTimer::EndOfChapter,
        None => SleepTimer::Off,
    }
}

pub struct TtsUi {
    pub button: gtk::MenuButton,
    /// The floating player over the page (enhancement 11).
    pub player: Rc<super::tts_player::PlayerCard>,
    play: gtk::Button,
    speed: gtk::Scale,
    pitch: gtk::Scale,
    sleep: gtk::DropDown,
    engine: gtk::Label,
    voice: gtk::DropDown,
    model: gtk::StringList,
    names: Rc<RefCell<Vec<String>>>,
    quiet: Rc<std::cell::Cell<bool>>,
}

impl TtsUi {
    pub fn build(prefs: &TtsPrefs, on_cmd: impl Fn(Cmd) + Clone + 'static) -> TtsUi {
        let button = gtk::MenuButton::new();
        button.set_icon_name("audio-speakers-symbolic");
        button.set_tooltip_text(Some(&tr("Read aloud")));
        let pop = gtk::Popover::new();
        let bx = gtk::Box::new(gtk::Orientation::Vertical, 10);
        bx.set_margin_top(12);
        bx.set_margin_bottom(12);
        bx.set_margin_start(12);
        bx.set_margin_end(12);
        bx.set_width_request(280);

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.set_halign(gtk::Align::Center);
        let prev = gtk::Button::from_icon_name("media-skip-backward-symbolic");
        prev.add_css_class("circular");
        prev.set_tooltip_text(Some(&tr("Previous sentence")));
        super::audit::label(&prev, &tr("Previous sentence"));
        let play = gtk::Button::from_icon_name("media-playback-start-symbolic");
        play.add_css_class("suggested-action");
        play.add_css_class("circular");
        play.set_tooltip_text(Some(&tr("Read aloud from here")));
        super::audit::label(&play, &tr("Read aloud from here"));
        let stop = gtk::Button::from_icon_name("media-playback-stop-symbolic");
        stop.add_css_class("circular");
        stop.set_tooltip_text(Some(&tr("Stop reading")));
        super::audit::label(&stop, &tr("Stop reading"));
        let next = gtk::Button::from_icon_name("media-skip-forward-symbolic");
        next.add_css_class("circular");
        next.set_tooltip_text(Some(&tr("Next sentence")));
        super::audit::label(&next, &tr("Next sentence"));
        row.append(&prev);
        row.append(&play);
        row.append(&stop);
        row.append(&next);
        bx.append(&row);

        let engine = gtk::Label::new(None);
        engine.set_xalign(0.5);
        engine.set_wrap(true);
        engine.set_max_width_chars(36);
        engine.add_css_class("dim-label");
        engine.add_css_class("caption");
        engine.set_visible(false);
        bx.append(&engine);

        let speed_l = gtk::Label::new(Some(&tr("Speed")));
        speed_l.set_xalign(0.0);
        speed_l.add_css_class("heading");
        bx.append(&speed_l);
        let speed = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.5, 2.5, 0.1);
        super::audit::label(&speed, &tr("Reading speed"));
        speed.set_value(prefs.speed);
        speed.set_draw_value(true);
        speed.set_value_pos(gtk::PositionType::Right);
        speed.add_mark(1.0, gtk::PositionType::Bottom, None);
        bx.append(&speed);

        let pitch_l = gtk::Label::new(Some(&tr("Pitch")));
        pitch_l.set_xalign(0.0);
        pitch_l.add_css_class("heading");
        bx.append(&pitch_l);
        let pitch = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.8, 1.25, 0.05);
        super::audit::label(&pitch, &tr("Pitch"));
        pitch.set_value(prefs.pitch);
        pitch.set_draw_value(true);
        pitch.set_value_pos(gtk::PositionType::Right);
        pitch.add_mark(1.0, gtk::PositionType::Bottom, None);
        bx.append(&pitch);

        let voice_l = gtk::Label::new(Some(&tr("Voice")));
        voice_l.set_xalign(0.0);
        voice_l.add_css_class("heading");
        bx.append(&voice_l);
        let model = gtk::StringList::new(&["Default voice"]);
        let voice = gtk::DropDown::new(Some(model.clone()), gtk::Expression::NONE);
        super::audit::label(&voice, &tr("Voice"));
        bx.append(&voice);

        let sleep_l = gtk::Label::new(Some(&tr("Sleep timer")));
        sleep_l.set_xalign(0.0);
        sleep_l.add_css_class("heading");
        bx.append(&sleep_l);
        let sleep = gtk::DropDown::from_strings(&SLEEP.map(|e| e.0));
        super::audit::label(&sleep, &tr("Sleep timer"));
        bx.append(&sleep);

        let save_ch = gtk::Button::with_label(&tr("Save Chapter as Audio"));
        save_ch.add_css_class("flat");
        bx.append(&save_ch);
        let save_bk = gtk::Button::with_label(&tr("Save Book as Audio"));
        save_bk.add_css_class("flat");
        bx.append(&save_bk);
        let settings = gtk::Button::with_label(&tr("Voice settings"));
        settings.add_css_class("flat");
        bx.append(&settings);
        pop.set_child(Some(&bx));
        button.set_popover(Some(&pop));

        let quiet = Rc::new(std::cell::Cell::new(false));
        let names = Rc::new(RefCell::new(vec![String::new()]));
        let on_cmd2 = on_cmd.clone();
        {
            let f = on_cmd.clone();
            play.connect_clicked(move |_| f(Cmd::PlayPause));
            let f = on_cmd.clone();
            stop.connect_clicked(move |_| f(Cmd::Stop));
            let f = on_cmd.clone();
            prev.connect_clicked(move |_| f(Cmd::Skip(-1)));
            let f = on_cmd.clone();
            next.connect_clicked(move |_| f(Cmd::Skip(1)));
            for (b, whole) in [(&save_ch, false), (&save_bk, true)] {
                let (f, p2) = (on_cmd.clone(), pop.clone());
                b.connect_clicked(move |_| {
                    p2.popdown();
                    f(Cmd::SaveAudio(whole))
                });
            }
            let (f, p2) = (on_cmd.clone(), pop.clone());
            settings.connect_clicked(move |_| {
                p2.popdown();
                f(Cmd::Settings)
            });
            let (f, q) = (on_cmd.clone(), quiet.clone());
            pitch.connect_value_changed(move |s| {
                if !q.get() {
                    f(Cmd::Pitch((s.value() * 100.0).round() / 100.0));
                }
            });
            let (f, q) = (on_cmd.clone(), quiet.clone());
            sleep.connect_selected_notify(move |d| {
                if !q.get() {
                    f(Cmd::Sleep(sleep_of(d.selected() as usize)));
                }
            });
            let f = on_cmd.clone();
            pop.connect_show(move |_| f(Cmd::Opened));
            let (f, q) = (on_cmd.clone(), quiet.clone());
            speed.connect_value_changed(move |s| {
                if !q.get() {
                    f(Cmd::Speed((s.value() * 10.0).round() / 10.0));
                }
            });
            let (f, q, n) = (on_cmd, quiet.clone(), names.clone());
            voice.connect_selected_notify(move |d| {
                if q.get() {
                    return;
                }
                if let Some(name) = n.borrow().get(d.selected() as usize) {
                    f(Cmd::Voice(name.clone()));
                }
            });
        }
        let player = super::tts_player::PlayerCard::new(Rc::new(on_cmd2));
        TtsUi {
            button,
            player,
            play,
            speed,
            pitch,
            sleep,
            engine,
            voice,
            model,
            names,
            quiet,
        }
    }

    /// Shows the voices (the default first) and selects `current`.
    pub fn set_voices(&self, voices: &[Voice], current: &str) {
        self.quiet.set(true);
        let n = self.model.n_items();
        self.model.splice(1, n.saturating_sub(1), &[]);
        let mut names = vec![String::new()];
        for v in voices {
            self.model.append(&if v.language.is_empty() {
                v.name.clone()
            } else {
                format!("{} ({})", v.name, v.language)
            });
            names.push(v.name.clone());
        }
        let at = names.iter().position(|n| n == current).unwrap_or(0);
        *self.names.borrow_mut() = names;
        self.voice.set_selected(at as u32);
        self.quiet.set(false);
        self.player.set_voices(voices, current);
    }

    /// Names the engine that reads (and why it is not the one chosen, if so).
    pub fn set_engine(&self, name: &str, notice: Option<&str>) {
        self.engine.set_text(&match notice {
            Some(n) => format!("{name}. {n}"),
            None => trf("Voice: {}", &[&name]),
        });
        self.engine.set_visible(true);
    }

    /// Shows `v` on the speed slider without sending a command.
    pub fn show_speed(&self, v: f64) {
        self.quiet.set(true);
        self.speed.set_value(v);
        self.quiet.set(false);
    }

    /// The sleep timer ran out or was reset: show "Off".
    pub fn set_sleep_off(&self) {
        self.quiet.set(true);
        self.sleep.set_selected(0);
        self.quiet.set(false);
        self.player.set_sleep_off();
    }

    pub fn set_playing(&self, playing: bool) {
        self.play.set_icon_name(if playing {
            "media-playback-pause-symbolic"
        } else {
            "media-playback-start-symbolic"
        });
        let tip = if playing {
            "Pause reading"
        } else {
            "Read aloud from here"
        };
        self.play.set_tooltip_text(Some(tip));
        super::audit::label(&self.play, tip);
        self.button.set_icon_name(if playing {
            "media-playback-start-symbolic"
        } else {
            "audio-speakers-symbolic"
        });
    }

    /// Test helper: the names of the voices in the list.
    pub fn dev_voices(&self) -> Vec<String> {
        self.names.borrow().clone()
    }

    /// Test helper: picks a voice by name through the dropdown.
    pub fn dev_pick_voice(&self, name: &str) {
        if let Some(i) = self.names.borrow().iter().position(|n| n == name) {
            self.voice.set_selected(i as u32);
        }
    }

    /// Test helper: moves the speed slider.
    pub fn dev_set_speed(&self, v: f64) {
        self.speed.set_value(v);
    }

    /// Test helper: moves the pitch slider.
    pub fn dev_set_pitch(&self, v: f64) {
        self.pitch.set_value(v);
    }

    /// Test helper: picks an entry of the sleep timer.
    pub fn dev_pick_sleep(&self, entry: u32) {
        self.sleep.set_selected(entry);
    }

    /// Test helper: the line about the engine.
    pub fn dev_engine_text(&self) -> String {
        self.engine.text().to_string()
    }

    /// Test helper: what the play button shows.
    pub fn dev_play_icon(&self) -> String {
        self.play
            .icon_name()
            .map(|s| s.to_string())
            .unwrap_or_default()
    }
}

/// Repeats `f` every 150 ms until the returned id is removed.
pub fn every_tick(f: impl Fn() + 'static) -> glib::SourceId {
    glib::timeout_add_local(std::time::Duration::from_millis(150), move || {
        f();
        glib::ControlFlow::Continue
    })
}

// ---- Save as audio ---------------------------------------------------------------------------

enum Msg {
    Progress(usize, usize, String),
    Done(Result<Vec<std::path::PathBuf>, String>),
}

/// Starts the export: asks where to write (a file for a chapter, a folder for the book), then
/// synthesizes on a worker thread behind a dialog with progress and Cancel. `chapters` are
/// (title, text) of what is saved. `target` (tests) skips the file chooser.
#[allow(clippy::too_many_arguments)]
pub fn save_audio(
    anchor: &gtk::Widget,
    shared: &super::SharedRef,
    whole: bool,
    book_title: &str,
    chapters: Vec<(String, String)>,
    language: String,
    toast: Rc<dyn Fn(String)>,
    target: Option<std::path::PathBuf>,
) {
    let prefs = shared.cfg.borrow().tts.clamped();
    let jobs = if whole {
        crate::tts_export::plan(&chapters)
    } else {
        let name = format!(
            "{}.wav",
            bookx_lookup::export::sanitize_filename(&format!(
                "{} - {}",
                book_title,
                chapters.first().map_or("chapter", |c| c.0.as_str())
            ))
        );
        chapters
            .first()
            .and_then(|c| crate::tts_export::single(&c.0, &c.1, &name))
            .into_iter()
            .collect()
    };
    if jobs.is_empty() {
        toast(tr("There is no text to read here"));
        return;
    }
    let start = {
        let (anchor2, toast2) = (anchor.clone(), toast.clone());
        Rc::new(
            move |dir: std::path::PathBuf,
                  file: Option<String>,
                  jobs: Vec<crate::tts_export::Job>| {
                let mut jobs = jobs;
                if let (Some(f), Some(j)) = (file, jobs.first_mut()) {
                    j.file = f;
                }
                run_export(
                    &anchor2,
                    prefs.clone(),
                    language.clone(),
                    dir,
                    jobs,
                    toast2.clone(),
                );
            },
        )
    };
    if let Some(t) = target {
        if whole {
            start(t, None, jobs);
        } else {
            let file = t.file_name().map(|f| f.to_string_lossy().into_owned());
            start(t.parent().map(Into::into).unwrap_or_default(), file, jobs);
        }
        return;
    }
    let parent = anchor.root().and_downcast::<gtk::Window>();
    let jobs = Rc::new(std::cell::RefCell::new(Some(jobs)));
    if whole {
        let fd = gtk::FileDialog::builder()
            .title(tr("Choose a folder for the audio files"))
            .build();
        fd.select_folder(parent.as_ref(), gtk::gio::Cancellable::NONE, move |res| {
            if let (Some(dir), Some(j)) =
                (res.ok().and_then(|f| f.path()), jobs.borrow_mut().take())
            {
                start(dir, None, j);
            }
        });
    } else {
        let name = jobs
            .borrow()
            .as_ref()
            .and_then(|j| j.first().map(|j| j.file.clone()))
            .unwrap_or_default();
        let fd = gtk::FileDialog::builder()
            .title(tr("Save chapter as audio"))
            .initial_name(name)
            .build();
        fd.save(parent.as_ref(), gtk::gio::Cancellable::NONE, move |res| {
            if let (Some(p), Some(j)) = (res.ok().and_then(|f| f.path()), jobs.borrow_mut().take())
            {
                let file = p.file_name().map(|f| f.to_string_lossy().into_owned());
                start(p.parent().map(Into::into).unwrap_or_default(), file, j);
            }
        });
    }
}

fn run_export(
    anchor: &gtk::Widget,
    prefs: TtsPrefs,
    language: String,
    dir: std::path::PathBuf,
    jobs: Vec<crate::tts_export::Job>,
    toast: Rc<dyn Fn(String)>,
) {
    use adw::prelude::*;
    let bar = gtk::ProgressBar::new();
    bar.set_show_text(true);
    bar.set_text(Some(&tr("Loading the voice…")));
    let dlg = adw::AlertDialog::new(
        Some(&tr("Saving as audio")),
        Some(&tr("This can take a while for a whole book.")),
    );
    dlg.set_extra_child(Some(&bar));
    dlg.add_response("cancel", &tr("Cancel"));
    dlg.set_close_response("cancel");
    let cancel = crate::tts::Cancel::new();
    {
        let c = cancel.clone();
        dlg.connect_response(None, move |_, _| c.cancel());
    }
    dlg.present(Some(anchor));
    let (tx, rx) = std::sync::mpsc::channel();
    let c = cancel.clone();
    std::thread::spawn(move || {
        let r = (|| -> anyhow::Result<Vec<std::path::PathBuf>> {
            let (mut synth, id) = crate::tts::open_synthesizer(&prefs, &language)?;
            let voice = prefs.voice_for(&id, &language);
            let tx2 = tx.clone();
            crate::tts_export::run(
                &jobs,
                &dir,
                synth.as_mut(),
                &voice,
                &language,
                prefs.speed,
                prefs.pitch,
                &c,
                &mut |d, t, title| {
                    let _ = tx2.send(Msg::Progress(d, t, title.to_string()));
                },
            )
        })()
        .map_err(|e| format!("{e:#}"));
        let _ = tx.send(Msg::Done(r));
    });
    glib::timeout_add_local(std::time::Duration::from_millis(80), move || loop {
        match rx.try_recv() {
            Ok(Msg::Progress(d, t, title)) => {
                bar.set_fraction(d as f64 / t.max(1) as f64);
                bar.set_text(Some(&format!("{title}: {d} / {t}")));
            }
            Ok(Msg::Done(r)) => {
                dlg.force_close();
                let msg = match r {
                    Ok(files) => {
                        eprintln!("tts_save: files={}", files.len());
                        match files.as_slice() {
                            [one] => trf("Saved {}", &[&one.display()]),
                            many => trf("Saved {} audio files", &[&many.len()]),
                        }
                    }
                    Err(e) if e == "cancelled" => {
                        eprintln!("tts_save: cancelled");
                        "Saving as audio was cancelled".to_string()
                    }
                    Err(e) => {
                        eprintln!("tts_save: error: {e}");
                        trf("Could not save the audio: {}", &[&e])
                    }
                };
                toast(msg);
                return glib::ControlFlow::Break;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(_) => return glib::ControlFlow::Break,
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sleep_dropdown_maps_to_timers() {
        assert_eq!(sleep_of(0), SleepTimer::Off);
        assert!(matches!(sleep_of(2), SleepTimer::At(_)));
        assert_eq!(sleep_of(5), SleepTimer::EndOfChapter);
        assert_eq!(sleep_of(99), SleepTimer::Off);
        assert_eq!(SLEEP.len(), 6);
    }
}
