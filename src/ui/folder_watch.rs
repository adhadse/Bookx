//! The import folder watcher (roadmap L12): while Bookx runs, new books that settle in the watched
//! folder are added by themselves. Off unless the user switches it on in Library Health. Uses
//! `gio::FileMonitor` on the folder and its sub-folders (no extra dependency) and a slow timer for
//! folders the monitor cannot see into.

use crate::i18n::{ntr, tr};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use relm4::adw::{self, prelude::*};
use relm4::gtk::{gio, glib};

use super::SharedRef;
use crate::core::inbox::{self, InboxState};

/// A monitor event is followed by a check after this long (a copy needs to settle first).
const DEBOUNCE: Duration = inbox::SETTLE;
/// Folders deeper than this are not monitored (the timer still sees their files).
const MAX_MONITORED_DIRS: usize = 200;
const RESCAN: Duration = Duration::from_secs(60);

type Import = Rc<dyn Fn(Vec<PathBuf>)>;

struct Watcher {
    monitors: Vec<gio::FileMonitor>,
    pending: Option<glib::SourceId>,
    timer: Option<glib::SourceId>,
}

thread_local! {
    static WATCHER: RefCell<Option<Watcher>> = const { RefCell::new(None) };
    static IMPORT: RefCell<Option<(SharedRef, Import, Rc<dyn Fn(String)>)>> = const { RefCell::new(None) };
}

/// Starts watching when the library asks for it. `import` adds files through the normal import
/// path (so duplicates, conversions and the Jobs panel work as always); `toast` tells the user.
pub fn start(shared: &SharedRef, import: Import, toast: Rc<dyn Fn(String)>) {
    IMPORT.with(|i| *i.borrow_mut() = Some((shared.clone(), import, toast)));
    refresh();
}

/// Re-reads the setting and the folder (after a change in Library Health).
pub fn refresh() {
    WATCHER.with(|w| {
        if let Some(old) = w.borrow_mut().take() {
            for m in &old.monitors {
                m.cancel();
            }
            if let Some(s) = old.pending {
                s.remove();
            }
            if let Some(s) = old.timer {
                s.remove();
            }
        }
    });
    let Some((shared, _, _)) = IMPORT.with(|i| i.borrow().clone()) else {
        return;
    };
    if shared.lib.borrow().is_read_only() {
        return;
    }
    let st = InboxState::load(&shared.lib.borrow().root);
    let (true, Some(folder)) = (st.watch, st.folder.clone()) else {
        return;
    };
    let mut monitors = Vec::new();
    for dir in dirs(&folder) {
        // no inotify left (the per-user limit is shared with every other program): the timed
        // rescan below still finds new books
        if let Ok(m) = gio::File::for_path(&dir)
            .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
        {
            m.connect_changed(|_, _, _, ev| {
                use gio::FileMonitorEvent::*;
                if matches!(ev, Created | ChangesDoneHint | MovedIn | Renamed) {
                    schedule();
                }
            });
            monitors.push(m);
        }
    }
    let timer = glib::timeout_add_local(RESCAN, || {
        check_now();
        glib::ControlFlow::Continue
    });
    WATCHER.with(|w| {
        *w.borrow_mut() = Some(Watcher {
            monitors,
            pending: None,
            timer: Some(timer),
        })
    });
    // what was already there when the watcher started is picked up too
    schedule();
}

/// The folder and its sub-folders (hidden ones skipped), at most [`MAX_MONITORED_DIRS`].
fn dirs(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![root.to_path_buf()];
    let mut i = 0;
    while i < out.len() && out.len() < MAX_MONITORED_DIRS {
        if let Ok(rd) = std::fs::read_dir(&out[i]) {
            let mut sub: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.is_dir()
                        && !p
                            .file_name()
                            .is_some_and(|n| n.to_string_lossy().starts_with('.'))
                })
                .collect();
            sub.sort();
            for s in sub {
                if out.len() < MAX_MONITORED_DIRS {
                    out.push(s);
                }
            }
        }
        i += 1;
    }
    out
}

fn schedule() {
    WATCHER.with(|w| {
        let mut w = w.borrow_mut();
        let Some(w) = w.as_mut() else { return };
        if w.pending.is_some() {
            return;
        }
        w.pending = Some(glib::timeout_add_local_once(
            DEBOUNCE + Duration::from_secs(1),
            || {
                WATCHER.with(|w| {
                    if let Some(w) = w.borrow_mut().as_mut() {
                        w.pending = None;
                    }
                });
                check_now();
            },
        ));
    });
}

