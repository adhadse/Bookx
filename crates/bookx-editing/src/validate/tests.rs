use super::*;
use crate::testepub::*;

fn report(files: &Files) -> Report {
    validate(&to_archive(files))
}

fn assert_ids(r: &Report, ids: &[&str]) {
    let mut want: Vec<String> = ids.iter().map(|s| s.to_string()).collect();
    want.sort();
    assert_eq!(r.ids(), want, "{}", render_text(r));
}

fn clean(version: u8) -> Files {
    if version == 2 {
        clean_epub2()
    } else {
        clean_epub3()
    }
}

// ---- clean books -----------------------------------------------------------------------------------------

#[test]
fn clean_epub3_triggers_nothing() {
    let r = report(&clean_epub3());
    assert!(r.messages.is_empty(), "{}", render_text(&r));
    assert!(r.passed);
    assert_eq!(r.version.as_deref(), Some("3.0"));
    assert_eq!(r.summary, Summary::default());
}

#[test]
fn clean_epub2_triggers_nothing() {
    let r = report(&clean_epub2());
    assert!(r.messages.is_empty(), "{}", render_text(&r));
    assert!(r.passed);
    assert_eq!(r.version.as_deref(), Some("2.0"));
}

#[test]
fn clean_books_pass_as_bytes_too() {
    for files in [clean_epub3(), clean_epub2()] {
        let r = validate_bytes(&to_bytes(&files));
        assert!(r.messages.is_empty(), "{}", render_text(&r));
    }
}

// ---- one fixture per rule -------------------------------------------------------------------------------

#[test]
fn every_rule_has_a_fixture_and_every_fixture_a_rule() {
    let mut ids: Vec<&str> = faults().iter().map(|f| f.id).collect();
    let n = ids.len();
    ids.dedup();
    assert_eq!(n, ids.len(), "two archive fixtures for one rule");
    let mut covered = ids;
    covered.extend(zip_faults().iter().map(|f| f.0));
    covered.sort();
    covered.dedup(); // PKG-006 has an archive fixture (missing) and a zip fixture (not first)
    let mut all: Vec<&str> = RULES.iter().map(|r| r.id).collect();
    all.sort();
    let mut dup = all.clone();
    dup.dedup();
    assert_eq!(all, dup, "duplicate rule id in RULES");
    assert_eq!(covered, all);
}

#[test]
fn each_fault_breaks_exactly_its_rule() {
    for fault in faults() {
        let mut files = clean(fault.version);
        (fault.apply)(&mut files);
        let r = report(&files);
        let mut want = vec![fault.id];
        want.extend(fault.also);
        assert_ids(&r, &want);
        let rule = rule(fault.id).unwrap();
        let m = r.of(fault.id)[0];
        assert_eq!(m.severity, rule.severity, "{}", fault.id);
        assert!(!m.text.is_empty());
        assert_eq!(
            r.passed,
            rule.severity != Severity::Error && fault.also.is_empty(),
            "{}: passed flag",
            fault.id
        );
    }
}

#[test]
fn each_zip_fault_breaks_exactly_its_rule() {
    for (id, bytes) in zip_faults() {
        let r = validate_bytes(&bytes);
        assert_ids(&r, &[id]);
        assert_eq!(r.of(id)[0].severity, rule(id).unwrap().severity);
        assert_eq!(
            r.passed,
            rule(id).unwrap().severity != Severity::Error,
            "{id}"
        );
    }
}

#[test]
fn severities_cover_all_four_levels() {
    for s in [
        Severity::Error,
        Severity::Warning,
        Severity::Info,
        Severity::Usage,
    ] {
        assert!(RULES.iter().any(|r| r.severity == s), "{s:?}");
    }
    assert_eq!(rule("OPF-918").unwrap().severity, Severity::Info);
    assert_eq!(rule("OPF-088").unwrap().severity, Severity::Usage);
    assert_eq!(rule("OPF-003").unwrap().severity, Severity::Warning);
    assert!(rule("NOPE-1").is_none());
}

#[test]
fn rule_ids_follow_the_catalogue_families() {
    for r in RULES {
        let (family, num) = r.id.split_once('-').unwrap();
        assert!(
            ["PKG", "OPF", "RSC", "HTM", "CSS", "NAV", "NCX", "ACC"].contains(&family),
            "{}",
            r.id
        );
        assert_eq!(num.len(), 3, "{}", r.id);
        assert!(num.chars().all(|c| c.is_ascii_digit()));
        assert!(!r.title.is_empty());
    }
}

// ---- messages carry places -------------------------------------------------------------------------------

#[test]
fn xml_errors_carry_line_and_column() {
    let mut files = clean_epub3();
    set(
        &mut files,
        "OEBPS/ch2.xhtml",
        "<html xmlns=\"http://www.w3.org/1999/xhtml\">\n<body>\n  <p>x</q>\n</body></html>",
    );
    let r = report(&files);
    let m = r.of("RSC-016")[0];
    assert_eq!(m.file, "OEBPS/ch2.xhtml");
    assert_eq!(m.line, Some(3));
    assert!(m.col.is_some());
}

#[test]
fn opf_findings_point_at_the_element() {
    let mut files = clean_epub3();
    add_item(&mut files, "gone", "gone.css", "text/css", "", None);
    let r = report(&files);
    let m = r.of("RSC-001")[0];
    assert_eq!(m.file, OPF);
    assert!(m.line.unwrap() >= 10);
    assert!(m.text.contains("gone.css"));
}

#[test]
fn css_positions_are_relative_to_the_file() {
    let mut files = clean_epub3();
    set(
        &mut files,
        "OEBPS/style.css",
        format!("{STYLE}\n\n@foo x;\n"),
    );
    let r = report(&files);
    let m = r.of("CSS-902")[0];
    assert_eq!((m.line, m.col), (Some(6), Some(1)));
}

// ---- reports ---------------------------------------------------------------------------------------------

