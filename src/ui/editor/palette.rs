//! The command palette of the editor window (Ctrl+Shift+P): its own commands, every item of the
//! Tools, Markup and Files menus, and the files of the book. The ranking is in [`crate::palette`].

use super::*;
use crate::palette::{self, Entry, Target};

/// The items of `menu` (sections and submenus flattened) as palette entries under `group`.
fn walk(menu: &gio::MenuModel, group: &str, out: &mut Vec<Entry>) {
    for i in 0..menu.n_items() {
        let label = menu
            .item_attribute_value(i, "label", Some(glib::VariantTy::STRING))
            .and_then(|v| v.str().map(str::to_string))
            .unwrap_or_default();
        if let Some(section) = menu.item_link(i, "section") {
            walk(&section, group, out);
        }
        if let Some(sub) = menu.item_link(i, "submenu") {
            walk(
                &sub,
                &format!("{group} › {}", palette::clean_label(&label)),
                out,
            );
        }
        let Some(action) = menu
            .item_attribute_value(i, "action", Some(glib::VariantTy::STRING))
            .and_then(|v| v.str().map(str::to_string))
        else {
            continue;
        };
        let param = menu
            .item_attribute_value(i, "target", None)
            .and_then(|v| v.str().map(str::to_string));
        out.push(palette::menu_entry(
            group,
            &label,
            &action,
            param.as_deref(),
        ));
    }
}

impl Editor {
    pub(super) fn palette_entries(&self) -> Vec<Entry> {
        let mut out = palette::editor_commands();
        for (group, menu) in &self.w.menus {
            walk(menu, group, &mut out);
        }
        for f in self.quick_open_results("") {
            out.push(palette::file_entry(&f));
        }
        out
    }

    pub(super) fn show_palette(&self) {
        let win = self.w.win.clone();
        let tx = self.sender.input_sender().clone();
        crate::ui::palette::show(
            &self.w.win,
            self.palette_entries(),
            Rc::new(move |e| {
                if !crate::devtools::script().is_empty() {
                    eprintln!("palette: picked {} -> {:?}", e.title, e.target);
                }
                match e.target {
                    Target::Action(a) => {
                        let _ = gtk::prelude::WidgetExt::activate_action(&win, &a, None);
                    }
                    Target::ActionWith(a, p) => {
                        let _ = gtk::prelude::WidgetExt::activate_action(
                            &win,
                            &a,
                            Some(&p.to_variant()),
                        );
                    }
                    Target::File(f) => tx.emit(Input::SelectFile(f)),
                    Target::Editor(id) => {
                        if let Some(i) = editor_input(&id) {
                            tx.emit(i);
                        }
                    }
                    Target::Book(_) | Target::Setting(_) => {}
                }
            }),
        );
    }
}

/// What the editor does for a command of [`palette::EDITOR_COMMANDS`].
fn editor_input(id: &str) -> Option<Input> {
    Some(match id {
        "save" => Input::Save,
        "find" => Input::ToggleFind,
        "check" => Input::Check,
        "go-to-line" => Input::PromptLine,
        "quick-open" => Input::QuickOpen,
        "preview" => Input::TogglePreview,
        "beautify" => Input::Reformat,
        "mark" => Input::MarkSelection,
        "metadata" => Input::EditMetadata,
        "find-all" => Input::FindAll,
        "close-tab" => Input::CloseTab,
        "next-file" => Input::StepFile(1),
        "previous-file" => Input::StepFile(-1),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_palette_command_has_an_input() {
        for (id, ..) in crate::palette::EDITOR_COMMANDS {
            assert!(editor_input(id).is_some(), "{id}");
        }
        assert!(editor_input("nope").is_none());
    }
}
