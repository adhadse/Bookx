//! Live preview and delayed re-highlighting, and the sync between the preview and the code view:
//! follow the cursor (CE-243), click to place the cursor (CE-244), links (CE-245), auto reload and refresh
//! (CE-241/242), the "pick a location" mode used by the Contents and by splitting (CE-246, CE-124),
//! copy and find in the preview (CE-247).

use crate::i18n::{tr, trf};
use std::cell::{Cell, RefCell};

use bookx_editing::preview_map::{self, SourceMap};
use bookx_editing::split_merge::SplitSpec;

use super::*;

/// What a click in "pick a location" mode is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// Point Contents entry `n` at the block.
    Toc(usize),
    /// Split the file in front of the block.
    Split,
}

#[derive(Debug)]
pub enum PvMsg {
    Follow(bool),
    AutoReload(bool),
    Refresh,
    /// A click on a block of the preview.
    Clicked(usize),
    Link(String),
    PickStart(Pick),
    PickCancel,
    TocPick,
    SplitCursor,
    SplitDialog,
    SplitApply(SplitSpec),
    CopySelection,
    /// The code view scrolled / the preview scrolled (scroll sync).
    CodeScrolled,
    PreviewScrolled,
    CodeStep,
    PreviewStep,
    SyncBoth(bool),
    FindToggle,
    Find(String),
    FindNext(bool),
    Checkpoints,
    CheckpointNew(String),
    CheckpointDelete(u64),
    CheckpointRename(u64, String),
    CheckpointPin(u64, bool),
    CheckpointsClosed,
    CompareCheckpoint(u64),
    CompareBook,
    CompareBookFile(std::path::PathBuf),
    Cmp(super::compare::CmpMsg),
    Css(super::live_css::CssMsg),
}

/// Everything the preview sync keeps: widgets and the map of the shown page.
pub(super) struct PvState {
    pub follow: Cell<bool>,
    pub auto: Cell<bool>,
    /// The text changed since the preview was drawn.
    pub stale: Cell<bool>,
    /// Source map of the page in the preview.
    pub map: RefCell<SourceMap>,
    /// Char offset in the buffer where each block starts.
    pub chars: RefCell<Vec<Option<usize>>>,
    pub map_file: RefCell<Option<String>>,
    pub pick: RefCell<Option<Pick>>,
    /// Shared with the click handler, which must claim clicks while picking.
    pub pick_flag: Rc<Cell<bool>>,
    /// The cursor was put at this char offset by a click in the preview: the preview does not follow it back.
    pub skip_follow: Cell<Option<usize>>,
    marker: gtk::Box,
    banner: gtk::Box,
    banner_label: gtk::Label,
    refresh_btn: gtk::Button,
    pub(super) find_rev: gtk::Revealer,
    pub(super) find_entry: gtk::SearchEntry,
    find_hits: RefCell<Vec<(i32, i32)>>,
    find_at: Cell<usize>,
    pub cmp: super::compare::CmpUi,
    pub css: super::live_css::CssUi,
    pub cp_dialog: RefCell<Option<super::compare::CpDialog>>,
    pub split_dialog: RefCell<Option<super::split::SplitDlg>>,
    css_timer: Rc<RefCell<Option<glib::SourceId>>>,
    /// Scroll sync: preview scrolls the code too.
    pub both: Cell<bool>,
    both_btn: gtk::ToggleButton,
    pub gate: Cell<bookx_editing::scroll_sync::Gate>,
    /// A sync step is under way (its own scrolls must not start another).
    pub syncing: Cell<bool>,
    sync_timer: Rc<RefCell<Option<glib::SourceId>>>,
    sync_timer_pv: Rc<RefCell<Option<glib::SourceId>>>,
    /// Anchors of the laid out page: (flow height, width, map); rebuilt when the layout or the map changes.
    pub scroll_map: RefCell<Option<(i64, i32, bookx_editing::scroll_sync::ScrollMap)>>,
    pub sync_us: Cell<u64>,
    pub sync_steps: Cell<u32>,
}

/// What `PvState::new` needs from the editor window while it is being built.
pub(super) struct PvInit<'a> {
    pub sender: &'a ComponentSender<Editor>,
    pub preview: &'a BookCanvas,
    pub preview_head: &'a gtk::Box,
    pub win: &'a adw::Window,
    pub tools_menu: &'a gio::Menu,
    pub edit_menu: &'a gio::Menu,
    pub sidebar_stack: &'a adw::ViewStack,
    pub center: &'a gtk::Stack,
    /// Adds a button to the Contents toolbar.
    pub toc_add: &'a dyn Fn(&gtk::Widget),
    /// Remembered "Sync scrolling both ways".
    pub both: bool,
    pub code_adj: Option<gtk::Adjustment>,
}

fn toggle(icon: &str, tip: &str, on: bool) -> gtk::ToggleButton {
    let b = gtk::ToggleButton::new();
    b.set_icon_name(icon);
    b.set_tooltip_text(Some(tip));
    b.add_css_class("flat");
    b.set_active(on);
    crate::ui::audit::label(&b, tip);
    b
}

fn flat_button(icon: &str, tip: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.set_tooltip_text(Some(tip));
    b.add_css_class("flat");
    crate::ui::audit::label(&b, tip);
    b
}

thread_local! {
    static MARKER_CSS: Cell<bool> = const { Cell::new(false) };
}

