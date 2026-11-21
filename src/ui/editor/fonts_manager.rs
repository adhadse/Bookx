//! Tools → Media and Fonts → Manage Fonts (CE-420): the fonts embedded in the book with their family, style, size
//! and use, a sample of the real glyphs, renaming a family everywhere it is named (`@font-face`, `font-family`,
//! `font`) and removing fonts with their rules. Embedding from the computer is the existing Embed Referenced Fonts.

use crate::i18n::{tr, trf};
use bookx_editing::fonts_manage::{self, ManagedFont};

use super::tools_ui::*;
use super::*;

const SAMPLE: &str = "Sphinx of black quartz, judge my vow. 0123456789";

fn kib(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else {
        format!("{} KB", bytes.div_ceil(1024))
    }
}

fn subtitle(f: &ManagedFont) -> String {
    let mut bits: Vec<String> = Vec::new();
    if !f.file_style.is_empty() {
        bits.push(f.file_style.clone());
    }
    bits.push(kib(f.bytes));
    bits.push(if f.unused {
        tr("not used by any text")
    } else {
        trf("{} characters used", &[&f.used_chars])
    });
    if f.missing_glyphs > 0 {
        bits.push(trf("{} used characters missing", &[&f.missing_glyphs]));
    }
    bits.push(tr(f.embedding.label()));
    bits.join(" · ")
}

impl Editor {
    pub(super) fn tool_fonts(&self) {
        let fonts = fonts_manage::list_fonts(&self.book.borrow());
        let dialog = crate::ui::dialog_toast::new_dialog();
        dialog.set_title(&tr("Manage Fonts"));
        let (w, h) = fit_dialog(&self.w.win, 600, 720);
        dialog.set_content_width(w);
        dialog.set_content_height(h);

        let sample_text = entry_row(&tr("Sample text"), SAMPLE);
        let sample = gtk::Picture::new();
        sample.set_content_fit(gtk::ContentFit::ScaleDown);
        sample.set_halign(gtk::Align::Start);
        sample.set_can_shrink(true);
        crate::ui::audit::label(&sample, &tr("Sample of the selected font"));
        let sample_note = gtk::Label::new(Some(&tr("Open a font below to see a sample.")));
        sample_note.add_css_class("dim-label");
        sample_note.set_xalign(0.0);
        sample_note.set_wrap(true);
        let chosen: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

        let show_sample: Rc<dyn Fn()> = {
            let (book, chosen, sample, note, text) = (
                self.book.clone(),
                chosen.clone(),
                sample.clone(),
                sample_note.clone(),
                sample_text.clone(),
            );
            Rc::new(move || {
                let Some(path) = chosen.borrow().clone() else {
                    return;
                };
                let data = book.borrow().archive.get(&path).map(<[u8]>::to_vec);
                let svg = data.and_then(|d| fonts_manage::sample_svg(&d, &text.text(), 28.0, 520));
                match svg.and_then(|s| super::metadata::texture_from_bytes(s.as_bytes())) {
                    Some(t) => {
                        sample.set_paintable(Some(&t));
                        note.set_text(&trf("Sample in {}", &[&file_name(&path)]));
                    }
                    None => {
                        sample.set_paintable(gdk::Paintable::NONE);
                        note.set_text(&tr(
                            "No sample: the font has none of the characters of the sample text.",
                        ));
                    }
                }
            })
        };
        {
            let s = show_sample.clone();
            sample_text.connect_changed(move |_| s());
        }

        let list = group(&tr("Fonts in this book"));
        let tx = self.sender.input_sender().clone();
        for f in &fonts {
            let ex = adw::ExpanderRow::new();
            ex.set_title(&glib::markup_escape_text(f.display_family()));
            ex.set_subtitle(&glib::markup_escape_text(&subtitle(f)));
            let path_row = adw::ActionRow::new();
            path_row.set_title(&tr("File"));
            path_row.set_subtitle(&glib::markup_escape_text(&f.path));
            path_row.set_subtitle_selectable(true);
            ex.add_row(&path_row);
            if !f.declared.is_empty() {
                let rename = entry_row(&tr("Family name in the book"), &f.declared[0]);
                let go = gtk::Button::with_label(&tr("Rename"));
                go.set_valign(gtk::Align::Center);
                rename.add_suffix(&go);
                let (tx2, d, from) = (tx.clone(), dialog.clone(), f.declared[0].clone());
                let rename2 = rename.clone();
                go.connect_clicked(move |_| {
                    let (from, to) = (from.clone(), rename2.text().to_string());
                    tx2.emit(Input::Ext(Cmd::Run(
                        "Rename Font".to_string(),
                        OpBox(Rc::new(move |b: &mut OpenBook, _: Option<&str>| {
                            crate::editor_structure::fonts_rename(b, &from, &to)
                        })),
                    )));
                    d.close();
                });
                ex.add_row(&rename);
            }
            let rm_row = adw::ActionRow::new();
            rm_row.set_title(&tr("Remove this font"));
            rm_row.set_subtitle(&tr("Deletes the file and its @font-face rules; the text falls back to the next font of its list"));
            let rm = gtk::Button::with_label(&tr("Remove"));
            rm.add_css_class("destructive-action");
            rm.set_valign(gtk::Align::Center);
            rm_row.add_suffix(&rm);
            {
                let (tx2, d, path) = (tx.clone(), dialog.clone(), f.path.clone());
                rm.connect_clicked(move |_| {
                    let p = vec![path.clone()];
                    tx2.emit(Input::Ext(Cmd::Run(
                        "Remove Font".to_string(),
                        OpBox(Rc::new(move |b: &mut OpenBook, _: Option<&str>| {
                            crate::editor_structure::fonts_remove(b, &p)
                        })),
                    )));
                    d.close();
                });
            }
            ex.add_row(&rm_row);
            {
                let (chosen, show, path) = (chosen.clone(), show_sample.clone(), f.path.clone());
                ex.connect_expanded_notify(move |e| {
                    if e.is_expanded() {
                        *chosen.borrow_mut() = Some(path.clone());
                        show();
                    }
                });
            }
            list.add(&ex);
        }

        let embed = gtk::Button::with_label(&tr("Embed Fonts from this Computer"));
        embed.set_halign(gtk::Align::Start);
        {
            let (tx, d) = (tx.clone(), dialog.clone());
            embed.connect_clicked(move |_| {
                tx.emit(Input::Ext(Cmd::Open("embed-fonts".into())));
                d.close();
            });
        }
        let sg = group(&tr("Sample"));
        sg.add(&sample_text);
        sg.set_widget_name("fonts-sample");
        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        body.set_margin_all(16);
        if fonts.is_empty() {
            let empty = adw::StatusPage::builder()
                .icon_name("font-x-generic-symbolic")
                .title(tr("No Embedded Fonts"))
                .description(tr("The book uses the fonts of the reading device"))
                .build();
            empty.set_widget_name("fonts-empty");
            embed.set_halign(gtk::Align::Center);
            embed.add_css_class("pill");
            embed.add_css_class("suggested-action");
            empty.set_child(Some(&embed));
            body.append(&empty);
        } else {
            body.append(&list);
            body.append(&sg);
            body.append(&sample);
            body.append(&sample_note);
            body.append(&embed);
        }
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_child(Some(&body));
        let view = adw::ToolbarView::new();
        view.add_top_bar(&adw::HeaderBar::new());
        view.set_content(Some(&scroll));
        dialog.set_child(Some(&view));
        *self.ext.structure.fonts_dialog.borrow_mut() = Some(dialog.clone());
        dialog.present(Some(&self.w.win));
    }

