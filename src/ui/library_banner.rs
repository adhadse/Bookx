//! The Continue Reading banner at the top of the cover grid.
//!
//! Two kinds (`library_nav::BannerKind`): the **hero** for the book in progress when the app was
//! just opened (a full-width card: big cover, "Continue reading", title, author, progress, chapter,
//! time left, a prominent Continue button and a small row of secondary actions, plus the other books
//! being read as small covers), and the **strip**, one slim row, on the visits to Home after that.
//! The layout follows the width of the book area through a few breakpoints (wide, medium, strip,
//! mini) with a cross-fade between them; the height follows the viewport and is animated when it
//! changes. Scrolling the books first shrinks the card to the strip's height and then slides it off,
//! the books stay put meanwhile. The colours come from the cover (`cover_color`).
//! Sizes come from `library_nav::banner_plan` / `banner_geometry`; this file puts them on widgets.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gdk, glib};

use crate::cover_color::{BannerColors, hex};
use crate::i18n::{tr, trf};
use crate::library_nav::{self as nav, BannerKind, Variant};

/// What the secondary actions of the banner ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BannerAct {
    Continue,
    Details,
    FromStart,
    Finish,
}

/// Size of the covers under "Also reading": big enough to hit easily with a pointer or a finger.
const CHIP_W: i32 = 72;
const CHIP_H: i32 = 108;
/// Padding around a cover inside its button (the whole button is the click target), the thin
/// progress bar under it and the gap between them.
const CHIP_PAD: i32 = 8;
const CHIP_BAR_GAP: i32 = 4;
const CHIP_BAR_H: i32 = 3;

/// The height of the "Also reading" column: its kicker (about 16 px), the gap and a chip.
#[cfg(test)]
const CHIP_COLUMN_H: i32 = 16 + 6 + 2 * CHIP_PAD + CHIP_H + CHIP_BAR_GAP + CHIP_BAR_H;

// big enough to hit with a pointer or a finger; the click target reaches 8 px beyond the cover
const _: () = assert!(CHIP_W >= 64 && CHIP_H >= 96 && CHIP_H > CHIP_W);
const _: () = assert!(CHIP_PAD >= 8 && CHIP_BAR_H <= 4);

/// What the banner shows.
#[derive(Clone)]
pub struct BannerData {
    pub id: u64,
    pub title: String,
    pub authors: String,
    pub progress: f64,
    pub cover: Option<gdk::Texture>,
    pub chapter: String,
    pub left: String,
    /// Other books being read: (id, title, cover), at most three.
    pub others: Vec<(u64, String, Option<gdk::Texture>, f64)>,
}

/// The widest the card gets on a very wide screen.
const MAX_WIDTH: i32 = 1100;

/// The banner widgets and their state.
pub struct BannerCtl {
    /// Clips the banner while it slides off; the first child of the book page.
    pub holder: gtk::ScrolledWindow,
    pub clamp: adw::Clamp,
    pub reveal: gtk::Revealer,
    /// The card.
    pub card: gtk::Box,
    /// The layouts: `wide`, `medium`, `strip`, `mini`.
    pub stack: gtk::Stack,
    /// The other books being read (shown from `CHIPS_FROM` px on).
    chips: RefCell<Vec<gtk::Widget>>,
    /// The book list: its adjustments give the room.
    scroll: gtk::ScrolledWindow,
    provider: gtk::CssProvider,
    /// How much of the banner the wheel has used up (0 = all of it visible).
    off: Cell<f64>,
    plan: Cell<Option<nav::BannerPlan>>,
    kind: Cell<BannerKind>,
    shown: Cell<bool>,
    visible_h: Cell<f64>,
    /// The height now (it moves to `plan.full` in about 200 ms when the plan changes).
    shown_full: Cell<f64>,
    anim: RefCell<Option<adw::TimedAnimation>>,
    /// No animation (e-ink mode); GTK's own setting for reduced motion is read when needed.
    pub no_motion: Cell<bool>,
    /// A viewport (height, width) that replaces the real one (the tests: Broadway windows are small).
    forced: Cell<Option<(f64, f64)>>,
}

fn animations_on() -> bool {
    gtk::Settings::default().is_some_and(|s| s.is_gtk_enable_animations())
}

