//! The "Read aloud" button of the reader (roadmap R10): play/pause, stop, speed and voice.
//! The reading itself is [`crate::tts`].

use crate::i18n::trf;
use crate::i18n::{tr, N_};
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
    (N_("Off"), Some(0)),
    (N_("After 15 minutes"), Some(15)),
    (N_("After 30 minutes"), Some(30)),
    (N_("After 45 minutes"), Some(45)),
    (N_("After 60 minutes"), Some(60)),
    (N_("At the end of the chapter"), None),
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
    engine: gtk::Label,
    names: Rc<RefCell<Vec<String>>>,
    on_cmd: Rc<dyn Fn(Cmd)>,
}

impl TtsUi {
    /// The button of the header and its popover: play and pause, and three rows (voice settings, save
    /// the chapter, save the book). Speed, voice, sleep timer and volume are on the player card.
    pub fn build(_prefs: &TtsPrefs, on_cmd: impl Fn(Cmd) + Clone + 'static) -> TtsUi {
        let button = gtk::MenuButton::new();
        button.set_icon_name("audio-speakers-symbolic");
        button.set_tooltip_text(Some(&tr("Read Aloud")));
        super::audit::label(&button, &tr("Read Aloud"));
        let pop = gtk::Popover::new();
        let bx = gtk::Box::new(gtk::Orientation::Vertical, 6);
        bx.set_margin_top(10);
        bx.set_margin_bottom(10);
        bx.set_margin_start(10);
        bx.set_margin_end(10);
        bx.set_width_request(240);

        let play = gtk::Button::from_icon_name("media-playback-start-symbolic");
        play.add_css_class("flat");
        play.add_css_class("circular");
        play.set_halign(gtk::Align::Center);
        play.set_tooltip_text(Some(&tr("Read Aloud from Here")));
        super::audit::label(&play, &tr("Read Aloud from Here"));
        bx.append(&play);

        let engine = gtk::Label::new(None);
        engine.set_xalign(0.5);
        engine.set_wrap(true);
        engine.set_max_width_chars(32);
        engine.add_css_class("dim-label");
        engine.add_css_class("caption");
        engine.set_visible(false);
        bx.append(&engine);
        bx.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

        let row = |label: &str| {
            let b = gtk::Button::with_label(label);
            b.add_css_class("flat");
            if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                l.set_xalign(0.0);
            }
            bx.append(&b);
            b
        };
        let settings = row(&tr("Voice Settings"));
        let save_ch = row(&tr("Save Chapter as Audio"));
        let save_bk = row(&tr("Save Book as Audio"));
        pop.set_child(Some(&bx));
        button.set_popover(Some(&pop));

        let names = Rc::new(RefCell::new(vec![String::new()]));
        let on_cmd: Rc<dyn Fn(Cmd)> = Rc::new(on_cmd);
        {
            let f = on_cmd.clone();
            play.connect_clicked(move |_| f(Cmd::PlayPause));
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
            let f = on_cmd.clone();
            pop.connect_show(move |_| f(Cmd::Opened));
        }
        let player = super::tts_player::PlayerCard::new(on_cmd.clone());
        TtsUi {
            button,
            player,
            play,
            engine,
            names,
            on_cmd,
        }
    }

    /// Shows the voices (the default first) and selects `current`.
    pub fn set_voices(&self, voices: &[Voice], current: &str) {
        let mut names = vec![String::new()];
        names.extend(voices.iter().map(|v| v.name.clone()));
        *self.names.borrow_mut() = names;
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

    /// The sleep timer ran out or was reset: show "Off".
    pub fn set_sleep_off(&self) {
        self.player.set_sleep_off();
    }

    pub fn set_playing(&self, playing: bool) {
        self.play.set_icon_name(if playing {
            "media-playback-pause-symbolic"
        } else {
            "media-playback-start-symbolic"
        });
        let tip = if playing {
            tr("Pause Reading")
        } else {
            tr("Read Aloud from Here")
        };
        self.play.set_tooltip_text(Some(&tip));
        super::audit::label(&self.play, &tip);
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

    /// Test helper: picks a voice by name the way the card's voice list does.
    pub fn dev_pick_voice(&self, name: &str) {
        if self.names.borrow().iter().any(|n| n == name) {
            (self.on_cmd)(Cmd::Voice(name.to_string()));
        }
    }

    /// Test helper: sets the speed the way the player card's buttons do.
    pub fn dev_set_speed(&self, v: f64) {
        (self.on_cmd)(Cmd::Speed((v * 10.0).round() / 10.0));
    }

    /// Test helper: sets the pitch (it is set in Preferences → Text to Speech).
    pub fn dev_set_pitch(&self, v: f64) {
        (self.on_cmd)(Cmd::Pitch((v * 100.0).round() / 100.0));
    }

    /// Test helper: picks an entry of the sleep timer the way the card's menu does.
    pub fn dev_pick_sleep(&self, entry: u32) {
        (self.on_cmd)(Cmd::Sleep(sleep_of(entry as usize)));
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
    bar.set_text(Some(&tr("Loading the voice")));
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
