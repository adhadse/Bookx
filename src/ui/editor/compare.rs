//! The compare view (CE-224..CE-227): two texts side by side with line numbers, only the differences with a
//! few lines of context, word marks inside changed lines, one scroll position for both sides; and the
//! checkpoints dialog (CE-220, CE-223).

use crate::i18n::ntr;
use crate::i18n::{tr, trf};
use std::cell::RefCell;

use bookx_editing::compare::{self, Row, RowKind, SideBySide};

use super::preview::PvMsg;
use super::*;
use crate::core::archive::Archive;

#[derive(Debug)]
pub enum CmpMsg {
    File(usize),
    Beautify(bool),
    Context(usize),
    Search(String),
    SearchNext(bool),
    Change(bool),
    /// Double click on a row: show that line in the editor.
    Goto(usize),
    Close,
}

struct Side {
    nums: gtk::TextView,
    text: gtk::TextView,
    scroll: gtk::ScrolledWindow,
    caption: gtk::Label,
}

impl Side {
    fn new(right: bool) -> Side {
        let mk = || {
            let v = gtk::TextView::new();
            v.set_editable(false);
            v.set_cursor_visible(false);
            v.set_monospace(true);
            v.set_wrap_mode(gtk::WrapMode::None);
            v
        };
        let nums = mk();
        nums.set_justification(gtk::Justification::Right);
        nums.set_can_focus(false);
        nums.set_can_target(false);
        nums.set_left_margin(6);
        nums.set_right_margin(8);
        nums.add_css_class("dim-label");
        crate::ui::audit::label(&nums, &tr("Line numbers"));
        let text = mk();
        text.set_left_margin(8);
        text.set_hexpand(true);
        crate::ui::audit::label(
            &text,
            &(if right {
                tr("Text on the right: the newer version")
            } else {
                tr("Text on the left: the older version")
            }),
        );
        let inner = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        inner.append(&nums);
        inner.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        inner.append(&text);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_child(Some(&inner));
        scroll.set_hexpand(true);
        scroll.set_vexpand(true);
        let caption = gtk::Label::new(None);
        caption.set_xalign(0.0);
        caption.add_css_class("caption-heading");
        caption.set_margin_all(4);
        caption.set_ellipsize(pango::EllipsizeMode::Middle);
        Side {
            nums,
            text,
            scroll,
            caption,
        }
    }
}

/// What is being compared and the settings of the view.
struct CmpData {
    left: Archive,
    right: Archive,
    left_name: String,
    right_name: String,
    /// Double click goes to the line of the left text (it is the book in the editor).
    goto_left: bool,
    files: Vec<String>,
    cur: usize,
    beautify: bool,
    context: usize,
    sbs: SideBySide,
    hits: Vec<usize>,
    hit_at: usize,
    change_at: Option<usize>,
}

pub(super) struct CmpUi {
    pub root: gtk::Box,
    title: gtk::Label,
    files: gtk::DropDown,
    stats: gtk::Label,
    beautify: gtk::ToggleButton,
    context: gtk::SpinButton,
    search: gtk::SearchEntry,
    left: Side,
    right: Side,
    data: RefCell<Option<CmpData>>,
    /// Ignore widget signals while the view is being set up.
    quiet: Cell<bool>,
}

fn rgba(r: f32, g: f32, b: f32, a: f32) -> gdk::RGBA {
    gdk::RGBA::new(r, g, b, a)
}

fn make_tags(buf: &gtk::TextBuffer, eink: bool) {
    let table = buf.tag_table();
    let tag = |name: &str, bg: Option<gdk::RGBA>| {
        let t = gtk::TextTag::new(Some(name));
        if let Some(bg) = bg {
            t.set_background_rgba(Some(&bg));
        }
        table.add(&t);
        t
    };
    if eink {
        tag("added", Some(rgba(0.0, 0.0, 0.0, 0.10))).set_weight(700);
        let r = tag("removed", Some(rgba(0.0, 0.0, 0.0, 0.22)));
        r.set_strikethrough(true);
        tag("changed", Some(rgba(0.0, 0.0, 0.0, 0.10)));
        for n in ["mark_old", "mark_new"] {
            let m = tag(n, Some(rgba(0.0, 0.0, 0.0, 0.30)));
            m.set_weight(700);
            m.set_underline(pango::Underline::Single);
        }
    } else {
        tag("added", Some(rgba(0.18, 0.63, 0.26, 0.26)));
        tag("removed", Some(rgba(0.88, 0.25, 0.22, 0.26)));
        tag("changed", Some(rgba(0.22, 0.52, 0.95, 0.22)));
        tag("mark_old", Some(rgba(0.88, 0.25, 0.22, 0.55)));
        tag("mark_new", Some(rgba(0.18, 0.63, 0.26, 0.55)));
    }
    tag("empty", Some(rgba(0.5, 0.5, 0.5, 0.10)));
    let skip = tag("skip", Some(rgba(0.5, 0.5, 0.5, 0.12)));
    skip.set_style(pango::Style::Italic);
    skip.set_foreground_rgba(Some(&rgba(0.5, 0.5, 0.5, 1.0)));
    tag("hit", Some(rgba(0.96, 0.83, 0.0, 0.5)));
}