#[test]
fn summary_counts_and_line() {
    let mut files = clean_epub3();
    remove(&mut files, "OEBPS/style.css"); // RSC-001 (error) + 2 RSC-007 refs (css links in ch1, ch2) ...
    let r = report(&files);
    assert!(!r.passed);
    assert_eq!(
        r.summary.errors,
        r.messages
            .iter()
            .filter(|m| m.severity == Severity::Error)
            .count()
    );
    let line = summary_line(&r);
    assert!(line.ends_with(": FAILED"), "{line}");
    assert!(
        line.starts_with(&format!("{} errors", r.summary.errors)),
        "{line}"
    );

    let ok = report(&clean_epub3());
    assert_eq!(
        summary_line(&ok),
        "0 errors, 0 warnings, 0 infos, 0 usages: PASSED"
    );
    let mut one = clean_epub3();
    set(&mut one, "OEBPS/extra.txt", "x");
    assert_eq!(
        summary_line(&report(&one)),
        "0 errors, 1 warning, 0 infos, 0 usages: PASSED"
    );
}

#[test]
fn text_rendering() {
    let mut files = clean_epub3();
    set(&mut files, "OEBPS/extra.txt", "x");
    rep(
        &mut files,
        "OEBPS/ch2.xhtml",
        "href=\"ch1.xhtml\"",
        "href=\"nofile.xhtml\"",
    );
    let r = report(&files);
    let text = render_text(&r);
    assert!(
        text.contains("WARNING(OPF-003) OEBPS/extra.txt: "),
        "{text}"
    );
    assert!(text.contains("ERROR(RSC-007) OEBPS/ch2.xhtml:"), "{text}");
    assert!(
        text.trim_end()
            .ends_with("1 error, 1 warning, 0 infos, 0 usages: FAILED"),
        "{text}"
    );
    // a message without a file
    let mut r2 = Report::default();
    r2.messages.push(Message {
        id: "PKG-004".into(),
        severity: Severity::Error,
        file: String::new(),
        line: None,
        col: None,
        text: "bad".into(),
    });
    assert!(render_text(&r2.finish()).starts_with("ERROR(PKG-004): bad\n"));
}

#[test]
fn json_rendering_round_trips() {
    let mut files = clean_epub3();
    set(&mut files, "OEBPS/extra.txt", "x");
    rep(
        &mut files,
        "OEBPS/ch2.xhtml",
        "href=\"ch1.xhtml\"",
        "href=\"nofile.xhtml\"",
    );
    let r = report(&files);
    let json = render_json(&r);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["passed"], false);
    assert_eq!(v["summary"]["errors"], 1);
    assert_eq!(v["summary"]["warnings"], 1);
    let sevs: Vec<&str> = v["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["severity"].as_str().unwrap())
        .collect();
    assert!(
        sevs.contains(&"ERROR") && sevs.contains(&"WARNING"),
        "{sevs:?}"
    );
    let back: Report = serde_json::from_str(&json).unwrap();
    assert_eq!(back, r);
}

#[test]
fn report_helpers() {
    let mut files = clean_epub3();
    set(&mut files, "OEBPS/extra.txt", "x");
    set(&mut files, "OEBPS/extra2.txt", "x");
    let r = report(&files);
    assert!(r.has("OPF-003") && !r.has("OPF-001"));
    assert_eq!(r.of("OPF-003").len(), 2);
    assert_eq!(r.ids_with(Severity::Warning), vec!["OPF-003"]);
    assert!(r.ids_with(Severity::Error).is_empty());
    assert_eq!(Severity::Usage.label(), "USAGE");
}

// ---- more cases per rule ----------------------------------------------------------------------------------

fn with_ch2_body(files: &mut Files, body: &str) {
    set(
        files,
        "OEBPS/ch2.xhtml",
        xhtml3(
            "<title>Two</title>",
            &format!("<section id=\"sec\">{body}<a href=\"ch1.xhtml\">back</a></section>"),
        ),
    );
}

#[test]
fn features_need_their_properties() {
    let cases: [(&str, &str); 6] = [
        (
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"5\" height=\"5\"><rect width=\"5\" height=\"5\"/></svg>",
            "svg",
        ),
        (
            "<math xmlns=\"http://www.w3.org/1998/Math/MathML\"><mi>x</mi></math>",
            "mathml",
        ),
        (
            "<epub:switch><epub:case required-namespace=\"http://www.w3.org/1998/Math/MathML\"><p>a</p></epub:case><epub:default><p>b</p></epub:default></epub:switch>",
            "switch",
        ),
        (
            "<img src=\"http://example.com/a.png\" alt=\"remote\"/>",
            "remote-resources",
        ),
        ("<p onclick=\"go()\">x</p>", "scripted"),
        (
            "<audio src=\"https://example.com/a.mp3\"></audio>",
            "remote-resources",
        ),
    ];
    for (body, prop) in cases {
        let mut files = clean_epub3();
        with_ch2_body(&mut files, body);
        let r = report(&files);
        assert_ids(&r, &["OPF-014"]);
        assert!(
            r.of("OPF-014")[0].text.contains(&format!("\"{prop}\"")),
            "{prop}: {}",
            render_text(&r)
        );
        // declaring the property fixes it
        rep(
            &mut files,
            OPF,
            "<item id=\"ch2\" href=\"ch2.xhtml\" media-type=\"application/xhtml+xml\"/>",
            &format!(
                "<item id=\"ch2\" href=\"ch2.xhtml\" media-type=\"application/xhtml+xml\" properties=\"{prop}\"/>"
            ),
        );
        let r = report(&files);
        assert!(r.messages.is_empty(), "{prop}: {}", render_text(&r));
    }
}

#[test]
fn non_javascript_script_blocks_do_not_need_scripted() {
    let mut files = clean_epub3();
    with_ch2_body(
        &mut files,
        "<script type=\"application/ld+json\">{\"a\":1}</script>",
    );
    assert!(report(&files).messages.is_empty());
}