fn install_marker_css() {
    if MARKER_CSS.with(|c| c.replace(true)) {
        return;
    }
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let p = gtk::CssProvider::new();
    p.load_from_string(
        ".pv-marker { background: #26a269; min-height: 3px; }\n.pv-marker.eink { background: #000; }\n\
         .pv-stale { color: @warning_color; }",
    );
    gtk::style_context_add_provider_for_display(
        &display,
        &p,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

/// Char offsets for ascending byte offsets of `text` in one pass.
fn bytes_to_chars(text: &str, bytes: &[Option<usize>]) -> Vec<Option<usize>> {
    let mut order: Vec<(usize, usize)> = bytes
        .iter()
        .enumerate()
        .filter_map(|(i, b)| b.map(|b| (b, i)))
        .collect();
    order.sort_unstable();
    let mut out = vec![None; bytes.len()];
    let mut k = 0;
    for (ci, (bi, _)) in text.char_indices().enumerate() {
        while k < order.len() && order[k].0 <= bi {
            out[order[k].1] = Some(ci);
            k += 1;
        }
        if k == order.len() {
            break;
        }
    }
    let total = text.chars().count();
    while k < order.len() {
        out[order[k].1] = Some(total);
        k += 1;
    }
    out
}

impl PvState {
    pub(super) fn new(init: PvInit) -> PvState {
        let PvInit {
            sender,
            preview,
            preview_head,
            win,
            tools_menu,
            edit_menu,
            sidebar_stack,
            center,
            toc_add,
            both,
            code_adj,
        } = init;
        install_marker_css();
        // ---- header: follow, auto reload, refresh, find, copy --------------------------------
        let follow_btn = toggle(
            "find-location-symbolic",
            "Follow the cursor: the preview scrolls to the element at the cursor",
            crate::editor_prefs::look().preview_follow,
        );
        let auto_btn = toggle(
            "media-playlist-repeat-symbolic",
            "Reload the preview automatically while typing",
            true,
        );
        let refresh_btn = flat_button("view-refresh-symbolic", &tr("Refresh the preview (F5)"));
        let both_btn = toggle(
            "object-flip-vertical-symbolic",
            &tr("Sync scrolling both ways: scrolling the preview also scrolls the code (the code always moves the preview)"),
            both,
        );
        let find_btn = flat_button("edit-find-symbolic", &tr("Find in the preview"));
        let copy_btn = flat_button(
            "edit-copy-symbolic",
            &tr("Copy the selection of the preview (Ctrl+Alt+C)"),
        );
        {
            let t = sender.input_sender().clone();
            follow_btn.connect_toggled(move |b| t.emit(Input::Pv(PvMsg::Follow(b.is_active()))));
            let t = sender.input_sender().clone();
            auto_btn.connect_toggled(move |b| t.emit(Input::Pv(PvMsg::AutoReload(b.is_active()))));
            let t = sender.input_sender().clone();
            both_btn.connect_toggled(move |b| t.emit(Input::Pv(PvMsg::SyncBoth(b.is_active()))));
            if let Some(adj) = &code_adj {
                let t = sender.input_sender().clone();
                adj.connect_value_changed(move |_| t.emit(Input::Pv(PvMsg::CodeScrolled)));
            }
            let t = sender.input_sender().clone();
            preview.on_position(move |_| t.emit(Input::Pv(PvMsg::PreviewScrolled)));
            let t = sender.input_sender().clone();
            refresh_btn.connect_clicked(move |_| t.emit(Input::Pv(PvMsg::Refresh)));
            let t = sender.input_sender().clone();
            find_btn.connect_clicked(move |_| t.emit(Input::Pv(PvMsg::FindToggle)));
            let t = sender.input_sender().clone();
            copy_btn.connect_clicked(move |_| t.emit(Input::Pv(PvMsg::CopySelection)));
        }
        for b in [
            follow_btn.upcast_ref::<gtk::Widget>(),
            auto_btn.upcast_ref(),
            both_btn.upcast_ref(),
            refresh_btn.upcast_ref(),
            find_btn.upcast_ref(),
            copy_btn.upcast_ref(),
        ] {
            preview_head.append(b);
        }

        // ---- the pane: find bar, pick banner, marker overlay ---------------------------------
        let pane = preview
            .area
            .parent()
            .and_then(|p| p.downcast::<gtk::Box>().ok());
        let find_entry = gtk::SearchEntry::new();
        find_entry.set_placeholder_text(Some(&tr("Find in the preview")));
        find_entry.set_hexpand(true);
        crate::ui::audit::label(&find_entry, &tr("Find in the preview"));
        let find_row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        find_row.set_margin_all(4);
        find_row.append(&find_entry);
        let find_rev = gtk::Revealer::new();
        find_rev.set_child(Some(&find_row));
        {
            let t = sender.input_sender().clone();
            find_entry.connect_search_changed(move |e| {
                t.emit(Input::Pv(PvMsg::Find(e.text().to_string())))
            });
            let t = sender.input_sender().clone();
            find_entry.connect_activate(move |_| t.emit(Input::Pv(PvMsg::FindNext(true))));
            let t = sender.input_sender().clone();
            find_entry.connect_next_match(move |_| t.emit(Input::Pv(PvMsg::FindNext(true))));
            let t = sender.input_sender().clone();
            find_entry.connect_previous_match(move |_| t.emit(Input::Pv(PvMsg::FindNext(false))));
        }
        let banner_label = gtk::Label::new(None);
        banner_label.set_wrap(true);
        banner_label.set_xalign(0.0);
        banner_label.set_hexpand(true);
        let cancel = gtk::Button::with_label(&tr("Cancel"));
        cancel.set_tooltip_text(Some(&tr("Stop picking a location")));
        {
            let t = sender.input_sender().clone();
            cancel.connect_clicked(move |_| t.emit(Input::Pv(PvMsg::PickCancel)));
        }
        let banner = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        banner.set_margin_all(6);
        banner.add_css_class("toolbar");
        banner.append(&banner_label);
        banner.append(&cancel);
        banner.set_visible(false);

        let marker = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        marker.add_css_class("pv-marker");
        if crate::eink::is_active() {
            marker.add_css_class("eink");
        }
        marker.set_valign(gtk::Align::Start);
        marker.set_can_target(false);
        marker.set_visible(false);
        let pick_flag = Rc::new(Cell::new(false));
        if let Some(pane) = &pane {
            pane.insert_child_after(&find_rev, Some(preview_head));
            pane.insert_child_after(&banner, Some(&find_rev));
            pane.remove(&preview.area);
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&preview.area));
            overlay.add_overlay(&marker);
            overlay.set_hexpand(true);
            overlay.set_vexpand(true);
            pane.append(&overlay);
        }

        // ---- clicks and hover in the preview -----------------------------------------------
        {
            let click = gtk::GestureClick::new();
            click.set_button(1);
            click.set_propagation_phase(gtk::PropagationPhase::Capture);
            let flag = pick_flag.clone();
            click.connect_pressed(move |g, _, _, _| {
                if flag.get() {
                    g.set_state(gtk::EventSequenceState::Claimed);
                }
            });
            let canvas = preview.clone();
            let t = sender.input_sender().clone();
            click.connect_released(move |_, n, x, y| {
                if n != 1 {
                    return;
                }
                if let Some(b) = canvas.block_at_point(x, y) {
                    t.emit(Input::Pv(PvMsg::Clicked(b)));
                }
            });
            preview.area.add_controller(click);

            let motion = gtk::EventControllerMotion::new();
            let flag = pick_flag.clone();
            let canvas = preview.clone();
            let mk = marker.clone();
            motion.connect_motion(move |_, x, y| {
                if !flag.get() {
                    return;
                }
                match canvas
                    .block_at_point(x, y)
                    .and_then(|b| canvas.block_top(b))
                {
                    Some(top) => {
                        mk.set_margin_top(top.max(0.0) as i32);
                        mk.set_visible(true);
                    }
                    None => mk.set_visible(false),
                }
            });
            preview.area.add_controller(motion);

            let t = sender.input_sender().clone();
            preview.on_link(move |href| t.emit(Input::Pv(PvMsg::Link(href))));
        }

        // ---- actions, shortcuts, menus ------------------------------------------------------
        let group = gio::SimpleActionGroup::new();
        for (name, make) in [
            ("split-cursor", (|| PvMsg::SplitCursor) as fn() -> PvMsg),
            ("split-pick", || PvMsg::PickStart(Pick::Split)),
            ("split-dialog", || PvMsg::SplitDialog),
            ("checkpoints", || PvMsg::Checkpoints),
            ("compare-book", || PvMsg::CompareBook),
            ("refresh", || PvMsg::Refresh),
            ("copy", || PvMsg::CopySelection),
            ("toc-pick", || PvMsg::TocPick),
        ] {
            let act = gio::SimpleAction::new(name, None);
            let t = sender.input_sender().clone();
            act.connect_activate(move |_, _| t.emit(Input::Pv(make())));
            group.add_action(&act);
        }
        win.insert_action_group("pv", Some(&group));
        let sc = gtk::ShortcutController::new();
        sc.set_scope(gtk::ShortcutScope::Local);
        for (accel, make) in [
            ("F5", (|| PvMsg::Refresh) as fn() -> PvMsg),
            ("<Control><Alt>c", || PvMsg::CopySelection),
        ] {
            let t = sender.input_sender().clone();
            if let Some(trigger) = gtk::ShortcutTrigger::parse_string(accel) {
                sc.add_shortcut(gtk::Shortcut::new(
                    Some(trigger),
                    Some(gtk::CallbackAction::new(move |_, _| {
                        t.emit(Input::Pv(make()));
                        glib::Propagation::Stop
                    })),
                ));
            }
        }
        win.add_controller(sc);

        let split = gio::Menu::new();
        split.append(Some(&tr("Split at Cursor")), Some("pv.split-cursor"));
        split.append(
            Some(&tr("Split by Picking in the Preview")),
            Some("pv.split-pick"),
        );
        split.append(
            Some(&tr("Split at Several Places")),
            Some("pv.split-dialog"),
        );
        edit_menu.append_section(None, &split);
        let cp = gio::Menu::new();
        cp.append(Some(&tr("Checkpoints")), Some("pv.checkpoints"));
        cp.append(
            Some(&tr("Compare with Another Book")),
            Some("pv.compare-book"),
        );
        tools_menu.append_section(None, &cp);

        // the Contents tab: point the selected entry at a place in the preview
        let pick_toc = flat_button(
            "find-location-symbolic",
            &tr("Point the entry at a place you click in the preview"),
        );
        {
            let t = sender.input_sender().clone();
            pick_toc.connect_clicked(move |_| t.emit(Input::Pv(PvMsg::TocPick)));
        }
        toc_add(pick_toc.upcast_ref());

        // ---- compare page and Live CSS page -------------------------------------------------
        let cmp = super::compare::CmpUi::new(sender);
        center.add_named(&cmp.root, Some("compare"));
        let css = super::live_css::CssUi::new(sender);
        sidebar_stack.add_titled_with_icon(
            &css.root,
            Some("css"),
            "Live CSS",
            "format-text-rich-symbolic",
        );
        {
            let t = sender.input_sender().clone();
            sidebar_stack.connect_visible_child_name_notify(move |s| {
                if s.visible_child_name().as_deref() == Some("css") {
                    t.emit(Input::Pv(PvMsg::Css(super::live_css::CssMsg::Refresh)));
                }
            });
        }

        PvState {
            follow: Cell::new(crate::editor_prefs::look().preview_follow),
            auto: Cell::new(true),
            stale: Cell::new(false),
            map: RefCell::new(SourceMap::default()),
            chars: RefCell::new(Vec::new()),
            map_file: RefCell::new(None),
            pick: RefCell::new(None),
            pick_flag,
            skip_follow: Cell::new(None),
            marker,
            banner,
            banner_label,
            refresh_btn,
            find_rev,
            find_entry,
            find_hits: RefCell::new(Vec::new()),
            find_at: Cell::new(0),
            cmp,
            css,
            cp_dialog: RefCell::new(None),
            split_dialog: RefCell::new(None),
            css_timer: Rc::new(RefCell::new(None)),
            both: Cell::new(both),
            both_btn,
            gate: Cell::new(Default::default()),
            syncing: Cell::new(false),
            sync_timer: Rc::new(RefCell::new(None)),
            sync_timer_pv: Rc::new(RefCell::new(None)),
            scroll_map: RefCell::new(None),
            sync_us: Cell::new(0),
            sync_steps: Cell::new(0),
        }
    }
}