/// The settled new files of the folder, each once.
pub fn new_files(root: &Path, st: &InboxState, now: SystemTime) -> Vec<(PathBuf, String)> {
    let Some(folder) = &st.folder else {
        return Vec::new();
    };
    let mut files = Vec::new();
    crate::import::collect(folder, true, &mut files);
    inbox::pending_files(st, &files, now, inbox::SETTLE)
        .into_iter()
        .filter(|(p, _)| !p.starts_with(root.join(crate::core::history::STATE_DIR)))
        .collect()
}

fn check_now() {
    let Some((shared, import, toast)) = IMPORT.with(|i| i.borrow().clone()) else {
        return;
    };
    let root = shared.lib.borrow().root.clone();
    let mut st = InboxState::load(&root);
    if !st.watch {
        return;
    }
    let found = new_files(&root, &st, SystemTime::now());
    if found.is_empty() {
        return;
    }
    // handled now, so a failing file does not come back every minute; the import reports it
    inbox::mark_done(&mut st, found.iter().map(|f| f.1.clone()));
    if st.save(&root).is_err() {
        return;
    }
    toast(ntr(
        "Adding {} new book from the import folder",
        "Adding {} new books from the import folder",
        found.len(),
    ));
    import(found.into_iter().map(|f| f.0).collect());
}

/// The switch of Library Health.
pub fn switch_row(shared: &SharedRef) -> adw::SwitchRow {
    let row = adw::SwitchRow::builder()
        .title(tr("Add new books automatically"))
        .subtitle(tr(
            "While Bookx runs, books that appear in the watched folder are added without asking",
        ))
        .build();
    row.set_widget_name("inbox-watch");
    row.set_active(InboxState::load(&shared.lib.borrow().root).watch);
    let shared = shared.clone();
    row.connect_active_notify(move |s| {
        let root = shared.lib.borrow().root.clone();
        let mut st = InboxState::load(&root);
        st.watch = s.is_active();
        let _ = st.save(&root);
        refresh();
    });
    row
}

/// Test hook: `library:inboxwatch=on|off|check|FOLDER` (a folder sets the watched folder).
pub fn dev(shared: &SharedRef, cmd: &str) -> String {
    let root = shared.lib.borrow().root.clone();
    let mut st = InboxState::load(&root);
    match cmd {
        "on" | "off" => {
            st.watch = cmd == "on";
            let _ = st.save(&root);
            refresh();
        }
        "check" => check_now(),
        folder => {
            st.folder = Some(PathBuf::from(folder));
            let _ = st.save(&root);
            refresh();
        }
    }
    let st = InboxState::load(&root);
    format!(
        "inboxwatch: watch={} folder={} monitors={} handled={} polling={}",
        st.watch,
        st.folder
            .as_ref()
            .map(|f| f.display().to_string())
            .unwrap_or_default(),
        WATCHER.with(|w| w.borrow().as_ref().map_or(0, |w| w.monitors.len())),
        st.done.len(),
        WATCHER.with(|w| w.borrow().as_ref().is_some_and(|w| w.monitors.is_empty()))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folder_and_its_subfolders_are_watched_but_not_hidden_ones() {
        let d = std::env::temp_dir().join(format!("bookx-watch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(d.join("a/b")).unwrap();
        std::fs::create_dir_all(d.join(".hidden")).unwrap();
        let v = dirs(&d);
        assert_eq!(v, vec![d.clone(), d.join("a"), d.join("a/b")]);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn only_settled_unhandled_book_files_count() {
        let d = std::env::temp_dir().join(format!("bookx-watch2-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("a.epub"), b"x").unwrap();
        std::fs::write(d.join("sub/b.txt"), b"x").unwrap();
        std::fs::write(d.join("note.bin"), b"x").unwrap();
        let st = InboxState {
            folder: Some(d.clone()),
            watch: true,
            ..Default::default()
        };
        let later = SystemTime::now() + Duration::from_secs(60);
        let found = new_files(Path::new("/nonexistent-root"), &st, later);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(
            new_files(Path::new("/x"), &st, SystemTime::now()).is_empty(),
            "not settled"
        );
        let _ = std::fs::remove_dir_all(d);
    }
}
