//! Writes the PDFs the UI tests open: `make_test_pdfs DIR`.
//!
//! * `story.pdf`: 8 pages with an outline, roman then arabic page labels, a link, metadata
//! * `locked.pdf`: the password is `sesame`
//! * `scan.pdf`: pages without a text layer
//! * `big.pdf`: 500 pages (open-time budget); `BIG=0` skips it

use bookx_pdf::testpdf::{OutlineEntry, Page, Target, TestPdf};

fn main() {
    let dir = std::env::args().nth(1).expect("usage: make_test_pdfs DIR");
    std::fs::create_dir_all(&dir).unwrap();
    let w = |name: &str, bytes: Vec<u8>| std::fs::write(format!("{dir}/{name}"), bytes).unwrap();

    let mut story = TestPdf::new().title("The Fox Papers").author("Ada Reader");
    for i in 0..8 {
        let mut p = Page::new(612.0, 792.0)
            .text(72.0, 720.0, 24.0, &format!("Chapter {}", i + 1))
            .text(
                72.0,
                680.0,
                12.0,
                "The quick brown fox jumps over the lazy dog and keeps",
            )
            .text(
                72.0,
                664.0,
                12.0,
                &format!("running through the forest of page {} until night.", i + 1),
            )
            .text(
                72.0,
                640.0,
                12.0,
                "Foxes are small wild animals with pointed noses.",
            );
        if i == 0 {
            p = p
                .link([72.0, 620.0, 300.0, 636.0], Target::Page(5))
                .text(72.0, 624.0, 12.0, "Jump to the end of the story")
                .link(
                    [72.0, 590.0, 300.0, 606.0],
                    Target::Uri("https://example.org/foxes".into()),
                )
                .text(72.0, 594.0, 12.0, "Read about foxes online");
        }
        if i == 4 {
            p = p.text(72.0, 600.0, 12.0, "A rare word: zyzzyva appears only here.");
        }
        story = story.page(p);
    }
    let story = story
        .outline(OutlineEntry::new("Beginning", 0).with(OutlineEntry::new("The first night", 1)))
        .outline(OutlineEntry::new("Middle", 3))
        .outline(OutlineEntry::new("End", 6))
        .label(0, "r", "", 1)
        .label(2, "D", "", 1);
    w("story.pdf", story.build());

    w(
        "locked.pdf",
        TestPdf::simple(&[
            &["Sealed content about owls and secrets."],
            &["Second sealed page."],
        ])
        .title("Locked Notes")
        .password("sesame")
        .build(),
    );

    w(
        "scan.pdf",
        TestPdf::new()
            .title("Scanned Pages")
            .page(Page::new(500.0, 700.0).rect([40.0, 40.0, 460.0, 660.0], 0.6))
            .page(Page::new(500.0, 700.0).rect([60.0, 60.0, 440.0, 640.0], 0.4))
            .build(),
    );

    // narrow text block on a large page: for crop margins
    w(
        "margins.pdf",
        TestPdf::new()
            .title("Wide Margins")
            .page(
                Page::new(612.0, 792.0)
                    .rect([200.0, 300.0, 400.0, 500.0], 0.0)
                    .text(210.0, 480.0, 12.0, "Text far from the paper edge"),
            )
            .page(Page::new(612.0, 792.0).rect([210.0, 310.0, 390.0, 490.0], 0.0))
            .build(),
    );

    if std::env::var("BIG").as_deref() != Ok("0") {
        let mut big = TestPdf::new().title("Five Hundred Pages");
        for i in 0..500 {
            let mut p =
                Page::new(612.0, 792.0).text(72.0, 740.0, 18.0, &format!("Section {}", i + 1));
            for j in 0..40 {
                p = p.text(
                    72.0,
                    700.0 - 16.0 * j as f32,
                    11.0,
                    &format!("Line {j} of page {}: pack my box with five dozen liquor jugs and then some more words.", i + 1),
                );
            }
            big = big.page(p);
        }
        for k in 0..25 {
            big = big.outline(OutlineEntry::new(&format!("Part {}", k + 1), k * 20));
        }
        w("big.pdf", big.build());
    }
}
