//! Drag and drop and keyboard moves in the Contents tab (CE-120): drag the grip of an entry onto another one to
//! move it before, after or into it (the pointer's place on the row chooses), Alt+arrow keys move and re-nest the
//! selected entry. The decisions are `bookx_editing::toc_ops`; this file is only the widgets.

use crate::i18n::tr;
use bookx_editing::toc_ops::{self, DropPos};

use super::struct_hub::StructCmd;
use super::tools_ui::*;
use super::*;

pub(super) fn install_css() {
    thread_local!(static DONE: Cell<bool> = const { Cell::new(false) });
    if DONE.with(|d| d.replace(true)) {
        return;
    }
    let css = gtk::CssProvider::new();
    css.load_from_string(
        ".toc-drop-before { box-shadow: inset 0 2px 0 0 @accent_bg_color; } \
         .toc-drop-after { box-shadow: inset 0 -2px 0 0 @accent_bg_color; } \
         .toc-drop-into { background-color: alpha(@accent_bg_color, 0.25); }",
    );
    if let Some(d) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &d,
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

fn clear(row: &impl IsA<gtk::Widget>) {
    for c in ["toc-drop-before", "toc-drop-after", "toc-drop-into"] {
        row.remove_css_class(c);
    }
}

fn class_of(pos: DropPos) -> &'static str {
    match pos {
        DropPos::Before => "toc-drop-before",
        DropPos::After => "toc-drop-after",
        DropPos::Into => "toc-drop-into",
    }
}

/// Makes the grip the drag source of the entry shown in `item` and the row a drop target. The list is
/// virtual, so the entry number is read from the item's position when the drag or the drop happens.
pub(super) fn toc_row_dnd(
    row: &gtk::Box,
    item: &gtk::ListItem,
    tx: relm4::Sender<Input>,
) -> gtk::Image {
    install_css();
    let grip = gtk::Image::from_icon_name("list-drag-handle-symbolic");
    grip.add_css_class("dim-label");
    grip.set_tooltip_text(Some(&tr("Drag to move this entry")));
    let src = gtk::DragSource::new();
    src.set_actions(gdk::DragAction::MOVE);
    {
        let item = item.downgrade();
        src.connect_prepare(move |_, _, _| {
            let i = item.upgrade()?.position();
            Some(gdk::ContentProvider::for_value(&i.to_value()))
        });
    }
    {
        let row = row.clone();
        src.connect_drag_begin(move |s, _| {
            let p = gtk::WidgetPaintable::new(Some(&row));
            s.set_icon(Some(&p), 0, 0);
        });
    }
    grip.add_controller(src);

    let target = gtk::DropTarget::new(u32::static_type(), gdk::DragAction::MOVE);
    {
        let row = row.clone();
        target.connect_motion(move |_, _, y| {
            clear(&row);
            row.add_css_class(class_of(toc_ops::drop_zone(y, f64::from(row.height()))));
            gdk::DragAction::MOVE
        });
    }
    {
        let row = row.clone();
        target.connect_leave(move |_| clear(&row));
    }
    {
        let row = row.clone();
        let item = item.downgrade();
        target.connect_drop(move |_, v, _, y| {
            clear(&row);
            let (Ok(from), Some(item)) = (v.get::<u32>(), item.upgrade()) else {
                return false;
            };
            let pos = toc_ops::drop_zone(y, f64::from(row.height()));
            tx.emit(Input::Ext(Cmd::Struct(StructCmd::TocDrop(
                from as usize,
                item.position() as usize,
                pos,
            ))));
            true
        });
    }
    row.add_controller(target);
    grip
}

impl Editor {
    /// Alt+Up/Down move the entry among its siblings, Alt+Left promotes, Alt+Right demotes (called once).
    pub(super) fn toc_keys_install(&self) {
        let key = gtk::EventControllerKey::new();
        key.set_propagation_phase(gtk::PropagationPhase::Capture);
        let tx = self.sender.input_sender().clone();
        let st = self.ext.structure.clone();
        key.connect_key_pressed(move |_, k, _, mods| {
            if !mods.contains(gdk::ModifierType::ALT_MASK)
                || mods.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK)
            {
                return glib::Propagation::Proceed;
            }
            let op = match k {
                gdk::Key::Up => TocOp::Up,
                gdk::Key::Down => TocOp::Down,
                gdk::Key::Left => TocOp::Outdent,
                gdk::Key::Right => TocOp::Indent,
                _ => return glib::Propagation::Proceed,
            };
            st.toc_refocus.set(true);
            tx.emit(Input::Toc(op));
            glib::Propagation::Stop
        });
        self.w.toc_list.add_controller(key);
    }

    /// The drop handler (also called by the dev hook `x:toc-drop`).
    pub(super) fn toc_drop(&mut self, from: usize, target: usize, pos: DropPos) {
        let mut t = self.toc.clone();
        match toc_ops::drop_entry(&mut t, from, target, pos) {
            Some(new) => {
                self.toc = t;
                self.toc_selected = Some(new);
                self.toc_dirty = true;
                self.rebuild_toc();
                self.update_title();
            }
            None => self.toast(&tr("That entry cannot go there")),
        }
    }
}
