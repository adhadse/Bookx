//! Editor plugins (CE-019): the host side of `bookx-plugin` ABI v2. An enabled `editor-action` plugin
//! adds its actions to Tools → Plugins (and so to the command palette). An action runs on a **copy**
//! of the open book ([`BookCopy`]); when it succeeds the copy replaces the book inside
//! [`Editor::run_op`], so the whole action is one checkpoint "Before: <label>" that *Restore
//! Checkpoint* undoes, and a failing action leaves the book untouched.
//!
//! Selection calls work on the text of the open file: the editor passes the selection and cursor as
//! character offsets, `replace_selection` / `insert_at_cursor` edit that text in the copy.

use std::sync::{Arc, Mutex};

use bookx_plugin::{EditorApi, EditorLimits, EditorSession};
use serde_json::Value;

use super::*;
use crate::core::archive::Archive;
use crate::core::opf::Metadata;

/// The book a plugin sees. Shared with the editor so it can read the result after the run.
pub(crate) struct BookCopy {
    pub archive: Archive,
    pub metadata: Metadata,
    pub meta_changed: bool,
    pub changed: bool,
    pub current: Option<String>,
    /// Char offsets of the selection in the current file, `cursor` when nothing is selected.
    pub selection: Option<(usize, usize)>,
    pub cursor: usize,
    /// Checkpoints the plugin asked for: `(name, book at that moment)`.
    pub checkpoints: Vec<(String, Archive)>,
    /// The checkpoint the editor takes itself before every action.
    auto_name: String,
    /// The current file was rewritten by `write_file`: the selection offsets no longer mean anything.
    selection_stale: bool,
}

pub(crate) type SharedCopy = Arc<Mutex<BookCopy>>;

impl BookCopy {
    pub fn new(
        archive: Archive,
        metadata: Metadata,
        current: Option<String>,
        selection: Option<(usize, usize)>,
        cursor: usize,
        label: &str,
    ) -> SharedCopy {
        Arc::new(Mutex::new(BookCopy {
            archive,
            metadata,
            meta_changed: false,
            changed: false,
            current,
            selection: selection.filter(|(a, b)| a != b),
            cursor,
            checkpoints: Vec::new(),
            auto_name: format!("Before: {label}"),
            selection_stale: false,
        }))
    }
}

fn byte_at(text: &str, chars: usize) -> Option<usize> {
    if chars == text.chars().count() {
        return Some(text.len());
    }
    text.char_indices().nth(chars).map(|(i, _)| i)
}

/// [`EditorApi`] over a [`BookCopy`].
pub(crate) struct BookAdapter(pub SharedCopy);

