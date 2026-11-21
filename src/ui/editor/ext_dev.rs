//! Dev hooks (`BOOKX_DEV_SCRIPT`: `editor:x:<command>`) for the tools, file browser and window features of this
//! change; test infrastructure for build-aux/ui-tests.sh. They run the same code as the dialogs, minus the dialog.

use bookx_convert::transforms::punctuation::SmartenOptions;
use bookx_editing::book_tools::{self, PageRole, Punctuation, QuoteStyle};
use bookx_editing::filter_styles::FilterSpec;
use bookx_editing::links;
use bookx_editing::semantics::{self, Semantic};
use bookx_editing::transform_css::RuleSet;

use super::*;
use crate::editor_state;
use crate::editor_tools::AccessEdits;

/// Text of a dev command: `\n` is a line break and `§` a comma (the script is comma separated).
fn dev_text(t: &str) -> String {
    t.replace("\\n", "\n").replace('§', ",")
}

fn out(topic: &str, text: impl std::fmt::Display) {
    eprintln!("editor_x: {topic}: {text}");
}

impl Editor {
    fn dev_pages(&self, scope: &str) -> Vec<String> {
        match scope {
            "all" | "" => Vec::new(),
            "current" => self.current.clone().into_iter().collect(),
            p => vec![p.to_string()],
        }
    }

