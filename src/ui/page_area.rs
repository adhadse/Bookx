//! `PageArea`: the drawing area of the reading canvas, with a `GtkAccessibleText` implementation.
//!
//! The page is drawn with cairo, so a screen reader sees nothing unless the widget says what is on it.
//! `PageArea` keeps the text of the visible page (paragraphs separated by line feeds) and answers
//! the AT-SPI Text interface from it. [`PageArea::set_page`] is called after every page turn.

use std::cell::RefCell;
use std::rc::Rc;

use relm4::gtk::subclass::prelude::*;
use relm4::gtk::{self, glib, prelude::*};

use crate::a11y::{PageText, Unit};

/// Where text is drawn: the canvas tells the accessible text (it owns the layout).
#[derive(Clone)]
pub struct Geometry {
    /// Rectangle (x, y, width, height) in widget coordinates of the page text range `[start, end)`.
    pub extents: Rc<dyn Fn(&PageText, u32, u32) -> Option<(f32, f32, f32, f32)>>,
    /// Page text offset under the point.
    pub offset: Rc<dyn Fn(&PageText, f32, f32) -> Option<usize>>,
}

/// UTF-8 text as the bytes GTK expects from `GtkAccessibleText`: the string and its terminating NUL.
fn c_string_bytes(mut s: String) -> glib::Bytes {
    s.retain(|c| c != '\0');
    let mut v = s.into_bytes();
    v.push(0);
    glib::Bytes::from_owned(v)
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct PageArea {
        pub geometry: RefCell<Option<Geometry>>,
        pub page: RefCell<PageText>,
        pub caret: std::cell::Cell<u32>,
        pub selection: std::cell::Cell<Option<(u32, u32)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PageArea {
        const NAME: &'static str = "BookxPageArea";
        type Type = super::PageArea;
        type ParentType = gtk::DrawingArea;
        type Interfaces = (gtk::AccessibleText,);
    }

    impl ObjectImpl for PageArea {}
    impl WidgetImpl for PageArea {}
    impl DrawingAreaImpl for PageArea {}

    impl AccessibleTextImpl for PageArea {
        fn contents(&self, start: u32, end: u32) -> Option<glib::Bytes> {
            let page = self.page.borrow();
            let len = page.len() as u32;
            // GTK hands the bytes to D-Bus as a C string: they must end with a NUL, and must never be missing
            let s = if start > len {
                String::new()
            } else {
                page.slice(start as usize, end.min(len) as usize)
            };
            Some(c_string_bytes(s))
        }

        fn contents_at(
            &self,
            offset: u32,
            granularity: gtk::AccessibleTextGranularity,
        ) -> Option<(u32, u32, glib::Bytes)> {
            let page = self.page.borrow();
            let unit = match granularity {
                gtk::AccessibleTextGranularity::Character => Unit::Char,
                gtk::AccessibleTextGranularity::Word => Unit::Word,
                gtk::AccessibleTextGranularity::Sentence => Unit::Sentence,
                gtk::AccessibleTextGranularity::Line => Unit::Line,
                gtk::AccessibleTextGranularity::Paragraph => Unit::Paragraph,
                _ => Unit::Paragraph,
            };
            let (s, e) = page.range_at(offset as usize, unit)?;
            Some((s as u32, e as u32, c_string_bytes(page.slice(s, e))))
        }

        fn caret_position(&self) -> u32 {
            self.caret.get()
        }

        fn selection(&self) -> Vec<gtk::AccessibleTextRange> {
            match self.selection.get() {
                Some((a, b)) if b > a => {
                    vec![gtk::AccessibleTextRange::new(a as usize, (b - a) as usize)]
                }
                _ => Vec::new(),
            }
        }

        // Orca asks where a range is drawn and which offset is under a point. These MUST be implemented:
        // the default calls the parent class, which a DrawingArea does not have (the app crashed with Orca running).
        fn extents(&self, start: u32, end: u32) -> Option<gtk::graphene::Rect> {
            let geo = self.geometry.borrow().clone()?;
            let (x, y, w, h) = (geo.extents)(&self.page.borrow(), start, end)?;
            Some(gtk::graphene::Rect::new(x, y, w, h))
        }

        fn offset(&self, point: &gtk::graphene::Point) -> Option<u32> {
            let geo = self.geometry.borrow().clone()?;
            (geo.offset)(&self.page.borrow(), point.x(), point.y()).map(|o| o as u32)
        }

        fn attributes(
            &self,
            _offset: u32,
        ) -> Vec<(gtk::AccessibleTextRange, glib::GString, glib::GString)> {
            // never empty: for an empty list gtk4-rs hands GTK NULL arrays, and GTK's GetAttributeRun /
            // GetAttributeValue walk them as NULL-terminated (the app crashed with Orca running)
            let len = self.page.borrow().len().max(1);
            vec![(
                gtk::AccessibleTextRange::new(0, len),
                "editable".into(),
                "false".into(),
            )]
        }

        fn default_attributes(&self) -> Vec<(glib::GString, glib::GString)> {
            vec![("editable".into(), "false".into())]
        }
    }
}

glib::wrapper! {
    pub struct PageArea(ObjectSubclass<imp::PageArea>)
        @extends gtk::DrawingArea, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::AccessibleText;
}

/// GTK 4.22 added `set_caret_position` and `set_selection` to `GtkAccessibleTextInterface`; its default implementations
/// refuse. The Rust bindings are built for GTK 4.20 (the Flatpak runtime) and cannot override them, so the caret one is
/// installed by hand, only when the running GTK has the field (4.22 or later). A screen reader's caret navigation (Orca's
/// Down arrow) needs it. Setting a selection stays refused.
unsafe extern "C" fn set_caret(
    text: *mut gtk::ffi::GtkAccessibleText,
    position: std::ffi::c_uint,
) -> glib::ffi::gboolean {
    use glib::translate::*;
    // SAFETY: GTK calls this vfunc with the instance it was installed for, a `PageArea`.
    let area: Borrowed<PageArea> =
        unsafe { from_glib_borrow(text as *mut <PageArea as ObjectType>::GlibType) };
    area.move_caret(position).into_glib()
}

fn fill_newer_vfuncs(obj: &PageArea) {
    static DONE: std::sync::Once = std::sync::Once::new();
    if gtk::minor_version() < 22 {
        return;
    }
    DONE.call_once(|| {
        use glib::translate::*;
        // SAFETY: the interface struct of a live instance belongs to the type and lives as long as the process; the
        // struct of a GTK >= 4.22 has the two fields (checked above), and the ffi struct matches that layout.
        unsafe {
            let inst = obj.as_ptr() as *mut glib::gobject_ffi::GTypeInstance;
            let iface = glib::gobject_ffi::g_type_interface_peek(
                (*inst).g_class as *mut _,
                gtk::AccessibleText::static_type().into_glib(),
            ) as *mut gtk::ffi::GtkAccessibleTextInterface;
            if !iface.is_null() {
                (*iface).set_caret_position = Some(set_caret);
            }
        }
    });
}

impl PageArea {
    pub fn new() -> Self {
        let this: Self = glib::Object::builder()
            .property("accessible-role", gtk::AccessibleRole::Document)
            .build();
        fill_newer_vfuncs(&this);
        this
    }

    /// Replaces the text a screen reader sees (after a page turn, a relayout or a new chapter).
    pub fn set_page(&self, page: PageText) {
        let imp = self.imp();
        if *imp.page.borrow() == page {
            return;
        }
        let old_len = imp.page.borrow().len() as u32;
        let new_len = page.len() as u32;
        *imp.page.borrow_mut() = page;
        imp.caret.set(0);
        imp.selection.set(None);
        self.update_contents(gtk::AccessibleTextContentChange::Remove, 0, old_len);
        self.update_contents(gtk::AccessibleTextContentChange::Insert, 0, new_len);
        self.update_caret_position();
    }

    /// A screen reader moves its caret through the page text (Orca's caret navigation): remember it and say so.
    pub fn move_caret(&self, position: u32) -> bool {
        let imp = self.imp();
        if position as usize > imp.page.borrow().len() {
            return false;
        }
        if imp.caret.get() != position {
            imp.caret.set(position);
            self.update_caret_position();
        }
        true
    }

    pub fn set_geometry(&self, g: Geometry) {
        *self.imp().geometry.borrow_mut() = Some(g);
    }

    /// The page as the screen reader sees it.
    pub fn page(&self) -> PageText {
        self.imp().page.borrow().clone()
    }

    /// The text of the page as a screen reader gets it.
    pub fn page_text(&self) -> String {
        self.imp().page.borrow().text.clone()
    }

    /// The selection, in chars of the page text (the part of it that is on the page).
    pub fn set_selection_range(&self, sel: Option<(u32, u32)>) {
        let imp = self.imp();
        if imp.selection.get() == sel {
            return;
        }
        imp.selection.set(sel);
        if let Some((a, _)) = sel {
            imp.caret.set(a);
            self.update_caret_position();
        }
        self.update_selection_bound();
    }
}

impl Default for PageArea {
    fn default() -> Self {
        Self::new()
    }
}