fn now_ms() -> u64 {
    (glib::monotonic_time() / 1000) as u64
}

impl Editor {
    pub(super) fn schedule_highlight(&self) {
        if let Some(t) = self.hl_timer.borrow_mut().take() {
            t.remove();
        }
        let tx = self.sender.input_sender().clone();
        let slot = self.hl_timer.clone();
        let id = glib::timeout_add_local_once(std::time::Duration::from_millis(250), move || {
            slot.borrow_mut().take();
            tx.emit(Input::Rehighlight);
        });
        *self.hl_timer.borrow_mut() = Some(id);
    }

    pub(super) fn schedule_preview(&self) {
        if !self.w.preview_pane.is_visible() {
            return;
        }
        if !self.pv.auto.get() {
            self.pv.stale.set(true);
            self.pv.refresh_btn.add_css_class("pv-stale");
            return;
        }
        if let Some(t) = self.preview_timer.borrow_mut().take() {
            t.remove();
        }
        let tx = self.sender.input_sender().clone();
        let slot = self.preview_timer.clone();
        let id = glib::timeout_add_local_once(std::time::Duration::from_millis(600), move || {
            slot.borrow_mut().take();
            tx.emit(Input::RefreshPreview);
        });
        *self.preview_timer.borrow_mut() = Some(id);
    }

