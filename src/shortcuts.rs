//! Keyboard shortcuts you can change, without GTK: the registry of every action with its default
//! accelerators, the user's overrides, conflict detection, the JSON file (also used to export and
//! import) and the lookup the reader's key handler uses.
//!
//! The registry is the command palette's (`palette::COMMANDS`) plus a few actions that only have
//! a key (`app.palette`, `book.edit-meta`, `book.delete`). The GTK side (`ui/shortcuts_dialog.rs`)
//! shows the Preferences page and applies the result to the application accelerators.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use serde::{Deserialize, Serialize};

use crate::palette::{self, Scope};

/// Which window an action lives in. Application actions work in both and so collide with both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Application,
    Library,
    Reader,
    /// The book editor window (CE-403): its keys never meet the library's or the reader's.
    Editor,
}

impl Group {
    pub fn title(self) -> &'static str {
        match self {
            Group::Application => crate::i18n::N_("Application"),
            Group::Library => crate::i18n::N_("Library"),
            Group::Reader => crate::i18n::N_("Reader"),
            Group::Editor => crate::i18n::N_("Editor"),
        }
    }

    /// The title in the language of the interface.
    pub fn label(self) -> String {
        crate::i18n::tr(self.title())
    }

    /// True when two actions of these groups can be triggered by the same key press.
    pub fn overlaps(self, other: Group) -> bool {
        if self == Group::Editor || other == Group::Editor {
            return self == other;
        }
        self == other || self == Group::Application || other == Group::Application
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub action: String,
    pub title: String,
    pub group: Group,
    pub defaults: Vec<String>,
    pub keywords: String,
}

/// Actions that have a key but are not in the command palette (or have more than one default).
const EXTRA: &[(&str, &str, Group, &[&str])] = &[
    (
        "app.palette",
        crate::i18n::N_("Command Palette"),
        Group::Application,
        &["<Control><Shift>p", "<Control>k"],
    ),
    (
        "book.edit-meta",
        crate::i18n::N_("Edit Metadata"),
        Group::Library,
        &["<Control>e"],
    ),
    (
        "book.delete",
        crate::i18n::N_("Remove Selected Books"),
        Group::Library,
        &["Delete"],
    ),
    (
        "book.edit-book",
        crate::i18n::N_("Edit Book"),
        Group::Library,
        &["<Control><Shift>e"],
    ),
    (
        "book.properties",
        crate::i18n::N_("Book Properties"),
        Group::Library,
        &["<Control>i"],
    ),
];

/// Second and third default keys of actions whose palette entry has one accelerator. Most come
/// from other readers people know (Calibre's viewer, Foliate, KOReader, Okular; see
/// `docs/shortcuts-research.md`). The built-in key handling of the reader already does the first
/// group (page turns); the others run through [`Shortcuts::default_reader_action`].
const MORE_DEFAULTS: &[(&str, &[&str])] = &[
    ("reader.next-page", &["space", "Page_Down"]),
    ("reader.prev-page", &["<Shift>space", "Page_Up"]),
    ("reader.contents", &["<Control>t"]),
    ("reader.notes", &["<Control>h"]),
    ("reader.back", &["<Control>w"]),
    ("reader.larger", &["<Control>equal"]),
];

/// Every action that can have a shortcut, in a stable order: application, library, reader.
pub fn registry() -> Vec<Action> {
    let mut out: Vec<Action> = Vec::new();
    for c in palette::COMMANDS {
        let group = match c.scope {
            None => Group::Application,
            Some(Scope::Library) => Group::Library,
            Some(Scope::Reader) => Group::Reader,
        };
        out.push(Action {
            action: c.action.to_string(),
            title: crate::i18n::tr(c.title),
            group,
            defaults: canonical(c.accel)
                .into_iter()
                .chain(
                    MORE_DEFAULTS
                        .iter()
                        .filter(|(a, _)| *a == c.action)
                        .flat_map(|(_, k)| k.iter().filter_map(|k| canonical(k))),
                )
                .collect(),
            keywords: c.keywords.to_string(),
        });
    }
    for (a, t, g, d) in EXTRA {
        if !out.iter().any(|x| x.action == *a) {
            out.push(Action {
                action: a.to_string(),
                title: crate::i18n::tr(t),
                group: *g,
                defaults: d.iter().filter_map(|s| canonical(s)).collect(),
                keywords: String::new(),
            });
        }
    }
    for c in crate::editor_keymap::COMMANDS {
        out.push(Action {
            action: c.id.to_string(),
            title: crate::i18n::tr(c.title),
            group: Group::Editor,
            defaults: c.defaults.iter().filter_map(|k| canonical(k)).collect(),
            keywords: c.keywords.to_string(),
        });
    }
    out.sort_by_key(|a| a.group);
    out
}

