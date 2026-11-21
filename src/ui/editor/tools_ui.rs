//! The dialogs of the Tools menu that need options: every tool shows what it will change before it runs, runs on a
//! checkpoint (so the result is one toast and one undo), and has a dev hook (`editor:x:...`, see `ext_dev.rs`).
//!
//! The shape is the same for all of them: [`Editor::tool_dialog`] builds an `AdwDialog` with an options area, a
//! *Preview* button (runs the tool on a copy of the book and lists what would change) and an *Apply* button (runs
//! it for real through `run_op`). Each tool only supplies its options and a function that turns them into an
//! operation.

use crate::i18n::ntr;
use crate::i18n::{tr, trf, N_};
use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use bookx_convert::transforms::punctuation::SmartenOptions;
use bookx_editing::book_tools::{self, AccessMeta, Punctuation, QuoteStyle};
use bookx_editing::filter_styles::FilterSpec;
use bookx_editing::fonts;
use bookx_editing::links::{self, LinkStatus};
use bookx_editing::semantics::{self, Semantic};

use super::*;
use crate::editor_tools::AccessEdits;

pub type Op = Rc<dyn Fn(&mut OpenBook, Option<&str>) -> anyhow::Result<editor_tools::Outcome>>;

/// An operation on its way to `run_op` (the message needs `Debug`).
pub struct OpBox(pub Op);

impl std::fmt::Debug for OpBox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Op")
    }
}

#[derive(Debug)]
pub enum Cmd {
    /// Opens the dialog of a tool (`smarten`, `fix-html`, ...).
    Open(String),
    /// Runs an operation as one undoable step, named in the first field.
    Run(String, OpBox),
    /// A file browser action on the selection (`files_extra.rs`).
    Files(String),
    /// A row was dropped on another row: (dragged path, target path, after the target).
    DropRow(String, String, bool),
    /// A window action (`window_features.rs`).
    Win(String),
    /// A new file from the user's template: (name, template number, link stylesheets).
    NewFromUser(String, usize, bool),
    /// The result of a link check: (url, status).
    LinkResults(Vec<(String, LinkStatus)>),
    /// A tool of `ext_dev.rs`.
    Dev(String),
    /// The structure tools (`struct_hub.rs`).
    Struct(super::struct_hub::StructCmd),
}

/// The pieces of an open tool dialog.
pub struct ToolUi {
    pub dialog: adw::Dialog,
    pub preview: gtk::Label,
    pub apply: gtk::Button,
}

pub struct Spec {
    pub title: &'static str,
    pub intro: String,
    pub apply: &'static str,
    /// Show what would change as soon as the dialog opens.
    pub auto_preview: bool,
    pub height: i32,
}

impl Spec {
    pub fn new(title: &'static str, intro: impl Into<String>, apply: &'static str) -> Spec {
        Spec {
            title,
            intro: intro.into(),
            apply,
            auto_preview: false,
            height: 460,
        }
    }
}

pub type MakeOp = Rc<dyn Fn() -> Result<(String, Op), String>>;

pub fn switch_row(title: &str, subtitle: &str, on: bool) -> adw::SwitchRow {
    let r = adw::SwitchRow::new();
    r.set_title(title);
    r.set_subtitle(subtitle);
    r.set_active(on);
    r
}

pub fn combo_row(title: &str, items: &[&str], selected: u32) -> adw::ComboRow {
    let r = adw::ComboRow::new();
    r.set_title(title);
    r.set_model(Some(&gtk::StringList::new(items)));
    r.set_selected(selected);
    r
}

pub fn entry_row(title: &str, text: &str) -> adw::EntryRow {
    let r = adw::EntryRow::new();
    r.set_title(title);
    r.set_text(text);
    r
}

pub fn group(title: &str) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    if !title.is_empty() {
        g.set_title(title);
    }
    g
}

/// Puts `content` into `dialog` inside a toast overlay, so [`toast_in_dialog`] has somewhere to show (a toast of
/// the window would sit behind the dialog).
pub fn set_dialog_content(dialog: &adw::Dialog, content: &impl IsA<gtk::Widget>) {
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(content));
    dialog.set_child(Some(&toasts));
}

/// A toast on top of `dialog` (a plain text, markup off). Does nothing for a dialog that was not filled with
/// [`set_dialog_content`].
pub fn toast_in_dialog(dialog: &adw::Dialog, text: &str) {
    if let Some(t) = dialog.child().and_downcast::<adw::ToastOverlay>() {
        let toast = adw::Toast::new(text);
        toast.set_use_markup(false);
        toast.set_priority(adw::ToastPriority::High);
        t.add_toast(toast);
    }
}

/// The placeholder row of an empty rule list: what to do, and a button that moves to the form below the list.
pub fn empty_rules_row(list: &gtk::ListBox) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&tr("No Rules Yet"));
    row.set_subtitle(&tr("Add a rule, or load a saved set"));
    row.set_activatable(false);
    let go = gtk::Button::with_label(&tr("Add a Rule"));
    go.set_valign(gtk::Align::Center);
    go.set_widget_name("rules-add-first");
    let list = list.clone();
    go.connect_clicked(move |_| {
        if let Some(form) = list.parent().and_then(|p| p.next_sibling()) {
            form.child_focus(gtk::DirectionType::TabForward);
        }
    });
    row.add_suffix(&go);
    row
}

/// A scrolling list of rows with a height limit.
pub fn scrolled(child: &impl IsA<gtk::Widget>, height: i32) -> gtk::ScrolledWindow {
    let s = gtk::ScrolledWindow::new();
    s.set_min_content_height(height);
    s.set_max_content_height(height);
    s.set_child(Some(child));
    s.add_css_class("card");
    s
}

/// A dialog size that fits the window it opens over (small windows get a smaller dialog that scrolls).
pub fn fit_dialog(win: &adw::Window, width: i32, height: i32) -> (i32, i32) {
    let (ww, wh) = (win.width(), win.height());
    let w = if ww > 0 { width.min(ww - 40) } else { width };
    let h = if wh > 0 { height.min(wh - 80) } else { height };
    (w.max(300), h.max(280))
}

pub fn menu_item(label: &str, action: &str, target: &str) -> gio::MenuItem {
    let it = gio::MenuItem::new(Some(label), None);
    it.set_action_and_target_value(Some(action), Some(&target.to_variant()));
    it
}

/// One entry of a Tools submenu.
#[derive(Clone, Copy)]
pub enum Item {
    /// A tool with a dialog (`ext.tool`, id).
    Dialog(&'static str, &'static str),
    /// A one-click tool of `editor_tools`.
    One(Tool),
    /// A plain action of the `tools` group.
    Act(&'static str, &'static str),
}

