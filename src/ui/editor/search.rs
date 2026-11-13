//! Find and replace, search scopes, marked text and "go to line".

use super::*;
use crate::editor_search::{self, Scope};
use crate::editor_search_modes::{Engine, Mode, RunCtx, Settings};
use crate::i18n::ntr;
use crate::i18n::{tr, trf};

type Ranges = Option<Vec<(usize, usize)>>;

impl Editor {
    // ---- find & replace -------------------------------------------------

    /// What the search bar currently asks for.
    pub(super) fn settings(&self) -> Settings {
        let mode = Mode::from_index(self.w.sx.mode.selected());
        Settings {
            find: self.w.find_entry.text().to_string(),
            replace: self.w.replace_entry.text().to_string(),
            mode,
            case_sensitive: self.w.case_btn.is_active(),
            dot_all: self.w.dotall_btn.is_active(),
            whole_word: self.w.sx.word_btn.is_active() && mode != Mode::Tag,
            ignore_markup: self.w.sx.markup_btn.is_active()
                && matches!(mode, Mode::Normal | Mode::Regex),
        }
    }

    pub(super) fn pattern(&self) -> Option<Engine> {
        let st = self.settings();
        if st.find.is_empty() {
            self.w.find_status.set_label("");
            return None;
        }
        match Engine::new(&st) {
            Ok(p) => Some(p),
            Err(e) => {
                self.w.find_status.set_label(&e);
                None
            }
        }
    }

    pub(super) fn replacement(&self) -> String {
        self.w.replace_entry.text().to_string()
    }

    pub(super) fn scope(&self) -> Scope {
        Scope::from_index(self.w.scope.selected())
    }

    /// The files of the current scope, the open file first.
    pub(super) fn scope_targets(&mut self) -> Vec<String> {
        let all = self.book.borrow().file_list();
        let selected = self.selected_paths();
        let open = self.tab_paths();
        let mut v = editor_search::scope_files(
            self.scope(),
            self.current.as_deref(),
            &all,
            &selected,
            &open,
        );
        if let Some(c) = &self.current {
            if let Some(i) = v.iter().position(|f| f == c) {
                v.rotate_left(i);
            }
        }
        v
    }

