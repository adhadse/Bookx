//! Tools → Media and Fonts → Set Cover (CE-129): pick an image of the book, a file from the computer or a cover
//! drawn from the title and author, crop and scale it with a live preview, and let the editor make the cover page
//! and update the package, the guide and the landmarks (`bookx_editing::cover`).

use crate::i18n::{tr, trf};
use bookx_editing::cover::{self, CoverOptions, CoverSource, MAX_COVER_BYTES};

use super::tools_ui::*;
use super::*;

const CROPS: [(&str, Option<f64>); 3] = [
    (crate::i18n::N_("Keep the whole image"), None),
    (crate::i18n::N_("Crop to 2:3 (centered)"), Some(2.0 / 3.0)),
    (crate::i18n::N_("Crop to 3:4 (centered)"), Some(3.0 / 4.0)),
];

const SIZES: [(&str, Option<u32>); 4] = [
    (crate::i18n::N_("Keep the size"), None),
    (crate::i18n::N_("At most 1600 pixels high"), Some(1600)),
    (crate::i18n::N_("At most 1200 pixels high"), Some(1200)),
    (crate::i18n::N_("At most 800 pixels high"), Some(800)),
];

/// The images of the book a cover can be made from.
fn book_images(book: &OpenBook) -> Vec<String> {
    book.archive
        .names()
        .iter()
        .filter(|n| {
            matches!(
                extension(n).as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "webp"
            ) && book.archive.size_of(n) <= MAX_COVER_BYTES
        })
        .cloned()
        .collect()
}

fn bytes_and_ext(book: &OpenBook, src: &CoverSource) -> Option<(Vec<u8>, String)> {
    match src {
        CoverSource::Existing(p) => book.archive.get(p).map(|b| (b.to_vec(), extension(p))),
        CoverSource::New { data, ext } => Some((data.clone(), ext.clone())),
    }
}

/// The options for the chosen source and the indices of the crop and size rows.
pub(super) fn cover_options(
    src: CoverSource,
    crop: usize,
    size: usize,
    page: bool,
    remove_old: bool,
    dims: Option<(u32, u32)>,
) -> CoverOptions {
    let mut o = CoverOptions::new(src);
    o.crop = CROPS
        .get(crop)
        .and_then(|c| c.1)
        .zip(dims)
        .map(|(a, (w, h))| cover::centered_crop(w, h, a));
    o.max_size = SIZES
        .get(size)
        .and_then(|s| s.1)
        .map(|h| (h.saturating_mul(2), h));
    o.page = page;
    o.remove_old = remove_old;
    o
}

fn generated(book: &OpenBook) -> anyhow::Result<CoverSource> {
    let m = book.metadata();
    let png = bookx_sources::cover::generate_cover(
        &m.title,
        &m.authors.join(", "),
        &m.series,
        800,
        1200,
    )?;
    Ok(CoverSource::New {
        data: png,
        ext: "png".into(),
    })
}

