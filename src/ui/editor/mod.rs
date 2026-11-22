//! The book editor — Bookx's take on Calibre's "Edit book": file browser, code editor with
//! syntax highlighting, live preview, find & replace, table-of-contents editor, metadata and
//! a book checker. Everything operates on an in-memory copy of the EPUB until saved.

use crate::i18n::{tr, trf};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gdk, gio, glib, pango};
use relm4::prelude::*;
use sourceview5 as sv;
use sourceview5::prelude::*;

use super::canvas::{BookCanvas, CanvasStyle, LayoutMode};
use super::metadata::{self, CoverChange};
use super::syntax::Lang;
use super::{alert, system_is_dark, SharedRef};
use crate::core::book::OpenBook;
use crate::core::check::Severity;
use crate::core::format::pretty_print_xml;
use crate::core::html::parse_html;
use crate::core::toc::{self, FlatToc};
use crate::core::util::*;
use crate::editor_prefs::EditorPrefs;
use crate::editor_tools::{self, Tool};
use bookx_editing::checkpoints::Checkpoints;
use bookx_editing::tags::CaseChange;
use bookx_editing::toc_gen::TocOptions;
use tags::TagAction;

mod a11y_form;
mod check_panel;
mod codeview;
mod compare;
mod completion;
mod cover_dialog;
mod css_transform_ui;
mod ctrl_hover;
mod dev;
mod dev9;
mod ext_dev;
mod files;
mod files_extra;
mod fonts_manager;
mod format_actions;
mod gutter;
mod html_transform_ui;
mod insert_dialogs;
mod jacket;
mod keymap;
mod live;
mod live_css;
mod long_line;
mod markup_hints;
mod markup_menu;
mod meta;
mod narration;
mod nav;
mod navigate;
mod palette;
pub(crate) mod plugin_host;
pub mod prefs_page;
mod preview;
mod preview_find;
mod reports;
mod row_data;
mod search;
mod search_bar;
mod search_fn;
mod search_saved;
mod snippets;
mod snippets_ui;
mod special_chars;
mod spelling;
mod spelling_dicts;
mod split;
mod struct_hub;
mod style_actions;
mod tags;
mod toc_dnd;
mod toc_tab;
mod tools;
mod tools_ui;
mod unpack_repack;
mod window_features;

#[derive(Debug, Clone, Copy)]
pub enum TocOp {
    Up,
    Down,
    Indent,
    Outdent,
    Delete,
    Add,
    LinkCurrent,
    Generate,
}

#[derive(Debug)]
pub enum Input {
    SelectFile(String),
    BufferChanged(String),
    Save,
    SaveAndClose,
    ForceClose,
    CloseRequested,
    /// The Undo button of a toast: back to this checkpoint, which is then removed.
    UndoSnapshot(u64),
    Undo,
    Redo,
    ToggleFind,
    /// Open the find bar with the replace row (Ctrl+H).
    ToggleReplace,
    FindNext(bool),
    Replace(bool),
    ReplaceAll,
    GoToLine(String),
    PromptLine,
    MarkSelection,
    FindAll,
    /// The extra search controls: modes, history, saved searches, functions.
    Search(search_bar::SearchAct),
    Check,
    Reformat,
    TogglePreview,
    /// F9: the file sidebar (an overlay when the window is narrow).
    ToggleSidebar,
    RefreshPreview,
    Rehighlight,
    Restyle,
    Live,
    Spell(live::SpellAct),
    ViewOption(live::ViewOpt, bool),
    FindChanged,
    FindCount,
    ManageSnippets,
    SnippetsChanged(Vec<crate::editor_snippets::Snippet>),
    AddFiles,
    AddPaths(Vec<std::path::PathBuf>),
    DeleteFile,
    RenameFile,
    MoveFile(bool),
    SetCover,
    TocSelect(usize),
    TocGo(usize),
    TocTitle(usize, String),
    Toc(TocOp),
    /// The Undo button of the "deleted from the contents" toast.
    TocUndo,
    EditMetadata,
    ApplyMetadata(crate::core::opf::Metadata, CoverChange),
    ResultActivated(usize),
    /// The Problems panel of the book check.
    Problems(check_panel::Act),
    CloseResults,
    RunTool(Tool),
    /// A transform plugin from the Tools menu (plugin id).
    RunPluginTool(String),
    ShowReport,
    CheckSpelling,
    /// The spelling side panel (Tools → Check Spelling).
    SpellPanel(spelling::Msg),
    /// F8 / Shift+F8: the next or previous misspelled word.
    SpellNext(bool),
    /// The Reports window (Tools → Reports).
    Reports(reports::Msg),
    /// Insert special character, Alt+X and entity typing.
    Chars(special_chars::Msg),
    SplitHeadings(u8),
    MergeNext,
    PreviewColors(crate::config::ReaderPrefs),
    CreateCheckpoint,
    RestoreCheckpoint(u64),
    CompareCheckpoint(u64),
    Cursor,
    QuickOpen,
    /// The command palette of the editor (Ctrl+Shift+P).
    Palette,
    StepFile(i32),
    CloseTab,
    TabsEmpty,
    Tag(TagAction, Option<String>),
    /// Code-editing helpers: format, insert, jump, rename class (`format_actions.rs`).
    Code(format_actions::CodeMsg),
    DeleteFiles(Vec<String>),
    /// Delete after asking when the rules of `editor_tools::delete_needs_confirmation` say so.
    AskDeleteFiles(Vec<String>),
    MergeFiles(Vec<String>),
    BulkRename(String, u32),
    NewFile(String, String, bool),
    LinkStyles(Vec<String>, bool),
    ExportTo(std::path::PathBuf),
    PromptMerge,
    PromptBulkRename,
    PromptNewFile,
    PromptLinkStyles,
    PromptExport,
    PromptTocGenerate,
    GenerateToc(TocOptions),
    Dev(String),
    /// Editor preferences, marks and the snippet list (`prefs_page.rs`).
    Prefs(prefs_page::PrefsAct),
    /// Preview sync, splitting, compare view, checkpoints dialog, Live CSS (`preview.rs`).
    Pv(preview::PvMsg),
    /// Tool dialogs, file browser extras and window features (`tools_ui.rs`).
    Ext(tools_ui::Cmd),
}

#[derive(Debug)]
pub enum Output {
    Saved,
    Closed,
    Toast(String),
}

enum ResultKind {
    Search,
    Check,
}

struct ResultRow {
    file: String,
    line: Option<u32>,
    range: Option<(usize, usize)>,
}

struct W {
    win: adw::Window,
    title: adw::WindowTitle,
    view: sv::View,
    map: sv::Map,
    marks_renderer: gutter::MarksRenderer,
    center: gtk::Stack,
    picture: gtk::Picture,
    image_info: gtk::Label,
    tabs: adw::TabView,
    sidebar_stack: adw::ViewStack,
    files: gtk::ListBox,
    toc_list: gtk::ListView,
    toc_store: gio::ListStore,
    toc_sel: gtk::SingleSelection,
    meta_box: gtk::Box,
    results: gtk::ListBox,
    results_rev: gtk::Revealer,
    results_title: gtk::Label,
    problems: check_panel::Widgets,
    find_rev: gtk::Revealer,
    replace_rev: gtk::Revealer,
    find_entry: gtk::SearchEntry,
    replace_entry: gtk::Entry,
    sx: search_bar::Extras,
    case_btn: gtk::ToggleButton,
    dotall_btn: gtk::ToggleButton,
    scope: gtk::DropDown,
    find_status: gtk::Label,
    status: gtk::Label,
    preview: BookCanvas,
    preview_pane: gtk::Box,
    preview_btn: gtk::ToggleButton,
    undo_btn: gtk::Button,
    redo_btn: gtk::Button,
    save_btn: gtk::Button,
    toasts: adw::ToastOverlay,
    banner: adw::Banner,
    split: adw::OverlaySplitView,
    sidebar_btn: gtk::ToggleButton,
    paned: gtk::Paned,
    restore_menu: gio::Menu,
    compare_menu: gio::Menu,
    /// The menus the command palette lists: (name, model).
    menus: Vec<(String, gio::MenuModel)>,
}

pub struct Editor {
    shared: SharedRef,
    book_id: u64,
    book: Rc<RefCell<OpenBook>>,
    sender: ComponentSender<Self>,
    w: W,
    buffers: HashMap<String, sv::Buffer>,
    prefs: EditorPrefs,
    /// What the gutter column shows for the open file.
    gutter_marks: gutter::SharedMarks,
    /// The book at the last checkpoint (id, files), for the change bar.
    baseline: Option<(u64, crate::core::archive::Archive)>,
    /// Text of each file as it was when the editor first showed it.
    original: HashMap<String, String>,
    /// Findings of the last book check, anchored in the text.
    issue_marks: HashMap<String, Vec<(gtk::TextMark, Severity, String)>>,
    spell: Rc<RefCell<live::SpellShared>>,
    spelling: spelling::Panel,
    reports: reports::State,
    chars: special_chars::State,
    live_timer: Rc<RefCell<Option<glib::SourceId>>>,
    search_settings: sv::SearchSettings,
    contexts: HashMap<String, sv::SearchContext>,
    comp: completion::SharedData,
    /// Search regions per file ("Marked text" scope).
    marks: HashMap<String, Vec<(gtk::TextMark, gtk::TextMark)>>,
    modified: HashSet<String>,
    current: Option<String>,
    toc: Vec<FlatToc>,
    /// The contents before the last Delete entry (for its Undo toast) and the selected row.
    toc_undo: RefCell<Option<(Vec<FlatToc>, Option<usize>)>>,
    toc_dirty: bool,
    structure_dirty: bool,
    results: Vec<ResultRow>,
    result_kind: ResultKind,
    problems: check_panel::State,
    file_rows: HashMap<String, (gtk::ListBoxRow, gtk::Image)>,
    preview_timer: Rc<RefCell<Option<glib::SourceId>>>,
    hl_timer: Rc<RefCell<Option<glib::SourceId>>>,
    force_close: Rc<Cell<bool>>,
    toc_selected: Option<usize>,
    /// Snapshots of the book taken before each tool and on request.
    checkpoints: Checkpoints,
    last_toast: RefCell<String>,
    /// Label and action of the button of the last toast (`None`: it had no button); the dev hook presses it.
    last_press: RefCell<Option<(String, Rc<dyn Fn()>)>>,
    tabs_guard: Rc<Cell<bool>>,
    pv: preview::PvState,
    ext: window_features::State,
    hints: Rc<markup_hints::Hints>,
}

/// Ctrl+H: with nothing to find yet the caret goes to Find, otherwise to the replacement.
fn replace_focus_on_find(find_text: &str) -> bool {
    find_text.is_empty()
}

