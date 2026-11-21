//! Performance budgets for the typography hot paths. Same rules as
//! `crates/bookx-core/tests/perf_budgets.rs`: release budgets, debug gets 30x, and
//! `BOOKX_PERF_SCALE` scales them.

use std::time::{Duration, Instant};

use bookx_text::css::StyleSheetSet;
use bookx_text::hyphenate::Hyphenator;
use bookx_text::linebreak::{break_greedy, break_paragraph, items_from_text, Params};

fn budget(release_ms: u64) -> Duration {
    let scale: f64 = std::env::var("BOOKX_PERF_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0);
    let f = if cfg!(debug_assertions) { 30.0 } else { 1.0 };
    Duration::from_micros((release_ms as f64 * 1000.0 * f * scale) as u64)
}

fn check(what: &str, took: Duration, max: Duration) {
    assert!(took <= max, "{what}: took {took:?}, budget {max:?}");
}

fn words(n: usize) -> String {
    const W: &[&str] = &[
        "extraordinary",
        "the",
        "typography",
        "of",
        "beautiful",
        "paragraphs",
        "requires",
        "patience",
        "and",
        "hyphenation",
        "dictionaries",
        "for",
        "every",
        "language",
    ];
    (0..n).map(|i| W[i % W.len()]).collect::<Vec<_>>().join(" ")
}

#[test]
fn hyphenating_10k_words_is_fast() {
    let h = Hyphenator::for_language("en-US");
    let text = words(10_000);
    let t = Instant::now();
    let out = h.insert_soft_hyphens(&text);
    check("hyphenating 10 000 words", t.elapsed(), budget(60));
    assert!(out.contains('\u{ad}'));
}

#[test]
fn breaking_a_300_word_paragraph_is_fast() {
    let h = Hyphenator::for_language("en-US");
    let text = words(300);
    let measure = |s: &str| s.chars().count() as f64 * 7.0;
    let items = items_from_text(&text, &measure, Some(&h), 50.0);
    let t = Instant::now();
    let lines = break_paragraph(&items, 560.0, &Params::default());
    check("Knuth-Plass over 300 words", t.elapsed(), budget(30));
    assert!(lines.len() > 5);
    let t = Instant::now();
    let greedy = break_greedy(&items, 560.0, &Params::default());
    check("greedy breaking over 300 words", t.elapsed(), budget(10));
    assert!(!greedy.is_empty());
}

#[test]
fn breaking_a_whole_chapter_stays_linear_enough() {
    let measure = |s: &str| s.chars().count() as f64 * 7.0;
    let h = Hyphenator::for_language("en-US");
    let t = Instant::now();
    for _ in 0..100 {
        let items = items_from_text(&words(120), &measure, Some(&h), 50.0);
        let lines = break_paragraph(&items, 560.0, &Params::default());
        assert!(!lines.is_empty());
    }
    check("100 paragraphs of 120 words", t.elapsed(), budget(400));
}

#[test]
fn computing_styles_for_1000_elements_is_fast() {
    let css = "p { margin: 0 0 1em; text-indent: 1.5em } .a { font-style: italic } \
               div > p.b { color: #333 } h1, h2 { font-weight: bold; margin-top: 2em } \
               h2 + p { text-indent: 0 }";
    let mut html = String::from("<html><body>");
    for i in 0..1000 {
        html.push_str(&format!(
            "<div><p class=\"{}\">text {i}</p></div>",
            if i % 2 == 0 { "a" } else { "b" }
        ));
    }
    html.push_str("</body></html>");
    let doc = scraper::Html::parse_document(&html);
    let sel = scraper::Selector::parse("p").unwrap();
    let sheets = StyleSheetSet::new().add_author(css);
    let t = Instant::now();
    let mut n = 0;
    for el in doc.select(&sel) {
        let st = sheets.compute(el);
        n += (st.text_indent > 0.0) as usize;
    }
    check("computing 1000 styles", t.elapsed(), budget(150));
    assert!(n > 0);
}

#[test]
fn planning_optimal_breaks_for_a_chapter_is_fast() {
    use bookx_text::paragraph::{plan, Request};
    let text = words(120);
    let x = |b: usize| text[..b].chars().count() as f64 * 7.0;
    let t = Instant::now();
    for _ in 0..200 {
        let p = plan(&Request {
            text: &text,
            x: &x,
            hyphen_width: 7.0,
            first_width: 560.0,
            width: 560.0,
            hyphen_penalty: 50.0,
            head: None,
        })
        .unwrap();
        assert!(p.lines > 3);
    }
    check(
        "200 paragraphs of 120 words planned",
        t.elapsed(),
        budget(800),
    );
}

#[test]
fn paginating_a_long_chapter_and_turning_pages_is_fast() {
    use bookx_text::linebreak::paginate_lines;
    let heights = vec![22.0; 20_000];
    let t = Instant::now();
    for _ in 0..20 {
        let pages = paginate_lines(&heights, 600.0, 600.0, 2, 2);
        assert!(pages.len() > 500);
    }
    check("paginating 20 x 20 000 lines", t.elapsed(), budget(400));
}

#[test]
fn widow_and_orphan_control_is_cheap_even_for_one_huge_paragraph() {
    use bookx_text::vertical::{paginate_with, FlowLine};
    // one paragraph of 100 000 lines, and 5 000 paragraphs of 20
    for per in [100_000usize, 20] {
        let lines: Vec<FlowLine> = (0..100_000)
            .map(|i| FlowLine {
                item: i / per,
                line: (i % per) as i32,
                v: i as f64 * 24.0,
                thick: 24.0,
            })
            .collect();
        let t = Instant::now();
        let pages = paginate_with(&lines, 600.0, 3, 3);
        check(
            "widows and orphans over 100 000 lines",
            t.elapsed(),
            budget(60),
        );
        assert!(pages.len() > 3000);
    }
}

#[test]
fn vertical_pagination_kinsoku_and_tate_chu_yoko_scale_to_a_long_book() {
    use bookx_text::vertical::{
        combine_ranges, paginate, squeezed, violations, FlowLine, Frame, TextCombine, WritingMode,
    };
    // 100 000 lines in 5 000 paragraphs
    let lines: Vec<FlowLine> = (0..100_000)
        .map(|i| FlowLine {
            item: i / 20,
            line: (i % 20) as i32,
            v: i as f64 * 24.0,
            thick: 24.0,
        })
        .collect();
    let t = Instant::now();
    let pages = paginate(&lines, 600.0);
    check(
        "cutting 100 000 columns into pages",
        t.elapsed(),
        budget(60),
    );
    assert!(pages.len() > 3000);

    let text =
        "「吾輩は猫である。」「名前はまだ無い。」平成12年3月24日、（見当がつかぬ）。".repeat(3000);
    let t = Instant::now();
    let n = squeezed(&text).len();
    let r = combine_ranges(&text, TextCombine::Digits(2)).len();
    check(
        "squeezing and combining 100 000 characters",
        t.elapsed(),
        budget(60),
    );
    assert!(n > 1000 && r > 1000);

    let rows: Vec<String> = (0..100_000)
        .map(|i| format!("吾輩は猫である{i}。"))
        .collect();
    let refs: Vec<&str> = rows.iter().map(String::as_str).collect();
    let t = Instant::now();
    let v = violations(&refs);
    check("checking 100 000 line breaks", t.elapsed(), budget(60));
    assert!(v.len() < refs.len());

    let f = Frame::new(WritingMode::VerticalRl, (10.0, 20.0, 400.0, 700.0), 1000.0);
    let t = Instant::now();
    let mut acc = 0.0;
    for i in 0..1_000_000 {
        let (x, y, ..) = f.rect(i as f64 % 600.0, 1000.0 + i as f64 % 400.0, 20.0, 30.0);
        let (u, v) = f.locate(x, y);
        acc += u + v;
    }
    check(
        "a million vertical rectangles mapped and located",
        t.elapsed(),
        budget(120),
    );
    assert!(acc > 0.0);
}