    pub(super) fn ext_dev(&mut self, c: &str) {
        let (name, arg) = c.split_once('=').unwrap_or((c, ""));
        self.flush();
        match name {
            "dialog" => self.open_tool(arg),
            // fixtures for the scenarios: no checkpoint, the editor reloads from the archive
            "put" => {
                // put=<path>|<text, \n for a line break>
                if let Some((p, t)) = arg.split_once('|') {
                    self.book.borrow_mut().archive.set_string(p, &dev_text(t));
                    self.after_archive_changed();
                }
            }
            "write-font" | "write-png" | "write-file" => {
                // files outside the book: fonts to find, canned web answers, templates
                let (path, text) = arg.split_once('|').unwrap_or((arg, ""));
                let data = match name {
                    "write-font" => bookx_editing::testfont::tiny_ttf(0),
                    "write-png" => bookx_sources::cover::generate_cover("T", "A", "", 120, 160)
                        .unwrap_or_default(),
                    _ => dev_text(text).into_bytes(),
                };
                let p = std::path::Path::new(path);
                if let Some(d) = p.parent() {
                    let _ = std::fs::create_dir_all(d);
                }
                out("written", std::fs::write(p, data).is_ok());
            }
            "write-template" => {
                let (file, text) = arg.split_once('|').unwrap_or((arg, ""));
                if let Some(d) = editor_state::templates_dir() {
                    let _ = std::fs::create_dir_all(&d);
                    out(
                        "template",
                        std::fs::write(d.join(file), dev_text(text)).is_ok(),
                    );
                }
            }
            "add" | "add-png" | "add-font" => {
                let (n, text) = arg.split_once('|').unwrap_or((arg, ""));
                let data = match name {
                    "add-png" => bookx_sources::cover::generate_cover("T", "A", "", 120, 160)
                        .unwrap_or_default(),
                    "add-font" => bookx_editing::testfont::tiny_ttf(0),
                    _ => dev_text(text).into_bytes(),
                };
                let res =
                    bookx_editing::files::add_file(&mut self.book.borrow_mut(), n, data, None);
                match res {
                    Ok(p) => out("added", p),
                    Err(e) => out("added", format!("failed {e}")),
                }
                self.after_archive_changed();
            }
            "toast" => self.toast(arg),
            "dialog-click" => {
                let h = self.ext.last_dialog.borrow().clone();
                match (h, arg) {
                    (Some((p, _, _)), "preview") => p.emit_clicked(),
                    (Some((_, a, _)), "apply") => a.emit_clicked(),
                    _ => out("dialog", "none open"),
                }
            }
            "dialog-text" => out(
                "dialog",
                self.ext
                    .last_dialog
                    .borrow()
                    .as_ref()
                    .map(|(_, _, l)| l.text().replace('\n', " / "))
                    .unwrap_or_default(),
            ),
            "smarten" => {
                // smarten=<scope>|smarten or unsmarten|<language>|<letters of q d e>
                let p: Vec<&str> = arg.split('|').collect();
                let pages = self.dev_pages(p.first().copied().unwrap_or("all"));
                let mode = if p.get(1) == Some(&"unsmarten") {
                    Punctuation::Unsmarten
                } else {
                    let l = p.get(3).copied().unwrap_or("qde");
                    Punctuation::Smarten(SmartenOptions {
                        quotes: l.contains('q'),
                        dashes: l.contains('d'),
                        ellipses: l.contains('e'),
                    })
                };
                let style =
                    QuoteStyle::from_id(p.get(2).copied().unwrap_or("en")).unwrap_or_default();
                self.run_op("Smarten Punctuation", move |b, _| {
                    editor_tools::smarten(b, &pages, mode, style)
                });
            }
            "fixhtml" => {
                let (scope, html5) = arg.split_once('|').unwrap_or((arg, ""));
                let pages = self.dev_pages(scope);
                self.run_op("Fix HTML", move |b, _| {
                    editor_tools::fix_html(b, &pages, html5 == "html5")
                });
            }
            "beautify" => {
                let pages = self.dev_pages(arg);
                self.run_op("Reformat Markup", move |b, _| {
                    editor_tools::beautify(b, &pages)
                });
            }
            "styles" => {
                let rules: Result<Vec<_>, _> = arg
                    .split(";;")
                    .filter(|r| !r.is_empty())
                    .map(editor_tools::rules::parse_rule)
                    .collect();
                match rules {
                    Ok(r) => self.run_op("Transform Styles", move |b, _| {
                        editor_tools::transform_styles(b, &r)
                    }),
                    Err(e) => self.toast(&e),
                }
            }
            "ruleset-save" => {
                // ruleset-save=Name;;rule;;rule
                let mut it = arg.split(";;");
                let name = it.next().unwrap_or("").to_string();
                let rules: Result<Vec<_>, _> = it.map(editor_tools::rules::parse_rule).collect();
                match (rules, editor_state::rules_dir()) {
                    (Ok(rules), Some(d)) => {
                        let r = editor_state::save_rule_set_in(&d, &RuleSet { name, rules });
                        out("ruleset-save", r.is_ok());
                    }
                    (Err(e), _) => out("ruleset-save", e),
                    _ => {}
                }
            }
            "ruleset-load" => {
                let set = editor_state::rules_dir()
                    .map(|d| editor_state::list_rule_sets_in(&d))
                    .and_then(|v| v.into_iter().find(|s| s.name == arg));
                match set {
                    Some(s) => self.run_op("Transform Styles", move |b, _| {
                        editor_tools::transform_styles(b, &s.rules)
                    }),
                    None => out("ruleset-load", "not found"),
                }
            }
            "ruleset-list" => out(
                "rulesets",
                editor_state::rules_dir()
                    .map(|d| editor_state::list_rule_sets_in(&d))
                    .unwrap_or_default()
                    .iter()
                    .map(|s| format!("{}({})", s.name, s.rules.len()))
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            "unused-css" => self.run_op("Remove Unused CSS", |b, _| {
                editor_tools::remove_unused_css(b)
            }),
            "unused-media-list" => out(
                "unused-media",
                book_tools::unused_media(&self.book.borrow())
                    .iter()
                    .map(|u| u.path.clone())
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            "unused-media" => {
                let paths: Vec<String> = if arg.is_empty() {
                    book_tools::unused_media(&self.book.borrow())
                        .into_iter()
                        .map(|u| u.path)
                        .collect()
                } else {
                    arg.split('|').map(str::to_string).collect()
                };
                self.run_op("Delete Unused Media", move |b, _| {
                    editor_tools::delete_unused_media(b, &paths)
                });
            }
            "fonts-list" => {
                let installed = bookx_editing::fonts::scan_system_fonts(
                    &bookx_editing::fonts::system_font_dirs(),
                );
                let c = editor_tools::font_candidates(&self.book.borrow(), &installed);
                out(
                    "fonts",
                    c.iter()
                        .map(|c| {
                            format!(
                                "{}:{}",
                                c.key.family,
                                c.font
                                    .as_ref()
                                    .map(|f| f.embedding.label())
                                    .unwrap_or("not installed")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(" | "),
                );
            }
            "embed" => {
                let (fams, subset) = arg.split_once('|').unwrap_or((arg, "subset"));
                let chosen: HashSet<String> =
                    fams.split(',').map(|s| s.to_ascii_lowercase()).collect();
                let installed = bookx_editing::fonts::scan_system_fonts(
                    &bookx_editing::fonts::system_font_dirs(),
                );
                self.run_op("Embed Fonts", move |b, _| {
                    editor_tools::embed_fonts(b, &installed, &chosen, subset == "subset")
                });
            }
            "semantics" => {
                // semantics=<guide type>:<path>;<guide type>:-
                let wanted: Vec<(Semantic, Option<String>)> = arg
                    .split(';')
                    .filter_map(|e| {
                        let (k, v) = e.split_once(':')?;
                        Some((
                            Semantic::from_guide_type(k)?,
                            (v != "-").then(|| v.to_string()),
                        ))
                    })
                    .collect();
                self.run_op("Set Semantics", move |b, _| {
                    editor_tools::set_semantics(b, &wanted)
                });
            }
            "semantics-dump" => out(
                "semantics",
                semantics::read_semantics(&self.book.borrow())
                    .iter()
                    .map(|e| format!("{}={}", e.semantic.guide_type(), e.href))
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            "access" => {
                // access=lang:de;alt:<file>#<index>=<text>;summary:<text>;all
                let mut e = AccessEdits::default();
                let mut meta = book_tools::read_access_meta(&self.book.borrow());
                let mut set_meta = false;
                for part in arg.split(';') {
                    if let Some(l) = part.strip_prefix("lang:") {
                        e.language = Some(l.to_string());
                    } else if let Some(a) = part.strip_prefix("alt:") {
                        if let Some((loc, text)) = a.split_once('=') {
                            if let Some((f, i)) = loc.split_once('#') {
                                e.alts.push((
                                    f.to_string(),
                                    i.parse().unwrap_or(0),
                                    text.to_string(),
                                ));
                            }
                        }
                    } else if let Some(s) = part.strip_prefix("summary:") {
                        meta.summary = s.to_string();
                        set_meta = true;
                    } else if let Some(m) = part.strip_prefix("mode:") {
                        meta.access_modes.insert(m.to_string());
                        set_meta = true;
                    } else if part == "all" {
                        e.all_uses = true;
                    }
                }
                if set_meta {
                    e.meta = Some(meta);
                }
                self.run_op("Accessibility", move |b, _| {
                    editor_tools::accessibility(b, &e)
                });
            }
            "access-dump" => {
                let b = self.book.borrow();
                let imgs = book_tools::image_uses(&b);
                let m = book_tools::read_access_meta(&b);
                out(
                    "access",
                    format!(
                        "lang={} images={} alts={} summary={:?} modes={}",
                        b.metadata().language,
                        imgs.len(),
                        imgs.iter()
                            .map(|i| format!("{}#{}={:?}", file_name(&i.file), i.index, i.alt))
                            .collect::<Vec<_>>()
                            .join(","),
                        m.summary,
                        m.access_modes.iter().cloned().collect::<Vec<_>>().join(",")
                    ),
                );
            }
            "filter-list" => out(
                "filter",
                bookx_editing::filter_styles::list_properties(&self.book.borrow())
                    .iter()
                    .map(|(k, v)| format!("{k}:{v}"))
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            "filter" => {
                let spec = FilterSpec {
                    properties: arg.split(',').map(str::to_string).collect(),
                    remove_empty_rules: true,
                    remove_empty_style_attrs: true,
                };
                self.run_op("Filter Style Information", move |b, _| {
                    editor_tools::filter_styles(b, &spec)
                });
            }
            "links-check" => {
                if !editor_tools::network_allowed() {
                    out("links", "network off");
                    return;
                }
                let urls: Vec<String> = links::external_links(&self.book.borrow())
                    .into_iter()
                    .filter(|l| matches!(l.kind, links::LinkKind::Http | links::LinkKind::Https))
                    .map(|l| l.url)
                    .fold(Vec::new(), |mut v, u| {
                        if !v.contains(&u) {
                            v.push(u);
                        }
                        v
                    });
                let f = editor_tools::net_fetcher();
                let done = editor_tools::check_urls(
                    &urls,
                    &*f,
                    0,
                    &mut || 0,
                    &mut |_| {},
                    &mut |_, _| {},
                    &|| false,
                );
                out(
                    "links",
                    done.iter()
                        .map(|(u, s)| format!("{u}={s:?}"))
                        .collect::<Vec<_>>()
                        .join(" "),
                );
                self.show_link_results(done);
            }
            "download" => {
                if !editor_tools::network_allowed() {
                    out("download", "network off");
                    return;
                }
                let urls = links::remote_resources(&self.book.borrow());
                let f = editor_tools::net_fetcher();
                let got = Rc::new(editor_tools::prefetch(&urls, &*f, 0, 20 << 20, &|| false));
                self.run_op("Download External Resources", move |b, _| {
                    editor_tools::download_resources(b, &got)
                });
            }
            "repair-links" => {
                self.run_op("Repair Broken Links", |b, _| editor_tools::repair_links(b))
            }
            "move" => {
                let paths = self.selected_paths();
                let folder = arg.to_string();
                self.run_op("Move Files", move |b, _| {
                    let mut last = None;
                    for p in &paths {
                        last = Some(editor_tools::move_to_folder(b, p, &folder)?);
                    }
                    last.ok_or_else(|| anyhow::anyhow!("Select files first"))
                });
            }
            "ext" => {
                let mut it = arg.split('|');
                let ext = it.next().unwrap_or("").to_string();
                let paths: Vec<String> = it.map(str::to_string).collect();
                let paths = if paths.is_empty() {
                    self.selected_paths()
                } else {
                    paths
                };
                self.run_op("Change Extension", move |b, _| {
                    editor_tools::change_extension(b, &paths, &ext)
                });
            }
            "replace" => {
                let p: Vec<&str> = arg.split('|').collect();
                if let [path, name, src] = p[..] {
                    self.replace_from_disk(path, name, std::path::Path::new(src));
                }
            }
            "mark" => {
                let role = match arg {
                    "cover" => PageRole::Cover,
                    "title" => PageRole::TitlePage,
                    _ => PageRole::Toc,
                };
                if let Some(p) = self.current.clone() {
                    self.run_op("Mark Page", move |b, _| {
                        editor_tools::mark_role(b, &p, role)
                    });
                }
            }
            "nonlinear" => {
                if let Some(p) = self.current.clone() {
                    self.run_op("Reading Order", move |b, _| {
                        editor_tools::toggle_nonlinear(b, &p)
                    });
                }
            }
            "drop" => {
                let p: Vec<&str> = arg.split('|').collect();
                if let [from, onto, side] = p[..] {
                    self.drop_row(from, onto, side == "after");
                }
            }
            "spine" => out("spine", self.book.borrow().spine_paths().join(" ")),
            "select-group" => self.files_action(&format!("select:{arg}")),
            "selected" => out("selected", self.selected_paths().join(" ")),
            "groups" => {
                // the rows of the file list in order, with their category
                let mut parts = Vec::new();
                let mut row = self.w.files.first_child();
                while let Some(r) = row {
                    if let Some(r) = r.downcast_ref::<gtk::ListBoxRow>() {
                        if let Some(p) = super::row_data::path(r) {
                            parts.push(format!("{}:{}", r.widget_name(), p));
                        }
                    }
                    row = r.next_sibling();
                }
                out("groups", parts.join(" "));
            }
            "fileinfo" => {
                let tip = self
                    .file_rows
                    .get(arg)
                    .and_then(|(r, _)| r.tooltip_text())
                    .map(|t| t.replace('\n', " / "))
                    .unwrap_or_default();
                let notes = book_tools::file_notes(&self.book.borrow(), arg);
                out("fileinfo", format!("{arg} notes={notes:?} tip={tip}"));
            }
            "rowlabel" => {
                let label = self.file_rows.get(arg).and_then(|(r, _)| {
                    let bx = r.child()?;
                    let mut c = bx.first_child();
                    while let Some(w) = c {
                        if let Some(l) = w.downcast_ref::<gtk::Label>() {
                            return Some(l.text().to_string());
                        }
                        c = w.next_sibling();
                    }
                    None
                });
                out("rowlabel", label.unwrap_or_default());
            }
            "fullpaths" => {
                let want = arg == "on";
                if self.ext.prefs.borrow().full_paths != want {
                    self.files_action("full-paths");
                }
            }
            "grid" => self.files_action("grid"),
            "open-with-export" => {
                self.files_action("open-with");
            }
            "external-accept" => {
                let pending = self.ext.pending_external.borrow_mut().take();
                match pending {
                    Some((p, t)) => self.replace_from_disk(&p, "", &t),
                    None => out("external", "nothing pending"),
                }
            }
            "external-pending" => out(
                "external",
                self.ext
                    .pending_external
                    .borrow()
                    .as_ref()
                    .map(|(p, _)| p.clone())
                    .unwrap_or_default(),
            ),
            "external-changed" => {
                if let Some((p, t)) = arg.split_once('|') {
                    self.replace_from_disk(p, "", std::path::Path::new(t));
                }
            }
            "newfile-user" => {
                let p: Vec<&str> = arg.split('|').collect();
                let idx = p.get(1).and_then(|i| i.parse().ok()).unwrap_or(0);
                self.new_file_from_user(
                    p.first().copied().unwrap_or(""),
                    idx,
                    p.get(2) == Some(&"link"),
                );
            }
            "templates" => out(
                "templates",
                editor_state::user_templates()
                    .iter()
                    .map(|t| format!("{}.{}", t.label, t.extension))
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            // ---- window
            "save-copy" => self.save_copy(std::path::Path::new(arg), false),
            "save-copy-edit" => self.save_copy(std::path::Path::new(arg), true),
            "undo-ckpt" => self.revert_before(),
            "redo-ckpt" => self.revert_after(),
            "close-others" => self.close_tabs(editor_state::CloseMode::Others, None),
            "close-right" => self.close_tabs(editor_state::CloseMode::Right, None),
            "wrap" => self.window_action(&format!("wrap:{}", arg == "on")),
            "wrap-status" => out("wrap", format!("{:?}", self.w.view.wrap_mode())),
            "open-book" => self.switch_book(arg.parse().unwrap_or(0)),
            "new-book" => self.new_book(arg),
            "import-file" => self.import_file(std::path::Path::new(arg)),
            "open-folder" => self.open_folder(std::path::Path::new(arg)),
            "state-save" => self.save_state(),
            "state-restore" => self.restore_state(),
            "state-dump" => {
                let s = editor_state::StateStore::load();
                out(
                    "state",
                    s.get(&self.book_id.to_string())
                        .map(|b| {
                            format!(
                                "tabs={} current={:?} cursors={:?}",
                                b.tabs.join(","),
                                b.current,
                                b.cursors
                            )
                        })
                        .unwrap_or_else(|| "none".into()),
                );
            }
            "help" => out("help", super::window_features::HELP_URL),
            "open-names" => {
                let (names, text) = arg.split_once('|').unwrap_or((arg, ""));
                let names: Vec<String> = names
                    .split(',')
                    .filter(|n| !n.is_empty())
                    .map(str::to_string)
                    .collect();
                self.open_names(&names, (!text.is_empty()).then_some(text));
            }
            "status" => out(
                "status",
                format!(
                    "book={} files={} checkpoints={} redo={} current={}",
                    self.book_id,
                    self.book.borrow().file_list().len(),
                    self.checkpoints.len(),
                    self.ext.redo.borrow().len(),
                    self.current.clone().unwrap_or_default()
                ),
            ),
            other => {
                if !self.struct_dev(other, arg) {
                    self.toast(&format!("Unknown dev command x:{other}"));
                }
            }
        }
    }
}
