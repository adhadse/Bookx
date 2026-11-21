//! Splitting a page: at the cursor (CE-248) and at several places chosen in a dialog (CE-249).

use crate::i18n::{tr, trf};
use bookx_editing::split_merge::{self, PageBreakRules, SplitSpec};

use super::*;

/// The split dialog while it is open (the dev hooks drive it).
pub(super) struct SplitDlg {
    #[allow(dead_code)] // keeps the dialog alive while the dev hooks use it
    pub dialog: adw::Dialog,
    pub mode: adw::ComboRow,
    pub level: adw::SpinRow,
    pub tag: adw::EntryRow,
    pub class: adw::EntryRow,
    pub sel: adw::EntryRow,
    pub go: gtk::Button,
    pub status: gtk::Label,
    pub list: gtk::ListBox,
}

/// Which way of choosing the places the dialog offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SplitMode {
    Headings,
    Class,
    PageBreaks,
    Selector,
}

impl SplitMode {
    pub(super) const ALL: [SplitMode; 4] = [
        SplitMode::Headings,
        SplitMode::Class,
        SplitMode::PageBreaks,
        SplitMode::Selector,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            SplitMode::Headings => "At headings",
            SplitMode::Class => "At elements with a class",
            SplitMode::PageBreaks => "At page breaks",
            SplitMode::Selector => "At elements chosen by a selector",
        }
    }
}

/// The split a dialog setting stands for.
pub(super) fn spec_for(
    mode: SplitMode,
    level: u8,
    tag: &str,
    class: &str,
    selector: &str,
) -> SplitSpec {
    match mode {
        SplitMode::Headings => SplitSpec::Headings(level.clamp(1, 6)),
        SplitMode::Class => SplitSpec::ByClass {
            tag: Some(tag.trim().to_string()).filter(|t| !t.is_empty()),
            class: class.trim().to_string(),
        },
        SplitMode::PageBreaks => SplitSpec::PageBreaks,
        SplitMode::Selector => SplitSpec::Selector(selector.trim().to_string()),
    }
}

/// The parts a split would give, or why it cannot be done.
pub(super) fn split_preview(
    src: &str,
    spec: &SplitSpec,
    rules: &PageBreakRules,
) -> Result<Vec<split_merge::PartPreview>, String> {
    if let SplitSpec::ByClass { class, .. } = spec {
        if class.is_empty() {
            return Err("Enter a class name".into());
        }
    }
    let pts = split_merge::split_points(src, spec, rules).map_err(|e| format!("{e:#}"))?;
    if pts.is_empty() {
        return Err("Nothing to split: no place found".into());
    }
    Ok(split_merge::preview_parts(src, &pts))
}

impl Editor {
    fn current_markup(&self) -> Option<String> {
        self.current
            .clone()
            .filter(|c| bookx_editing::markup::is_markup_path(c))
    }

    /// CE-248: the cursor is where the file is cut.
    pub(super) fn split_at_cursor(&mut self) {
        let Some(path) = self.current_markup() else {
            self.toast(&tr("Open a chapter first"));
            return;
        };
        let buf = self.buffer_for(&path);
        let text = buffer_text(&buf);
        let c = buf.iter_at_mark(&buf.get_insert()).offset() as usize;
        let at = super::search::char_to_byte_idx(&text, c);
        self.split_apply(SplitSpec::AtOffsets(vec![at]), "Split File at Cursor");
    }

    pub(super) fn split_apply(&mut self, spec: SplitSpec, label: &str) {
        self.run_op(label, move |b, cur| {
            let Some(p) = cur.filter(|p| b.package.item_by_path(p).is_some()) else {
                anyhow::bail!("Select a chapter first");
            };
            let res = split_merge::split_file(b, p, &spec)?;
            Ok(editor_tools::Outcome {
                changed: true,
                summary: trf("Split into {} files", &[&res.parts.len()]),
                details: res.parts,
            })
        });
    }

