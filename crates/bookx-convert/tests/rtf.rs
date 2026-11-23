//! RTF writer: checked with a separate tokenizer written for the tests (brace balance, ASCII
//! only, text extraction, picture decoding), the formats reader and Calibre.

mod common;

use bookx_convert::writers::RtfWriter;
use bookx_core::contracts::FormatWriter;
use common::*;

fn rtf(book: &bookx_core::book::OpenBook) -> String {
    let bytes = RtfWriter.write(book).unwrap();
    String::from_utf8(bytes).expect("RTF must be ASCII")
}

/// Checks `{`/`}` balance (skipping escaped ones) and returns the maximum depth.
fn check_braces(s: &str) -> usize {
    let b = s.as_bytes();
    let (mut depth, mut max) = (0i32, 0i32);
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' => {
                i += 1;
                if b.get(i) == Some(&b'\'') {
                    i += 2;
                }
            }
            b'{' => {
                depth += 1;
                max = max.max(depth);
            }
            b'}' => {
                depth -= 1;
                assert!(depth >= 0, "unbalanced `}}` at byte {i}");
            }
            _ => {}
        }
        i += 1;
    }
    assert_eq!(depth, 0, "unclosed groups");
    max as usize
}

/// A deliberately simple RTF to text converter.
fn rtf_text(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::new();
    let mut stack: Vec<bool> = vec![false]; // skipping?
    let mut i = 0;
    let mut pending_skip = 0usize; // characters to drop after \uN
    while i < b.len() {
        let c = b[i];
        let skipping = *stack.last().unwrap();
        match c {
            b'{' => {
                stack.push(skipping);
                // destination groups
                let rest = &s[i + 1..];
                if rest.starts_with("\\*")
                    || [
                        "\\fonttbl",
                        "\\colortbl",
                        "\\stylesheet",
                        "\\info",
                        "\\pict",
                        "\\fldinst",
                    ]
                    .iter()
                    .any(|d| rest.starts_with(d))
                {
                    *stack.last_mut().unwrap() = true;
                }
                i += 1;
            }
            b'}' => {
                stack.pop();
                i += 1;
            }
            b'\\' => {
                i += 1;
                let n = b[i];
                if matches!(n, b'\\' | b'{' | b'}') {
                    if !skipping {
                        out.push(n as char);
                    }
                    i += 1;
                } else if n == b'\'' {
                    let byte = u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap();
                    if !skipping {
                        out.push(
                            encoding_rs::WINDOWS_1252
                                .decode_without_bom_handling(&[byte])
                                .0
                                .chars()
                                .next()
                                .unwrap(),
                        );
                    }
                    i += 3;
                } else if n.is_ascii_alphabetic() {
                    let start = i;
                    while i < b.len() && b[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    let word = &s[start..i];
                    let ps = i;
                    if i < b.len() && (b[i] == b'-' || b[i].is_ascii_digit()) {
                        i += 1;
                        while i < b.len() && b[i].is_ascii_digit() {
                            i += 1;
                        }
                    }
                    let param = &s[ps..i];
                    if i < b.len() && b[i] == b' ' {
                        i += 1;
                    }
                    if skipping {
                        continue;
                    }
                    match word {
                        "par" | "line" | "row" => out.push('\n'),
                        "tab" => out.push('\t'),
                        "cell" => out.push(' '),
                        "u" => {
                            let v: i32 = param.parse().unwrap();
                            let cp = if v < 0 { v + 65536 } else { v } as u32;
                            out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                            pending_skip = 1;
                        }
                        _ => {}
                    }
                } else {
                    i += 1;
                }
            }
            b'\r' | b'\n' => i += 1,
            _ => {
                if !skipping {
                    if pending_skip > 0 {
                        pending_skip -= 1;
                    } else {
                        out.push(c as char);
                    }
                }
                i += 1;
            }
        }
    }
    // surrogate pairs were pushed as two replacement characters; good enough for BMP checks
    out
}

fn normalized(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn header_groups_and_ascii_output() {
    let s = rtf(&sample_book());
    assert!(s.starts_with("{\\rtf1\\ansi\\ansicpg1252"));
    assert!(s.is_ascii());
    assert!(check_braces(&s) >= 4);
    for g in [
        "{\\fonttbl",
        "{\\colortbl;",
        "{\\stylesheet",
        "{\\info{\\title ",
    ] {
        assert!(s.contains(g), "missing {g}");
    }
    assert!(s.contains("\\f0\\froman"), "default font");
    assert!(s.contains("\\paperw12240\\paperh15840"));
    assert!(s.trim_end().ends_with('}'));
    // header time: info carries title and author
    assert!(s.contains("{\\author Ada Lovelace & Grace Hopper}"));
    assert!(s.contains("{\\keywords testing, fiction}"));
    assert!(s.contains("{\\company Bookx Press}"));
    assert!(s.contains("{\\subject A short description & more.}"));
}

#[test]
fn text_survives_the_independent_extractor() {
    let s = rtf(&sample_book());
    let t = normalized(&rtf_text(&s));
    assert_sentences(&t, "rtf text");
    assert!(t.contains("日本語"), "unicode through \\uN? escapes: {t}");
    assert!(t.contains("café"));
}

#[test]
fn escaping_rules() {
    let book = book_with(
        &["<p>a\\b {c} é “q” € 日 😀</p><pre>tab\there</pre>"],
        false,
    );
    let s = rtf(&book);
    assert!(s.contains("a\\\\b \\{c\\}"));
    assert!(s.contains("\\'e9"), "é as cp1252");
    assert!(s.contains("\\'93q\\'94"), "curly quotes as cp1252");
    assert!(s.contains("\\'80"), "euro sign");
    assert!(s.contains("\\u26085?"), "CJK as signed 16-bit escape");
    // 😀 U+1F600 is the surrogate pair D83D DE00 = -10179, -8704
    assert!(
        s.contains("\\u-10179?\\u-8704?"),
        "astral characters as surrogates"
    );
    assert!(s.contains("tab\\tab here"));
    check_braces(&s);
}

#[test]
fn character_formatting_groups() {
    let book = book_with(
        &["<p><b>b</b><i>i</i><u>u</u><s>s</s><sup>p</sup><sub>q</sub><code>c</code></p>"],
        false,
    );
    let s = rtf(&book);
    for w in [
        "{\\b b}",
        "{\\i i}",
        "{\\ul u}",
        "{\\strike s}",
        "{\\super p}",
        "{\\sub q}",
        "{\\f1 c}",
    ] {
        assert!(s.contains(w), "missing {w}");
    }
}

#[test]
fn headings_have_styles_outline_levels_and_sizes() {
    let book = book_with(
        &["<h1>One</h1><h2>Two</h2><h3>Three</h3><h6>Six</h6>"],
        false,
    );
    let s = rtf(&book);
    assert!(s.contains(
        "{\\s1\\sbasedon0\\snext0\\sb240\\sa120\\keepn\\outlinelevel0\\b\\f0\\fs36 heading 1;}"
    ));
    assert!(s.contains("\\s2\\ql") && s.contains("\\outlinelevel1"));
    assert!(s.contains("\\s6\\ql") && s.contains("\\outlinelevel5"));
    assert!(s.contains("\\fs36 "));
    assert!(s.contains("\\fs22 "));
    assert_eq!(
        s.matches("\\keepn\\outlinelevel").count(),
        6 + 4,
        "stylesheet plus uses"
    );
}

#[test]
fn alignment_and_quotes_and_rules() {
    let book = book_with(
        &[
            "<p style=\"text-align:center\">c</p><p style=\"text-align:right\">r</p><blockquote><p>q</p></blockquote><hr/>",
        ],
        false,
    );
    let s = rtf(&book);
    assert!(s.contains("\\qc\\sa120"));
    assert!(s.contains("\\qr\\sa120"));
    assert!(s.contains("\\li720"), "quote indent");
    assert!(s.contains("\\brdrb\\brdrs"), "rule as a bottom border");
}

#[test]
fn hyperlinks_are_fields_with_bookmarks() {
    let s = rtf(&sample_book());
    assert!(s.contains("{\\field{\\*\\fldinst{HYPERLINK \"https://example.org/x\"}}{\\fldrslt{"));
    assert_eq!(
        s.matches("HYPERLINK \\\\l \"bx").count(),
        2,
        "two internal links"
    );
    // every linked name is a closed bookmark (bookmarks of the table of contents may be unlinked)
    let mut linked = 0;
    for (pos, _) in s.match_indices("HYPERLINK \\\\l \"") {
        let name: String = s[pos + 15..]
            .chars()
            .take_while(|c| c.is_alphanumeric())
            .collect();
        assert!(
            s.contains(&format!("{{\\*\\bkmkstart {name}}}")),
            "{name} starts"
        );
        assert!(
            s.contains(&format!("{{\\*\\bkmkend {name}}}")),
            "{name} ends"
        );
        linked += 1;
    }
    assert_eq!(linked, 2);
    // link text is blue and underlined
    assert!(s.contains("\\cf2\\ul "));
    check_braces(&s);
}

#[test]
fn field_targets_are_escaped() {
    let book = book_with(&["<p><a href=\"https://x.org/a\\b{c}\">l</a></p>"], false);
    let s = rtf(&book);
    assert!(s.contains("HYPERLINK \"https://x.org/a\\\\b\\{c\\}\""));
    check_braces(&s);
}

fn decode_hex(h: &str) -> Vec<u8> {
    let clean: String = h.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    (0..clean.len() / 2)
        .map(|i| u8::from_str_radix(&clean[i * 2..i * 2 + 2], 16).unwrap())
        .collect()
}

fn picts(s: &str) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(p) = rest.find("{\\pict") {
        let tail = &rest[p..];
        let end = tail.find('}').unwrap();
        let group = &tail[..end];
        let hex_start = group.find('\n').unwrap();
        out.push((
            group[..hex_start].to_string(),
            decode_hex(&group[hex_start..]),
        ));
        rest = &tail[end..];
    }
    out
}

#[test]
fn pictures_embed_the_original_bytes_with_sizes() {
    let book = book_with(
        &["<p><img src=\"pic.png\"/> <img src=\"pic.jpg\"/> <img src=\"anim.gif\"/></p>"],
        false,
    );
    let s = rtf(&book);
    let pics = picts(&s);
    assert_eq!(pics.len(), 3);
    assert!(
        pics[0]
            .0
            .starts_with("{\\pict\\pngblip\\picw40\\pich30\\picwgoal600\\pichgoal450")
    );
    assert_eq!(pics[0].1, png(40, 30));
    assert!(pics[1].0.starts_with("{\\pict\\jpegblip\\picw64\\pich48"));
    assert_eq!(pics[1].1, jpeg(64, 48));
    // GIF is converted to PNG, which RTF can carry
    assert!(pics[2].0.contains("\\pngblip\\picw16\\pich16"));
    assert!(pics[2].1.starts_with(&[0x89, b'P', b'N', b'G']));
    // hex lines are short
    assert!(
        s.lines()
            .all(|l| l.len() <= 200 || l.contains("\\fonttbl") || l.contains("\\stylesheet"))
    );
}

#[test]
fn wide_pictures_are_scaled_to_the_text_width() {
    let mut book = book_with(&["<p><img src=\"wide.png\"/></p>"], false);
    book.archive.set("OEBPS/wide.png", png(1200, 600));
    let s = rtf(&book);
    let p = picts(&s);
    assert!(
        p[0].0
            .contains("\\picw1200\\pich600\\picwgoal9360\\pichgoal4680"),
        "{}",
        p[0].0
    );
}

#[test]
fn cover_comes_first_and_chapters_break_pages() {
    let s = rtf(&sample_book());
    let first_pict = s.find("{\\pict").unwrap();
    let first_text = s.find("Chapter One").unwrap();
    assert!(first_pict < first_text, "cover before the text");
    assert!(s.contains("\\qc"), "centered cover");
    // cover + 3 chapters = 3 page-break-before paragraphs (cover->ch1, ch1->ch2, ch2->ch3)
    assert_eq!(s.matches("\\pagebb").count(), 3);
    let no_cover = book_with(&["<p>a</p>", "<p>b</p>"], false);
    assert_eq!(rtf(&no_cover).matches("\\pagebb").count(), 1);
}

#[test]
fn lists_get_markers_and_tabs() {
    let book = book_with(
        &["<ul><li>apple<ul><li>nested</li></ul></li></ul><ol><li>first</li><li>second</li></ol>"],
        false,
    );
    let s = rtf(&book);
    assert!(s.contains("\\u8226?\\tab apple"));
    assert!(s.contains("1.\\tab first"));
    assert!(s.contains("2.\\tab second"));
    assert!(s.contains("\\fi-360\\li720"));
    assert!(
        s.contains("\\fi-360\\li1440"),
        "nested list indents further"
    );
    let t = rtf_text(&s);
    assert!(t.contains("•\tapple") && t.contains("1.\tfirst"));
}

#[test]
fn tables_use_rows_cells_and_merges() {
    let book = book_with(
        &[
            "<table><tr><th>H1</th><th>H2</th><th>H3</th></tr><tr><td colspan=\"2\">wide</td><td>x</td></tr><tr><td>only</td></tr></table><p>after</p>",
        ],
        false,
    );
    let s = rtf(&book);
    assert_eq!(s.matches("\\trowd").count(), 3);
    assert_eq!(s.matches("\\row").count(), 3);
    assert_eq!(s.matches("\\cell\n").count(), 9, "3 rows x 3 grid columns");
    assert_eq!(s.matches("\\cellx").count(), 9);
    assert_eq!(s.matches("\\clmgf").count(), 1);
    assert_eq!(s.matches("\\clmrg").count(), 1);
    assert_eq!(s.matches("\\intbl").count(), 9);
    let t = normalized(&rtf_text(&s));
    assert!(
        t.contains("H1 H2 H3") && t.contains("wide") && t.contains("only") && t.contains("after")
    );
    check_braces(&s);
}

#[test]
fn preformatted_text_keeps_line_breaks() {
    let book = book_with(&["<pre>one\n  two</pre>"], false);
    let s = rtf(&book);
    assert!(rtf_text(&s).contains("one\n  two"), "{s}");
    assert!(s.contains("\\line "));
    assert!(s.contains("\\f1\\fs20"));
}

#[test]
fn empty_book_is_a_valid_empty_document() {
    let book = book_with(&["<div></div>"], false);
    let s = rtf(&book);
    check_braces(&s);
    assert!(s.starts_with("{\\rtf1"));
}

// ---- oracles ---------------------------------------------------------------------------------

#[test]
fn round_trips_through_the_rtf_reader() {
    use bookx_core::contracts::FormatReader;
    let bytes = RtfWriter.write(&sample_book()).unwrap();
    let archive = bookx_formats::rtf::RtfReader.read(&bytes).unwrap();
    let t = epub_text(archive);
    assert_sentences(&t, "rtf round trip");
    assert!(t.contains("日本語"));
}

#[test]
fn calibre_reads_our_rtf() {
    let bytes = RtfWriter.write(&sample_book()).unwrap();
    let Some(t) = calibre_text(&bytes, "ours.rtf") else {
        eprintln!("skipped: Calibre not available");
        return;
    };
    assert_sentences(&t, "Calibre read of our rtf");
}

#[test]
fn calibre_reads_formatting_of_our_rtf() {
    let book = book_with(
        &["<h1>Title</h1><p>plain <b>bold</b> <i>ital</i></p><ul><li>one</li></ul>"],
        false,
    );
    let bytes = RtfWriter.write(&book).unwrap();
    let Some(t) = calibre_text(&bytes, "ours.rtf") else {
        eprintln!("skipped: Calibre not available");
        return;
    };
    for w in ["Title", "plain", "bold", "ital", "one"] {
        assert!(t.contains(w), "{w} in {t}");
    }
}