impl CmpUi {
    pub(super) fn new(sender: &ComponentSender<Editor>) -> CmpUi {
        let tx = sender.input_sender().clone();
        let send = move |m: CmpMsg| tx.emit(Input::Pv(PvMsg::Cmp(m)));
        let send = Rc::new(send);

        let close = gtk::Button::from_icon_name("go-previous-symbolic");
        close.set_tooltip_text(Some(&tr("Close the comparison and go back to the code")));
        crate::ui::audit::label(&close, &tr("Close the comparison"));
        let title = gtk::Label::new(None);
        title.set_xalign(0.0);
        title.set_hexpand(true);
        title.add_css_class("heading");
        title.set_ellipsize(pango::EllipsizeMode::End);
        let files = gtk::DropDown::from_strings(&[]);
        files.set_tooltip_text(Some(&tr("The file to show")));
        crate::ui::audit::label(&files, &tr("File to compare"));
        files.set_enable_search(true);
        let prev = gtk::Button::from_icon_name("go-up-symbolic");
        prev.set_tooltip_text(Some(&tr("Previous change")));
        crate::ui::audit::label(&prev, &tr("Previous change"));
        let next = gtk::Button::from_icon_name("go-down-symbolic");
        next.set_tooltip_text(Some(&tr("Next change")));
        crate::ui::audit::label(&next, &tr("Next change"));
        let context = gtk::SpinButton::with_range(0.0, 200.0, 1.0);
        context.set_value(3.0);
        context.set_tooltip_text(Some(&tr("Unchanged lines shown around each change")));
        crate::ui::audit::label(&context, &tr("Lines of context"));
        let beautify = gtk::ToggleButton::with_label(&tr("Beautify"));
        beautify.set_tooltip_text(Some(
            &tr("Pretty-print the markup before comparing, so formatting is ignored (line numbers then refer to the pretty-printed text)"),
        ));
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some(&tr("Search in the differences")));
        search.set_max_width_chars(22);
        crate::ui::audit::label(&search, &tr("Search in the differences"));
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        bar.add_css_class("toolbar");
        bar.set_margin_start(6);
        bar.set_margin_end(6);
        for w in [
            close.upcast_ref::<gtk::Widget>(),
            title.upcast_ref(),
            files.upcast_ref(),
            prev.upcast_ref(),
            next.upcast_ref(),
            context.upcast_ref(),
            beautify.upcast_ref(),
            search.upcast_ref(),
        ] {
            bar.append(w);
        }