impl BannerCtl {
    pub fn new(scroll: &gtk::ScrolledWindow) -> Rc<BannerCtl> {
        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.set_transition_duration(200);
        stack.set_hexpand(true);
        stack.set_vexpand(true);
        // the card is as tall as the visible layout asks, not as the tallest of them
        stack.set_vhomogeneous(false);
        stack.set_hhomogeneous(false);
        let card = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        card.add_css_class("continue-banner");
        card.append(&stack);
        let reveal = gtk::Revealer::new();
        reveal.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        reveal.set_child(Some(&card));
        let clamp = adw::Clamp::new();
        clamp.set_maximum_size(MAX_WIDTH);
        clamp.set_tightening_threshold(MAX_WIDTH);
        clamp.set_margin_start(12);
        clamp.set_margin_end(12);
        clamp.set_margin_top(12);
        clamp.set_margin_bottom(12);
        clamp.set_halign(gtk::Align::Fill);
        clamp.set_valign(gtk::Align::End);
        clamp.set_child(Some(&reveal));
        let holder = gtk::ScrolledWindow::new();
        holder.set_policy(gtk::PolicyType::Never, gtk::PolicyType::External);
        holder.set_propagate_natural_height(true);
        holder.set_min_content_height(0);
        // the books below must get the free room, not the banner
        holder.set_vexpand(false);
        clamp.set_vexpand(false);
        reveal.set_vexpand(false);
        card.set_vexpand(false);
        holder.set_child(Some(&clamp));
        let provider = gtk::CssProvider::new();
        if let Some(d) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &d,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 2,
            );
        }
        Rc::new(BannerCtl {
            holder,
            clamp,
            reveal,
            card,
            stack,
            chips: RefCell::new(Vec::new()),
            scroll: scroll.clone(),
            provider,
            off: Cell::new(0.0),
            plan: Cell::new(None),
            kind: Cell::new(BannerKind::Hero),
            shown: Cell::new(false),
            visible_h: Cell::new(0.0),
            shown_full: Cell::new(0.0),
            anim: RefCell::new(None),
            no_motion: Cell::new(false),
            forced: Cell::new(None),
        })
    }

    /// Wires the wheel and the adjustments of the book list to the banner. `mode` reads the
    /// setting (`""` adaptive, `large`, `compact`, `hidden`).
    pub fn connect(self: &Rc<Self>, page: &gtk::Box, mode: Rc<dyn Fn() -> String>) {
        let ctl = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        ctl.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let me = self.clone();
            ctl.connect_scroll(move |_, _, dy| {
                if me.wheel(dy * 40.0) {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            });
        }
        page.add_controller(ctl);
        let (vadj, hadj) = (self.scroll.vadjustment(), self.scroll.hadjustment());
        {
            // keys and the scroll bar move the books: the banner is out of the way by then
            let me = self.clone();
            vadj.connect_value_changed(move |a| {
                let full = me.full();
                if a.value() > 0.5 && me.off.get() < full {
                    me.set_offset(full);
                }
            });
        }
        // the room changed (window, sidebar, details panel): plan again
        // (not in the signal itself: it fires while the book list is being allocated, and changing the size of the
        // banner then leaves it without an allocation when the frame is drawn: "Trying to snapshot GtkScrolledWindow
        // without a current allocation")
        let pending = Rc::new(Cell::new(false));
        for adj in [vadj, hadj] {
            let (me, mode, pending) = (self.clone(), mode.clone(), pending.clone());
            adj.connect_changed(move |_| {
                if pending.replace(true) {
                    return;
                }
                let (me, mode, pending) = (me.clone(), mode.clone(), pending.clone());
                glib::idle_add_local_full(glib::Priority::HIGH_IDLE, move || {
                    pending.set(false);
                    me.replan(&mode());
                    glib::ControlFlow::Break
                });
            });
        }
    }

    /// Sets the colours of the card (from the cover).
    pub fn set_colors(&self, c: Option<BannerColors>) {
        match c {
            None => {
                self.card.remove_css_class("themed");
                self.provider.load_from_string("");
            }
            Some(c) => {
                let (bg1, bg2, fg) = (hex(c.bg1), hex(c.bg2), hex(c.fg));
                self.provider.load_from_string(&format!(
                    ".continue-banner.themed {{ background-image: linear-gradient(135deg, {bg1}, {bg2}); color: {fg}; }}\n\
                     .continue-banner.themed .banner-kicker, .continue-banner.themed .banner-author, .continue-banner.themed .banner-done {{ color: alpha({fg}, .78); }}\n\
                     .continue-banner.themed .banner-bar trough {{ background: alpha({fg}, .22); }}\n\
                     .continue-banner.themed .banner-bar progress {{ background: {fg}; }}\n\
                     .continue-banner.themed button.banner-continue {{ background: {fg}; color: {bg1}; box-shadow: none; }}\n\
                     .continue-banner.themed button.banner-secondary {{ background: alpha({fg}, .14); color: {fg}; box-shadow: none; }}\n\
                     .continue-banner.themed button.banner-chip {{ background: alpha({fg}, .1); padding: 0; }}\n"
                ));
                self.card.add_css_class("themed");
            }
        }
    }

    /// Puts what the banner shows into its layouts.
    pub fn set_data(&self, d: &BannerData, act: Rc<dyn Fn(u64, BannerAct)>) {
        for name in ["wide", "medium", "strip", "mini"] {
            if let Some(c) = self.stack.child_by_name(name) {
                self.stack.remove(&c);
            }
        }
        self.chips.borrow_mut().clear();
        for (name, variant) in [
            ("wide", Variant::Wide),
            ("medium", Variant::Medium),
            ("strip", Variant::Strip),
            ("mini", Variant::Mini),
        ] {
            let page = page(d, variant, &act, &self.chips);
            self.stack.add_named(&page, Some(name));
        }
        // a click on the card (not on a button) continues reading
        for c in self.card.observe_controllers().into_iter().flatten() {
            if let Ok(g) = c.downcast::<gtk::GestureClick>() {
                self.card.remove_controller(&g);
            }
        }
        let click = gtk::GestureClick::new();
        let (act, id) = (act, d.id);
        click.connect_released(move |_, _, _, _| act(id, BannerAct::Continue));
        self.card.add_controller(click);
        self.card.set_cursor_from_name(Some("pointer"));
    }

    /// The height the banner has when the books are at the top (0 when not shown).
    pub fn full(&self) -> f64 {
        match (self.shown.get(), self.plan.get()) {
            (true, Some(p)) => p.full,
            _ => 0.0,
        }
    }

    /// Shows or hides the banner (a book to continue, the grid is the view, nothing searched).
    pub fn set_shown(self: &Rc<Self>, on: bool, kind: BannerKind, mode: &str) {
        self.shown.set(on);
        self.kind.set(kind);
        self.reveal.set_reveal_child(on);
        self.off.set(0.0);
        self.replan_inner(mode, true);
    }

    /// The viewport of the books and the banner together: (height, width).
    fn viewport(&self) -> (f64, f64) {
        if let Some(v) = self.forced.get() {
            return v;
        }
        let h = self.scroll.vadjustment().page_size() + self.visible_h.get();
        (h, self.scroll.hadjustment().page_size())
    }

    pub fn replan(self: &Rc<Self>, mode: &str) {
        self.replan_inner(mode, false);
    }

    fn replan_inner(self: &Rc<Self>, mode: &str, force: bool) {
        let (vh, vw) = self.viewport();
        if !self.shown.get() {
            self.plan.set(None);
            self.stop_animation();
            self.shown_full.set(0.0);
            self.apply();
            return;
        }
        if vh < 1.0 && !force {
            return;
        }
        let plan = if vh >= 1.0 {
            nav::banner_plan(mode, self.kind.get(), vh, vw)
        } else {
            self.plan.get()
        };
        if plan == self.plan.get() && !force {
            return;
        }
        self.plan.set(plan);
        self.off
            .set(self.off.get().min(plan.map_or(0.0, |p| p.full)));
        let to = plan.map_or(0.0, |p| p.full);
        let from = self.shown_full.get();
        if force
            || from < 1.0
            || (from - to).abs() < 1.0
            || self.no_motion.get()
            || !animations_on()
        {
            self.stop_animation();
            self.shown_full.set(to);
            self.apply();
            return;
        }
        // the height moves smoothly (about 200 ms) when the room changes
        self.stop_animation();
        let me = self.clone();
        let target = adw::CallbackAnimationTarget::new(move |v| {
            me.shown_full.set(v);
            me.apply();
        });
        let anim = adw::TimedAnimation::builder()
            .widget(&self.card)
            .value_from(from)
            .value_to(to)
            .duration(200)
            .easing(adw::Easing::EaseOutCubic)
            .target(&target)
            .build();
        anim.play();
        *self.anim.borrow_mut() = Some(anim);
        self.apply();
    }

    fn stop_animation(&self) {
        if let Some(a) = self.anim.borrow_mut().take() {
            a.skip();
        }
    }

    /// The wheel moves the banner before the books. True when the banner used the scroll.
    pub fn wheel(&self, delta: f64) -> bool {
        let at_top = self.scroll.vadjustment().value() <= 0.5;
        let (n, used) = nav::banner_scroll(self.off.get(), self.full(), at_top, delta);
        if used {
            self.set_offset(n);
        }
        used
    }

    pub fn set_offset(&self, off: f64) {
        self.off.set(off);
        self.apply();
    }

    /// Puts the plan and the scroll state on the widgets.
    fn apply(&self) {
        let Some(plan) = self.plan.get().filter(|_| self.shown.get()) else {
            self.holder.set_visible(false);
            self.visible_h.set(0.0);
            return;
        };
        let off = self.off.get();
        // the animated height replaces the plan's while it moves
        let eff = nav::BannerPlan {
            full: self.shown_full.get().max(plan.slim.min(plan.full)),
            ..plan
        };
        let (card_h, vis_h, slide) = nav::banner_geometry(eff, off);
        // the strip once the books are scrolled (the banner is shrinking), else the planned layout
        let variant = if off > 0.5 && plan.hero() {
            if self.viewport().1 >= nav::STRIP_FROM {
                Variant::Strip
            } else {
                Variant::Mini
            }
        } else {
            plan.variant
        };
        self.stack.set_visible_child_name(match variant {
            Variant::Wide => "wide",
            Variant::Medium => "medium",
            Variant::Strip => "strip",
            Variant::Mini => "mini",
        });
        let (_, vw) = self.viewport();
        for c in self.chips.borrow().iter() {
            c.set_visible(variant == Variant::Wide && vw >= nav::CHIPS_FROM);
        }
        // the plan counts the whole footprint; the clamp has 12 px of margin around the card
        self.card
            .set_height_request((card_h - 24.0).max(48.0).round() as i32);
        self.visible_h.set(vis_h);
        self.holder.set_visible(vis_h > 0.5);
        self.holder.set_max_content_height(vis_h.round() as i32);
        self.holder.vadjustment().set_value(slide);
    }

    /// Plans for this viewport instead of the real one (test command).
    pub fn force_viewport(self: &Rc<Self>, v: Option<(f64, f64)>, mode: &str) {
        self.forced.set(v);
        self.replan_inner(mode, true);
    }

    /// `shown=… kind=… variant=… card=… visible=… full=… viewport=HxW percent=… width=…` for the test commands.
    pub fn describe(&self) -> String {
        let (vh, vw) = self.viewport();
        let (card, vis) = match self.plan.get() {
            Some(p) => {
                let (c, v, _) = nav::banner_geometry(
                    nav::BannerPlan {
                        full: self.shown_full.get().max(p.slim.min(p.full)),
                        ..p
                    },
                    self.off.get(),
                );
                (c, v)
            }
            None => (0.0, 0.0),
        };
        let shown = self.shown.get() && self.holder.is_visible();
        let full = self.full();
        format!(
            "shown={shown} kind={:?} variant={} card={card:.0} visible={vis:.0} full={full:.0} viewport={vh:.0}x{vw:.0} percent={:.1} off={:.0} card_w={} area_w={} card_h={}",
            self.kind.get(),
            self.stack.visible_child_name().unwrap_or_default(),
            if vh > 0.0 { 100.0 * full / vh } else { 0.0 },
            self.off.get(),
            self.card.width(),
            self.clamp.width(),
            self.card.height()
        ) + self.describe_chips().as_str()
    }

    fn chip_buttons(&self) -> Vec<gtk::Widget> {
        fn collect(w: &gtk::Widget, out: &mut Vec<gtk::Widget>) {
            let mut c = w.first_child();
            while let Some(ch) = c {
                if ch.has_css_class("banner-chip") {
                    out.push(ch.clone());
                }
                collect(&ch, out);
                c = ch.next_sibling();
            }
        }
        let mut out = Vec::new();
        for col in self.chips.borrow().iter() {
            collect(col, &mut out);
        }
        out
    }

    /// Puts the keyboard focus on the first "Also reading" cover (test command).
    pub fn focus_chip(&self) -> bool {
        self.chip_buttons().first().is_some_and(|b| b.grab_focus())
    }

    /// ` chips=N chip=WxH col_in=true|false`: the visible "Also reading" buttons, the first one's
    /// size and whether the column lies inside the card.
    fn describe_chips(&self) -> String {
        let col = self.chips.borrow().iter().find(|c| c.is_mapped()).cloned();
        let Some(col) = col else {
            return " chips=0".into();
        };
        let buttons = self.chip_buttons();
        let first = buttons
            .first()
            .map(|b| format!("{}x{}", b.width(), b.height()))
            .unwrap_or_default();
        let inside = col
            .compute_bounds(&self.card)
            .map(|b| {
                b.x() >= 0.0
                    && b.y() >= 0.0
                    && b.x() + b.width() <= self.card.width() as f32 + 0.5
                    && b.y() + b.height() <= self.card.height() as f32 + 0.5
            })
            .unwrap_or(false);
        let focused = buttons.iter().any(|b| b.has_focus());
        format!(
            " chips={} chip={first} col_in={inside} chip_focus={focused}",
            buttons.len()
        )
    }
}

