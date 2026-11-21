//! Runs one editor action over a whole book without an editor (`bookx-cli plugin run`, and any other
//! caller that has an [`Archive`] and no GUI).
//!
//! The action sees the book as [`MemBook`] files; selection and cursor calls fail ("nothing is
//! selected") and metadata edits are reported but not applied. Only files changed, added or removed by
//! the plugin come back, and the caller decides whether to keep them.

use std::sync::{Arc, Mutex};

use anyhow::Result;
use bookx_core::archive::Archive;
use serde_json::Value;

use crate::editor::{ActionResult, EditorApi, EditorLimits, EditorSession, MemBook};
use crate::runtime::Plugin;

/// Hands the same [`MemBook`] to the session and to the caller.
struct Shared(Arc<Mutex<MemBook>>);

impl Shared {
    fn with<R>(&self, f: impl FnOnce(&mut MemBook) -> R) -> R {
        f(&mut self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

impl EditorApi for Shared {
    fn list_files(&mut self) -> Vec<String> {
        self.with(|b| b.list_files())
    }
    fn read_file(&mut self, path: &str) -> Option<Vec<u8>> {
        self.with(|b| b.read_file(path))
    }
    fn write_file(&mut self, path: &str, data: &[u8]) -> Result<(), String> {
        self.with(|b| b.write_file(path, data))
    }
    fn add_file(&mut self, path: &str, data: &[u8]) -> Result<(), String> {
        self.with(|b| b.add_file(path, data))
    }
    fn rename_file(&mut self, from: &str, to: &str) -> Result<usize, String> {
        self.with(|b| b.rename_file(from, to))
    }
    fn current_file(&mut self) -> Option<String> {
        self.with(|b| b.current_file())
    }
    fn selection(&mut self) -> Option<(String, String)> {
        self.with(|b| b.selection())
    }
    fn replace_selection(&mut self, text: &str) -> Result<(), String> {
        self.with(|b| b.replace_selection(text))
    }
    fn insert_at_cursor(&mut self, text: &str) -> Result<(), String> {
        self.with(|b| b.insert_at_cursor(text))
    }
    fn metadata(&mut self) -> Value {
        self.with(|b| b.metadata())
    }
    fn set_metadata(&mut self, fields: &Value) -> Result<(), String> {
        self.with(|b| b.set_metadata(fields))
    }
    fn checkpoint(&mut self, name: &str) -> Result<(), String> {
        self.with(|b| b.checkpoint(name))
    }
}

/// What a whole-book run produced.
#[derive(Debug)]
pub struct BatchOutcome {
    /// The book after the action (equal to the input when nothing changed).
    pub archive: Archive,
    /// Names of files the action changed, added or removed.
    pub changed_files: Vec<String>,
    pub result: ActionResult,
}

/// Runs `action` of `plugin` on a copy of `archive`. `label` names the automatic checkpoint the host
/// takes before a write (there is nowhere to keep it here, the original archive is the undo).
/// An error or a trap leaves the input untouched: nothing is returned.
pub fn run_on_archive(
    plugin: &Plugin,
    action: &str,
    archive: &Archive,
    label: &str,
) -> Result<BatchOutcome> {
    let mut book = MemBook::default();
    for name in archive.names() {
        if let Some(b) = archive.get(name) {
            book.files.insert(name.clone(), b.to_vec());
        }
    }
    let before = book.files.clone();
    let shared = Arc::new(Mutex::new(book));
    let session = EditorSession::new(
        Box::new(Shared(shared.clone())),
        &plugin.manifest().permissions,
        label,
        EditorLimits::default(),
    );
    let result = plugin.run_editor_action(action, &session)?;
    let after = shared
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .files
        .clone();
    let mut changed_files = Vec::new();
    let mut out = archive.clone();
    for (name, data) in &after {
        if before.get(name) != Some(data) {
            out.set(name, data.clone());
            changed_files.push(name.clone());
        }
    }
    for name in before.keys() {
        if !after.contains_key(name) {
            out.remove(name);
            changed_files.push(name.clone());
        }
    }
    changed_files.sort();
    Ok(BatchOutcome {
        archive: out,
        changed_files,
        result,
    })
}