// ---- accelerator text ----

const MODS: [(&str, &str); 4] = [
    ("Control", "Control"),
    ("Shift", "Shift"),
    ("Alt", "Alt"),
    ("Super", "Super"),
];

/// A valid accelerator in its one canonical spelling (`<Control><Shift><Alt><Super>key`), or `None`.
/// Primary / Ctrl / Ctl and Meta are accepted as spellings.
pub fn canonical(accel: &str) -> Option<String> {
    let mut rest = accel.trim();
    let mut have = [false; 4];
    while let Some(r) = rest.strip_prefix('<') {
        let (m, tail) = r.split_once('>')?;
        let i = match m.to_ascii_lowercase().as_str() {
            "control" | "primary" | "ctrl" | "ctl" => 0,
            "shift" => 1,
            "alt" | "mod1" => 2,
            "super" | "meta" | "mod4" => 3,
            _ => return None,
        };
        have[i] = true;
        rest = tail;
    }
    if rest.is_empty() || !rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    let key = if rest.chars().count() == 1 {
        rest.to_ascii_lowercase()
    } else {
        rest.to_string()
    };
    // a lone modifier is not a shortcut
    if matches!(
        key.as_str(),
        "Shift_L"
            | "Shift_R"
            | "Control_L"
            | "Control_R"
            | "Alt_L"
            | "Alt_R"
            | "Super_L"
            | "Super_R"
    ) {
        return None;
    }
    let mut out = String::new();
    for (i, (m, _)) in MODS.iter().enumerate() {
        if have[i] {
            out.push('<');
            out.push_str(m);
            out.push('>');
        }
    }
    out.push_str(&key);
    Some(out)
}

/// The accelerator for a key press: modifiers as flags, the key's GDK name and its character.
/// Shift is dropped for printable non-letters (Ctrl+Shift+= is the key `plus`, `?` is Shift+/).
pub fn from_press(
    shift: bool,
    ctrl: bool,
    alt: bool,
    sup: bool,
    key_name: &str,
    key_char: Option<char>,
) -> Option<String> {
    let printable_symbol = key_char.is_some_and(|c| !c.is_alphanumeric() && !c.is_whitespace());
    let shift = shift && !printable_symbol;
    let mut s = String::new();
    if ctrl {
        s.push_str("<Control>");
    }
    if shift {
        s.push_str("<Shift>");
    }
    if alt {
        s.push_str("<Alt>");
    }
    if sup {
        s.push_str("<Super>");
    }
    s.push_str(key_name);
    canonical(&s)
}

/// A key on its own that would make typing or navigation unusable if it were taken for a command.
/// (Letters, digits and space without a modifier.)
pub fn is_bare_text_key(accel: &str) -> bool {
    match canonical(accel) {
        Some(c) => !c.starts_with('<') && c.chars().count() == 1 && c != "?",
        None => false,
    }
}

/// "<Control>o" as "Ctrl+O" (see `palette::accel_label`).
pub fn label(accel: &str) -> String {
    palette::accel_label(accel)
}

// ---- the user's choices ----

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shortcuts {
    /// Only actions whose keys differ from the default; an empty list is "no shortcut".
    #[serde(default)]
    pub bindings: BTreeMap<String, Vec<String>>,
}

/// What importing a file did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub applied: usize,
    pub skipped: Vec<String>,
}