// ---- the layouts ---------------------------------------------------------------------------------

fn cover_picture(tex: Option<&gdk::Texture>) -> gtk::Picture {
    let pic = gtk::Picture::new();
    pic.set_can_shrink(true);
    pic.set_content_fit(gtk::ContentFit::Contain);
    pic.add_css_class("book-cover");
    if let Some(t) = tex {
        pic.set_paintable(Some(t));
    }
    pic
}

/// A widget limited to `w` x `h` pixels (a picture would otherwise ask for its own size).
fn sized(child: &impl IsA<gtk::Widget>, w: i32, h: i32) -> adw::Clamp {
    let v = adw::Clamp::builder()
        .orientation(gtk::Orientation::Vertical)
        .maximum_size(h)
        .tightening_threshold(h)
        .child(child)
        .build();
    adw::Clamp::builder()
        .orientation(gtk::Orientation::Horizontal)
        .maximum_size(w)
        .tightening_threshold(w)
        .child(&v)
        .build()
}

fn button(
    label: &str,
    class: &str,
    act: &Rc<dyn Fn(u64, BannerAct)>,
    id: u64,
    what: BannerAct,
) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    b.add_css_class(class);
    b.set_valign(gtk::Align::Center);
    let act = act.clone();
    b.connect_clicked(move |_| act(id, what));
    b
}

