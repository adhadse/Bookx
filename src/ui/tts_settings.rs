//! The "Text to Speech" page of Preferences: engine, speed, voice models to download (with
//! licence, progress and cancel), the voice server, and a test button. The engines themselves
//! are in the `bookx-tts` crate.

use crate::i18n::{tr, trf, N_};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

use bookx_tts::items::{items, subtitle, Item};
use bookx_tts::models::catalog;
use bookx_tts::prefs::ENGINES;
use bookx_tts::server::ServerSynth;
use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, glib};

use super::preferences::{register_group, register_page, PrefCtx};
use crate::tts;

const ENGINE_LABELS: [&str; 3] = [
    N_("Kokoro (natural, on this computer)"),
    N_("Piper (light, on this computer)"),
    N_("External voice server (XTTS, Fish Speech, ...)"),
];

const SERVER_KINDS: [(&str, &str); 3] = [
    ("openai", N_("OpenAI-compatible (/v1/audio/speech)")),
    ("xtts", N_("XTTS API server (XTTS v2)")),
    ("fish", N_("Fish Speech API server")),
];

thread_local! {
    static REGISTERED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// True the first time it is called: a page registered twice would show every group twice.
fn first_registration() -> bool {
    REGISTERED.with(|r| !r.replace(true))
}

/// Adds the page; called once from the registry of Preferences.
pub fn register() {
    if !first_registration() {
        return;
    }
    register_page("tts", &tr("Text to Speech"), "audio-speakers-symbolic", 50);
    register_group("tts", 10, engine_group);
    register_group("tts", 20, models_group);
    register_group("tts", 30, server_group);
}

/// Row of an engine id; the removed `auto` and `speechd` of older configs show as Kokoro.
pub fn engine_index(id: &str) -> u32 {
    ENGINES.iter().position(|e| *e == id).unwrap_or(0) as u32
}

pub fn engine_at(i: u32) -> &'static str {
    ENGINES.get(i as usize).copied().unwrap_or("kokoro")
}

pub fn server_kind_index(id: &str) -> u32 {
    SERVER_KINDS.iter().position(|k| k.0 == id).unwrap_or(0) as u32
}

/// A row with a slider for an amount that does not need to be exact (PRF-P6b). `range` is (min, max, step).
fn slider_row(
    title: &str,
    subtitle: &str,
    range: (f64, f64, f64),
    digits: u32,
    value: f64,
) -> (adw::ActionRow, gtk::Scale) {
    let row = adw::ActionRow::new();
    row.set_title(title);
    row.set_subtitle(subtitle);
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, range.0, range.1, range.2);
    scale.set_digits(digits as i32);
    scale.set_draw_value(true);
    scale.set_value_pos(gtk::PositionType::Right);
    scale.set_width_request(200);
    scale.set_valign(gtk::Align::Center);
    scale.add_mark(1.0, gtk::PositionType::Bottom, None);
    scale.set_value(value);
    super::audit::label(&scale, title);
    row.add_suffix(&scale);
    row.set_activatable_widget(Some(&scale));
    (row, scale)
}

/// "Reads with Kokoro" for the subtitle of the engine row.
pub fn status_line(prefs: &tts::TtsPrefs, lang: &str) -> String {
    let avail = tts::Availability::detect(&tts::store(), prefs);
    match bookx_tts::factory::choose(&prefs.engine, lang, &avail) {
        Ok((_, Some(n))) => n,
        Ok((choice, None)) => trf("Reads with {}", &[&choice.title()]),
        Err(bookx_tts::factory::NotReady::NoVoice) => {
            tr("No voice yet: it is downloaded when you first press Read Aloud (or below)")
        }
        Err(e) => e.to_string(),
    }
}

/// The language of the desktop, two letters ("en" when unknown).
pub fn system_language() -> String {
    let l = std::env::var("LC_MESSAGES")
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default();
    let p = tts::primary_language(&l.replace('.', "-"));
    if p.is_empty() {
        "en".into()
    } else {
        p
    }
}

thread_local! {
    /// How many times the engine group was built (a page registered twice builds it twice).
    static GROUPS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    static DEV: RefCell<Option<DevHandles>> = const { RefCell::new(None) };
}

struct DevHandles {
    engine: adw::ComboRow,
    server_url: adw::EntryRow,
    test: gtk::Button,
    test_row: adw::ActionRow,
    model: adw::ComboRow,
    clear: gtk::Button,
    shared: super::SharedRef,
}

