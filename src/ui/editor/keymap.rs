//! The editor's keys (CE-403): what each id of `editor_keymap` does, and the shortcut controllers
//! built from the user's current bindings (rebuilt whenever they change).

use super::*;
use crate::editor_keymap::{Cmd as KeyCmd, COMMANDS};
use crate::shortcuts;
use bookx_editing::code_format::Format;
use bookx_editing::code_style::{Align, ColourKind};
use format_actions::CodeMsg;
use prefs_page::PrefsAct;

/// What the editor does for a registry id.
pub(super) fn input_for(id: &str) -> Option<Input> {
    Some(match id {
        "editor.save" => Input::Save,
        "editor.close-tab" => Input::CloseTab,
        "editor.new-file" => Input::PromptNewFile,
        "editor.quick-open" => Input::QuickOpen,
        "editor.palette" => Input::Palette,
        "editor.next-file" => Input::StepFile(1),
        "editor.previous-file" => Input::StepFile(-1),
        "editor.go-to-line" => Input::PromptLine,
        "editor.preferences" => Input::Prefs(PrefsAct::Open),
        "editor.metadata" => Input::EditMetadata,
        "editor.preview" => Input::TogglePreview,
        "editor.sidebar" => Input::ToggleSidebar,
        "editor.beautify" => Input::Reformat,
        "editor.find" => Input::ToggleFind,
        "editor.replace-bar" => Input::ToggleReplace,
        "editor.find-next" => Input::FindNext(true),
        "editor.find-previous" => Input::FindNext(false),
        "editor.replace" => Input::Replace(true),
        "editor.replace-previous" => Input::Replace(false),
        "editor.find-all" => Input::FindAll,
        "editor.mark" => Input::MarkSelection,
        "editor.clear-marks" => Input::Prefs(PrefsAct::ClearMarks),
        "editor.insert-snippet" => Input::Prefs(PrefsAct::InsertSnippet),
        "editor.jump-matching-tag" => Input::Tag(TagAction::JumpMatching, None),
        "editor.select-matching-tag" => Input::Tag(TagAction::SelectMatching, None),
        "editor.format-bold" => Input::Code(CodeMsg::Format(Format::Bold)),
        "editor.format-italic" => Input::Code(CodeMsg::Format(Format::Italic)),
        "editor.format-underline" => Input::Code(CodeMsg::Format(Format::Underline)),
        "editor.format-strike" => Input::Code(CodeMsg::Format(Format::Strike)),
        "editor.format-sup" => Input::Code(CodeMsg::Format(Format::Superscript)),
        "editor.format-sub" => Input::Code(CodeMsg::Format(Format::Subscript)),
        "editor.format-code" => Input::Code(CodeMsg::Format(Format::Code)),
        "editor.colour-text" => Input::Code(CodeMsg::ColourPick(ColourKind::Text)),
        "editor.colour-background" => Input::Code(CodeMsg::ColourPick(ColourKind::Background)),
        "editor.align-left" => Input::Code(CodeMsg::Align(Some(Align::Left))),
        "editor.align-center" => Input::Code(CodeMsg::Align(Some(Align::Center))),
        "editor.align-right" => Input::Code(CodeMsg::Align(Some(Align::Right))),
        "editor.align-justify" => Input::Code(CodeMsg::Align(Some(Align::Justify))),
        "editor.split-tag" => Input::Code(CodeMsg::SplitTag),
        "editor.insert-image" => Input::Code(CodeMsg::InsertImage),
        "editor.insert-link" => Input::Code(CodeMsg::InsertLink),
        "editor.go-to-definition" => Input::Code(CodeMsg::GotoDef),
        "editor.sort-css" => Input::Code(CodeMsg::SortCss),
        "editor.close-tag" => Input::Tag(TagAction::CloseTag, None),
        "editor.wrap-tag" => Input::Tag(TagAction::Wrap, None),
        "editor.unwrap-tag" => Input::Tag(TagAction::Unwrap, None),
        "editor.rename-tag" => Input::Tag(TagAction::Rename, None),
        "editor.comment" => Input::Tag(TagAction::Comment, None),
        "editor.check" => Input::Check,
        "editor.next-problem" => Input::Problems(check_panel::Act::Next),
        "editor.previous-problem" => Input::Problems(check_panel::Act::Prev),
        "editor.report" => Input::ShowReport,
        "editor.special-characters" => Input::Chars(special_chars::Msg::Open),
        "editor.check-spelling" => Input::CheckSpelling,
        "editor.next-misspelling" => Input::SpellNext(true),
        "editor.previous-misspelling" => Input::SpellNext(false),
        "editor.checkpoint" => Input::CreateCheckpoint,
        "editor.split" => Input::SplitHeadings(1),
        "editor.merge" => Input::MergeNext,
        "editor.refresh-preview" => Input::Pv(preview::PvMsg::Refresh),
        "editor.copy-preview" => Input::Pv(preview::PvMsg::CopySelection),
        other => {
            if let Some(n) = other
                .strip_prefix("editor.heading-")
                .and_then(|n| n.parse::<u8>().ok())
            {
                Input::Tag(TagAction::Heading(n), None)
            } else {
                let win = other.strip_prefix("editor.")?;
                if matches!(
                    win,
                    "save-copy"
                        | "save-copy-edit"
                        | "open-book"
                        | "undo-checkpoint"
                        | "redo-checkpoint"
                        | "close-others"
                        | "close-right"
                        | "help"
                ) {
                    Input::Ext(tools_ui::Cmd::Win(win.to_string()))
                } else {
                    return None;
                }
            }
        }
    })
}

