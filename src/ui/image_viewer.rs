//! The reader's image viewer: a click on a picture opens it large, with zoom (buttons, keys,
//! Ctrl+scroll, pinch, double click), pan (drag, scroll, arrows), *Copy* and *Save Image As…*.
//! The zoom arithmetic is `crate::image_view`.

use crate::i18n::tr;
use std::cell::RefCell;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gdk, gio, glib};

use crate::image_view::{self as iv, View};

struct Viewer {
    dialog: adw::Dialog,
    fixed: gtk::Fixed,
    /// How far the picture is moved left and up (never negative).
    offset: std::cell::Cell<(f64, f64)>,
    picture: gtk::Picture,
    percent: gtk::Label,
    view: RefCell<View>,
    texture: gdk::Texture,
    bytes: Vec<u8>,
    name: String,
    pinch_start: std::cell::Cell<f64>,
    drag_start: std::cell::Cell<(f64, f64)>,
}

thread_local! {
    static OPEN: RefCell<Option<Rc<Viewer>>> = const { RefCell::new(None) };
}

impl Viewer {
    /// Resizes the picture to the view, keeping the point under `focus` in place.
    fn apply(&self, old_scale: f64, focus: Option<(f64, f64)>) {
        let view = *self.view.borrow();
        let (w, h) = view.size();
        self.picture.set_size_request(w as i32, h as i32);
        self.percent.set_text(&format!("{}%", view.percent()));
        let focus = focus.unwrap_or((view.window.0 / 2.0, view.window.1 / 2.0));
        let to = view.scroll_after_zoom(self.offset.get(), old_scale, focus);
        self.offset.set(to);
        self.place();
    }

    /// Puts the picture where the offset says (centred when it is smaller than the window).
    fn place(&self) {
        let view = *self.view.borrow();
        let (w, h) = view.size();
        let off = view.clamp_scroll(self.offset.get());
        self.offset.set(off);
        let x = if w < view.window.0 {
            (view.window.0 - w) / 2.0
        } else {
            -off.0
        };
        let y = if h < view.window.1 {
            (view.window.1 - h) / 2.0
        } else {
            -off.1
        };
        self.fixed.move_(&self.picture, x, y);
    }

    fn zoom(&self, f: impl FnOnce(&mut View), focus: Option<(f64, f64)>) {
        let old = self.view.borrow().scale();
        f(&mut self.view.borrow_mut());
        self.apply(old, focus);
    }

    fn pan(&self, dx: f64, dy: f64) {
        let o = self.offset.get();
        self.offset.set((o.0 + dx, o.1 + dy));
        self.place();
    }

    fn copy(&self) {
        self.dialog.clipboard().set_texture(&self.texture);
    }

    fn save_to(&self, path: &std::path::Path) -> std::io::Result<()> {
        std::fs::write(path, &self.bytes)
    }

    fn save_dialog(self: &Rc<Self>) {
        let dlg = gtk::FileDialog::builder()
            .title(tr("Save Image As"))
            .initial_name(&self.name)
            .build();
        let win = self.dialog.root().and_downcast::<gtk::Window>();
        let me = self.clone();
        dlg.save(win.as_ref(), gio::Cancellable::NONE, move |res| {
            if let Some(path) = res.ok().and_then(|f| f.path()) {
                if let Err(e) = me.save_to(&path) {
                    let d = adw::AlertDialog::new(
                        Some(&tr("Could not save the image")),
                        Some(&e.to_string()),
                    );
                    d.add_response("ok", &tr("OK"));
                    d.present(Some(&me.dialog));
                }
            }
        });
    }
}

