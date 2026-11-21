//! The two pieces of typed data the editor hangs on its widgets: a file path (`String`) on file rows and
//! pages, and an id (`u64`) on checkpoint rows. GObject's `data`/`set_data` are `unsafe` because the stored
//! type is not checked; keeping every use behind these four functions means one type per key, in one place.

use relm4::gtk::glib;
use relm4::gtk::prelude::*;

const PATH: &str = "path";
const ID: &str = "id";

/// Remembers a book-relative path on `w`.
pub fn set_path(w: &impl IsA<glib::Object>, path: impl Into<String>) {
    // SAFETY: under the key `path` this module only ever stores a `String` (here) and reads a `String` (`path`).
    unsafe { w.set_data(PATH, path.into()) };
}

/// The path remembered by [`set_path`], if any.
pub fn path(w: &impl IsA<glib::Object>) -> Option<String> {
    // SAFETY: see `set_path`: the value under `path` is always a `String`.
    unsafe { w.data::<String>(PATH).map(|p| p.as_ref().clone()) }
}

/// Remembers an id on `w`.
pub fn set_id(w: &impl IsA<glib::Object>, id: u64) {
    // SAFETY: under the key `id` this module only ever stores a `u64` (here) and reads a `u64` (`id`).
    unsafe { w.set_data(ID, id) };
}

/// The id remembered by [`set_id`], if any.
pub fn id(w: &impl IsA<glib::Object>) -> Option<u64> {
    // SAFETY: see `set_id`: the value under `id` is always a `u64`.
    unsafe { w.data::<u64>(ID).map(|p| *p.as_ref()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_widget_remembers_its_path_and_id() {
        let o = glib::Object::new::<glib::Object>();
        assert_eq!(path(&o), None);
        set_path(&o, "OEBPS/a.xhtml");
        assert_eq!(path(&o).as_deref(), Some("OEBPS/a.xhtml"));
        set_path(&o, String::from("b"));
        assert_eq!(path(&o).as_deref(), Some("b"));
        assert_eq!(id(&o), None);
        set_id(&o, 7);
        assert_eq!(id(&o), Some(7));
        // the two keys do not collide
        assert_eq!(path(&o).as_deref(), Some("b"));
    }
}
