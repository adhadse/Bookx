//! Cover thumbnails and placeholder colours.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use relm4::gtk::{gdk, gdk_pixbuf::Pixbuf};

thread_local! {
    static CACHE: RefCell<HashMap<(PathBuf, i32), Option<gdk::Texture>>> = RefCell::new(HashMap::new());
}

pub fn thumbnail(path: &Path, width: i32) -> Option<gdk::Texture> {
    CACHE.with(|c| {
        let key = (path.to_path_buf(), width);
        if let Some(t) = c.borrow().get(&key) {
            return t.clone();
        }
        #[allow(deprecated)]
        let tex = Pixbuf::from_file_at_scale(path, width * 2, width * 3, true)
            .ok()
            .map(|p| gdk::Texture::for_pixbuf(&p));
        if c.borrow().len() > 2000 {
            c.borrow_mut().clear();
        }
        c.borrow_mut().insert(key, tex.clone());
        tex
    })
}

pub fn forget(path: &Path) {
    CACHE.with(|c| c.borrow_mut().retain(|(p, _), _| p != path));
}

pub fn clear() {
    CACHE.with(|c| c.borrow_mut().clear());
}

/// CSS class (`cover-0` … `cover-7`) for a stable placeholder colour.
pub fn placeholder_class(title: &str) -> String {
    let h = title
        .bytes()
        .fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    format!("cover-{}", h % 8)
}
