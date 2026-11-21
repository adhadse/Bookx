//! Tools → Add Narration / Remove Narration (CE-422): EPUB 3 Media Overlays made with the read-aloud
//! engines. The engines run on a worker thread behind a dialog with progress and Cancel (and show up
//! in the Jobs panel); the worker narrates a **copy** of the book chapter by chapter, and when it
//! stops, finished or not, the copy replaces the book inside [`Editor::run_op`], so the whole run is
//! one checkpoint "Before: Add Narration". A cancelled run keeps its finished chapters; running it
//! again continues with the rest. The logic is in `bookx_editing::media_overlay_*` and
//! `crate::narration`; this file is the dialogs and the thread.
//!
//! The menu items reach here through `Input::RunPluginTool("narration:<cmd>")` (`add`, `start`,
//! `remove`, `apply`), so the dev hook `tool=plugin:narration:start` runs it without the dialogs.

use std::cell::RefCell;
use std::collections::HashMap;

use bookx_editing::media_overlay_check::has_overlays;
use bookx_editing::media_overlay_narrate as mo;

use super::*;
use crate::core::archive::Archive;
use crate::core::book::OpenBook;

enum Msg {
    Progress(mo::Progress),
    Done(Finished),
}

/// What the worker hands back: the book as it is when the run ended.
struct Finished {
    archive: Archive,
    result: anyhow::Result<mo::Report>,
    /// Chapters narrated by this run (also when it ended with an error).
    narrated: usize,
    /// The fingerprint of the book when the run started (the book may be edited while it runs).
    started: u64,
}

thread_local! {
    static FINISHED: RefCell<HashMap<u64, Finished>> = RefCell::new(HashMap::new());
}

/// The text of the toast for an error of the run.
fn error_text(e: &anyhow::Error) -> String {
    use crate::tts::NotReady;
    match e.downcast_ref::<NotReady>() {
        Some(NotReady::NoVoice) => tr(
            "No voice is installed. Download one in Preferences → Text to Speech, then try again.",
        ),
        Some(NotReady::NoEspeak) => tr(
            "The voice needs espeak-ng, which is not installed. See Preferences → Text to Speech.",
        ),
        Some(NotReady::Language(l)) => trf(
            "No downloaded voice reads {}. Download a voice for it in Preferences → Text to Speech.",
            &[l],
        ),
        None => trf("Narration stopped: {}", &[&format!("{e:#}")]),
    }
}

impl Editor {
    pub(super) fn narration_cmd(&mut self, cmd: &str) {
        match cmd {
            "add" => self.narration_confirm(),
            "start" => self.narration_start(),
            "apply" => self.narration_apply(),
            "remove" => self.narration_remove(),
            other => self.toast(&format!("Unknown narration command {other}")),
        }
    }

    fn narration_confirm(&mut self) {
        self.flush();
        if !self.book.borrow().package.is_epub3() {
            self.toast(&tr(
                "Narration needs an EPUB 3 book. Use Tools → Upgrade to EPUB 3 first.",
            ));
            return;
        }
        let (todo, chars) = mo::pending(&self.book.borrow());
        if todo == 0 {
            self.toast(&tr(
                "Every chapter is narrated already, or there is no text to read.",
            ));
            return;
        }
        let dlg = adw::AlertDialog::new(
            Some(&tr("Add Narration?")),
            Some(&crate::narration::confirm_text(todo, chars)),
        );
        dlg.add_response("cancel", &tr("Cancel"));
        dlg.add_response("start", &tr("Start"));
        dlg.set_response_appearance("start", adw::ResponseAppearance::Suggested);
        dlg.set_default_response(Some("start"));
        dlg.set_close_response("cancel");
        let tx = self.sender.input_sender().clone();
        dlg.connect_response(None, move |_, r| {
            if r == "start" {
                tx.emit(Input::RunPluginTool("narration:start".into()));
            }
        });
        dlg.present(Some(&self.w.win));
    }

