//! A vertical stepper for setups with several steps (like the Material “vertical, editable,
//! non-linear stepper”, built from GTK widgets because libadwaita has none): a numbered round badge
//! per step (a check mark when the step is done), the title with a short status, a connector line
//! to the next step, and the step’s content under the title, shown for one step at a time. Every
//! step can be opened (non-linear); when nobody picked one, the first step that is not done is open.
//! The headers are buttons, so Tab, Enter and Space work; each has a stable accessible name
//! (“Step 2 of 4: Create your sync account, current, alice”). Colours are libadwaita’s named colours,
//! so light, dark and high contrast follow the theme. The state rules are pure functions (tested
//! below); the widget only draws them.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use relm4::adw::prelude::*;
use relm4::gtk;

use crate::i18n::{tr, trf};

/// What the user picked: nothing yet (the first open step shows), one step, or all closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picked {
    Auto,
    Step(usize),
    Closed,
}

/// How a step looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepState {
    Done,
    /// Open: its content is shown.
    Active,
    Pending,
}

impl StepState {
    pub fn key(self) -> &'static str {
        match self {
            StepState::Done => "done",
            StepState::Active => "active",
            StepState::Pending => "pending",
        }
    }
}

/// The step that is open: the picked one, or (nothing picked) the first that is not done.
pub fn active_index(done: &[bool], picked: Picked) -> Option<usize> {
    match picked {
        Picked::Closed => None,
        Picked::Step(i) if i < done.len() => Some(i),
        _ => done.iter().position(|d| !d),
    }
}

/// The look of every step: done steps stay marked done even when open.
pub fn states(done: &[bool], active: Option<usize>) -> Vec<StepState> {
    done.iter()
        .enumerate()
        .map(|(i, d)| {
            if *d {
                StepState::Done
            } else if Some(i) == active {
                StepState::Active
            } else {
                StepState::Pending
            }
        })
        .collect()
}

/// Is a step drawn open? (Open done steps stay “done” in colour but show their content.)
pub fn is_open(i: usize, active: Option<usize>) -> bool {
    active == Some(i)
}

/// The connector line runs from every step to the next, not after the last.
pub fn connector_visible(i: usize, total: usize) -> bool {
    i + 1 < total
}

/// What a screen reader says for a header.
pub fn accessible_name(
    i: usize,
    total: usize,
    title: &str,
    state: StepState,
    status: &str,
) -> String {
    let state = match state {
        StepState::Done => tr("done"),
        StepState::Active => tr("current"),
        StepState::Pending => tr("not done"),
    };
    let mut s: String = trf("Step {} of {}: {}", &[&(i + 1), &total, &title]);
    s.push_str(", ");
    s.push_str(&state);
    if !status.is_empty() {
        s.push_str(", ");
        s.push_str(status);
    }
    s
}

/// What the owner of the stepper says about one step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepInfo {
    pub done: bool,
    pub status: String,
}

struct Row {
    title: String,
    header: gtk::Button,
    status: gtk::Label,
    stack: gtk::Stack,
    badge: gtk::Box,
    revealer: gtk::Revealer,
    line: gtk::Box,
}

pub struct Stepper {
    root: gtk::Box,
    rows: Vec<Row>,
    picked: Cell<Picked>,
    done: RefCell<Vec<bool>>,
    statuses: RefCell<Vec<String>>,
}

const CSS: &str = "
.stepper-badge { min-width: 28px; min-height: 28px; border-radius: 14px;
  background-color: alpha(@window_fg_color, 0.14); color: @window_fg_color; font-weight: bold; }
.stepper-badge.active { background-color: @accent_bg_color; color: @accent_fg_color; }
.stepper-badge.done { background-color: @success_bg_color; color: @success_fg_color; }
.stepper-line { min-width: 2px; background-color: alpha(@window_fg_color, 0.2); margin-top: 3px; margin-bottom: 3px; }
.stepper-header { padding: 4px 6px; border-radius: 8px; }
.stepper-title { font-weight: bold; }
";