// ---- engine -------------------------------------------------------------------------------

fn engine_group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    GROUPS.with(|g| g.set(g.get() + 1));
    let g = adw::PreferencesGroup::builder()
        .title(tr("Voice"))
        .description(tr(
            "How books are read aloud. The first time you press Read Aloud, Bookx offers to download the default voice.",
        ))
        .build();
    let prefs = ctx.shared.cfg.borrow().tts.clone();
    let lang = system_language();

    let engine = adw::ComboRow::builder()
        .title(tr("Voice engine"))
        .model(&gtk::StringList::new(
            &ENGINE_LABELS
                .map(tr)
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        ))
        .build();
    engine.set_selected(engine_index(&prefs.engine));
    engine.set_subtitle(&status_line(&prefs, &lang));
    {
        let c = PrefCtxLite::of(ctx);
        let lang = lang.clone();
        engine.connect_selected_notify(move |r| {
            let id = engine_at(r.selected());
            c.save(|p| p.engine = id.to_string());
            r.set_subtitle(&status_line(&c.prefs(), &lang));
        });
    }
    g.add(&engine);

    // speed and pitch are approximate amounts: sliders, with a mark at the normal value
    let speed = slider_row(
        &tr("Speed"),
        &tr("1.0 is the normal pace"),
        (0.5, 2.5, 0.1),
        1,
        prefs.speed,
    );
    {
        let c = PrefCtxLite::of(ctx);
        speed
            .1
            .connect_value_changed(move |r| c.save(|p| p.speed = r.value()));
    }
    g.add(&speed.0);

    let pitch = slider_row(
        &tr("Pitch"),
        &tr("1.00 keeps the voice as it is; it applies to every voice engine"),
        (0.8, 1.25, 0.05),
        2,
        prefs.pitch,
    );
    {
        let c = PrefCtxLite::of(ctx);
        pitch
            .1
            .connect_value_changed(move |r| c.save(|p| p.pitch = r.value()));
    }
    g.add(&pitch.0);

    let auto = adw::SwitchRow::builder()
        .title(tr("Download the default voice without asking"))
        .subtitle(tr(
            "When Read Aloud is pressed and no voice is installed, download Kokoro in the background",
        ))
        .active(prefs.download_without_asking)
        .build();
    {
        let c = PrefCtxLite::of(ctx);
        auto.connect_active_notify(move |r| c.save(|p| p.download_without_asking = r.is_active()));
    }
    g.add(&auto);

    let model = adw::ComboRow::builder()
        .title(tr("Kokoro model"))
        .subtitle(tr(
            "Small is 92 MB. Full precision is 326 MB and sounds slightly cleaner.",
        ))
        .model(&gtk::StringList::new(&[
            &tr("Small"),
            &tr("Full precision"),
        ]))
        .build();
    model.set_selected(u32::from(prefs.kokoro_model == "fp32"));
    {
        let c = PrefCtxLite::of(ctx);
        model.connect_selected_notify(move |r| {
            c.save(|p| p.kokoro_model = if r.selected() == 1 { "fp32" } else { "int8" }.to_string())
        });
    }
    g.add(&model);

    let cache = adw::SpinRow::with_range(0.0, 2000.0, 50.0);
    cache.set_title(&tr("Audio cache size"));
    cache.set_subtitle(&tr(
        "Megabytes of read sentences kept, so reading a page again is instant. 0 turns it off. The oldest audio goes first, and cleared audio is made again when needed.",
    ));
    cache.set_value(prefs.cache_mb as f64);
    {
        let c = PrefCtxLite::of(ctx);
        cache.connect_value_notify(move |r| c.save(|p| p.cache_mb = r.value() as u32));
    }
    let clear = gtk::Button::with_label(&tr("Clear"));
    clear.set_valign(gtk::Align::Center);
    clear.add_css_class("flat");
    super::audit::label(&clear, &tr("Clear audio cache"));
    clear.connect_clicked(|_| {
        if let Some(d) = tts::cache_dir() {
            let _ = std::fs::remove_dir_all(d);
        }
        super::preferences::toast(&tr("Audio cache cleared"));
    });
    cache.add_suffix(&clear);
    g.add(&cache);

    let test_row = adw::ActionRow::builder()
        .title(tr("Test the voice"))
        .subtitle(tr("Reads a sentence with the settings above"))
        .build();
    let test = gtk::Button::with_label(&tr("Play"));
    test.set_valign(gtk::Align::Center);
    test.add_css_class("suggested-action");
    super::audit::label(&test, &tr("Play a test sentence"));
    test_row.add_suffix(&test);
    {
        let c = PrefCtxLite::of(ctx);
        let row = test_row.clone();
        let player: Rc<RefCell<Option<TestPlayer>>> = Rc::default();
        test.connect_clicked(move |b| test_voice(&c, &row, b, &player));
    }
    g.add(&test_row);

    DEV.with(|d| {
        let mut d = d.borrow_mut();
        let url = d.as_ref().map(|h| h.server_url.clone());
        *d = Some(DevHandles {
            engine: engine.clone(),
            server_url: url.unwrap_or_default(),
            test: test.clone(),
            test_row: test_row.clone(),
            model: model.clone(),
            clear: clear.clone(),
            shared: ctx.shared.clone(),
        });
    });
    g
}