/// What a key press means in a window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolve {
    /// A user-chosen binding: run this action.
    Action(String),
    /// A default key of an action that was rebound or cleared: do nothing.
    Disabled,
    /// Not customised: the built-in key handling decides.
    Unbound,
}

/// How to settle a key that another action already has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settle {
    /// The other actions give the key up.
    TakeAway,
    /// The other action gets the key this action had.
    Swap,
}

impl Shortcuts {
    fn find(action: &str) -> Option<Action> {
        registry().into_iter().find(|a| a.action == action)
    }

    pub fn is_customised(&self, action: &str) -> bool {
        self.bindings.contains_key(action)
    }

    pub fn any_customised(&self) -> bool {
        !self.bindings.is_empty()
    }

    /// The keys an action answers to now.
    pub fn effective(&self, action: &str) -> Vec<String> {
        if let Some(v) = self.bindings.get(action) {
            return v.clone();
        }
        Self::find(action).map(|a| a.defaults).unwrap_or_default()
    }

    /// "Ctrl+O, Ctrl+K" for menus, the palette and the shortcuts window.
    pub fn display(&self, action: &str) -> String {
        self.effective(action)
            .iter()
            .map(|a| label(a))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn store(&mut self, action: &str, keys: Vec<String>) {
        let default = Self::find(action).map(|a| a.defaults).unwrap_or_default();
        if keys == default {
            self.bindings.remove(action);
        } else {
            self.bindings.insert(action.to_string(), keys);
        }
    }

    /// Other actions that answer to `accel` where `action` is also active.
    pub fn conflicts(&self, action: &str, accel: &str) -> Vec<Action> {
        let Some(accel) = canonical(accel) else {
            return vec![];
        };
        let Some(me) = Self::find(action) else {
            return vec![];
        };
        registry()
            .into_iter()
            .filter(|o| o.action != action && o.group.overlaps(me.group))
            .filter(|o| self.effective(&o.action).contains(&accel))
            .collect()
    }

    /// Gives `action` the key `accel` as its only shortcut. Fails with the actions that already use
    /// it (nothing changes); settle with [`Shortcuts::bind_settled`].
    pub fn bind(&mut self, action: &str, accel: &str) -> Result<(), Vec<Action>> {
        let accel = canonical(accel).ok_or_else(Vec::new)?;
        let c = self.conflicts(action, &accel);
        if !c.is_empty() {
            return Err(c);
        }
        self.store(action, vec![accel]);
        Ok(())
    }

    /// Like [`Shortcuts::bind`] but settles a conflict: take the key away from the others or swap.
    pub fn bind_settled(&mut self, action: &str, accel: &str, how: Settle) -> bool {
        let Some(accel) = canonical(accel) else {
            return false;
        };
        let before = self.effective(action);
        for other in self.conflicts(action, &accel) {
            let mut keys = self.effective(&other.action);
            keys.retain(|k| *k != accel);
            if how == Settle::Swap {
                if let Some(old) = before.first() {
                    if !keys.contains(old) {
                        keys.push(old.clone());
                    }
                }
            }
            self.store(&other.action, keys);
        }
        self.store(action, vec![accel]);
        true
    }

    /// No shortcut for this action.
    pub fn clear(&mut self, action: &str) {
        self.store(action, vec![]);
    }

    pub fn reset(&mut self, action: &str) {
        self.bindings.remove(action);
    }

    pub fn reset_all(&mut self) {
        self.bindings.clear();
    }

    /// Pairs of actions that share a key in a window (possible after an import).
    pub fn all_conflicts(&self) -> Vec<(String, String, String)> {
        let reg = registry();
        let mut out = Vec::new();
        for (i, a) in reg.iter().enumerate() {
            for b in &reg[i + 1..] {
                if !a.group.overlaps(b.group) {
                    continue;
                }
                for k in self.effective(&a.action) {
                    if self.effective(&b.action).contains(&k) {
                        out.push((a.action.clone(), b.action.clone(), k));
                    }
                }
            }
        }
        out
    }

    /// What a key press means in a window of `group` (reader key handling).
    pub fn resolve(&self, group: Group, accel: &str) -> Resolve {
        let Some(accel) = canonical(accel) else {
            return Resolve::Unbound;
        };
        if self.bindings.is_empty() {
            return Resolve::Unbound;
        }
        let reg = registry();
        let active = |a: &Action| a.group == group || a.group == Group::Application;
        for a in reg.iter().filter(|a| active(a)) {
            if self.is_customised(&a.action) && self.effective(&a.action).contains(&accel) {
                return Resolve::Action(a.action.clone());
            }
        }
        for a in reg.iter().filter(|a| active(a)) {
            if self.is_customised(&a.action) && a.defaults.contains(&accel) {
                return Resolve::Disabled;
            }
        }
        Resolve::Unbound
    }

    /// The reader action that has `accel` among its keys and was not changed by the user, for keys
    /// the reader's built-in handling does not know. `None` when there is none (or the user took
    /// the key for another action, which [`Shortcuts::resolve`] has already handled).
    pub fn default_reader_action(&self, accel: &str) -> Option<String> {
        let accel = canonical(accel)?;
        registry()
            .into_iter()
            .filter(|a| a.group == Group::Reader && !self.is_customised(&a.action))
            .find(|a| a.defaults.contains(&accel))
            .map(|a| a.action)
    }

    // ---- file ----

    pub fn to_json(&self) -> String {
        #[derive(Serialize)]
        struct File<'a> {
            version: u32,
            bindings: &'a BTreeMap<String, Vec<String>>,
        }
        serde_json::to_string_pretty(&File {
            version: 1,
            bindings: &self.bindings,
        })
        .unwrap_or_default()
    }

