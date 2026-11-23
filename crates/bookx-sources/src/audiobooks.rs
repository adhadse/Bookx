//! Audiobook pairing: finds `.m4b` files and folders of `.mp3`/`.m4a` parts and pairs them with
//! library books by identifier, title and author. The pairing only links; Bookx plays nothing
//! (the details panel opens the item with the system player).

use std::path::{Path, PathBuf};

use bookx_core::library::Book;

use crate::authors::{same_author, to_first_last};
use crate::dedup::title_key;
use crate::isbn;
use crate::text::normalize;

const SINGLE: &[&str] = &["m4b"];
const PARTS: &[&str] = &["mp3", "m4a", "ogg", "opus", "flac", "aac"];

/// An audiobook found on disk: one `.m4b` file or a folder of parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub path: PathBuf,
    pub title_guess: String,
    pub author_guess: String,
    pub isbn: Option<String>,
    pub files: usize,
    pub bytes: u64,
}

fn ext(p: &Path) -> String {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default()
}

/// `Author - Title`, `Title (Author)` or just `Title`. Underscores and a trailing `(Unabridged)`
/// style note are dropped.
pub fn guess_names(name: &str) -> (String, String) {
    let mut n = name.replace('_', " ");
    for junk in [
        "(unabridged)",
        "[unabridged]",
        "(audiobook)",
        "[audiobook]",
        "(abridged)",
    ] {
        if let Some(i) = n.to_lowercase().find(junk) {
            n.replace_range(i..i + junk.len(), "");
        }
    }
    let n = n.trim().to_string();
    if let Some((a, t)) = n.split_once(" - ") {
        return (a.trim().to_string(), t.trim().to_string());
    }
    if let (Some(open), true) = (n.rfind('('), n.ends_with(')')) {
        return (
            n[open + 1..n.len() - 1].trim().to_string(),
            n[..open].trim().to_string(),
        );
    }
    (String::new(), n)
}

fn isbn_in(text: &str) -> Option<String> {
    isbn::extract_isbns(text).into_iter().next()
}

/// Looks for audiobooks below `dir`, at most `depth` folders deep. A folder with at least one
/// `.m4b` or at least two audio parts is one audiobook (its parts do not become candidates of
/// their own); a lone `.m4b` file is one too. Names come from the folder, the author from the
/// parent folder when the folder name does not say.
pub fn scan(dir: &Path, depth: usize) -> Vec<Candidate> {
    let mut out = Vec::new();
    scan_into(dir, depth, &mut out);
    out
}

fn scan_into(dir: &Path, depth: usize, out: &mut Vec<Candidate>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    let files: Vec<PathBuf> = entries.iter().filter(|p| p.is_file()).cloned().collect();
    let singles: Vec<PathBuf> = files
        .iter()
        .filter(|p| SINGLE.contains(&ext(p).as_str()))
        .cloned()
        .collect();
    let parts: Vec<PathBuf> = files
        .iter()
        .filter(|p| PARTS.contains(&ext(p).as_str()))
        .cloned()
        .collect();
    if parts.len() + singles.len() >= 2 {
        // a folder of parts (or several m4b chapters): one audiobook
        let all: Vec<&PathBuf> = singles.iter().chain(parts.iter()).collect();
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let (mut author, title) = guess_names(&name);
        if author.is_empty() {
            author = dir
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
        }
        out.push(Candidate {
            path: dir.to_path_buf(),
            isbn: isbn_in(&name),
            title_guess: title,
            author_guess: author,
            files: all.len(),
            bytes: all
                .iter()
                .filter_map(|p| std::fs::metadata(p).ok())
                .map(|m| m.len())
                .sum(),
        });
        return;
    }
    for s in &singles {
        let stem = s
            .file_stem()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let (mut author, title) = guess_names(&stem);
        if author.is_empty() {
            // inside "Author/Title.m4b" the folder is the author, but only when it is not a library book folder
            author = dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
        }
        out.push(Candidate {
            path: s.clone(),
            isbn: isbn_in(&stem),
            title_guess: title,
            author_guess: author,
            files: 1,
            bytes: std::fs::metadata(s).map(|m| m.len()).unwrap_or(0),
        });
    }
    if depth == 0 {
        return;
    }
    for p in entries.iter().filter(|p| p.is_dir()) {
        let hidden = p
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'));
        if !hidden {
            scan_into(p, depth - 1, out);
        }
    }
}