#[test]
fn remote_stylesheets_and_fonts_need_remote_resources() {
    let mut files = clean_epub3();
    set(
        &mut files,
        "OEBPS/style.css",
        format!("{STYLE}@import url(\"https://example.com/x.css\");\n"),
    );
    let r = report(&files);
    assert_ids(&r, &["OPF-014"]);
    assert!(r.of("OPF-014")[0].text.contains("style.css"));
    rep(
        &mut files,
        OPF,
        "<item id=\"css\" href=\"style.css\" media-type=\"text/css\"/>",
        "<item id=\"css\" href=\"style.css\" media-type=\"text/css\" properties=\"remote-resources\"/>",
    );
    assert!(report(&files).messages.is_empty());
}

#[test]
fn css_references_are_resolved() {
    let mut files = clean_epub3();
    set(
        &mut files,
        "OEBPS/style.css",
        format!(
            "{STYLE}@import \"other.css\";\n@import url(missing.css);\nh1 {{ background: url('images/nope.png'); }}\n@font-face {{ font-family: U; src: url(fonts/nofont.ttf) format('truetype'); }}\np {{ background: url(data:image/png;base64,AAAA); }}\n"
        ),
    );
    add_item(
        &mut files,
        "other",
        "other.css",
        "text/css",
        "",
        Some(OTHER_CSS_TEST.as_bytes()),
    );
    let r = report(&files);
    assert_ids(&r, &["RSC-007"]);
    let texts: Vec<String> = r.of("RSC-007").iter().map(|m| m.text.clone()).collect();
    assert_eq!(texts.len(), 3, "{texts:?}");
    for want in [
        "OEBPS/missing.css",
        "OEBPS/images/nope.png",
        "OEBPS/fonts/nofont.ttf",
    ] {
        assert!(
            texts.iter().any(|t| t.contains(want)),
            "{want} in {texts:?}"
        );
    }
}

const OTHER_CSS_TEST: &str = "p { margin: 0; }\n";

#[test]
fn css_comments_hide_rules_and_strings_hide_braces() {
    let mut files = clean_epub3();
    set(
        &mut files,
        "OEBPS/style.css",
        format!(
            "{STYLE}/* @foo {{ direction: rtl; url(nothing.png) */\nq::before {{ content: \"}} {{ @bar\"; }}\n"
        ),
    );
    let r = report(&files);
    assert!(r.messages.is_empty(), "{}", render_text(&r));
}

#[test]
fn css_syntax_errors() {
    for (css, what) in [
        ("p { color: red; } }", "unexpected }"),
        ("p { color: red; ", "unclosed block"),
        ("/* never closed", "unterminated comment"),
        ("p { content: \"abc\n }", "unterminated string"),
    ] {
        let mut files = clean_epub3();
        set(&mut files, "OEBPS/style.css", css);
        let r = report(&files);
        assert_ids(&r, &["CSS-008"]);
        assert!(
            r.of("CSS-008")[0].text.contains(what),
            "{what}: {}",
            render_text(&r)
        );
    }
}

#[test]
fn css_vendor_at_rules_are_known_and_unknown_ones_warn() {
    let mut files = clean_epub3();
    set(
        &mut files,
        "OEBPS/style.css",
        "@-webkit-keyframes a { from { margin: 0 } to { margin: 1px } }\n@media print { p { margin: 0 } }\n@page { margin: 1em }\n@namespace url(http://www.w3.org/1999/xhtml);\n",
    );
    assert!(report(&files).messages.is_empty());
    set(&mut files, "OEBPS/style.css", "@-epub-foo x;\n");
    assert_ids(&report(&files), &["CSS-902"]);
}

#[test]
fn css_charset_utf8_in_any_case_and_bad_bytes() {
    let mut files = clean_epub3();
    set(
        &mut files,
        "OEBPS/style.css",
        "@charset 'utf-8';\np { margin: 0 }\n",
    );
    assert!(report(&files).messages.is_empty());
    set(&mut files, "OEBPS/style.css", vec![b'p', b'{', 0xE9, b'}']);
    assert_ids(&report(&files), &["CSS-003"]);
    set(&mut files, "OEBPS/style.css", "unicode-bidi : embed;");
    assert_ids(&report(&files), &["CSS-001"]);
}

#[test]
fn style_elements_are_checked_like_style_sheets() {
    let mut files = clean_epub3();
    set(
        &mut files,
        "OEBPS/ch2.xhtml",
        xhtml3(
            "<title>Two</title><style>p { direction: rtl; background: url(none.png); }</style>",
            "<section id=\"sec\"><a href=\"ch1.xhtml\">back</a></section>",
        ),
    );
    let r = report(&files);
    assert_ids(&r, &["CSS-001", "RSC-007"]);
}

#[test]
fn epub2_forbids_html5_elements_and_inline_svg() {
    for body in [
        "<section><p>x</p></section>",
        "<audio></audio>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
        "<math xmlns=\"http://www.w3.org/1998/Math/MathML\"><mi>x</mi></math>",
        "<script type=\"text/javascript\"></script>",
    ] {
        let mut files = clean_epub2();
        rep(
            &mut files,
            "OEBPS/ch2.xhtml",
            "</div>",
            &format!("{body}</div>"),
        );
        assert_ids(&report(&files), &["HTM-906"]);
    }
    // the same elements are fine in EPUB 3
    let mut files = clean_epub3();
    with_ch2_body(&mut files, "<aside><p>x</p></aside>");
    assert!(report(&files).messages.is_empty());
}

