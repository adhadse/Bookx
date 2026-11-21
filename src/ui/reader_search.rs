//! The search popover of the readers: the status line, the size that fits the window and the
//! empty state shown when a finished search found nothing.

use crate::i18n::{ntr, tr};
use relm4::adw;
use relm4::gtk::{self, prelude::*};

/// More hits than this are not listed (the search stops there).
pub const MAX_HITS: usize = 400;

/// The status line under the search entry: `n` hits so far, `finished` when the search is over.
pub fn search_status(n: usize, finished: bool) -> String {
    if finished && n == 0 {
        tr("No results")
    } else if n > MAX_HITS {
        tr("More than 400 results")
    } else if finished {
        ntr("{} result", "{} results", n as u64)
    } else {
        ntr("{} result so far", "{} results so far", n as u64)
    }
}

/// Width and height of the search popover: 380 x 460 at most, narrower than the window, and no
/// taller than 55 % of it.
pub fn popover_size(window_w: i32, window_h: i32) -> (i32, i32) {
    let w = 380.min((window_w - 24).max(260));
    let h = 460.min(window_h * 55 / 100).max(200);
    (w, h)
}

/// Fits `search_box` to the window it is shown in.
pub fn fit(search_box: &gtk::Box) {
    if let Some(win) = super::window_of(search_box) {
        let (w, h) = popover_size(win.width(), win.height());
        search_box.set_size_request(w, h);
    }
}

/// The empty state of a finished search.
pub fn empty_page() -> adw::StatusPage {
    let page = adw::StatusPage::new();
    page.set_icon_name(Some("system-search-symbolic"));
    page.set_title(&tr("No Results"));
    page.set_description(Some(&tr("Try a different word.")));
    page.add_css_class("compact");
    page
}

/// Shows the empty state in `list` only when `show` is true.
pub fn set_empty_state(list: &gtk::ListBox, page: &adw::StatusPage, show: bool) {
    if show {
        list.set_placeholder(Some(page));
    } else {
        list.set_placeholder(gtk::Widget::NONE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_covers_every_state() {
        assert_eq!(search_status(0, true), "No results");
        assert_eq!(search_status(0, false), "0 results so far");
        assert_eq!(search_status(1, true), "1 result");
        assert_eq!(search_status(12, true), "12 results");
        assert_eq!(search_status(1, false), "1 result so far");
        assert_eq!(search_status(30, false), "30 results so far");
        assert_eq!(search_status(401, true), "More than 400 results");
    }

    #[test]
    fn the_popover_fits_small_windows() {
        assert_eq!(popover_size(1200, 900), (380, 460));
        assert_eq!(popover_size(900, 600), (380, 330));
        let (w, h) = popover_size(360, 640);
        assert!((260..=360 - 24 + 1).contains(&w));
        assert!(h <= 640 * 55 / 100);
        assert!(popover_size(100, 100).1 >= 200);
    }
}