    fn narration_start(&mut self) {
        self.flush();
        if !self.book.borrow().package.is_epub3() {
            self.toast(&tr(
                "Narration needs an EPUB 3 book. Use Tools → Upgrade to EPUB 3 first.",
            ));
            return;
        }
        let prefs = self.shared.cfg.borrow().tts.clamped();
        let language = crate::tts::primary_language(&self.book.borrow().metadata().language);
        let archive = self.book.borrow().archive.clone();
        let started = editor_tools::archive_fingerprint(&archive);
        let title = trf(
            "Narrating {}",
            &[&self.book.borrow().metadata().title.clone()],
        );
        let (job, token) = crate::jobs::start(crate::jobs::Kind::Convert, &title, true);

        let bar = gtk::ProgressBar::new();
        bar.set_show_text(true);
        bar.set_text(Some(&tr("Loading the voice…")));
        let dlg = adw::Dialog::new();
        dlg.set_title(&tr("Adding Narration"));
        dlg.set_content_width(420);
        let note = gtk::Label::new(Some(&tr(
            "Finished chapters are kept if you cancel. You can keep working; editing the book while this runs discards the narration.",
        )));
        note.set_wrap(true);
        note.set_xalign(0.0);
        note.add_css_class("dim-label");
        let background = gtk::Button::with_label(&tr("Run in Background"));
        background.set_widget_name("narration-background");
        let stop = gtk::Button::with_label(&tr("Cancel Narration"));
        stop.set_widget_name("narration-cancel");
        stop.add_css_class("destructive-action");
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        buttons.set_halign(gtk::Align::End);
        buttons.append(&stop);
        buttons.append(&background);
        let col = gtk::Box::new(gtk::Orientation::Vertical, 12);
        col.set_margin_all(16);
        col.append(&note);
        col.append(&bar);
        col.append(&buttons);
        let view = adw::ToolbarView::new();
        view.add_top_bar(&adw::HeaderBar::new());
        view.set_content(Some(&col));
        dlg.set_child(Some(&view));
        let cancel = crate::tts::Cancel::new();
        {
            let (c, d) = (cancel.clone(), dlg.clone());
            stop.connect_clicked(move |_| {
                c.cancel();
                d.close();
            });
            let d = dlg.clone();
            background.connect_clicked(move |_| {
                d.close();
            });
        }
        dlg.present(Some(&self.w.win));

        let (tx, rx) = std::sync::mpsc::channel::<Msg>();
        let (c, t) = (cancel.clone(), token.clone());
        std::thread::spawn(move || {
            let stop = {
                let (c, t) = (c.clone(), t.clone());
                move || c.is_cancelled() || t.is_cancelled()
            };
            let mut book = match OpenBook::from_archive(archive.clone()) {
                Ok(b) => b,
                Err(e) => {
                    let _ = tx.send(Msg::Done(Finished {
                        archive,
                        result: Err(e),
                        narrated: 0,
                        started,
                    }));
                    return;
                }
            };
            let result = (|| -> anyhow::Result<mo::Report> {
                let (synth, id) = crate::tts::open_synthesizer(&prefs, &language)?;
                let voice = prefs.voice_for(&id, &language);
                let mut ts = crate::narration::TtsSynth {
                    synth,
                    voice,
                    language: language.clone(),
                    speed: prefs.speed,
                    pitch: prefs.pitch,
                    cancel: c.clone(),
                };
                let tx2 = tx.clone();
                mo::narrate_book(
                    &mut book,
                    &mut ts,
                    &mo::Options::default(),
                    &stop,
                    &mut |p| {
                        let _ = tx2.send(Msg::Progress(p.clone()));
                    },
                )
            })();
            let narrated = match &result {
                Ok(r) => r.narrated,
                Err(_) => {
                    let (_, done_now) = mo::narration_status(&book);
                    let (_, done_before) = OpenBook::from_archive(archive)
                        .map_or((0, 0), |b| mo::narration_status(&b));
                    done_now.saturating_sub(done_before)
                }
            };
            let _ = tx.send(Msg::Done(Finished {
                archive: book.archive,
                result,
                narrated,
                started,
            }));
        });

        let book_id = self.book_id;
        let input = self.sender.input_sender().clone();
        glib::timeout_add_local(std::time::Duration::from_millis(80), move || loop {
            match rx.try_recv() {
                Ok(Msg::Progress(p)) => {
                    let whole =
                        (p.chapter - 1) as f64 + p.sentence as f64 / p.sentences.max(1) as f64;
                    let f = whole / p.chapters.max(1) as f64;
                    bar.set_fraction(f);
                    let text = trf(
                        "Chapter {} of {}: sentence {} of {}",
                        &[&p.chapter, &p.chapters, &p.sentence, &p.sentences],
                    );
                    bar.set_text(Some(&text));
                    crate::jobs::progress(job, Some(f), &text);
                }
                Ok(Msg::Done(fin)) => {
                    dlg.force_close();
                    let cancelled = matches!(&fin.result, Ok(r) if r.cancelled);
                    match &fin.result {
                        Err(e) if fin.narrated == 0 => {
                            crate::jobs::finish(job, Err(format!("{e:#}")))
                        }
                        _ if cancelled => crate::jobs::finish(job, Err("cancelled".into())),
                        _ => crate::jobs::finish(job, Ok(())),
                    }
                    FINISHED.with(|f| f.borrow_mut().insert(book_id, fin));
                    input.emit(Input::RunPluginTool("narration:apply".into()));
                    return glib::ControlFlow::Break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(_) => {
                    dlg.force_close();
                    crate::jobs::finish(job, Err("the worker stopped".into()));
                    return glib::ControlFlow::Break;
                }
            }
        });
    }

    fn narration_apply(&mut self) {
        let Some(fin) = FINISHED.with(|f| f.borrow_mut().remove(&self.book_id)) else {
            return;
        };
        let Finished {
            archive,
            result,
            narrated,
            started,
        } = fin;
        self.flush();
        if narrated > 0 && editor_tools::archive_fingerprint(&self.book.borrow().archive) != started
        {
            self.toast(&tr(
                "The book was edited while it was being narrated, so the narration was not added. Run it again.",
            ));
            return;
        }
        let (cancelled, ms) = match &result {
            Ok(r) => (r.cancelled, r.duration_ms),
            Err(_) => (false, 0),
        };
        if narrated > 0 {
            let summary = crate::narration::summary(narrated, cancelled, ms);
            self.run_op(&tr("Add Narration"), move |book, _| {
                book.archive = archive;
                book.reload_package()?;
                Ok(editor_tools::Outcome {
                    changed: true,
                    summary,
                    details: Vec::new(),
                })
            });
        } else if cancelled {
            self.toast(&tr("Cancelled: nothing was narrated"));
        }
        if let Err(e) = result {
            self.toast(&error_text(&e));
        }
    }

    fn narration_remove(&mut self) {
        self.flush();
        if !has_overlays(&self.book.borrow().archive) {
            self.toast(&tr("This book has no narration."));
            return;
        }
        self.run_op(&tr("Remove Narration"), |book, _| {
            let r = mo::remove_narration(book)?;
            Ok(editor_tools::Outcome {
                changed: true,
                summary: trf(
                    "Removed the narration of {} chapters ({} files)",
                    &[&r.chapters, &r.files],
                ),
                details: Vec::new(),
            })
        });
    }
}

impl Editor {
    /// Dev hooks of the scenarios (`editor:mo:<command>`): what the book has after a narration run.
    pub(super) fn mo_dev(&mut self, c: &str) {
        self.flush();
        let b = self.book.borrow();
        match c {
            "files" => {
                let names = b.archive.names();
                let count = |ext: &str| names.iter().filter(|n| n.ends_with(ext)).count();
                let opf = b
                    .archive
                    .get_string(&b.package.opf_path)
                    .unwrap_or_default();
                let (total, done) = mo::narration_status(&b);
                eprintln!(
                    "editor_mo: smil={} wav={} overlay_attrs={} chapters={total} narrated={done} spans={}",
                    count(".smil"),
                    count(".wav"),
                    opf.matches("media-overlay=").count(),
                    names
                        .iter()
                        .filter(|n| n.ends_with(".xhtml"))
                        .filter_map(|n| b.archive.get_string(n))
                        .map(|t| t.matches("<span id=\"bxs").count())
                        .sum::<usize>()
                );
            }
            "check" => {
                let ids: Vec<String> =
                    bookx_editing::media_overlay_check::check_overlays(&b.archive)
                        .into_iter()
                        .filter(|m| m.id != "MED-011")
                        .map(|m| m.id)
                        .collect();
                eprintln!("editor_mo: check={}", ids.join(","));
            }
            "total" => {
                let opf = b
                    .archive
                    .get_string(&b.package.opf_path)
                    .unwrap_or_default();
                let total = opf
                    .split("<meta property=\"media:duration\">")
                    .nth(1)
                    .and_then(|r| r.split('<').next())
                    .unwrap_or("");
                eprintln!("editor_mo: total={total}");
            }
            other => eprintln!("editor_mo: unknown command {other}"),
        }
    }
}

/// The Tools menu entries.
pub(super) fn menu_section() -> gio::Menu {
    let m = gio::Menu::new();
    for (label, cmd) in [
        (tr("Add Narration"), "narration:add"),
        (tr("Remove Narration"), "narration:remove"),
    ] {
        let it = gio::MenuItem::new(Some(&label), None);
        it.set_action_and_target_value(Some("tools.run"), Some(&cmd.to_variant()));
        m.append_item(&it);
    }
    m
}