/// Opens the viewer on a picture's bytes. `None` when GTK cannot read the picture.
pub fn show(parent: &gtk::Widget, title: &str, bytes: Vec<u8>, src: &str) -> bool {
    let Ok(texture) = gdk::Texture::from_bytes(&glib::Bytes::from(&bytes)) else {
        return false;
    };
    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(title);
    dialog.set_content_width(900);
    dialog.set_content_height(560);

    let picture = gtk::Picture::for_paintable(&texture);
    picture.set_can_shrink(false);
    picture.set_content_fit(gtk::ContentFit::Fill);
    picture.set_alternative_text(Some(title));
    let fixed = gtk::Fixed::new();
    fixed.put(&picture, 0.0, 0.0);
    fixed.set_overflow(gtk::Overflow::Hidden);
    let tracker = gtk::DrawingArea::new();
    tracker.set_content_width(300);
    tracker.set_content_height(200);
    let scroll = gtk::Overlay::new();
    scroll.set_child(Some(&tracker));
    scroll.add_overlay(&fixed);
    scroll.set_overflow(gtk::Overflow::Hidden);
    scroll.set_hexpand(true);
    scroll.set_vexpand(true);
    scroll.set_focusable(true);
    super::audit::label(&scroll, &tr("Image"));
    scroll.add_css_class("image-viewer");

    let header = adw::HeaderBar::new();
    let btn = |icon: &str, tip: &str| {
        let b = gtk::Button::from_icon_name(icon);
        b.set_tooltip_text(Some(tip));
        b.add_css_class("flat");
        b
    };
    let zoom_out = btn("zoom-out-symbolic", "Zoom out (−)");
    let zoom_in = btn("zoom-in-symbolic", "Zoom in (+)");
    let fit = btn("zoom-fit-best-symbolic", "Fit the window (0)");
    let actual = btn("zoom-original-symbolic", "Actual size (1)");
    let copy = btn("edit-copy-symbolic", "Copy the image (Ctrl+C)");
    let save = btn("document-save-symbolic", "Save the image as… (Ctrl+S)");
    let percent = gtk::Label::new(Some("100%"));
    percent.add_css_class("numeric");
    percent.add_css_class("dim-label");
    for (b, n) in [
        (&zoom_out, "Zoom out"),
        (&zoom_in, "Zoom in"),
        (&fit, "Fit the window"),
        (&actual, "Actual size"),
        (&copy, "Copy image"),
        (&save, "Save image as"),
    ] {
        super::audit::label(b, n);
    }
    header.pack_start(&zoom_out);
    header.pack_start(&percent);
    header.pack_start(&zoom_in);
    header.pack_start(&fit);
    header.pack_start(&actual);
    header.pack_end(&save);
    header.pack_end(&copy);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&scroll));
    dialog.set_child(Some(&toolbar));
    dialog.set_focus(Some(&scroll));

    let view = View::new(
        (texture.width() as f64, texture.height() as f64),
        (850.0, 560.0),
    );
    let viewer = Rc::new(Viewer {
        dialog: dialog.clone(),
        fixed: fixed.clone(),
        offset: std::cell::Cell::new((0.0, 0.0)),
        picture,
        percent,
        view: RefCell::new(view),
        texture,
        name: iv::name_for(src, &bytes),
        bytes,
        pinch_start: std::cell::Cell::new(1.0),
        drag_start: std::cell::Cell::new((0.0, 0.0)),
    });
    viewer.apply(1.0, None);

    // the visible area follows the window
    {
        let v = viewer.clone();
        tracker.connect_resize(move |_, w, h| {
            let (w, h) = (w as f64, h as f64);
            if v.view.borrow().window != (w, h) {
                v.view.borrow_mut().window = (w, h);
                let s = v.view.borrow().scale();
                v.apply(s, None);
            }
        });
    }
    {
        let v = viewer.clone();
        zoom_in.connect_clicked(move |_| v.zoom(View::zoom_in, None));
        let v = viewer.clone();
        zoom_out.connect_clicked(move |_| v.zoom(View::zoom_out, None));
        let v = viewer.clone();
        fit.connect_clicked(move |_| v.zoom(View::fit, None));
        let v = viewer.clone();
        actual.connect_clicked(move |_| v.zoom(View::actual_size, None));
        let v = viewer.clone();
        copy.connect_clicked(move |_| v.copy());
        let v = viewer.clone();
        save.connect_clicked(move |_| v.save_dialog());
    }
    // keys
    {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let v = viewer.clone();
        keys.connect_key_pressed(move |_, key, _, mods| {
            let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
            match key {
                gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add => v.zoom(View::zoom_in, None),
                gdk::Key::minus | gdk::Key::KP_Subtract => v.zoom(View::zoom_out, None),
                gdk::Key::_0 | gdk::Key::KP_0 => v.zoom(View::fit, None),
                gdk::Key::_1 | gdk::Key::KP_1 => v.zoom(View::actual_size, None),
                gdk::Key::Left => v.pan(-60.0, 0.0),
                gdk::Key::Right => v.pan(60.0, 0.0),
                gdk::Key::Up => v.pan(0.0, -60.0),
                gdk::Key::Down => v.pan(0.0, 60.0),
                gdk::Key::c if ctrl => v.copy(),
                gdk::Key::s if ctrl => v.save_dialog(),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        dialog.add_controller(keys);
    }
    // Ctrl+scroll zooms at the pointer; plain scroll pans
    {
        let sc = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        sc.set_propagation_phase(gtk::PropagationPhase::Capture);
        let v = viewer.clone();
        let sc2 = sc.clone();
        sc.connect_scroll(move |_, dx, dy| {
            let ctrl = sc2
                .current_event_state()
                .contains(gdk::ModifierType::CONTROL_MASK);
            if ctrl {
                let f = if dy < 0.0 { iv::STEP } else { 1.0 / iv::STEP };
                v.zoom(|x| x.zoom_by(f), None);
            } else {
                v.pan(dx * 40.0, dy * 40.0);
            }
            glib::Propagation::Stop
        });
        scroll.add_controller(sc);
    }
    // pinch
    {
        let g = gtk::GestureZoom::new();
        let v = viewer.clone();
        g.connect_begin(move |_, _| v.pinch_start.set(v.view.borrow().scale()));
        let v = viewer.clone();
        g.connect_scale_changed(move |_, factor| {
            let start = v.pinch_start.get();
            v.zoom(|x| x.pinch(start, factor), None);
        });
        scroll.add_controller(g);
    }
    // drag to pan, double click to toggle
    {
        let g = gtk::GestureDrag::new();
        let v = viewer.clone();
        g.connect_drag_begin(move |_, _, _| v.drag_start.set(v.offset.get()));
        let v = viewer.clone();
        g.connect_drag_update(move |_, dx, dy| {
            let (sx, sy) = v.drag_start.get();
            v.offset.set((sx - dx, sy - dy));
            v.place();
        });
        scroll.add_controller(g);
        let click = gtk::GestureClick::new();
        let v = viewer.clone();
        click.connect_pressed(move |_, n, x, y| {
            if n == 2 {
                v.zoom(View::toggle, Some((x, y)));
            }
        });
        scroll.add_controller(click);
    }
    dialog.connect_closed(|_| OPEN.with(|o| *o.borrow_mut() = None));
    OPEN.with(|o| *o.borrow_mut() = Some(viewer.clone()));
    dialog.present(Some(parent));
    true
}

/// `reader:image-viewer=…` dev commands; printed as `image_viewer: …`.
pub fn dev(cmd: &str) -> String {
    let Some(v) = OPEN.with(|o| o.borrow().clone()) else {
        return "closed".into();
    };
    let (name, arg) = cmd.split_once('=').unwrap_or((cmd, ""));
    match name {
        "in" => v.zoom(View::zoom_in, None),
        "out" => v.zoom(View::zoom_out, None),
        "fit" => v.zoom(View::fit, None),
        "actual" => v.zoom(View::actual_size, None),
        "toggle" => v.zoom(View::toggle, None),
        "pinch" => {
            let f: f64 = arg.parse().unwrap_or(1.0);
            v.pinch_start.set(1.0);
            v.zoom(|x| x.pinch(1.0, f), None);
        }
        "pan" => {
            let (dx, dy) = arg.split_once('+').unwrap_or((arg, "0"));
            v.pan(dx.parse().unwrap_or(0.0), dy.parse().unwrap_or(0.0));
        }
        "save" => {
            return format!("save {:?}", v.save_to(std::path::Path::new(arg)).is_ok());
        }
        "copy" => {
            v.copy();
            return format!("copy clipboard_has_image={}", clipboard_has_image());
        }
        "close" => {
            v.dialog.close();
            return "closed".into();
        }
        _ => {}
    }
    let view = *v.view.borrow();
    let (w, h) = view.size();
    format!(
        "{name} percent={} size={}x{} window={:.0}x{:.0} fit={} scroll={:.0},{:.0} picture={}x{} image={}x{}",
        view.percent(),
        w,
        h,
        view.window.0,
        view.window.1,
        view.mode == iv::Mode::Fit,
        v.offset.get().0,
        v.offset.get().1,
        v.picture.width(),
        v.picture.height(),
        v.texture.width(),
        v.texture.height()
    )
}

/// The clipboard holds a picture (tests).
pub fn clipboard_has_image() -> bool {
    OPEN.with(|o| {
        o.borrow().as_ref().is_some_and(|v| {
            v.dialog
                .clipboard()
                .formats()
                .contains_type(gdk::Texture::static_type())
        })
    })
}
