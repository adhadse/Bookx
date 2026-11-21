//! A book card with the look of the library's cover grid: a 2:3 cover, a title of at most two lines
//! (ellipsis at the end) and one dim line below it, 10 px round the card.
//!
//! The Devices window uses it for both of its lists. The sizing rules are plain functions so they are
//! unit tested; the library keeps its own card (`library.rs`) and may adopt this builder later.

use relm4::gtk::{self, prelude::*};

/// The library's default cover width (`Config::cover_size`, tested against it).
pub const LIBRARY_COVER: i32 = 150;
/// The margin round a card, the library's `root.set_margin_all(10)`.
pub const CARD_MARGIN: i32 = 10;
/// The gap between a cover and its caption, and between the caption lines.
pub const CARD_SPACING: i32 = 6;
/// Cards narrower than this stop being readable.
const MIN_COVER: i32 = 96;

/// The cover width of a card in the dialog: the library's cover size, but not wider than the library's
/// default (three cards must fit in the 640 px dialog) and not narrower than a readable cover.
pub fn cover_width(library_cover_size: i32) -> i32 {
    library_cover_size.clamp(MIN_COVER, LIBRARY_COVER)
}

/// Covers are 2:3, as in the library.
pub fn cover_height(width: i32) -> i32 {
    width * 3 / 2
}

/// How many characters a caption line shows before it is cut: the library's `size / 9`, at least 8.
pub fn caption_chars(cover: i32) -> i32 {
    (cover / 9).max(8)
}

/// The width a card takes in a row of cards, margins included.
pub fn card_width(cover: i32) -> i32 {
    cover + 2 * CARD_MARGIN
}

/// What a card shows.
pub struct Spec<'a> {
    pub title: &'a str,
    /// The dim line (author, format).
    pub detail: &'a str,
    /// Seed of the placeholder colour.
    pub seed: &'a str,
    pub tooltip: &'a str,
    pub warning: Option<&'a str>,
}

/// The widgets of one card.
pub struct Tile {
    pub root: gtk::Box,
    pub picture: gtk::Picture,
    pub caption: gtk::Label,
}

/// Builds a card; `check` is laid over the cover's top-left corner.
pub fn build(spec: &Spec, cover: i32, check: &gtk::CheckButton) -> Tile {
    let root = gtk::Box::new(gtk::Orientation::Vertical, CARD_SPACING);
    root.set_margin_top(CARD_MARGIN);
    root.set_margin_bottom(CARD_MARGIN);
    root.set_margin_start(CARD_MARGIN);
    root.set_margin_end(CARD_MARGIN);
    root.set_halign(gtk::Align::Center);
    root.set_valign(gtk::Align::Start);
    root.set_focusable(true);
    root.add_css_class("pick-tile");
    root.add_css_class("selection-mode");
    if !spec.tooltip.is_empty() {
        root.set_tooltip_text(Some(spec.tooltip));
    }
    let (cw, ch) = (cover, cover_height(cover));

    let overlay = gtk::Overlay::new();
    overlay.add_css_class("book-cover");
    overlay.set_overflow(gtk::Overflow::Hidden);
    overlay.set_halign(gtk::Align::Center);
    let fallback = gtk::Box::new(gtk::Orientation::Vertical, 0);
    fallback.set_size_request(cw, ch);
    fallback.add_css_class(&super::covers::placeholder_class(spec.seed));
    let fallback_label = gtk::Label::new(Some(spec.title));
    fallback_label.set_wrap(true);
    fallback_label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    fallback_label.set_justify(gtk::Justification::Center);
    fallback_label.set_valign(gtk::Align::Center);
    fallback_label.set_vexpand(true);
    fallback_label.set_margin_start(10);
    fallback_label.set_margin_end(10);
    fallback_label.set_max_width_chars(14);
    fallback_label.add_css_class("cover-title");
    fallback.append(&fallback_label);
    overlay.set_child(Some(&fallback));
    let picture = gtk::Picture::new();
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.set_can_shrink(true);
    picture.set_size_request(cw, ch);
    overlay.add_overlay(&picture);
    overlay.add_overlay(check);
    if let Some(w) = spec.warning {
        let icon = gtk::Image::from_icon_name("dialog-warning-symbolic");
        icon.add_css_class("osd");
        icon.add_css_class("warning");
        icon.set_halign(gtk::Align::End);
        icon.set_valign(gtk::Align::Start);
        icon.set_margin_top(6);
        icon.set_margin_end(6);
        icon.set_tooltip_text(Some(w));
        overlay.add_overlay(&icon);
    }

    let caption = gtk::Label::new(Some(spec.title));
    caption.set_ellipsize(gtk::pango::EllipsizeMode::End);
    caption.set_lines(2);
    caption.set_wrap(true);
    caption.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    caption.set_justify(gtk::Justification::Center);
    caption.set_width_chars(1);
    caption.set_max_width_chars(caption_chars(cover));
    caption.add_css_class("heading");
    let detail = gtk::Label::new(Some(spec.detail));
    detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
    detail.set_width_chars(1);
    detail.set_max_width_chars(caption_chars(cover));
    detail.add_css_class("dim-label");
    detail.add_css_class("caption");
    detail.set_visible(!spec.detail.is_empty());

    root.append(&overlay);
    root.append(&caption);
    root.append(&detail);
    Tile {
        root,
        picture,
        caption,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_cover_matches_the_config_default() {
        assert_eq!(LIBRARY_COVER, crate::config::Config::default().cover_size);
    }

    #[test]
    fn cover_width_follows_the_library_within_bounds() {
        assert_eq!(cover_width(150), 150);
        assert_eq!(cover_width(300), 150, "never wider than the default");
        assert_eq!(cover_width(40), 96, "never unreadable");
        assert_eq!(cover_width(120), 120);
    }

    #[test]
    fn covers_are_two_by_three() {
        assert_eq!(cover_height(150), 225);
        assert_eq!(cover_height(96), 144);
    }

    #[test]
    fn captions_cut_like_the_library() {
        assert_eq!(caption_chars(150), 16);
        assert_eq!(caption_chars(40), 8);
    }

    #[test]
    fn three_cards_fit_the_640px_dialog() {
        // the dialog is 640 wide with 12 px margins each side: three cards of the library's size fit, four do not
        let avail = 640 - 24;
        assert!(3 * card_width(cover_width(150)) <= avail);
        assert!(4 * card_width(cover_width(150)) > avail);
    }

    #[test]
    fn card_width_adds_the_margins() {
        assert_eq!(card_width(150), 170);
    }
}