/// The submenus of the Tools menu, in order: (title, sections of entries). Every capability appears once, under
/// one name (EDT-E4); the top level of the menu is these submenus plus the entries around them.
pub const TOOL_SUBMENUS: &[(&str, &[&[Item]])] = &[
    (
        N_("Fix & Clean"),
        &[
            &[
                Item::Act(N_("Reformat Markup"), "tools.reformat"),
                Item::Dialog(N_("Reformat Markup in Files"), "beautify"),
            ],
            &[
                Item::Dialog(N_("Smarten Punctuation"), "smarten"),
                Item::Dialog(N_("Fix HTML"), "fix-html"),
                Item::One(Tool::UpgradeToEpub3),
            ],
        ],
    ),
    (
        N_("Styles"),
        &[&[
            Item::Dialog(N_("Transform Styles"), "styles"),
            Item::Dialog(N_("Transform HTML"), "transform-html"),
            Item::Dialog(N_("Remove Unused CSS Rules"), "unused-css"),
            Item::Dialog(N_("Filter Style Information"), "filter"),
        ]],
    ),
    (
        N_("Media and Fonts"),
        &[
            &[
                Item::Dialog(N_("Delete Unused Media"), "unused-media"),
                Item::One(Tool::CompressImages),
                Item::Dialog(N_("Set Cover"), "cover"),
                Item::Dialog(N_("Download External Resources"), "download"),
            ],
            &[
                Item::Dialog(N_("Manage Fonts"), "fonts"),
                Item::Dialog(N_("Embed Referenced Fonts"), "embed-fonts"),
                Item::One(Tool::SubsetFonts),
            ],
        ],
    ),
    (
        N_("Structure"),
        &[
            &[
                Item::Act(N_("Split File at Chapter Headings"), "tools.split"),
                Item::Act(N_("Merge with Next File"), "tools.merge"),
                Item::One(Tool::ArrangeFiles),
            ],
            &[
                Item::One(Tool::AddInlineToc),
                Item::One(Tool::RemoveInlineToc),
                Item::Dialog(N_("Set Semantics"), "semantics"),
                Item::Dialog(N_("Book Jacket"), "jacket"),
            ],
        ],
    ),
    (
        N_("Accessibility"),
        &[&[
            Item::One(Tool::MakeAccessible),
            Item::Dialog(N_("Check Accessibility"), "accessibility"),
            Item::Dialog(N_("Accessibility Metadata"), "a11y-form"),
        ]],
    ),
    (
        N_("Links"),
        &[&[
            Item::Dialog(N_("Check External Links"), "check-links"),
            Item::Dialog(N_("Repair Broken Links"), "repair-links"),
            Item::Dialog(N_("Move Files to a Folder"), "move-files"),
        ]],
    ),
];

fn tool_item(it: Item) -> gio::MenuItem {
    match it {
        Item::Dialog(label, id) => menu_item(&tr(label), "ext.tool", id),
        Item::One(t) => menu_item(&tr(t.label()), "tools.run", t.id()),
        Item::Act(label, action) => {
            let m = gio::MenuItem::new(Some(&tr(label)), None);
            m.set_action_and_target_value(Some(action), None);
            m
        }
    }
}

/// The Tools menu: Check Book and Fix Problems, the submenus of [`TOOL_SUBMENUS`], Check Spelling and Reports,
/// then Checkpoints and Plugins. At most 12 top-level entries.
pub fn build_tools_menu(
    restore: &gio::Menu,
    compare: &gio::Menu,
    plugins: Option<&gio::Menu>,
) -> (gio::Menu, gio::Menu) {
    let menu = gio::Menu::new();
    let first = gio::Menu::new();
    first.append(Some(&tr("Check Book")), Some("tools.check"));
    first.append_item(&tool_item(Item::One(Tool::FixProblems)));
    menu.append_section(None, &first);
    let subs = gio::Menu::new();
    for (title, sections) in TOOL_SUBMENUS {
        let sub = gio::Menu::new();
        for items in *sections {
            let sec = gio::Menu::new();
            for it in *items {
                sec.append_item(&tool_item(*it));
            }
            sub.append_section(None, &sec);
        }
        if *title == N_("Media and Fonts") {
            sub.append_section(None, &narration::menu_section());
        }
        subs.append_submenu(Some(&tr(title)), &sub);
    }
    menu.append_section(None, &subs);
    let info = gio::Menu::new();
    info.append(Some(&tr("Check Spelling")), Some("tools.spelling"));
    info.append(Some(&tr("Reports")), Some("tools.report"));
    menu.append_section(None, &info);
    let last = gio::Menu::new();
    let cp = gio::Menu::new();
    cp.append(Some(&tr("Create Checkpoint")), Some("tools.checkpoint"));
    cp.append_submenu(Some(&tr("Restore Checkpoint")), restore);
    cp.append_submenu(Some(&tr("Compare with Checkpoint")), compare);
    last.append_submenu(Some(&tr("Checkpoints")), &cp);
    if let Some(p) = plugins {
        last.append_submenu(Some(&tr("Plugins")), p);
    }
    menu.append_section(None, &last);
    (menu, cp)
}

pub fn actions(tx: &relm4::Sender<Input>) -> gio::SimpleActionGroup {
    let g = gio::SimpleActionGroup::new();
    let tool = gio::SimpleAction::new("tool", Some(glib::VariantTy::STRING));
    let t = tx.clone();
    tool.connect_activate(move |_, p| {
        if let Some(id) = p.and_then(|v| v.str()) {
            t.emit(Input::Ext(Cmd::Open(id.to_string())));
        }
    });
    g.add_action(&tool);
    g
}

fn markup_current(e: &Editor) -> Option<String> {
    e.current
        .clone()
        .filter(|p| bookx_editing::markup::is_markup_path(p))
}

