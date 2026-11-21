//! The keys of the book editor (CE-403): every editor action with its default accelerators, in the
//! one registry of `shortcuts` (group *Editor*), so they can be changed in Preferences, are checked
//! for conflicts and shown in the command palette. No GTK in here; the editor turns an id into what
//! it does (`ui/editor/keymap.rs`).

use crate::i18n::N_;

pub struct Cmd {
    /// Registry id, `editor.…`.
    pub id: &'static str,
    pub title: &'static str,
    pub defaults: &'static [&'static str],
    pub keywords: &'static str,
    /// Handled before the code view sees the key (function keys the view would swallow).
    pub capture: bool,
    /// Where the palette finds the same command: `editor:ID` (an editor command), or an action name
    /// with an optional `::parameter` (a menu item).
    pub palette: &'static str,
}

macro_rules! c {
    ($id:expr, $t:expr, $d:expr, $k:expr, $p:expr) => {
        Cmd {
            id: $id,
            title: $t,
            defaults: $d,
            keywords: $k,
            capture: false,
            palette: $p,
        }
    };
    (capture $id:expr, $t:expr, $d:expr, $k:expr, $p:expr) => {
        Cmd {
            id: $id,
            title: $t,
            defaults: $d,
            keywords: $k,
            capture: true,
            palette: $p,
        }
    };
}