fn is_text_path(path: &str) -> bool {
    matches!(
        extension(path).as_str(),
        "xhtml"
            | "html"
            | "htm"
            | "xht"
            | "css"
            | "xml"
            | "opf"
            | "ncx"
            | "txt"
            | "svg"
            | "js"
            | "smil"
            | "md"
    ) || path == "mimetype"
}

fn file_icon(path: &str) -> &'static str {
    match extension(path).as_str() {
        "xhtml" | "html" | "htm" | "xht" => "text-html-symbolic",
        "css" => "format-text-rich-symbolic",
        "opf" | "ncx" | "xml" => "text-x-generic-symbolic",
        e if is_image_path(&format!("x.{e}")) => "image-x-generic-symbolic",
        "ttf" | "otf" | "woff" | "woff2" => "font-x-generic-symbolic",
        _ => "text-x-generic-symbolic",
    }
}

fn group_of(path: &str) -> &'static str {
    match extension(path).as_str() {
        "xhtml" | "html" | "htm" | "xht" => crate::i18n::N_("Text"),
        "css" => crate::i18n::N_("Styles"),
        "opf" | "ncx" | "xml" => crate::i18n::N_("Package"),
        e if is_image_path(&format!("x.{e}")) => crate::i18n::N_("Images"),
        "ttf" | "otf" | "woff" | "woff2" => crate::i18n::N_("Fonts"),
        _ => crate::i18n::N_("Other"),
    }
}

fn icon_button(icon: &str, tip: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.set_tooltip_text(Some(tip));
    b
}

fn buffer_text(b: &impl IsA<gtk::TextBuffer>) -> String {
    let (s, e) = b.bounds();
    b.text(&s, &e, false).to_string()
}

fn buffer_text_range(
    b: &impl IsA<gtk::TextBuffer>,
    a: &gtk::TextIter,
    e: &gtk::TextIter,
) -> String {
    b.text(a, e, false).to_string()
}

fn set_text_irreversible(b: &impl IsA<gtk::TextBuffer>, text: &str) {
    b.begin_irreversible_action();
    b.set_text(text);
    b.end_irreversible_action();
}

impl Editor {
    fn book_title(&self) -> String {
        self.book.borrow().metadata().title.clone()
    }

    fn is_dirty(&self) -> bool {
        !self.modified.is_empty() || self.toc_dirty || self.structure_dirty
    }

    fn update_title(&self) {
        let t = self.book_title();
        let dirty = self.is_dirty();
        self.w
            .title
            .set_title(&crate::editor_state::header_title(&t, dirty));
        self.w
            .title
            .set_subtitle(&self.current.clone().unwrap_or_else(|| tr("Edit Book")));
        self.w.save_btn.set_sensitive(dirty);
        self.w.win.set_title(Some(&trf("Edit Book — {}", &[&t])));
        let desc = if dirty {
            tr("Unsaved changes")
        } else {
            String::new()
        };
        self.w
            .win
            .upcast_ref::<gtk::Widget>()
            .update_property(&[gtk::accessible::Property::Description(&desc)]);
    }

    fn buffer_for(&mut self, path: &str) -> sv::Buffer {
        if let Some(b) = self.buffers.get(path) {
            return b.clone();
        }
        let buf = codeview::new_buffer(system_is_dark());
        codeview::apply_language(&buf, path);
        codeview::apply_scheme(&buf, system_is_dark(), crate::eink::is_active());
        prefs_page::apply_buffer(&buf);
        let text = self
            .book
            .borrow()
            .archive
            .get_string(path)
            .unwrap_or_default();
        let plain = codeview::too_big_to_colour(&text);
        if plain {
            codeview::mark_plain(&buf);
        }
        set_text_irreversible(&buf, &text);
        buf.set_enable_undo(true);
        if plain {
            self.toast(&tr(
                "Syntax colouring is off for this file because it has very long lines",
            ));
        }
        self.original
            .entry(path.to_string())
            .or_insert_with(|| text.clone());
        let ctx = sv::SearchContext::new(&buf, Some(&self.search_settings));
        ctx.set_highlight(false);
        let tx = self.sender.input_sender().clone();
        ctx.connect_occurrences_count_notify(move |_| tx.emit(Input::FindCount));
        self.contexts.insert(path.to_string(), ctx);
        let tx = self.sender.input_sender().clone();
        let p = path.to_string();
        buf.connect_changed(move |_| tx.emit(Input::BufferChanged(p.clone())));
        format_actions::watch_slash(&buf, self.sender.input_sender().clone());
        let tx = self.sender.input_sender().clone();
        buf.connect_mark_set(move |_, _, mark| {
            if mark.name().as_deref() == Some("insert") {
                tx.emit(Input::Cursor);
            }
        });
        self.buffers.insert(path.to_string(), buf.clone());
        buf
    }

    /// Writes every edited buffer back into the in-memory archive.
    fn flush(&mut self) {
        let paths: Vec<String> = self.modified.iter().cloned().collect();
        for p in paths {
            if let Some(b) = self.buffers.get(&p) {
                self.book
                    .borrow_mut()
                    .archive
                    .set_string(&p, &buffer_text(b));
            }
        }
    }

    /// After the archive changed behind the editor's back (metadata, TOC, rename…).
    fn reload_buffer(&mut self, path: &str) {
        if let Some(b) = self.buffers.get(path).cloned() {
            let text = self
                .book
                .borrow()
                .archive
                .get_string(path)
                .unwrap_or_default();
            if buffer_text(&b) != text {
                self.comp.borrow_mut().edited_ids.remove(path);
                self.comp.borrow_mut().edited_snips.remove(path);
                set_text_irreversible(&b, &text);
                self.modified.remove(path);
                if self.current.as_deref() == Some(path) {
                    self.schedule_highlight();
                }
            }
        }
    }

    fn reload_package_buffers(&mut self) {
        let (opf, ncx, nav) = {
            let b = self.book.borrow();
            (
                Some(b.package.opf_path.clone()),
                b.package.ncx_item().map(|i| b.package.item_path(i)),
                b.package.nav_item().map(|i| b.package.item_path(i)),
            )
        };
        for p in [opf, ncx, nav].into_iter().flatten() {
            self.reload_buffer(&p);
        }
    }

    fn select_file(&mut self, path: &str) {
        if !self.book.borrow().archive.contains(path) {
            return;
        }
        // ids typed into the file we leave are known to the completion even before saving
        if let Some(prev) = self.current.clone().filter(|p| self.modified.contains(p)) {
            if let Some(b) = self.buffers.get(&prev) {
                let ids = bookx_editing::markup::defined_ids(&buffer_text(b));
                self.comp.borrow_mut().edited_ids.insert(prev, ids);
            }
        }
        self.current = Some(path.to_string());
        self.comp.borrow_mut().current = path.to_string();
        self.show_tab(path);
        if let Some((row, _)) = self.file_rows.get(path) {
            if !row.is_selected() {
                self.w.files.select_row(Some(row));
            }
        }
        if is_text_path(path) {
            let buf = self.buffer_for(path);
            if self.long_line_gate(path, &buf) {
                self.refresh_preview_now();
            } else {
                self.show_code(path, &buf);
            }
        } else if is_image_path(path) {
            let data = self
                .book
                .borrow()
                .archive
                .get(path)
                .map(|d| d.to_vec())
                .unwrap_or_default();
            match super::metadata::texture_from_bytes(&data) {
                Some(t) => {
                    self.w.image_info.set_label(&trf(
                        "{} · {}×{} px · {}",
                        &[
                            &file_name(path),
                            &t.width(),
                            &t.height(),
                            &human_size(data.len() as u64),
                        ],
                    ));
                    self.w.picture.set_paintable(Some(&t));
                }
                None => {
                    self.w.picture.set_paintable(gdk::Paintable::NONE);
                    self.w.image_info.set_label(&format!(
                        "{} · {}",
                        file_name(path),
                        human_size(data.len() as u64)
                    ));
                }
            }
            self.w.center.set_visible_child_name("image");
        } else {
            let n = self.book.borrow().archive.size_of(path);
            self.w.image_info.set_label(&trf(
                "{} · {} — binary file, cannot be edited",
                &[&file_name(path), &human_size(n as u64)],
            ));
            self.w.picture.set_paintable(gdk::Paintable::NONE);
            self.w.center.set_visible_child_name("image");
        }
        self.update_title();
    }

    /// Puts `buf` into the code view and shows it.
    fn show_code(&mut self, path: &str, buf: &sv::Buffer) {
        self.w.view.set_buffer(Some(buf));
        self.w.center.set_visible_child_name("code");
        codeview::fallback_highlight(buf, path);
        self.focus_search_context();
        self.refresh_find_status();
        self.schedule_live();
        self.update_cursor_status();
        self.describe_view();
        self.refresh_preview_now();
    }

    fn update_cursor_status(&self) {
        let Some(path) = &self.current else { return };
        let buf = self.w.view.buffer();
        let it = buf.iter_at_mark(&buf.get_insert());
        // a hard-to-see character under the caret is named
        let under = it.char();
        let hidden = if !it.is_end() && !codeview::hidden_chars(&under.to_string()).is_empty() {
            format!("   ·   {}", codeview::describe_hidden(under))
        } else {
            String::new()
        };
        self.w.status.set_label(&trf(
            "{}   ·   Ln {}, Col {}   ·   {} lines{}",
            &[
                &path,
                &(it.line() + 1),
                &(it.line_offset() + 1),
                &buf.line_count(),
                &hidden,
            ],
        ));
        self.w.undo_btn.set_sensitive(buf.can_undo());
        self.w.redo_btn.set_sensitive(buf.can_redo());
    }

    /// The accessible description of the code view (set on status refreshes, not while typing).
    fn describe_view(&self) {
        let marks = self.gutter_marks.borrow();
        let text = crate::editor_marks::view_description(
            &self.current.clone().unwrap_or_default(),
            marks.finding_count(),
            marks.changed_count(),
        );
        self.w
            .view
            .upcast_ref::<gtk::Widget>()
            .update_property(&[gtk::accessible::Property::Description(&text)]);
    }

    /// A failed save stays on screen with a button that tries again (a toast would disappear while the book
    /// stays unsaved).
    fn save_failed(&self, why: &str) {
        let b = &self.w.banner;
        b.set_title(&trf("Could not save the book: {}", &[&why]));
        b.set_button_label(Some(&tr("Try Again")));
        b.set_revealed(true);
    }

