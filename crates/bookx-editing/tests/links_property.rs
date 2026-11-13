//! Property tests: random moves and arrangements never break links, never change text.

mod common;

use bookx_core::book::OpenBook;
use bookx_editing::links::{arrange_files, move_file, ArrangeOptions};

struct Lcg(u64);
impl Lcg {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % n
    }
}

fn random_book(r: &mut Lcg) -> OpenBook {
    let n = 3 + r.next(4) as usize;
    let bodies: Vec<String> = (1..=n)
        .map(|c| {
            let mut s =
                format!("<h1 id=\"top{c}\">Chapter {c}</h1><p>Text of chapter {c} &amp; more.</p>");
            for _ in 0..(1 + r.next(3)) {
                let t = 1 + r.next(n as u64);
                s.push_str(&format!(
                    "<p>see <a href=\"chapter-{t}.xhtml#top{t}\">chapter {t}</a></p>"
                ));
            }
            if r.next(2) == 0 {
                s.push_str("<p><img src=\"images/pic.png\" alt=\"pic\"/></p>");
            }
            if r.next(3) == 0 {
                s.push_str("<p><img src=\"images/other.png\" alt=\"other\"/></p>");
            }
            s
        })
        .collect();
    let mut b = common::book_from_bodies(&bodies);
    b.archive.set("OEBPS/images/pic.png", common::png(4, 4));
    b.archive.set("OEBPS/images/other.png", common::png(5, 5));
    b.archive.set_string("OEBPS/style.css", "@import url(extra.css);\nbody { background: url(images/pic.png) }\n@font-face { font-family: F; src: url(fonts/f.ttf) }\n");
    b.archive.set_string(
        "OEBPS/extra.css",
        "p { background: url(images/other.png) }\n",
    );
    b.archive
        .set("OEBPS/fonts/f.ttf", bookx_editing::testfont::tiny_ttf(0));
    let pkg = b.package.clone();
    for (id, f, m) in [
        ("pic", "images/pic.png", "image/png"),
        ("oth", "images/other.png", "image/png"),
        ("extra", "extra.css", "text/css"),
        ("font", "fonts/f.ttf", "font/ttf"),
    ] {
        pkg.add_manifest_item(&mut b.archive, id, &format!("OEBPS/{f}"), m, "")
            .unwrap();
    }
    b.reload_package().unwrap();
    // images that a chapter does not use are fine; those referenced must exist, so drop unused manifest entries
    b
}

#[test]
fn random_moves_then_arrange_keep_the_book_valid() {
    let dirs = [
        "OEBPS/a",
        "OEBPS/a/b",
        "OEBPS/z y",
        "OEBPS",
        "OEBPS/Text/deep/er",
    ];
    let mut total_moves = 0;
    for seed in 1..=25u64 {
        let mut r = Lcg(seed * 104729);
        let mut b = random_book(&mut r);
        common::validate_epub(&b.archive.to_bytes().unwrap());
        let text = common::spine_text(&b);
        let toc: Vec<String> = b.toc().into_iter().map(|t| t.title).collect();
        for _ in 0..(2 + r.next(6)) {
            let names: Vec<String> = b
                .package
                .manifest
                .iter()
                .map(|i| b.package.item_path(i))
                .filter(|p| b.archive.contains(p))
                .collect();
            let from = names[r.next(names.len() as u64) as usize].clone();
            let dir = dirs[r.next(dirs.len() as u64) as usize];
            let file = from.rsplit('/').next().unwrap();
            let to = format!("{dir}/{file}");
            match move_file(&mut b, &from, &to) {
                Ok(_) => total_moves += 1,
                Err(e) => assert!(e.to_string().contains("already exists"), "seed {seed}: {e}"),
            }
            common::validate_epub(&b.archive.to_bytes().unwrap());
        }
        assert_eq!(
            common::spine_text(&b),
            text,
            "seed {seed}: moves changed the text"
        );
        arrange_files(&mut b, &ArrangeOptions::default()).unwrap();
        common::validate_epub(&b.archive.to_bytes().unwrap());
        assert_eq!(
            common::spine_text(&b),
            text,
            "seed {seed}: arrange changed the text"
        );
        assert_eq!(
            b.toc().into_iter().map(|t| t.title).collect::<Vec<_>>(),
            toc
        );
        let second = arrange_files(&mut b, &ArrangeOptions::default()).unwrap();
        assert!(second.moved.is_empty(), "seed {seed}: not idempotent");
        let other = ArrangeOptions {
            text: "xhtml".into(),
            images: "assets/img".into(),
            styles: "assets/css".into(),
            fonts: "assets/fonts".into(),
            ..Default::default()
        };
        arrange_files(&mut b, &other).unwrap();
        common::validate_epub(&b.archive.to_bytes().unwrap());
        assert_eq!(common::spine_text(&common::reopen(&b)), text);
    }
    assert!(total_moves > 50, "only {total_moves} moves happened");
}