        let left = Side::new(false);
        let right = Side::new(true);
        right
            .scroll
            .set_vadjustment(Some(&left.scroll.vadjustment()));
        right
            .scroll
            .set_vscrollbar_policy(gtk::PolicyType::External);
        let captions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        captions.set_homogeneous(true);
        captions.append(&left.caption);
        captions.append(&right.caption);
        let panes = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        panes.set_homogeneous(true);
        panes.set_vexpand(true);
        panes.append(&left.scroll);
        panes.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        panes.append(&right.scroll);
        let stats = gtk::Label::new(None);
        stats.set_xalign(0.0);
        stats.set_margin_all(4);
        stats.add_css_class("dim-label");

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&bar);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        root.append(&captions);
        root.append(&panes);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        root.append(&stats);

        let eink = crate::eink::is_active();
        for s in [&left, &right] {
            make_tags(&s.text.buffer(), eink);
        }

        let ui = CmpUi {
            root,
            title,
            files,
            stats,
            beautify,
            context,
            search,
            left,
            right,
            data: RefCell::new(None),
            quiet: Cell::new(false),
        };
        {
            let s = send.clone();
            close.connect_clicked(move |_| s(CmpMsg::Close));
            let s = send.clone();
            prev.connect_clicked(move |_| s(CmpMsg::Change(false)));
            let s = send.clone();
            next.connect_clicked(move |_| s(CmpMsg::Change(true)));
            let s = send.clone();
            ui.beautify
                .connect_toggled(move |b| s(CmpMsg::Beautify(b.is_active())));
            let s = send.clone();
            ui.context
                .connect_value_changed(move |c| s(CmpMsg::Context(c.value() as usize)));
            let s = send.clone();
            ui.search
                .connect_search_changed(move |e| s(CmpMsg::Search(e.text().to_string())));
            let s = send.clone();
            ui.search
                .connect_activate(move |_| s(CmpMsg::SearchNext(true)));
            let s = send.clone();
            ui.files
                .connect_selected_notify(move |d| s(CmpMsg::File(d.selected() as usize)));
            for side in [&ui.left, &ui.right] {
                let click = gtk::GestureClick::new();
                let view = side.text.clone();
                let s = send.clone();
                click.connect_pressed(move |_, n, x, y| {
                    if n != 2 {
                        return;
                    }
                    let (bx, by) = view.window_to_buffer_coords(
                        gtk::TextWindowType::Widget,
                        x as i32,
                        y as i32,
                    );
                    if let Some(it) = view.iter_at_location(bx, by) {
                        s(CmpMsg::Goto(it.line() as usize));
                    }
                });
                side.text.add_controller(click);
            }
        }
        ui
    }

    fn set_files(&self, names: &[String]) {
        self.quiet.set(true);
        let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        self.files.set_model(Some(&gtk::StringList::new(&refs)));
        self.files.set_selected(0);
        self.quiet.set(false);
    }

    /// Fills both sides with the rows of `sbs`.
    fn render(&self, sbs: &SideBySide) {
        let (mut lt, mut rt, mut ln, mut rn) =
            (String::new(), String::new(), String::new(), String::new());
        type Mark = (&'static str, i32, i32);
        let (mut lm, mut rm): (Vec<Mark>, Vec<Mark>) = (Vec::new(), Vec::new());
        let (mut lc, mut rc) = (0i32, 0i32);
        let n = sbs.rows.len();
        for (i, row) in sbs.rows.iter().enumerate() {
            let nl = if i + 1 < n { "\n" } else { "" };
            match row {
                Row::Skip(k) => {
                    let t = format!("⋯ {k} unchanged line{}", if *k == 1 { "" } else { "s" });
                    let len = t.chars().count() as i32;
                    lm.push(("skip", lc, lc + len));
                    rm.push(("skip", rc, rc + len));
                    lt.push_str(&t);
                    rt.push_str(&t);
                    lt.push_str(nl);
                    rt.push_str(nl);
                    lc += len + nl.len() as i32;
                    rc += len + nl.len() as i32;
                    ln.push_str(nl);
                    rn.push_str(nl);
                }
                Row::Line { kind, old, new } => {
                    let tag = match kind {
                        RowKind::Equal => None,
                        RowKind::Added => Some("added"),
                        RowKind::Removed => Some("removed"),
                        RowKind::Changed => Some("changed"),
                    };
                    for (cell, text, nums, marks, count, mark_tag, empty) in [
                        (
                            old,
                            &mut lt,
                            &mut ln,
                            &mut lm,
                            &mut lc,
                            "mark_old",
                            *kind == RowKind::Added,
                        ),
                        (
                            new,
                            &mut rt,
                            &mut rn,
                            &mut rm,
                            &mut rc,
                            "mark_new",
                            *kind == RowKind::Removed,
                        ),
                    ] {
                        match cell {
                            Some(c) => {
                                let len = c.text.chars().count() as i32;
                                if let Some(t) = tag {
                                    marks.push((t, *count, *count + len.max(1)));
                                }
                                for m in &c.marks {
                                    let a = c.text[..m.start].chars().count() as i32;
                                    let b = c.text[..m.end].chars().count() as i32;
                                    marks.push((mark_tag, *count + a, *count + b));
                                }
                                text.push_str(&c.text);
                                *count += len;
                                nums.push_str(&c.line.to_string());
                            }
                            None => {
                                if empty {
                                    marks.push(("empty", *count, *count + 1));
                                }
                            }
                        }
                        text.push_str(nl);
                        nums.push_str(nl);
                        *count += nl.len() as i32;
                    }
                }
            }
        }
        for (side, text, nums, marks) in [(&self.left, &lt, &ln, &lm), (&self.right, &rt, &rn, &rm)]
        {
            let buf = side.text.buffer();
            buf.set_text(text);
            side.nums.buffer().set_text(nums);
            for (tag, a, b) in marks {
                if let Some(t) = buf.tag_table().lookup(tag) {
                    let (ia, ib) = (buf.iter_at_offset(*a), buf.iter_at_offset(*b));
                    buf.apply_tag(&t, &ia, &ib);
                }
            }
        }
        // a comfortable width for the numbers: the digits of the largest one
        let digits = sbs
            .rows
            .iter()
            .filter_map(|r| match r {
                Row::Line { old, new, .. } => Some(
                    old.iter()
                        .chain(new.iter())
                        .map(|c| c.line.to_string().len())
                        .max()
                        .unwrap_or(0),
                ),
                _ => None,
            })
            .max()
            .unwrap_or(1);
        for s in [&self.left, &self.right] {
            s.nums.set_width_request(18 + 9 * digits as i32);
        }
    }

    fn mark_hits(&self, rows: &[usize], active: Option<usize>) {
        for side in [&self.left, &self.right] {
            let buf = side.text.buffer();
            let Some(tag) = buf.tag_table().lookup("hit") else {
                continue;
            };
            buf.remove_tag(&tag, &buf.start_iter(), &buf.end_iter());
            for r in rows {
                if let Some(mut a) = buf.iter_at_line(*r as i32) {
                    let mut b = a;
                    b.forward_to_line_end();
                    buf.apply_tag(&tag, &a, &b);
                    let _ = &mut a;
                }
            }
        }
        if let Some(r) = active.and_then(|a| rows.get(a)) {
            self.scroll_to_row(*r);
        }
    }

    fn scroll_to_row(&self, row: usize) {
        let buf = self.left.text.buffer();
        if let Some(mut it) = buf.iter_at_line(row as i32) {
            self.left.text.scroll_to_iter(&mut it, 0.0, true, 0.0, 0.25);
        }
    }
}