impl Editor {
    pub(super) fn ext_update(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Open(id) => self.open_tool(&id),
            Cmd::Run(label, op) => {
                let op = op.0;
                self.run_op(&label, move |b, c| op(b, c));
            }
            Cmd::Files(id) => self.files_action(&id),
            Cmd::DropRow(from, onto, after) => self.drop_row(&from, &onto, after),
            Cmd::Win(id) => self.window_action(&id),
            Cmd::NewFromUser(name, idx, link) => self.new_file_from_user(&name, idx, link),
            Cmd::LinkResults(r) => self.show_link_results(r),
            Cmd::Dev(c) => self.ext_dev(&c),
            Cmd::Struct(c) => self.struct_update(c),
        }
    }

    pub(super) fn open_tool(&mut self, id: &str) {
        self.flush();
        self.commit_toc();
        match id {
            "smarten" => self.tool_smarten(),
            "fix-html" => self.tool_fix_html(),
            "beautify" => self.tool_beautify(),
            "styles" => self.tool_transform_styles(),
            "unused-css" => self.tool_unused_css(),
            "filter" => self.tool_filter(),
            "unused-media" => self.tool_unused_media(),
            "embed-fonts" => self.tool_embed_fonts(),
            "download" => self.tool_download(),
            "semantics" => self.tool_semantics(),
            "accessibility" => self.tool_accessibility(),
            "check-links" => self.tool_check_links(),
            "repair-links" => self.tool_repair_links(),
            "move-files" => self.tool_move_files(),
            other => {
                if !self.struct_open_tool(other) {
                    self.toast(&trf("Unknown tool {}", &[&other]));
                }
            }
        }
    }

    /// The shared dialog. `options` is the tool's own widget (may be `None`); `make` turns the options into
    /// an operation (or says what is wrong with them). With `custom_apply`, Apply runs that instead.
    pub(super) fn tool_dialog(
        &self,
        spec: Spec,
        options: Option<gtk::Widget>,
        make: MakeOp,
        custom_apply: Option<Rc<dyn Fn(&ToolUi)>>,
    ) -> ToolUi {
        let dialog = adw::Dialog::new();
        dialog.set_title(&tr(spec.title));
        let (w, h) = fit_dialog(&self.w.win, 560, spec.height);
        dialog.set_content_width(w);
        dialog.set_content_height(h);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 14);
        body.set_margin_top(12);
        body.set_margin_bottom(12);
        body.set_margin_start(16);
        body.set_margin_end(16);
        let (first, rest) = editor_tools::split_intro(&tr(&spec.intro));
        let intro = gtk::Label::new(Some(&first));
        intro.set_wrap(true);
        intro.set_xalign(0.0);
        intro.add_css_class("dim-label");
        body.append(&intro);
        if let Some(o) = &options {
            match (rest.is_empty(), o.downcast_ref::<adw::PreferencesGroup>()) {
                (true, _) => {}
                (false, Some(g)) => g.set_description(Some(&glib::markup_escape_text(&rest))),
                (false, None) => {
                    let more = gtk::Label::new(Some(&rest));
                    more.set_wrap(true);
                    more.set_xalign(0.0);
                    more.add_css_class("dim-label");
                    more.add_css_class("caption");
                    body.append(&more);
                }
            }
            body.append(o);
        } else if !rest.is_empty() {
            intro.set_label(&format!("{first} {rest}"));
        }
        let pg = group(&tr("What will change"));
        let preview = gtk::Label::new(Some(&tr("Press Preview to see what this would do.")));
        preview.set_wrap(true);
        preview.set_xalign(0.0);
        preview.set_selectable(true);
        preview.set_margin_all(12);
        preview.add_css_class("monospace");
        crate::ui::audit::label(&preview, &tr("What the tool will change"));
        let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        card.add_css_class("card");
        card.append(&preview);
        pg.add(&card);
        // the card stays folded until Preview (or the tool's own button) is pressed
        let pg_rev = gtk::Revealer::new();
        pg_rev.set_widget_name("tool-change-card");
        pg_rev.set_child(Some(&pg));
        pg_rev.set_reveal_child(spec.auto_preview);
        body.append(&pg_rev);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_child(Some(&body));

        let preview_btn = gtk::Button::with_label(&tr("Preview"));
        let apply = gtk::Button::with_label(&tr(spec.apply));
        apply.add_css_class("suggested-action");
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        bar.set_margin_all(12);
        bar.set_halign(gtk::Align::End);
        if custom_apply.is_none() {
            bar.append(&preview_btn);
        }
        bar.append(&apply);
        let view = adw::ToolbarView::new();
        view.add_top_bar(&adw::HeaderBar::new());
        view.set_content(Some(&scroll));
        view.add_bottom_bar(&bar);
        dialog.set_child(Some(&view));

        let ui = ToolUi {
            dialog: dialog.clone(),
            preview: preview.clone(),
            apply: apply.clone(),
        };
        let book = self.book.clone();
        let current = self.current.clone();
        let do_preview: Rc<dyn Fn()> = {
            let make = make.clone();
            let preview = preview.clone();
            Rc::new(move || match make() {
                Err(m) => preview.set_text(&m),
                Ok((_, op)) => {
                    let before = book.borrow().archive.clone();
                    let text = match OpenBook::from_archive(before.clone()) {
                        Ok(mut copy) => {
                            let r = op(&mut copy, current.as_deref());
                            editor_tools::preview_report(&before, &copy.archive, &r)
                        }
                        Err(e) => format!("{e:#}"),
                    };
                    preview.set_text(&text);
                }
            })
        };
        {
            let p = do_preview.clone();
            let rev = pg_rev.clone();
            preview_btn.connect_clicked(move |_| {
                rev.set_reveal_child(true);
                p()
            });
        }
        {
            let rev = pg_rev.clone();
            apply.connect_clicked(move |_| rev.set_reveal_child(true));
        }
        match custom_apply {
            Some(custom) => {
                let ui2 = ToolUi {
                    dialog: dialog.clone(),
                    preview: preview.clone(),
                    apply: apply.clone(),
                };
                apply.connect_clicked(move |_| custom(&ui2));
            }
            None => {
                let tx = self.sender.input_sender().clone();
                let d = dialog.clone();
                let preview = preview.clone();
                apply.connect_clicked(move |_| match make() {
                    Err(m) => preview.set_text(&m),
                    Ok((label, op)) => {
                        tx.emit(Input::Ext(Cmd::Run(label, OpBox(op))));
                        d.close();
                    }
                });
            }
        }
        *self.ext.last_dialog.borrow_mut() =
            Some((preview_btn.clone(), apply.clone(), preview.clone()));
        dialog.present(Some(&self.w.win));
        if spec.auto_preview {
            do_preview();
        }
        ui
    }

    // ---- text ----------------------------------------------------------

    fn scope_row(&self) -> adw::ComboRow {
        combo_row(&tr("Pages"), &["All pages", "The open page only"], 0)
    }

    fn tool_smarten(&self) {
        let quotes = switch_row(
            &tr("Curly quotation marks"),
            &tr("\" and ' become “ ” ‘ ’"),
            true,
        );
        let dashes = switch_row(&tr("Dashes"), &tr("-- becomes – and --- becomes —"), true);
        let ellipses = switch_row(&tr("Ellipses"), &tr("... becomes …"), true);
        let lang = self.book.borrow().metadata().language.clone();
        let default_style = QuoteStyle::for_language(&lang);
        let labels_owned: Vec<String> = QuoteStyle::ALL.iter().map(|s| tr(s.label())).collect();
        let labels: Vec<&str> = labels_owned.iter().map(String::as_str).collect();
        let style = combo_row(
            &tr("Language rules for double quotes"),
            &labels,
            QuoteStyle::ALL
                .iter()
                .position(|s| *s == default_style)
                .unwrap_or(0) as u32,
        );
        let mode = combo_row(
            &tr("Action"),
            &["Smarten", "Straighten (undo smartening)"],
            0,
        );
        let scope = self.scope_row();
        let g = group("");
        for w in [
            mode.upcast_ref::<gtk::Widget>(),
            scope.upcast_ref(),
            quotes.upcast_ref(),
            style.upcast_ref(),
            dashes.upcast_ref(),
            ellipses.upcast_ref(),
        ] {
            g.add(w);
        }
        let me = self.scope_closure();
        let make: MakeOp = Rc::new(move || {
            let paths = me(&scope)?;
            let m = if mode.selected() == 0 {
                Punctuation::Smarten(SmartenOptions {
                    quotes: quotes.is_active(),
                    dashes: dashes.is_active(),
                    ellipses: ellipses.is_active(),
                })
            } else {
                Punctuation::Unsmarten
            };
            let st = QuoteStyle::ALL[style.selected() as usize % QuoteStyle::ALL.len()];
            Ok((
                "Smarten Punctuation".to_string(),
                Rc::new(move |b: &mut OpenBook, _: Option<&str>| {
                    editor_tools::smarten(b, &paths, m, st)
                }) as Op,
            ))
        });
        self.tool_dialog(
            Spec::new(
                crate::i18n::N_("Smarten Punctuation"),
                crate::i18n::N_("Curly quotes, proper dashes and ellipses in the text of the pages. Code, scripts, styles and attributes are never touched."),
                crate::i18n::N_("Apply"),
            ),
            Some(g.upcast()),
            make,
            None,
        );
    }

    /// A function that turns the scope row into page paths, remembering the open page now.
    fn scope_closure(&self) -> Rc<dyn Fn(&adw::ComboRow) -> Result<Vec<String>, String>> {
        let cur = markup_current(self);
        Rc::new(move |row| {
            if row.selected() == 0 {
                Ok(Vec::new())
            } else {
                cur.clone()
                    .map(|p| vec![p])
                    .ok_or_else(|| "Open a page of the book first".to_string())
            }
        })
    }

    fn tool_fix_html(&self) {
        let scope = self.scope_row();
        let html5 = switch_row(
            &tr("Always use the HTML5 parser"),
            &tr("Rebuild broken pages the way a browser reads them. Otherwise it is only used when the quick repair fails."),
            false,
        );
        let g = group("");
        g.add(&scope);
        g.add(&html5);
        let me = self.scope_closure();
        let make: MakeOp = Rc::new(move || {
            let paths = me(&scope)?;
            let h = html5.is_active();
            Ok((
                "Fix HTML".to_string(),
                Rc::new(move |b: &mut OpenBook, _: Option<&str>| {
                    editor_tools::fix_html(b, &paths, h)
                }) as Op,
            ))
        });
        self.tool_dialog(
            Spec::new(
                crate::i18n::N_("Fix HTML"),
                crate::i18n::N_("Repairs pages that are not well-formed XHTML: unclosed and misnested tags, stray ampersands, missing namespaces. Pages that are fine are left alone."),
                crate::i18n::N_("Fix"),
            ),
            Some(g.upcast()),
            make,
            None,
        );
    }

    fn tool_beautify(&self) {
        let scope = combo_row(
            &tr("Files"),
            &[
                "All pages, stylesheets and package files",
                "The open file only",
            ],
            0,
        );
        let g = group("");
        g.add(&scope);
        let cur = self.current.clone();
        let make: MakeOp = Rc::new(move || {
            let paths: Vec<String> = if scope.selected() == 0 {
                Vec::new()
            } else {
                vec![cur.clone().ok_or("Open a file first")?]
            };
            Ok((
                "Reformat Markup".to_string(),
                Rc::new(move |b: &mut OpenBook, _: Option<&str>| editor_tools::beautify(b, &paths))
                    as Op,
            ))
        });
        self.tool_dialog(
            Spec::new(
                crate::i18n::N_("Reformat Markup in Files"),
                crate::i18n::N_("Tidies the layout of the files. Only the white space between block-level tags changes, so the pages look exactly the same."),
                crate::i18n::N_("Reformat"),
            ),
            Some(g.upcast()),
            make,
            None,
        );
    }

    // ---- styles --------------------------------------------------------

    fn tool_unused_css(&self) {
        let make: MakeOp = Rc::new(|| {
            Ok((
                "Remove Unused CSS".to_string(),
                Rc::new(|b: &mut OpenBook, _: Option<&str>| editor_tools::remove_unused_css(b))
                    as Op,
            ))
        });
        self.tool_dialog(
            Spec {
                auto_preview: true,
                height: 380,
                ..Spec::new(
                    crate::i18n::N_("Remove Unused CSS Rules"),
                    crate::i18n::N_("Deletes the rules of the stylesheets that match nothing in the book. @font-face, @media and other @-rules are kept."),
                    crate::i18n::N_("Remove"),
                )
            },
            None,
            make,
            None,
        );
    }

    fn tool_filter(&self) {
        let props = bookx_editing::filter_styles::list_properties(&self.book.borrow());
        if props.is_empty() {
            self.toast(&tr("The book has no style declarations"));
            return;
        }
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.add_css_class("boxed-list");
        let mut checks: Vec<(String, gtk::CheckButton)> = Vec::new();
        for (name, n) in &props {
            let row = adw::ActionRow::new();
            row.set_title(name);
            row.set_subtitle(&trf("used {} times", &[&n]));
            let c = gtk::CheckButton::new();
            crate::ui::audit::label(&c, &trf("Remove {}", &[&name]));
            row.add_prefix(&c);
            row.set_activatable_widget(Some(&c));
            list.append(&row);
            checks.push((name.clone(), c));
        }
        let empty_rules = switch_row(&tr("Remove rules left empty"), "", true);
        let empty_attrs = switch_row(&tr("Remove empty style attributes"), "", true);
        let g = group("");
        g.add(&empty_rules);
        g.add(&empty_attrs);
        let bx = gtk::Box::new(gtk::Orientation::Vertical, 10);
        bx.append(&scrolled(&list, 220));
        bx.append(&g);
        let make: MakeOp = Rc::new(move || {
            let spec = FilterSpec {
                properties: checks
                    .iter()
                    .filter(|(_, c)| c.is_active())
                    .map(|(n, _)| n.clone())
                    .collect(),
                remove_empty_rules: empty_rules.is_active(),
                remove_empty_style_attrs: empty_attrs.is_active(),
            };
            if spec.properties.is_empty() {
                return Err("Tick the properties to remove".to_string());
            }
            Ok((
                "Filter Style Information".to_string(),
                Rc::new(move |b: &mut OpenBook, _: Option<&str>| {
                    editor_tools::filter_styles(b, &spec)
                }) as Op,
            ))
        });
        self.tool_dialog(
            Spec {
                height: 640,
                ..Spec::new(
                    crate::i18n::N_("Filter Style Information"),
                    crate::i18n::N_("Removes the ticked properties from stylesheets, <style> blocks and style attributes."),
                    crate::i18n::N_("Remove"),
                )
            },
            Some(bx.upcast()),
            make,
            None,
        );
    }

    // ---- media ---------------------------------------------------------

    fn tool_unused_media(&self) {
        let unused = book_tools::unused_media(&self.book.borrow());
        if unused.is_empty() {
            self.toast(&tr("Every image, font and media file is used"));
            return;
        }
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.add_css_class("boxed-list");
        let mut checks = Vec::new();
        for u in &unused {
            let row = adw::ActionRow::new();
            row.set_title(&glib::markup_escape_text(&u.path));
            row.set_subtitle(&format!(
                "{} · {}",
                tr(u.kind.label()),
                human_size(u.bytes as u64)
            ));
            let c = gtk::CheckButton::new();
            c.set_active(true);
            crate::ui::audit::label(&c, &trf("Delete {}", &[&u.path]));
            row.add_prefix(&c);
            row.set_activatable_widget(Some(&c));
            list.append(&row);
            checks.push((u.path.clone(), c));
        }
        let make: MakeOp = Rc::new(move || {
            let paths: Vec<String> = checks
                .iter()
                .filter(|(_, c)| c.is_active())
                .map(|(p, _)| p.clone())
                .collect();
            if paths.is_empty() {
                return Err("Tick the files to delete".to_string());
            }
            Ok((
                "Delete Unused Media".to_string(),
                Rc::new(move |b: &mut OpenBook, _: Option<&str>| {
                    editor_tools::delete_unused_media(b, &paths)
                }) as Op,
            ))
        });
        self.tool_dialog(
            Spec {
                height: 600,
                auto_preview: true,
                ..Spec::new(
                    crate::i18n::N_("Delete Unused Media"),
                    crate::i18n::N_("These images, fonts and media files are not used by any page, stylesheet or the package. The cover is never listed."),
                    crate::i18n::N_("Delete"),
                )
            },
            Some(scrolled(&list, 220).upcast()),
            make,
            None,
        );
    }

    fn tool_embed_fonts(&self) {
        if fonts::missing_faces(&self.book.borrow()).is_empty() {
            self.toast(&tr(
                "The book does not use any font it does not already carry",
            ));
            return;
        }
        type Chosen = (
            Vec<(String, gtk::CheckButton)>,
            adw::SwitchRow,
            Rc<Vec<fonts::SystemFont>>,
        );
        let state: Rc<RefCell<Option<Chosen>>> = Rc::new(RefCell::new(None));
        // the options appear when the search of the installed fonts is done (it runs on a worker thread)
        let holder = gtk::Box::new(gtk::Orientation::Vertical, 10);
        let searching = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        searching.set_halign(gtk::Align::Center);
        searching.set_margin_top(12);
        searching.set_margin_bottom(12);
        let spinner = gtk::Spinner::new();
        spinner.start();
        let progress = gtk::Label::new(Some(&tr("Searching the installed fonts")));
        searching.append(&spinner);
        searching.append(&progress);
        holder.append(&searching);
        let make: MakeOp = {
            let state = state.clone();
            Rc::new(move || {
                let st = state.borrow();
                let Some((checks, subset, inst)) = st.as_ref() else {
                    return Err(tr(
                        "Still searching the installed fonts, try again in a moment",
                    ));
                };
                let chosen: HashSet<String> = checks
                    .iter()
                    .filter(|(_, c)| c.is_active())
                    .map(|(f, _)| f.to_ascii_lowercase())
                    .collect();
                if chosen.is_empty() {
                    return Err("Tick the fonts to embed".to_string());
                }
                let sub = subset.is_active();
                let inst = inst.clone();
                Ok((
                    "Embed Fonts".to_string(),
                    Rc::new(move |b: &mut OpenBook, _: Option<&str>| {
                        editor_tools::embed_fonts(b, &inst, &chosen, sub)
                    }) as Op,
                ))
            })
        };
        let ui = self.tool_dialog(
            Spec {
                height: 620,
                ..Spec::new(
                    crate::i18n::N_("Embed Referenced Fonts"),
                    crate::i18n::N_("Fonts the pages ask for but the book does not carry, found among the fonts installed here. Only embed fonts whose licence allows it."),
                    crate::i18n::N_("Embed"),
                )
            },
            Some(holder.clone().upcast()),
            make,
            None,
        );
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let c = cancel.clone();
            ui.dialog
                .connect_closed(move |_| c.store(true, Ordering::Relaxed));
        }
        enum Msg {
            Progress(usize, usize),
            Done(Vec<fonts::SystemFont>),
        }
        let (tx, rx) = mpsc::channel::<Msg>();
        let c = cancel.clone();
        std::thread::spawn(move || {
            let tx2 = tx.clone();
            let found = fonts::scan_system_fonts_with(
                &fonts::system_font_dirs(),
                &mut |d, t| {
                    let _ = tx2.send(Msg::Progress(d, t));
                },
                &|| c.load(Ordering::Relaxed),
            );
            let _ = tx.send(Msg::Done(found));
        });
        let book = self.book.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(60), move || {
            let mut last = None;
            loop {
                match rx.try_recv() {
                    Ok(Msg::Progress(d, t)) => last = Some((d, t)),
                    Ok(Msg::Done(found)) => {
                        let installed = Rc::new(found);
                        let cands = editor_tools::font_candidates(&book.borrow(), &installed);
                        let list = gtk::ListBox::new();
                        list.set_selection_mode(gtk::SelectionMode::None);
                        list.add_css_class("boxed-list");
                        let mut checks: Vec<(String, gtk::CheckButton)> = Vec::new();
                        for c in &cands {
                            let row = adw::ActionRow::new();
                            row.set_title(&glib::markup_escape_text(&format!(
                                "{} · {}{}",
                                c.key.family,
                                c.key.weight,
                                if c.key.italic { " italic" } else { "" }
                            )));
                            let check = gtk::CheckButton::new();
                            crate::ui::audit::label(&check, &trf("Embed {}", &[&c.key.family]));
                            match &c.font {
                                None => {
                                    row.set_subtitle(&tr("Not installed on this computer"));
                                    check.set_sensitive(false);
                                }
                                Some(f) => {
                                    row.set_subtitle(&glib::markup_escape_text(&format!(
                                        "{} · {}",
                                        f.path.display(),
                                        tr(f.embedding.label())
                                    )));
                                    match f.embedding {
                                        fonts::Embedding::Restricted => check.set_sensitive(false),
                                        fonts::Embedding::Installable => check.set_active(true),
                                        _ => {}
                                    }
                                    if f.embedding != fonts::Embedding::Installable {
                                        let w =
                                            gtk::Image::from_icon_name("dialog-warning-symbolic");
                                        w.set_tooltip_text(Some(match f.embedding {
                                            fonts::Embedding::Restricted => {
                                                "The licence of this font forbids embedding it in a book."
                                            }
                                            fonts::Embedding::Unknown => {
                                                "This font does not say whether embedding is allowed. Check its licence."
                                            }
                                            _ => "This font may only be embedded for viewing and printing.",
                                        }));
                                        row.add_suffix(&w);
                                    }
                                }
                            }
                            row.add_prefix(&check);
                            row.set_activatable_widget(Some(&check));
                            list.append(&row);
                            checks.push((c.key.family.clone(), check));
                        }
                        let subset = switch_row(
                            &tr("Keep only the characters the book uses"),
                            &tr("Smaller files. Text you add later may need glyphs that are no longer there."),
                            true,
                        );
                        let g = group("");
                        g.add(&subset);
                        let bx = gtk::Box::new(gtk::Orientation::Vertical, 10);
                        bx.append(&scrolled(&list, 200));
                        bx.append(&g);

                        while let Some(c) = holder.first_child() {
                            holder.remove(&c);
                        }
                        holder.append(&bx);
                        *state.borrow_mut() = Some((checks, subset, installed));
                        return glib::ControlFlow::Break;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(_) => return glib::ControlFlow::Break,
                }
            }
            if let Some((d, t)) = last {
                progress.set_text(&trf("Searching the installed fonts: {} of {}", &[&d, &t]));
            }
            glib::ControlFlow::Continue
        });
    }

    // ---- structure -----------------------------------------------------

    fn tool_semantics(&self) {
        let (targets, current) = {
            let b = self.book.borrow();
            let mut files = b.spine_paths();
            for f in b.file_list() {
                if matches!(extension(&f).as_str(), "xhtml" | "html" | "htm" | "xht")
                    && !files.contains(&f)
                {
                    files.push(f);
                }
            }
            (files, semantics::read_semantics(&b))
        };
        let mut rows: Vec<(Semantic, adw::ComboRow, Vec<String>)> = Vec::new();
        let g = group("");
        for sem in semantics::ALL_SEMANTICS {
            let have = current
                .iter()
                .find(|e| e.semantic == *sem)
                .map(|e| e.href.clone());
            let mut options: Vec<String> = targets.clone();
            if let Some(h) = &have {
                if !options.contains(h) {
                    options.push(h.clone());
                }
            }
            let mut labels = vec![tr("(none)")];
            labels.extend(options.iter().cloned());
            let labs: Vec<&str> = labels.iter().map(String::as_str).collect();
            let sel = have
                .as_ref()
                .and_then(|h| options.iter().position(|o| o == h))
                .map(|i| i as u32 + 1)
                .unwrap_or(0);
            let row = combo_row(&tr(sem.label()), &labs, sel);
            row.set_use_subtitle(true);
            g.add(&row);
            rows.push((*sem, row, options));
        }
        let make: MakeOp = Rc::new(move || {
            let wanted: Vec<(Semantic, Option<String>)> = rows
                .iter()
                .map(|(s, r, opts)| {
                    let i = r.selected() as usize;
                    (
                        *s,
                        if i == 0 {
                            None
                        } else {
                            opts.get(i - 1).cloned()
                        },
                    )
                })
                .collect();
            Ok((
                "Set Semantics".to_string(),
                Rc::new(move |b: &mut OpenBook, _: Option<&str>| {
                    editor_tools::set_semantics(b, &wanted)
                }) as Op,
            ))
        });
        self.tool_dialog(
            Spec {
                height: 640,
                ..Spec::new(
                    crate::i18n::N_("Set Semantics"),
                    crate::i18n::N_("Tell readers where the cover, the title page, the table of contents, the foreword and so on are. Written to the guide and, in EPUB 3, to the landmarks."),
                    crate::i18n::N_("Apply"),
                )
            },
            Some(scrolled(&g, 300).upcast()),
            make,
            None,
        );
    }

    fn tool_accessibility(&self) {
        let (lang, images, meta, epub3) = {
            let b = self.book.borrow();
            (
                b.metadata().language.clone(),
                book_tools::image_uses(&b),
                book_tools::read_access_meta(&b),
                b.package.is_epub3(),
            )
        };
        let stack = adw::ViewStack::new();
        // -- language
        let lang_row = entry_row(&tr("Language of the book"), &lang);
        let lg = group("");
        lg.add(&lang_row);
        let lang_note = gtk::Label::new(Some(
            "A language code such as en, fr-CA or de. It is written to the book's metadata and to every page, so screen readers pick the right voice.",
        ));
        lang_note.set_wrap(true);
        lang_note.set_xalign(0.0);
        lang_note.add_css_class("dim-label");
        let lp = gtk::Box::new(gtk::Orientation::Vertical, 10);
        lp.set_margin_all(12);
        lp.append(&lg);
        lp.append(&lang_note);
        stack.add_titled(&lp, Some("language"), "Language");
        // -- images
        let all_uses = switch_row(
            &tr("Use the same description for every use of an image"),
            "",
            true,
        );
        let ig = group("");
        let mut img_rows: Vec<(
            String,
            usize,
            Option<String>,
            adw::EntryRow,
            gtk::CheckButton,
        )> = Vec::new();
        for u in images.iter().take(300) {
            let row = adw::EntryRow::new();
            row.set_title(&trf("{} in {}", &[&file_name(&u.src), &file_name(&u.file)]));
            row.set_text(u.alt.as_deref().unwrap_or(""));
            let deco = gtk::CheckButton::with_label(&tr("Decorative"));
            deco.set_active(u.alt.as_deref() == Some(""));
            row.add_suffix(&deco);
            {
                let row = row.clone();
                deco.connect_toggled(move |d| {
                    if d.is_active() {
                        row.set_text("");
                    }
                });
            }
            ig.add(&row);
            img_rows.push((u.file.clone(), u.index, u.alt.clone(), row, deco));
        }
        let ip = gtk::Box::new(gtk::Orientation::Vertical, 10);
        ip.set_margin_all(12);
        if images.is_empty() {
            let s = adw::StatusPage::new();
            s.set_title(&tr("No images"));
            s.set_icon_name(Some("image-x-generic-symbolic"));
            ip.append(&s);
        } else {
            let note = gtk::Label::new(Some(
                "Describe what each image shows. Mark purely decorative images so screen readers skip them.",
            ));
            note.set_wrap(true);
            note.set_xalign(0.0);
            note.add_css_class("dim-label");
            ip.append(&note);
            let g2 = group("");
            g2.add(&all_uses);
            ip.append(&g2);
            ip.append(&scrolled(&ig, 180));
        }
        stack.add_titled(&ip, Some("images"), "Image descriptions");
        // -- metadata
        let summary = entry_row(&tr("Summary"), &meta.summary);
        let conf = entry_row(&tr("Conforms to"), &meta.conformance);
        let suff = entry_row(
            &tr("Sufficient modes (comma separated)"),
            &meta.sufficient_modes.join(","),
        );
        let mg = group("");
        mg.add(&summary);
        mg.add(&suff);
        mg.add(&conf);
        let mk_checks = |title: &str, all: &[&str], on: &std::collections::BTreeSet<String>| {
            let ex = adw::ExpanderRow::new();
            ex.set_title(title);
            let mut cs = Vec::new();
            for v in all {
                let r = adw::ActionRow::new();
                r.set_title(v);
                let c = gtk::CheckButton::new();
                c.set_active(on.contains(*v));
                crate::ui::audit::label(&c, &format!("{title}: {v}"));
                r.add_prefix(&c);
                r.set_activatable_widget(Some(&c));
                ex.add_row(&r);
                cs.push((v.to_string(), c));
            }
            (ex, cs)
        };
        let (modes_ex, modes) =
            mk_checks("Access modes", book_tools::ACCESS_MODES, &meta.access_modes);
        let (feat_ex, feats) = mk_checks("Features", book_tools::ACCESS_FEATURES, &meta.features);
        let (haz_ex, hazs) = mk_checks("Hazards", book_tools::ACCESS_HAZARDS, &meta.hazards);
        mg.add(&modes_ex);
        mg.add(&feat_ex);
        mg.add(&haz_ex);
        let mp = gtk::Box::new(gtk::Orientation::Vertical, 10);
        mp.set_margin_all(12);
        if epub3 {
            mp.append(&scrolled(&mg, 280));
        } else {
            let s = adw::StatusPage::new();
            s.set_title(&tr("Needs EPUB 3"));
            s.set_description(Some(&tr(
                "Accessibility metadata is part of EPUB 3. Use Tools → Upgrade to EPUB 3 first.",
            )));
            s.set_icon_name(Some("dialog-information-symbolic"));
            mp.append(&s);
        }
        stack.add_titled(&mp, Some("metadata"), "Accessibility metadata");
        let switcher = adw::ViewSwitcher::new();
        switcher.set_stack(Some(&stack));
        switcher.set_policy(adw::ViewSwitcherPolicy::Wide);
        let bx = gtk::Box::new(gtk::Orientation::Vertical, 6);
        bx.append(&switcher);
        bx.append(&stack);

        let orig_lang = lang.clone();
        let orig_meta = meta.clone();
        let make: MakeOp = Rc::new(move || {
            let language = {
                let t = lang_row.text().to_string();
                (t.trim() != orig_lang).then_some(t)
            };
            let alts: Vec<(String, usize, String)> = img_rows
                .iter()
                .filter_map(|(f, i, old, row, deco)| {
                    let text = if deco.is_active() {
                        String::new()
                    } else {
                        row.text().to_string()
                    };
                    let was = old.clone().unwrap_or_default();
                    (text != was || (old.is_none() && deco.is_active()))
                        .then(|| (f.clone(), *i, text))
                })
                .collect();
            let collect =
                |cs: &[(String, gtk::CheckButton)]| -> std::collections::BTreeSet<String> {
                    cs.iter()
                        .filter(|(_, c)| c.is_active())
                        .map(|(v, _)| v.clone())
                        .collect()
                };
            let new_meta = epub3.then(|| AccessMeta {
                summary: summary.text().trim().to_string(),
                access_modes: collect(&modes),
                sufficient_modes: suff
                    .text()
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect(),
                features: collect(&feats),
                hazards: collect(&hazs),
                conformance: conf.text().trim().to_string(),
            });
            let edits = AccessEdits {
                language,
                alts,
                all_uses: all_uses.is_active(),
                meta: new_meta.filter(|m| *m != orig_meta),
            };
            Ok((
                "Check Accessibility".to_string(),
                Rc::new(move |b: &mut OpenBook, _: Option<&str>| {
                    editor_tools::accessibility(b, &edits)
                }) as Op,
            ))
        });
        self.tool_dialog(
            Spec {
                height: 700,
                ..Spec::new(
                    crate::i18n::N_("Check Accessibility"),
                    crate::i18n::N_("Language, image descriptions and the accessibility metadata that stores and shops show."),
                    crate::i18n::N_("Apply"),
                )
            },
            Some(bx.upcast()),
            make,
            None,
        );
    }

    // ---- links ---------------------------------------------------------

    fn tool_repair_links(&self) {
        let make: MakeOp = Rc::new(|| {
            Ok((
                "Repair Broken Links".to_string(),
                Rc::new(|b: &mut OpenBook, _: Option<&str>| editor_tools::repair_links(b)) as Op,
            ))
        });
        self.tool_dialog(
            Spec {
                auto_preview: true,
                height: 420,
                ..Spec::new(
                    crate::i18n::N_("Repair Broken Links"),
                    crate::i18n::N_("Fixes links to files and anchors that no longer exist when a clear match is found (a different capitalisation, a renamed file). The others are listed."),
                    crate::i18n::N_("Repair"),
                )
            },
            None,
            make,
            None,
        );
    }

    fn tool_move_files(&self) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            self.toast(&tr("Select the files to move in the file list first"));
            return;
        }
        let folder = entry_row(&tr("Folder inside the book's content folder"), &tr("Text"));
        let g = group("");
        g.add(&folder);
        let make: MakeOp = Rc::new({
            let paths = paths.clone();
            move || {
                let f = folder.text().to_string();
                let paths = paths.clone();
                Ok((
                    "Move Files".to_string(),
                    Rc::new(move |b: &mut OpenBook, _: Option<&str>| {
                        let mut all = editor_tools::Outcome {
                            changed: false,
                            summary: String::new(),
                            details: vec![],
                        };
                        for p in &paths {
                            let o = editor_tools::move_to_folder(b, p, &f)?;
                            all.changed |= o.changed;
                            all.details.extend(o.details);
                        }
                        all.summary = trf(
                            "Moved {}",
                            &[&ntr("{} file", "{} files", all.details.len())],
                        );
                        Ok(all)
                    }) as Op,
                ))
            }
        });
        self.tool_dialog(
            Spec::new(
                crate::i18n::N_("Move Files to a Folder"),
                trf("Moves the {} selected file(s) to another folder of the book. Every link to them, and every link they contain, is updated.", &[&paths.len()]),
                crate::i18n::N_("Move"),
            ),
            Some(g.upcast()),
            make,
            None,
        );
    }

    fn tool_check_links(&self) {
        if !editor_tools::network_allowed() {
            self.toast(&tr("Network tools are switched off"));
            return;
        }
        let ext = links::external_links(&self.book.borrow());
        let mut urls: Vec<String> = Vec::new();
        for l in &ext {
            if matches!(
                l.kind,
                links::LinkKind::Http | links::LinkKind::Https | links::LinkKind::Ftp
            ) && !urls.contains(&l.url)
            {
                urls.push(l.url.clone());
            }
        }
        if urls.is_empty() {
            self.toast(&tr("The book has no external links to check"));
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let tx_ui = self.sender.input_sender().clone();
        let book = self.book.clone();
        let n = urls.len();
        let custom: Rc<dyn Fn(&ToolUi)> = {
            let cancel = cancel.clone();
            Rc::new(move |ui: &ToolUi| {
                ui.apply.set_sensitive(false);
                ui.preview.set_text(&trf("Checking 0 of {}…", &[&n]));
                let progress = gtk::ProgressBar::new();
                progress.set_widget_name("links-progress");
                progress.set_margin_all(12);
                crate::ui::audit::label(&progress, &tr("Progress of the link check"));
                if let Some(card) = ui.preview.parent().and_downcast::<gtk::Box>() {
                    card.append(&progress);
                }
                let stop = gtk::Button::with_label(&tr("Cancel"));
                stop.set_widget_name("links-cancel");
                if let Some(bar) = ui.apply.parent().and_downcast::<gtk::Box>() {
                    bar.insert_child_after(&stop, None::<&gtk::Widget>);
                }
                {
                    let c = cancel.clone();
                    stop.connect_clicked(move |b| {
                        c.store(true, Ordering::Relaxed);
                        b.set_sensitive(false);
                    });
                }
                let (tx, rx) = mpsc::channel::<Result<(usize, usize), Vec<(String, LinkStatus)>>>();
                let urls = urls.clone();
                let c = cancel.clone();
                std::thread::spawn(move || {
                    let fetcher = editor_tools::net_fetcher();
                    let start = std::time::Instant::now();
                    let tx2 = tx.clone();
                    let done = editor_tools::check_urls(
                        &urls,
                        &*fetcher,
                        400,
                        &mut || start.elapsed().as_millis() as u64,
                        &mut |ms| std::thread::sleep(std::time::Duration::from_millis(ms)),
                        &mut |d, t| {
                            let _ = tx2.send(Ok((d, t)));
                        },
                        &|| c.load(Ordering::Relaxed),
                    );
                    let _ = tx.send(Err(done));
                });
                let label = ui.preview.clone();
                let dialog = ui.dialog.clone();
                let tx_ui = tx_ui.clone();
                let book = book.clone();
                let (progress, stop) = (progress.clone(), stop.clone());
                glib::timeout_add_local(std::time::Duration::from_millis(80), move || loop {
                    match rx.try_recv() {
                        Ok(Ok((d, t))) => {
                            label.set_text(&trf("Checking {} of {}…", &[&d, &t]));
                            progress.set_fraction(if t == 0 { 0.0 } else { d as f64 / t as f64 });
                        }
                        Ok(Err(done)) => {
                            progress.set_visible(false);
                            stop.set_visible(false);
                            let checker = editor_tools::Gathered(done.iter().cloned().collect());
                            let res = links::check_external_links(&book.borrow(), &checker);
                            let (broken, ok, lines) = editor_tools::link_report(&res);
                            let mut text = vec![format!(
                                "{} checked: {ok} fine, {broken} broken",
                                done.len()
                            )];
                            text.extend(lines.into_iter().take(14));
                            label.set_text(&text.join("\n"));
                            tx_ui.emit(Input::Ext(Cmd::LinkResults(done)));
                            dialog.set_title(&tr("External Links Checked"));
                            return glib::ControlFlow::Break;
                        }
                        Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                        Err(_) => return glib::ControlFlow::Break,
                    }
                });
            })
        };
        let make: MakeOp = Rc::new(|| Err("This tool only reads the network".into()));
        let ui = self.tool_dialog(
            Spec {
                height: 440,
                ..Spec::new(
                    crate::i18n::N_("Check External Links"),
                    trf("Visits the {} web address(es) the book links to, one every few moments, and lists the ones that do not answer. Nothing in the book changes.", &[&n]),
                    crate::i18n::N_("Start"),
                )
            },
            None,
            make,
            Some(custom),
        );
        ui.dialog
            .connect_closed(move |_| cancel.store(true, Ordering::Relaxed));
    }

    fn tool_download(&self) {
        if !editor_tools::network_allowed() {
            self.toast(&tr("Network tools are switched off"));
            return;
        }
        let urls = links::remote_resources(&self.book.borrow());
        if urls.is_empty() {
            self.toast(&tr("The book does not use any resource from the web"));
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let tx_ui = self.sender.input_sender().clone();
        let n = urls.len();
        let list = urls.join("\n");
        let custom: Rc<dyn Fn(&ToolUi)> = {
            let cancel = cancel.clone();
            Rc::new(move |ui: &ToolUi| {
                ui.apply.set_sensitive(false);
                ui.preview
                    .set_text(&trf("Downloading {} resource(s)…", &[&n]));
                let (tx, rx) = mpsc::channel::<links::MockFetcher>();
                let urls = urls.clone();
                let c = cancel.clone();
                std::thread::spawn(move || {
                    let fetcher = editor_tools::net_fetcher();
                    let got =
                        editor_tools::prefetch(&urls, &*fetcher, 250, 20 * 1024 * 1024, &|| {
                            c.load(Ordering::Relaxed)
                        });
                    let _ = tx.send(got);
                });
                let dialog = ui.dialog.clone();
                let tx_ui = tx_ui.clone();
                glib::timeout_add_local(std::time::Duration::from_millis(80), move || {
                    match rx.try_recv() {
                        Ok(got) => {
                            let got = Rc::new(got);
                            tx_ui.emit(Input::Ext(Cmd::Run(
                                "Download External Resources".to_string(),
                                OpBox(Rc::new(move |b: &mut OpenBook, _: Option<&str>| {
                                    editor_tools::download_resources(b, &got)
                                })),
                            )));
                            dialog.close();
                            glib::ControlFlow::Break
                        }
                        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(_) => glib::ControlFlow::Break,
                    }
                });
            })
        };
        let make: MakeOp = Rc::new(|| Err("Press Download".into()));
        let ui = self.tool_dialog(
            Spec {
                height: 480,
                ..Spec::new(
                    crate::i18n::N_("Download External Resources"),
                    crate::i18n::N_("Images, stylesheets and media the pages load from the web are copied into the book and the links point at the copies. Only download what you are allowed to keep: check the licences."),
                    crate::i18n::N_("Download"),
                )
            },
            None,
            make,
            Some(custom),
        );
        ui.preview
            .set_text(&trf("{} resource(s):\n{}", &[&n, &list]));
        ui.dialog
            .connect_closed(move |_| cancel.store(true, Ordering::Relaxed));
    }

    /// The final result of a link check: broken links go to the results list, with a jump to the page.
    pub(super) fn show_link_results(&mut self, done: Vec<(String, LinkStatus)>) {
        let checker = editor_tools::Gathered(
            done.into_iter()
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        );
        let res = links::check_external_links(&self.book.borrow(), &checker);
        let (broken, ok, _) = editor_tools::link_report(&res);
        let mut rows = Vec::new();
        let mut rrows = Vec::new();
        for r in res
            .iter()
            .filter(|r| !matches!(r.status, LinkStatus::Ok | LinkStatus::Skipped))
        {
            let why = match &r.status {
                LinkStatus::NotFound => "not found".to_string(),
                LinkStatus::Redirect(to) => format!("moved to {to}"),
                LinkStatus::Error(e) => e.clone(),
                LinkStatus::Invalid(e) => format!("invalid: {e}"),
                _ => String::new(),
            };
            rows.push((
                "dialog-warning-symbolic".to_string(),
                r.link.url.clone(),
                format!("{} — {why}", r.link.file),
                "issue-warning",
            ));
            rrows.push(ResultRow {
                file: r.link.file.clone(),
                line: None,
                range: None,
            });
        }
        let title = format!("{ok} links fine, {broken} broken");
        *self.last_toast.borrow_mut() = title.clone();
        if rows.is_empty() {
            self.toast(&title);
            return;
        }
        self.results = rrows;
        self.result_kind = ResultKind::Check;
        self.show_results(&title, rows);
        self.toast(&title);
    }
}

#[cfg(test)]
mod menu_tests {
    use super::*;

    fn labels(m: &gio::MenuModel, out: &mut Vec<String>) {
        for i in 0..m.n_items() {
            if let Some(v) = m.item_attribute_value(i, "label", Some(glib::VariantTy::STRING)) {
                out.push(v.str().unwrap_or_default().to_string());
            }
            for link in ["section", "submenu"] {
                if let Some(s) = m.item_link(i, link) {
                    labels(&s, out);
                }
            }
        }
    }

    #[test]
    fn the_tools_menu_has_at_most_twelve_top_level_entries_and_no_repeated_label() {
        let plugins = gio::Menu::new();
        plugins.append(Some("A plugin"), Some("tools.run"));
        let (menu, _) = build_tools_menu(&gio::Menu::new(), &gio::Menu::new(), Some(&plugins));
        let top = super::super::window_features::top_level_labels(menu.upcast_ref());
        assert!(top.len() <= 12, "{top:?}");
        let mut all = Vec::new();
        labels(menu.upcast_ref(), &mut all);
        let mut seen = std::collections::HashSet::new();
        for l in &all {
            assert!(seen.insert(l.clone()), "{l} appears twice");
            assert!(!l.ends_with("(plugin)"));
        }
        assert!(top.contains(&"Check Book".to_string()));
    }

    #[test]
    fn every_tool_dialog_is_reachable_from_the_menu_once() {
        let ids: Vec<&str> = TOOL_SUBMENUS
            .iter()
            .flat_map(|(_, secs)| secs.iter().flat_map(|s| s.iter()))
            .filter_map(|i| match i {
                Item::Dialog(_, id) => Some(*id),
                _ => None,
            })
            .collect();
        for id in [
            "smarten",
            "fix-html",
            "beautify",
            "styles",
            "unused-css",
            "filter",
            "unused-media",
            "embed-fonts",
            "download",
            "semantics",
            "accessibility",
            "check-links",
            "repair-links",
            "move-files",
        ] {
            assert_eq!(ids.iter().filter(|i| **i == id).count(), 1, "{id}");
        }
    }
}