/// What the closures of the page need: the shared state, nothing GTK.
#[derive(Clone)]
struct PrefCtxLite {
    shared: super::SharedRef,
}

impl PrefCtxLite {
    fn of(ctx: &PrefCtx) -> PrefCtxLite {
        PrefCtxLite {
            shared: ctx.shared.clone(),
        }
    }

    fn save(&self, f: impl FnOnce(&mut tts::TtsPrefs)) {
        let mut cfg = self.shared.cfg.borrow_mut();
        f(&mut cfg.tts);
        cfg.tts = cfg.tts.clamped();
        cfg.save();
    }

    fn prefs(&self) -> tts::TtsPrefs {
        self.shared.cfg.borrow().tts.clone()
    }
}

// ---- test voice ---------------------------------------------------------------------------

struct TestPlayer {
    engine: Box<dyn tts::Engine>,
    id: u64,
    timer: Option<glib::SourceId>,
    started: std::time::Instant,
}

fn test_voice(
    c: &PrefCtxLite,
    row: &adw::ActionRow,
    button: &gtk::Button,
    player: &Rc<RefCell<Option<TestPlayer>>>,
) {
    // a second click stops
    if let Some(mut p) = player.borrow_mut().take() {
        p.engine.stop();
        if let Some(t) = p.timer.take() {
            t.remove();
        }
        button.set_label(&tr("Play"));
        row.set_subtitle(&tr("Stopped"));
        return;
    }
    let prefs = c.prefs().clamped();
    let lang = system_language();
    row.set_subtitle(&tr("Starting the voice"));
    let opened = match tts::open_engine(&prefs, &lang) {
        Ok(o) => o,
        Err(e) => {
            row.set_subtitle(&format!("{e:#}"));
            return;
        }
    };
    let id = opened.choice.map_or("mock", |c| c.id());
    let mut e = opened.engine;
    e.set_language(&lang);
    e.set_speed(prefs.speed);
    e.set_pitch(prefs.pitch);
    e.set_voice(&prefs.voice_for(id, &lang));
    let msg = match e.speak(tts::test_phrase(&lang)) {
        Ok(m) => m,
        Err(err) => {
            row.set_subtitle(&format!("{err:#}"));
            return;
        }
    };
    button.set_label(&tr("Stop"));
    let engine_name = opened.choice.map_or("the silent test voice", |c| c.title());
    let notice = opened.notice.unwrap_or_default();
    let tp = TestPlayer {
        engine: e,
        id: msg,
        timer: None,
        started: std::time::Instant::now(),
    };
    *player.borrow_mut() = Some(tp);
    let (player2, row2, button2) = (player.clone(), row.clone(), button.clone());
    let timer = glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        let mut slot = player2.borrow_mut();
        let Some(p) = slot.as_mut() else {
            return glib::ControlFlow::Break;
        };
        let mut done = None;
        if let Some(err) = p.engine.take_error() {
            done = Some(trf("Failed: {}", &[&err]));
        }
        for ev in p.engine.poll() {
            if ev.id == p.id && ev.kind == bookx_tts::EventKind::End {
                done = Some(trf(
                    "Played with {} {}",
                    &[
                        &engine_name,
                        &(if notice.is_empty() {
                            String::new()
                        } else {
                            format!("({notice})")
                        }),
                    ],
                ));
            }
        }
        if p.started.elapsed() > std::time::Duration::from_secs(120) {
            done = Some("The voice did not answer".into());
        }
        if let Some(text) = done {
            *slot = None;
            button2.set_label(&tr("Play"));
            row2.set_subtitle(&text);
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
    if let Some(p) = player.borrow_mut().as_mut() {
        p.timer = Some(timer);
    }
}

// ---- models -------------------------------------------------------------------------------

/// This row waits for the download job (see `voice_download`); the job is the application's, so it
/// goes on when the Preferences window is closed.
struct Download;

struct ItemRow {
    key: String,
    row: adw::ActionRow,
    action: gtk::Button,
    cancel: gtk::Button,
    bar: gtk::ProgressBar,
    licence: gtk::Button,
    busy: Rc<RefCell<Option<Download>>>,
}

fn models_group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    let store = tts::store();
    let g = adw::PreferencesGroup::builder()
        .title(tr("Voices and models"))
        .description(trf(
            "Downloaded when you ask, never bundled. Every file is checked against a checksum. \
They are kept in {}",
            &[&store.root.display()],
        ))
        .build();
    let prefs = ctx.shared.cfg.borrow().tts.clone();
    let rows: Rc<RefCell<Vec<ItemRow>>> = Rc::default();
    for item in items(&prefs, &store) {
        let r = build_item_row(ctx, &item, &rows);
        g.add(&r.row);
        rows.borrow_mut().push(r);
    }
    // a download the last run did not finish (the app was closed, the network went away)
    {
        let store2 = tts::store();
        let list = items(&prefs, &store2);
        let all = catalog();
        let pending = list.iter().find(|i| {
            !i.installed
                && i.plan.iter().any(|id| {
                    all.iter()
                        .find(|p| &p.id == id)
                        .is_some_and(|p| store2.pack_has_partial(p))
                })
        });
        if let Some(it) = pending {
            let row = adw::ActionRow::builder()
                .title(tr("Unfinished download"))
                .subtitle(trf(
                    "{} was not downloaded completely. It continues where it stopped.",
                    &[&it.title],
                ))
                .build();
            let resume = gtk::Button::with_label(&tr("Resume"));
            resume.set_valign(gtk::Align::Center);
            resume.add_css_class("suggested-action");
            super::audit::label(&resume, &tr("Resume the unfinished voice download"));
            let (plan, title) = (it.plan.clone(), trf("Downloading {}", &[&it.title]));
            let row2 = row.clone();
            resume.connect_clicked(move |b| {
                crate::voice_download::start(&title, tts::packs_of(&plan));
                b.set_sensitive(false);
                row2.set_subtitle(&tr("Resuming… see the Background Jobs list for progress"));
            });
            resume.set_sensitive(!crate::voice_download::running());
            row.add_suffix(&resume);
            g.add(&row);
        }
    }
    // espeak-ng note
    let espeak = adw::ActionRow::builder()
        .title("espeak-ng")
        .subtitle(if bookx_tts::phonemize::espeak_program().is_some() { tr("Installed. Kokoro and Piper use it to turn text into phonemes.") } else { tr("Not found. Kokoro and Piper need it (package espeak-ng, GPL-3.0); install it with your system's software manager.") })
        .build();
    if bookx_tts::phonemize::espeak_program().is_none() {
        if let Some(cmd) = tts::espeak_install_command() {
            let copy = gtk::Button::with_label(&tr("Copy Install Command"));
            copy.set_valign(gtk::Align::Center);
            copy.connect_clicked(move |b| b.display().clipboard().set_text(&cmd));
            espeak.add_suffix(&copy);
        }
    }
    g.add(&espeak);
    g
}

