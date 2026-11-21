//! A list of books with a check box each, shown as rows or as tiles: the widget behind both lists of
//! the Device window ("Books to send", "Books on the device").
//!
//! Layout rules (they are tested through the dev hooks of the window):
//! * the check box is the first element of a row, in a column of fixed width;
//! * the cover sits in a fixed 40x60 slot (the picture is fitted and centred in it), so the rows of a
//!   list line up whatever the shape of the covers;
//! * the title takes one line and is cut with an ellipsis, details follow on dim lines;
//! * clicking anywhere on a row or a tile toggles it; Shift-click selects the range from the last click;
//! * a tile shows its check box in the top-left corner and is highlighted while it is selected;
//! * the lists carry the libadwaita `.selection-mode` class: the check boxes stay invisible at rest
//!   and appear on hover, focus and for selected items (XC-3 / LIB-D5).

use std::cell::Cell;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use relm4::gtk::{self, gdk, graphene, prelude::*};

pub const COVER_W: i32 = 40;
pub const COVER_H: i32 = 60;
const CHECK_COLUMN: i32 = 34;
const TILE_W: i32 = 104;
const TILE_H: i32 = 156;

/// One book of a list.
#[derive(Clone, Default)]
pub struct Item {
    pub title: String,
    /// Dim lines below the title (author, format, size…).
    pub lines: Vec<String>,
    /// A warning shown below the details.
    pub warning: Option<String>,
    pub checked: bool,
    pub cover: Option<PathBuf>,
    pub placeholder_seed: String,
}

/// Whether a `.selection-mode` check box is shown for one item. It is hidden at rest, and shown
/// while the pointer or the keyboard is on the row or the item is selected.
pub fn selection_check_visible(selected: bool, hovered: bool, focused: bool) -> bool {
    selected || hovered || focused
}

struct Row {
    row_check: gtk::CheckButton,
    tile_check: gtk::CheckButton,
    row: gtk::ListBoxRow,
    tile: gtk::Box,
    row_pic: gtk::Picture,
    tile_pic: gtk::Picture,
    hover: Cell<bool>,
    focus: Cell<bool>,
}

pub struct PickList {
    pub widget: gtk::Stack,
    list: gtk::ListBox,
    grid: gtk::FlowBox,
    rows: RefCell<Vec<Row>>,
    state: RefCell<Vec<bool>>,
    last: Cell<Option<usize>>,
    covers: Cell<usize>,
    on_change: Rc<dyn Fn()>,
}

impl PickList {
    pub fn new(on_change: Rc<dyn Fn()>) -> Rc<PickList> {
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.add_css_class("boxed-list");
        list.add_css_class("selection-mode");
        let list_scroll = gtk::ScrolledWindow::new();
        list_scroll.set_child(Some(&list));
        list_scroll.set_min_content_height(180);
        list_scroll.set_max_content_height(360);
        list_scroll.set_propagate_natural_height(false);
        let grid = gtk::FlowBox::new();
        grid.set_selection_mode(gtk::SelectionMode::None);
        grid.add_css_class("selection-mode");
        grid.set_homogeneous(true);
        grid.set_max_children_per_line(8);
        grid.set_min_children_per_line(2);
        grid.set_row_spacing(8);
        grid.set_column_spacing(8);
        grid.set_valign(gtk::Align::Start);
        let grid_scroll = gtk::ScrolledWindow::new();
        grid_scroll.set_child(Some(&grid));
        grid_scroll.set_min_content_height(180);
        grid_scroll.set_max_content_height(360);
        grid_scroll.set_propagate_natural_height(false);
        let stack = gtk::Stack::new();
        stack.add_named(&grid_scroll, Some("grid"));
        stack.add_named(&list_scroll, Some("list"));
        stack.set_vhomogeneous(false);
        stack.set_visible_child_name("grid");
        Rc::new(PickList {
            widget: stack,
            list,
            grid,
            rows: RefCell::default(),
            state: RefCell::default(),
            last: Cell::new(None),
            covers: Cell::new(0),
            on_change,
        })
    }

    pub fn set_grid(&self, grid: bool) {
        self.widget
            .set_visible_child_name(if grid { "grid" } else { "list" });
    }

