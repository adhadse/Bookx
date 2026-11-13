//! End-to-end: HTML + CSS -> computed style -> hyphenation -> line breaking -> pages, plus model roundtrips.

use bookx_text::css::{Hyphens, StyleSheetSet, TextAlign};
use bookx_text::highlight::{Highlight, HighlightColor, HighlightSet, HighlightStyle, Range};
use bookx_text::hyphenate::Hyphenator;
use bookx_text::linebreak::{
    break_greedy, break_paragraph, items_from_text, paginate_lines, total_demerits, Params,
};
use bookx_text::pages::{PageMap, PageSource, PageTarget, SpineItem};
use bookx_text::typography::{
    pick_profile, ProfileContext, ProfileRule, ProfileSet, ReadingProfile, Trigger,
    TypographyOverride, TypographySettings,
};
use scraper::{Html, Selector};

fn measure(s: &str) -> f64 {
    s.chars().count() as f64
}

const TEXT: &str = "Typography is the art and technique of arranging type to make written language legible, readable and appealing when displayed. \
    The arrangement of type involves selecting typefaces, point sizes, line lengths, line-spacing, and letter-spacing.";

#[test]
fn css_drives_hyphenation_and_justification() {
    let html = Html::parse_document(&format!(
        "<html><body><p class='body'>{TEXT}</p></body></html>"
    ));
    let sheets = StyleSheetSet::new()
        .add_author(".body { hyphens: auto; text-align: justify; text-indent: 2em }");
    let p = html.select(&Selector::parse("p").unwrap()).next().unwrap();
    let style = sheets.compute(p);
    assert_eq!(style.hyphens, Hyphens::Auto);
    assert_eq!(style.text_align, TextAlign::Justify);

    let hy = (style.hyphens == Hyphens::Auto).then(|| Hyphenator::for_language("en"));
    let items = items_from_text(TEXT, &measure, hy.as_ref(), 50.0);
    let params = Params::default();
    let lines = break_paragraph(&items, 38.0, &params);
    assert!(lines.len() >= 6);
    assert!(lines.iter().all(|l| !l.overfull));
    // text is preserved
    let mut rebuilt = String::new();
    for l in &lines {
        let mut t = l.text(&items);
        if l.hyphenated {
            t.pop();
        } else {
            t.push(' ');
        }
        rebuilt.push_str(&t);
    }
    assert_eq!(rebuilt.trim_end(), TEXT);
    // every non-last line is justified to the exact width
    for l in &lines[..lines.len() - 1] {
        let g = l.glue_widths(&items);
        let total = l.natural_width - g.len() as f64 + g.iter().sum::<f64>();
        assert!((total - 38.0).abs() < 1e-6, "{}: {total}", l.text(&items));
    }
}

#[test]
fn optimal_never_worse_than_greedy_on_real_text() {
    let items = items_from_text(TEXT, &measure, Some(&Hyphenator::for_language("en")), 50.0);
    let params = Params {
        tolerance: 1e9,
        consecutive_hyphen_limit: None,
        ..Params::default()
    };
    for width in [20.0, 24.0, 30.0, 36.0, 44.0, 60.0, 80.0] {
        let g = break_greedy(&items, width, &params);
        let o = break_paragraph(&items, width, &params);
        if g.iter().any(|l| l.overfull || l.ratio.is_infinite()) {
            continue;
        }
        assert!(
            total_demerits(&o, &params) <= total_demerits(&g, &params) + 1e-6,
            "width {width}"
        );
    }
}

#[test]
fn lines_paginate_with_widows_and_orphans() {
    let items = items_from_text(TEXT, &measure, None, 50.0);
    let lines = break_paragraph(&items, 30.0, &Params::default());
    let heights = vec![1.0; lines.len()];
    let pages = paginate_lines(&heights, (lines.len() - 1) as f64, 40.0, 2, 2);
    assert!(pages.len() >= 2);
    let last = pages.last().unwrap();
    assert!(last.len() >= 2, "no widow");
    assert_eq!(pages.iter().map(|r| r.len()).sum::<usize>(), lines.len());
}

#[test]
fn profiles_settings_and_pages_work_together() {
    let mut set = ProfileSet::default();
    set.upsert(ReadingProfile::new(
        "night",
        TypographyOverride {
            font_size: Some(14.0),
            line_height: Some(1.7),
            ..Default::default()
        },
    ));
    set.rules.push(ProfileRule {
        profile: "night".into(),
        triggers: vec![Trigger::TimeWindow {
            start_minute: 22 * 60,
            end_minute: 6 * 60,
        }],
    });
    let ctx = ProfileContext {
        minute_of_day: 23 * 60 + 30,
        ..Default::default()
    };
    assert_eq!(pick_profile(&set.rules, &ctx), Some("night"));
    let eff = set.effective(
        &TypographySettings::default(),
        None,
        &TypographyOverride::default(),
        &ctx,
    );
    assert_eq!((eff.font_size, eff.line_height), (14.0, 1.7));
    let back = ProfileSet::from_json(&set.to_json()).unwrap();
    assert_eq!(back, set);

    // pages: characters per page derived from the settings
    let items = vec![
        SpineItem::hidden(500.0),
        SpineItem::new(40_000.0),
        SpineItem::new(20_000.0),
    ];
    let map = PageMap::new(&items, 2000.0, PageSource::Auto);
    assert_eq!(map.total_pages(), 30);
    assert_eq!(map.position_of(16), Some((1, 30_000.0)));
    let (spine, off) = map.position_of(21).unwrap();
    assert_eq!((spine, off), (2, 0.0));
    assert_eq!(map.page_at(spine, off).unwrap().number, 21);

    let listed = vec![SpineItem::new(100.0)
        .with_targets(vec![PageTarget::new("i", 0.0), PageTarget::new("ii", 50.0)])];
    let map = PageMap::new(&listed, 2000.0, PageSource::Auto);
    assert_eq!(map.page_at(0, 75.0).unwrap().label, "ii");
}

#[test]
fn highlights_survive_json_and_merge() {
    let mut set = HighlightSet::new();
    set.add(Highlight::new(
        "a",
        Range::new(10, 4),
        HighlightStyle::Underline,
        HighlightColor::Green,
    ));
    set.add(Highlight::new(
        "b",
        Range::new(9, 14),
        HighlightStyle::Underline,
        HighlightColor::Green,
    ));
    let json = set.to_json();
    assert!(json.contains("\"underline\"") && json.contains("#33cc6b"));
    let mut back = HighlightSet::from_json(&json).unwrap();
    assert_eq!(back, set);
    assert_eq!(back.merge_same_style(), 1);
    assert_eq!(back.get("a").unwrap().range, Range::new(4, 14));
    assert_eq!(back.overlapping_at(12).len(), 1);
}