    /// Reads a file; unknown actions and invalid keys are skipped and reported. Fails only when it
    /// is not a shortcuts file at all.
    pub fn from_json(text: &str) -> Result<(Shortcuts, ImportReport), String> {
        let raw: Shortcuts = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let reg = registry();
        let mut out = Shortcuts::default();
        let mut report = ImportReport::default();
        for (action, keys) in raw.bindings {
            if !reg.iter().any(|a| a.action == action) {
                report.skipped.push(format!("{action}: unknown action"));
                continue;
            }
            let mut ok = Vec::new();
            let mut bad = false;
            for k in &keys {
                match canonical(k) {
                    Some(c) if !ok.contains(&c) => ok.push(c),
                    Some(_) => {}
                    None => {
                        report.skipped.push(format!("{action}: invalid key {k}"));
                        bad = true;
                    }
                }
            }
            if bad && ok.is_empty() && !keys.is_empty() {
                continue;
            }
            out.store(&action, ok);
            report.applied += 1;
        }
        Ok((out, report))
    }

    pub fn load(path: &std::path::Path) -> Shortcuts {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| Shortcuts::from_json(&t).ok())
            .map(|(s, _)| s)
            .unwrap_or_default()
    }

    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        if self.bindings.is_empty() {
            let _ = std::fs::remove_file(path);
            return Ok(());
        }
        std::fs::write(path, self.to_json())
    }
}

// ---- the one set the running application uses ----

type Listener = Rc<dyn Fn()>;

thread_local! {
    static CURRENT: RefCell<Shortcuts> = RefCell::new(Shortcuts::default());
    static LISTENERS: RefCell<Vec<Listener>> = const { RefCell::new(Vec::new()) };
}

/// Reads the file of the config folder; call once at start-up.
pub fn load_global() {
    if let Some(p) = crate::config::shortcuts_path() {
        CURRENT.with(|c| *c.borrow_mut() = Shortcuts::load(&p));
    }
}

pub fn current() -> Shortcuts {
    CURRENT.with(|c| c.borrow().clone())
}

/// The keys of an action as text for menus, the palette and the shortcuts window; falls back to
/// `default` (the palette's single default) for actions the registry does not know.
pub fn display(action: &str, default: &str) -> String {
    CURRENT.with(|c| {
        let c = c.borrow();
        if c.is_customised(action) || registry().iter().any(|a| a.action == action) {
            c.display(action)
        } else {
            label(default)
        }
    })
}