#[test]
fn doctype_documents_may_use_named_entities() {
    let mut files = clean_epub2();
    rep(
        &mut files,
        "OEBPS/ch2.xhtml",
        "<p>More",
        "<p>A&nbsp;B &mdash; &copy; &hellip; More",
    );
    let r = report(&files);
    assert!(r.messages.is_empty(), "{}", render_text(&r));
    // an unknown entity is still not well-formed
    let mut files = clean_epub2();
    rep(&mut files, "OEBPS/ch2.xhtml", "<p>More", "<p>&bogus; More");
    assert_ids(&report(&files), &["RSC-016"]);
    // without a doctype named entities are errors
    let mut files = clean_epub3();
    set(
        &mut files,
        "OEBPS/ch2.xhtml",
        "<html xmlns=\"http://www.w3.org/1999/xhtml\" lang=\"en\"><head><title>t</title></head><body><section id=\"sec\"><p>&nbsp;<a href=\"ch1.xhtml\">b</a></p></section></body></html>",
    );
    assert_ids(&report(&files), &["RSC-016"]);
}

#[test]
fn img_without_alt_inside_a_captioned_figure_is_allowed_in_epub3() {
    let mut files = clean_epub3();
    with_ch2_body(
        &mut files,
        "<figure><img src=\"images/cover.png\"/><figcaption>c</figcaption></figure>",
    );
    assert!(report(&files).messages.is_empty());
    let mut files = clean_epub3();
    with_ch2_body(
        &mut files,
        "<figure><img src=\"images/cover.png\"/></figure>",
    );
    assert_ids(&report(&files), &["HTM-907"]);
    let mut files = clean_epub3();
    with_ch2_body(&mut files, "<img src=\"images/cover.png\" alt=\"\"/>");
    assert!(
        report(&files).messages.is_empty(),
        "empty alt marks a decorative image"
    );
}

#[test]
fn links_with_images_or_aria_labels_have_text() {
    let mut files = clean_epub3();
    with_ch2_body(
        &mut files,
        "<a href=\"ch1.xhtml\"><img src=\"images/cover.png\" alt=\"c\"/></a><a href=\"ch1.xhtml\" aria-label=\"x\"></a><a id=\"anchor\"></a>",
    );
    assert!(report(&files).messages.is_empty());
}

#[test]
fn epub_type_prefixes() {
    let mut files = clean_epub3();
    with_ch2_body(
        &mut files,
        "<p epub:type=\"z3998:verse noteref\">x</p><p epub:type=\"my:thing\">y</p>",
    );
    assert_ids(&report(&files), &["OPF-028"]);
    rep(
        &mut files,
        "OEBPS/ch2.xhtml",
        "lang=\"en\" xml:lang",
        "epub:prefix=\"my: http://example.com/vocab\" lang=\"en\" xml:lang",
    );
    assert!(report(&files).messages.is_empty());
}

#[test]
fn idrefs() {
    let mut files = clean_epub3();
    with_ch2_body(
        &mut files,
        "<p id=\"l\">x</p><p aria-describedby=\"l\">y</p><label for=\"l\">z</label>",
    );
    assert!(report(&files).messages.is_empty());
    with_ch2_body(
        &mut files,
        "<table><tr><td headers=\"h1 h2\">x</td></tr></table>",
    );
    let r = report(&files);
    assert_eq!(r.of("HTM-909").len(), 2);
}

#[test]
fn links_ignore_other_schemes_and_resolve_encoding() {
    let mut files = clean_epub3();
    with_ch2_body(
        &mut files,
        "<a href=\"mailto:a@b.c\">m</a><a href=\"tel:123\">t</a><a href=\"https://example.com/\">w</a><a href=\"urn:isbn:1\">u</a><a href=\"images/../ch1.xhtml?x=1#top\">q</a><a href=\"%63h1.xhtml\">pct</a>",
    );
    let r = report(&files);
    assert!(r.messages.is_empty(), "{}", render_text(&r));
}

#[test]
fn absolute_paths_leak() {
    let mut files = clean_epub3();
    with_ch2_body(&mut files, "<a href=\"/ch1.xhtml\">abs</a>");
    assert_ids(&report(&files), &["RSC-026"]);
}

#[test]
fn fragment_in_unparsed_target_is_not_checked() {
    let mut files = clean_epub3();
    with_ch2_body(
        &mut files,
        "<a href=\"images/cover.png#whatever\">img</a><a href=\"ch1.xhtml#epubcfi(/6/2)\">cfi</a>",
    );
    assert!(report(&files).messages.is_empty());
}

#[test]
fn a_name_anchor_is_a_fragment_target() {
    let mut files = clean_epub3();
    set(
        &mut files,
        "OEBPS/ch1.xhtml",
        xhtml3(
            "<title>One</title>",
            "<section><a name=\"old\"></a><h1 id=\"top\">One</h1><a href=\"ch2.xhtml#sec\">n</a><a href=\"#old\">o</a></section>",
        ),
    );
    let r = report(&files);
    assert!(r.messages.is_empty(), "{}", render_text(&r));
}