    /// For a book of another format: a banner that says Save asks where to write; otherwise the banner is hidden.
    pub(super) fn show_foreign_banner(&self) {
        let b = &self.w.banner;
        let name = self
            .ext
            .structure
            .origin
            .borrow()
            .as_ref()
            .and_then(|o| o.path.file_name().map(|n| n.to_string_lossy().to_string()));
        match name {
            Some(n) => {
                b.set_title(&trf(
                    "Editing a copy of {}. Save asks where to write",
                    &[&n],
                ));
                b.set_button_label(None);
                b.set_revealed(true);
            }
            None => b.set_revealed(false),
        }
    }

    fn toast(&self, t: &str) {
        *self.last_toast.borrow_mut() = t.to_string();
        *self.last_press.borrow_mut() = None;
        let toast = adw::Toast::new(t);
        toast.set_use_markup(false);
        self.w.toasts.add_toast(toast);
    }

    /// A toast with one button (about 10 s) that sends the input `make` builds.
    fn toast_button(&self, t: &str, label: &str, make: impl Fn() -> Input + 'static) -> adw::Toast {
        *self.last_toast.borrow_mut() = t.to_string();
        let tx = self.sender.input_sender().clone();
        let make = Rc::new(make);
        let press: Rc<dyn Fn()> = Rc::new(move || {
            let _ = tx.send(make());
        });
        *self.last_press.borrow_mut() = Some((label.to_string(), press.clone()));
        let toast = adw::Toast::new(t);
        toast.set_use_markup(false);
        toast.set_timeout(10);
        toast.set_button_label(Some(label));
        toast.connect_button_clicked(move |_| press());
        self.w.toasts.add_toast(toast.clone());
        toast
    }

    /// A toast with an Undo button that returns the book to the checkpoint `snapshot` (the state before the
    /// operation) and drops that checkpoint again.
    fn toast_undo(&self, t: &str, snapshot: u64) {
        let _ = self.toast_button(t, &tr("Undo"), move || Input::UndoSnapshot(snapshot));
    }

    /// After a tool changed the archive: the package is read again, every open buffer is rebuilt
    /// from the archive, and the file list, contents and preview follow.
    fn after_archive_changed(&mut self) {
        let _ = self.book.borrow_mut().reload_package();
        self.buffers.clear();
        self.contexts.clear();
        self.issue_marks.clear();
        self.comp.borrow_mut().edited_ids.clear();
        self.comp.borrow_mut().edited_snips.clear();
        self.modified.clear();
        self.structure_dirty = true;
        let keep = self.current.take();
        self.toc = toc::flatten(&self.book.borrow().toc());
        self.toc_dirty = false;
        self.rebuild_files();
        self.rebuild_toc();
        self.rebuild_meta();
        self.update_title();
        let target = keep
            .filter(|p| self.book.borrow().archive.contains(p))
            .or_else(|| self.book.borrow().spine_paths().into_iter().next());
        match target {
            Some(p) => self.select_file(&p),
            None => self.w.center.set_visible_child_name("empty"),
        }
    }

    /// Writes the contents edited in the Contents tab into the in-memory book (nav document / NCX).
    fn commit_toc(&mut self) -> bool {
        if !self.toc_dirty {
            return true;
        }
        let entries = toc::unflatten(&self.toc);
        let res = {
            let mut b = self.book.borrow_mut();
            let pkg = b.package.clone();
            toc::write_toc(&mut b.archive, &pkg, &entries)
        };
        match res {
            Ok(()) => {
                self.reload_package_buffers();
                self.toc_dirty = false;
                true
            }
            Err(e) => {
                self.toast(&trf("Could not write the table of contents: {}", &[&e]));
                false
            }
        }
    }

    // ---- saving ---------------------------------------------------------

    fn save(&mut self) -> bool {
        self.flush();
        if !self.commit_toc() {
            return false;
        }
        if self.ext.structure.origin.borrow().is_some() {
            self.prompt_save_foreign();
            return false;
        }
        let res = {
            let b = self.book.borrow();
            self.shared
                .lib
                .borrow_mut()
                .replace_archive(self.book_id, &b.archive)
        };
        match res {
            Ok(()) => {
                self.modified.clear();
                self.structure_dirty = false;
                for (_, dot) in self.file_rows.values() {
                    dot.set_visible(false);
                }
                let _ = self.book.borrow_mut().reload_package();
                self.update_title();
                self.toast(&tr("Book saved"));
                self.show_foreign_banner();
                let _ = self.sender.output(Output::Saved);
                true
            }
            Err(e) => {
                self.save_failed(&e.to_string());
                false
            }
        }
    }
}

impl SimpleComponent for Editor {
    type Init = (SharedRef, u64);
    type Input = Input;
    type Output = Output;
    type Root = adw::Window;
    type Widgets = ();

    fn init_root() -> Self::Root {
        // An application window: its `app.` actions (Preferences, Keyboard Shortcuts, About) resolve and are enabled
        adw::Window::builder()
            .application(&relm4::main_adw_application())
            .default_width(1380)
            .default_height(860)
            .build()
    }