    /// CE-249: choose the places, see the files that result, then split.
    pub(super) fn split_dialog(&mut self) {
        let Some(path) = self.current_markup() else {
            self.toast(&tr("Open a chapter first"));
            return;
        };
        self.flush();
        let buf = self.buffer_for(&path);
        let src = buffer_text(&buf);
        let rules = split_merge::page_break_rules_for(&self.book.borrow(), &src, &path);

        let dialog = crate::ui::dialog_toast::new_dialog();
        dialog.set_title(&tr("Split File"));
        dialog.set_content_width(480);
        dialog.set_content_height(560);
        let toolbar = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        header.set_show_start_title_buttons(false);
        header.set_show_end_title_buttons(false);
        let cancel = gtk::Button::with_label(&tr("Cancel"));
        let go = gtk::Button::with_label(&tr("Split"));
        go.add_css_class("suggested-action");
        go.set_sensitive(false);
        header.pack_start(&cancel);
        header.pack_end(&go);
        toolbar.add_top_bar(&header);

        let page = adw::PreferencesPage::new();
        let group = adw::PreferencesGroup::new();
        group.set_title(&tr("Where to split"));
        group.set_description(Some(&trf(
            "{} will be cut in front of every match",
            &[&file_name(&path)],
        )));
        let labels: Vec<&str> = SplitMode::ALL.iter().map(|m| m.label()).collect();
        let mode_row = adw::ComboRow::new();
        mode_row.set_title(&tr("Split"));
        mode_row.set_model(Some(&gtk::StringList::new(&labels)));
        let level_row = adw::SpinRow::with_range(1.0, 6.0, 1.0);
        level_row.set_title(&tr("Down to heading level"));
        level_row.set_value(1.0);
        let tag_row = adw::EntryRow::new();
        tag_row.set_title(&tr("Element (empty for any)"));
        let class_row = adw::EntryRow::new();
        class_row.set_title(&tr("Class"));
        let sel_row = adw::EntryRow::new();
        sel_row.set_title(&tr("Selector, like //div[@class='part'] or h2.title"));
        group.add(&mode_row);
        group.add(&level_row);
        group.add(&tag_row);
        group.add(&class_row);
        group.add(&sel_row);
        page.add(&group);
        let result = adw::PreferencesGroup::new();
        result.set_title(&tr("Resulting files"));
        let status = gtk::Label::new(None);
        status.set_xalign(0.0);
        status.set_wrap(true);
        status.add_css_class("dim-label");
        result.set_header_suffix(Some(&status));
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        crate::ui::audit::label(&list, &tr("Files the split would make"));
        result.add(&list);
        page.add(&result);
        toolbar.set_content(Some(&page));
        dialog.set_child(Some(&toolbar));
        dialog.set_default_widget(Some(&go));

        let current: Rc<RefCell<Option<SplitSpec>>> = Rc::new(RefCell::new(None));
        let refresh: Rc<dyn Fn()> = {
            let (mode_row, level_row, tag_row, class_row, sel_row) = (
                mode_row.clone(),
                level_row.clone(),
                tag_row.clone(),
                class_row.clone(),
                sel_row.clone(),
            );
            let (list, status, go, current) =
                (list.clone(), status.clone(), go.clone(), current.clone());
            Rc::new(move || {
                let mode = SplitMode::ALL[(mode_row.selected() as usize).min(3)];
                level_row.set_visible(mode == SplitMode::Headings);
                tag_row.set_visible(mode == SplitMode::Class);
                class_row.set_visible(mode == SplitMode::Class);
                sel_row.set_visible(mode == SplitMode::Selector);
                let spec = spec_for(
                    mode,
                    level_row.value() as u8,
                    &tag_row.text(),
                    &class_row.text(),
                    &sel_row.text(),
                );
                while let Some(c) = list.first_child() {
                    list.remove(&c);
                }
                match split_preview(&src, &spec, &rules) {
                    Ok(parts) => {
                        status.set_text(&trf("{} files", &[&parts.len()]));
                        for (i, p) in parts.iter().enumerate() {
                            let row = adw::ActionRow::new();
                            row.set_title(&glib::markup_escape_text(&p.title));
                            row.set_subtitle(&trf("{} characters", &[&p.chars]));
                            let n = gtk::Label::new(Some(&(i + 1).to_string()));
                            n.add_css_class("dim-label");
                            row.add_prefix(&n);
                            list.append(&row);
                        }
                        *current.borrow_mut() = Some(spec);
                        go.set_sensitive(true);
                    }
                    Err(e) => {
                        status.set_text(&e);
                        *current.borrow_mut() = None;
                        go.set_sensitive(false);
                    }
                }
            })
        };
        {
            let r = refresh.clone();
            mode_row.connect_selected_notify(move |_| r());
            let r = refresh.clone();
            level_row.connect_value_notify(move |_| r());
            for e in [&tag_row, &class_row, &sel_row] {
                let r = refresh.clone();
                e.connect_changed(move |_| r());
            }
        }
        refresh();
        {
            let d = dialog.clone();
            cancel.connect_clicked(move |_| {
                d.close();
            });
            let d = dialog.clone();
            let tx = self.sender.input_sender().clone();
            go.connect_clicked(move |_| {
                if let Some(spec) = current.borrow().clone() {
                    tx.emit(Input::Pv(super::preview::PvMsg::SplitApply(spec)));
                }
                d.close();
            });
        }
        crate::ui::audit::label(&mode_row, &tr("How to choose the places"));
        dialog.present(Some(&self.w.win));
        // dev scripts drive the dialog's settings through this handle
        self.pv.split_dialog.replace(Some(SplitDlg {
            dialog,
            mode: mode_row,
            level: level_row,
            tag: tag_row,
            class: class_row,
            sel: sel_row,
            go,
            status,
            list,
        }));
    }

