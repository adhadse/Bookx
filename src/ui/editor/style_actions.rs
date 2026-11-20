//! Text colour, background colour and alignment of the Markup menu (CE-350). The decisions are in
//! `bookx_editing::code_style`; this file asks for the colour (GtkColorDialog), moves the edit into the buffer
//! and, in class mode (View ▸ Use CSS Classes for Colours and Alignment), adds the class rule to the first style
//! sheet of the book.

use super::*;
use crate::i18n::{tr, trf};
use bookx_editing::code_format::FormatError;
use bookx_editing::code_style::{self as cs, Align, ColourKind, Via};

/// `#rrggbb` of a colour the dialog returned.
fn rgba_hex(c: &gdk::RGBA) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    cs::hex(b(c.red()), b(c.green()), b(c.blue()))
}

pub(super) fn colour_label(kind: ColourKind) -> String {
    match kind {
        ColourKind::Text => tr("Text Colour"),
        ColourKind::Background => tr("Background Colour"),
    }
}

pub(super) fn colour_off_label(kind: ColourKind) -> String {
    match kind {
        ColourKind::Text => tr("Remove Text Colour"),
        ColourKind::Background => tr("Remove Background Colour"),
    }
}

pub(super) fn align_label(a: Option<Align>) -> String {
    match a {
        Some(Align::Left) => tr("Align Left"),
        Some(Align::Center) => tr("Align Centre"),
        Some(Align::Right) => tr("Align Right"),
        Some(Align::Justify) => tr("Justify"),
        None => tr("Remove Alignment"),
    }
}

/// Menu ids of the actions: `colour-text`, `colour-background`, `colour-text-off`, `colour-background-off`,
/// `align-left` … `align-justify`, `align-off`.
pub(super) fn style_msg_for(id: &str) -> Option<super::format_actions::CodeMsg> {
    use super::format_actions::CodeMsg;
    let kind = |k: &str| match k {
        "text" => Some(ColourKind::Text),
        "background" => Some(ColourKind::Background),
        _ => None,
    };
    if let Some(k) = id.strip_prefix("colour-") {
        return match k.strip_suffix("-off") {
            Some(k) => Some(CodeMsg::ColourDo(kind(k)?, None)),
            None => Some(CodeMsg::ColourPick(kind(k)?)),
        };
    }
    let a = id.strip_prefix("align-")?;
    if a == "off" {
        return Some(CodeMsg::Align(None));
    }
    Some(CodeMsg::Align(Some(Align::from_id(a)?)))
}

impl Editor {
    /// The buffer when it holds an HTML page the style actions can edit; says why not otherwise.
    fn style_buffer(&mut self) -> Option<gtk::TextBuffer> {
        let buf = self.code_buffer()?;
        let ok = self.current.as_deref().is_some_and(|p| {
            bookx_editing::markup::is_markup_path(p)
                || matches!(extension(p).as_str(), "xml" | "svg")
        });
        if !ok {
            self.toast(&tr("Colours and alignment work in HTML pages"));
            return None;
        }
        Some(buf)
    }

    /// Asks for a colour and applies it to the selection when it comes back.
    pub(super) fn pick_colour(&mut self, kind: ColourKind) {
        if self.style_buffer().is_none() {
            return;
        }
        let dialog = gtk::ColorDialog::new();
        dialog.set_title(&match kind {
            ColourKind::Text => tr("Text Colour"),
            ColourKind::Background => tr("Background Colour"),
        });
        dialog.set_with_alpha(false);
        let tx = self.sender.input_sender().clone();
        dialog.choose_rgba(
            Some(&self.w.win),
            None::<&gdk::RGBA>,
            gio::Cancellable::NONE,
            move |r| {
                if let Ok(c) = r {
                    tx.emit(Input::Code(super::format_actions::CodeMsg::ColourDo(
                        kind,
                        Some(rgba_hex(&c)),
                    )));
                }
            },
        );
    }

    /// Sets (`Some`) or removes (`None`) the colour of the selection.
    pub(super) fn apply_colour(&mut self, kind: ColourKind, colour: Option<String>) {
        let Some(buf) = self.style_buffer() else {
            return;
        };
        if colour.as_deref().is_some_and(|c| !cs::valid_colour(c)) {
            self.toast(&tr("That is not a colour"));
            return;
        }
        let classes = self.hints.prefs.borrow().colour_classes;
        let class = colour
            .as_deref()
            .filter(|_| classes)
            .and_then(|c| cs::colour_class(kind, c));
        let via = class.as_deref().map_or(Via::Inline, Via::Class);
        let (text, _, sel) = self.code_state(&buf);
        match cs::apply_colour(&text, sel.0..sel.1, kind, colour.as_deref(), via) {
            Ok(edit) => {
                self.apply_code_edit(&buf, &text, &edit);
                if let (Some(class), Some(c)) = (&class, &colour) {
                    self.ensure_rule(class, &cs::colour_rule(kind, class, c));
                }
            }
            Err(FormatError::Unbalanced) => self.toast(&tr(
                "The selection cuts through tags; select whole elements or plain text",
            )),
            Err(FormatError::NotText) if colour.is_none() => {
                self.toast(&tr("There is no colour here to remove"))
            }
            Err(FormatError::NotText) => self.toast(&tr("The cursor is inside a tag or comment")),
        }
    }