    fn init(
        (shared, book_id): Self::Init,
        win: adw::Window,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let opened = shared.lib.borrow().open_book(book_id);
        let book = match opened {
            Ok(b) => b,
            Err(e) => {
                let _ = sender.output(Output::Toast(trf(
                    "Cannot open the book for editing: {}",
                    &[&e],
                )));
                OpenBook::from_archive(crate::core::convert::create_epub(
                    &Default::default(),
                    &[],
                    None,
                ))
                .expect("empty book")
            }
        };
        let toc_flat = toc::flatten(&book.toc());
        let book = Rc::new(RefCell::new(book));
        let dark = system_is_dark();

        // ---- editor widgets -------------------------------------------------
        let view = codeview::new_view();
        let prefs = EditorPrefs::load();
        let problems_state = check_panel::State::new(&prefs, book_id);
        let marks: gutter::SharedMarks = Rc::default();
        let marks_renderer = gutter::install(&view, marks.clone());
        codeview::set_whitespace(&view, prefs.show_whitespace);
        prefs_page::apply_view(&view, &crate::editor_prefs::look());
        prefs_page::install_typing(&view);
        let spell: Rc<RefCell<live::SpellShared>> = Rc::default();
        live::install_spell_menu(&view, spell.clone(), sender.input_sender().clone());
        let chars_state = special_chars::State::new(prefs.convert_entities);
        special_chars::install_typing(&view, chars_state.entities.clone());
        let spelling_panel = spelling::Panel::new(sender.input_sender().clone());
        let comp: completion::SharedData = Rc::new(RefCell::new(completion::Data {
            current: String::new(),
            files: Vec::new(),
            classes: Vec::new(),
            words: Vec::new(),
            edited_ids: HashMap::new(),
            edited_snips: HashMap::new(),
            book: book.clone(),
            snippets: crate::editor_snippets::merged(&crate::editor_snippets::load()),
            region: Rc::default(),
            populated: (0, 0),
            waits: Default::default(),
        }));
        completion::install(&view, comp.clone());
        let hints = markup_hints::Hints::new();
        markup_hints::install_tooltips(&view, hints.clone());
        ctrl_hover::install(&view);
        navigate::install(&view, sender.input_sender().clone());
        format_actions::install_keys(&view, sender.input_sender().clone(), hints.clone());
        {
            // Tab expands a snippet trigger before the caret (and keeps moving between tab stops
            // while the caret is inside an expanded snippet)
            let keys = gtk::EventControllerKey::new();
            keys.set_propagation_phase(gtk::PropagationPhase::Capture);
            let v = view.clone();
            let c = comp.clone();
            keys.connect_key_pressed(move |_, key, _, state| {
                let plain = !state.intersects(
                    gdk::ModifierType::CONTROL_MASK
                        | gdk::ModifierType::SHIFT_MASK
                        | gdk::ModifierType::ALT_MASK,
                );
                if key == gdk::Key::Tab && plain && completion::expand_at_caret(&v, &c) {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            });
            view.add_controller(keys);
        }
        {
            let tx = sender.input_sender().clone();
            if let Some(adj) = gtk::prelude::ScrollableExt::vadjustment(&view) {
                adj.connect_value_changed(move |_| tx.emit(Input::Live));
            }
        }
        let search_settings = sv::SearchSettings::new();
        search_settings.set_wrap_around(true);
        let code_scroll = gtk::ScrolledWindow::new();
        code_scroll.set_child(Some(&view));
        code_scroll.set_vexpand(true);
        code_scroll.set_hexpand(true);
        let map = sv::Map::new();
        crate::ui::audit::label(&map, &tr("Minimap of the file"));
        map.set_view(&view);
        map.set_width_request(110);
        map.set_visible(prefs.minimap);
        let code_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        code_row.append(&code_scroll);
        code_row.append(&map);

        let picture = gtk::Picture::new();
        picture.set_content_fit(gtk::ContentFit::ScaleDown);
        picture.set_margin_all(24);
        picture.set_vexpand(true);
        let image_info = gtk::Label::new(None);
        image_info.add_css_class("dim-label");
        image_info.set_margin_bottom(8);
        let set_cover_btn = gtk::Button::with_label(&tr("Use as Cover"));
        set_cover_btn.add_css_class("pill");
        set_cover_btn.set_halign(gtk::Align::Center);
        set_cover_btn.set_margin_bottom(16);
        {
            let tx = sender.input_sender().clone();
            set_cover_btn.connect_clicked(move |_| tx.emit(Input::SetCover));
        }
        let image_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        image_page.append(&picture);
        image_page.append(&image_info);
        image_page.append(&set_cover_btn);

        let empty = adw::StatusPage::builder()
            .icon_name("document-edit-symbolic")
            .title(tr("Select a File"))
            .description(tr(
                "Pick a chapter, stylesheet or image from the Files list to start editing",
            ))
            .build();

        let ext_state = window_features::State::new();
        {
            // A dev script can wait for the real file monitor instead of guessing how long an
            // external editor takes on a loaded machine.
            let pending = ext_state.pending_external.clone();
            crate::devtools::probe("editor-external-pending", move || {
                pending.borrow().is_some()
            });
        }
        let center = gtk::Stack::new();
        center.add_named(&empty, Some("empty"));
        center.add_named(&code_row, Some("code"));
        center.add_named(&image_page, Some("image"));
        center.add_named(
            &long_line::long_line_page(sender.input_sender().clone(), &ext_state.structure),
            Some("longline"),
        );
        center.set_visible_child_name("empty");

        // preview
        let preview = BookCanvas::new();
        {
            let b = book.clone();
            preview.set_image_provider(move |p| b.borrow().archive.get(p).map(|d| d.to_vec()));
            let mut st: CanvasStyle = shared.cfg.borrow().reader.canvas_style(dark);
            st.layout = LayoutMode::Scroll;
            st.margin = 24.0;
            preview.set_style(prefs_page::preview_style(st, &crate::editor_prefs::look()));
        }
        let preview_title = gtk::Label::new(Some(&tr("Preview")));
        preview_title.add_css_class("caption-heading");
        preview_title.set_xalign(0.0);
        preview_title.set_hexpand(true);
        let colors_btn = gtk::MenuButton::new();
        colors_btn.set_icon_name("applications-graphics-symbolic");
        colors_btn.set_tooltip_text(Some(&tr(
            "Preview colours: theme, the book's original colours, or your own",
        )));
        colors_btn.add_css_class("flat");
        {
            let cur = Rc::new(RefCell::new(shared.cfg.borrow().reader.clone()));
            let tx = sender.input_sender().clone();
            let cur2 = cur.clone();
            let emit: Rc<dyn Fn()> =
                Rc::new(move || tx.emit(Input::PreviewColors(cur2.borrow().clone())));
            let controls = super::colors::ColorControls::new(cur, emit);
            controls.root.set_margin_all(10);
            let pop = gtk::Popover::new();
            pop.set_child(Some(&controls.root));
            colors_btn.set_popover(Some(&pop));
        }
        let preview_head = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        preview_head.set_margin_all(6);
        preview_head.append(&preview_title);
        preview_head.append(&colors_btn);
        let preview_pane = gtk::Box::new(gtk::Orientation::Vertical, 0);
        preview_pane.append(&preview_head);
        preview_pane.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        preview_pane.append(&preview.area);
        preview_pane.set_width_request(360);

        let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
        paned.set_start_child(Some(&center));
        paned.set_end_child(Some(&preview_pane));
        paned.set_resize_start_child(true);
        paned.set_resize_end_child(true);
        paned.set_shrink_end_child(false);
        paned.set_position(760);

        // ---- find bar ----------------------------------------------------------
        let find_entry = gtk::SearchEntry::new();
        find_entry.set_placeholder_text(Some(&tr("Find")));
        find_entry.set_hexpand(true);
        let replace_entry = gtk::Entry::new();
        replace_entry.set_placeholder_text(Some(&tr("Replace with")));
        replace_entry.set_hexpand(true);
        let case_btn = gtk::ToggleButton::with_label(&tr("Aa"));
        case_btn.set_tooltip_text(Some(&tr("Match case")));
        crate::ui::audit::label(&case_btn, &tr("Match case"));
        let dotall_btn = gtk::ToggleButton::with_label(&tr(".\u{23ce}"));
        dotall_btn.set_tooltip_text(Some(&tr("Dot also matches line breaks")));
        crate::ui::audit::label(&dotall_btn, &tr("Dot matches line breaks"));
        let sx = search_bar::build(sender.input_sender(), &replace_entry, &dotall_btn);
        let scope =
            gtk::DropDown::from_strings(&crate::editor_search::Scope::ALL.map(|s| s.label()));
        crate::ui::audit::label(&scope, &tr("Search in"));
        let prev = icon_button("go-up-symbolic", &tr("Previous match"));
        let next = icon_button("go-down-symbolic", &tr("Next match (Enter)"));
        let repl = gtk::Button::with_label(&tr("Replace"));
        let repl_all = gtk::Button::with_label(&tr("Replace All"));
        let find_all = gtk::Button::with_label(&tr("Find All"));
        let find_status = gtk::Label::new(None);
        find_status.add_css_class("dim-label");
        find_status.add_css_class("caption");
        let find_close = icon_button("window-close-symbolic", &tr("Close"));
        find_close.add_css_class("flat");
        let options_btn = gtk::MenuButton::new();
        options_btn.set_icon_name("emblem-system-symbolic");
        options_btn.set_tooltip_text(Some(&tr("Search options: mode, case, whole words, scope")));
        crate::ui::audit::label(&options_btn, &tr("Search Options"));
        options_btn.add_css_class("flat");
        {
            let opts = adw::WrapBox::new();
            opts.set_child_spacing(6);
            opts.set_line_spacing(6);
            opts.set_natural_line_length(520);
            for w in [
                sx.mode.upcast_ref::<gtk::Widget>(),
                case_btn.upcast_ref(),
                sx.word_btn.upcast_ref(),
                dotall_btn.upcast_ref(),
                sx.markup_btn.upcast_ref(),
                scope.upcast_ref(),
                sx.fn_dd.upcast_ref(),
                sx.special.upcast_ref(),
                sx.more.upcast_ref(),
            ] {
                opts.append(w);
            }
            opts.set_margin_all(10);
            let pop = gtk::Popover::new();
            pop.set_child(Some(&opts));
            options_btn.set_popover(Some(&pop));
        }
        let row1 = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        for w in [
            find_entry.upcast_ref::<gtk::Widget>(),
            sx.find_hist.upcast_ref(),
            prev.upcast_ref(),
            next.upcast_ref(),
            find_all.upcast_ref(),
            sx.count_btn.upcast_ref(),
            find_status.upcast_ref(),
            options_btn.upcast_ref(),
        ] {
            row1.append(w);
        }
        let row2 = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        for w in [
            replace_entry.upcast_ref::<gtk::Widget>(),
            sx.repl_hist.upcast_ref(),
            repl.upcast_ref(),
            repl_all.upcast_ref(),
        ] {
            row2.append(w);
        }
        let replace_rev = gtk::Revealer::new();
        replace_rev.set_child(Some(&row2));
        replace_rev.set_margin_top(6);
        let find_col = gtk::Box::new(gtk::Orientation::Vertical, 0);
        find_col.append(&row1);
        find_col.append(&replace_rev);
        find_col.set_hexpand(true);
        let find_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        find_box.set_margin_top(4);
        find_box.set_margin_bottom(4);
        find_box.set_margin_start(8);
        find_box.set_margin_end(8);
        find_box.append(&find_col);
        find_box.append(&find_close);
        find_box.set_valign(gtk::Align::Start);
        let find_rev = gtk::Revealer::new();
        find_rev.set_child(Some(&find_box));
        find_rev.add_css_class("toolbar");
        {
            let tx = sender.input_sender().clone();
            next.connect_clicked(move |_| tx.emit(Input::FindNext(true)));
            let tx = sender.input_sender().clone();
            prev.connect_clicked(move |_| tx.emit(Input::FindNext(false)));
            let tx = sender.input_sender().clone();
            find_entry.connect_activate(move |_| tx.emit(Input::FindNext(true)));
            let tx = sender.input_sender().clone();
            find_entry.connect_next_match(move |_| tx.emit(Input::FindNext(true)));
            let tx = sender.input_sender().clone();
            find_entry.connect_previous_match(move |_| tx.emit(Input::FindNext(false)));
            let tx = sender.input_sender().clone();
            repl.connect_clicked(move |_| tx.emit(Input::Replace(true)));
            let tx = sender.input_sender().clone();
            repl_all.connect_clicked(move |_| tx.emit(Input::ReplaceAll));
            let tx = sender.input_sender().clone();
            find_all.connect_clicked(move |_| tx.emit(Input::FindAll));
            let tx = sender.input_sender().clone();
            find_entry.connect_search_changed(move |_| tx.emit(Input::FindChanged));
            for b in [&case_btn, &dotall_btn] {
                let tx = sender.input_sender().clone();
                b.connect_toggled(move |_| tx.emit(Input::FindChanged));
            }
            let tx = sender.input_sender().clone();
            find_close.connect_clicked(move |_| tx.emit(Input::ToggleFind));
            let tx = sender.input_sender().clone();
            find_entry.connect_stop_search(move |_| tx.emit(Input::ToggleFind));
        }

        // ---- results panel -------------------------------------------------------
        let results = gtk::ListBox::new();
        results.add_css_class("boxed-list-separate");
        results.set_selection_mode(gtk::SelectionMode::Single);
        let results_scroll = gtk::ScrolledWindow::new();
        results_scroll.set_min_content_height(160);
        results_scroll.set_max_content_height(240);
        results_scroll.set_propagate_natural_height(true);
        results_scroll.set_child(Some(&results));
        let results_title = gtk::Label::new(None);
        results_title.add_css_class("heading");
        results_title.set_xalign(0.0);
        results_title.set_hexpand(true);
        let results_close = icon_button("window-close-symbolic", &tr("Close"));
        results_close.add_css_class("flat");
        let results_head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        results_head.set_margin_all(6);
        results_head.append(&results_title);
        results_head.append(&results_close);
        let results_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        results_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        results_box.append(&results_head);
        results_box.append(&results_scroll);
        let results_rev = gtk::Revealer::new();
        results_rev.set_child(Some(&results_box));
        {
            let tx = sender.input_sender().clone();
            results_close.connect_clicked(move |_| tx.emit(Input::CloseResults));
            let tx = sender.input_sender().clone();
            results.connect_row_activated(move |_, r| {
                tx.emit(Input::ResultActivated(r.index() as usize))
            });
        }

        let status = gtk::Label::new(None);
        status.add_css_class("editor-status");
        status.set_xalign(0.0);

        // ---- sidebar ---------------------------------------------------------------
        let files = gtk::ListBox::new();
        files.add_css_class("navigation-sidebar");
        files.set_selection_mode(gtk::SelectionMode::Multiple);
        files.set_header_func(|row, before| {
            let g = row.widget_name();
            if before.map(|b| b.widget_name()) != Some(g.clone()) {
                let l = gtk::Label::new(Some(&tr(g.as_str())));
                l.add_css_class("caption-heading");
                l.add_css_class("dim-label");
                l.set_xalign(0.0);
                l.set_margin_start(10);
                l.set_margin_top(10);
                l.set_margin_bottom(2);
                row.set_header(Some(&l));
            } else {
                row.set_header(None::<&gtk::Widget>);
            }
        });
        {
            let tx = sender.input_sender().clone();
            files.connect_row_activated(move |_, r| {
                if let Some(p) = row_data::path(r) {
                    tx.emit(Input::SelectFile(p));
                }
            });
            let tx = sender.input_sender().clone();
            files.connect_selected_rows_changed(move |l| {
                let rows = l.selected_rows();
                if let [r] = rows.as_slice() {
                    if let Some(p) = row_data::path(r) {
                        tx.emit(Input::SelectFile(p));
                    }
                }
            });
            let keys = gtk::EventControllerKey::new();
            let tx = sender.input_sender().clone();
            keys.connect_key_pressed(move |_, key, _, _| match key {
                gdk::Key::Delete => {
                    tx.emit(Input::DeleteFile);
                    glib::Propagation::Stop
                }
                gdk::Key::F2 => {
                    tx.emit(Input::RenameFile);
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            });
            files.add_controller(keys);
        }
        let files_scroll = gtk::ScrolledWindow::new();
        files_scroll.set_vexpand(true);
        files_scroll.set_child(Some(&files));
        let files_bar = adw::WrapBox::new();
        files_bar.set_margin_all(6);
        files_bar.set_child_spacing(2);
        let mk = |icon: &str, tip: &str| {
            let b = icon_button(icon, tip);
            b.add_css_class("flat");
            b
        };
        let add_b = mk("list-add-symbolic", &tr("Add files"));
        let up_b = mk("go-up-symbolic", &tr("Move up in reading order"));
        let down_b = mk("go-down-symbolic", &tr("Move down in reading order"));
        {
            let tx = sender.input_sender().clone();
            add_b.connect_clicked(move |_| tx.emit(Input::AddFiles));
            let tx = sender.input_sender().clone();
            up_b.connect_clicked(move |_| tx.emit(Input::MoveFile(true)));
            let tx = sender.input_sender().clone();
            down_b.connect_clicked(move |_| tx.emit(Input::MoveFile(false)));
        }
        let files_menu = gio::Menu::new();
        {
            let a = gio::Menu::new();
            a.append(Some(&tr("New File")), Some("files.new"));
            a.append(Some(&tr("Add Files")), Some("files.add"));
            files_menu.append_section(None, &a);
            let m = gio::Menu::new();
            m.append(Some(&tr("Merge Selected")), Some("files.merge"));
            m.append(Some(&tr("Bulk Rename")), Some("files.bulk"));
            m.append(Some(&tr("Link Stylesheets")), Some("files.link"));
            m.append(Some(&tr("Export Selected")), Some("files.export"));
            files_menu.append_section(None, &m);
            let d = gio::Menu::new();
            d.append(Some(&tr("Rename")), Some("files.rename"));
            d.append(Some(&tr("Delete Selected")), Some("files.delete"));
            files_menu.append_section(None, &d);
        }
        let files_more = gtk::MenuButton::new();
        files_more.set_icon_name("view-more-symbolic");
        files_more.set_tooltip_text(Some(&tr(
            "More file actions: rename, delete, merge, bulk rename, link stylesheets, export",
        )));
        files_more.set_menu_model(Some(&files_menu));
        files_more.add_css_class("flat");
        {
            let ctx = gtk::PopoverMenu::from_model(Some(&files_menu));
            ctx.set_parent(&files_scroll);
            ctx.set_has_arrow(false);
            let ctx_gone = ctx.clone();
            files_scroll.connect_destroy(move |_| ctx_gone.unparent());
            let click = gtk::GestureClick::new();
            click.set_button(3);
            let f = files.clone();
            let sc = files_scroll.clone();
            click.connect_pressed(move |_, _, x, y| {
                if let Some(row) = f.row_at_y(y as i32) {
                    if !row.is_selected() {
                        f.unselect_all();
                        f.select_row(Some(&row));
                    }
                }
                let (px, py) = f
                    .compute_point(&sc, &gtk::graphene::Point::new(x as f32, y as f32))
                    .map(|p| (p.x() as f64, p.y() as f64))
                    .unwrap_or((x, y));
                ctx.set_pointing_to(Some(&gdk::Rectangle::new(px as i32, py as i32, 1, 1)));
                ctx.popup();
            });
            files.add_controller(click);
        }
        for b in [&add_b, &up_b, &down_b] {
            files_bar.append(b);
        }
        files_bar.append(&files_more);
        let files_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        files_page.append(&files_scroll);
        files_page.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        files_page.append(&files_bar);

        let toc_ui = toc_tab::toc_list_new(sender.input_sender().clone(), &ext_state.structure);
        let (toc_list, toc_store, toc_sel, toc_scroll) =
            (toc_ui.list, toc_ui.store, toc_ui.sel, toc_ui.root);
        let toc_bar = adw::WrapBox::new();
        toc_bar.set_margin_all(6);
        toc_bar.set_child_spacing(2);
        toc_bar.set_line_spacing(2);
        for (icon, tip, op) in [
            (
                "list-add-symbolic",
                crate::i18n::N_("Add entry for the current file"),
                TocOp::Add,
            ),
            (
                "user-trash-symbolic",
                crate::i18n::N_("Delete entry"),
                TocOp::Delete,
            ),
            (
                "insert-link-symbolic",
                crate::i18n::N_("Point entry at the current file"),
                TocOp::LinkCurrent,
            ),
        ] {
            let b = icon_button(icon, &tr(tip));
            b.add_css_class("flat");
            let tx = sender.input_sender().clone();
            b.connect_clicked(move |_| tx.emit(Input::Toc(op)));
            toc_bar.append(&b);
        }
        {
            // Up, Down, Indent and Outdent share one menu (Alt+arrows and drag and drop stay)
            let move_menu = gio::Menu::new();
            for (label, id) in [
                (tr("Move Up"), "up"),
                (tr("Move Down"), "down"),
                (tr("Indent (Make a Sub-Entry)"), "indent"),
                (tr("Outdent"), "outdent"),
            ] {
                move_menu.append_item(&tools_ui::menu_item(&label, "toc.move", id));
            }
            let group = gio::SimpleActionGroup::new();
            let act = gio::SimpleAction::new("move", Some(glib::VariantTy::STRING));
            let tx = sender.input_sender().clone();
            act.connect_activate(move |_, p| {
                let op = match p.and_then(|v| v.str()) {
                    Some("up") => TocOp::Up,
                    Some("down") => TocOp::Down,
                    Some("indent") => TocOp::Indent,
                    Some("outdent") => TocOp::Outdent,
                    _ => return,
                };
                tx.emit(Input::Toc(op));
            });
            group.add_action(&act);
            toc_bar.insert_action_group("toc", Some(&group));
            let mb = gtk::MenuButton::new();
            mb.set_icon_name("view-more-symbolic");
            mb.set_tooltip_text(Some(&tr("Move the entry: up, down, in, out")));
            crate::ui::audit::label(&mb, &tr("Move entry"));
            mb.add_css_class("flat");
            mb.set_menu_model(Some(&move_menu));
            toc_bar.append(&mb);
        }
        let r#gen = gtk::Button::with_label(&tr("Generate"));
        r#gen.set_tooltip_text(Some(&tr(
            "Build the table of contents: from headings, links or files",
        )));
        r#gen.add_css_class("flat");
        {
            let tx = sender.input_sender().clone();
            r#gen.connect_clicked(move |_| tx.emit(Input::PromptTocGenerate));
        }
        toc_bar.append(&r#gen);
        let toc_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        toc_page.append(&toc_scroll);
        toc_page.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        toc_page.append(&toc_bar);

        let meta_box = gtk::Box::new(gtk::Orientation::Vertical, 0);

        let sidebar_stack = adw::ViewStack::new();
        sidebar_stack.add_titled_with_icon(
            &files_page,
            Some("files"),
            &tr("Files"),
            "folder-symbolic",
        );
        sidebar_stack.add_titled_with_icon(
            &toc_page,
            Some("toc"),
            &tr("Contents"),
            "view-list-symbolic",
        );
        sidebar_stack.add_titled_with_icon(
            &meta_box,
            Some("meta"),
            &tr("Metadata"),
            "document-properties-symbolic",
        );
        // the sidebar is 250-340 px wide: icons only, the titles are the tooltips and accessible names
        let switcher_inner = adw::ViewSwitcher::new();
        switcher_inner.set_stack(Some(&sidebar_stack));
        switcher_inner.set_policy(adw::ViewSwitcherPolicy::Narrow);
        switcher_inner.set_hexpand(true);
        let switcher = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        switcher.add_css_class("toolbar");
        switcher.append(&switcher_inner);

        let side_header = adw::HeaderBar::new();
        side_header.set_show_end_title_buttons(false);
        side_header.set_show_start_title_buttons(false);
        side_header.set_title_widget(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
        let side_tb = adw::ToolbarView::new();
        side_tb.add_top_bar(&side_header);
        side_tb.set_content(Some(&sidebar_stack));
        side_tb.add_bottom_bar(&switcher);

        // ---- header ----------------------------------------------------------------
        let title = adw::WindowTitle::new("", "");
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&title));
        let save_btn = gtk::Button::with_label(&tr("Save"));
        save_btn.add_css_class("suggested-action");
        save_btn.set_tooltip_text(Some(&tr("Save changes to the library (Ctrl+S)")));
        let undo_btn = icon_button("edit-undo-symbolic", &tr("Undo (Ctrl+Z)"));
        let redo_btn = icon_button("edit-redo-symbolic", &tr("Redo (Ctrl+Shift+Z)"));
        let find_btn = gtk::ToggleButton::new();
        find_btn.set_icon_name("edit-find-replace-symbolic");
        find_btn.set_tooltip_text(Some(&tr("Find (Ctrl+F), Replace (Ctrl+H)")));
        let preview_btn = gtk::ToggleButton::new();
        preview_btn.set_icon_name("sidebar-show-right-symbolic");
        preview_btn.set_tooltip_text(Some(&tr("Hide Preview")));
        crate::ui::audit::label(&preview_btn, &tr("Preview"));
        preview_btn.set_active(true);
        {
            preview_btn.connect_active_notify(|b| {
                b.set_tooltip_text(Some(&if b.is_active() {
                    tr("Hide Preview")
                } else {
                    tr("Show Preview")
                }));
            });
        }
        header.pack_start(&save_btn);
        header.pack_start(&undo_btn);
        header.pack_start(&redo_btn);
        // ---- tools menu: one-click operations of `bookx-editing`, each snapshotted first ----
        let restore_menu = gio::Menu::new();
        let compare_menu = gio::Menu::new();
        let loaded = crate::plugins::current();
        let plugin_menu = gio::Menu::new();
        {
            let plugin_tools = gio::Menu::new();
            for t in loaded.transforms.iter() {
                let name = loaded
                    .names
                    .get(t.id())
                    .cloned()
                    .unwrap_or_else(|| t.id().to_string());
                let it = gio::MenuItem::new(Some(&name), None);
                it.set_action_and_target_value(
                    Some("tools.run"),
                    Some(&format!("plugin:{}", t.id()).to_variant()),
                );
                plugin_tools.append_item(&it);
            }
            if plugin_tools.n_items() > 0 {
                plugin_menu.append_section(None, &plugin_tools);
            }
            if let Some(section) = plugin_host::menu_items(&loaded) {
                plugin_menu.append_section(None, &section);
            }
        }
        let (tools_menu, cp_menu) = tools_ui::build_tools_menu(
            &restore_menu,
            &compare_menu,
            (plugin_menu.n_items() > 0).then_some(&plugin_menu),
        );
        let tools_btn = gtk::MenuButton::new();
        tools_btn.set_icon_name("applications-engineering-symbolic");
        tools_btn.set_tooltip_text(Some(&tr(
            "Tools: check, fix, upgrade, compress, report, checkpoints",
        )));
        crate::ui::audit::label(&tools_btn, &tr("Tools"));
        tools_btn.set_menu_model(Some(&tools_menu));
        let markup_built = markup_menu::build();
        let edit_menu = markup_built.root.clone();
        let edit_btn = gtk::MenuButton::new();
        edit_btn.set_icon_name("format-text-markup-symbolic");
        edit_btn.set_tooltip_text(Some(&tr(
            "Markup: matching tags, wrap, rename, headings, case",
        )));
        edit_btn.set_menu_model(Some(&edit_menu));
        crate::ui::audit::label(&edit_btn, &tr("Markup"));
        let book_btn = window_features::book_button();
        header.pack_end(&book_btn);
        header.pack_end(&edit_btn);
        header.pack_end(&tools_btn);
        header.pack_end(&preview_btn);
        header.pack_end(&find_btn);
        {
            let tx = sender.input_sender().clone();
            save_btn.connect_clicked(move |_| tx.emit(Input::Save));
            let tx = sender.input_sender().clone();
            undo_btn.connect_clicked(move |_| tx.emit(Input::Undo));
            let tx = sender.input_sender().clone();
            redo_btn.connect_clicked(move |_| tx.emit(Input::Redo));
            let tx = sender.input_sender().clone();
            find_btn.connect_clicked(move |_| tx.emit(Input::ToggleFind));
            let tx = sender.input_sender().clone();
            preview_btn.connect_toggled(move |_| tx.emit(Input::TogglePreview));
        }
        {
            let b = find_btn.clone();
            find_rev.connect_child_revealed_notify(move |r| b.set_active(r.reveals_child()));
        }