/// The shortcut controllers for the current bindings: (bubble phase, capture phase).
fn controllers(tx: &relm4::Sender<Input>) -> (gtk::ShortcutController, gtk::ShortcutController) {
    let cur = shortcuts::current();
    let bubble = gtk::ShortcutController::new();
    bubble.set_scope(gtk::ShortcutScope::Local);
    let capture = gtk::ShortcutController::new();
    capture.set_scope(gtk::ShortcutScope::Local);
    capture.set_propagation_phase(gtk::PropagationPhase::Capture);
    for c in COMMANDS {
        let target = if c.capture { &capture } else { &bubble };
        for accel in cur.effective(c.id) {
            let Some(trigger) = gtk::ShortcutTrigger::parse_string(&accel) else {
                continue;
            };
            let (tx, id) = (tx.clone(), c.id);
            target.add_shortcut(gtk::Shortcut::new(
                Some(trigger),
                Some(gtk::CallbackAction::new(move |_, _| {
                    if let Some(i) = input_for(id) {
                        tx.emit(i);
                    }
                    glib::Propagation::Stop
                })),
            ));
        }
    }
    (bubble, capture)
}

/// Puts the editor's shortcuts on `win` and keeps them in step with the bindings.
pub(super) fn install(win: &adw::Window, tx: relm4::Sender<Input>) {
    let current: Rc<RefCell<Option<(gtk::ShortcutController, gtk::ShortcutController)>>> =
        Rc::default();
    let weak = win.downgrade();
    let apply = Rc::new(move || {
        let Some(win) = weak.upgrade() else { return };
        if let Some((a, b)) = current.borrow_mut().take() {
            win.remove_controller(&a);
            win.remove_controller(&b);
        }
        let (a, b) = controllers(&tx);
        win.add_controller(a.clone());
        win.add_controller(b.clone());
        *current.borrow_mut() = Some((a, b));
    });
    apply();
    shortcuts::on_change(move || apply());
}

/// The keys of a menu or palette target for display (`editor:save`, `tools.split`,
/// `edit.tag::wrap`), empty when the target has no shortcut.
pub(super) fn display_for_target(target: &str) -> String {
    COMMANDS
        .iter()
        .find(|c: &&KeyCmd| !c.palette.is_empty() && c.palette == target)
        .map(|c| shortcuts::display(c.id, ""))
        .unwrap_or_default()
}

/// The first current binding of the command a menu target stands for, in the GTK spelling (`<Control>b`).
pub(super) fn accel_for_target(target: &str) -> Option<String> {
    let c = COMMANDS
        .iter()
        .find(|c: &&KeyCmd| !c.palette.is_empty() && c.palette == target)?;
    shortcuts::current().effective(c.id).into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_registered_command_does_something() {
        for c in COMMANDS {
            assert!(input_for(c.id).is_some(), "{} has no action", c.id);
        }
        assert!(input_for("editor.nope").is_none());
    }

    #[test]
    fn a_target_finds_its_keys() {
        assert_eq!(display_for_target("nothing"), "");
        assert!(!display_for_target("editor:save").is_empty());
    }
}
