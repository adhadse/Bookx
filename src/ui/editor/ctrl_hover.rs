//! While Ctrl is held the link target or class name under the pointer is underlined and the pointer becomes a
//! hand: the same places Ctrl+click jumps from (`bookx_editing::code_nav::hit_at`, tested there).

use std::cell::RefCell;
use std::rc::Rc;

use super::search::{byte_to_char_idx, char_to_byte_idx};
use super::*;
use bookx_editing::code_nav::{self as nav, Hit};

pub const TAG_CTRL: &str = "ctrl-hover";
/// Files above this size are not scanned on every pointer move.
const MAX_BYTES: usize = 1_500_000;

#[derive(Default)]
struct State {
    ctrl: bool,
    pos: Option<(f64, f64)>,
    /// Character range that carries the tag now, and the buffer it is on.
    shown: Option<(gtk::TextBuffer, i32, i32)>,
    /// `(ctrl, offset)` of the last pass, to skip identical ones.
    last: Option<(bool, i32)>,
}

pub(super) fn add_tag(table: &gtk::TextTagTable) {
    if table.lookup(TAG_CTRL).is_none() {
        let t = gtk::TextTag::new(Some(TAG_CTRL));
        t.set_underline(gtk::pango::Underline::Single);
        table.add(&t);
    }
}

fn is_css(buf: &gtk::TextBuffer) -> bool {
    buf.downcast_ref::<sv::Buffer>()
        .and_then(|b| b.language())
        .is_some_and(|l| l.id() == "css")
}

/// The character range a Ctrl+click at character `off` would act on.
pub(super) fn target_range(buf: &gtk::TextBuffer, off: i32) -> Option<(i32, i32)> {
    let (s, e) = buf.bounds();
    let text = buf.text(&s, &e, false).to_string();
    if text.len() > MAX_BYTES {
        return None;
    }
    let byte = char_to_byte_idx(&text, off.max(0) as usize);
    let range = match nav::hit_at(&text, byte, is_css(buf))? {
        Hit::Link { range, .. } | Hit::Class { range, .. } | Hit::Id { range, .. } => range,
    };
    Some((
        byte_to_char_idx(&text, range.start) as i32,
        byte_to_char_idx(&text, range.end) as i32,
    ))
}

/// Underlines `range` (or nothing) and sets the pointer.
fn show(view: &sv::View, st: &mut State, range: Option<(i32, i32)>) {
    if let Some((b, s, e)) = st.shown.take() {
        b.remove_tag_by_name(TAG_CTRL, &b.iter_at_offset(s), &b.iter_at_offset(e));
    }
    match range {
        Some((s, e)) => {
            let b = view.buffer();
            b.apply_tag_by_name(TAG_CTRL, &b.iter_at_offset(s), &b.iter_at_offset(e));
            st.shown = Some((b, s, e));
            view.set_cursor_from_name(Some("pointer"));
        }
        None => view.set_cursor_from_name(Some("text")),
    }
}

fn update(view: &sv::View, st: &mut State) {
    let off = match (st.ctrl, st.pos) {
        (true, Some((x, y))) => {
            let (bx, by) =
                view.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
            // only a character under the pointer counts, not the empty space right of a line
            view.iter_at_location(bx, by).filter(|it| {
                let r = view.iter_location(it);
                bx >= r.x()
                    && bx < r.x() + r.width().max(1)
                    && by >= r.y()
                    && by < r.y() + r.height()
            })
        }
        _ => None,
    }
    .map(|it| it.offset());
    let key = (st.ctrl, off.unwrap_or(-1));
    if st.last == Some(key) {
        return;
    }
    st.last = Some(key);
    let range = off.and_then(|o| target_range(&view.buffer(), o));
    show(view, st, range);
}

fn is_ctrl(key: gdk::Key) -> bool {
    matches!(key, gdk::Key::Control_L | gdk::Key::Control_R)
}

pub(super) fn install(view: &sv::View) {
    let st: Rc<RefCell<State>> = Rc::default();
    let motion = gtk::EventControllerMotion::new();
    motion.set_propagation_phase(gtk::PropagationPhase::Capture);
    let (v, s) = (view.clone(), st.clone());
    motion.connect_motion(move |m, x, y| {
        let mut st = s.borrow_mut();
        st.pos = Some((x, y));
        let state = m.current_event_state();
        st.ctrl = state.contains(gdk::ModifierType::CONTROL_MASK)
            && !state.intersects(gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::ALT_MASK);
        update(&v, &mut st);
    });
    let (v, s) = (view.clone(), st.clone());
    motion.connect_leave(move |_| {
        let mut st = s.borrow_mut();
        st.pos = None;
        update(&v, &mut st);
    });
    view.add_controller(motion);
    // The modifier state a key event carries is the one before it (a press of Ctrl still says "no Ctrl", its
    // release still says "Ctrl"), so the key itself is what counts; a motion event knows the true state.
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let (v, s) = (view.clone(), st.clone());
    keys.connect_key_pressed(move |_, key, _, _| {
        if is_ctrl(key) {
            let mut st = s.borrow_mut();
            st.ctrl = true;
            update(&v, &mut st);
        }
        glib::Propagation::Proceed
    });
    let (v, s) = (view.clone(), st.clone());
    keys.connect_key_released(move |_, key, _, _| {
        if is_ctrl(key) {
            let mut st = s.borrow_mut();
            st.ctrl = false;
            update(&v, &mut st);
        }
    });
    view.add_controller(keys);
    // the text is another one after a file switch or an edit: the next pass looks again
    let (v, s) = (view.clone(), st);
    view.connect_buffer_notify(move |_| {
        let mut st = s.borrow_mut();
        st.shown = None;
        st.last = None;
        update(&v, &mut st);
    });
}

impl Editor {
    /// Dev hooks `code:hover=<text>|N` (as Ctrl+hover at that character) and `code:hover=none`.
    pub(super) fn dev_hover(&mut self, c: &str) -> bool {
        let Some(a) = c.strip_prefix("hover=") else {
            return false;
        };
        let mut st = State::default();
        let all = self.w.view.buffer();
        all.remove_tag_by_name(TAG_CTRL, &all.start_iter(), &all.end_iter());
        let range = if a == "none" {
            None
        } else {
            self.dev_offset(a)
                .and_then(|(off, _)| target_range(&self.w.view.buffer(), off))
        };
        show(&self.w.view, &mut st, range);
        let buf = self.w.view.buffer();
        let target = range
            .map(|(s, e)| {
                buf.text(&buf.iter_at_offset(s), &buf.iter_at_offset(e), false)
                    .to_string()
            })
            .unwrap_or_default();
        let underlined = buf
            .tag_table()
            .lookup(TAG_CTRL)
            .and_then(|tag| {
                let mut it = buf.start_iter();
                let mut start = None;
                loop {
                    if it.has_tag(&tag) {
                        start.get_or_insert(it.offset());
                    } else if let Some(s) = start.take() {
                        return Some(format!("{s}..{}", it.offset()));
                    }
                    if !it.forward_char() {
                        return start.map(|s| format!("{s}..{}", it.offset()));
                    }
                }
            })
            .unwrap_or_default();
        eprintln!(
            "editor_code: hover target=[{target}] underlined={underlined} cursor={}",
            self.w
                .view
                .cursor()
                .and_then(|c| c.name())
                .map(|n| n.to_string())
                .unwrap_or_else(|| "default".into())
        );
        true
    }
}