#[test]
fn nav_structure_variants() {
    // a heading span with a nested list is fine
    let mut files = clean_epub3();
    rep(
        &mut files,
        "OEBPS/nav.xhtml",
        "<li><a href=\"ch1.xhtml\">One</a><ol>",
        "<li><span>Part</span><ol><li><a href=\"ch1.xhtml\">One</a></li>",
    );
    assert!(report(&files).messages.is_empty());
    // a span without a list is not
    let mut files = clean_epub3();
    rep(
        &mut files,
        "OEBPS/nav.xhtml",
        "<li><a href=\"ch2.xhtml\">Two</a></li>",
        "<li><span>Two</span></li>",
    );
    assert_ids(&report(&files), &["NAV-001"]);
    // empty <ol>, missing href, empty link text, two lists
    for (from, to) in [
        (
            "<li><a href=\"ch2.xhtml\">Two</a></li>",
            "<li><a href=\"ch2.xhtml\">Two</a><ol></ol></li>",
        ),
        ("<a href=\"ch2.xhtml\">Two</a>", "<a>Two</a>"),
        (
            "<a href=\"ch2.xhtml\">Two</a>",
            "<a href=\"ch2.xhtml\"> </a>",
        ),
        (
            "</ol></nav>",
            "</ol><ol><li><a href=\"ch2.xhtml\">Two</a></li></ol></nav>",
        ),
        ("<h1>Contents</h1>", "<h1>Contents</h1><p>stray</p>"),
        ("<li><a href=\"ch2.xhtml\">Two</a></li>", "<div>Two</div>"),
    ] {
        let mut files = clean_epub3();
        rep(&mut files, "OEBPS/nav.xhtml", from, to);
        let r = report(&files);
        assert!(r.has("NAV-001"), "{from} -> {to}: {}", render_text(&r));
    }
    // other navs (landmarks) are not held to the toc rules but their links are resolved
    let mut files = clean_epub3();
    rep(
        &mut files,
        "OEBPS/nav.xhtml",
        "</nav>",
        "</nav><nav epub:type=\"landmarks\"><ol><li><a epub:type=\"bodymatter\" href=\"ch1.xhtml\">Start</a></li></ol></nav>",
    );
    assert!(report(&files).messages.is_empty());
    rep(
        &mut files,
        "OEBPS/nav.xhtml",
        "href=\"ch1.xhtml\">Start",
        "href=\"gone.xhtml\">Start",
    );
    assert_ids(&report(&files), &["RSC-007"]);
}

#[test]
fn nav_links_are_resolved() {
    let mut files = clean_epub3();
    rep(
        &mut files,
        "OEBPS/nav.xhtml",
        "href=\"ch1.xhtml#top\"",
        "href=\"ch1.xhtml#nowhere\"",
    );
    assert_ids(&report(&files), &["RSC-012"]);
}

#[test]
fn ncx_in_epub3_is_checked_when_present() {
    let mut files = clean_epub3();
    add_item(
        &mut files,
        "ncx",
        "toc.ncx",
        "application/x-dtbncx+xml",
        "",
        Some(ncx("urn:uuid:wrong", NAV_MAP).as_bytes()),
    );
    rep(&mut files, OPF, "<spine>", "<spine toc=\"ncx\">");
    assert_ids(&report(&files), &["NCX-001"]);
}

#[test]
fn ncx_targets_are_resolved() {
    let mut files = clean_epub2();
    rep(
        &mut files,
        "OEBPS/toc.ncx",
        "src=\"ch1.xhtml#top\"",
        "src=\"ch1.xhtml#zzz\"",
    );
    assert_ids(&report(&files), &["RSC-012"]);
    let mut files = clean_epub2();
    rep(
        &mut files,
        "OEBPS/toc.ncx",
        "src=\"ch2.xhtml\"",
        "src=\"gone.xhtml\"",
    );
    assert_ids(&report(&files), &["RSC-007"]);
    // same playOrder for the same target is fine
    let mut files = clean_epub2();
    rep(
        &mut files,
        "OEBPS/toc.ncx",
        "<navPoint id=\"n11\" playOrder=\"2\">",
        "<navPoint id=\"n11\" playOrder=\"1\">",
    );
    assert!(report(&files).messages.is_empty() || report(&files).ids() == vec!["NCX-903"]);
}

#[test]
fn ncx_uid_comparison_ignores_whitespace() {
    let mut files = clean_epub2();
    set(
        &mut files,
        "OEBPS/toc.ncx",
        ncx(&format!("  {UID} "), NAV_MAP),
    );
    assert!(report(&files).messages.is_empty());
}

#[test]
fn spine_fallbacks_make_non_core_items_acceptable() {
    let mut files = clean_epub3();
    // an image in the spine with a fallback to a content document is fine
    rep(
        &mut files,
        OPF,
        "<item id=\"cover\" href=\"images/cover.png\" media-type=\"image/png\" properties=\"cover-image\"/>",
        "<item id=\"cover\" href=\"images/cover.png\" media-type=\"image/png\" properties=\"cover-image\" fallback=\"ch1\"/>",
    );
    rep(
        &mut files,
        OPF,
        "</spine>",
        "<itemref idref=\"cover\"/></spine>",
    );
    assert!(report(&files).messages.is_empty());
    // a chain that never reaches one is not
    rep(&mut files, OPF, "fallback=\"ch1\"", "fallback=\"font\"");
    assert_ids(&report(&files), &["OPF-042"]);
    // SVG is a content document in EPUB 3 but not in EPUB 2
    let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1\" height=\"1\"/>";
    let mut files = clean_epub3();
    add_item(
        &mut files,
        "svg",
        "p.svg",
        "image/svg+xml",
        "",
        Some(svg.as_bytes()),
    );
    rep(
        &mut files,
        OPF,
        "</spine>",
        "<itemref idref=\"svg\"/></spine>",
    );
    assert!(report(&files).messages.is_empty());
    let mut files = clean_epub2();
    add_item(
        &mut files,
        "svg",
        "p.svg",
        "image/svg+xml",
        "",
        Some(svg.as_bytes()),
    );
    rep(
        &mut files,
        OPF,
        "</spine>",
        "<itemref idref=\"svg\"/></spine>",
    );
    assert_ids(&report(&files), &["OPF-042"]);
}

#[test]
fn spine_item_without_linear_attribute_and_all_nonlinear() {
    let mut files = clean_epub3();
    rep(
        &mut files,
        OPF,
        "<itemref idref=\"ch1\"/>",
        "<itemref idref=\"ch1\" linear=\"no\"/>",
    );
    rep(
        &mut files,
        OPF,
        "<itemref idref=\"ch2\"/>",
        "<itemref idref=\"ch2\" linear=\"no\"/>",
    );
    let r = report(&files);
    assert_eq!(r.ids(), vec!["OPF-033", "OPF-918"]);
}

#[test]
fn itemref_properties_are_validated() {
    let mut files = clean_epub3();
    rep(
        &mut files,
        OPF,
        "<itemref idref=\"ch1\"/>",
        "<itemref idref=\"ch1\" properties=\"page-spread-right rendition:layout-pre-paginated\"/>",
    );
    assert!(report(&files).messages.is_empty());
    rep(&mut files, OPF, "page-spread-right", "page-spread-middle");
    assert_ids(&report(&files), &["OPF-027"]);
}

