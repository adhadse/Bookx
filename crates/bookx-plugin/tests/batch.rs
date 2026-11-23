//! `batch::run_on_archive`: a whole-book run with the committed trim-whitespace example.

use std::path::PathBuf;

use bookx_core::archive::Archive;
use bookx_plugin::{Host, Manifest, Plugin, batch::run_on_archive};

fn example() -> Plugin {
    let dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins/trim-whitespace");
    let manifest =
        Manifest::parse(&std::fs::read_to_string(dir.join("plugin.toml")).unwrap()).unwrap();
    Plugin::load(&dir, manifest, Host::default()).unwrap()
}

fn book(text: &str) -> Archive {
    let mut a = Archive::default();
    a.set("mimetype", b"application/epub+zip".to_vec());
    a.set("OEBPS/a.xhtml", text.as_bytes().to_vec());
    a
}

#[test]
fn only_the_files_the_plugin_changed_come_back_changed() {
    let a = book("<p>one</p>  \n<p>two</p>\t\n");
    let o = run_on_archive(&example(), "trim", &a, "Trim").unwrap();
    assert_eq!(o.changed_files, vec!["OEBPS/a.xhtml".to_string()]);
    assert_eq!(
        o.archive.get_string("OEBPS/a.xhtml").unwrap(),
        "<p>one</p>\n<p>two</p>\n"
    );
    // the input is a copy: untouched
    assert_eq!(
        a.get_string("OEBPS/a.xhtml").unwrap(),
        "<p>one</p>  \n<p>two</p>\t\n"
    );
    assert_eq!(o.archive.get("mimetype"), a.get("mimetype"));
}

#[test]
fn a_clean_book_reports_no_changes() {
    let a = book("<p>clean</p>\n");
    let o = run_on_archive(&example(), "trim", &a, "Trim").unwrap();
    assert!(o.changed_files.is_empty());
}

#[test]
fn an_unknown_action_is_an_error_and_returns_nothing() {
    let a = book("<p>x</p>  \n");
    let e = run_on_archive(&example(), "nope", &a, "x").unwrap_err();
    assert!(format!("{e:#}").contains("declares no action"));
}