pub const COMMANDS: &[Cmd] = &[
    c!(
        "editor.save",
        N_("Save"),
        &["<Control>s"],
        "write book",
        "editor:save"
    ),
    c!(
        "editor.close-tab",
        N_("Close Tab"),
        &["<Control>w"],
        "file",
        "editor:close-tab"
    ),
    c!(
        "editor.new-file",
        N_("New File"),
        &["<Control>n"],
        "add create",
        "files.new"
    ),
    c!(
        "editor.quick-open",
        N_("Open File"),
        &["<Control>p"],
        "quick switch",
        "editor:quick-open"
    ),
    c!(
        "editor.palette",
        N_("Command Palette"),
        &["<Control><Shift>p"],
        "commands",
        ""
    ),
    c!(
        "editor.next-file",
        N_("Next File"),
        &["<Control><Alt>Down"],
        "step spine",
        "editor:next-file"
    ),
    c!(
        "editor.previous-file",
        N_("Previous File"),
        &["<Control><Alt>Up"],
        "step spine",
        "editor:previous-file"
    ),
    c!(
        "editor.go-to-line",
        N_("Go to Line"),
        &["<Control>l"],
        "jump",
        "editor:go-to-line"
    ),
    c!(
        "editor.preferences",
        N_("Editor Preferences"),
        &[],
        "settings font tab width colour scheme",
        ""
    ),
    c!(
        "editor.metadata",
        N_("Edit Metadata"),
        &[],
        "title author cover",
        "editor:metadata"
    ),
    c!(
        "editor.sidebar",
        N_("Show or Hide the Sidebar"),
        &["F9"],
        "files contents book panel",
        ""
    ),
    c!(
        "editor.preview",
        N_("Toggle Preview"),
        &["<Control><Alt>v"],
        "live render",
        "editor:preview"
    ),
    c!(
        "editor.beautify",
        N_("Reformat Markup"),
        &[],
        "beautify format pretty indent",
        "editor:beautify"
    ),
    c!(
        "editor.find",
        N_("Find and Replace"),
        &["<Control>f"],
        "search regex",
        "editor:find"
    ),
    c!(
        "editor.replace-bar",
        N_("Find and Replace With Replace Row"),
        &["<Control>h"],
        "search replace regex",
        ""
    ),
    c!(
        "editor.find-next",
        N_("Find Next"),
        &["F3", "<Control>g"],
        "search",
        ""
    ),
    c!(
        "editor.find-previous",
        N_("Find Previous"),
        &["<Shift>F3", "<Control><Shift>g"],
        "search",
        ""
    ),
    c!(
        "editor.replace",
        N_("Replace and Find Next"),
        &["<Control>r", "<Control>bracketright"],
        "search",
        ""
    ),
    c!(
        "editor.replace-previous",
        N_("Replace and Find Previous"),
        &["<Control>bracketleft"],
        "search",
        ""
    ),
    c!(
        "editor.find-all",
        N_("Find All in Book"),
        &["<Control><Shift>f"],
        "search results",
        "editor:find-all"
    ),
    c!(
        "editor.mark",
        N_("Mark Selection"),
        &["<Control><Shift>m"],
        "highlight search in marked text",
        "editor:mark"
    ),
    c!(
        "editor.clear-marks",
        N_("Clear Marked Text"),
        &[],
        "highlight remove",
        "editor:clear-marks"
    ),
    c!(
        "editor.insert-snippet",
        N_("Insert Snippet"),
        &["<Control>j"],
        "template expand",
        "editor:insert-snippet"
    ),
    c!(
        "editor.jump-matching-tag",
        N_("Jump to Matching Tag"),
        &["<Control><Alt>j"],
        "markup",
        "edit.tag::jump"
    ),
    c!(
        "editor.select-matching-tag",
        N_("Select Matching Tag"),
        &["<Control><Alt>k"],
        "markup",
        "edit.tag::select"
    ),
    c!(
        "editor.close-tag",
        N_("Close Current Tag"),
        &["<Control><Alt>period"],
        "markup",
        "edit.tag::close"
    ),
    c!(
        "editor.wrap-tag",
        N_("Wrap Selection in Tag"),
        &["<Control><Alt>w", "<Control>less"],
        "markup surround",
        "edit.tag::wrap"
    ),
    c!(
        "editor.unwrap-tag",
        N_("Remove Surrounding Tag"),
        &["<Control><Alt>u", "<Control>greater"],
        "markup",
        "edit.tag::unwrap"
    ),
    c!(
        "editor.split-tag",
        N_("Split Tag at Caret"),
        &["<Control><Alt>greater"],
        "markup divide paragraph element",
        "edit.code::split"
    ),
    c!(
        "editor.format-bold",
        N_("Bold"),
        &["<Control>b"],
        "format strong b",
        "edit.code::bold"
    ),
    c!(
        "editor.format-italic",
        N_("Italic"),
        &["<Control>i"],
        "format emphasis em i",
        "edit.code::italic"
    ),
    c!(
        "editor.format-underline",
        N_("Underline"),
        &["<Control>u"],
        "format u",
        "edit.code::underline"
    ),
    c!(
        "editor.format-strike",
        N_("Strikethrough"),
        &[],
        "format strike del s",
        "edit.code::strike"
    ),
    c!(
        "editor.format-sup",
        N_("Superscript"),
        &[],
        "format sup footnote",
        "edit.code::sup"
    ),
    c!(
        "editor.format-sub",
        N_("Subscript"),
        &[],
        "format sub",
        "edit.code::sub"
    ),
    c!(
        "editor.format-code",
        N_("Code"),
        &[],
        "format monospace tt",
        "edit.code::code"
    ),
    c!(
        capture "editor.insert-link",
        N_("Insert Hyperlink"),
        &["<Control>k"],
        "link href anchor",
        "edit.code::link"
    ),
    c!(
        "editor.insert-image",
        N_("Insert Image"),
        &["<Control><Alt>m"],
        "picture img",
        "edit.code::image"
    ),
    c!(
        "editor.colour-text",
        N_("Text Colour"),
        &[],
        "colour color font span",
        "edit.code::colour-text"
    ),
    c!(
        "editor.colour-background",
        N_("Background Colour"),
        &[],
        "colour color highlight span",
        "edit.code::colour-background"
    ),
    c!(
        "editor.align-left",
        N_("Align Left"),
        &[],
        "text-align paragraph",
        "edit.code::align-left"
    ),
    c!(
        "editor.align-center",
        N_("Align Centre"),
        &[],
        "text-align paragraph center",
        "edit.code::align-center"
    ),
    c!(
        "editor.align-right",
        N_("Align Right"),
        &[],
        "text-align paragraph",
        "edit.code::align-right"
    ),
    c!(
        "editor.align-justify",
        N_("Justify"),
        &[],
        "text-align paragraph",
        "edit.code::align-justify"
    ),
    c!(
        "editor.go-to-definition",
        N_("Go to Definition"),
        &["F12"],
        "jump link class anchor ctrl click",
        "edit.code::goto"
    ),
    c!(
        "editor.sort-css",
        N_("Sort CSS Rules"),
        &[],
        "stylesheet order",
        "edit.code::sort-css"
    ),
    c!(
        "editor.rename-tag",
        N_("Rename Tag"),
        &["<Control><Alt>r"],
        "markup",
        "edit.tag::rename"
    ),
    c!(
        "editor.comment",
        N_("Toggle Comment"),
        &["<Control>slash"],
        "markup",
        "edit.tag::comment"
    ),
    c!(
        "editor.heading-0",
        N_("Make Paragraph"),
        &["<Control>0"],
        "markup heading",
        "edit.tag::heading0"
    ),
    c!(
        "editor.heading-1",
        N_("Make Heading 1"),
        &["<Control>1"],
        "markup",
        "edit.tag::heading1"
    ),
    c!(
        "editor.heading-2",
        N_("Make Heading 2"),
        &["<Control>2"],
        "markup",
        "edit.tag::heading2"
    ),
    c!(
        "editor.heading-3",
        N_("Make Heading 3"),
        &["<Control>3"],
        "markup",
        "edit.tag::heading3"
    ),
    c!(
        "editor.heading-4",
        N_("Make Heading 4"),
        &["<Control>4"],
        "markup",
        "edit.tag::heading4"
    ),
    c!(
        "editor.heading-5",
        N_("Make Heading 5"),
        &["<Control>5"],
        "markup",
        "edit.tag::heading5"
    ),
    c!(
        "editor.heading-6",
        N_("Make Heading 6"),
        &["<Control>6"],
        "markup",
        "edit.tag::heading6"
    ),
    c!(
        capture "editor.check",
        N_("Check Book"),
        &["F7"],
        "validate problems errors",
        "editor:check"
    ),
    c!(
        capture "editor.next-problem",
        N_("Next Problem"),
        &["F8"],
        "check",
        "tools.next-problem"
    ),
    c!(
        capture "editor.previous-problem",
        N_("Previous Problem"),
        &["<Shift>F8"],
        "check",
        "tools.previous-problem"
    ),
    c!(
        "editor.report",
        N_("Reports"),
        &["<Control><Shift>r"],
        "statistics",
        "tools.report"
    ),
    c!(
        "editor.special-characters",
        N_("Insert Special Character"),
        &["<Control><Alt>i"],
        "unicode symbol",
        "view.special"
    ),
    c!(
        "editor.check-spelling",
        N_("Check Spelling"),
        &["<Control><Shift>F7"],
        "spell dictionary",
        "tools.spelling"
    ),
    c!(
        "editor.next-misspelling",
        N_("Next Misspelled Word"),
        &["<Control>F8"],
        "spell",
        ""
    ),
    c!(
        "editor.previous-misspelling",
        N_("Previous Misspelled Word"),
        &["<Control><Shift>F8"],
        "spell",
        ""
    ),
    c!(
        "editor.checkpoint",
        N_("Create Checkpoint"),
        &[],
        "snapshot backup",
        "tools.checkpoint"
    ),
    c!(
        "editor.split",
        N_("Split File at Chapter Headings"),
        &[],
        "tools",
        "tools.split"
    ),
    c!(
        "editor.merge",
        N_("Merge with Next File"),
        &[],
        "tools",
        "tools.merge"
    ),
    c!(
        "editor.save-copy",
        N_("Save a Copy"),
        &["<Control><Shift>s"],
        "export",
        ""
    ),
    c!(
        "editor.save-copy-edit",
        N_("Save a Copy and Edit It"),
        &["<Control><Shift><Alt>s"],
        "export",
        ""
    ),
    c!(
        "editor.open-book",
        N_("Open Another Book"),
        &["<Control>o"],
        "load",
        ""
    ),
    c!(
        "editor.undo-checkpoint",
        N_("Undo to the Previous Checkpoint"),
        &["<Control><Alt>z"],
        "revert",
        ""
    ),
    c!(
        "editor.redo-checkpoint",
        N_("Redo to the Next Checkpoint"),
        &["<Control><Shift><Alt>z"],
        "revert",
        ""
    ),
    c!(
        "editor.close-others",
        N_("Close Other Tabs"),
        &["<Control><Shift>w"],
        "tabs",
        ""
    ),
    c!(
        "editor.close-right",
        N_("Close Tabs to the Right"),
        &["<Shift><Alt>w"],
        "tabs",
        ""
    ),
    c!("editor.help", N_("Editor Help"), &["F1"], "manual", ""),
    c!(
        "editor.refresh-preview",
        N_("Refresh the Preview"),
        &["F5"],
        "reload",
        ""
    ),
    c!(
        "editor.copy-preview",
        N_("Copy the Selection of the Preview"),
        &["<Control><Alt>c"],
        "clipboard",
        ""
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shortcuts::{self, Group, Shortcuts};

    /// Ctrl+K is also the application's palette key; in the editor it stays Insert Hyperlink, which
    /// only wins because its controller runs in the capture phase (session scenario editor_palette_keys).
    #[test]
    fn ctrl_k_is_insert_hyperlink_and_runs_before_the_app_palette_key() {
        let link = COMMANDS
            .iter()
            .find(|c| c.id == "editor.insert-link")
            .unwrap();
        assert_eq!(link.defaults, &["<Control>k"]);
        assert!(link.capture);
        let pal = COMMANDS.iter().find(|c| c.id == "editor.palette").unwrap();
        assert_eq!(pal.defaults, &["<Control><Shift>p"]);
    }

    #[test]
    fn ids_are_unique_and_prefixed_and_keys_valid() {
        let mut seen = std::collections::HashSet::new();
        for c in COMMANDS {
            assert!(c.id.starts_with("editor."), "{}", c.id);
            assert!(seen.insert(c.id), "{} twice", c.id);
            for d in c.defaults {
                assert_eq!(
                    shortcuts::canonical(d).as_deref(),
                    Some(*d),
                    "{}: {d} is not in canonical spelling",
                    c.id
                );
            }
        }
    }

    #[test]
    fn no_default_key_is_used_twice_in_the_editor() {
        let mut keys = std::collections::HashMap::new();
        for c in COMMANDS {
            for d in c.defaults {
                if let Some(other) = keys.insert(*d, c.id) {
                    panic!("{d} is the default of {other} and {}", c.id);
                }
            }
        }
    }

    #[test]
    fn every_command_is_in_the_registry_under_editor() {
        let reg = shortcuts::registry();
        for c in COMMANDS {
            let a = reg
                .iter()
                .find(|a| a.action == c.id)
                .unwrap_or_else(|| panic!("{} is not registered", c.id));
            assert_eq!(a.group, Group::Editor);
        }
        assert_eq!(
            Shortcuts::default().all_conflicts(),
            vec![],
            "defaults of all windows together"
        );
    }

    #[test]
    fn rebinding_an_editor_key_wins_and_frees_the_default() {
        let mut s = Shortcuts::default();
        assert_eq!(s.effective("editor.save"), vec!["<Control>s".to_string()]);
        assert!(s.bind("editor.save", "<Control><Alt>x").is_ok());
        assert_eq!(
            s.effective("editor.save"),
            vec!["<Control><Alt>x".to_string()]
        );
        // the freed key can be given to another editor action
        assert!(s.bind("editor.close-tab", "<Control>s").is_ok());
        let back = Shortcuts::from_json(&s.to_json()).unwrap().0;
        assert_eq!(back.effective("editor.save"), s.effective("editor.save"));
        assert_eq!(
            back.effective("editor.close-tab"),
            vec!["<Control>s".to_string()]
        );
    }

    #[test]
    fn a_key_taken_in_the_editor_is_a_conflict_but_not_with_the_reader() {
        let s = Shortcuts::default();
        let c = s.conflicts("editor.close-tab", "<Control>s");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].action, "editor.save");
        // the reader window never sees editor keys, so Ctrl+T (reader contents) may be used
        assert!(s.conflicts("editor.close-tab", "<Control>t").is_empty());
        let mut s = s;
        assert!(s.bind("editor.close-tab", "<Control>s").is_err());
        assert!(s.bind_settled("editor.close-tab", "<Control>s", shortcuts::Settle::Swap));
        assert_eq!(s.effective("editor.save"), vec!["<Control>w".to_string()]);
    }

    #[test]
    fn palette_targets_name_existing_ids() {
        for c in COMMANDS {
            if let Some(id) = c.palette.strip_prefix("editor:") {
                assert!(
                    crate::palette::EDITOR_COMMANDS.iter().any(|e| e.0 == id),
                    "{id} is not a palette command"
                );
            }
        }
    }
}