impl Editor {
    pub(super) fn tool_cover(&self) {
        let book = self.book.clone();
        let source: Rc<RefCell<Option<CoverSource>>> = Rc::new(RefCell::new(None));
        let images = book_images(&book.borrow());
        let current = book.borrow().package.cover_path();

        let dialog = crate::ui::dialog_toast::new_dialog();
        dialog.set_title(&tr("Set Cover"));
        let (w, h) = fit_dialog(&self.w.win, 560, 720);
        dialog.set_content_width(w);
        dialog.set_content_height(h);

        let pic = gtk::Picture::new();
        pic.set_content_fit(gtk::ContentFit::Contain);
        pic.set_size_request(-1, 220);
        pic.add_css_class("card");
        crate::ui::audit::label(&pic, &tr("Cover preview"));
        let info = gtk::Label::new(None);
        info.add_css_class("dim-label");
        info.add_css_class("caption");
        info.set_wrap(true);
        info.set_xalign(0.0);

        let mut names: Vec<String> = vec![tr("(choose an image)")];
        names.extend(
            images
                .iter()
                .map(|p| p.rsplit('/').next().unwrap_or(p).to_string()),
        );
        let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let pick = combo_row(&tr("Image of the book"), &name_refs, 0);
        let file_btn = gtk::Button::with_label(&tr("Choose a File"));
        file_btn.set_valign(gtk::Align::Center);
        let gen_btn = gtk::Button::with_label(&tr("Draw from Title and Author"));
        gen_btn.set_valign(gtk::Align::Center);
        let src_row = adw::ActionRow::new();
        src_row.set_title(&tr("Or use"));
        src_row.add_suffix(&file_btn);
        src_row.add_suffix(&gen_btn);
        let crop_labels: Vec<String> = CROPS.iter().map(|c| tr(c.0)).collect();
        let crop_refs: Vec<&str> = crop_labels.iter().map(String::as_str).collect();
        let crop = combo_row(&tr("Crop"), &crop_refs, 0);
        let size_labels: Vec<String> = SIZES.iter().map(|c| tr(c.0)).collect();
        let size_refs: Vec<&str> = size_labels.iter().map(String::as_str).collect();
        let size = combo_row(&tr("Size"), &size_refs, 0);
        let page = switch_row(
            &tr("Make a cover page"),
            &tr(
                "A first page that shows the cover, listed as the cover in the guide and landmarks",
            ),
            true,
        );
        let old = switch_row(
            &tr("Delete the old cover image"),
            &tr("Only when no page uses it any more"),
            true,
        );
        let g = group("");
        let remove = gtk::Button::with_label(&tr("Remove Cover"));
        remove.add_css_class("destructive-action");
        remove.set_valign(gtk::Align::Center);
        remove.set_sensitive(current.is_some());
        let current_row = adw::ActionRow::new();
        current_row.set_title(&tr("Current cover"));
        current_row.set_subtitle(&match &current {
            Some(c) => c.rsplit('/').next().unwrap_or(c).to_string(),
            None => tr("The book has no cover"),
        });
        current_row.add_suffix(&remove);
        for r in [
            current_row.upcast_ref::<gtk::Widget>(),
            pick.upcast_ref(),
            src_row.upcast_ref(),
            crop.upcast_ref(),
            size.upcast_ref(),
            page.upcast_ref(),
            old.upcast_ref(),
        ] {
            g.add(r);
        }
        let msg = gtk::Label::new(None);
        msg.set_wrap(true);
        msg.set_xalign(0.0);
        msg.add_css_class("error");

        let apply = gtk::Button::with_label(&tr("Set Cover"));
        apply.add_css_class("suggested-action");
        apply.set_sensitive(false);

        // shows the image as it will be stored
        let refresh: Rc<dyn Fn()> = {
            let (source, book, pic, info, crop, size, msg, apply) = (
                source.clone(),
                book.clone(),
                pic.clone(),
                info.clone(),
                crop.clone(),
                size.clone(),
                msg.clone(),
                apply.clone(),
            );
            Rc::new(move || {
                let Some(src) = source.borrow().clone() else {
                    pic.set_paintable(gdk::Paintable::NONE);
                    apply.set_sensitive(false);
                    info.set_text(&tr("Choose the image to use as the cover."));
                    return;
                };
                let Some((data, ext)) = bytes_and_ext(&book.borrow(), &src) else {
                    return;
                };
                let dims = cover::decode_limited(&data)
                    .ok()
                    .map(|i| (i.width(), i.height()));
                let o = cover_options(
                    src,
                    crop.selected() as usize,
                    size.selected() as usize,
                    true,
                    true,
                    dims,
                );
                match cover::prepare_image(&data, &ext, o.crop, o.max_size) {
                    Ok((bytes, _)) => {
                        msg.set_text("");
                        let (w, h) = bookx_editing::images::image_dimensions(&bytes);
                        info.set_text(&trf(
                            "{} × {} pixels, {} KB",
                            &[&w, &h, &(bytes.len() / 1024)],
                        ));
                        if let Some(t) = super::metadata::texture_from_bytes(&bytes) {
                            pic.set_paintable(Some(&t));
                        }
                        apply.set_sensitive(true);
                    }
                    Err(e) => {
                        pic.set_paintable(gdk::Paintable::NONE);
                        msg.set_text(&format!("{e:#}"));
                        apply.set_sensitive(false);
                    }
                }
            })
        };
        {
            let (source, refresh, images) = (source.clone(), refresh.clone(), images.clone());
            pick.connect_selected_notify(move |r| {
                let i = r.selected() as usize;
                if i > 0 {
                    *source.borrow_mut() = images.get(i - 1).cloned().map(CoverSource::Existing);
                    refresh();
                }
            });
        }
        for r in [&crop, &size] {
            let refresh = refresh.clone();
            r.connect_selected_notify(move |_| refresh());
        }
        {
            let (source, refresh, pick, msg, book) = (
                source.clone(),
                refresh.clone(),
                pick.clone(),
                msg.clone(),
                book.clone(),
            );
            gen_btn.connect_clicked(move |_| match generated(&book.borrow()) {
                Ok(s) => {
                    pick.set_selected(0);
                    *source.borrow_mut() = Some(s);
                    refresh();
                }
                Err(e) => msg.set_text(&format!("{e:#}")),
            });
        }
        {
            let (source, refresh, pick, msg, win) = (
                source.clone(),
                refresh.clone(),
                pick.clone(),
                msg.clone(),
                self.w.win.clone(),
            );
            file_btn.connect_clicked(move |_| {
                let filter = gtk::FileFilter::new();
                filter.set_name(Some(&tr("Images")));
                for p in ["*.png", "*.jpg", "*.jpeg", "*.gif", "*.webp"] {
                    filter.add_pattern(p);
                }
                let filters = gio::ListStore::new::<gtk::FileFilter>();
                filters.append(&filter);
                let dlg = gtk::FileDialog::builder()
                    .title(tr("Choose a Cover Image"))
                    .filters(&filters)
                    .build();
                let (source, refresh, pick, msg) =
                    (source.clone(), refresh.clone(), pick.clone(), msg.clone());
                dlg.open(Some(&win), gio::Cancellable::NONE, move |res| {
                    let Some(p) = res.ok().and_then(|f| f.path()) else {
                        return;
                    };
                    match read_cover_file(&p) {
                        Ok(s) => {
                            pick.set_selected(0);
                            *source.borrow_mut() = Some(s);
                            refresh();
                        }
                        Err(e) => msg.set_text(&format!("{e:#}")),
                    }
                });
            });
        }
        {
            let (tx, d) = (self.sender.input_sender().clone(), dialog.clone());
            remove.connect_clicked(move |_| {
                tx.emit(Input::Ext(Cmd::Run(
                    "Remove Cover".to_string(),
                    OpBox(Rc::new(|b: &mut OpenBook, _: Option<&str>| {
                        crate::editor_structure::remove_cover(b)
                    })),
                )));
                d.close();
            });
        }
        {
            let (tx, d) = (self.sender.input_sender().clone(), dialog.clone());
            let (source, book, crop, size, page, old) = (
                source.clone(),
                book.clone(),
                crop.clone(),
                size.clone(),
                page.clone(),
                old.clone(),
            );
            apply.connect_clicked(move |_| {
                let Some(src) = source.borrow().clone() else {
                    return;
                };
                let dims = bytes_and_ext(&book.borrow(), &src)
                    .and_then(|(d, _)| cover::decode_limited(&d).ok())
                    .map(|i| (i.width(), i.height()));
                let o = cover_options(
                    src,
                    crop.selected() as usize,
                    size.selected() as usize,
                    page.is_active(),
                    old.is_active(),
                    dims,
                );
                tx.emit(Input::Ext(Cmd::Run(
                    "Set Cover".to_string(),
                    OpBox(Rc::new(move |b: &mut OpenBook, _: Option<&str>| {
                        crate::editor_structure::set_cover(b, &o)
                    })),
                )));
                d.close();
            });
        }
        // the current cover is the starting preview
        if let Some(c) = &current {
            if let Some(i) = images.iter().position(|p| p == c) {
                pick.set_selected(i as u32 + 1);
            }
        }
        refresh();

        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        body.set_margin_all(16);
        body.append(&pic);
        body.append(&info);
        body.append(&g);
        body.append(&msg);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_child(Some(&body));
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        bar.set_margin_all(12);
        bar.set_halign(gtk::Align::End);
        let close = gtk::Button::with_label(&tr("Close"));
        {
            let d = dialog.clone();
            close.connect_clicked(move |_| {
                d.close();
            });
        }
        bar.append(&close);
        bar.append(&apply);
        let view = adw::ToolbarView::new();
        view.add_top_bar(&adw::HeaderBar::new());
        view.set_content(Some(&scroll));
        view.add_bottom_bar(&bar);
        dialog.set_child(Some(&view));
        dialog.present(Some(&self.w.win));
    }