    pub fn len(&self) -> usize {
        self.state.borrow().len()
    }

    pub fn checked(&self) -> Vec<bool> {
        self.state.borrow().clone()
    }

    pub fn selected(&self) -> usize {
        self.state.borrow().iter().filter(|c| **c).count()
    }

    pub fn covers_shown(&self) -> usize {
        self.covers.get()
    }

    /// `(shown, hidden)` check boxes, for the `devwin:layout` hook.
    pub fn checkbox_visibility(&self) -> (usize, usize) {
        let shown = self
            .rows
            .borrow()
            .iter()
            .filter(|r| r.row_check.opacity() > 0.0)
            .count();
        (shown, self.rows.borrow().len() - shown)
    }

    /// Both views carry the libadwaita selection-mode class (XC-3).
    pub fn has_selection_mode_class(&self) -> bool {
        self.list.has_css_class("selection-mode") && self.grid.has_css_class("selection-mode")
    }

    fn refresh_check(&self, i: usize) {
        let rows = self.rows.borrow();
        let Some(r) = rows.get(i) else {
            return;
        };
        let selected = self.state.borrow().get(i).copied().unwrap_or(false);
        let opacity = if selection_check_visible(selected, r.hover.get(), r.focus.get()) {
            1.0
        } else {
            0.0
        };
        r.row_check.set_opacity(opacity);
        r.tile_check.set_opacity(opacity);
    }

    fn paint(&self, i: usize) {
        let on = self.state.borrow().get(i).copied().unwrap_or(false);
        if let Some(r) = self.rows.borrow().get(i) {
            if r.row_check.is_active() != on {
                r.row_check.set_active(on);
            }
            if r.tile_check.is_active() != on {
                r.tile_check.set_active(on);
            }
            for w in [r.tile.upcast_ref::<gtk::Widget>(), r.row.upcast_ref()] {
                if on {
                    w.add_css_class("pick-selected");
                } else {
                    w.remove_css_class("pick-selected");
                }
            }
        }
        self.refresh_check(i);
    }

    pub fn set_checked(&self, i: usize, on: bool) {
        if let Some(c) = self.state.borrow_mut().get_mut(i) {
            *c = on;
        } else {
            return;
        }
        self.paint(i);
        (self.on_change)();
    }

    pub fn set_all(&self, on: bool) {
        let n = self.len();
        for c in self.state.borrow_mut().iter_mut() {
            *c = on;
        }
        for i in 0..n {
            self.paint(i);
        }
        (self.on_change)();
    }

    /// A click on item `i`: toggles it, or with Shift sets the range from the last click to its new state.
    pub fn click(&self, i: usize, shift: bool) {
        let now = !self.state.borrow().get(i).copied().unwrap_or(false);
        match (shift, self.last.get()) {
            (true, Some(from)) if from < self.len() => {
                let (a, b) = (from.min(i), from.max(i));
                for k in a..=b {
                    if let Some(c) = self.state.borrow_mut().get_mut(k) {
                        *c = now;
                    }
                    self.paint(k);
                }
                (self.on_change)();
            }
            _ => self.set_checked(i, now),
        }
        self.last.set(Some(i));
    }

    /// Shows the cover image in file `path` on item `i`.
    pub fn set_cover(&self, i: usize, path: &std::path::Path) {
        let Some(tex) = super::covers::thumbnail(path, 100) else {
            return;
        };
        if let Some(r) = self.rows.borrow().get(i) {
            r.row_pic.set_paintable(Some(&tex));
            r.tile_pic.set_paintable(Some(&tex));
            for p in [&r.row_pic, &r.tile_pic] {
                for c in p.css_classes() {
                    if c.starts_with("cover-") {
                        p.remove_css_class(&c);
                    }
                }
            }
            self.covers.set(self.covers.get() + 1);
        }
    }