    /// `x:fonts-manage=list` | `rename=<from>|<to>` | `remove=<path>` | `sample=<path>[|<text>]` | `open`.
    pub(super) fn dev_fonts(&mut self, arg: &str) {
        let (cmd, rest) = arg.split_once('=').unwrap_or((arg, ""));
        match cmd {
            "list" => {
                let l = fonts_manage::list_fonts(&self.book.borrow());
                eprintln!(
                    "editor_x: fonts-manage: {}",
                    l.iter()
                        .map(|f| format!(
                            "{}|{}|{}|{}|{}",
                            f.display_family(),
                            f.path,
                            f.bytes,
                            f.used_chars,
                            f.unused
                        ))
                        .collect::<Vec<_>>()
                        .join(";")
                );
            }
            "rename" => {
                if let Some((from, to)) = rest.split_once('|') {
                    let (from, to) = (from.to_string(), to.to_string());
                    self.run_op("Rename Font", move |b, _| {
                        crate::editor_structure::fonts_rename(b, &from, &to)
                    });
                }
            }
            "remove" => {
                let p: Vec<String> = rest.split('|').map(str::to_string).collect();
                self.run_op("Remove Font", move |b, _| {
                    crate::editor_structure::fonts_remove(b, &p)
                });
            }
            "sample" => {
                let (path, text) = rest.split_once('|').unwrap_or((rest, SAMPLE));
                let svg = self
                    .book
                    .borrow()
                    .archive
                    .get(path)
                    .and_then(|d| fonts_manage::sample_svg(d, text, 28.0, 520));
                eprintln!(
                    "editor_x: fonts-manage: sample={} texture={}",
                    svg.as_ref().map(|s| s.len()).unwrap_or(0),
                    svg.and_then(|s| super::metadata::texture_from_bytes(s.as_bytes()))
                        .is_some()
                );
            }
            "open" => self.tool_fonts(),
            "close" => {
                if let Some(d) = self.ext.structure.fonts_dialog.borrow_mut().take() {
                    d.close();
                }
            }
            _ => self.toast(&format!("Unknown fonts-manage command {cmd}")),
        }
    }
}