    pub(super) fn refresh_preview_now(&self) {
        let Some(path) = &self.current else { return };
        if !self.w.preview_pane.is_visible() {
            return;
        }
        if !matches!(Lang::for_path(path), Lang::Xml)
            || extension(path) == "opf"
            || extension(path) == "ncx"
        {
            return;
        }
        let text = self.buffers.get(path).map(buffer_text).unwrap_or_default();
        let frac = self.w.preview.fraction();
        // the book's own colours are only parsed when they are asked for
        let mut doc =
            if self.shared.cfg.borrow().reader.colors == crate::config::ColorMode::Original {
                self.book.borrow().doc_from_source_styled(&text, path)
            } else {
                parse_html(&text, path)
            };
        let map = preview_map::map_blocks(&text, &doc);
        preview_map::tag_blocks(&mut doc, &map);
        let starts: Vec<Option<usize>> = map
            .blocks
            .iter()
            .map(|b| b.as_ref().map(|s| s.range.start))
            .collect();
        *self.pv.chars.borrow_mut() = bytes_to_chars(&text, &starts);
        *self.pv.map.borrow_mut() = map;
        *self.pv.map_file.borrow_mut() = Some(path.clone());
        self.pv.stale.set(false);
        self.pv.refresh_btn.remove_css_class("pv-stale");
        self.bump_gate();
        *self.pv.scroll_map.borrow_mut() = None;
        self.w.preview.set_doc(doc, Some(frac), None, false);
        // the preview shows what the code view shows (the map of this reload), then the cursor is kept in view
        self.sync_code_to_preview();
        self.pv_follow_cursor();
        // the canvas may still apply its own start position when it is laid out: sync again afterwards
        self.schedule_sync(true);
    }

    // ---- scroll sync (enhancement 26) ---------------------------------------------------------

    fn bump_gate(&self) {
        let mut g = self.pv.gate.get();
        g.cursor_moved(now_ms());
        self.pv.gate.set(g);
    }

    /// Passes a scroll of one pane on after a short debounce (and after the quiet time of a cursor move).
    fn schedule_sync(&self, from_code: bool) {
        if self.pv.syncing.get() || !self.w.preview_pane.is_visible() {
            return;
        }
        let slot = if from_code {
            self.pv.sync_timer.clone()
        } else {
            self.pv.sync_timer_pv.clone()
        };
        if slot.borrow().is_some() {
            return;
        }
        let delay = bookx_editing::scroll_sync::step_delay_ms(&self.pv.gate.get(), now_ms(), 40);
        let tx = self.sender.input_sender().clone();
        let s2 = slot.clone();
        let id = glib::timeout_add_local_once(std::time::Duration::from_millis(delay), move || {
            let _ = &s2;
            tx.emit(Input::Pv(if from_code {
                PvMsg::CodeStep
            } else {
                PvMsg::PreviewStep
            }));
        });
        *slot.borrow_mut() = Some(id);
    }

    /// Whether a page with a source map is shown for the open file (the last known map is used even when
    /// the text changed since).
    fn sync_ready(&self) -> bool {
        self.w.preview_pane.is_visible()
            && self.pv.map_file.borrow().as_deref() == self.current.as_deref()
            && !self.pv.map.borrow().blocks.is_empty()
    }

    /// Runs `f` with the anchors of the laid out page, rebuilding them when the layout changed.
    fn with_scroll_map<R>(
        &self,
        f: impl FnOnce(&bookx_editing::scroll_sync::ScrollMap) -> R,
    ) -> Option<R> {
        let canvas = &self.w.preview;
        let (anchors, flow_h) = {
            let key = self
                .pv
                .scroll_map
                .borrow()
                .as_ref()
                .map(|(h, w, _)| (*h, *w));
            let now_key = (canvas.flow_height() as i64, canvas.area.width());
            if key == Some(now_key) {
                let g = self.pv.scroll_map.borrow();
                return g.as_ref().map(|(_, _, m)| f(m));
            }
            canvas.block_anchors()
        };
        let chars = self.pv.chars.borrow();
        let pairs: Vec<(usize, f64)> = anchors
            .iter()
            .filter_map(|(b, y)| chars.get(*b).copied().flatten().map(|c| (c, *y)))
            .collect();
        let len = self.buffers.get(self.current.as_ref()?)?.char_count() as usize;
        let m = bookx_editing::scroll_sync::ScrollMap::new(&pairs, len, flow_h);
        let r = (!m.is_empty()).then(|| f(&m));
        *self.pv.scroll_map.borrow_mut() = Some((flow_h as i64, canvas.area.width(), m));
        r
    }