/// How a candidate was matched to a book.
#[derive(Debug, Clone, PartialEq)]
pub struct Pairing {
    pub book: u64,
    pub candidate: usize,
    pub score: f64,
    pub reason: &'static str,
}

fn book_isbns(b: &Book) -> Vec<String> {
    b.meta
        .identifiers
        .iter()
        .filter_map(|i| isbn::canonical(&i.value))
        .collect()
}

/// Pairs candidates with books, one candidate per book and one book per candidate, best score
/// first. `book_dir` gives the absolute folder of a book when it has one of its own (an audiobook
/// inside that folder is paired with certainty). Scores below 0.8 are dropped: a wrong link is
/// worse than none.
pub fn pair(
    books: &[Book],
    candidates: &[Candidate],
    book_dir: &dyn Fn(&Book) -> Option<PathBuf>,
) -> Vec<Pairing> {
    use std::collections::{BTreeMap, HashMap, HashSet};
    // indexes over the books, so a library of tens of thousands is not compared one by one
    let mut by_dir: HashMap<PathBuf, Vec<usize>> = HashMap::new();
    let mut by_isbn: HashMap<String, Vec<usize>> = HashMap::new();
    let mut by_title: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, b) in books.iter().enumerate() {
        if let Some(d) = book_dir(b) {
            by_dir.entry(d).or_default().push(i);
        }
        for n in book_isbns(b) {
            by_isbn.entry(n).or_default().push(i);
        }
        let k = title_key(&b.meta.title).main;
        if !k.is_empty() {
            by_title.entry(k).or_default().push(i);
        }
    }
    let mut all: Vec<Pairing> = Vec::new();
    for (ci, c) in candidates.iter().enumerate() {
        // book index -> (score, reason); the first rule that matches a book wins, as before
        let mut hits: BTreeMap<usize, (f64, &'static str)> = BTreeMap::new();
        for dir in c.path.ancestors() {
            for &i in by_dir.get(dir).into_iter().flatten() {
                hits.entry(i).or_insert((1.0, "in the book's folder"));
            }
        }
        if let Some(key) = c.isbn.as_deref().and_then(isbn::canonical) {
            for &i in by_isbn.get(&key).into_iter().flatten() {
                hits.entry(i).or_insert((0.99, "same ISBN"));
            }
        }
        let ctitle = title_key(&c.title_guess).main;
        if !ctitle.is_empty() {
            for &i in by_title.get(&ctitle).into_iter().flatten() {
                if hits.contains_key(&i) {
                    continue;
                }
                let b = &books[i];
                let author_ok = c.author_guess.is_empty()
                    || b.meta.authors.is_empty()
                    || b.meta.authors.iter().any(|a| {
                        same_author(a, &c.author_guess)
                            || normalize(&to_first_last(a))
                                == normalize(&to_first_last(&c.author_guess))
                    });
                if author_ok {
                    hits.insert(i, (0.9, "same title and author"));
                }
            }
        }
        for (i, (score, reason)) in hits {
            if score >= 0.8 {
                all.push(Pairing {
                    book: books[i].id,
                    candidate: ci,
                    score,
                    reason,
                });
            }
        }
    }
    all.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.book.cmp(&b.book)));
    let mut used_books = HashSet::new();
    let mut used_cands = HashSet::new();
    let mut out = Vec::new();
    for p in all {
        if used_books.contains(&p.book) || used_cands.contains(&p.candidate) {
            continue;
        }
        used_books.insert(p.book);
        used_cands.insert(p.candidate);
        out.push(p);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-audio-{}", uuid()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn uuid() -> String {
        format!("{}-{:?}", std::process::id(), std::time::Instant::now())
            .replace([' ', '{', '}', ':', ','], "")
    }

    fn touch(p: &Path, bytes: usize) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, vec![0u8; bytes]).unwrap();
    }

    fn book(id: u64, title: &str, author: &str) -> Book {
        let mut b: Book = serde_json::from_str(
            r#"{"id":1,"file":"a.epub","meta":{"title":"T","title_sort":"T","authors":[],"author_sort":"","publisher":"","language":"en","description":"","tags":[],"series":"","series_index":0.0,"identifiers":[],"published":"","rights":"","rating":0},"added":"2026-01-01T00:00:00Z","modified":"2026-01-01T00:00:00Z","size":1}"#,
        )
        .unwrap();
        b.id = id;
        b.meta.title = title.into();
        b.meta.authors = vec![author.into()];
        b
    }

    #[test]
    fn names_are_guessed_from_common_patterns() {
        assert_eq!(
            guess_names("Frank Herbert - Dune (Unabridged)"),
            ("Frank Herbert".into(), "Dune".into())
        );
        assert_eq!(
            guess_names("Dune (Frank Herbert)"),
            ("Frank Herbert".into(), "Dune".into())
        );
        assert_eq!(
            guess_names("The_Hobbit"),
            (String::new(), "The Hobbit".into())
        );
    }

    #[test]
    fn m4b_files_and_part_folders_are_found_but_parts_are_not_listed_twice() {
        let d = tmp();
        touch(&d.join("Frank Herbert/Dune.m4b"), 100);
        touch(&d.join("Audiobooks/Emma - Jane Austen/01.mp3"), 10);
        touch(&d.join("Audiobooks/Emma - Jane Austen/02.mp3"), 10);
        touch(&d.join("Music/song.mp3"), 10);
        touch(&d.join("Notes/readme.txt"), 1);
        let c = scan(&d, 4);
        assert_eq!(c.len(), 2, "{c:?}");
        let dune = c.iter().find(|c| c.title_guess == "Dune").unwrap();
        assert_eq!(dune.author_guess, "Frank Herbert", "from the parent folder");
        assert_eq!((dune.files, dune.bytes), (1, 100));
        let emma = c.iter().find(|c| c.files == 2).unwrap();
        assert_eq!(emma.author_guess, "Emma", "pattern is Author - Title");
        assert!(emma.path.is_dir());
        assert!(scan(&d, 0).is_empty(), "depth limits the search");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn pairing_uses_the_book_folder_then_isbn_then_title_and_author() {
        let books = vec![
            book(1, "Dune", "Frank Herbert"),
            book(2, "Emma", "Jane Austen"),
            book(3, "The Hobbit", "J.R.R. Tolkien"),
        ];
        let cands = vec![
            Candidate {
                path: PathBuf::from("/lib/Frank Herbert/Dune (1)/audio.m4b"),
                title_guess: "audio".into(),
                author_guess: "Dune (1)".into(),
                isbn: None,
                files: 1,
                bytes: 1,
            },
            Candidate {
                path: PathBuf::from("/x/Emma - Jane Austen"),
                title_guess: "Jane Austen".into(),
                author_guess: "Emma".into(),
                isbn: None,
                files: 3,
                bytes: 1,
            },
            Candidate {
                path: PathBuf::from("/x/Hobbit.m4b"),
                title_guess: "Hobbit, The".into(),
                author_guess: "Someone Else".into(),
                isbn: None,
                files: 1,
                bytes: 1,
            },
        ];
        let dir = |b: &Book| (b.id == 1).then(|| PathBuf::from("/lib/Frank Herbert/Dune (1)"));
        let p = pair(&books, &cands, &dir);
        assert_eq!(p.len(), 1, "{p:?}");
        assert_eq!((p[0].book, p[0].candidate), (1, 0));
        assert_eq!(p[0].reason, "in the book's folder");
        // a title match with the wrong author is not paired; the right author is
        let mut c2 = cands.clone();
        c2[2].author_guess = "J. R. R. Tolkien".into();
        let p = pair(&books, &c2, &dir);
        assert!(
            p.iter()
                .any(|x| x.book == 3 && x.reason == "same title and author")
        );
        // an ISBN in the name wins over everything but the folder
        let mut b2 = books.clone();
        b2[1].meta.identifiers.push(bookx_core::opf::Identifier {
            scheme: "isbn".into(),
            value: "9780141439587".into(),
            xml_id: None,
        });
        c2[1].isbn = Some("9780141439587".into());
        c2[1].title_guess = "x".into();
        let p = pair(&b2, &c2, &dir);
        assert!(p.iter().any(|x| x.book == 2 && x.reason == "same ISBN"));
        // one audiobook is never given to two books
        let twin = vec![
            book(1, "Dune", "Frank Herbert"),
            book(9, "Dune", "Frank Herbert"),
        ];
        let c = vec![Candidate {
            path: PathBuf::from("/x/Dune.m4b"),
            title_guess: "Dune".into(),
            author_guess: "Frank Herbert".into(),
            isbn: None,
            files: 1,
            bytes: 1,
        }];
        assert_eq!(pair(&twin, &c, &|_| None).len(), 1);
    }
}