#[test]
fn package_prefix_attribute_declares_prefixes() {
    let mut files = clean_epub3();
    rep(
        &mut files,
        OPF,
        "<item id=\"css\" href=\"style.css\" media-type=\"text/css\"/>",
        "<item id=\"css\" href=\"style.css\" media-type=\"text/css\" properties=\"foo:bar\"/>",
    );
    rep(
        &mut files,
        OPF,
        "unique-identifier=\"uid\"",
        "unique-identifier=\"uid\" prefix=\"foo: http://example.com/foo\"",
    );
    assert!(report(&files).messages.is_empty());
}

#[test]
fn modified_with_refines_does_not_count() {
    let mut files = clean_epub3();
    rep(
        &mut files,
        OPF,
        "<meta property=\"dcterms:modified\">2024-01-02T03:04:05Z</meta>",
        "<meta property=\"dcterms:modified\" refines=\"#uid\">2024-01-02T03:04:05Z</meta>",
    );
    assert_ids(&report(&files), &["OPF-904"]);
}

#[test]
fn language_tags() {
    for ok in ["en", "en-US", "zh-Hans-CN", "de-1996"] {
        let mut files = clean_epub3();
        rep(
            &mut files,
            OPF,
            "<dc:language>en</dc:language>",
            &format!("<dc:language>{ok}</dc:language>"),
        );
        assert!(report(&files).messages.is_empty(), "{ok}");
    }
    for bad in ["", "e", "english language", "en_US", "-en"] {
        let mut files = clean_epub3();
        rep(
            &mut files,
            OPF,
            "<dc:language>en</dc:language>",
            &format!("<dc:language>{bad}</dc:language>"),
        );
        assert_ids(&report(&files), &["OPF-092"]);
    }
}

#[test]
fn version_values() {
    let mut files = clean_epub3();
    rep(&mut files, OPF, " version=\"3.0\"", "");
    assert_ids(&report(&files), &["OPF-001"]);
    let mut files = clean_epub3();
    rep(&mut files, OPF, "version=\"3.0\"", "version=\"3.1\"");
    assert!(report(&files).messages.is_empty());
    let mut files = clean_epub3();
    set(&mut files, OPF, "<?xml version=\"1.0\"?><notpackage/>");
    let r = report(&files);
    assert_ids(&r, &["OPF-001"]);
    assert!(r.of("OPF-001")[0].text.contains("notpackage"));
}

#[test]
fn broken_opf_stops_cleanly() {
    let mut files = clean_epub3();
    set(&mut files, OPF, "<package");
    let r = report(&files);
    assert_ids(&r, &["RSC-016"]);
    assert_eq!(r.of("RSC-016")[0].file, OPF);
    let mut files = clean_epub3();
    set(&mut files, "META-INF/container.xml", "<container");
    assert_ids(&report(&files), &["RSC-016"]);
}

#[test]
fn container_rootfile_choice() {
    // the first rootfile with the package media type wins, others are ignored
    let mut files = clean_epub3();
    set(
        &mut files,
        "META-INF/container.xml",
        "<container xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile full-path=\"x.pdf\" media-type=\"application/pdf\"/><rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>",
    );
    // a pdf rootfile has a media type of its own, so it is skipped
    let r = report(&files);
    assert!(r.messages.is_empty(), "{}", render_text(&r));
    set(
        &mut files,
        "META-INF/container.xml",
        "<container xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile full-path=\"\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>",
    );
    assert_ids(&report(&files), &["RSC-003"]);
}

#[test]
fn known_encryption_algorithms_are_accepted() {
    let mut files = clean_epub3();
    let enc = ENC.replace("ALG", "http://www.idpf.org/2008/embedding");
    set(&mut files, "META-INF/encryption.xml", enc);
    assert!(report(&files).messages.is_empty());
    let enc = ENC.replace("ALG", "http://ns.adobe.com/pdf/enc#RC");
    set(&mut files, "META-INF/encryption.xml", enc);
    assert!(report(&files).messages.is_empty());
    set(&mut files, "META-INF/encryption.xml", "<encryption");
    assert_ids(&report(&files), &["RSC-016"]);
}

const ENC: &str = "<encryption xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\" xmlns:enc=\"http://www.w3.org/2001/04/xmlenc#\"><enc:EncryptedData><enc:EncryptionMethod Algorithm=\"ALG\"/><enc:CipherData><enc:CipherReference URI=\"OEBPS/fonts/f.ttf\"/></enc:CipherData></enc:EncryptedData></encryption>";

#[test]
fn encrypted_items_are_not_parsed() {
    let mut files = clean_epub3();
    set(
        &mut files,
        "META-INF/encryption.xml",
        ENC.replace("ALG", "http://example.com/aes")
            .replace("OEBPS/fonts/f.ttf", "OEBPS/ch2.xhtml"),
    );
    set(&mut files, "OEBPS/ch2.xhtml", vec![0xDE, 0xAD, 0xBE, 0xEF]);
    assert_ids(&report(&files), &["RSC-004"]);
}

#[test]
fn manifest_href_variants() {
    // a remote manifest item is not looked up in the container
    let mut files = clean_epub3();
    add_item(
        &mut files,
        "remote",
        "https://example.com/v.mp4",
        "video/mp4",
        "",
        None,
    );
    assert!(report(&files).messages.is_empty());
    // a manifest href that leaves the container
    let mut files = clean_epub3();
    add_item(&mut files, "up", "../../x.css", "text/css", "", None);
    assert_ids(&report(&files), &["RSC-026"]);
}