        // ---- layout ---------------------------------------------------------------
        let tabs = adw::TabView::new();
        tabs.set_visible(false);
        let tab_bar = adw::TabBar::new();
        tab_bar.set_view(Some(&tabs));
        tab_bar.set_autohide(false);
        let tabs_guard = Rc::new(Cell::new(false));
        {
            let tx = sender.input_sender().clone();
            let guard = tabs_guard.clone();
            tabs.connect_selected_page_notify(move |t| {
                if guard.get() {
                    return;
                }
                if let Some(p) = t.selected_page().and_then(|pg| row_data::path(&pg)) {
                    tx.emit(Input::SelectFile(p));
                }
            });
            let tx = sender.input_sender().clone();
            let guard = tabs_guard.clone();
            tabs.connect_n_pages_notify(move |t| {
                if t.n_pages() == 0 && !guard.get() {
                    tx.emit(Input::TabsEmpty);
                }
            });
        }
        let main_col = gtk::Box::new(gtk::Orientation::Vertical, 0);
        main_col.append(&tab_bar);
        main_col.append(&tabs);
        main_col.append(&find_rev);
        let paned_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        paned.set_hexpand(true);
        paned_row.append(&paned);
        paned_row.append(&spelling_panel.widget());
        paned_row.set_vexpand(true);
        main_col.append(&paned_row);
        paned.set_vexpand(true);
        main_col.append(&results_rev);
        let problems_w = check_panel::build(&sender);
        main_col.append(&problems_w.root);
        main_col.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        main_col.append(&status);