impl CmpData {
    fn path(&self) -> Option<&str> {
        self.files.get(self.cur).map(|s| s.as_str())
    }

    fn recompute(&mut self) {
        self.sbs = match self.path() {
            Some(p) => {
                compare::file_side_by_side(&self.left, &self.right, p, self.beautify, self.context)
                    .unwrap_or_default()
            }
            None => SideBySide::default(),
        };
        self.hits.clear();
        self.hit_at = 0;
        self.change_at = None;
    }
}

/// Text files that differ, for the file list.
fn differing_files(left: &Archive, right: &Archive, beautify: bool) -> Vec<String> {
    let d = compare::compare_archives(
        left,
        right,
        compare::CompareOptions {
            normalize_xml: beautify,
        },
    );
    let mut out: Vec<String> = d
        .changed
        .iter()
        .filter(|c| c.text)
        .map(|c| c.path.clone())
        .chain(d.added.iter().filter(|p| compare::is_text_path(p)).cloned())
        .chain(
            d.removed
                .iter()
                .filter(|p| compare::is_text_path(p))
                .cloned(),
        )
        .collect();
    out.sort();
    out
}

fn human_time(created: i64) -> String {
    chrono::DateTime::from_timestamp(created, 0)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%H:%M:%S")
                .to_string()
        })
        .unwrap_or_default()
}

/// The checkpoints dialog while it is open.
pub(super) struct CpDialog {
    dialog: adw::Dialog,
    list: gtk::ListBox,
    entry: adw::EntryRow,
}

impl CpDialog {
    pub(super) fn close(&self) {
        self.dialog.close();
    }
}

impl Editor {
    /// Shows the comparison of `left` (older) and `right` (newer) in the editor's centre.
    pub(super) fn cmp_open(
        &mut self,
        left: Archive,
        right: Archive,
        names: (&str, &str),
        goto_left: bool,
        select: Option<&str>,
    ) -> bool {
        let beautify = self.pv.cmp.beautify.is_active();
        let files = differing_files(&left, &right, beautify);
        if files.is_empty() {
            self.toast(&tr("No differences in the text files"));
            return false;
        }
        let ui = &self.pv.cmp;
        let cur = select
            .and_then(|s| files.iter().position(|f| f == s))
            .unwrap_or(0);
        let mut data = CmpData {
            left,
            right,
            left_name: names.0.to_string(),
            right_name: names.1.to_string(),
            goto_left,
            files: files.clone(),
            cur,
            beautify,
            context: ui.context.value() as usize,
            sbs: SideBySide::default(),
            hits: Vec::new(),
            hit_at: 0,
            change_at: None,
        };
        data.recompute();
        ui.title.set_text(&format!("{} ↔ {}", names.0, names.1));
        ui.left.caption.set_text(names.0);
        ui.right.caption.set_text(names.1);
        ui.set_files(&files);
        ui.quiet.set(true);
        ui.files.set_selected(cur as u32);
        ui.search.set_text("");
        ui.quiet.set(false);
        ui.render(&data.sbs);
        ui.stats.set_text(&stats_text(&data));
        *ui.data.borrow_mut() = Some(data);
        self.w.center.set_visible_child_name("compare");
        true
    }

    /// Called by the old checkpoint comparison after it listed the changed files.
    pub(super) fn pv_open_checkpoint_compare(&mut self, id: u64) {
        let Ok(old) = self.checkpoints.restore(id) else {
            return;
        };
        let name = self
            .checkpoints
            .list()
            .into_iter()
            .find(|c| c.id == id)
            .map(|c| c.name)
            .unwrap_or_default();
        let cur = self.current.clone();
        let now = self.book.borrow().archive.clone();
        self.cmp_open(old, now, (&name, "Current book"), false, cur.as_deref());
    }