    /// `editor:pv-split=KIND:ARG...`: runs a split without the dialog, or prints its preview with `preview:`.
    pub(super) fn dev_split(&mut self, r: &str, sender: &ComponentSender<Self>) {
        let (preview, r) = r
            .strip_prefix("preview:")
            .map(|r| (true, r))
            .unwrap_or((false, r));
        let (kind, arg) = r.split_once(':').unwrap_or((r, ""));
        let spec = match kind {
            "headings" => SplitSpec::Headings(arg.parse().unwrap_or(1)),
            "class" => {
                let (tag, class) = arg.split_once('.').unwrap_or(("", arg));
                SplitSpec::ByClass {
                    tag: Some(tag.to_string()).filter(|t| !t.is_empty()),
                    class: class.to_string(),
                }
            }
            "pagebreaks" => SplitSpec::PageBreaks,
            "selector" => SplitSpec::Selector(arg.to_string()),
            "offset" => SplitSpec::AtOffsets(vec![arg.parse().unwrap_or(0)]),
            _ => {
                eprintln!("editor_split: unknown kind {kind}");
                return;
            }
        };
        if preview {
            let Some(path) = self.current_markup() else {
                return;
            };
            let src = self.buffers.get(&path).map(buffer_text).unwrap_or_default();
            let rules = split_merge::page_break_rules_for(&self.book.borrow(), &src, &path);
            match split_preview(&src, &spec, &rules) {
                Ok(parts) => eprintln!(
                    "editor_split: preview parts={} titles={}",
                    parts.len(),
                    parts
                        .iter()
                        .map(|p| p.title.as_str())
                        .collect::<Vec<_>>()
                        .join("|")
                ),
                Err(e) => eprintln!("editor_split: preview error={e}"),
            }
        } else {
            sender.input(Input::Pv(super::preview::PvMsg::SplitApply(spec)));
        }
    }