        let split = adw::OverlaySplitView::new();
        split.set_sidebar(Some(&side_tb));
        split.set_content(Some(&main_col));
        split.set_min_sidebar_width(250.0);
        split.set_max_sidebar_width(340.0);
        split.set_sidebar_width_fraction(0.2);

        let sidebar_btn = gtk::ToggleButton::new();
        sidebar_btn.set_icon_name("sidebar-show-symbolic");
        sidebar_btn.set_tooltip_text(Some(&tr("Show Sidebar")));
        crate::ui::audit::label(&sidebar_btn, &tr("Sidebar"));
        sidebar_btn.set_visible(false);
        split
            .bind_property("show-sidebar", &sidebar_btn, "active")
            .bidirectional()
            .sync_create()
            .build();
        split
            .bind_property("collapsed", &sidebar_btn, "visible")
            .sync_create()
            .build();
        header.pack_start(&sidebar_btn);
        win.set_width_request(360);
        win.set_height_request(294);
        {
            // 900x600 and narrower: the file sidebar slides over the code, the minimap goes, and below 700sp the
            // preview sits under the code instead of beside it
            let wide = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
                adw::BreakpointConditionLengthType::MaxWidth,
                1000.0,
                adw::LengthUnit::Sp,
            ));
            wide.add_setter(&split, "collapsed", Some(&true.to_value()));
            wide.add_setter(&split, "show-sidebar", Some(&false.to_value()));
            wide.add_setter(&map, "visible", Some(&false.to_value()));
            wide.add_setter(&preview_pane, "width-request", Some(&280i32.to_value()));
            win.add_breakpoint(wide);
            let narrow = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
                adw::BreakpointConditionLengthType::MaxWidth,
                700.0,
                adw::LengthUnit::Sp,
            ));
            narrow.add_setter(
                &paned,
                "orientation",
                Some(&gtk::Orientation::Vertical.to_value()),
            );
            win.add_breakpoint(narrow);
        }
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&split));
        let banner = adw::Banner::new("");
        {
            let tx = sender.input_sender().clone();
            banner.connect_button_clicked(move |_| tx.emit(Input::Save));
        }
        toolbar.add_top_bar(&banner);
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&toolbar));
        {
            let tools = gio::SimpleActionGroup::new();
            let run = gio::SimpleAction::new("run", Some(glib::VariantTy::STRING));
            let tx = sender.input_sender().clone();
            run.connect_activate(move |_, p| {
                let id = p.and_then(|v| v.str());
                if let Some(t) = id.and_then(Tool::from_id) {
                    tx.emit(Input::RunTool(t));
                } else if let Some(pid) = id.and_then(|i| i.strip_prefix("plugin:")) {
                    tx.emit(Input::RunPluginTool(pid.to_string()));
                } else if let Some(n) = id.filter(|i| i.starts_with("narration:")) {
                    tx.emit(Input::RunPluginTool(n.to_string()));
                }
            });
            tools.add_action(&run);
            for (name, make) in [
                ("check", (|| Input::Check) as fn() -> Input),
                ("reformat", || Input::Reformat),
            ] {
                let act = gio::SimpleAction::new(name, None);
                let tx = sender.input_sender().clone();
                act.connect_activate(move |_, _| tx.emit(make()));
                tools.add_action(&act);
            }
            let report = gio::SimpleAction::new("report", None);
            let tx = sender.input_sender().clone();
            report.connect_activate(move |_, _| tx.emit(Input::ShowReport));
            tools.add_action(&report);
            for (name, forward) in [("next-problem", true), ("previous-problem", false)] {
                let act = gio::SimpleAction::new(name, None);
                let tx = sender.input_sender().clone();
                act.connect_activate(move |_, _| {
                    tx.emit(Input::Problems(if forward {
                        check_panel::Act::Next
                    } else {
                        check_panel::Act::Prev
                    }))
                });
                tools.add_action(&act);
            }
            for name in ["split", "merge"] {
                let act = gio::SimpleAction::new(name, None);
                let tx = sender.input_sender().clone();
                act.connect_activate(move |_, _| {
                    tx.emit(if name == "split" {
                        Input::SplitHeadings(1)
                    } else {
                        Input::MergeNext
                    })
                });
                tools.add_action(&act);
            }
            let spell = gio::SimpleAction::new("spelling", None);
            let tx = sender.input_sender().clone();
            spell.connect_activate(move |_, _| tx.emit(Input::CheckSpelling));
            tools.add_action(&spell);
            let cp = gio::SimpleAction::new("checkpoint", None);
            let tx = sender.input_sender().clone();
            cp.connect_activate(move |_, _| tx.emit(Input::CreateCheckpoint));
            tools.add_action(&cp);
            let restore = gio::SimpleAction::new("restore", Some(glib::VariantTy::STRING));
            let tx = sender.input_sender().clone();
            restore.connect_activate(move |_, p| {
                if let Some(id) = p.and_then(|v| v.str()).and_then(|s| s.parse().ok()) {
                    tx.emit(Input::RestoreCheckpoint(id));
                }
            });
            tools.add_action(&restore);
            let compare = gio::SimpleAction::new("compare", Some(glib::VariantTy::STRING));
            let tx = sender.input_sender().clone();
            compare.connect_activate(move |_, p| {
                if let Some(id) = p.and_then(|v| v.str()).and_then(|s| s.parse().ok()) {
                    tx.emit(Input::CompareCheckpoint(id));
                }
            });
            tools.add_action(&compare);
            win.insert_action_group("tools", Some(&tools));
            let edit = gio::SimpleActionGroup::new();
            let tag = gio::SimpleAction::new("tag", Some(glib::VariantTy::STRING));
            let tx = sender.input_sender().clone();
            tag.connect_activate(move |_, p| {
                if let Some(a) = p.and_then(|v| v.str()).and_then(TagAction::from_id) {
                    tx.emit(Input::Tag(a, None));
                }
            });
            edit.add_action(&tag);
            let code = gio::SimpleAction::new("code", Some(glib::VariantTy::STRING));
            let tx = sender.input_sender().clone();
            code.connect_activate(move |_, p| {
                if let Some(m) = p
                    .and_then(|v| v.str())
                    .and_then(format_actions::code_msg_for)
                {
                    tx.emit(Input::Code(m));
                }
            });
            edit.add_action(&code);
            win.insert_action_group("edit", Some(&edit));
            let view_actions = gio::SimpleActionGroup::new();
            for (name, opt, on) in [
                ("minimap", live::ViewOpt::Minimap, prefs.minimap),
                (
                    "whitespace",
                    live::ViewOpt::Whitespace,
                    prefs.show_whitespace,
                ),
            ] {
                let act = gio::SimpleAction::new_stateful(name, None, &on.to_variant());
                let tx = sender.input_sender().clone();
                act.connect_activate(move |a, _| {
                    let now = !a.state().and_then(|s| s.get::<bool>()).unwrap_or(false);
                    a.set_state(&now.to_variant());
                    tx.emit(Input::ViewOption(opt, now));
                });
                view_actions.add_action(&act);
            }
            let snippets = gio::SimpleAction::new("snippets", None);
            let tx = sender.input_sender().clone();
            snippets.connect_activate(move |_, _| tx.emit(Input::ManageSnippets));
            view_actions.add_action(&snippets);
            let special = gio::SimpleAction::new("special", None);
            let tx = sender.input_sender().clone();
            special.connect_activate(move |_, _| tx.emit(Input::Chars(special_chars::Msg::Open)));
            view_actions.add_action(&special);
            let entities = gio::SimpleAction::new_stateful(
                "entities",
                None,
                &prefs.convert_entities.to_variant(),
            );
            let tx = sender.input_sender().clone();
            entities.connect_activate(move |a, _| {
                let now = !a.state().and_then(|s| s.get::<bool>()).unwrap_or(false);
                a.set_state(&now.to_variant());
                tx.emit(Input::Chars(special_chars::Msg::ConvertEntities(now)));
            });
            view_actions.add_action(&entities);
            markup_hints::view_actions(&view_actions, sender.input_sender(), &hints);
            win.insert_action_group("view", Some(&view_actions));
            let fl = gio::SimpleActionGroup::new();
            for (name, make) in [
                ("new", (|| Input::PromptNewFile) as fn() -> Input),
                ("add", || Input::AddFiles),
                ("merge", || Input::PromptMerge),
                ("bulk", || Input::PromptBulkRename),
                ("link", || Input::PromptLinkStyles),
                ("export", || Input::PromptExport),
                ("rename", || Input::RenameFile),
                ("delete", || Input::DeleteFile),
            ] {
                let act = gio::SimpleAction::new(name, None);
                let tx = sender.input_sender().clone();
                act.connect_activate(move |_, _| tx.emit(make()));
                fl.add_action(&act);
            }
            win.insert_action_group("files", Some(&fl));
        }
        win.set_content(Some(&toasts));
        {
            let tx = sender.input_sender().clone();
            let sm = adw::StyleManager::default();
            let id = Cell::new(Some(
                sm.connect_dark_notify(move |_| tx.emit(Input::Restyle)),
            ));
            win.connect_destroy(move |_| {
                if let Some(i) = id.take() {
                    sm.disconnect(i);
                }
            });
        }

        // shortcuts: every editor action is in the shortcut registry (group Editor)
        keymap::install(&win, sender.input_sender().clone());
        snippets_ui::install_entry(&find_entry, comp.clone());
        snippets_ui::install_entry(&replace_entry, comp.clone());

        let pv = preview::PvState::new(preview::PvInit {
            sender: &sender,
            preview: &preview,
            preview_head: &preview_head,
            win: &win,
            cp_menu: &cp_menu,
            edit_menu: &markup_built.structure,
            sidebar_stack: &sidebar_stack,
            center: &center,
            toc_add: &|w| toc_bar.append(w),
            both: prefs.scroll_both,
            code_adj: gtk::prelude::ScrollableExt::vadjustment(&view),
        });

        let force_close = Rc::new(Cell::new(false));
        let w = W {
            win: win.clone(),
            title,
            view,
            map,
            marks_renderer,
            center,
            picture,
            image_info,
            tabs,
            sidebar_stack,
            files,
            toc_list,
            toc_store,
            toc_sel,
            meta_box,
            results,
            results_rev,
            results_title,
            problems: problems_w,
            find_rev,
            replace_rev,
            find_entry,
            replace_entry,
            sx,
            case_btn,
            dotall_btn,
            scope,
            find_status,
            status,
            preview,
            preview_pane,
            preview_btn,
            undo_btn,
            redo_btn,
            save_btn,
            toasts,
            banner,
            split: split.clone(),
            sidebar_btn: sidebar_btn.clone(),
            paned: paned.clone(),
            restore_menu,
            compare_menu,
            menus: vec![
                ("Tools".into(), tools_menu.clone().upcast()),
                ("Markup".into(), edit_menu.clone().upcast()),
                ("Files".into(), files_menu.clone().upcast()),
            ],
        };
        {
            let tx = sender.input_sender().clone();
            let fc = force_close.clone();
            win.connect_close_request(move |_| {
                if fc.get() {
                    glib::Propagation::Proceed
                } else {
                    tx.emit(Input::CloseRequested);
                    glib::Propagation::Stop
                }
            });
            let out = sender.clone();
            win.connect_destroy(move |_| {
                let _ = out.output(Output::Closed);
            });
        }

        {
            let tx = sender.input_sender().clone();
            let sub = crate::settings_bus::editor()
                .subscribe(move || tx.emit(Input::Prefs(prefs_page::PrefsAct::Changed)));
            let sub = RefCell::new(Some(sub));
            win.connect_destroy(move |_| {
                sub.borrow_mut().take();
            });
        }
        let mut model = Editor {
            shared,
            book_id,
            book,
            sender: sender.clone(),
            w,
            buffers: HashMap::new(),
            prefs,
            gutter_marks: marks,
            marks: HashMap::new(),
            baseline: None,
            original: HashMap::new(),
            issue_marks: HashMap::new(),
            spell,
            spelling: spelling_panel,
            reports: reports::State::default(),
            chars: chars_state,
            live_timer: Rc::new(RefCell::new(None)),
            search_settings,
            contexts: HashMap::new(),
            comp,
            modified: HashSet::new(),
            current: None,
            toc: toc_flat,
            toc_dirty: false,
            structure_dirty: false,
            results: Vec::new(),
            result_kind: ResultKind::Check,
            problems: problems_state,
            file_rows: HashMap::new(),
            preview_timer: Rc::new(RefCell::new(None)),
            hl_timer: Rc::new(RefCell::new(None)),
            force_close,
            toc_selected: None,
            checkpoints: Checkpoints::new(25),
            last_toast: RefCell::new(String::new()),
            last_press: RefCell::new(None),
            toc_undo: RefCell::new(None),
            tabs_guard,
            pv,
            ext: ext_state,
            hints: hints.clone(),
        };
        model.rebuild_files();
        model.rebuild_toc();
        model.rebuild_meta();
        model.update_title();
        {
            // the Markup and Tools menus show the keys of the registry (the preview and the tool sections are
            // added to them above, so the labels go on once everything is in)
            let menus = [edit_menu.clone(), tools_menu.clone()];
            glib::idle_add_local_once(move || markup_menu::keep_accels(&menus));
        }
        model.ext_install();

        // open the first chapter
        let first = model.book.borrow().spine_paths().into_iter().next();
        if let Some(f) = first {
            let tx = sender.input_sender().clone();
            glib::idle_add_local_once(move || tx.emit(Input::SelectFile(f)));
        }
        ComponentParts { model, widgets: () }
    }

    fn update(&mut self, msg: Input, sender: ComponentSender<Self>) {
        match msg {
            Input::SelectFile(p) => {
                if self.current.as_deref() != Some(&p)
                    || self.w.center.visible_child_name().as_deref() == Some("empty")
                {
                    self.select_file(&p);
                }
            }
            Input::Problems(a) => self.problems_act(a),
            Input::BufferChanged(p) => {
                self.problems_mark_stale(&p);
                if self.modified.insert(p.clone()) {
                    if let Some((_, dot)) = self.file_rows.get(&p) {
                        dot.set_visible(true);
                    }
                }
                self.update_title();
                if self.current.as_deref() == Some(&p) {
                    self.schedule_highlight();
                    self.schedule_preview();
                    self.schedule_live();
                    self.update_cursor_status();
                }
            }
            Input::Restyle => {
                let (dark, eink) = (system_is_dark(), crate::eink::is_active());
                for b in self.buffers.values() {
                    codeview::apply_scheme(b, dark, eink);
                    codeview::recolor(&b.tag_table(), dark);
                }
            }
            Input::Rehighlight => {
                if let Some(p) = &self.current {
                    if let Some(b) = self.buffers.get(p) {
                        codeview::fallback_highlight(b, p);
                    }
                }
            }
            Input::RefreshPreview => self.refresh_preview_now(),
            Input::Cursor => {
                self.update_cursor_status();
                self.update_pair_highlight();
                self.pv_cursor_moved();
            }
            Input::Pv(m) => self.pv_msg(m),
            Input::Prefs(a) => self.prefs_update(a),
            Input::Ext(c) => self.ext_update(c),
            Input::Live => self.run_live(),
            Input::Spell(a) => self.spell_act(a),
            Input::ViewOption(o, on) => self.set_view_option(o, on),
            Input::FindChanged => self.find_changed(),
            Input::FindCount => self.refresh_find_status(),
            Input::ManageSnippets => self.manage_snippets(),
            Input::SnippetsChanged(user) => {
                self.comp.borrow_mut().snippets = crate::editor_snippets::merged(&user);
            }
            Input::Save => {
                self.save();
            }
            Input::SaveAndClose => {
                if self.save() {
                    sender.input(Input::ForceClose);
                }
            }
            Input::ForceClose => {
                self.save_state();
                self.force_close.set(true);
                self.w.win.close();
            }
            Input::CloseRequested => {
                if self.is_dirty() {
                    let d = adw::AlertDialog::new(
                        Some(&tr("Save Changes?")),
                        Some(&tr(
                            "Your changes to this book will be lost if you don't save them.",
                        )),
                    );
                    d.add_responses(&[
                        ("cancel", &tr("Cancel")),
                        ("discard", &tr("Discard")),
                        ("save", &tr("Save")),
                    ]);
                    d.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
                    d.set_response_appearance("save", adw::ResponseAppearance::Suggested);
                    d.set_default_response(Some("save"));
                    d.set_close_response("cancel");
                    let tx = sender.input_sender().clone();
                    d.connect_response(None, move |_, r| match r {
                        "save" => tx.emit(Input::SaveAndClose),
                        "discard" => tx.emit(Input::ForceClose),
                        _ => {}
                    });
                    d.present(Some(&self.w.win));
                } else {
                    sender.input(Input::ForceClose);
                }
            }
            Input::Undo => {
                let b = self.w.view.buffer();
                if b.can_undo() {
                    b.undo();
                }
            }
            Input::Redo => {
                let b = self.w.view.buffer();
                if b.can_redo() {
                    b.redo();
                }
            }
            Input::ToggleFind => {
                if self.find_in_preview_if_focused() {
                    return;
                }
                let show = !self.w.find_rev.reveals_child();
                self.w.find_rev.set_reveal_child(show);
                if !show {
                    self.w.replace_rev.set_reveal_child(false);
                }
                if show {
                    // prefill with the selection
                    let b = self.w.view.buffer();
                    if let Some((s, e)) = b.selection_bounds() {
                        let t = b.text(&s, &e, false);
                        if !t.contains('\n') {
                            self.w.find_entry.set_text(&t);
                        }
                    }
                    self.w.find_entry.grab_focus();
                } else {
                    self.w.view.grab_focus();
                }
            }
            Input::ToggleReplace => {
                self.w.find_rev.set_reveal_child(true);
                self.w.replace_rev.set_reveal_child(true);
                if replace_focus_on_find(&self.w.find_entry.text()) {
                    self.w.find_entry.grab_focus();
                } else {
                    self.w.replace_entry.grab_focus();
                }
            }
            Input::FindNext(f) => self.find_next(f),
            Input::Replace(fwd) => self.replace_one(fwd),
            Input::GoToLine(t) => self.go_to_line(&t),
            Input::PromptLine => self.prompt_line(),
            Input::MarkSelection => self.mark_selection(),
            Input::ReplaceAll => self.replace_all(),
            Input::FindAll => self.find_all(),
            Input::Search(a) => self.search_act(a),
            Input::Check => self.run_check(),
            Input::CloseResults => self.w.results_rev.set_reveal_child(false),
            Input::RunTool(t) => self.run_tool(t),
            Input::RunPluginTool(id) => self.run_plugin_tool(&id),
            Input::ShowReport => self.reports(reports::Msg::Open(None)),
            Input::CheckSpelling => self.spell_panel(spelling::Msg::Toggle),
            Input::SpellPanel(m) => self.spell_panel(m),
            Input::SpellNext(f) => self.spell_panel(spelling::Msg::NextMistake(f)),
            Input::Reports(m) => self.reports(m),
            Input::Chars(m) => self.chars(m),
            Input::SplitHeadings(level) => {
                self.run_op("Split File at Headings", move |b, cur| match cur {
                    Some(p) if b.package.item_by_path(p).is_some() => {
                        editor_tools::split_at_headings(b, p, level)
                    }
                    _ => anyhow::bail!("Select a chapter first"),
                })
            }
            Input::MergeNext => self.run_op("Merge with Next File", |b, cur| match cur {
                Some(p) => editor_tools::merge_with_next(b, p),
                None => anyhow::bail!("Select a chapter first"),
            }),
            Input::PreviewColors(p) => {
                {
                    let mut cfg = self.shared.cfg.borrow_mut();
                    cfg.reader.theme = p.theme;
                    cfg.reader.colors = p.colors;
                    cfg.reader.custom_fg = p.custom_fg;
                    cfg.reader.custom_bg = p.custom_bg;
                    cfg.save();
                }
                let mut st: CanvasStyle = self
                    .shared
                    .cfg
                    .borrow()
                    .reader
                    .canvas_style(system_is_dark());
                st.layout = LayoutMode::Scroll;
                st.margin = 24.0;
                self.w
                    .preview
                    .set_style(prefs_page::preview_style(st, &crate::editor_prefs::look()));
                self.refresh_preview_now();
            }
            Input::CreateCheckpoint => {
                let n = self.checkpoints.len() + 1;
                self.create_checkpoint(&format!("Checkpoint {n}"));
                self.toast(&tr("Checkpoint created"));
            }
            Input::RestoreCheckpoint(id) => self.restore_checkpoint(id),
            Input::UndoSnapshot(id) => self.undo_snapshot(id),
            Input::CompareCheckpoint(id) => self.compare_checkpoint(id),
            Input::Reformat => {
                if let Some(p) = self.current.clone() {
                    if !matches!(Lang::for_path(&p), Lang::Xml) {
                        self.toast(&tr("Reformatting works on XHTML, OPF, NCX and XML files"));
                        return;
                    }
                    let buf = self.buffer_for(&p);
                    let text = buffer_text(&buf);
                    match pretty_print_xml(&text) {
                        Ok(new) if new != text => {
                            buf.begin_user_action();
                            let (mut s, mut e) = buf.bounds();
                            buf.delete(&mut s, &mut e);
                            let mut s = buf.start_iter();
                            buf.insert(&mut s, &new);
                            buf.end_user_action();
                            self.toast(&tr("Reformatted"));
                        }
                        Ok(_) => self.toast(&tr("Already nicely formatted")),
                        Err(e) => self.toast(&trf("Cannot reformat: {}", &[&e])),
                    }
                }
            }
            Input::ToggleSidebar => {
                let on = !self.w.split.shows_sidebar();
                self.w.split.set_show_sidebar(on);
            }
            Input::TogglePreview => {
                self.w
                    .preview_pane
                    .set_visible(self.w.preview_btn.is_active());
                self.refresh_preview_now();
            }
            Input::ResultActivated(i) => {
                if let Some(r) = self.results.get(i) {
                    let (file, line, range) = (r.file.clone(), r.line, r.range);
                    if !self.book.borrow().archive.contains(&file) {
                        return;
                    }
                    match (&self.result_kind, range) {
                        (ResultKind::Search, Some((a, b))) => self.select_range(&file, a, b),
                        _ => {
                            self.select_file(&file);
                            if let Some(l) = line {
                                let buf = self.w.view.buffer();
                                if let Some(mut it) = buf.iter_at_line(l as i32 - 1) {
                                    buf.place_cursor(&it);
                                    self.w.view.scroll_to_iter(&mut it, 0.1, true, 0.5, 0.3);
                                }
                                self.w.view.grab_focus();
                            }
                        }
                    }
                }
            }
            Input::AddFiles => {
                let dlg = gtk::FileDialog::builder()
                    .title(tr("Add Files to the Book"))
                    .build();
                let tx = sender.input_sender().clone();
                dlg.open_multiple(Some(&self.w.win), gio::Cancellable::NONE, move |res| {
                    if let Ok(list) = res {
                        let files: Vec<std::path::PathBuf> = (0..list.n_items())
                            .filter_map(|i| list.item(i).and_downcast::<gio::File>())
                            .filter_map(|f| f.path())
                            .collect();
                        tx.emit(Input::AddPaths(files));
                    }
                });
            }
            Input::AddPaths(p) => self.add_paths(p),
            Input::DeleteFile => {
                let paths = self.selected_paths();
                self.ask_delete_files(paths);
            }
            Input::AskDeleteFiles(paths) => self.ask_delete_files(paths),
            Input::DeleteFiles(p) => self.delete_files(p),
            Input::MergeFiles(p) => self.merge_files(p),
            Input::BulkRename(prefix, start) => self.bulk_rename(&prefix, start),
            Input::NewFile(n, t, l) => self.new_file(&n, &t, l),
            Input::LinkStyles(c, r) => self.link_styles(c, r),
            Input::ExportTo(p) => self.export_files(&p),
            Input::PromptMerge => self.prompt_merge(),
            Input::PromptBulkRename => self.prompt_bulk_rename(),
            Input::PromptNewFile => self.prompt_new_file(),
            Input::PromptLinkStyles => self.prompt_link_styles(),
            Input::PromptExport => self.prompt_export(),
            Input::PromptTocGenerate => self.prompt_toc_generate(),
            Input::GenerateToc(o) => self.generate_toc(&o),
            Input::Tag(a, arg) => self.tag_action(a, arg),
            Input::Code(m) => self.code_msg(m),
            Input::QuickOpen => self.quick_open(),
            Input::Palette => self.show_palette(),
            Input::StepFile(n) => self.step_file(n),
            Input::CloseTab => self.close_current_tab(),
            Input::TabsEmpty => {
                self.current = None;
                self.w.center.set_visible_child_name("empty");
                self.update_title();
            }
            Input::RenameFile => {
                if let Some(p) = self.selected_path() {
                    let d = adw::AlertDialog::new(
                        Some(&tr("Rename File")),
                        Some(
                            "References to the file in other chapters and stylesheets are updated.",
                        ),
                    );
                    let entry = gtk::Entry::new();
                    entry.set_text(file_name(&p));
                    entry.set_activates_default(true);
                    d.set_extra_child(Some(&entry));
                    d.add_responses(&[("cancel", &tr("Cancel")), ("rename", &tr("Rename"))]);
                    d.set_response_appearance("rename", adw::ResponseAppearance::Suggested);
                    d.set_default_response(Some("rename"));
                    let tx = sender.input_sender().clone();
                    let old = p;
                    d.connect_response(None, move |_, r| {
                        if r == "rename" {
                            tx.emit(Input::Dev(format!("rename={old}|{}", entry.text())));
                        }
                    });
                    d.present(Some(&self.w.win));
                }
            }
            Input::MoveFile(up) => self.move_file(up),
            Input::SetCover => {
                if let Some(p) = self.current.clone().filter(|p| is_image_path(p)) {
                    self.flush();
                    let data = self.book.borrow().archive.get(&p).map(|d| d.to_vec());
                    if let Some(data) = data {
                        let res = self.book.borrow_mut().set_cover(data, &extension(&p));
                        match res {
                            Ok(()) => {
                                self.structure_dirty = true;
                                self.reload_package_buffers();
                                self.rebuild_files();
                                self.rebuild_meta();
                                self.update_title();
                                self.toast(&tr("Cover updated"));
                            }
                            Err(e) => self.toast(&trf("Could not set the cover: {}", &[&e])),
                        }
                    }
                }
            }
            Input::TocSelect(i) => self.toc_select(i, false),
            Input::TocGo(i) => self.toc_select(i, true),
            Input::TocTitle(i, t) => {
                if let Some(e) = self.toc.get_mut(i) {
                    if e.title != t {
                        e.title = t;
                        self.toc_dirty = true;
                        self.update_title();
                    }
                }
            }
            Input::Toc(op) => self.toc_op(op),
            Input::TocUndo => self.toc_undo_delete(),
            Input::EditMetadata => {
                self.flush();
                let (md, cover) = {
                    let b = self.book.borrow();
                    (b.metadata().clone(), b.cover_bytes().map(|c| c.to_vec()))
                };
                let tx = sender.input_sender().clone();
                metadata::edit_dialog(&self.w.win, &md, cover, move |m, c| {
                    tx.emit(Input::ApplyMetadata(m, c))
                });
            }
            Input::ApplyMetadata(md, change) => {
                self.create_checkpoint("Before: Metadata Change");
                let before_md = self.book.borrow().metadata().clone();
                let after_md = md.clone();
                let res = (|| -> anyhow::Result<()> {
                    let mut b = self.book.borrow_mut();
                    match change {
                        CoverChange::Keep => {}
                        CoverChange::Set(d, ext) => b.set_cover(d, &ext)?,
                        CoverChange::Remove => b.remove_cover()?,
                    }
                    b.set_metadata(md)
                })();
                match res {
                    Ok(()) => {
                        crate::library_health::record_edit_as(
                            &self.shared.guard.borrow().store,
                            self.book_id,
                            &before_md,
                            &after_md,
                            &crate::ui::metadata::take_fetch_sources(),
                            crate::library_health::EDITOR,
                            chrono::Utc::now().timestamp(),
                        );
                        self.structure_dirty = true;
                        self.struct_after_metadata();
                        self.reload_package_buffers();
                        self.rebuild_files();
                        self.rebuild_meta();
                        self.update_title();
                        self.toast(&tr("Metadata updated — save to keep your changes"));
                    }
                    Err(e) => self.toast(&trf("Could not update metadata: {}", &[&e])),
                }
            }
            Input::Dev(c) => self.dev(c, &sender),
        }
    }
}

#[cfg(test)]
mod replace_focus_tests {
    use super::replace_focus_on_find;

    #[test]
    fn ctrl_h_starts_in_find_until_there_is_something_to_find() {
        assert!(replace_focus_on_find(""));
        assert!(!replace_focus_on_find("zz"));
    }
}