    pub(super) fn compare_book_dialog(&mut self) {
        let dlg = gtk::FileDialog::builder()
            .title(tr("Compare with Another Book"))
            .modal(true)
            .build();
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(&tr("EPUB books")));
        filter.add_suffix("epub");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        dlg.set_filters(Some(&filters));
        let tx = self.sender.input_sender().clone();
        dlg.open(Some(&self.w.win), gio::Cancellable::NONE, move |res| {
            if let Some(p) = res.ok().and_then(|f| f.path()) {
                tx.emit(Input::Pv(PvMsg::CompareBookFile(p)));
            }
        });
    }

    /// CE-227: the book in the editor against another EPUB file.
    pub(super) fn compare_with_file(&mut self, path: &std::path::Path) {
        self.flush();
        let other = match crate::core::book::OpenBook::open(path) {
            Ok(b) => b,
            Err(e) => {
                self.toast(&trf("Could not open {}: {}", &[&path.display(), &e]));
                return;
            }
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let cur = self.current.clone();
        let mine = self.book.borrow().archive.clone();
        self.cmp_open(
            mine,
            other.archive,
            ("This book", &name),
            true,
            cur.as_deref(),
        );
    }

    pub(super) fn cmp_msg(&mut self, m: CmpMsg) {
        if matches!(m, CmpMsg::Close) {
            let name = if self.current.is_some() {
                "code"
            } else {
                "empty"
            };
            self.w.center.set_visible_child_name(name);
            return;
        }
        if self.pv.cmp.quiet.get() {
            return;
        }
        let goto = {
            let ui = &self.pv.cmp;
            let mut guard = ui.data.borrow_mut();
            let Some(d) = guard.as_mut() else { return };
            let mut goto: Option<(String, usize)> = None;
            match m {
                CmpMsg::File(i) => {
                    if i == d.cur || i >= d.files.len() {
                        return;
                    }
                    d.cur = i;
                    d.recompute();
                    ui.render(&d.sbs);
                    ui.search.set_text("");
                }
                CmpMsg::Beautify(on) => {
                    d.beautify = on;
                    let files = differing_files(&d.left, &d.right, on);
                    let keep = d.path().map(str::to_string);
                    d.files = files;
                    d.cur = keep
                        .and_then(|k| d.files.iter().position(|f| *f == k))
                        .unwrap_or(0);
                    d.recompute();
                    ui.set_files(&d.files.clone());
                    ui.quiet.set(true);
                    ui.files.set_selected(d.cur as u32);
                    ui.quiet.set(false);
                    ui.render(&d.sbs);
                }
                CmpMsg::Context(n) => {
                    if n == d.context {
                        return;
                    }
                    d.context = n;
                    d.recompute();
                    ui.render(&d.sbs);
                }
                CmpMsg::Search(q) => {
                    d.hits = d.sbs.search(&q, false);
                    d.hit_at = 0;
                    ui.mark_hits(&d.hits, if d.hits.is_empty() { None } else { Some(0) });
                }
                CmpMsg::SearchNext(fwd) => {
                    if !d.hits.is_empty() {
                        let n = d.hits.len();
                        d.hit_at = if fwd {
                            (d.hit_at + 1) % n
                        } else {
                            (d.hit_at + n - 1) % n
                        };
                        ui.scroll_to_row(d.hits[d.hit_at]);
                    }
                }
                CmpMsg::Change(fwd) => {
                    let starts = d.sbs.change_starts();
                    if !starts.is_empty() {
                        let n = starts.len();
                        let next = match d.change_at {
                            None => {
                                if fwd {
                                    0
                                } else {
                                    n - 1
                                }
                            }
                            Some(i) => {
                                if fwd {
                                    (i + 1) % n
                                } else {
                                    (i + n - 1) % n
                                }
                            }
                        };
                        d.change_at = Some(next);
                        ui.scroll_to_row(starts[next]);
                    }
                }
                CmpMsg::Goto(row) => {
                    if let (Some(Row::Line { old, new, .. }), Some(p)) =
                        (d.sbs.rows.get(row), d.path())
                    {
                        let (mine, other) = if d.goto_left { (old, new) } else { (new, old) };
                        if let Some(c) = mine.as_ref().or(other.as_ref()) {
                            goto = Some((p.to_string(), c.line));
                        }
                    }
                }
                CmpMsg::Close => {}
            }
            ui.stats.set_text(&stats_text(d));
            goto
        };
        if let Some((path, line)) = goto {
            if !self.book.borrow().archive.contains(&path) {
                self.toast(&trf("{} is not in the open book", &[&path]));
                return;
            }
            self.select_file(&path);
            self.go_to_line(&line.to_string());
        }
    }

    // ---- checkpoints dialog (CE-220, CE-223) -------------------------------------------------

    pub(super) fn checkpoints_dialog(&mut self) {
        if let Some(d) = self.pv.cp_dialog.borrow().as_ref() {
            d.dialog.present(Some(&self.w.win));
            return;
        }
        let dialog = crate::ui::dialog_toast::new_dialog();
        dialog.set_title(&tr("Checkpoints"));
        dialog.set_content_width(560);
        dialog.set_content_height(520);
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&adw::HeaderBar::new());
        let page = adw::PreferencesPage::new();
        let create = adw::PreferencesGroup::new();
        create.set_title(&tr("New Checkpoint"));
        create.set_description(Some(
            &tr("A checkpoint is a copy of the whole book you can compare with or go back to. One is taken automatically before every tool."),
        ));
        let entry = adw::EntryRow::new();
        entry.set_title(&tr("Name"));
        entry.set_show_apply_button(true);
        create.add(&entry);
        page.add(&create);
        let saved = adw::PreferencesGroup::new();
        saved.set_title(&tr("Saved States"));
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        crate::ui::audit::label(&list, &tr("Checkpoints"));
        let empty = adw::StatusPage::new();
        empty.set_title(&tr("No Checkpoints Yet"));
        empty.set_description(Some(&tr(
            "One is taken before every tool. Create one yourself above.",
        )));
        empty.set_icon_name(Some("document-open-recent-symbolic"));
        list.set_placeholder(Some(&empty));
        saved.add(&list);
        page.add(&saved);
        toolbar.set_content(Some(&page));
        dialog.set_child(Some(&toolbar));
        {
            let tx = self.sender.input_sender().clone();
            entry.connect_apply(move |e| {
                tx.emit(Input::Pv(PvMsg::CheckpointNew(e.text().to_string())));
                e.set_text("");
            });
            let tx = self.sender.input_sender().clone();
            dialog.connect_closed(move |_| tx.emit(Input::Pv(PvMsg::CheckpointsClosed)));
        }
        dialog.present(Some(&self.w.win));
        *self.pv.cp_dialog.borrow_mut() = Some(CpDialog {
            dialog,
            list,
            entry,
        });
        self.refresh_cp_dialog();
    }

    pub(super) fn refresh_cp_dialog(&self) {
        let guard = self.pv.cp_dialog.borrow();
        let Some(d) = guard.as_ref() else { return };
        let _ = &d.entry;
        while let Some(c) = d.list.first_child() {
            d.list.remove(&c);
        }
        let infos = self.visible_checkpoints();
        for c in infos.iter().rev() {
            let row = adw::ActionRow::new();
            row.set_title(&glib::markup_escape_text(&c.name));
            let base = trf(
                "{} · {} files · {}",
                &[
                    &human_time(c.created),
                    &c.file_count,
                    &human_size(c.bytes as u64),
                ],
            );
            row.set_subtitle(&glib::markup_escape_text(&cp_subtitle(
                &base,
                c.pinned,
                &tr("Kept"),
            )));
            let id = c.id;
            let compare = gtk::Button::with_label(&tr("Compare"));
            compare.set_valign(gtk::Align::Center);
            compare.add_css_class("flat");
            compare.set_tooltip_text(Some(&tr("Compare with the book")));
            crate::ui::audit::label(&compare, &format!("{}: {}", tr("Compare"), c.name));
            let tx = self.sender.input_sender().clone();
            compare.connect_clicked(move |_| tx.emit(Input::Pv(PvMsg::CompareCheckpoint(id))));

            let menu = gio::Menu::new();
            menu.append(
                Some(&tr("Revert the Book to This State")),
                Some("cp.revert"),
            );
            menu.append(Some(&tr("Rename")), Some("cp.rename"));
            menu.append(
                Some(&if c.pinned {
                    tr("Stop Keeping")
                } else {
                    tr("Keep")
                }),
                Some("cp.pin"),
            );
            let del = gio::Menu::new();
            del.append(Some(&tr("Delete")), Some("cp.delete"));
            menu.append_section(None, &del);
            let more = gtk::MenuButton::new();
            more.set_icon_name("view-more-symbolic");
            more.set_menu_model(Some(&menu));
            more.set_valign(gtk::Align::Center);
            more.add_css_class("flat");
            more.set_tooltip_text(Some(&tr("Checkpoint Actions")));
            crate::ui::audit::label(&more, &format!("{}: {}", tr("Checkpoint Actions"), c.name));

            let group = gio::SimpleActionGroup::new();
            let act = |name: &str, f: Box<dyn Fn()>| {
                let a = gio::SimpleAction::new(name, None);
                a.connect_activate(move |_, _| f());
                group.add_action(&a);
            };
            let tx = self.sender.input_sender().clone();
            let dialog = d.dialog.clone();
            act(
                "revert",
                Box::new(move || {
                    tx.emit(Input::RestoreCheckpoint(id));
                    dialog.close();
                }),
            );
            let tx = self.sender.input_sender().clone();
            let pinned = c.pinned;
            act(
                "pin",
                Box::new(move || tx.emit(Input::Pv(PvMsg::CheckpointPin(id, !pinned)))),
            );
            let tx = self.sender.input_sender().clone();
            act(
                "delete",
                Box::new(move || tx.emit(Input::Pv(PvMsg::CheckpointDelete(id)))),
            );
            let tx = self.sender.input_sender().clone();
            let name = c.name.clone();
            let parent = d.dialog.clone();
            act(
                "rename",
                Box::new(move || {
                    let e = gtk::Entry::new();
                    e.set_text(&name);
                    e.set_activates_default(true);
                    crate::ui::audit::label(&e, &tr("New name of the checkpoint"));
                    let a = adw::AlertDialog::new(Some(&tr("Rename Checkpoint")), None);
                    a.set_extra_child(Some(&e));
                    a.add_responses(&[("cancel", &tr("Cancel")), ("ok", &tr("Rename"))]);
                    a.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
                    a.set_default_response(Some("ok"));
                    a.set_close_response("cancel");
                    let tx = tx.clone();
                    a.connect_response(None, move |_, r| {
                        if r == "ok" {
                            tx.emit(Input::Pv(PvMsg::CheckpointRename(id, e.text().to_string())));
                        }
                    });
                    a.present(Some(&parent));
                }),
            );
            row.insert_action_group("cp", Some(&group));
            row.add_suffix(&compare);
            row.add_suffix(&more);
            d.list.append(&row);
        }
    }

    /// Delete from the dialog: the row and the menu entries go away at once, the checkpoint itself after the Undo
    /// toast (10 s) is gone.
    pub(super) fn cp_delete_later(&mut self, id: u64) {
        if !self.checkpoints.list().iter().any(|c| c.id == id) {
            return;
        }
        let name = self
            .checkpoints
            .list()
            .into_iter()
            .find(|c| c.id == id)
            .map(|c| c.name)
            .unwrap_or_default();
        self.pv.cp_hidden.borrow_mut().push(id);
        self.refresh_restore_menu();
        self.refresh_cp_dialog();
        let text = trf("Checkpoint “{}” deleted", &[&name]);
        let toast = self.toast_button(&text, &tr("Undo"), move || {
            Input::Pv(PvMsg::CheckpointDeleteUndo(id))
        });
        let tx = self.sender.input_sender().clone();
        toast.connect_dismissed(move |_| tx.emit(Input::Pv(PvMsg::CheckpointDeleteCommit(id))));
    }

    /// Dev hooks of the compare view and the checkpoints dialog.
    pub(super) fn cmp_dev(&mut self, c: &str, sender: &ComponentSender<Self>) -> bool {
        let send = |m: CmpMsg| sender.input(Input::Pv(PvMsg::Cmp(m)));
        if c == "cmp-dump" {
            let g = self.pv.cmp.data.borrow();
            match g.as_ref() {
                Some(d) => {
                    let count = |kind: RowKind| {
                        d.sbs
                            .rows
                            .iter()
                            .filter(|r| matches!(r, Row::Line { kind: k, .. } if *k == kind))
                            .count()
                    };
                    let skips = d
                        .sbs
                        .rows
                        .iter()
                        .filter(|r| matches!(r, Row::Skip(_)))
                        .count();
                    let marks = d
                        .sbs
                        .rows
                        .iter()
                        .map(|r| match r {
                            Row::Line { old, new, .. } => {
                                old.iter().chain(new.iter()).map(|c| c.marks.len()).sum()
                            }
                            _ => 0,
                        })
                        .sum::<usize>();
                    let lb = self.pv.cmp.left.text.buffer();
                    let hit_tag = lb.tag_table().lookup("hit");
                    let hit_lines = (0..lb.line_count())
                        .filter(|l| {
                            lb.iter_at_line(*l)
                                .zip(hit_tag.as_ref())
                                .is_some_and(|(it, t)| it.has_tag(t))
                        })
                        .count();
                    eprintln!(
                        "editor_cmp: visible={} title={:?} files={} file={:?} rows={} equal={} added={} removed={} changed={} skips={} marks={} beautify={} context={} hits={} hit_lines={} left_lines={} right_lines={} goto_left={}",
                        self.w.center.visible_child_name().as_deref() == Some("compare"),
                        self.pv.cmp.title.text(),
                        d.files.len(),
                        d.path(),
                        d.sbs.rows.len(),
                        count(RowKind::Equal),
                        count(RowKind::Added),
                        count(RowKind::Removed),
                        count(RowKind::Changed),
                        skips,
                        marks,
                        d.beautify,
                        d.context,
                        d.hits.len(),
                        hit_lines,
                        self.pv.cmp.left.text.buffer().line_count(),
                        self.pv.cmp.right.text.buffer().line_count(),
                        d.goto_left,
                    );
                    eprintln!("editor_cmp_files: {}", d.files.join(" "));
                    eprintln!("editor_cmp_names: {} | {}", d.left_name, d.right_name);
                    let lt = buffer_text(&self.pv.cmp.left.text.buffer());
                    let rt = buffer_text(&self.pv.cmp.right.text.buffer());
                    let ln = buffer_text(&self.pv.cmp.left.nums.buffer());
                    eprintln!(
                        "editor_cmp_text: left={:?} right={:?} nums={:?}",
                        lt.chars().take(300).collect::<String>(),
                        rt.chars().take(300).collect::<String>(),
                        ln.chars().take(80).collect::<String>()
                    );
                }
                None => eprintln!("editor_cmp: none"),
            }
        } else if let Some(i) = c.strip_prefix("cmp-file=") {
            let idx = {
                let g = self.pv.cmp.data.borrow();
                g.as_ref()
                    .and_then(|d| d.files.iter().position(|f| f.ends_with(i)))
            };
            if let Some(idx) = idx {
                send(CmpMsg::File(idx));
            }
        } else if let Some(v) = c.strip_prefix("cmp-beautify=") {
            self.pv.cmp.beautify.set_active(v == "on");
        } else if let Some(n) = c.strip_prefix("cmp-context=") {
            self.pv.cmp.context.set_value(n.parse().unwrap_or(3.0));
        } else if let Some(q) = c.strip_prefix("cmp-search=") {
            self.pv.cmp.search.set_text(q);
        } else if c == "cmp-next-change" {
            send(CmpMsg::Change(true));
        } else if c == "cmp-search-next" {
            send(CmpMsg::SearchNext(true));
        } else if let Some(r) = c.strip_prefix("cmp-goto-text=") {
            // double click on the first row that shows this text
            let row = {
                let g = self.pv.cmp.data.borrow();
                g.as_ref().and_then(|d| {
                    d.sbs.rows.iter().position(|row| match row {
                        Row::Line { old, new, .. } => {
                            old.iter().chain(new.iter()).any(|c| c.text.contains(r))
                        }
                        _ => false,
                    })
                })
            };
            if let Some(row) = row {
                send(CmpMsg::Goto(row));
            }
        } else if c == "cmp-close" {
            send(CmpMsg::Close);
        } else if let Some(p) = c.strip_prefix("cmp-book=") {
            sender.input(Input::Pv(PvMsg::CompareBookFile(p.into())));
        } else if c == "cp-dialog" {
            sender.input(Input::Pv(PvMsg::Checkpoints));
        } else if let Some(n) = c.strip_prefix("cp-new=") {
            sender.input(Input::Pv(PvMsg::CheckpointNew(n.to_string())));
        } else if c == "cp-list" {
            eprintln!(
                "editor_cp: dialog={} list={}",
                self.pv.cp_dialog.borrow().is_some(),
                self.visible_checkpoints()
                    .iter()
                    .map(|c| format!("{}:{}{}", c.id, c.name, if c.pinned { "*" } else { "" }))
                    .collect::<Vec<_>>()
                    .join("|")
            );
            if let Some(d) = self.pv.cp_dialog.borrow().as_ref() {
                let mut n = 0;
                let mut ch = d.list.first_child();
                while let Some(w) = ch {
                    n += 1;
                    ch = w.next_sibling();
                }
                eprintln!("editor_cp: dialog_rows={n}");
            }
        } else if c == "cp-controls" {
            // `editor_cp_rows: rows=N max_controls=M subtitles=a|b` for the open dialog
            if let Some(d) = self.pv.cp_dialog.borrow().as_ref() {
                let (mut rows, mut max, mut subs) = (0, 0, Vec::new());
                let mut ch = d.list.first_child();
                while let Some(w) = ch {
                    rows += 1;
                    max = max.max(count_controls(&w));
                    if let Some(r) = w.downcast_ref::<adw::ActionRow>() {
                        subs.push(r.subtitle().map(|s| s.to_string()).unwrap_or_default());
                    }
                    ch = w.next_sibling();
                }
                eprintln!(
                    "editor_cp_rows: rows={rows} max_controls={max} subtitles={}",
                    subs.join("|")
                );
            }
        } else if let Some(id) = c.strip_prefix("cp-delete-undo=") {
            sender.input(Input::Pv(PvMsg::CheckpointDeleteUndo(
                id.parse().unwrap_or(0),
            )));
        } else if let Some(id) = c.strip_prefix("cp-delete=") {
            sender.input(Input::Pv(PvMsg::CheckpointDelete(id.parse().unwrap_or(0))));
        } else if let Some(r) = c.strip_prefix("cp-rename=") {
            let (id, name) = r.split_once('|').unwrap_or((r, ""));
            sender.input(Input::Pv(PvMsg::CheckpointRename(
                id.parse().unwrap_or(0),
                name.to_string(),
            )));
        } else if let Some(id) = c.strip_prefix("cp-pin=") {
            sender.input(Input::Pv(PvMsg::CheckpointPin(
                id.parse().unwrap_or(0),
                true,
            )));
        } else if let Some(id) = c.strip_prefix("cp-compare=") {
            sender.input(Input::Pv(PvMsg::CompareCheckpoint(id.parse().unwrap_or(0))));
        } else {
            return false;
        }
        true
    }
}