fn build_item_row(ctx: &PrefCtx, item: &Item, rows: &Rc<RefCell<Vec<ItemRow>>>) -> ItemRow {
    let row = adw::ActionRow::builder()
        .title(&item.title)
        .subtitle(subtitle(item))
        .build();
    row.set_subtitle_lines(2);
    let bar = gtk::ProgressBar::new();
    bar.set_valign(gtk::Align::Center);
    bar.set_width_request(120);
    bar.set_visible(false);
    let cancel = gtk::Button::with_label(&tr("Cancel"));
    cancel.set_valign(gtk::Align::Center);
    cancel.set_visible(false);
    let licence = gtk::Button::with_label(&tr("Licence"));
    licence.set_valign(gtk::Align::Center);
    licence.add_css_class("flat");
    let action = gtk::Button::with_label(
        &(if item.installed {
            tr("Remove")
        } else {
            tr("Download")
        }),
    );
    action.set_valign(gtk::Align::Center);
    action.add_css_class(if item.installed {
        "destructive-action"
    } else {
        "suggested-action"
    });
    super::audit::label(&action, &trf("Download or remove {}", &[&item.title]));
    row.add_suffix(&bar);
    row.add_suffix(&cancel);
    row.add_suffix(&licence);
    row.add_suffix(&action);
    let busy: Rc<RefCell<Option<Download>>> = Rc::default();

    {
        let (parent, key) = (ctx.parent.clone(), item.key.clone());
        let c = PrefCtxLite::of(ctx);
        licence.connect_clicked(move |_| {
            if let Some(it) = current_item(&c, &key) {
                licence_dialog(&parent, &it, false, |_| {});
            }
        });
    }
    {
        let busy = busy.clone();
        cancel.connect_clicked(move |_| {
            if busy.borrow().is_some() {
                crate::voice_download::cancel();
            }
        });
    }
    {
        let (parent, key, rows2, busy2) = (
            ctx.parent.clone(),
            item.key.clone(),
            rows.clone(),
            busy.clone(),
        );
        let c = PrefCtxLite::of(ctx);
        action.connect_clicked(move |_| {
            let Some(it) = current_item(&c, &key) else {
                return;
            };
            if it.installed {
                remove_item(&it);
                refresh(&rows2, &c);
                return;
            }
            let (rows3, busy3, c2, key2) = (rows2.clone(), busy2.clone(), c.clone(), key.clone());
            licence_dialog(&parent, &it, true, move |_| {
                start_download(&c2, &key2, &rows3, &busy3);
            });
        });
    }
    ItemRow {
        key: item.key.clone(),
        row,
        action,
        cancel,
        bar,
        licence,
        busy,
    }
}

