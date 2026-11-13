//! Spell checking a real EPUB with dictionary files read from disk.

mod common;

use bookx_editing::spell::{apply_replacement, check_book_spelling, replace_in_book, SpellChecker};

const AFF: &str = "SET UTF-8\nTRY esianrtolcdugmphbyfvkwz\nSFX S Y 1\nSFX S 0 s .\n";
const DIC: &str =
    "14\nthe\nquick\nbrown\nfox/S\njumps\nover\nlazy\ndog/S\nand\nchapter\nof\ntext\nwords\nboth\n";

#[test]
fn dictionary_files_check_replace_and_user_words() {
    let dir = common::unique_dir("spell");
    std::fs::write(dir.join("en_US.aff"), AFF).unwrap();
    std::fs::write(dir.join("en_US.dic"), DIC).unwrap();
    let mut checker = SpellChecker::new();
    checker
        .add_dictionary_files("en_US", &dir.join("en_US.aff"), &dir.join("en_US.dic"))
        .unwrap();

    let mut book = common::book_from_bodies(&[
        "<p>The quikc brown foxes jumps ovr the lazy dogs. Bookx and <code>notaword</code> and <span lang=\"fr\">bonjour</span>.</p>".to_string(),
        "<p>Another chapter of text with wrods &amp; more quikc words.</p>".to_string(),
    ]);
    let report = check_book_spelling(&book, &checker);
    let chapter_words: Vec<&str> = report
        .misspellings
        .iter()
        .filter(|m| m.file.starts_with("OEBPS/chapter"))
        .map(|m| m.word.as_str())
        .collect();
    assert_eq!(
        chapter_words,
        vec!["quikc", "foxes", "ovr", "Bookx", "Another", "with", "wrods", "more", "quikc"]
    );
    assert!(report.unavailable_languages.contains("fr"));
    assert!(!chapter_words.contains(&"notaword"), "code is not checked");
    assert!(!chapter_words.contains(&"bonjour"));

    // suggestions
    assert!(checker
        .suggest("en", "quikc")
        .contains(&"quick".to_string()));

    // fix one occurrence by offset, the rest through replace-all
    let m = report
        .misspellings
        .iter()
        .find(|m| m.word == "ovr")
        .unwrap();
    let src = book.archive.get_string(&m.file).unwrap();
    let fixed = apply_replacement(&src, m, "over").unwrap();
    book.archive.set_string(&m.file, &fixed);
    assert!(fixed.contains("over the lazy"));
    assert_eq!(
        replace_in_book(&mut book, "OEBPS/chapter-1.xhtml", "quikc", "quick").unwrap(),
        1
    );
    assert_eq!(
        replace_in_book(&mut book, "OEBPS/chapter-2.xhtml", "quikc", "quick").unwrap(),
        1
    );

    checker.add_user_word("Bookx");
    let again = check_book_spelling(&book, &checker);
    let words: Vec<&str> = again
        .misspellings
        .iter()
        .filter(|m| m.file.starts_with("OEBPS/chapter"))
        .map(|m| m.word.as_str())
        .collect();
    assert_eq!(words, vec!["foxes", "Another", "with", "wrods", "more"]);
    // the edited files are still valid XML
    common::validate_epub(&book.archive.to_bytes().unwrap());
    std::fs::remove_dir_all(dir).ok();
}