/// The more actions of the medium layout in one menu button.
fn more_menu(act: &Rc<dyn Fn(u64, BannerAct)>, id: u64) -> gtk::MenuButton {
    let m = gtk::MenuButton::new();
    m.set_icon_name("view-more-symbolic");
    m.add_css_class("banner-secondary");
    m.set_valign(gtk::Align::Center);
    let tip = tr("More");
    m.set_tooltip_text(Some(&tip));
    super::audit::label(&m, &tip);
    let group = gtk::gio::SimpleActionGroup::new();
    let menu = gtk::gio::Menu::new();
    for (name, label, what) in [
        ("details", tr("Details"), BannerAct::Details),
        ("from-start", tr("Open from Start"), BannerAct::FromStart),
        ("finish", tr("Mark as Finished"), BannerAct::Finish),
    ] {
        let a = gtk::gio::SimpleAction::new(name, None);
        let act = act.clone();
        a.connect_activate(move |_, _| act(id, what));
        group.add_action(&a);
        menu.append(Some(&label), Some(&format!("more.{name}")));
    }
    m.insert_action_group("more", Some(&group));
    m.set_menu_model(Some(&menu));
    m
}

fn label(text: &str, classes: &[&str], align: gtk::Align) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    for c in classes {
        l.add_css_class(c);
    }
    l.set_halign(align);
    l.set_xalign(0.0);
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    l
}