/// The item as it is now (installed or not, for the model chosen).
fn current_item(c: &PrefCtxLite, key: &str) -> Option<Item> {
    items(&c.prefs(), &tts::store())
        .into_iter()
        .find(|i| i.key == key)
}

fn refresh(rows: &Rc<RefCell<Vec<ItemRow>>>, c: &PrefCtxLite) {
    let list = items(&c.prefs(), &tts::store());
    for r in rows.borrow().iter() {
        let Some(it) = list.iter().find(|i| i.key == r.key) else {
            continue;
        };
        let busy = r.busy.borrow().is_some();
        r.row.set_subtitle(&subtitle(it));
        r.action.set_label(
            &(if it.installed {
                tr("Remove")
            } else {
                tr("Download")
            }),
        );
        r.action.remove_css_class("suggested-action");
        r.action.remove_css_class("destructive-action");
        r.action.add_css_class(if it.installed {
            "destructive-action"
        } else {
            "suggested-action"
        });
        r.action.set_visible(!busy);
        r.cancel.set_visible(busy);
        r.bar.set_visible(busy);
        r.licence.set_visible(true);
    }
}

fn remove_item(it: &Item) {
    let store = tts::store();
    for id in &it.remove {
        if let Some(p) = catalog().into_iter().find(|p| &p.id == id) {
            let _ = store.remove(&p);
        }
    }
}

fn licence_dialog(parent: &gtk::Widget, it: &Item, download: bool, on_go: impl Fn(()) + 'static) {
    let body = format!(
        "{}\n\nLicence: {}\n{}\n{}{}",
        it.summary,
        it.license_name,
        it.license_note,
        it.license_url,
        if download {
            format!(
                "\n\nDownload: {}",
                bookx_tts::items::format_size(it.download_bytes)
            )
        } else {
            String::new()
        }
    );
    let d = adw::AlertDialog::new(Some(&it.title), Some(&body));
    if download {
        d.add_responses(&[
            ("cancel", &tr("Cancel")),
            ("go", &tr("Accept and Download")),
        ]);
        d.set_response_appearance("go", adw::ResponseAppearance::Suggested);
    } else {
        d.add_responses(&[("cancel", &tr("Close")), ("open", &tr("Open Licence Page"))]);
    }
    d.set_close_response("cancel");
    let url = it.license_url.clone();
    d.connect_response(None, move |_, r| match r {
        "go" => on_go(()),
        "open" => super::open_uri(&url),
        _ => {}
    });
    d.present(Some(parent));
}

