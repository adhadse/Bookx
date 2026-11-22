//! Dev hooks driven by `BOOKX_DEV_SCRIPT` (`editor:...`); test infrastructure for build-aux/ui-tests.sh.

use super::*;

impl Editor {
    pub(super) fn dev(&mut self, c: String, sender: &ComponentSender<Self>) {
        if let Some(r) = c.strip_prefix("check:") {
            self.problems_dev(r);
            return;
        }
        if self.dev_spelling(&c) || self.dev_reports(&c) || self.dev_chars(&c) || self.dev_code(&c)
        {
            return;
        }
        if self.prefs_dev(&c) || self.snippets_dev(&c) {
            return;
        }
        if self.pv_dev(&c, sender) {
            return;
        }
        if self.dev9(&c, sender) {
            return;
        }
        if let Some(x) = c.strip_prefix("x:") {
            self.ext_dev(x);
        } else if let Some(x) = c.strip_prefix("mo:") {
            self.mo_dev(x);
        } else if let Some(p) = c.strip_prefix("delete=") {
            self.delete_files(p.split('|').map(str::to_string).collect());
        } else if let Some(p) = c.strip_prefix("select-files=") {
            self.w.files.unselect_all();
            for f in p.split('|') {
                if let Some((row, _)) = self.file_rows.get(f) {
                    self.w.files.select_row(Some(row));
                }
            }
        } else if c == "merge-selected" {
            let mut paths = self.selected_paths();
            editor_tools::sort_by_reading_order(&self.book.borrow(), &mut paths);
            sender.input(Input::MergeFiles(paths));
        } else if let Some(r) = c.strip_prefix("bulk=") {
            let (prefix, start) = r.split_once('|').unwrap_or((r, "1"));
            sender.input(Input::BulkRename(
                prefix.to_string(),
                start.parse().unwrap_or(1),
            ));
        } else if let Some(r) = c.strip_prefix("newfile=") {
            let mut it = r.split('|');
            let (n, t, l) = (
                it.next().unwrap_or(""),
                it.next().unwrap_or("blank"),
                it.next(),
            );
            sender.input(Input::NewFile(n.into(), t.into(), l == Some("link")));
        } else if let Some(r) = c.strip_prefix("linkcss") {
            let css = bookx_editing::markup::css_files(&self.book.borrow().archive);
            sender.input(Input::LinkStyles(css, r == "=remove"));
        } else if let Some(d) = c.strip_prefix("export=") {
            sender.input(Input::ExportTo(d.into()));
        } else if let Some(r) = c.strip_prefix("tocgen=") {
            use bookx_editing::toc_gen::{TocMode, TocOptions, TocSource};
            let mut it = r.split('|');
            let source = match it.next() {
                Some("links") => TocSource::Links,
                Some("files") => TocSource::Files,
                _ => TocSource::Headings,
            };
            let max_level = it.next().and_then(|l| l.parse().ok()).unwrap_or(3);
            let mode = if it.next() == Some("append") {
                TocMode::Append
            } else {
                TocMode::Replace
            };
            let links_file = it.next().map(str::to_string);
            sender.input(Input::GenerateToc(TocOptions {
                source,
                max_level,
                mode,
                links_file,
            }));
        } else if let Some(i) = c.strip_prefix("toc-go=") {
            sender.input(Input::TocGo(i.parse().unwrap_or(0)));
        } else if let Some(i) = c.strip_prefix("toc-select=") {
            sender.input(Input::TocSelect(i.parse().unwrap_or(0)));
        } else if let Some(n) = c.strip_prefix("toc-row=") {
            // a click on a row of the Contents list: the selection of the virtual list drives the editor
            self.w.toc_sel.set_selected(n.parse().unwrap_or(0));
        } else if c == "toc-rows" {
            // the Contents list is virtual: entries in the model against the row widgets that exist
            let alive = self
                .ext
                .structure
                .toc_entries
                .borrow()
                .iter()
                .filter(|(i, e)| i.upgrade().is_some() && e.upgrade().is_some())
                .count();
            eprintln!(
                "editor_toc_rows: entries={} model={} widgets={} selected={} hint={}",
                self.toc.len(),
                self.w.toc_store.n_items(),
                alive,
                self.w.toc_sel.selected() as i64,
                self.ext
                    .structure
                    .toc_hint
                    .borrow()
                    .as_ref()
                    .is_some_and(|h| h.is_visible()),
            );
        } else if let Some(a) = c.strip_prefix("longline=") {
            self.long_line_choose(a == "reformat");
        } else if c == "syntax-state" {
            // whether the open file is coloured: GtkSourceView language, its switch and the plain mark
            let sb = self.current.as_ref().and_then(|p| self.buffers.get(p));
            let b: gtk::TextBuffer =
                sb.map_or_else(|| self.w.view.buffer(), |b| b.clone().upcast());
            eprintln!(
                "editor_syntax: file={} page={} chars={} language={} highlight={} plain={}",
                self.current.clone().unwrap_or_default(),
                self.w.center.visible_child_name().unwrap_or_default(),
                b.char_count(),
                sb.and_then(|b| b.language()).is_some(),
                sb.is_some_and(|b| b.is_highlight_syntax()),
                sb.is_some_and(codeview::is_plain),
            );
        } else if c == "toc-dump" {
            eprintln!(
                "editor_toc: entries={} dirty={} selected={:?} titles={}",
                self.toc.len(),
                self.toc_dirty,
                self.toc_selected,
                self.toc
                    .iter()
                    .map(|t| format!("{}:{}", t.depth, t.title))
                    .collect::<Vec<_>>()
                    .join("/")
            );
        } else if let Some(n) = c.strip_prefix("goto=") {
            self.go_to_line(n);
        } else if c == "mark" {
            self.mark_selection();
        } else if let Some(n) = c.strip_prefix("cursor=") {
            let buf = self.w.view.buffer();
            let it = buf.iter_at_offset(n.parse().unwrap_or(0));
            buf.place_cursor(&it);
        } else if let Some(r) = c.strip_prefix("select=") {
            let buf = self.w.view.buffer();
            let (a, b) = r.split_once('-').unwrap_or((r, r));
            let (a, b) = (
                buf.iter_at_offset(a.parse().unwrap_or(0)),
                buf.iter_at_offset(b.parse().unwrap_or(0)),
            );
            buf.select_range(&a, &b);
        } else if let Some(r) = c.strip_prefix("find-in-text=") {
            // puts the cursor at the first occurrence of the text (or selects it with `|sel`)
            let (needle, sel) = r
                .strip_suffix("|sel")
                .map(|n| (n, true))
                .unwrap_or((r, false));
            let buf = self.w.view.buffer();
            let text = buffer_text(&buf);
            if let Some(b) = text.find(needle) {
                let a = super::search::byte_to_char_idx(&text, b) as i32;
                let e = a + needle.chars().count() as i32;
                let (ia, ie) = (
                    buf.iter_at_offset(a),
                    buf.iter_at_offset(if sel { e } else { a }),
                );
                buf.select_range(&ia, &ie);
            }
        } else if let Some(t) = c.strip_prefix("tag=") {
            let (id, arg) = t
                .split_once('|')
                .map(|(a, b)| (a, Some(b.to_string())))
                .unwrap_or((t, None));
            match super::tags::TagAction::from_id(id) {
                Some(a) => self.tag_action(a, arg),
                None => self.toast(&format!("Unknown tag action {id}")),
            }
        } else if let Some(p) = c.strip_prefix("cat=") {
            self.flush();
            let text = self.book.borrow().archive.get_string(p).unwrap_or_default();
            eprintln!("editor_cat: {}", text.replace('\n', "⏎"));
        } else if c == "cps" {
            let names: Vec<String> = self
                .checkpoints
                .list()
                .into_iter()
                .map(|c| c.name)
                .collect();
            eprintln!("editor_cps: {}", names.join("/"));
        } else if let Some(p) = c.strip_prefix("addpaths=") {
            sender.input(Input::AddPaths(p.split('|').map(Into::into).collect()));
        } else if let Some(t) = c.strip_prefix("title=") {
            let mut md = self.book.borrow().metadata().clone();
            md.title = t.to_string();
            sender.input(Input::ApplyMetadata(md, CoverChange::Keep));
        } else if let Some(t) = c.strip_prefix("insert=") {
            self.w
                .view
                .buffer()
                .insert_at_cursor(&t.replace("\\n", "\n"));
        } else if let Some(n) = c.strip_prefix("has=") {
            let text = buffer_text(&self.w.view.buffer());
            eprintln!("editor_has: {n} = {}", text.matches(n).count());
        } else if let Some(d) = c.strip_prefix("move=") {
            self.move_file(d == "up");
        } else if c == "tabs" {
            eprintln!(
                "editor_tabs: n={} selected={} paths={}",
                self.w.tabs.n_pages(),
                self.current.clone().unwrap_or_default(),
                self.tab_paths().join(" ")
            );
        } else if c == "tab-close" {
            self.close_current_tab();
        } else if let Some(d) = c.strip_prefix("step=") {
            self.step_file(d.parse().unwrap_or(1));
        } else if let Some(q) = c.strip_prefix("quickopen=") {
            let r = self.quick_open_results(q);
            eprintln!(
                "editor_quickopen: {q} -> {}",
                r.iter().take(3).cloned().collect::<Vec<_>>().join(" ")
            );
            if let Some(f) = r.first() {
                self.select_file(&f.clone());
            }
        } else if c == "cmdpalette" {
            self.show_palette();
        } else if c == "quickopen-popup" {
            self.quick_open();
        } else if c == "buf" {
            let buf = self.w.view.buffer();
            eprintln!("editor_buf: {}", buffer_text(&buf).replace('\n', "⏎"));
        } else if c == "cur" {
            let buf = self.w.view.buffer();
            let it = buf.iter_at_mark(&buf.get_insert());
            let sel = buf
                .selection_bounds()
                .map(|(a, b)| buffer_text_range(&buf, &a, &b))
                .unwrap_or_default();
            eprintln!(
                "editor_cur: file={} line={} col={} sel={:?} marks={}",
                self.current.clone().unwrap_or_default(),
                it.line() + 1,
                it.line_offset() + 1,
                sel,
                self.marks.values().map(|v| v.len()).sum::<usize>()
            );
        } else if c == "view" {
            let v = &self.w.view;
            let b = v.buffer().downcast::<sv::Buffer>().expect("source buffer");
            eprintln!(
                "editor_view: file={} lang={} scheme={} line_numbers={} current_line={} brackets={} auto_indent={} indent_tab={} smart_home={} margin={} whitespace={} minimap={} fallback_tags={}",
                self.current.clone().unwrap_or_default(),
                b.language().map(|l| l.id().to_string()).unwrap_or_else(|| "none".into()),
                codeview::scheme_id(&b),
                v.shows_line_numbers(),
                v.is_highlight_current_line(),
                b.is_highlight_matching_brackets(),
                v.is_auto_indent(),
                v.is_indent_on_tab(),
                v.smart_home_end() == sv::SmartHomeEndType::Before,
                v.shows_right_margin(),
                codeview::whitespace_on(v),
                self.w.map.is_visible(),
                !self.tag_segments("syn-tag").is_empty()
            );
        } else if let Some(m) = c.strip_prefix("scheme=") {
            // `scheme=dark|light|eink` restyles the open buffer as the app would
            let buf = self.buffer_for(&self.current.clone().unwrap_or_default());
            let (dark, eink) = (m == "dark", m == "eink");
            codeview::apply_scheme(&buf, dark, eink);
            eprintln!("editor_scheme: {m} -> {}", codeview::scheme_id(&buf));
        } else if let Some(o) = c.strip_prefix("view-opt=") {
            let (name, on) = o.split_once(':').unwrap_or((o, "on"));
            let opt = if name == "minimap" {
                live::ViewOpt::Minimap
            } else {
                live::ViewOpt::Whitespace
            };
            sender.input(Input::ViewOption(opt, on == "on"));
        } else if c == "live" {
            self.run_live();
        } else if c == "marks" {
            self.run_live();
            let m = self.gutter_marks.borrow();
            let lines: Vec<String> = m
                .lines()
                .map(|(l, i)| {
                    format!(
                        "{}{}{}{}",
                        l + 1,
                        match i.change {
                            Some(crate::editor_marks::Change::Added) => "+",
                            Some(crate::editor_marks::Change::Modified) => "~",
                            Some(crate::editor_marks::Change::Removed) => "-",
                            None => "",
                        },
                        match i.severity {
                            Some(Severity::Error) => "E",
                            Some(Severity::Warning) => "W",
                            Some(Severity::Info) => "I",
                            None => "",
                        },
                        if i.misspelled > 0 {
                            format!("s{}", i.misspelled)
                        } else {
                            String::new()
                        }
                    )
                })
                .collect();
            eprintln!(
                "editor_marks: changed={} findings={} misspelled={} lines={}",
                m.changed_count(),
                m.finding_count(),
                m.misspelled_total(),
                lines.join(",")
            );
        } else if let Some(l) = c.strip_prefix("tip=") {
            let line: u32 = l.parse().unwrap_or(1);
            eprintln!(
                "editor_tip: {line} = {}",
                self.gutter_marks
                    .borrow()
                    .tooltip(line.saturating_sub(1))
                    .unwrap_or_default()
                    .replace('\n', " / ")
            );
        } else if c == "spots" {
            self.run_live();
            let sh = self.spell.borrow();
            eprintln!(
                "editor_spots: ready={} note={:?} n={} words={} underlined={}",
                sh.state.as_ref().is_some_and(|s| s.is_ready()),
                sh.state.as_ref().and_then(|s| s.note()).unwrap_or(""),
                sh.spots.len(),
                sh.spots
                    .iter()
                    .map(|s| s.word.clone())
                    .collect::<Vec<_>>()
                    .join(" "),
                self.tag_segments(codeview::TAG_SPELL)
            );
        } else if let Some(a) = c.strip_prefix("spell-act=") {
            // `spell-act=replace|word|with`, `spell-act=ignore|word`, `spell-act=add|word`
            let mut it = a.split('|');
            let (kind, word) = (it.next().unwrap_or(""), it.next().unwrap_or(""));
            self.run_live();
            let spot = self
                .spell
                .borrow()
                .spots
                .iter()
                .find(|s| s.word == word)
                .cloned();
            self.spell.borrow_mut().target = spot;
            self.spell_act(match kind {
                "replace" => live::SpellAct::Replace(it.next().unwrap_or("").to_string()),
                "add" => live::SpellAct::Add,
                _ => live::SpellAct::Ignore,
            });
        } else if let Some(w) = c.strip_prefix("spell-menu=") {
            // the right-click entries for the first misspelled `w`
            self.run_live();
            let text = buffer_text(&self.w.view.buffer());
            let at = text.find(w).map(|i| i + 1).unwrap_or(0);
            let menu = gio::Menu::new();
            live::build_spell_menu(&menu, &self.spell, &text, at);
            let mut labels = Vec::new();
            for i in 0..menu.n_items() {
                if let Some(sec) = menu.item_link(i, "section").and_downcast::<gio::Menu>() {
                    for j in 0..sec.n_items() {
                        let l = sec
                            .item_attribute_value(j, "label", None)
                            .and_then(|v| v.str().map(str::to_string))
                            .unwrap_or_default();
                        match sec.item_link(j, "submenu").and_downcast::<gio::Menu>() { Some(sub) => {
                            labels.push(format!(
                                "{l}[{}]",
                                (0..sub.n_items())
                                    .filter_map(|k| sub
                                        .item_attribute_value(k, "label", None)
                                        .and_then(|v| v.str().map(str::to_string)))
                                    .collect::<Vec<_>>()
                                    .join(",")
                            ));
                        } _ => {
                            labels.push(l);
                        }}
                    }
                }
            }
            eprintln!("editor_spell_menu: {w} -> {}", labels.join(" | "));
        } else if c == "complete-show" {
            // really opens the popup at the caret (the provider is asked by GtkSourceView)
            self.w.view.grab_focus();
            self.w.view.buffer().insert_at_cursor("<di");
            self.w.view.completion().show();
        } else if c == "complete-last" {
            let d = self.comp.borrow();
            eprintln!(
                "editor_popup: calls={} last={}",
                d.populated.0, d.populated.1
            );
        } else if c == "complete-waits" {
            // the popup answered every request only after the debounce (see `completion::POPUP_DELAY_MS`)
            let w = self.comp.borrow().waits;
            eprintln!(
                "editor_popup_waits: answers={} shortest_ms={} debounced={}",
                w.answers,
                w.shortest_us.map_or(-1, |u| u / 1000),
                w.debounced()
            );
        } else if c == "pair" {
            self.update_pair_highlight();
            eprintln!("editor_pair: {}", self.tag_segments(codeview::TAG_PAIR));
        } else if c == "hidden" {
            self.run_live();
            eprintln!(
                "editor_hidden: {}",
                self.tag_segments(codeview::TAG_HIDDEN)
                    .replace('\u{a0}', "<nbsp>")
                    .replace('\u{200b}', "<zwsp>")
            );
        } else if let Some(q) = c.strip_prefix("search:") {
            self.dev_search(q);
        } else if let Some(q) = c.strip_prefix("find-set=") {
            self.w.find_rev.set_reveal_child(true);
            self.w.find_entry.set_text(q);
        } else if let Some(o) = c.strip_prefix("find-opt=") {
            match o {
                "regex" => self
                    .w
                    .sx
                    .mode
                    .set_selected(crate::editor_search_modes::Mode::Regex.index()),
                "noregex" => self
                    .w
                    .sx
                    .mode
                    .set_selected(crate::editor_search_modes::Mode::Normal.index()),
                "case" => self.w.case_btn.set_active(true),
                "dotall" => self.w.dotall_btn.set_active(true),
                _ => {}
            }
        } else if c == "find-status" {
            let ctx = self.search_context();
            eprintln!(
                "editor_find: status={:?} highlight={} count={} error={}",
                self.w.find_status.label(),
                ctx.as_ref().is_some_and(|c| c.is_highlight()),
                ctx.as_ref().map(|c| c.occurrences_count()).unwrap_or(-9),
                ctx.as_ref().is_some_and(|c| c.regex_error().is_some())
            );
        } else if let Some(t) = c.strip_prefix("complete=") {
            // text before the caret, `⏎` for a line break; prints what the popup would offer
            let before = t.replace('⏎', "\n");
            let d = self.comp.borrow();
            let items = completion::proposals(&d, &before);
            eprintln!(
                "editor_complete: n={} {}",
                items.len(),
                items
                    .iter()
                    .take(12)
                    .map(|i| {
                        format!(
                            "{}[{}:{}]{}",
                            i.label,
                            i.kind,
                            i.replace_len,
                            i.detail
                                .as_ref()
                                .map(|d| format!("«{d}»"))
                                .unwrap_or_default()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            );
        } else if let Some(t) = c.strip_prefix("snippet=") {
            // types the trigger at the caret and presses Tab
            self.w.view.buffer().insert_at_cursor(t);
            let done = completion::expand_at_caret(&self.w.view, &self.comp);
            eprintln!("editor_snippet: expanded={done}");
        } else if let Some(r) = c.strip_prefix("snippet-add=") {
            // `name|trigger|applies|body` (`\n` is a line break)
            let p: Vec<&str> = r.splitn(4, '|').collect();
            if let [name, trigger, applies, body] = p[..] {
                let mut user = crate::editor_snippets::load();
                user.push(crate::editor_snippets::Snippet::new(
                    name,
                    trigger,
                    &body.replace("\\n", "\n"),
                    applies,
                ));
                let _ = crate::editor_snippets::save(&user);
                sender.input(Input::SnippetsChanged(user));
            }
        } else if c == "snippets" {
            let d = self.comp.borrow();
            eprintln!(
                "editor_snippets: n={} triggers={}",
                d.snippets.len(),
                d.snippets
                    .iter()
                    .map(|s| s.trigger.clone())
                    .collect::<Vec<_>>()
                    .join(" ")
            );
        } else if c == "snippets-dialog" {
            self.manage_snippets();
        } else if let Some(r) = c.strip_prefix("rename=") {
            if let Some((old, new)) = r.split_once('|') {
                self.rename_file(old, new);
            }
        } else if let Some(p) = c.strip_prefix("open=") {
            self.select_file(p);
        } else if let Some(t) = c.strip_prefix("tool=") {
            match Tool::from_id(t) {
                Some(t) => self.run_tool(t),
                None => match t.strip_prefix("plugin:") {
                    Some(id) => self.run_plugin_tool(id),
                    None => self.toast(&format!("Unknown tool {t}")),
                },
            }
        } else if c == "compare=last" {
            if let Some(id) = self.checkpoints.list().last().map(|c| c.id) {
                self.compare_checkpoint(id);
            }
        } else if let Some(paths) = c.strip_prefix("delete-ask=") {
            // `delete-ask=<path>|<path>` runs the delete command of the file list without pressing the alert:
            // `editor_delete: confirm=yes|no alert=<heading>|<body>` (`-` when deleted at once)
            self.ext.last_alert.borrow_mut().clear();
            self.ask_delete_files(paths.split('|').map(String::from).collect());
            let a = self.ext.last_alert.borrow().clone();
            eprintln!(
                "editor_delete: confirm={} alert={}",
                if a.is_empty() { "no" } else { "yes" },
                if a.is_empty() { "-" } else { &a }
            );
        } else if c == "toast-info" {
            // `editor_toast: text="…" button=<label|-> checkpoints=N` for the last toast
            eprintln!(
                "editor_toast: text={:?} button={} checkpoints={}",
                self.last_toast.borrow(),
                self.last_press
                    .borrow()
                    .as_ref()
                    .map_or("-".to_string(), |(l, _)| l.clone()),
                self.checkpoints.len()
            );
        } else if c == "toast-action" {
            // presses the button of the last toast: `editor_toast_action: pressed=<label|none>`
            let press = self.last_press.borrow().clone();
            match press {
                Some((label, f)) => {
                    f();
                    eprintln!("editor_toast_action: pressed={label}");
                }
                None => eprintln!("editor_toast_action: pressed=none"),
            }
        } else if let Some(i) = c.strip_prefix("restore=") {
            // `restore=last` restores the newest checkpoint (the state before the last tool)
            let list = self.checkpoints.list();
            let id = if i == "last" {
                list.last().map(|c| c.id)
            } else {
                i.parse().ok()
            };
            if let Some(id) = id {
                self.restore_checkpoint(id);
            }
        } else if let Some(r) = c.strip_prefix("meta-apply=") {
            // `meta-apply=publisher:Editor Press` applies the form of the Metadata tab (the same
            // path as the Apply button), for the provenance test: fields `publisher`, `title`, `series`
            let mut md = self.book.borrow().metadata().clone();
            for part in r.split('|') {
                match part.split_once(':') {
                    Some(("publisher", v)) => md.publisher = v.to_string(),
                    Some(("title", v)) => md.title = v.to_string(),
                    Some(("series", v)) => md.series = v.to_string(),
                    _ => {}
                }
            }
            sender.input(Input::ApplyMetadata(md, CoverChange::Keep));
        } else if let Some(n) = c.strip_prefix("addfile=") {
            // `addfile=<name>` puts a stray file next to the package (for tool tests)
            let base = self.book.borrow().package.base_dir().to_string();
            self.book
                .borrow_mut()
                .archive
                .set(&join_path(&base, n), b"p { margin: 0 }".to_vec());
            self.structure_dirty = true;
        } else {
            match c.as_str() {
                "find" => {
                    self.w.find_rev.set_reveal_child(true);
                    self.w.find_entry.set_text("the");
                    self.w.replace_entry.set_text("THE");
                    self.find_all();
                }
                "check" => self.run_check(),
                "close-results" => sender.input(Input::CloseResults),
                "report" => self.show_report(),
                "spelling" => self.run_spelling(),
                "split" => sender.input(Input::SplitHeadings(1)),
                "merge" => sender.input(Input::MergeNext),
                "addheading" => {
                    // `addheading` appends a second chapter heading to the open file (for tests)
                    if let Some(p) = self.current.clone() {
                        let buf = self.buffer_for(&p);
                        let mut end = buf.end_iter();
                        let at = buf.text(&buf.start_iter(), &end, false).rfind("</body>");
                        if let Some(i) = at {
                            end = buf.iter_at_offset(
                                buf.text(&buf.start_iter(), &buf.end_iter(), false)[..i]
                                    .chars()
                                    .count() as i32,
                            );
                        }
                        buf.insert(&mut end, "<h1>Second Part</h1><p>More text here.</p>");
                    }
                }
                "colors-original" | "colors-theme" | "colors-custom" => {
                    let mut p = self.shared.cfg.borrow().reader.clone();
                    p.colors = match c.as_str() {
                        "colors-original" => crate::config::ColorMode::Original,
                        "colors-custom" => crate::config::ColorMode::Custom,
                        _ => crate::config::ColorMode::Theme,
                    };
                    sender.input(Input::PreviewColors(p));
                }
                "palette" => eprintln!("editor_palette: {}", self.w.preview.dev_palette()),
                "checkpoint" => {
                    let n = self.checkpoints.len() + 1;
                    self.create_checkpoint(&format!("Checkpoint {n}"));
                }
                "dump" => {
                    let b = self.book.borrow();
                    eprintln!(
                        "editor: files={} epub3={} checkpoints={} results={} problems={} toast={:?}",
                        b.file_list().len(),
                        b.package.is_epub3(),
                        self.checkpoints.len(),
                        self.results.len(),
                        self.problem_count(),
                        self.last_toast.borrow()
                    );
                    eprintln!("editor_files: {}", b.file_list().join(" "));
                }
                "toc" => {
                    self.w.sidebar_stack.set_visible_child_name("toc");
                }
                "meta" => {
                    self.w.sidebar_stack.set_visible_child_name("meta");
                }
                "gentoc" => {
                    self.w.sidebar_stack.set_visible_child_name("toc");
                    self.toc_op(TocOp::Generate);
                }
                "first-image" => {
                    let img = self
                        .book
                        .borrow()
                        .file_list()
                        .into_iter()
                        .find(|f| is_image_path(f));
                    if let Some(i) = img {
                        self.select_file(&i);
                    }
                }
                "save" => {
                    self.save();
                }
                _ => {}
            }
        }
    }
}