fn stats_text(d: &CmpData) -> String {
    let n = d.sbs.change_starts().len();
    let files = match d.files.len() {
        1 => "1 file differs".to_string(),
        k => format!("{k} files differ"),
    };
    trf(
        "{} · {} · {}",
        &[
            &(d.path().unwrap_or("")),
            &ntr("{} change", "{} changes", n),
            &files,
        ],
    )
}

/// Subtitle of a checkpoint row: the details, with "Kept" in front of a pinned one.
pub(super) fn cp_subtitle(base: &str, pinned: bool, kept: &str) -> String {
    if pinned {
        format!("{kept} · {base}")
    } else {
        base.to_string()
    }
}

/// Buttons and menu buttons under `w` (the button inside a menu button does not count twice).
fn count_controls(w: &gtk::Widget) -> usize {
    let mut n = 0;
    if w.is::<gtk::MenuButton>() {
        return 1;
    }
    if w.is::<gtk::Button>() {
        n += 1;
    }
    let mut ch = w.first_child();
    while let Some(c) = ch {
        n += count_controls(&c);
        ch = c.next_sibling();
    }
    n
}

#[cfg(test)]
mod cp_tests {
    use super::cp_subtitle;

    #[test]
    fn pinned_checkpoint_shows_kept_badge() {
        assert_eq!(
            cp_subtitle("12:00 · 3 files", true, "Kept"),
            "Kept · 12:00 · 3 files"
        );
        assert_eq!(
            cp_subtitle("12:00 · 3 files", false, "Kept"),
            "12:00 · 3 files"
        );
    }
}