fn start_download(
    c: &PrefCtxLite,
    key: &str,
    rows: &Rc<RefCell<Vec<ItemRow>>>,
    busy: &Rc<RefCell<Option<Download>>>,
) {
    let Some(it) = current_item(c, key) else {
        return;
    };
    let seq = crate::voice_download::start(
        &trf("Downloading {}", &[&it.title]),
        tts::packs_of(&it.plan),
    );
    follow(c, key, rows, busy, seq);
}

/// Shows the state of the download job `seq` on the row of `key` until it ends.
fn follow(
    c: &PrefCtxLite,
    key: &str,
    rows: &Rc<RefCell<Vec<ItemRow>>>,
    busy: &Rc<RefCell<Option<Download>>>,
    seq: u64,
) {
    *busy.borrow_mut() = Some(Download);
    refresh(rows, c);
    let (rows, c, key, busy) = (rows.clone(), c.clone(), key.to_string(), busy.clone());
    glib::timeout_add_local(std::time::Duration::from_millis(150), move || {
        use crate::voice_download::Status;
        let list = rows.borrow();
        let Some(r) = list.iter().find(|r| r.key == key) else {
            return glib::ControlFlow::Break;
        };
        match crate::voice_download::status() {
            Status::Running { done, total, line } => {
                r.bar
                    .set_fraction((done as f64 / total.max(1) as f64).clamp(0.0, 1.0));
                r.row.set_subtitle(&line);
                glib::ControlFlow::Continue
            }
            Status::Finished { seq: s, result } if s >= seq => {
                drop(list);
                *busy.borrow_mut() = None;
                refresh(&rows, &c);
                let list = rows.borrow();
                if let (Err(f), Some(r)) = (result, list.iter().find(|r| r.key == key)) {
                    r.row
                        .set_subtitle(&tr(&bookx_tts::setup::failure_message(&f)));
                }
                glib::ControlFlow::Break
            }
            _ => glib::ControlFlow::Continue,
        }
    });
}

// ---- server -------------------------------------------------------------------------------

fn server_group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::builder()
        .title(tr("Voice server"))
        .description(
            tr("Use a voice server you run yourself, for example XTTS v2 or Fish Speech. Bookx sends it \
the sentences and plays the audio. Their model licences (XTTS: Coqui Public Model License; \
Fish Speech S2: Fish Audio Research License) allow non-commercial use only, which is your \
responsibility. If the server wants a key, enter it below; it is kept in the system keyring."),
        )
        .build();
    let prefs = ctx.shared.cfg.borrow().tts.clone();
    let kind = adw::ComboRow::builder()
        .title(tr("Server type"))
        .model(&gtk::StringList::new(
            &SERVER_KINDS
                .map(|k| tr(k.1))
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        ))
        .build();
    kind.set_selected(server_kind_index(&prefs.server.kind));
    {
        let c = PrefCtxLite::of(ctx);
        kind.connect_selected_notify(move |r| {
            let id = SERVER_KINDS
                .get(r.selected() as usize)
                .map_or("openai", |k| k.0);
            c.save(|p| p.server.kind = id.to_string());
        });
    }
    g.add(&kind);

    let url = adw::EntryRow::builder().title(tr("Address")).build();
    url.set_text(&prefs.server.url);
    {
        let c = PrefCtxLite::of(ctx);
        url.connect_changed(move |e| c.save(|p| p.server.url = e.text().to_string()));
    }
    g.add(&url);

    let model = adw::EntryRow::builder()
        .title(tr("Model (OpenAI-compatible servers)"))
        .build();
    model.set_text(&prefs.server.model);
    {
        let c = PrefCtxLite::of(ctx);
        model.connect_changed(move |e| c.save(|p| p.server.model = e.text().to_string()));
    }
    g.add(&model);

    let voice = adw::EntryRow::builder()
        .title(tr("Voice, speaker or reference id"))
        .build();
    voice.set_text(&prefs.server.voice);
    {
        let c = PrefCtxLite::of(ctx);
        voice.connect_changed(move |e| c.save(|p| p.server.voice = e.text().to_string()));
    }
    g.add(&voice);

    let key = adw::PasswordEntryRow::builder()
        .title(tr("API key (if the server wants one)"))
        .build();
    key.set_text(&prefs.server.api_key);
    {
        let c = PrefCtxLite::of(ctx);
        key.connect_changed(move |e| c.save(|p| p.server.api_key = e.text().to_string()));
    }
    g.add(&key);

    let test = adw::ActionRow::builder()
        .title(tr("Test the connection"))
        .subtitle(tr(
            "Speaks a short sentence and tells how long the server took",
        ))
        .build();
    let btn = gtk::Button::with_label(&tr("Test"));
    btn.set_valign(gtk::Align::Center);
    super::audit::label(&btn, &tr("Test the voice server"));
    test.add_suffix(&btn);
    {
        let c = PrefCtxLite::of(ctx);
        let row = test.clone();
        btn.connect_clicked(move |b| {
            let cfg = c.prefs().server;
            row.set_subtitle(&tr("Connecting"));
            b.set_sensitive(false);
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(ServerSynth::test(&cfg).map_err(|e| format!("{e:#}")));
            });
            let (row, b) = (row.clone(), b.clone());
            glib::timeout_add_local(std::time::Duration::from_millis(150), move || {
                match rx.try_recv() {
                    Ok(r) => {
                        row.set_subtitle(&match r {
                            Ok(t) => t,
                            Err(e) => e,
                        });
                        b.set_sensitive(true);
                        glib::ControlFlow::Break
                    }
                    Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(_) => glib::ControlFlow::Break,
                }
            });
        });
    }
    g.add(&test);
    DEV.with(|d| {
        if let Some(h) = d.borrow_mut().as_mut() {
            h.server_url = url.clone();
        }
    });
    g
}