    /// Aligns the block around the caret (`None` removes the alignment).
    pub(super) fn align_block(&mut self, align: Option<Align>) {
        let Some(buf) = self.style_buffer() else {
            return;
        };
        let classes = self.hints.prefs.borrow().colour_classes;
        let all: Vec<String> = Align::ALL.iter().map(|a| cs::align_class(*a)).collect();
        let others: Vec<&str> = all.iter().map(String::as_str).collect();
        let name = align.map(cs::align_class);
        let via = match (&name, classes) {
            (Some(n), true) => Via::Class(n),
            (None, true) => Via::Class(""),
            _ => Via::Inline,
        };
        let (text, cursor, _) = self.code_state(&buf);
        // a removal in class mode also takes an inline alignment off
        let result = if classes && align.is_none() {
            let mut edit = cs::set_align(&text, cursor, None, Via::Inline, &[]);
            if let Ok(e) = &edit {
                if e.apply(&text) == text {
                    edit = cs::set_align(&text, cursor, None, Via::Class(""), &others);
                }
            }
            edit
        } else {
            cs::set_align(&text, cursor, align, via, &others)
        };
        match result {
            Ok(edit) => {
                self.apply_code_edit(&buf, &text, &edit);
                if let (Some(a), Some(n)) = (align, name.filter(|_| classes)) {
                    self.ensure_rule(&n, &cs::align_rule(&n, a));
                }
            }
            Err(_) => self.toast(&tr("The cursor is not inside a paragraph or other block")),
        }
    }

    /// Makes sure the first style sheet of the book defines `class`.
    fn ensure_rule(&mut self, class: &str, rule: &str) {
        let css = bookx_editing::markup::css_files(&self.book.borrow().archive)
            .into_iter()
            .next();
        let Some(css) = css else {
            self.toast(&tr("The book has no style sheet for the class"));
            return;
        };
        let buf = self.buffer_for(&css);
        let text = buffer_text(&buf);
        if text.contains(&format!(".{class}")) {
            return;
        }
        let sep = if text.is_empty() || text.ends_with('\n') {
            ""
        } else {
            "\n"
        };
        let mut end = buf.end_iter();
        buf.insert(&mut end, &format!("{sep}{rule}"));
        self.toast(&trf(
            "Added .{} to {}",
            &[&class, &css.rsplit('/').next().unwrap_or(&css)],
        ));
    }

    /// `code:menu`: the Markup menu as the user sees it: the entries of its first page, the biggest page and the
    /// labels with their accelerators.
    fn dev_menu(&self) {
        let Some(root) = self
            .w
            .menus
            .iter()
            .find(|(n, _)| n == "Markup")
            .and_then(|(_, m)| m.downcast_ref::<gio::Menu>().cloned())
        else {
            return;
        };
        let top: Vec<String> = (0..root.n_items())
            .filter_map(|i| {
                root.item_attribute_value(i, "label", None)
                    .and_then(|v| v.str().map(str::to_string))
            })
            .collect();
        let biggest = (0..root.n_items())
            .filter_map(|i| {
                root.item_link(i, "submenu")
                    .or_else(|| root.item_link(i, "section"))
            })
            .map(|m| markup_menu::page_rows(&m))
            .max()
            .unwrap_or(0);
        eprintln!("editor_menu: top={} biggest_page={biggest}", top.join(","));
        for (l, a) in markup_menu::labels(&root) {
            if !a.is_empty() {
                eprintln!("editor_menu_accel: {l}={a}");
            }
        }
    }

    /// Dev hooks `code:colour=text|#ff0000` (or `off`), `code:align=center` (or `off`), `code:colour-classes=on|off`.
    pub(super) fn dev_style(&mut self, c: &str) -> bool {
        if let Some(a) = c.strip_prefix("colour=") {
            let (k, v) = a.split_once('|').unwrap_or((a, "off"));
            let kind = if k == "background" {
                ColourKind::Background
            } else {
                ColourKind::Text
            };
            self.apply_colour(kind, (v != "off").then(|| v.to_string()));
        } else if let Some(a) = c.strip_prefix("align=") {
            self.align_block(Align::from_id(a));
        } else if c == "menu" {
            self.dev_menu();
            return true;
        } else if let Some(a) = c.strip_prefix("rebind=") {
            let (id, accel) = a.split_once('|').unwrap_or((a, ""));
            let mut keys = crate::shortcuts::current();
            let ok = keys.bind(id, accel).is_ok();
            crate::shortcuts::replace(keys);
            eprintln!("editor_code: rebind {id} {accel} ok={ok}");
            return true;
        } else if let Some(v) = c.strip_prefix("colour-classes=") {
            self.hints.prefs.borrow_mut().colour_classes = v == "on";
            return true;
        } else {
            return false;
        }
        let buf = self.w.view.buffer();
        eprintln!(
            "editor_code: style {} toast={}",
            self.code_dump(&buf),
            self.last_toast.borrow()
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use super::super::format_actions::CodeMsg;
    use super::*;

    #[test]
    fn menu_ids_name_their_actions() {
        assert!(matches!(
            style_msg_for("colour-text"),
            Some(CodeMsg::ColourPick(ColourKind::Text))
        ));
        assert!(matches!(
            style_msg_for("colour-background-off"),
            Some(CodeMsg::ColourDo(ColourKind::Background, None))
        ));
        assert!(matches!(
            style_msg_for("align-justify"),
            Some(CodeMsg::Align(Some(Align::Justify)))
        ));
        assert!(matches!(
            style_msg_for("align-off"),
            Some(CodeMsg::Align(None))
        ));
        assert!(style_msg_for("align-diagonal").is_none());
        assert!(style_msg_for("colour-").is_none());
        assert!(style_msg_for("bold").is_none());
    }

    #[test]
    fn dialog_colours_become_hex() {
        assert_eq!(rgba_hex(&gdk::RGBA::new(1.0, 0.0, 0.5, 1.0)), "#ff0080");
    }
}