/// One layout of the card.
fn page(
    d: &BannerData,
    variant: Variant,
    act: &Rc<dyn Fn(u64, BannerAct)>,
    chips: &RefCell<Vec<gtk::Widget>>,
) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.set_hexpand(true);
    let pct = trf("{}% completed", &[&format!("{:.0}", d.progress * 100.0)]);
    let bar = |w: i32| {
        let b = gtk::ProgressBar::new();
        b.set_fraction(d.progress);
        b.set_valign(gtk::Align::Center);
        b.set_width_request(w);
        b.add_css_class("banner-bar");
        b
    };
    match variant {
        Variant::Wide | Variant::Medium => {
            let wide = variant == Variant::Wide;
            row.set_spacing(if wide { 28 } else { 20 });
            row.set_valign(gtk::Align::Fill);
            let frame = gtk::AspectFrame::new(0.5, 0.5, 2.0 / 3.0, false);
            frame.set_child(Some(&cover_picture(d.cover.as_ref())));
            frame.set_vexpand(true);
            frame.set_halign(gtk::Align::Start);
            frame.set_margin_top(2);
            frame.set_margin_bottom(2);
            row.append(&frame);
            let text = gtk::Box::new(gtk::Orientation::Vertical, if wide { 6 } else { 4 });
            text.set_valign(gtk::Align::Center);
            text.set_hexpand(true);
            text.append(&label(
                &tr("Continue reading"),
                &["caption-heading", "banner-kicker"],
                gtk::Align::Start,
            ));
            let t = label(
                &d.title,
                &[if wide { "title-1" } else { "title-2" }],
                gtk::Align::Start,
            );
            t.set_wrap(true);
            t.set_lines(2);
            text.append(&t);
            text.append(&label(
                &d.authors,
                &["title-4", "banner-author"],
                gtk::Align::Start,
            ));
            let meta = [d.chapter.as_str(), d.left.as_str(), pct.as_str()]
                .iter()
                .filter(|s| !s.is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join(" · ");
            let m = label(&meta, &["caption", "banner-done"], gtk::Align::Start);
            m.set_margin_top(4);
            text.append(&m);
            let b = bar(if wide { 360 } else { 220 });
            b.set_halign(gtk::Align::Start);
            text.append(&b);
            let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            actions.set_margin_top(if wide { 10 } else { 6 });
            let go = button(
                &tr("Continue"),
                "banner-continue",
                act,
                d.id,
                BannerAct::Continue,
            );
            go.add_css_class("pill");
            go.add_css_class("suggested-action");
            actions.append(&go);
            if wide {
                for (l, w) in [
                    (tr("Details"), BannerAct::Details),
                    (tr("Open from Start"), BannerAct::FromStart),
                    (tr("Mark as Finished"), BannerAct::Finish),
                ] {
                    let b = button(&l, "banner-secondary", act, d.id, w);
                    b.add_css_class("pill");
                    actions.append(&b);
                }
            } else {
                actions.append(&more_menu(act, d.id));
            }
            text.append(&actions);
            row.append(&text);
            if wide && !d.others.is_empty() {
                // the other books being read: small covers that open on a click
                let col = gtk::Box::new(gtk::Orientation::Vertical, 6);
                col.set_valign(gtk::Align::Center);
                col.set_halign(gtk::Align::End);
                col.append(&label(
                    &tr("Also reading"),
                    &["caption-heading", "banner-kicker"],
                    gtk::Align::Start,
                ));
                let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                for (id, title, tex, progress) in &d.others {
                    let b = gtk::Button::new();
                    b.add_css_class("banner-chip");
                    b.add_css_class("flat");
                    let inner = gtk::Box::new(gtk::Orientation::Vertical, CHIP_BAR_GAP);
                    let p: gtk::Widget = match tex {
                        Some(_) => sized(&cover_picture(tex.as_ref()), CHIP_W, CHIP_H).upcast(),
                        None => {
                            // a book without a cover: its title on the colour the grid uses
                            let l = gtk::Label::new(Some(title));
                            l.set_wrap(true);
                            l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                            l.set_lines(4);
                            l.set_justify(gtk::Justification::Center);
                            l.set_max_width_chars(8);
                            l.add_css_class("caption");
                            l.add_css_class("placeholder-title");
                            let f = gtk::Box::new(gtk::Orientation::Vertical, 0);
                            f.add_css_class("book-cover");
                            f.add_css_class(&super::covers::placeholder_class(title));
                            l.set_valign(gtk::Align::Center);
                            l.set_vexpand(true);
                            f.append(&l);
                            f.upcast()
                        }
                    };
                    p.set_size_request(CHIP_W, CHIP_H);
                    p.set_halign(gtk::Align::Center);
                    inner.append(&p);
                    let bar = gtk::ProgressBar::new();
                    bar.set_fraction(*progress);
                    bar.add_css_class("banner-bar");
                    bar.set_can_target(false);
                    bar.set_size_request(CHIP_W, -1);
                    inner.append(&bar);
                    inner.set_margin_top(CHIP_PAD);
                    inner.set_margin_bottom(CHIP_PAD);
                    inner.set_margin_start(CHIP_PAD);
                    inner.set_margin_end(CHIP_PAD);
                    b.set_child(Some(&inner));
                    let pct = trf("{}% completed", &[&format!("{:.0}", progress * 100.0)]);
                    b.set_tooltip_text(Some(&format!("{title} · {pct}")));
                    super::audit::label(&b, &trf("Continue reading {}, {}", &[title, &pct]));
                    let (act, id) = (act.clone(), *id);
                    b.connect_clicked(move |_| act(id, BannerAct::Continue));
                    line.append(&b);
                }
                col.append(&line);
                col.set_visible(false);
                chips.borrow_mut().push(col.clone().upcast());
                row.append(&col);
            }
        }
        Variant::Strip => {
            row.set_spacing(12);
            row.set_valign(gtk::Align::Center);
            let pic = cover_picture(d.cover.as_ref());
            let p = sized(&pic, 36, 54);
            p.set_valign(gtk::Align::Center);
            row.append(&p);
            let text = gtk::Box::new(gtk::Orientation::Vertical, 1);
            text.set_valign(gtk::Align::Center);
            text.set_hexpand(true);
            text.append(&label(&d.title, &["heading"], gtk::Align::Start));
            let sub = if d.authors.is_empty() {
                pct.clone()
            } else {
                format!("{} · {:.0}%", d.authors, d.progress * 100.0)
            };
            text.append(&label(
                &sub,
                &["caption", "banner-author"],
                gtk::Align::Start,
            ));
            let b = bar(0);
            b.set_hexpand(true);
            b.set_margin_top(2);
            text.append(&b);
            row.append(&text);
            let go = button(
                &tr("Continue"),
                "banner-continue",
                act,
                d.id,
                BannerAct::Continue,
            );
            go.add_css_class("pill");
            go.add_css_class("suggested-action");
            row.append(&go);
        }
        Variant::Mini => {
            row.set_spacing(10);
            row.set_valign(gtk::Align::Center);
            let t = label(&d.title, &["heading"], gtk::Align::Start);
            t.set_hexpand(true);
            row.append(&t);
            let go = gtk::Button::from_icon_name("media-playback-start-symbolic");
            go.add_css_class("banner-continue");
            go.add_css_class("circular");
            go.add_css_class("suggested-action");
            let tip = tr("Continue");
            go.set_tooltip_text(Some(&tip));
            super::audit::label(&go, &tip);
            let (act2, id) = (act.clone(), d.id);
            go.connect_clicked(move |_| act2(id, BannerAct::Continue));
            row.append(&go);
        }
    }
    row
}

#[cfg(test)]
mod chip_tests {
    use super::*;

    #[test]
    fn the_chips_column_fits_the_smallest_hero() {
        // the card has 12 px of margin and 10 px of padding above and below the content
        let inner = nav::hero_height(nav::HERO_MIN_VH) - 24.0 - 20.0;
        assert!(
            f64::from(CHIP_COLUMN_H) <= inner,
            "{CHIP_COLUMN_H} > {inner}"
        );
        // three chips and their gaps fit the card at the width where they show
        let w = 3 * (CHIP_W + 2 * CHIP_PAD) + 2 * 10;
        assert!(f64::from(w) < nav::CHIPS_FROM / 3.0);
    }
}