    /// `x:cover=<source>;crop=<0..2>;size=<0..3>;page=<0|1>;old=<0|1>` with the source `existing:<path>`,
    /// `file:<path>`, `gen`, or the whole argument `remove`.
    pub(super) fn dev_cover(&mut self, arg: &str) {
        if arg == "remove" {
            self.run_op("Remove Cover", |b, _| {
                crate::editor_structure::remove_cover(b)
            });
            return;
        }
        let mut parts = arg.split(';');
        let src = parts.next().unwrap_or("");
        let source = if let Some(p) = src.strip_prefix("existing:") {
            Ok(CoverSource::Existing(p.to_string()))
        } else if let Some(p) = src.strip_prefix("file:") {
            read_cover_file(std::path::Path::new(p))
        } else {
            generated(&self.book.borrow())
        };
        let source = match source {
            Ok(s) => s,
            Err(e) => {
                self.toast(&format!("{e:#}"));
                return;
            }
        };
        let (mut crop, mut size, mut page, mut old) = (0, 0, true, true);
        for p in parts {
            match p.split_once('=') {
                Some(("crop", v)) => crop = v.parse().unwrap_or(0),
                Some(("size", v)) => size = v.parse().unwrap_or(0),
                Some(("page", v)) => page = v != "0",
                Some(("old", v)) => old = v != "0",
                _ => {}
            }
        }
        let dims = bytes_and_ext(&self.book.borrow(), &source)
            .and_then(|(d, _)| cover::decode_limited(&d).ok())
            .map(|i| (i.width(), i.height()));
        let o = cover_options(source, crop, size, page, old, dims);
        self.run_op("Set Cover", move |b, _| {
            crate::editor_structure::set_cover(b, &o)
        });
    }