    /// The source position (chars, fractional) at the top of the code view.
    fn code_top_offset(&self) -> f64 {
        let view = &self.w.view;
        let r = view.visible_rect();
        let (it, top) = view.line_at_y(r.y());
        let (_, h) = view.line_yrange(&it);
        let frac = if h > 0 {
            ((r.y() - top) as f64 / h as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };
        it.offset() as f64 + frac * it.chars_in_line().max(1) as f64
    }

    /// Scrolls the code view so that source position `off` (chars) is at its top.
    fn set_code_top(&self, off: f64) {
        let view = &self.w.view;
        let buf = view.buffer();
        let c = (off.floor().max(0.0) as i32).min(buf.char_count());
        let it = buf.iter_at_offset(c);
        let (y, h) = view.line_yrange(&it);
        let start = {
            let mut s = it;
            s.set_line_offset(0);
            s.offset()
        };
        let in_line = (off - start as f64).max(0.0) / it.chars_in_line().max(1) as f64;
        let target = y as f64 + in_line.min(1.0) * h as f64;
        if let Some(adj) = gtk::prelude::ScrollableExt::vadjustment(view) {
            let max = (adj.upper() - adj.page_size()).max(0.0);
            adj.set_value(target.clamp(0.0, max));
        }
    }

    /// Code view -> preview: the preview shows the part of the page the code view shows.
    pub(super) fn sync_code_to_preview(&self) {
        if !self.sync_ready() {
            return;
        }
        let t = std::time::Instant::now();
        let off = self.code_top_offset();
        if let Some(y) = self.with_scroll_map(|m| m.y_of(off)).flatten() {
            self.pv.syncing.set(true);
            self.w.preview.set_scroll_top(y);
            self.pv.syncing.set(false);
            self.pv.sync_steps.set(self.pv.sync_steps.get() + 1);
            self.pv.sync_us.set(t.elapsed().as_micros() as u64);
        }
    }

    /// Preview -> code view (the option "Sync scrolling both ways").
    fn sync_preview_to_code(&self) {
        if !self.sync_ready() {
            return;
        }
        let y = self.w.preview.scroll_top();
        if let Some(off) = self.with_scroll_map(|m| m.offset_of(y)).flatten() {
            self.pv.syncing.set(true);
            self.set_code_top(off);
            self.pv.syncing.set(false);
        }
    }

    /// Char offset of the caret in the open buffer.
    fn caret_char(&self) -> Option<usize> {
        let path = self.current.as_ref()?;
        let buf = self.buffers.get(path)?;
        Some(buf.iter_at_mark(&buf.get_insert()).offset() as usize)
    }

    fn pv_map_is_current(&self) -> bool {
        self.pv.map_file.borrow().as_deref() == self.current.as_deref()
            && !self.pv.stale.get()
            && !self.pv.map.borrow().blocks.is_empty()
    }

    /// The block of the preview that holds the char offset `c` of the open buffer.
    pub(super) fn block_at_char(&self, c: usize) -> Option<usize> {
        let chars = self.pv.chars.borrow();
        let mut best = None;
        for (i, s) in chars.iter().enumerate() {
            match s {
                Some(s) if *s <= c => best = Some(i),
                Some(_) => break,
                None => {}
            }
        }
        best
    }

    /// CE-243: the preview shows the element at the cursor.
    pub(super) fn pv_follow_cursor(&self) {
        if !self.pv.follow.get() || !self.pv_map_is_current() {
            return;
        }
        let Some(c) = self.caret_char() else { return };
        if self.pv.skip_follow.take() == Some(c) {
            return;
        }
        if let Some(b) = self.block_at_char(c) {
            self.w.preview.reveal_block(b);
        }
    }

    /// Called whenever the caret moved.
    pub(super) fn pv_cursor_moved(&self) {
        self.bump_gate();
        self.pv_follow_cursor();
        if self.w.sidebar_stack.visible_child_name().as_deref() == Some("css") {
            if let Some(t) = self.pv.css_timer.borrow_mut().take() {
                t.remove();
            }
            let tx = self.sender.input_sender().clone();
            let slot = self.pv.css_timer.clone();
            let id =
                glib::timeout_add_local_once(std::time::Duration::from_millis(180), move || {
                    slot.borrow_mut().take();
                    tx.emit(Input::Pv(PvMsg::Css(super::live_css::CssMsg::Refresh)));
                });
            *self.pv.css_timer.borrow_mut() = Some(id);
        }
    }

    /// Puts the caret at byte `off` of `path` (opening it) and shows it.
    pub(super) fn goto_offset(&mut self, path: &str, off: usize) {
        if self.current.as_deref() != Some(path) {
            self.select_file(path);
        }
        if !is_text_path(path) || self.current.as_deref() != Some(path) {
            return;
        }
        let buf = self.buffer_for(path);
        let text = buffer_text(&buf);
        let c = super::search::byte_to_char_idx(&text, off.min(text.len()));
        let mut it = buf.iter_at_offset(c as i32);
        buf.place_cursor(&it);
        self.w.view.scroll_to_iter(&mut it, 0.1, true, 0.5, 0.3);
    }

    fn pick_banner(&self, text: Option<&str>) {
        match text {
            Some(t) => {
                self.pv.banner_label.set_text(t);
                self.pv.banner.set_visible(true);
                self.pv.pick_flag.set(true);
                self.w.preview.area.set_cursor_from_name(Some("crosshair"));
            }
            None => {
                self.pv.banner.set_visible(false);
                self.pv.pick_flag.set(false);
                self.pv.marker.set_visible(false);
                self.w.preview.area.set_cursor(None);
                *self.pv.pick.borrow_mut() = None;
            }
        }
    }

    pub(super) fn pv_msg(&mut self, m: PvMsg) {
        match m {
            PvMsg::Follow(on) => {
                self.pv.follow.set(on);
                if on {
                    self.pv_follow_cursor();
                }
            }
            PvMsg::AutoReload(on) => {
                self.pv.auto.set(on);
                if on && self.pv.stale.get() {
                    self.refresh_preview_now();
                }
            }
            PvMsg::Refresh => {
                if !self.w.preview_pane.is_visible() {
                    self.w.preview_btn.set_active(true);
                }
                self.flush();
                self.refresh_preview_now();
            }
            PvMsg::Clicked(b) => self.pv_clicked(b),
            PvMsg::Link(href) => self.pv_link(&href),
            PvMsg::PickStart(p) => self.pv_pick_start(p),
            PvMsg::PickCancel => self.pick_banner(None),
            PvMsg::TocPick => match self.toc_selected {
                Some(i) if i < self.toc.len() => self.pv_pick_start(Pick::Toc(i)),
                _ => self.toast(&tr("Select a Contents entry first")),
            },
            PvMsg::SplitCursor => self.split_at_cursor(),
            PvMsg::SplitDialog => self.split_dialog(),
            PvMsg::SplitApply(spec) => self.split_apply(spec, "Split File"),
            PvMsg::CopySelection => match self.w.preview.selected_text() {
                Some(t) if !t.is_empty() => {
                    if let Some(d) = gdk::Display::default() {
                        d.clipboard().set_text(&t);
                    }
                    self.toast(&tr("Copied the selected text of the preview"));
                }
                _ => self.toast(&tr("Nothing is selected in the preview")),
            },
            PvMsg::CodeScrolled => self.schedule_sync(true),
            PvMsg::PreviewScrolled => {
                if self.pv.both.get() {
                    self.schedule_sync(false);
                }
            }
            PvMsg::CodeStep => {
                self.pv.sync_timer.borrow_mut().take();
                if self.pv.gate.get().wait_ms(now_ms()) > 0 {
                    self.schedule_sync(true);
                } else {
                    self.sync_code_to_preview();
                }
            }
            PvMsg::PreviewStep => {
                self.pv.sync_timer_pv.borrow_mut().take();
                if self.pv.gate.get().wait_ms(now_ms()) > 0 {
                    self.schedule_sync(false);
                } else if self.pv.both.get() {
                    self.sync_preview_to_code();
                }
            }
            PvMsg::SyncBoth(on) => {
                self.pv.both.set(on);
                self.prefs.scroll_both = on;
                self.prefs.save();
            }
            PvMsg::FindToggle => {
                let show = !self.pv.find_rev.reveals_child();
                self.pv.find_rev.set_reveal_child(show);
                if show {
                    self.pv.find_entry.grab_focus();
                } else {
                    self.w.preview.set_hits(Vec::new(), None);
                }
            }
            PvMsg::Find(q) => self.pv_find(&q),
            PvMsg::FindNext(fwd) => self.pv_find_step(fwd),
            PvMsg::Checkpoints => self.checkpoints_dialog(),
            PvMsg::CheckpointNew(name) => {
                let name = name.trim();
                let name = if name.is_empty() {
                    format!("Checkpoint {}", self.checkpoints.len() + 1)
                } else {
                    name.to_string()
                };
                self.create_checkpoint(&name);
                self.toast(&trf("Checkpoint “{}” created", &[&name]));
                self.refresh_cp_dialog();
            }
            PvMsg::CheckpointDelete(id) => {
                if self.checkpoints.delete(id).is_ok() {
                    self.refresh_restore_menu();
                    self.refresh_cp_dialog();
                }
            }
            PvMsg::CheckpointRename(id, name) => {
                if self.checkpoints.rename(id, &name).is_ok() {
                    self.refresh_restore_menu();
                    self.refresh_cp_dialog();
                }
            }
            PvMsg::CheckpointPin(id, on) => {
                let _ = self.checkpoints.set_pinned(id, on);
                self.refresh_cp_dialog();
            }
            PvMsg::CheckpointsClosed => *self.pv.cp_dialog.borrow_mut() = None,
            PvMsg::CompareCheckpoint(id) => {
                if let Some(d) = self.pv.cp_dialog.borrow().as_ref() {
                    d.close();
                }
                self.compare_checkpoint(id);
            }
            PvMsg::CompareBook => self.compare_book_dialog(),
            PvMsg::CompareBookFile(p) => self.compare_with_file(&p),
            PvMsg::Cmp(m) => self.cmp_msg(m),
            PvMsg::Css(m) => self.css_msg(m),
        }
    }

    /// CE-244: a click on a block of the preview puts the caret at its source.
    fn pv_clicked(&mut self, b: usize) {
        let Some(path) = self.current.clone() else {
            return;
        };
        if self.pv.pick.borrow().is_some() {
            self.pv_picked(b);
            return;
        }
        if !self.pv_map_is_current() {
            return;
        }
        let Some(c) = self.pv.chars.borrow().get(b).copied().flatten() else {
            return;
        };
        self.bump_gate();
        let buf = self.buffer_for(&path);
        let mut it = buf.iter_at_offset(c as i32);
        self.pv.skip_follow.set(Some(c));
        buf.place_cursor(&it);
        self.w.view.scroll_to_iter(&mut it, 0.1, true, 0.5, 0.3);
    }

    /// CE-245: a link of the book opens its file in the editor and the preview.
    fn pv_link(&mut self, href: &str) {
        if href.contains("://") || href.starts_with("mailto:") {
            self.toast(&trf("External link: {}", &[&href]));
            return;
        }
        let path = href.split('#').next().unwrap_or("");
        if path.is_empty() || !self.book.borrow().archive.contains(path) {
            self.toast(&trf("The book has no file {}", &[&href]));
            return;
        }
        self.open_location(href);
    }

    fn pv_pick_start(&mut self, p: Pick) {
        let ok = self.current.as_ref().is_some_and(|c| {
            bookx_editing::markup::is_markup_path(c) && self.book.borrow().archive.contains(c)
        });
        if !ok {
            self.toast(&tr("Open a chapter first"));
            return;
        }
        if !self.w.preview_pane.is_visible() {
            self.w.preview_btn.set_active(true);
        }
        self.flush();
        self.pv.stale.set(true);
        self.refresh_preview_now();
        let text = match &p {
            Pick::Toc(i) => trf(
                "Click in the preview where “{}” should point",
                &[&(self.toc.get(*i).map(|t| t.title.as_str()).unwrap_or(""))],
            ),
            Pick::Split => "Click in the preview where the file should be split".to_string(),
        };
        *self.pv.pick.borrow_mut() = Some(p);
        self.pick_banner(Some(&text));
    }

    /// A click while picking a location.
    fn pv_picked(&mut self, b: usize) {
        let Some(p) = self.pv.pick.borrow().clone() else {
            return;
        };
        let seg = self.pv.map.borrow().blocks.get(b).cloned().flatten();
        let Some(seg) = seg else {
            self.toast(&tr("That part of the preview has no place in the source"));
            return;
        };
        let Some(path) = self.current.clone() else {
            return;
        };
        self.pick_banner(None);
        match p {
            Pick::Split => {
                self.split_apply(SplitSpec::AtOffsets(vec![seg.range.start]), "Split File")
            }
            Pick::Toc(i) => {
                let buf = self.buffer_for(&path);
                let text = buffer_text(&buf);
                let base = format!("toc{}", i + 1);
                let Some((new_src, id)) = preview_map::ensure_id(&text, seg.tag.clone(), &base)
                else {
                    self.toast(&tr("Could not place a marker there"));
                    return;
                };
                if let Some(new_src) = new_src {
                    // only the start tag changes: keep the caret and the undo history of the rest
                    let a = super::search::byte_to_char_idx(&text, seg.tag.start);
                    let e = super::search::byte_to_char_idx(&text, seg.tag.end);
                    let new_tag =
                        &new_src[seg.tag.start..seg.tag.end + (new_src.len() - text.len())];
                    let mut ia = buf.iter_at_offset(a as i32);
                    let mut ie = buf.iter_at_offset(e as i32);
                    buf.begin_user_action();
                    buf.delete(&mut ia, &mut ie);
                    let mut at = buf.iter_at_offset(a as i32);
                    buf.insert(&mut at, new_tag);
                    buf.end_user_action();
                }
                if let Some(t) = self.toc.get_mut(i) {
                    t.href = format!("{path}#{id}");
                }
                self.toc_dirty = true;
                self.rebuild_toc();
                self.update_title();
                self.pv.stale.set(true);
                self.refresh_preview_now();
                self.toast(&trf("The entry now points to {}#{}", &[&path, &id]));
            }
        }
    }

    fn pv_find(&mut self, q: &str) {
        let hits = if q.is_empty() {
            Vec::new()
        } else {
            self.w.preview.find_all(q)
        };
        self.pv.find_at.set(0);
        if let Some(h) = hits.first() {
            self.w.preview.set_hits(hits.clone(), Some(0));
            self.w.preview.goto_char(h.0);
        } else {
            self.w.preview.set_hits(Vec::new(), None);
        }
        *self.pv.find_hits.borrow_mut() = hits;
    }

    fn pv_find_step(&mut self, fwd: bool) {
        let hits = self.pv.find_hits.borrow().clone();
        if hits.is_empty() {
            return;
        }
        let n = hits.len();
        let at = self.pv.find_at.get();
        let next = if fwd { (at + 1) % n } else { (at + n - 1) % n };
        self.pv.find_at.set(next);
        self.w.preview.set_hits(hits.clone(), Some(next));
        self.w.preview.goto_char(hits[next].0);
    }

    /// Dev hooks of the preview sync (`editor:pv-...`); `true` when the command was one of them.
    pub(super) fn pv_dev(&mut self, c: &str, sender: &ComponentSender<Self>) -> bool {
        let canvas = self.w.preview.clone();
        if c == "pv-state" {
            let map = self.pv.map.borrow();
            let mapped = map.blocks.iter().filter(|b| b.is_some()).count();
            eprintln!(
                "editor_pv: follow={} auto={} stale={} blocks={} mapped={} file={:?} pick={:?} marker={} banner={} top={:?}",
                self.pv.follow.get(),
                self.pv.auto.get(),
                self.pv.stale.get(),
                map.blocks.len(),
                mapped,
                self.pv.map_file.borrow(),
                self.pv.pick.borrow(),
                self.pv.marker.is_visible(),
                self.pv.banner.is_visible(),
                canvas.block_top(0),
            );
        } else if let Some(v) = c.strip_prefix("pv-follow=") {
            sender.input(Input::Pv(PvMsg::Follow(v == "on")));
        } else if let Some(v) = c.strip_prefix("pv-auto=") {
            sender.input(Input::Pv(PvMsg::AutoReload(v == "on")));
        } else if c == "pv-refresh" {
            sender.input(Input::Pv(PvMsg::Refresh));
        } else if let Some(n) = c.strip_prefix("pv-click-block=") {
            // a real hit test: the point just inside the top of block n goes through the same path as a click
            let n: usize = n.parse().unwrap_or(0);
            match canvas.block_top(n) {
                Some(top) => {
                    let hit = canvas.block_at_point(60.0, top + 6.0);
                    eprintln!("editor_pv: click block={n} hit={hit:?}");
                    if let Some(h) = hit {
                        sender.input(Input::Pv(PvMsg::Clicked(h)));
                    }
                }
                None => eprintln!("editor_pv: click block={n} hit=None (not laid out in view)"),
            }
        } else if let Some(n) = c.strip_prefix("pv-click-text=") {
            // scroll to the block whose text contains `n`, then click it
            let found = self
                .w
                .preview
                .canonical_text()
                .find(n)
                .map(|_| ())
                .is_some();
            let map = self.pv.map.borrow().clone();
            let text = self
                .current
                .as_ref()
                .and_then(|p| self.buffers.get(p))
                .map(buffer_text)
                .unwrap_or_default();
            let b = map.blocks.iter().position(|s| {
                s.as_ref()
                    .is_some_and(|s| text.get(s.range.clone()).is_some_and(|t| t.contains(n)))
            });
            eprintln!("editor_pv: text {n:?} block={b:?} found_in_preview={found}");
            if let Some(b) = b {
                canvas.reveal_block(b);
                glib::idle_add_local_once({
                    let tx = sender.input_sender().clone();
                    move || tx.emit(Input::Dev(format!("pv-click-block={b}")))
                });
            }
        } else if let Some(h) = c.strip_prefix("pv-link=") {
            sender.input(Input::Pv(PvMsg::Link(h.to_string())));
        } else if let Some(i) = c.strip_prefix("pv-pick-toc=") {
            self.toc_selected = i.parse().ok();
            sender.input(Input::Pv(PvMsg::TocPick));
        } else if c == "pv-pick-split" {
            sender.input(Input::Pv(PvMsg::PickStart(Pick::Split)));
        } else if c == "pv-pick-cancel" {
            sender.input(Input::Pv(PvMsg::PickCancel));
        } else if c == "pv-split-cursor" {
            sender.input(Input::Pv(PvMsg::SplitCursor));
        } else if let Some(r) = c.strip_prefix("pv-split=") {
            self.dev_split(r, sender);
        } else if let Some(q) = c.strip_prefix("pv-find=") {
            sender.input(Input::Pv(PvMsg::Find(q.to_string())));
            let n = self.w.preview.find_all(q).len();
            eprintln!("editor_pv: find {q:?} hits={n}");
        } else if c == "pv-copy" {
            canvas.select_all();
            eprintln!(
                "editor_pv: copy {:?}",
                canvas
                    .selected_text()
                    .unwrap_or_default()
                    .chars()
                    .take(40)
                    .collect::<String>()
            );
            sender.input(Input::Pv(PvMsg::CopySelection));
        } else if let Some(n) = c.strip_prefix("pv-fill=") {
            // N filler paragraphs in front of </body>, so the file is long enough to scroll
            if let Some(p) = self.current.clone() {
                let buf = self.buffer_for(&p);
                let text = buffer_text(&buf);
                if let Some(b) = text.rfind("</body>") {
                    let at = super::search::byte_to_char_idx(&text, b);
                    let n: usize = n.parse().unwrap_or(100);
                    let fill: String = (1..=n)
                        .map(|k| format!("<p>Filler paragraph {k} of the long test chapter, with a few words to wrap.</p>\n"))
                        .collect();
                    let mut it = buf.iter_at_offset(at as i32);
                    buf.insert(&mut it, &fill);
                }
            }
        } else if let Some(v) = c.strip_prefix("pv-hide=") {
            self.w.preview_btn.set_active(v != "on");
        } else if let Some(n) = c.strip_prefix("pv-code-scroll=") {
            // the code view scrolls to the top of line N, as the wheel would (the sync reacts to the event)
            let buf = self.w.view.buffer();
            if let Some(it) = buf.iter_at_line(n.parse::<i32>().unwrap_or(1) - 1) {
                let (y, _) = self.w.view.line_yrange(&it);
                if let Some(adj) = gtk::prelude::ScrollableExt::vadjustment(&self.w.view) {
                    adj.set_value(y as f64);
                }
            }
        } else if let Some(f) = c.strip_prefix("pv-preview-scroll=") {
            // the preview scrolls to this fraction of its height, as the wheel would
            canvas.goto_fraction(f.parse().unwrap_or(0.0));
        } else if let Some(v) = c.strip_prefix("pv-both=") {
            self.pv.both_btn.set_active(v == "on");
        } else if c == "pv-sync-state" {
            let off = self.code_top_offset();
            let buf = self.w.view.buffer();
            let line = buf.iter_at_offset(off as i32).line() + 1;
            let code_block = self.block_at_char(off as usize);
            let preview_block = canvas.block_at_point(60.0, 40.0);
            eprintln!(
                "editor_sync: both={} code_line={} code_block={:?} preview_block={:?} preview_y={:.0} steps={} step_us={} map={}",
                self.pv.both.get(),
                line,
                code_block,
                preview_block,
                canvas.scroll_top(),
                self.pv.sync_steps.get(),
                self.pv.sync_us.get(),
                self.pv.scroll_map.borrow().as_ref().map(|m| m.2.len()).unwrap_or(0)
            );
        } else if c == "pv-where" {
            let b = self.caret_char().and_then(|c| self.block_at_char(c));
            let top = b.and_then(|b| canvas.block_top(b));
            let view = canvas.visible_height();
            let caret_text: String = self
                .current
                .as_ref()
                .and_then(|p| self.buffers.get(p))
                .map(|buf| {
                    let it = buf.iter_at_mark(&buf.get_insert());
                    let mut e = it;
                    e.forward_chars(26);
                    buf.text(&it, &e, false).to_string()
                })
                .unwrap_or_default();
            eprintln!(
                "editor_pv: caret_block={:?} top={:?} visible={} view={:.0} frac={:.2} caret_text={:?}",
                b,
                top,
                top.is_some_and(|t| t >= 0.0 && t < view),
                view,
                canvas.fraction(),
                caret_text
            );
        } else if let Some(n) = c.strip_prefix("pv-hover=") {
            // what moving the pointer over block n does while picking
            let top = n.parse().ok().and_then(|n| canvas.block_top(n));
            if self.pv.pick_flag.get() {
                if let Some(t) = top {
                    self.pv.marker.set_margin_top(t.max(0.0) as i32);
                    self.pv.marker.set_visible(true);
                }
            }
            eprintln!(
                "editor_pv: marker visible={} top={}",
                self.pv.marker.is_visible(),
                self.pv.marker.margin_top()
            );
        } else if c == "pv-toc" {
            eprintln!(
                "editor_pv_toc: {}",
                self.toc
                    .iter()
                    .map(|t| format!("{}={}", t.title, t.href))
                    .collect::<Vec<_>>()
                    .join("|")
            );
        } else if c == "pv-scroll-top" {
            canvas.goto_fraction(0.0);
        } else if c == "pv-scroll-end" {
            canvas.goto_fraction(1.0);
        } else if c == "pv-block-count" {
            eprintln!("editor_pv: blocks={}", self.pv.map.borrow().blocks.len());
        } else if c.starts_with("pv-dialog") {
            return self.dev_dialog(c);
        } else if c.starts_with("cmp-") || c.starts_with("cp-") {
            return self.cmp_dev(c, sender);
        } else if c.starts_with("css-") {
            return self.css_dev(c, sender);
        } else {
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::bytes_to_chars;

    #[test]
    fn byte_offsets_become_char_offsets_in_one_pass() {
        let text = "aé—b<p>ü</p>";
        let p = text.find("<p>").unwrap();
        let u = text.find('ü').unwrap();
        let out = bytes_to_chars(text, &[Some(p), None, Some(0), Some(u), Some(text.len())]);
        assert_eq!(out, [Some(4), None, Some(0), Some(7), Some(12)]);
        assert_eq!(bytes_to_chars("", &[Some(0)]), [Some(0)]);
        assert!(bytes_to_chars(text, &[]).is_empty());
    }

    #[test]
    fn offsets_past_the_end_are_clamped_to_the_end() {
        assert_eq!(bytes_to_chars("ab", &[Some(99)]), [Some(2)]);
    }
}