fn ensure_css() {
    thread_local! { static DONE: Cell<bool> = const { Cell::new(false) }; }
    if DONE.with(|d| d.replace(true)) {
        return;
    }
    if let Some(display) = gtk::gdk::Display::default() {
        let p = gtk::CssProvider::new();
        p.load_from_string(CSS);
        gtk::style_context_add_provider_for_display(
            &display,
            &p,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

impl Stepper {
    /// A stepper of `steps` (title, content). `compact` puts the status beside the title (for small
    /// cards) instead of under it. Call [`Stepper::update`] to give it its state.
    pub fn new(steps: Vec<(String, gtk::Widget)>, compact: bool) -> Rc<Stepper> {
        ensure_css();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let total = steps.len();
        let mut rows = Vec::new();
        let this: Rc<RefCell<Option<std::rc::Weak<Stepper>>>> = Rc::default();
        for (i, (title, body)) in steps.into_iter().enumerate() {
            let outer = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            let left = gtk::Box::new(gtk::Orientation::Vertical, 0);
            left.set_margin_top(4);
            let stack = gtk::Stack::new();
            let num = gtk::Label::new(Some(&(i + 1).to_string()));
            let check = gtk::Image::from_icon_name("object-select-symbolic");
            stack.add_named(&num, Some("num"));
            stack.add_named(&check, Some("check"));
            let badge = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            badge.add_css_class("stepper-badge");
            badge.set_halign(gtk::Align::Center);
            badge.set_valign(gtk::Align::Start);
            badge.set_size_request(28, 28);
            stack.set_halign(gtk::Align::Center);
            stack.set_hexpand(true);
            stack.set_valign(gtk::Align::Center);
            badge.append(&stack);
            left.append(&badge);
            let line = gtk::Box::new(gtk::Orientation::Vertical, 0);
            line.add_css_class("stepper-line");
            line.set_halign(gtk::Align::Center);
            line.set_vexpand(true);
            line.set_visible(connector_visible(i, total));
            left.append(&line);
            outer.append(&left);

            let right = gtk::Box::new(gtk::Orientation::Vertical, 0);
            right.set_hexpand(true);
            let header = gtk::Button::new();
            header.add_css_class("flat");
            header.add_css_class("stepper-header");
            let text = gtk::Box::new(
                if compact {
                    gtk::Orientation::Horizontal
                } else {
                    gtk::Orientation::Vertical
                },
                if compact { 12 } else { 2 },
            );
            let t = gtk::Label::new(Some(&title));
            t.add_css_class("stepper-title");
            t.set_xalign(0.0);
            t.set_wrap(true);
            t.set_hexpand(true);
            let status = gtk::Label::new(None);
            status.add_css_class("dim-label");
            status.add_css_class("caption");
            status.set_xalign(if compact { 1.0 } else { 0.0 });
            status.set_wrap(true);
            text.append(&t);
            text.append(&status);
            header.set_child(Some(&text));
            header.set_halign(gtk::Align::Fill);
            right.append(&header);
            let revealer = gtk::Revealer::new();
            revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
            body.set_margin_start(6);
            body.set_margin_end(2);
            body.set_margin_top(4);
            body.set_margin_bottom(14);
            revealer.set_child(Some(&body));
            right.append(&revealer);
            outer.append(&right);
            root.append(&outer);

            let weak = this.clone();
            header.connect_clicked(move |_| {
                if let Some(s) = weak.borrow().as_ref().and_then(|w| w.upgrade()) {
                    s.toggle(i);
                }
            });
            rows.push(Row {
                title,
                header,
                status,
                stack,
                badge,
                revealer,
                line,
            });
        }
        let s = Rc::new(Stepper {
            root,
            picked: Cell::new(Picked::Auto),
            done: RefCell::new(vec![false; total]),
            statuses: RefCell::new(vec![String::new(); total]),
            rows,
        });
        *this.borrow_mut() = Some(Rc::downgrade(&s));
        s.render();
        s
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Gives every step its state and status text and redraws.
    pub fn update(&self, infos: &[StepInfo]) {
        {
            let mut d = self.done.borrow_mut();
            let mut s = self.statuses.borrow_mut();
            for (i, info) in infos.iter().enumerate().take(d.len()) {
                d[i] = info.done;
                s[i] = info.status.clone();
            }
        }
        self.render();
    }

    /// Starts with every step closed (small cards).
    pub fn close_all(&self) {
        self.picked.set(Picked::Closed);
        self.render();
    }

    /// Opens step `i` (the user clicked it, or a test did).
    pub fn pick(&self, i: usize) {
        self.picked.set(Picked::Step(i));
        self.render();
    }

    /// A click on a header opens that step; a click on the open one closes it.
    fn toggle(&self, i: usize) {
        let active = active_index(&self.done.borrow(), self.picked.get());
        self.picked.set(if is_open(i, active) {
            Picked::Closed
        } else {
            Picked::Step(i)
        });
        self.render();
    }

    fn render(&self) {
        let done = self.done.borrow().clone();
        let statuses = self.statuses.borrow().clone();
        let active = active_index(&done, self.picked.get());
        let total = self.rows.len();
        for (i, (row, st)) in self.rows.iter().zip(states(&done, active)).enumerate() {
            row.status.set_label(&statuses[i]);
            row.status.set_visible(!statuses[i].is_empty());
            for c in ["active", "done"] {
                row.badge.remove_css_class(c);
            }
            match st {
                StepState::Done => row.badge.add_css_class("done"),
                StepState::Active => row.badge.add_css_class("active"),
                StepState::Pending => {}
            }
            // an open done step keeps its green badge; the header shows the open state instead
            if is_open(i, active) && st == StepState::Done {
                row.badge.add_css_class("done");
            }
            row.stack.set_visible_child_name(if st == StepState::Done {
                "check"
            } else {
                "num"
            });
            row.revealer.set_reveal_child(is_open(i, active));
            row.line.set_visible(connector_visible(i, total));
            let name = accessible_name(i, total, &row.title, st, &statuses[i]);
            super::audit::label(&row.header, &name);
            row.header
                .update_state(&[gtk::accessible::State::Expanded(Some(is_open(i, active)))]);
            row.header.set_tooltip_text(Some(&name));
        }
    }

    /// For the UI tests: `steps=4 active=1 states=done,active,pending,pending statuses=…`.
    pub fn dump(&self) -> String {
        let done = self.done.borrow().clone();
        let active = active_index(&done, self.picked.get());
        let st: Vec<&str> = states(&done, active).iter().map(|s| s.key()).collect();
        let open: Vec<String> = self
            .rows
            .iter()
            .map(|r| r.revealer.reveals_child().to_string())
            .collect();
        format!(
            "steps={} active={} states={} open={} statuses={} names={}",
            self.rows.len(),
            active.map_or("none".to_string(), |a| (a + 1).to_string()),
            st.join(","),
            open.join(","),
            self.statuses.borrow().join("|"),
            self.rows
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    accessible_name(
                        i,
                        self.rows.len(),
                        &r.title,
                        states(&done, active)[i],
                        &statuses_of(&self.statuses.borrow(), i),
                    )
                })
                .collect::<Vec<_>>()
                .join("|")
        )
    }
}

fn statuses_of(s: &[String], i: usize) -> String {
    s.get(i).cloned().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_step_that_is_not_done_is_open_by_default() {
        assert_eq!(active_index(&[false, false, false], Picked::Auto), Some(0));
        assert_eq!(active_index(&[true, false, false], Picked::Auto), Some(1));
        assert_eq!(active_index(&[true, true, false], Picked::Auto), Some(2));
        assert_eq!(active_index(&[true, true, true], Picked::Auto), None);
    }

    #[test]
    fn any_step_can_be_picked_and_all_can_be_closed() {
        let done = [true, false, false];
        assert_eq!(active_index(&done, Picked::Step(0)), Some(0));
        assert_eq!(active_index(&done, Picked::Step(2)), Some(2));
        assert_eq!(active_index(&done, Picked::Closed), None);
        // a pick outside the list falls back to the automatic one
        assert_eq!(active_index(&done, Picked::Step(9)), Some(1));
    }

    #[test]
    fn states_mark_done_active_and_pending() {
        let st = states(&[true, false, false, false], Some(1));
        assert_eq!(
            st,
            [
                StepState::Done,
                StepState::Active,
                StepState::Pending,
                StepState::Pending
            ]
        );
        // an open done step stays done
        let st = states(&[true, false], Some(0));
        assert_eq!(st, [StepState::Done, StepState::Pending]);
        assert!(is_open(0, Some(0)) && !is_open(1, Some(0)) && !is_open(0, None));
    }

    #[test]
    fn the_connector_joins_steps_and_stops_after_the_last() {
        assert!(connector_visible(0, 4) && connector_visible(2, 4));
        assert!(!connector_visible(3, 4));
        assert!(!connector_visible(0, 1));
    }

    #[test]
    fn the_accessible_name_says_which_step_what_state_and_status() {
        assert_eq!(
            accessible_name(
                1,
                4,
                "Create your sync account",
                StepState::Active,
                "Not created"
            ),
            "Step 2 of 4: Create your sync account, current, Not created"
        );
        assert_eq!(
            accessible_name(0, 4, "Turn on the server", StepState::Done, ""),
            "Step 1 of 4: Turn on the server, done"
        );
        assert_eq!(
            accessible_name(3, 4, "Test", StepState::Pending, "Waiting"),
            "Step 4 of 4: Test, not done, Waiting"
        );
    }

    #[test]
    fn state_keys_are_stable() {
        assert_eq!(StepState::Done.key(), "done");
        assert_eq!(StepState::Active.key(), "active");
        assert_eq!(StepState::Pending.key(), "pending");
    }
}