    pub fn set_items(self: &Rc<Self>, items: &[Item]) {
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        while let Some(c) = self.grid.first_child() {
            self.grid.remove(&c);
        }
        self.covers.set(0);
        self.last.set(None);
        *self.state.borrow_mut() = items.iter().map(|i| i.checked).collect();
        let mut rows = Vec::new();
        for (i, it) in items.iter().enumerate() {
            rows.push(self.build(i, it));
        }
        *self.rows.borrow_mut() = rows;
        for (i, it) in items.iter().enumerate() {
            self.paint(i);
            if let Some(p) = &it.cover {
                self.set_cover(i, p);
            }
        }
        self.widget.set_visible(!items.is_empty());
    }

    fn build(self: &Rc<Self>, i: usize, it: &Item) -> Row {
        let seed = if it.placeholder_seed.is_empty() {
            &it.title
        } else {
            &it.placeholder_seed
        };
        let class = super::covers::placeholder_class(seed);
        let pic = |w: i32, h: i32| {
            let p = gtk::Picture::new();
            p.set_size_request(w, h);
            p.set_content_fit(gtk::ContentFit::Contain);
            p.set_can_shrink(true);
            p.set_halign(gtk::Align::Center);
            p.set_valign(gtk::Align::Center);
            p.add_css_class(&class);
            p.add_css_class("device-cover");
            p
        };
        // the check boxes take no clicks themselves: the whole row or tile does
        let check = || {
            let c = gtk::CheckButton::new();
            c.set_can_target(false);
            c.set_opacity(0.0);
            super::audit::label(&c, &crate::i18n::trf("Select {}", &[&it.title]));
            c
        };

        // ---- row: check column · cover slot · text
        let row_check = check();
        row_check.set_halign(gtk::Align::Center);
        row_check.set_valign(gtk::Align::Center);
        let check_col = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        check_col.set_size_request(CHECK_COLUMN, -1);
        check_col.set_hexpand(false);
        check_col.set_halign(gtk::Align::Start);
        check_col.append(&row_check);
        check_col.set_homogeneous(true);
        let row_pic = pic(COVER_W, COVER_H);
        let slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        slot.set_size_request(COVER_W, COVER_H);
        slot.set_hexpand(false);
        slot.set_vexpand(false);
        slot.set_halign(gtk::Align::Start);
        slot.set_valign(gtk::Align::Center);
        slot.set_overflow(gtk::Overflow::Hidden);
        slot.append(&row_pic);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 1);
        text.set_hexpand(true);
        text.set_valign(gtk::Align::Center);
        let title = gtk::Label::new(Some(&it.title));
        title.set_xalign(0.0);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.set_single_line_mode(true);
        title.add_css_class("heading");
        text.append(&title);
        for l in it.lines.iter().filter(|l| !l.is_empty()) {
            let d = gtk::Label::new(Some(l));
            d.set_xalign(0.0);
            d.set_ellipsize(gtk::pango::EllipsizeMode::End);
            d.set_single_line_mode(true);
            d.add_css_class("dim-label");
            d.add_css_class("caption");
            text.append(&d);
        }
        if let Some(w) = &it.warning {
            let l = gtk::Label::new(Some(&format!("⚠ {w}")));
            l.set_xalign(0.0);
            l.set_wrap(true);
            l.add_css_class("caption");
            l.add_css_class("warning");
            text.append(&l);
        }
        let inner = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        inner.set_margin_top(6);
        inner.set_margin_bottom(6);
        inner.set_margin_start(6);
        inner.set_margin_end(10);
        inner.append(&check_col);
        inner.append(&slot);
        inner.append(&text);
        let row = gtk::ListBoxRow::new();
        row.set_activatable(false);
        row.set_selectable(false);
        row.set_focusable(true);
        row.set_child(Some(&inner));
        row.add_css_class("pick-row");
        row.add_css_class("selection-mode");
        self.list.append(&row);
        self.attach_click(&row, i);

