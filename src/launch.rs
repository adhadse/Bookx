//! What the command line asks Bookx to do (X20), without GTK: `bookx FILE…` (Open With: import), `bookx
//! --doc FILE` (import if needed, then open in the reader), `bookx bookx://…` links, `--edit` (handled by
//! `editor_state`) and `--safe-mode`.
//!
//! A second `bookx` process hands links, files and documents to the running one through application
//! actions (`app.open-uri`, `app.open-files`, `app.open-doc`) and exits; relative paths are made absolute
//! first, because the running process has another working directory.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Request {
    /// `bookx://…` links.
    pub links: Vec<String>,
    /// Files to import (Open With, `bookx a.epub b.epub`).
    pub files: Vec<PathBuf>,
    /// `--doc FILE`: open in the reader.
    pub docs: Vec<PathBuf>,
    /// `--edit` is present: the editor request handles the arguments.
    pub edit: bool,
    pub safe_mode: bool,
}

impl Request {
    /// Whether there is anything to hand to a running instance.
    pub fn is_forwardable(&self) -> bool {
        !(self.edit || (self.links.is_empty() && self.files.is_empty() && self.docs.is_empty()))
    }
}

/// `file:///a%20b.epub` or a plain path as an absolute path (relative to `cwd`).
pub fn path_of(arg: &str, cwd: &Path) -> Option<PathBuf> {
    let p = if let Some(rest) = arg.strip_prefix("file://") {
        // file://host/path is not local: only an empty host or localhost
        let rest = rest.strip_prefix("localhost").unwrap_or(rest);
        if !rest.starts_with('/') {
            return None;
        }
        PathBuf::from(percent_decode(rest)?)
    } else if arg.contains("://") {
        return None;
    } else {
        PathBuf::from(arg)
    };
    Some(if p.is_absolute() { p } else { cwd.join(p) })
}

fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            out.push(u8::from_str_radix(s.get(i + 1..i + 3)?, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Reads the arguments (without the program name). `exists` tells whether a plain argument names a file
/// (kept out of the pure part so tests need no files).
pub fn parse_with(args: &[String], cwd: &Path, exists: &dyn Fn(&Path) -> bool) -> Request {
    let mut r = Request::default();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--safe-mode" {
            r.safe_mode = true;
        } else if a == "--edit" {
            r.edit = true;
            break; // everything after it belongs to the editor request
        } else if a == "--doc" {
            if let Some(p) = args.get(i + 1).and_then(|v| path_of(v, cwd)) {
                r.docs.push(p);
            }
            i += 1;
        } else if let Some(v) = a.strip_prefix("--doc=") {
            if let Some(p) = path_of(v, cwd) {
                r.docs.push(p);
            }
        } else if a.starts_with("--") {
            // not ours (GTK or desktop options)
        } else if crate::links::is_link(a) {
            r.links.push(a.clone());
        } else if let Some(p) = path_of(a, cwd).filter(|p| exists(p)) {
            r.files.push(p);
        }
        i += 1;
    }
    r
}

/// [`parse_with`] for this process.
pub fn parse(args: &[String]) -> Request {
    let cwd = std::env::current_dir().unwrap_or_default();
    parse_with(args, &cwd, &|p| p.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }
    fn all(_: &Path) -> bool {
        true
    }
    const CWD: &str = "/work";

    fn p(v: &[&str]) -> Request {
        parse_with(&a(v), Path::new(CWD), &all)
    }

    #[test]
    fn plain_files_links_and_docs_are_told_apart() {
        let r = p(&[
            "a.epub",
            "/abs/b.epub",
            "bookx://open?id=3",
            "--doc",
            "c.epub",
        ]);
        assert_eq!(
            r.files,
            vec![PathBuf::from("/work/a.epub"), PathBuf::from("/abs/b.epub")]
        );
        assert_eq!(r.links, vec!["bookx://open?id=3"]);
        assert_eq!(r.docs, vec![PathBuf::from("/work/c.epub")]);
        assert!(r.is_forwardable() && !r.edit && !r.safe_mode);
    }

    #[test]
    fn the_doc_file_is_not_also_imported() {
        let r = p(&["--doc", "c.epub"]);
        assert!(r.files.is_empty());
        let r = p(&["--doc=c.epub", "d.epub"]);
        assert_eq!(r.docs, vec![PathBuf::from("/work/c.epub")]);
        assert_eq!(r.files, vec![PathBuf::from("/work/d.epub")]);
    }

    #[test]
    fn file_uris_are_decoded_and_remote_ones_refused() {
        let r = p(&["file:///home/me/My%20Book.epub", "file://localhost/x.epub"]);
        assert_eq!(
            r.files,
            vec![
                PathBuf::from("/home/me/My Book.epub"),
                PathBuf::from("/x.epub")
            ]
        );
        assert!(p(&[
            "file://otherhost/x.epub",
            "https://x/y.epub",
            "file:///a%zz.epub"
        ])
        .files
        .is_empty());
    }

    #[test]
    fn missing_files_and_unknown_options_are_ignored() {
        let none = |_: &Path| false;
        let r = parse_with(
            &a(&["ghost.epub", "--gapplication-service"]),
            Path::new(CWD),
            &none,
        );
        assert_eq!(r, Request::default());
        assert!(!r.is_forwardable());
    }

    #[test]
    fn edit_and_safe_mode_are_recognised_and_edit_is_not_forwarded() {
        let r = p(&["--safe-mode", "--edit", "book.epub", "OEBPS/a.xhtml"]);
        assert!(r.safe_mode && r.edit);
        assert!(
            r.files.is_empty(),
            "the editor request owns what follows --edit"
        );
        assert!(!r.is_forwardable());
    }

    #[test]
    fn a_dangling_doc_option_is_harmless() {
        assert_eq!(p(&["--doc"]), Request::default());
    }
}