#[test]
fn media_type_sniffing() {
    for (declared, file, ok) in [
        ("image/png", png(), true),
        ("image/jpeg", png(), false),
        ("image/gif", png(), false),
        ("application/octet-stream", png(), true),
    ] {
        let mut files = clean_epub3();
        add_item(&mut files, "pic", "p.bin", declared, "", Some(&file));
        let r = report(&files);
        assert_eq!(r.messages.is_empty(), ok, "{declared}: {}", render_text(&r));
    }
    let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0, 0];
    let gif = b"GIF89a....";
    let webp = b"RIFF\x00\x00\x00\x00WEBPVP8 ";
    for (declared, data) in [
        ("image/jpeg", &jpeg[..]),
        ("image/gif", &gif[..]),
        ("image/webp", &webp[..]),
    ] {
        let mut files = clean_epub3();
        add_item(&mut files, "pic", "p.bin", declared, "", Some(data));
        assert!(report(&files).messages.is_empty(), "{declared}");
        let mut files = clean_epub3();
        add_item(&mut files, "pic", "p.bin", "image/png", "", Some(data));
        assert_ids(&report(&files), &["OPF-029"]);
    }
}

#[test]
fn cover_meta_in_epub2() {
    let mut files = clean_epub2();
    rep(
        &mut files,
        OPF,
        "<meta name=\"cover\" content=\"cover\"/>",
        "<meta name=\"cover\" content=\"css\"/>",
    );
    let r = report(&files);
    assert_ids(&r, &["OPF-915"]);
    assert!(r.of("OPF-915")[0].text.contains("not an image"));
}

#[test]
fn archive_without_package_properties_in_epub2_are_ignored() {
    let mut files = clean_epub2();
    rep(
        &mut files,
        OPF,
        "<item id=\"css\" href=\"style.css\" media-type=\"text/css\"/>",
        "<item id=\"css\" href=\"style.css\" media-type=\"text/css\" properties=\"whatever\"/>",
    );
    assert!(report(&files).messages.is_empty());
}

#[test]
fn svg_documents_are_checked() {
    let mut files = clean_epub3();
    add_item(&mut files, "svg", "p.svg", "image/svg+xml", "", Some(b"<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\"><image xlink:href=\"images/cover.png\"/><image href=\"images/none.png\"/></svg>"));
    assert_ids(&report(&files), &["RSC-007"]);
    set(&mut files, "OEBPS/p.svg", "<svg");
    assert_ids(&report(&files), &["RSC-016"]);
}

#[test]
fn zip_names_and_mimetype_in_archive() {
    let mut a = Archive::default();
    a.set("mimetype", b"application/epub+zip".to_vec());
    a.set("a b/é.txt", vec![1]);
    let r = validate(&a);
    assert!(r.has("PKG-010") && r.has("PKG-012"));
    let mut a = Archive::default();
    a.set("mimetype", b"nope".to_vec());
    let r = validate(&a);
    assert!(r.has("PKG-007") && r.has("RSC-002"));
    assert!(r.of("PKG-007")[0].text.contains("nope"));
    // empty archive
    let r = validate(&Archive::default());
    assert_eq!(r.ids(), vec!["PKG-006", "RSC-002"]);
    assert!(!r.passed);
}

#[test]
fn bytes_that_are_not_zips() {
    for data in [&b""[..], b"PK", b"hello", &[0u8; 100][..]] {
        let r = validate_bytes(data);
        assert_ids(&r, &["PKG-004"]);
        assert!(!r.passed);
    }
}

#[test]
fn non_empty_directory_entries_are_fine() {
    let mut files = clean_epub3();
    files.retain(|f| f.0 != "mimetype");
    let mut bytes = Vec::new();
    {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut bytes));
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();
        zip.add_directory("META-INF/", stored).unwrap();
        for (n, d) in &files {
            zip.start_file(n, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(d).unwrap();
        }
        zip.finish().unwrap();
    }
    let r = validate_bytes(&bytes);
    assert!(r.messages.is_empty(), "{}", render_text(&r));
}

#[test]
fn path_helpers() {
    assert!(
        matches!(resolve("OEBPS/a.xhtml", "b.xhtml#x"), Target::Local(p, Some(f)) if p == "OEBPS/b.xhtml" && f == "x")
    );
    assert!(matches!(resolve("OEBPS/a.xhtml", "#x"), Target::Local(p, Some(_)) if p.is_empty()));
    assert!(
        matches!(resolve("OEBPS/a.xhtml", "../b.xhtml"), Target::Local(p, None) if p == "b.xhtml")
    );
    assert!(matches!(resolve("a.xhtml", "../b.xhtml"), Target::Leaks));
    assert!(matches!(resolve("a.xhtml", "http://x/y"), Target::Remote));
    assert!(matches!(resolve("a.xhtml", "//x/y"), Target::Remote));
    assert!(matches!(resolve("a.xhtml", "mailto:x@y"), Target::Ignored));
    assert!(
        matches!(resolve("a.xhtml", "c:/x"), Target::Local(..))
            || matches!(resolve("a.xhtml", "c:/x"), Target::Ignored)
    );
    assert!(
        matches!(resolve("OEBPS/a.xhtml", "my%20file.html"), Target::Local(p, _) if p == "OEBPS/my file.html")
    );
    assert_eq!(offset_pos("ab\ncd", 4), Some((2, 2)));
    assert_eq!(offset_pos("é\né", 5), Some((2, 2)));
    assert_eq!(offset_pos("é\né", 4), Some((2, 1)), "inside a character");
    assert_eq!(offset_pos("", 10), Some((1, 1)));
}

// ---- fuzzing ---------------------------------------------------------------------------------------------

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as usize
    }
}