impl BookAdapter {
    fn with<R>(&self, f: impl FnOnce(&mut BookCopy) -> R) -> R {
        f(&mut self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

impl EditorApi for BookAdapter {
    fn list_files(&mut self) -> Vec<String> {
        self.with(|b| b.archive.names().to_vec())
    }

    fn read_file(&mut self, path: &str) -> Option<Vec<u8>> {
        self.with(|b| b.archive.get(path).map(<[u8]>::to_vec))
    }

    fn write_file(&mut self, path: &str, data: &[u8]) -> Result<(), String> {
        self.with(|b| {
            if !b.archive.contains(path) {
                return Err(format!("no such file: {path}"));
            }
            b.archive.set(path, data.to_vec());
            b.changed = true;
            if b.current.as_deref() == Some(path) {
                b.selection_stale = true;
            }
            Ok(())
        })
    }

    fn add_file(&mut self, path: &str, data: &[u8]) -> Result<(), String> {
        self.with(|b| {
            if b.archive.contains(path) {
                return Err(format!("{path} already exists"));
            }
            b.archive.set(path, data.to_vec());
            b.changed = true;
            Ok(())
        })
    }

    fn rename_file(&mut self, from: &str, to: &str) -> Result<usize, String> {
        self.with(|b| {
            if !b.archive.contains(from) {
                return Err(format!("no such file: {from}"));
            }
            if b.archive.contains(to) {
                return Err(format!("{to} already exists"));
            }
            let map = std::collections::HashMap::from([(from.to_string(), to.to_string())]);
            let n = bookx_editing::markup::rename_paths(&mut b.archive, &map);
            if b.current.as_deref() == Some(from) {
                b.current = Some(to.to_string());
            }
            b.changed = true;
            Ok(n)
        })
    }

    fn current_file(&mut self) -> Option<String> {
        self.with(|b| b.current.clone())
    }

    fn selection(&mut self) -> Option<(String, String)> {
        self.with(|b| {
            let (a, z) = b.selection.filter(|_| !b.selection_stale)?;
            let file = b.current.clone()?;
            let text = b.archive.get_string(&file)?;
            let (i, j) = (byte_at(&text, a)?, byte_at(&text, z)?);
            Some((file, text.get(i..j)?.to_string()))
        })
    }

    fn replace_selection(&mut self, text: &str) -> Result<(), String> {
        self.with(|b| {
            let (Some((a, z)), Some(file)) = (b.selection, b.current.clone()) else {
                return Err("nothing is selected".into());
            };
            if b.selection_stale {
                return Err("the file changed since the selection was made".into());
            }
            let old = b.archive.get_string(&file).ok_or("the file is not text")?;
            let i = byte_at(&old, a).ok_or("the selection is outside the file")?;
            let j = byte_at(&old, z).ok_or("the selection is outside the file")?;
            let mut new = String::with_capacity(old.len() + text.len());
            new.push_str(&old[..i]);
            new.push_str(text);
            new.push_str(&old[j..]);
            b.archive.set_string(&file, &new);
            let n = text.chars().count();
            b.selection = Some((a, a + n)).filter(|(x, y)| x != y);
            b.cursor = a + n;
            b.changed = true;
            Ok(())
        })
    }

    fn insert_at_cursor(&mut self, text: &str) -> Result<(), String> {
        self.with(|b| {
            let Some(file) = b.current.clone() else {
                return Err("no file is open".into());
            };
            if b.selection_stale {
                return Err("the file changed since the cursor was placed".into());
            }
            let old = b.archive.get_string(&file).ok_or("the file is not text")?;
            let i = byte_at(&old, b.cursor).ok_or("the cursor is outside the file")?;
            let mut new = String::with_capacity(old.len() + text.len());
            new.push_str(&old[..i]);
            new.push_str(text);
            new.push_str(&old[i..]);
            b.archive.set_string(&file, &new);
            b.cursor += text.chars().count();
            b.selection = None;
            b.changed = true;
            Ok(())
        })
    }

    fn metadata(&mut self) -> Value {
        self.with(|b| serde_json::to_value(&b.metadata).unwrap_or(Value::Null))
    }

    fn set_metadata(&mut self, fields: &Value) -> Result<(), String> {
        self.with(|b| {
            let Some(patch) = fields.as_object() else {
                return Err("metadata must be an object".into());
            };
            let mut cur = serde_json::to_value(&b.metadata).map_err(|e| e.to_string())?;
            let known = cur.as_object().cloned().unwrap_or_default();
            for (k, v) in patch {
                if !known.contains_key(k) {
                    return Err(format!("unknown metadata field `{k}`"));
                }
                cur[k] = v.clone();
            }
            let md: Metadata =
                serde_json::from_value(cur).map_err(|e| format!("invalid metadata: {e}"))?;
            if md != b.metadata {
                b.metadata = md;
                b.meta_changed = true;
                b.changed = true;
            }
            Ok(())
        })
    }

    fn checkpoint(&mut self, name: &str) -> Result<(), String> {
        self.with(|b| {
            // the editor already takes "Before: <label>" itself
            if name != b.auto_name {
                let snap = b.archive.clone();
                b.checkpoints.push((name.to_string(), snap));
            }
            Ok(())
        })
    }
}

/// Entry points for the tests of `crate::plugins` (the adapter itself stays private to the editor).
#[cfg(test)]
pub(crate) mod for_tests {
    use super::*;

    pub fn new_copy(archive: Archive) -> SharedCopy {
        BookCopy::new(archive, Metadata::default(), None, None, 0, "Trim")
    }

    pub fn adapter(c: SharedCopy) -> impl EditorApi {
        BookAdapter(c)
    }
}

/// Char offset of an iter.
fn offset(it: &gtk::TextIter) -> usize {
    it.offset().max(0) as usize
}

impl Editor {
    /// Runs `action` of the editor plugin `plugin_id` (Tools → Plugins).
    pub(super) fn run_editor_plugin(&mut self, plugin_id: &str, action: &str) {
        let loaded = crate::plugins::current();
        let Some(entry) = loaded
            .editor_actions
            .iter()
            .find(|e| e.plugin.id() == plugin_id && e.action.id == action)
            .cloned()
        else {
            self.toast(&trf("The plugin {} is not enabled", &[&plugin_id]));
            return;
        };
        self.flush();
        let label = entry.action.label.clone();
        let (selection, cursor) = match &self.current {
            Some(_) => {
                let buf = self.w.view.buffer();
                let sel = buf
                    .selection_bounds()
                    .map(|(a, b)| (offset(&a), offset(&b)));
                (sel, offset(&buf.iter_at_mark(&buf.get_insert())))
            }
            None => (None, 0),
        };
        let copy = {
            let b = self.book.borrow();
            BookCopy::new(
                b.archive.clone(),
                b.metadata().clone(),
                self.current.clone(),
                selection,
                cursor,
                &label,
            )
        };
        let session = EditorSession::new(
            Box::new(BookAdapter(copy.clone())),
            &entry.plugin.manifest().permissions,
            &label,
            EditorLimits::default(),
        );
        let outcome: Rc<RefCell<Option<bookx_plugin::ActionResult>>> = Rc::default();
        let out = outcome.clone();
        let (plugin, aid) = (entry.plugin.clone(), entry.action.id.clone());
        let run_copy = copy.clone();
        self.run_op(&label, move |book, _| {
            let r = plugin.run_editor_action(&aid, &session)?;
            let c = run_copy.lock().unwrap_or_else(|e| e.into_inner());
            let changed = c.changed;
            if changed {
                book.archive = c.archive.clone();
                book.reload_package()?;
                if c.meta_changed {
                    book.set_metadata(c.metadata.clone())?;
                }
            }
            let mut details = r.messages.clone();
            let summary = match &r.reply {
                Value::String(s) if !s.is_empty() => s.clone(),
                _ => details
                    .first()
                    .cloned()
                    .unwrap_or_else(|| tr(if changed { "Done" } else { "Nothing changed" })),
            };
            if details.len() == 1 && details[0] == summary {
                details.clear();
            }
            *out.borrow_mut() = Some(r);
            Ok(editor_tools::Outcome {
                changed,
                summary,
                details,
            })
        });
        // checkpoints the plugin named itself
        if outcome.borrow().is_some() {
            let named =
                std::mem::take(&mut copy.lock().unwrap_or_else(|e| e.into_inner()).checkpoints);
            for (name, snap) in named {
                self.checkpoints.create(&name, &snap);
            }
            self.refresh_restore_menu();
        }
    }
}

/// `plugin:action:<plugin id>:<action id>`: the target of a Tools → Plugins menu item.
pub(super) fn action_target(plugin_id: &str, action_id: &str) -> String {
    format!("plugin:action:{plugin_id}:{action_id}")
}

/// Splits the part after `plugin:` of [`action_target`] back into `(plugin id, action id)`.
pub(super) fn parse_action_target(id: &str) -> Option<(&str, &str)> {
    id.strip_prefix("action:")?.split_once(':')
}

/// The Tools → Plugins section: one submenu per group ("Plugins" when the action has none).
pub(super) fn menu_section(loaded: &crate::plugins::Loaded) -> Option<gio::Menu> {
    if loaded.editor_actions.is_empty() {
        return None;
    }
    let section = gio::Menu::new();
    let plugins = gio::Menu::new();
    for e in loaded.editor_actions.iter() {
        let it = gio::MenuItem::new(Some(&e.action.label), None);
        it.set_action_and_target_value(
            Some("tools.run"),
            Some(&action_target(e.plugin.id(), &e.action.id).to_variant()),
        );
        plugins.append_item(&it);
    }
    section.append_submenu(Some(&tr("Plugins")), &plugins);
    Some(section)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn copy(text: &str, sel: Option<(usize, usize)>, cursor: usize) -> (SharedCopy, BookAdapter) {
        let mut a = Archive::default();
        a.set_string("OEBPS/a.xhtml", text);
        a.set_string("mimetype", "application/epub+zip");
        let c = BookCopy::new(
            a,
            Metadata::default(),
            Some("OEBPS/a.xhtml".into()),
            sel,
            cursor,
            "Run",
        );
        (c.clone(), BookAdapter(c))
    }

    #[test]
    fn target_round_trip() {
        let t = action_target("org.x-y", "do_it");
        let id = t.strip_prefix("plugin:").unwrap();
        assert_eq!(parse_action_target(id), Some(("org.x-y", "do_it")));
        assert_eq!(parse_action_target("other"), None);
    }

    #[test]
    fn replace_selection_uses_char_offsets() {
        let (c, mut a) = copy("héllo wörld", Some((6, 11)), 11);
        assert_eq!(
            a.selection(),
            Some(("OEBPS/a.xhtml".into(), "wörld".into()))
        );
        a.replace_selection("Bookx").unwrap();
        assert_eq!(
            c.lock()
                .unwrap()
                .archive
                .get_string("OEBPS/a.xhtml")
                .unwrap(),
            "héllo Bookx"
        );
        assert!(c.lock().unwrap().changed);
        // the selection now covers the new text
        assert_eq!(a.selection().unwrap().1, "Bookx");
        a.insert_at_cursor("!").unwrap();
        assert_eq!(
            c.lock()
                .unwrap()
                .archive
                .get_string("OEBPS/a.xhtml")
                .unwrap(),
            "héllo Bookx!"
        );
    }

    #[test]
    fn selection_errors_and_staleness() {
        let (_, mut a) = copy("abc", None, 1);
        assert!(a.selection().is_none());
        assert!(a.replace_selection("x").is_err());
        a.insert_at_cursor("X").unwrap();
        let (_, mut a) = copy("abc", Some((0, 99)), 0);
        assert!(a.selection().is_none());
        assert!(a.replace_selection("x").is_err());
        let (_, mut a) = copy("abc", Some((0, 1)), 0);
        a.write_file("OEBPS/a.xhtml", b"zzz").unwrap();
        assert!(a.selection().is_none());
        assert!(a.replace_selection("x").is_err());
        assert!(a.insert_at_cursor("x").is_err());
    }

    #[test]
    fn files_add_write_rename() {
        let (c, mut a) = copy("<a href=\"b.xhtml\"/>", None, 0);
        c.lock().unwrap().archive.set_string("OEBPS/b.xhtml", "b");
        assert!(a.write_file("nope", b"x").is_err());
        assert!(a.add_file("OEBPS/b.xhtml", b"x").is_err());
        a.add_file("OEBPS/c.xhtml", b"c").unwrap();
        assert_eq!(a.rename_file("OEBPS/b.xhtml", "OEBPS/d.xhtml").unwrap(), 1);
        assert!(c.lock().unwrap().archive.contains("OEBPS/d.xhtml"));
        assert!(!c.lock().unwrap().archive.contains("OEBPS/b.xhtml"));
        assert!(a.rename_file("OEBPS/zzz", "OEBPS/q").is_err());
        assert!(a.rename_file("OEBPS/d.xhtml", "OEBPS/c.xhtml").is_err());
        assert!(a.list_files().contains(&"OEBPS/c.xhtml".to_string()));
    }

    #[test]
    fn metadata_merges_known_fields_only() {
        let (c, mut a) = copy("x", None, 0);
        a.set_metadata(&serde_json::json!({"title": "T", "authors": ["A"]}))
            .unwrap();
        assert_eq!(c.lock().unwrap().metadata.title, "T");
        assert!(c.lock().unwrap().meta_changed);
        assert!(a.set_metadata(&serde_json::json!({"nope": 1})).is_err());
        assert!(a
            .set_metadata(&serde_json::json!({"rating": "high"}))
            .is_err());
        assert!(a.set_metadata(&serde_json::json!([1])).is_err());
        assert_eq!(a.metadata()["title"], "T");
    }

    #[test]
    fn the_automatic_checkpoint_is_not_duplicated() {
        let (c, mut a) = copy("x", None, 0);
        a.checkpoint("Before: Run").unwrap();
        assert!(c.lock().unwrap().checkpoints.is_empty());
        a.write_file("OEBPS/a.xhtml", b"y").unwrap();
        a.checkpoint("mine").unwrap();
        let g = c.lock().unwrap();
        assert_eq!(g.checkpoints.len(), 1);
        assert_eq!(g.checkpoints[0].0, "mine");
        assert_eq!(g.checkpoints[0].1.get_string("OEBPS/a.xhtml").unwrap(), "y");
    }

    #[test]
    fn nothing_leaves_the_copy() {
        // the original archive is a separate value: the copy can be dropped without effect
        let mut orig = Archive::default();
        orig.set_string("a", "1");
        let c = BookCopy::new(orig.clone(), Metadata::default(), None, None, 0, "R");
        BookAdapter(c.clone()).write_file("a", b"2").unwrap();
        assert_eq!(orig.get_string("a").unwrap(), "1");
    }
}