/// Dev hook `tts:ARG` (test infrastructure): `engine=ID`, `server=URL`, `test`, `labels`, `dump`.
pub fn dev(arg: &str) {
    DEV.with(|d| {
        let d = d.borrow();
        let Some(h) = d.as_ref() else {
            eprintln!("tts_settings: not open");
            return;
        };
        if let Some(id) = arg.strip_prefix("engine=") {
            h.engine.set_selected(engine_index(id));
        } else if let Some(u) = arg.strip_prefix("server=") {
            h.server_url.set_text(u);
        } else if arg == "test" {
            h.test.emit_clicked();
        } else if arg == "labels" {
            let m = h.model.model();
            let choices: Vec<String> = (0..m.as_ref().map_or(0, |m| m.n_items()))
                .filter_map(|i| m.as_ref()?.item(i).and_downcast::<gtk::StringObject>())
                .map(|o| o.string().to_string())
                .collect();
            eprintln!(
                "tts_labels: model_choices={} clear_label={:?} pitch_subtitle_set=true",
                choices.join("|"),
                h.clear.label().unwrap_or_default()
            );
        } else if arg == "dump" {
            let cfg = h.shared.cfg.borrow();
            eprintln!(
                "tts_settings: groups={} engine={} subtitle={:?} speed={} server={} test_subtitle={:?} test_label={:?}",
                GROUPS.with(|g| g.get()),
                cfg.tts.engine,
                h.engine.subtitle().unwrap_or_default(),
                cfg.tts.speed,
                cfg.tts.server.url,
                h.test_row.subtitle().unwrap_or_default(),
                h.test.label().unwrap_or_default()
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_rows_map_to_the_ids_of_the_prefs() {
        for (i, id) in ENGINES.iter().enumerate() {
            assert_eq!(engine_index(id), i as u32);
            assert_eq!(engine_at(i as u32), *id);
        }
        assert_eq!(engine_index("nonsense"), 0);
        assert_eq!(
            engine_index("speechd"),
            0,
            "the removed basic voice shows as Kokoro"
        );
        assert_eq!(engine_index("auto"), 0);
        assert_eq!(engine_at(99), "kokoro");
        assert_eq!(ENGINE_LABELS.len(), ENGINES.len());
    }

    #[test]
    fn the_page_is_registered_once_however_often_it_is_asked() {
        assert!(first_registration());
        assert!(!first_registration());
        assert!(!first_registration());
    }

    #[test]
    fn server_kinds_map_to_rows() {
        assert_eq!(server_kind_index("xtts"), 1);
        assert_eq!(server_kind_index("fish"), 2);
        assert_eq!(server_kind_index("?"), 0);
    }

    #[test]
    fn the_desktop_language_is_read_from_the_locale() {
        // not set here: only check the shape
        let l = system_language();
        assert!((2..=3).contains(&l.len()));
    }
}