/// Replaces the set, saves it and tells everyone who applies it (accelerators, library keys).
pub fn replace(new: Shortcuts) {
    if let Some(p) = crate::config::shortcuts_path() {
        let _ = new.save(&p);
    }
    CURRENT.with(|c| *c.borrow_mut() = new);
    let ls: Vec<Listener> = LISTENERS.with(|l| l.borrow().clone());
    for l in ls {
        l();
    }
}

/// Changes the set with `f`; returns what `f` returned.
pub fn update<R>(f: impl FnOnce(&mut Shortcuts) -> R) -> R {
    let mut s = current();
    let r = f(&mut s);
    replace(s);
    r
}

/// Registers a function that runs after every change.
pub fn on_change(f: impl Fn() + 'static) {
    LISTENERS.with(|l| l.borrow_mut().push(Rc::new(f)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s() -> Shortcuts {
        Shortcuts::default()
    }

    #[test]
    fn registry_has_unique_actions_and_valid_defaults() {
        let reg = registry();
        let mut seen = std::collections::HashSet::new();
        for a in &reg {
            assert!(seen.insert(a.action.clone()), "{} twice", a.action);
            for d in &a.defaults {
                assert_eq!(
                    canonical(d).as_deref(),
                    Some(d.as_str()),
                    "{}: {d}",
                    a.action
                );
            }
        }
        assert!(reg
            .iter()
            .any(|a| a.action == "app.palette" && a.defaults.len() == 2));
    }

    #[test]
    fn default_keys_do_not_collide() {
        assert_eq!(s().all_conflicts(), vec![]);
    }

    #[test]
    fn accelerators_have_one_spelling() {
        assert_eq!(
            canonical("<Control><Shift>P").as_deref(),
            Some("<Control><Shift>p")
        );
        assert_eq!(canonical("<Primary>o").as_deref(), Some("<Control>o"));
        assert_eq!(
            canonical("<Control><Alt>Left").as_deref(),
            Some("<Control><Alt>Left")
        );
        assert_eq!(canonical("F11").as_deref(), Some("F11"));
        assert_eq!(canonical(""), None);
        assert_eq!(canonical("<Control>"), None);
        assert_eq!(canonical("<Bogus>x"), None);
        assert_eq!(canonical("Control_L"), None);
        assert_eq!(canonical("<Control>a b"), None);
    }

    #[test]
    fn key_presses_become_accelerators() {
        assert_eq!(
            from_press(true, true, false, false, "p", Some('p')).as_deref(),
            Some("<Control><Shift>p")
        );
        // Ctrl+? is the key "question" with Shift consumed
        assert_eq!(
            from_press(true, true, false, false, "question", Some('?')).as_deref(),
            Some("<Control>question")
        );
        assert_eq!(
            from_press(false, false, true, false, "Left", None).as_deref(),
            Some("<Alt>Left")
        );
        assert_eq!(
            from_press(true, false, false, false, "F8", None).as_deref(),
            Some("<Shift>F8")
        );
    }

    #[test]
    fn bare_letters_are_not_shortcuts() {
        assert!(is_bare_text_key("j"));
        assert!(is_bare_text_key("5"));
        assert!(!is_bare_text_key("<Control>j"));
        assert!(!is_bare_text_key("F5"));
        assert!(!is_bare_text_key("Left"));
    }

    #[test]
    fn rebinding_replaces_the_default_and_frees_it() {
        let mut c = s();
        assert_eq!(c.effective("reader.reload"), vec!["<Control>r"]);
        c.bind("reader.reload", "F5").unwrap();
        assert_eq!(c.effective("reader.reload"), vec!["F5"]);
        assert_eq!(c.display("reader.reload"), "F5");
        // the old key is now free for another action
        c.bind("reader.bookmark", "<Control>r").unwrap();
        assert!(c.all_conflicts().is_empty());
    }

    #[test]
    fn a_taken_key_is_reported_and_nothing_changes() {
        let mut c = s();
        let err = c.bind("reader.reload", "<Control>b").unwrap_err();
        assert_eq!(err.len(), 1);
        assert_eq!(err[0].action, "reader.bookmark");
        assert!(!c.any_customised());
    }

    #[test]
    fn library_and_reader_may_share_a_key_but_application_may_not() {
        let mut c = s();
        // Ctrl+F is search in both windows by default
        assert!(c.all_conflicts().is_empty());
        // an application action collides with both
        let err = c.bind("app.about", "<Control>f").unwrap_err();
        assert_eq!(err.len(), 2);
        // library and reader are separate
        c.bind("lib.select-all", "<Control>b").unwrap();
        assert!(c.all_conflicts().is_empty());
    }

    #[test]
    fn settle_by_taking_the_key_away() {
        let mut c = s();
        assert!(c.bind_settled("reader.reload", "<Control>b", Settle::TakeAway));
        assert_eq!(c.effective("reader.reload"), vec!["<Control>b"]);
        assert!(c.effective("reader.bookmark").is_empty());
        assert!(c.is_customised("reader.bookmark"));
        assert!(c.all_conflicts().is_empty());
    }

    #[test]
    fn settle_by_swapping_the_keys() {
        let mut c = s();
        assert!(c.bind_settled("reader.reload", "<Control>b", Settle::Swap));
        assert_eq!(c.effective("reader.reload"), vec!["<Control>b"]);
        assert_eq!(c.effective("reader.bookmark"), vec!["<Control>r"]);
        assert!(c.all_conflicts().is_empty());
    }

    #[test]
    fn rebinding_back_to_the_default_forgets_the_override() {
        let mut c = s();
        c.bind("reader.reload", "F5").unwrap();
        c.bind("reader.reload", "<Control>r").unwrap();
        assert!(!c.any_customised());
    }

    #[test]
    fn clear_and_reset() {
        let mut c = s();
        c.clear("app.quit");
        assert!(c.effective("app.quit").is_empty());
        assert_eq!(c.display("app.quit"), "");
        c.reset("app.quit");
        assert_eq!(c.effective("app.quit"), vec!["<Control>q"]);
        c.clear("app.quit");
        c.bind("reader.reload", "F5").unwrap();
        c.reset_all();
        assert!(!c.any_customised());
    }

    #[test]
    fn an_action_with_two_defaults_gets_one_after_rebinding() {
        let mut c = s();
        assert_eq!(c.display("app.palette"), "Ctrl+Shift+P, Ctrl+K");
        c.bind("app.palette", "F10").unwrap();
        assert_eq!(c.effective("app.palette"), vec!["F10"]);
    }

    #[test]
    fn the_reader_looks_up_keys() {
        let mut c = s();
        assert_eq!(c.resolve(Group::Reader, "<Control>r"), Resolve::Unbound);
        c.bind("reader.reload", "F5").unwrap();
        assert_eq!(
            c.resolve(Group::Reader, "F5"),
            Resolve::Action("reader.reload".into())
        );
        // the old key does nothing now
        assert_eq!(c.resolve(Group::Reader, "<Control>r"), Resolve::Disabled);
        // other keys keep their built-in handling
        assert_eq!(c.resolve(Group::Reader, "Right"), Resolve::Unbound);
        // library actions are not the reader's business
        c.bind("lib.select-all", "F6").unwrap();
        assert_eq!(c.resolve(Group::Reader, "F6"), Resolve::Unbound);
    }

    #[test]
    fn a_freed_default_goes_to_its_new_owner_not_disabled() {
        let mut c = s();
        c.bind("reader.reload", "F5").unwrap();
        c.bind("reader.bookmark", "<Control>r").unwrap();
        assert_eq!(
            c.resolve(Group::Reader, "<Control>r"),
            Resolve::Action("reader.bookmark".into())
        );
    }

    #[test]
    fn export_and_import_round_trip() {
        let mut c = s();
        c.bind("reader.reload", "F5").unwrap();
        c.clear("app.quit");
        let text = c.to_json();
        let (back, report) = Shortcuts::from_json(&text).unwrap();
        assert_eq!(back, c);
        assert_eq!(report.applied, 2);
        assert!(report.skipped.is_empty());
    }

    #[test]
    fn import_skips_what_it_cannot_use() {
        let text = r#"{"version":1,"bindings":{
            "reader.reload":["F5"],
            "no.such":["F6"],
            "app.about":["<Bogus>x"],
            "app.quit":[]}}"#;
        let (c, report) = Shortcuts::from_json(text).unwrap();
        assert_eq!(c.effective("reader.reload"), vec!["F5"]);
        assert!(c.effective("app.quit").is_empty());
        assert!(!c.is_customised("app.about"));
        assert_eq!(report.applied, 2);
        assert_eq!(report.skipped.len(), 2);
        assert!(Shortcuts::from_json("nonsense").is_err());
    }

    #[test]
    fn imported_duplicates_are_found() {
        let text = r#"{"bindings":{"reader.reload":["<Control>b"]}}"#;
        let (c, _) = Shortcuts::from_json(text).unwrap();
        let all = c.all_conflicts();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].2, "<Control>b");
    }

    #[test]
    fn the_file_is_saved_and_loaded_and_removed_when_default() {
        let dir = std::env::temp_dir().join(format!("bookx-sc-{}", std::process::id()));
        let path = dir.join("shortcuts.json");
        let mut c = s();
        c.bind("reader.reload", "F5").unwrap();
        c.save(&path).unwrap();
        assert_eq!(Shortcuts::load(&path), c);
        c.reset_all();
        c.save(&path).unwrap();
        assert!(!path.exists());
        assert_eq!(Shortcuts::load(&path), Shortcuts::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_or_broken_file_means_defaults() {
        let dir = std::env::temp_dir().join(format!("bookx-sc2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("shortcuts.json");
        std::fs::write(&path, "{{{").unwrap();
        assert_eq!(Shortcuts::load(&path), Shortcuts::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_actions_have_no_keys() {
        let c = s();
        assert!(c.effective("nothing").is_empty());
        assert!(c.conflicts("nothing", "F1").is_empty());
    }

    #[test]
    fn the_palette_and_menus_show_the_new_keys() {
        let mut c = s();
        c.bind("reader.reload", "F5").unwrap();
        c.clear("reader.bookmark");
        CURRENT.with(|x| *x.borrow_mut() = c);
        let entries = palette::commands(Scope::Reader);
        let shortcut = |title: &str| {
            entries
                .iter()
                .find(|e| e.title == title)
                .map(|e| e.shortcut.clone())
        };
        assert_eq!(shortcut("Reload the Book").as_deref(), Some("F5"));
        assert_eq!(shortcut("Toggle Bookmark").as_deref(), Some(""));
        assert_eq!(display("reader.reload", ""), "F5");
    }

    #[test]
    fn display_falls_back_to_the_given_default() {
        assert_eq!(display("unknown.thing", "<Control>x"), "Ctrl+X");
        assert_eq!(display("app.quit", ""), "Ctrl+Q");
    }

    fn keys_of(action: &str) -> Vec<String> {
        s().effective(action)
    }

    #[test]
    fn the_well_known_reader_keys_are_defaults() {
        let want: &[(&str, &[&str])] = &[
            ("reader.search", &["<Control>f"]),
            ("reader.fullscreen", &["F11"]),
            ("reader.contents", &["F9", "<Control>t"]),
            ("reader.bookmark", &["<Control>b"]),
            ("reader.notes", &["<Control><Shift>b", "<Control>h"]),
            ("reader.larger", &["<Control>plus", "<Control>equal"]),
            ("reader.smaller", &["<Control>minus"]),
            ("reader.reset-size", &["<Control>0"]),
            ("reader.goto", &["<Control>g"]),
            ("reader.jump-back", &["<Alt>Left"]),
            ("reader.jump-forward", &["<Alt>Right"]),
            ("reader.start", &["Home"]),
            ("reader.end", &["End"]),
            ("reader.next-page", &["Right", "space", "Page_Down"]),
            ("reader.prev-page", &["Left", "<Shift>space", "Page_Up"]),
            ("reader.cycle-theme", &["t"]),
            ("reader.map", &["m"]),
            ("reader.read-aloud", &["r"]),
            ("reader.open", &["<Control>o"]),
            ("reader.properties", &["<Control>i"]),
            ("reader.back", &["Escape", "<Control>w"]),
            ("lib.add-books", &["<Control>o"]),
            ("book.edit-meta", &["<Control>e"]),
            ("book.edit-book", &["<Control><Shift>e"]),
            ("book.properties", &["<Control>i"]),
            ("app.quit", &["<Control>q"]),
            ("app.preferences", &["<Control>comma"]),
            ("app.shortcuts", &["<Control>question"]),
            ("app.help", &["F1"]),
        ];
        for (action, keys) in want {
            assert_eq!(&keys_of(action), keys, "{action}");
        }
    }

    #[test]
    fn no_two_default_actions_share_a_key_in_the_same_window() {
        let reg = registry();
        for (i, a) in reg.iter().enumerate() {
            for b in &reg[i + 1..] {
                if !a.group.overlaps(b.group) {
                    continue;
                }
                for k in &a.defaults {
                    assert!(
                        !b.defaults.contains(k),
                        "{k} is the default of both {} and {}",
                        a.action,
                        b.action
                    );
                }
            }
            // within one action a key is listed once
            let mut d = a.defaults.clone();
            d.sort();
            d.dedup();
            assert_eq!(d.len(), a.defaults.len(), "{}", a.action);
        }
    }

    #[test]
    fn keys_without_built_in_handling_find_their_reader_action() {
        let c = s();
        assert_eq!(c.default_reader_action("m").as_deref(), Some("reader.map"));
        assert_eq!(
            c.default_reader_action("<Control>t").as_deref(),
            Some("reader.contents")
        );
        assert_eq!(
            c.default_reader_action("<Control>I").as_deref(),
            Some("reader.properties")
        );
        // library and application keys are not the reader's
        assert_eq!(c.default_reader_action("<Control>q"), None);
        assert_eq!(c.default_reader_action("<Control>e"), None);
        assert_eq!(c.default_reader_action("nonsense key"), None);
    }

    #[test]
    fn the_users_choice_wins_over_the_new_defaults() {
        let mut c = s();
        c.bind("reader.map", "F6").unwrap();
        // the old key is not the map's any more, the new one is
        assert_eq!(c.default_reader_action("m"), None);
        assert_eq!(c.resolve(Group::Reader, "m"), Resolve::Disabled);
        assert_eq!(
            c.resolve(Group::Reader, "F6"),
            Resolve::Action("reader.map".into())
        );
        // a second default of an untouched action still works
        c.bind("reader.contents", "F2").unwrap();
        assert_eq!(c.default_reader_action("<Control>t"), None);
        assert_eq!(c.resolve(Group::Reader, "<Control>t"), Resolve::Disabled);
        // the file keeps the choice
        let (back, report) = Shortcuts::from_json(&c.to_json()).unwrap();
        assert_eq!(report.skipped.len(), 0);
        assert_eq!(back.effective("reader.map"), vec!["F6".to_string()]);
        assert_eq!(
            back.effective("reader.reset-size"),
            vec!["<Control>0".to_string()]
        );
        // the key a user took from a default goes to them, whatever the new defaults are
        let mut d = s();
        d.bind("reader.cycle-theme", "<Control>m").unwrap();
        assert_eq!(d.resolve(Group::Reader, "t"), Resolve::Disabled);
    }

    #[test]
    fn a_taken_new_default_is_reported_as_a_conflict() {
        let c = s();
        let who = c.conflicts("reader.map", "<Control>t");
        assert_eq!(who.len(), 1);
        assert_eq!(who[0].action, "reader.contents");
        // space is a default of the next page, so it is not free either
        assert_eq!(c.conflicts("reader.map", "space").len(), 1);
    }
}