    pub(super) fn dev_cover_state(&self) {
        let b = self.book.borrow();
        let page = bookx_editing::cover::find_cover_page(&b);
        let sem = bookx_editing::semantics::read_semantics(&b)
            .into_iter()
            .filter(|e| e.semantic == bookx_editing::semantics::Semantic::Cover)
            .map(|e| e.href)
            .collect::<Vec<_>>()
            .join(",");
        let opf = b
            .archive
            .get_string(&b.package.opf_path)
            .unwrap_or_default();
        eprintln!(
            "editor_x: cover-state: image={} page={} semantics={} cover-image-items={} first-spine={} images={}",
            b.package.cover_path().unwrap_or_default(),
            page.unwrap_or_default(),
            sem,
            opf.matches("cover-image").count(),
            b.spine_paths().first().cloned().unwrap_or_default(),
            book_images(&b).join(",")
        );
    }
}

/// A cover file chosen by the user: size and type are checked before it is used.
pub(super) fn read_cover_file(p: &std::path::Path) -> anyhow::Result<CoverSource> {
    use anyhow::Context;
    let meta = std::fs::metadata(p).with_context(|| format!("cannot read {}", p.display()))?;
    if !meta.is_file() {
        anyhow::bail!("{} is not a file", p.display());
    }
    if meta.len() as usize > MAX_COVER_BYTES {
        anyhow::bail!("the image is too large ({} MB)", meta.len() / (1024 * 1024));
    }
    let data = std::fs::read(p).with_context(|| format!("cannot read {}", p.display()))?;
    cover::decode_limited(&data)?;
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png")
        .to_ascii_lowercase();
    Ok(CoverSource::New { data, ext })
}