fn mutate(rng: &mut Lcg, data: &mut Vec<u8>) {
    if data.is_empty() {
        return;
    }
    for _ in 0..1 + rng.next() % 6 {
        let at = rng.next() % data.len();
        match rng.next() % 5 {
            0 => data[at] = (rng.next() % 256) as u8,
            1 => data[at] ^= 1 << (rng.next() % 8),
            2 => {
                data.remove(at);
            }
            3 => data.insert(
                at,
                b"<>&\"'/#%".get(rng.next() % 8).copied().unwrap_or(b'<'),
            ),
            _ => {
                let end = (at + rng.next() % 40).min(data.len());
                data.drain(at..end);
            }
        }
        if data.is_empty() {
            return;
        }
    }
}

#[test]
fn five_hundred_mutated_epub_streams_never_panic() {
    let mut rng = Lcg(0xfeed_beef_1234_5678);
    let bases: Vec<Files> = vec![clean_epub3(), clean_epub2()];
    let mut failed_to_open = 0;
    let mut reached_content = 0;
    for i in 0..600 {
        let files = &bases[i % 2];
        let bytes = match i % 3 {
            // raw bytes of a deflated zip
            0 => {
                let mut b = to_bytes(files);
                mutate(&mut rng, &mut b);
                if rng.next() % 6 == 0 {
                    let cut = rng.next() % b.len().max(1);
                    b.truncate(cut);
                }
                b
            }
            // raw bytes of a stored zip: damage lands in the XML
            1 => {
                let mut b = stored_zip(files);
                mutate(&mut rng, &mut b);
                b
            }
            // damage inside one file of a valid zip
            _ => {
                let mut f = files.clone();
                let k = rng.next() % f.len();
                if f[k].0 != "mimetype" {
                    mutate(&mut rng, &mut f[k].1);
                }
                to_bytes(&f)
            }
        };
        let r = validate_bytes(&bytes);
        if r.has("PKG-004") {
            failed_to_open += 1;
        } else {
            reached_content += 1;
        }
        // the report is always consistent
        assert_eq!(r.passed, r.summary.errors == 0);
        assert_eq!(
            r.summary.errors + r.summary.warnings + r.summary.infos + r.summary.usages,
            r.messages.len()
        );
        for m in &r.messages {
            assert!(rule(&m.id).is_some(), "unknown rule {}", m.id);
        }
        let _ = render_text(&r);
        let _ = render_json(&r);
    }
    assert!(
        reached_content > 200,
        "only {reached_content} streams reached the checkers ({failed_to_open} unreadable)"
    );
}

fn stored_zip(files: &Files) -> Vec<u8> {
    use std::io::Write;
    let mut out = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
        for (n, d) in files {
            zip.start_file(
                n,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
            zip.write_all(d).unwrap();
        }
        zip.finish().unwrap();
    }
    out
}

#[test]
fn deeply_nested_and_huge_documents_do_not_overflow() {
    let mut files = clean_epub3();
    let deep = format!("{}x{}", "<div>".repeat(900), "</div>".repeat(900));
    with_ch2_body(&mut files, &deep);
    let _ = report(&files);
    let mut files = clean_epub3();
    let nested_nav = format!(
        "<nav epub:type=\"toc\"><ol>{}<li><a href=\"ch1.xhtml\">x</a></li>{}</ol></nav>",
        "<li><span>s</span><ol>".repeat(300),
        "</ol></li>".repeat(300)
    );
    set(
        &mut files,
        "OEBPS/nav.xhtml",
        xhtml3("<title>n</title>", &nested_nav),
    );
    let _ = report(&files);
    let mut files = clean_epub3();
    set(&mut files, "OEBPS/style.css", "a{".repeat(100_000));
    let r = report(&files);
    assert!(r.has("CSS-008"));
}

#[test]
fn file_name_rules_in_detail() {
    let ids_for = |name: &str| {
        let mut a = Archive::default();
        a.set("mimetype", b"application/epub+zip".to_vec());
        a.set(name, vec![1]);
        let mut ids: Vec<String> = validate(&a)
            .ids()
            .into_iter()
            .filter(|i| i.starts_with("PKG-"))
            .collect();
        ids.sort();
        ids
    };
    assert!(ids_for("OEBPS/ok-name_1.xhtml").is_empty());
    assert_eq!(ids_for("a<b"), ["PKG-009"]);
    assert_eq!(ids_for("a|b"), ["PKG-009"]);
    assert_eq!(ids_for("a\tb"), ["PKG-009"]);
    assert_eq!(ids_for("a\u{e000}b"), ["PKG-009", "PKG-012"]);
    assert_eq!(ids_for("dir./file"), ["PKG-011"]);
    assert_eq!(ids_for("file."), ["PKG-011"]);
    assert_eq!(ids_for("a b"), ["PKG-010"]);
    assert_eq!(ids_for("ü"), ["PKG-012"]);
    assert_eq!(ids_for(&"x".repeat(256)), ["PKG-028"]);
    assert!(ids_for(&"x".repeat(255)).is_empty());
    // case collisions are reported once, on the second name
    let mut a = Archive::default();
    a.set("mimetype", b"application/epub+zip".to_vec());
    a.set("Foo.txt", vec![1]);
    a.set("foo.TXT", vec![1]);
    a.set("FOO.txt", vec![1]);
    assert_eq!(validate(&a).of("PKG-024").len(), 2);
}

#[test]
fn mimetype_text_is_shown_truncated() {
    let mut a = Archive::default();
    a.set("mimetype", "x".repeat(500).into_bytes());
    let r = validate(&a);
    assert!(r.of("PKG-007")[0].text.len() < 200);
}

#[test]
fn central_directory_scan_tolerates_damage() {
    let bytes = to_bytes(&clean_epub3());
    // the end of the central directory record cut off at every length near the end
    for cut in 0..70 {
        let _ = validate_bytes(&bytes[..bytes.len() - cut]);
    }
    // a central directory header with absurd lengths
    let mut b = bytes.clone();
    let at = b
        .windows(4)
        .rposition(|w| w == [0x50, 0x4B, 0x01, 0x02])
        .unwrap();
    b[at + 28] = 0xFF;
    b[at + 29] = 0xFF;
    let _ = validate_bytes(&b);
}
