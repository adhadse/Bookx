//! Names that the editing crate (and the rule kinds) return from `label()` functions: the crates cannot
//! translate, the dialogs run them through `tr`. They are listed here so that they are extracted into the
//! template; the test keeps the list in step with the sources.

use crate::i18n::N_;

#[allow(dead_code)]
pub const LABELS: [&str; 52] = [
    N_("English “double”"),
    N_("German „double“"),
    N_("French « double »"),
    N_("Swedish ”double”"),
    N_("Image"),
    N_("Font"),
    N_("Audio"),
    N_("Video"),
    N_("Blank page (XHTML)"),
    N_("Chapter with a heading"),
    N_("Title page"),
    N_("Stylesheet (CSS)"),
    N_("Embedding allowed"),
    N_("Embedding allowed (editable documents)"),
    N_("Embedding allowed for viewing and printing only"),
    N_("The font's licence forbids embedding"),
    N_("The font does not say whether it may be embedded"),
    N_("Cover"),
    N_("Title Page"),
    N_("Table of Contents"),
    N_("Start of Content"),
    N_("Foreword"),
    N_("Preface"),
    N_("Copyright"),
    N_("Dedication"),
    N_("Epigraph"),
    N_("Acknowledgements"),
    N_("Index"),
    N_("Glossary"),
    N_("Bibliography"),
    N_("Colophon"),
    N_("Notes"),
    N_("List of Illustrations"),
    N_("List of Tables"),
    N_("Rename the tag"),
    N_("Remove the tag, keep its content"),
    N_("Remove the element and its content"),
    N_("Set an attribute"),
    N_("Remove an attribute"),
    N_("Replace text in an attribute"),
    N_("Wrap in a new element"),
    N_("Add a class"),
    N_("Remove a class"),
    N_("Change a class"),
    N_("If a property has a value, change it"),
    N_("Set a property everywhere"),
    N_("Remove a property"),
    N_("Remove a property when it has a value"),
    N_("Multiply all font sizes"),
    N_("Make a font family italic"),
    N_("Rename a font family"),
    N_("Change one colour into another"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_label_of_the_editing_crate_is_listed_for_translation() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut missing = Vec::new();
        let mut files: Vec<std::path::PathBuf> =
            std::fs::read_dir(root.join("crates/bookx-editing/src"))
                .unwrap()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "rs"))
                .collect();
        files.push(root.join("src/editor_tools/rules.rs"));
        for f in files {
            let s = std::fs::read_to_string(&f).unwrap();
            for (at, _) in s.match_indices("pub fn label(self) -> &'static str {") {
                let rest = &s[at..];
                let end = rest.find("\n    }\n").unwrap_or(rest.len());
                for part in rest[..end].split("=> \"").skip(1) {
                    let text = part.split('"').next().unwrap();
                    if !LABELS.contains(&text) {
                        missing.push(format!("{}: {text}", f.display()));
                    }
                }
            }
        }
        assert!(missing.is_empty(), "add these to LABELS: {missing:#?}");
    }
}