        // ---- tile: cover with the check box in the corner, caption below
        let tile_check = check();
        tile_check.set_halign(gtk::Align::Start);
        tile_check.set_valign(gtk::Align::Start);
        tile_check.set_margin_start(6);
        tile_check.set_margin_top(6);
        tile_check.add_css_class("osd");
        let tile_pic = pic(TILE_W - 12, TILE_H - 12);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&tile_pic));
        overlay.add_overlay(&tile_check);
        let cap = gtk::Label::new(Some(&it.title));
        cap.set_ellipsize(gtk::pango::EllipsizeMode::End);
        cap.set_max_width_chars(14);
        cap.set_lines(2);
        cap.set_wrap(true);
        cap.add_css_class("caption");
        let tile = gtk::Box::new(gtk::Orientation::Vertical, 4);
        tile.set_margin_top(6);
        tile.set_margin_bottom(6);
        tile.set_margin_start(6);
        tile.set_margin_end(6);
        tile.set_focusable(true);
        tile.add_css_class("pick-tile");
        tile.add_css_class("selection-mode");
        tile.set_tooltip_text(Some(
            &std::iter::once(it.title.clone())
                .chain(it.lines.iter().cloned())
                .chain(it.warning.clone())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
        ));
        tile.append(&overlay);
        tile.append(&cap);
        if it.warning.is_some() {
            let w = gtk::Label::new(Some("⚠"));
            w.add_css_class("warning");
            tile.append(&w);
        }
        self.grid.append(&tile);
        self.attach_click(&tile, i);
        Row {
            row_check,
            tile_check,
            row,
            tile,
            row_pic,
            tile_pic,
            hover: Cell::new(false),
            focus: Cell::new(false),
        }
    }

    fn attach_click(self: &Rc<Self>, w: &impl IsA<gtk::Widget>, i: usize) {
        let g = gtk::GestureClick::new();
        g.set_button(gdk::BUTTON_PRIMARY);
        let weak = Rc::downgrade(self);
        g.connect_released(move |g, _, _, _| {
            let shift = g
                .current_event_state()
                .contains(gdk::ModifierType::SHIFT_MASK);
            if let Some(me) = weak.upgrade() {
                me.click(i, shift);
            }
        });
        w.add_controller(g);

        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(self);
        motion.connect_enter(move |_, _, _| {
            if let Some(me) = weak.upgrade() {
                if let Some(r) = me.rows.borrow().get(i) {
                    r.hover.set(true);
                }
                me.refresh_check(i);
            }
        });
        let weak = Rc::downgrade(self);
        motion.connect_leave(move |_| {
            if let Some(me) = weak.upgrade() {
                if let Some(r) = me.rows.borrow().get(i) {
                    r.hover.set(false);
                }
                me.refresh_check(i);
            }
        });
        w.add_controller(motion);

        let widget = w.clone().upcast::<gtk::Widget>();
        let weak = Rc::downgrade(self);
        w.connect_has_focus_notify(move |_| {
            if let Some(me) = weak.upgrade() {
                if let Some(r) = me.rows.borrow().get(i) {
                    r.focus.set(widget.has_focus());
                }
                me.refresh_check(i);
            }
        });
    }

    /// For the UI tests: for every row of the list view `(check x, cover slot x, cover slot width,
    /// cover slot height)` relative to the list. Zeros while the list view is not shown.
    pub fn layout(&self) -> Vec<(i32, i32, i32, i32)> {
        let origin = graphene::Point::new(0.0, 0.0);
        self.rows
            .borrow()
            .iter()
            .map(|r| {
                let cx = r
                    .row_check
                    .compute_point(&self.list, &origin)
                    .map_or(-1, |p| p.x().round() as i32);
                let slot = r.row_pic.parent();
                let (px, pw, ph) = slot.map_or((-1, 0, 0), |s| {
                    (
                        s.compute_point(&self.list, &origin)
                            .map_or(-1, |p| p.x().round() as i32),
                        s.width(),
                        s.height(),
                    )
                });
                (cx, px, pw, ph)
            })
            .collect()
    }

    /// The text of the rows, for the UI tests.
    pub fn visible_view(&self) -> String {
        self.widget
            .visible_child_name()
            .map(|s| s.to_string())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::selection_check_visible;

    #[test]
    fn checkboxes_are_hidden_at_rest() {
        assert!(!selection_check_visible(false, false, false));
        assert!(
            selection_check_visible(false, true, false),
            "hover shows it"
        );
        assert!(
            selection_check_visible(false, false, true),
            "focus shows it"
        );
        assert!(
            selection_check_visible(true, false, false),
            "a selected item keeps it"
        );
    }
}