    /// `editor:pv-dialog=MODE|LEVEL|TAG|CLASS|SELECTOR` opens the split dialog with those settings (MODE is
    /// 0 headings, 1 class, 2 page breaks, 3 selector); `pv-dialog-status` prints what it shows and
    /// `pv-dialog-go` presses Split.
    pub(super) fn dev_dialog(&mut self, c: &str) -> bool {
        if let Some(r) = c.strip_prefix("pv-dialog=") {
            self.split_dialog();
            let g = self.pv.split_dialog.borrow();
            let Some(d) = g.as_ref() else { return true };
            let mut it = r.split('|');
            d.mode
                .set_selected(it.next().and_then(|m| m.parse().ok()).unwrap_or(0));
            d.level
                .set_value(it.next().and_then(|m| m.parse().ok()).unwrap_or(1.0));
            d.tag.set_text(it.next().unwrap_or(""));
            d.class.set_text(it.next().unwrap_or(""));
            d.sel.set_text(it.next().unwrap_or(""));
        } else if c == "pv-dialog-status" {
            let g = self.pv.split_dialog.borrow();
            if let Some(d) = g.as_ref() {
                let mut rows = 0;
                let mut ch = d.list.first_child();
                while let Some(w) = ch {
                    rows += 1;
                    ch = w.next_sibling();
                }
                eprintln!(
                    "editor_split: dialog status={:?} rows={rows} split_enabled={}",
                    d.status.text(),
                    d.go.is_sensitive()
                );
            }
        } else if c == "pv-dialog-go" {
            let go = self.pv.split_dialog.borrow().as_ref().map(|d| d.go.clone());
            if let Some(go) = go {
                go.emit_clicked();
            }
        } else {
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>t</title></head><body><p>intro</p><h1>A</h1><p>x</p><h2 class=\"sub\">B</h2><p>y</p></body></html>";

    #[test]
    fn dialog_settings_become_split_specs() {
        assert_eq!(
            spec_for(SplitMode::Headings, 9, "", "", ""),
            SplitSpec::Headings(6)
        );
        assert_eq!(
            spec_for(SplitMode::Class, 1, " h2 ", " sub ", ""),
            SplitSpec::ByClass {
                tag: Some("h2".into()),
                class: "sub".into()
            }
        );
        assert_eq!(
            spec_for(SplitMode::Class, 1, "", "sub", ""),
            SplitSpec::ByClass {
                tag: None,
                class: "sub".into()
            }
        );
        assert_eq!(
            spec_for(SplitMode::PageBreaks, 1, "", "", ""),
            SplitSpec::PageBreaks
        );
        assert_eq!(
            spec_for(SplitMode::Selector, 1, "", "", " //h1 "),
            SplitSpec::Selector("//h1".into())
        );
    }

    #[test]
    fn the_preview_lists_parts_or_says_why_not() {
        let rules = PageBreakRules::default();
        let parts = split_preview(SRC, &SplitSpec::Headings(2), &rules).unwrap();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[1].title, "A");
        assert_eq!(parts[2].title, "B");
        assert!(
            split_preview(SRC, &SplitSpec::Headings(1), &rules)
                .unwrap()
                .len()
                == 2
        );
        let none = SplitSpec::ByClass {
            tag: None,
            class: "zzz".into(),
        };
        assert!(split_preview(SRC, &none, &rules)
            .unwrap_err()
            .contains("Nothing to split"));
        let empty = SplitSpec::ByClass {
            tag: None,
            class: String::new(),
        };
        assert_eq!(
            split_preview(SRC, &empty, &rules).unwrap_err(),
            "Enter a class name"
        );
        assert!(split_preview(SRC, &SplitSpec::Selector("//h1[".into()), &rules).is_err());
        assert!(split_preview("<p>not xhtml", &SplitSpec::Headings(1), &rules).is_err());
    }
}