    /// The marked byte ranges of `path` when the scope is "Marked text", else `None` (= everywhere).
    pub(super) fn ranges_for(&mut self, path: &str) -> Ranges {
        if self.scope() != Scope::MarkedText {
            return None;
        }
        let buf = self.buffer_for(path);
        let text = buffer_text(&buf);
        let mut v: Vec<(usize, usize)> = self
            .marks
            .get(path)
            .map(|ms| {
                ms.iter()
                    .filter(|(a, b)| !a.is_deleted() && !b.is_deleted())
                    .map(|(a, b)| {
                        let ia = buf.iter_at_mark(a).offset() as usize;
                        let ib = buf.iter_at_mark(b).offset() as usize;
                        (char_to_byte_idx(&text, ia), char_to_byte_idx(&text, ib))
                    })
                    .filter(|(a, b)| a < b)
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        Some(v)
    }

    /// Marks the selected text as a search region (Ctrl+Shift+M); with no selection the marks of the file are cleared.
    pub(super) fn mark_selection(&mut self) {
        let Some(path) = self.current.clone().filter(|p| is_text_path(p)) else {
            return;
        };
        let buf = self.buffer_for(&path);
        let Some((a, b)) = buf.selection_bounds() else {
            self.clear_marks(&path);
            self.toast(&tr("Marked text cleared"));
            return;
        };
        let ma = gtk::TextMark::new(None, true);
        let mb = gtk::TextMark::new(None, false);
        buf.add_mark(&ma, &a);
        buf.add_mark(&mb, &b);
        buf.apply_tag_by_name("marked-region", &a, &b);
        self.marks.entry(path).or_default().push((ma, mb));
        self.toast(&tr(
            "Text marked: choose the scope “Marked text” to search only there",
        ));
    }

    pub(super) fn clear_marks(&mut self, path: &str) {
        if let Some(ms) = self.marks.remove(path) {
            if let Some(buf) = self.buffers.get(path) {
                for (a, b) in ms {
                    if !a.is_deleted() {
                        buf.delete_mark(&a);
                    }
                    if !b.is_deleted() {
                        buf.delete_mark(&b);
                    }
                }
                let (s, e) = buf.bounds();
                buf.remove_tag_by_name("marked-region", &s, &e);
            }
        }
    }

    pub(super) fn text_of(&mut self, path: &str) -> String {
        let b = self.buffer_for(path);
        buffer_text(&b)
    }

    pub(super) fn select_range(&mut self, path: &str, start_byte: usize, end_byte: usize) {
        if self.current.as_deref() != Some(path) {
            self.select_file(path);
        }
        let buf = self.w.view.buffer();
        let text = buffer_text(&buf);
        let a = byte_to_char_idx(&text, start_byte) as i32;
        let b = byte_to_char_idx(&text, end_byte) as i32;
        let (ia, ib) = (buf.iter_at_offset(a), buf.iter_at_offset(b));
        buf.select_range(&ib, &ia);
        let mut ia = buf.iter_at_offset(a);
        self.w.view.scroll_to_iter(&mut ia, 0.1, true, 0.5, 0.4);
        self.w.view.grab_focus();
    }

    /// First match at or after `from`, honouring the marked ranges.
    fn next_match(re: &Engine, text: &str, from: usize, ranges: &Ranges) -> Option<(usize, usize)> {
        match ranges {
            None => re.find_at(text, from),
            Some(r) => re.find_all(text, Some(r)).into_iter().find(|m| m.0 >= from),
        }
    }

    pub(super) fn find_next(&mut self, forward: bool) {
        let Some(re) = self.pattern() else { return };
        self.remember_search();
        let files = self.scope_targets();
        if files.is_empty() {
            return;
        }
        if self.scope() == Scope::MarkedText
            && self.ranges_for(&files[0]).is_none_or(|r| r.is_empty())
        {
            self.w
                .find_status
                .set_label(&tr("Nothing is marked (select text, then Ctrl+Shift+M)"));
            return;
        }
        let cur_path = self.current.clone().unwrap_or_default();
        let buf = self.w.view.buffer();
        let cur_text = buffer_text(&buf);
        let (sel_s, sel_e) = buf
            .selection_bounds()
            .map(|(a, b)| (a.offset(), b.offset()))
            .unwrap_or_else(|| {
                let o = buf.iter_at_mark(&buf.get_insert()).offset();
                (o, o)
            });
        let after = char_to_byte_idx(&cur_text, if forward { sel_e } else { sel_s } as usize);

        if forward {
            // rest of the current file, then following files, then wrap to the start of current
            let ranges = self.ranges_for(&cur_path);
            if files.first() == Some(&cur_path) {
                if let Some(m) = Self::next_match(&re, &cur_text, after, &ranges) {
                    self.select_range(&cur_path, m.0, m.1);
                    self.update_find_count(&re);
                    return;
                }
            }
            for (i, f) in files.iter().skip(1).chain(files.iter().take(1)).enumerate() {
                let t = if *f == cur_path {
                    cur_text.clone()
                } else {
                    self.text_of(f)
                };
                let r = self.ranges_for(f);
                if let Some(m) = Self::next_match(&re, &t, 0, &r) {
                    self.select_range(f, m.0, m.1);
                    self.update_find_count(&re);
                    if i + 1 == files.len() && files.first() == Some(&cur_path) {
                        self.mark_wrapped();
                    }
                    return;
                }
            }
        } else {
            let ranges = self.ranges_for(&cur_path);
            if files.first() == Some(&cur_path) {
                if let Some(m) = re.find_before(&cur_text, after, ranges.as_deref()) {
                    self.select_range(&cur_path, m.0, m.1);
                    self.update_find_count(&re);
                    return;
                }
            }
            for (i, f) in files
                .iter()
                .skip(1)
                .rev()
                .chain(files.iter().take(1))
                .enumerate()
            {
                let t = if *f == cur_path {
                    cur_text.clone()
                } else {
                    self.text_of(f)
                };
                let r = self.ranges_for(f);
                if let Some(m) = re.find_before(&t, t.len(), r.as_deref()) {
                    self.select_range(f, m.0, m.1);
                    self.update_find_count(&re);
                    if i + 1 == files.len() && files.first() == Some(&cur_path) {
                        self.mark_wrapped();
                    }
                    return;
                }
            }
        }
        self.w.find_status.set_label(&tr("No matches"));
    }

    fn mark_wrapped(&self) {
        let l = self.w.find_status.label();
        self.w
            .find_status
            .set_label(&trf("{} · wrapped around", &[&l]));
    }

    pub(super) fn update_find_count(&mut self, re: &Engine) {
        let buf = self.w.view.buffer();
        let text = buffer_text(&buf);
        let ranges = self.current.clone().and_then(|c| self.ranges_for(&c));
        let all = re.find_all(&text, ranges.as_deref());
        let total = all.len();
        let cursor = buf.selection_bounds().map(|(a, _)| a.offset()).unwrap_or(0);
        let cb = char_to_byte_idx(&text, cursor as usize);
        let idx = all
            .iter()
            .position(|m| m.0 >= cb)
            .map(|i| i + 1)
            .unwrap_or(total);
        self.w
            .find_status
            .set_label(&trf("{} of {} in this file", &[&idx, &total]));
    }

    /// Replaces the selected match (when the selection is one) and moves on.
    pub(super) fn replace_one(&mut self, forward: bool) {
        let Some(re) = self.pattern() else { return };
        let buf = self.w.view.buffer();
        if let Some((a, b)) = buf.selection_bounds() {
            let text = buffer_text(&buf);
            let (sb, eb) = (
                char_to_byte_idx(&text, a.offset() as usize),
                char_to_byte_idx(&text, b.offset() as usize),
            );
            if let Some(rep) = re.replacement_for(&text, sb, eb, &self.replacement()) {
                buf.begin_user_action();
                let mut a = a;
                let mut b = b;
                buf.delete(&mut a, &mut b);
                buf.insert(&mut a, &rep);
                buf.end_user_action();
                if !forward {
                    // step back over the inserted text so "previous" does not find it again
                    let start = buf.iter_at_offset(a.offset() - rep.chars().count() as i32);
                    buf.place_cursor(&start);
                }
            }
        }
        self.find_next(forward);
    }

    /// Replaces in the scope. A checkpoint is taken first when anything will change. Function mode runs the
    /// chosen function over the files (in its file order) and shows what it printed afterwards.
    pub(super) fn replace_all(&mut self) {
        let Some(re) = self.pattern() else { return };
        self.remember_search();
        let rep = self.replacement();
        let mut files = self.scope_targets();
        let mut runner = None;
        if re.is_function() {
            match self.make_runner() {
                Ok(r) => {
                    let spine = self.book.borrow().spine_paths();
                    files = crate::editor_search_fn::order_files(&files, &spine, r.func.file_order);
                    runner = Some(r);
                }
                Err(e) => {
                    self.show_fn_result(None, Some(e));
                    return;
                }
            }
        }
        let mut plan: Vec<(String, String, usize)> = Vec::new();
        for f in files {
            let text = self.text_of(&f);
            let ranges = self.ranges_for(&f);
            let mut ctx = RunCtx {
                runner: runner.as_mut(),
                file: if self.scope() == Scope::MarkedText {
                    ""
                } else {
                    &f
                },
            };
            match re.replace_all(&text, &rep, ranges.as_deref(), &mut ctx) {
                Ok((new, n)) => {
                    if n > 0 && new != text {
                        plan.push((f, new, n));
                    }
                }
                Err(e) => {
                    let msg = format!("{}: {e}", file_name(&f));
                    if runner.is_some() {
                        self.show_fn_result(runner.as_ref(), Some(msg));
                    } else {
                        self.toast(&msg);
                    }
                    return;
                }
            }
        }
        let mut final_output = None;
        if let Some(r) = runner.as_mut() {
            match r.finish("") {
                Ok(f) => final_output = f,
                Err(e) => {
                    self.show_fn_result(Some(r), Some(e.to_string()));
                    return;
                }
            }
        }
        if plan.is_empty() {
            self.toast(&tr("Nothing to replace"));
            if let Some(r) = runner.as_ref() {
                self.finish_fn_run(r, final_output, 0, 0);
            }
            return;
        }
        self.create_checkpoint("Before: Replace All");
        let (mut total, changed_files) = (0, plan.len());
        for (f, new, n) in plan {
            let buf = self.buffer_for(&f);
            buf.begin_user_action();
            let (mut s, mut e) = buf.bounds();
            buf.delete(&mut s, &mut e);
            let mut s = buf.start_iter();
            buf.insert(&mut s, &new);
            buf.end_user_action();
            total += n;
        }
        self.toast(&trf(
            "Replaced {} in {}",
            &[
                &ntr("{} occurrence", "{} occurrences", total),
                &ntr("{} file", "{} files", changed_files),
            ],
        ));
        if let Some(r) = runner.as_ref() {
            self.finish_fn_run(r, final_output, total, changed_files);
        }
    }

    /// Counts the matches over the whole scope and says so in the bar (Count All).
    pub(super) fn count_all(&mut self) {
        let Some(re) = self.pattern() else { return };
        self.remember_search();
        let files = self.scope_targets();
        let (mut total, mut with) = (0, 0);
        let mut per_file = Vec::new();
        for f in files {
            let text = self.text_of(&f);
            let ranges = self.ranges_for(&f);
            let n = re.find_all(&text, ranges.as_deref()).len();
            if n > 0 {
                total += n;
                with += 1;
                per_file.push(format!("{}: {n}", file_name(&f)));
            }
        }
        self.w.find_status.set_label(&count_label(total, with));
        let tip = per_file.join("\n");
        self.w
            .find_status
            .set_tooltip_text(if tip.is_empty() { None } else { Some(&tip) });
    }

    pub(super) fn find_all(&mut self) {
        let Some(re) = self.pattern() else { return };
        self.remember_search();
        let files = self.scope_targets();
        let mut rows = Vec::new();
        let mut shown: Vec<(String, String)> = Vec::new();
        'files: for f in files {
            let text = self.text_of(&f);
            let ranges = self.ranges_for(&f);
            let mut line_starts = vec![0usize];
            line_starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
            for m in re.find_all(&text, ranges.as_deref()) {
                let line = line_starts.partition_point(|s| *s <= m.0);
                let ls = line_starts[line - 1];
                let le = text[ls..].find('\n').map(|i| ls + i).unwrap_or(text.len());
                let snippet: String = text[ls..le].trim().chars().take(140).collect();
                rows.push(ResultRow {
                    file: f.clone(),
                    line: Some(line as u32),
                    range: Some((m.0, m.1)),
                });
                shown.push((format!("{}:{line}", file_name(&f)), snippet));
                if rows.len() >= 500 {
                    break 'files;
                }
            }
        }
        self.results = rows;
        self.result_kind = ResultKind::Search;
        self.show_results(
            &format!(
                "{} match{}",
                self.results.len(),
                if self.results.len() == 1 { "" } else { "es" }
            ),
            shown
                .into_iter()
                .map(|(t, s)| ("find-location-symbolic".to_string(), t, s, ""))
                .collect(),
        );
    }

    // ---- go to line -----------------------------------------------------

    pub(super) fn go_to_line(&mut self, text: &str) {
        let total = self.w.view.buffer().line_count().max(0) as usize;
        match editor_search::parse_line(text, total) {
            Some(l) => {
                let buf = self.w.view.buffer();
                if let Some(mut it) = buf.iter_at_line(l as i32) {
                    buf.place_cursor(&it);
                    self.w.view.scroll_to_iter(&mut it, 0.1, true, 0.5, 0.3);
                }
                self.w.view.grab_focus();
                self.update_cursor_status();
            }
            None => self.toast(&trf("Enter a line number from 1 to {}", &[&total])),
        }
    }

    /// A small popover with one entry, anchored on the code view.
    pub(super) fn prompt_line(&self) {
        if self.current.is_none() || self.w.center.visible_child_name().as_deref() != Some("code") {
            return;
        }
        let pop = gtk::Popover::new();
        pop.set_parent(&self.w.view);
        let entry = gtk::Entry::new();
        entry.set_placeholder_text(Some(&tr("Go to line")));
        entry.set_input_purpose(gtk::InputPurpose::Digits);
        pop.set_child(Some(&entry));
        let tx = self.sender.input_sender().clone();
        let p = pop.clone();
        entry.connect_activate(move |e| {
            tx.emit(Input::GoToLine(e.text().to_string()));
            p.popdown();
        });
        pop.connect_closed(|p| {
            let p = p.clone();
            glib::idle_add_local_once(move || p.unparent());
        });
        pop.popup();
        entry.grab_focus();
    }
}

/// "12 matches in 3 files" for Count All.
pub(super) fn count_label(total: usize, files: usize) -> String {
    trf(
        "{} in {}",
        &[
            &ntr("{} match", "{} matches", total),
            &ntr("{} file", "{} files", files),
        ],
    )
}

pub(super) fn byte_to_char_idx(s: &str, b: usize) -> usize {
    crate::ui::canvas::byte_to_char(s, b)
}
pub(super) fn char_to_byte_idx(s: &str, c: usize) -> usize {
    crate::ui::canvas::char_to_byte(s, c)
}
