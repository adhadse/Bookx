//! CSS rules: a small tolerant scanner (comments, strings, blocks, at-rules, `url()`), no cascade.

use std::sync::LazyLock;

use regex::Regex;

use super::{Cx, Features, Item, RefKind, add_ref, offset_pos};

const KNOWN_AT_RULES: &[&str] = &[
    "charset",
    "import",
    "namespace",
    "media",
    "supports",
    "page",
    "font-face",
    "keyframes",
    "counter-style",
    "font-feature-values",
    "viewport",
    "layer",
    "container",
    "property",
    "top-left",
    "top-center",
    "top-right",
    "bottom-left",
    "bottom-center",
    "bottom-right",
    "left-top",
    "left-middle",
    "left-bottom",
    "right-top",
    "right-middle",
    "right-bottom",
    "top-left-corner",
    "top-right-corner",
    "bottom-left-corner",
    "bottom-right-corner",
];

static URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)url\(\s*(?:"([^"]*)"|'([^']*)'|([^)\s]*))\s*\)"#).expect("static regex")
});
static IMPORT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)@import\s+(?:"([^"]*)"|'([^']*)')"#).expect("static regex"));
static FORBIDDEN_PROPERTY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:^|[;{\s])(direction|unicode-bidi)\s*:").expect("static regex")
});
static NAMESPACE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)@namespace[^;{]*").expect("static regex"));
static CHARSET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)^\s*@charset\s+(?:"([^"]*)"|'([^']*)')"#).expect("static regex")
});

pub(super) fn check_css(cx: &mut Cx, it: &Item) {
    let path = it.path.clone();
    let Some(bytes) = cx.get(&path) else { return };
    let text = match std::str::from_utf8(bytes) {
        Ok(t) => t.trim_start_matches('\u{feff}').to_string(),
        Err(_) => {
            cx.emit("CSS-003", &path, None, "the style sheet is not valid UTF-8");
            String::from_utf8_lossy(bytes).to_string()
        }
    };
    let mut f = Features::default();
    scan(cx, &path, &text, 0, &mut f);
    cx.features.insert(path, f);
}

/// Scans style sheet `text` found in `path` (for a `<style>` element, `line_base` is the line before it).
pub(super) fn scan(cx: &mut Cx, path: &str, text: &str, line_base: u32, feat: &mut Features) {
    let pos = |off: usize| offset_pos(text, off).map(|(l, c)| (l + line_base, c));
    // blank out comments and find unterminated strings, keeping offsets
    let bytes = text.as_bytes();
    let mut clean = bytes.to_vec();
    let mut i = 0;
    let mut stack: Vec<(usize, bool)> = Vec::new(); // block start, is @font-face
    let mut reported_syntax = false;
    let mut at_rule_start: Option<(String, usize)> = None;
    let mut last_at_font_face = false;
    while i < bytes.len() {
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                let end = text[i + 2..].find("*/").map(|e| i + 2 + e + 2);
                let stop = end.unwrap_or(bytes.len());
                for b in clean.iter_mut().take(stop).skip(i) {
                    if *b != b'\n' {
                        *b = b' ';
                    }
                }
                if end.is_none() && !reported_syntax {
                    reported_syntax = true;
                    cx.emit("CSS-008", path, pos(i), "unterminated comment");
                }
                i = stop;
            }
            q @ (b'"' | b'\'') => {
                let mut j = i + 1;
                let mut closed = false;
                while j < bytes.len() {
                    match bytes[j] {
                        b'\\' => j += 2,
                        b'\n' => break,
                        c if c == q => {
                            closed = true;
                            break;
                        }
                        _ => j += 1,
                    }
                }
                if !closed && !reported_syntax {
                    reported_syntax = true;
                    cx.emit("CSS-008", path, pos(i), "unterminated string");
                }
                i = (j + 1).min(bytes.len());
            }
            b'@' => {
                let rest = &text[i + 1..];
                let ident_len = rest
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
                    .unwrap_or(rest.len());
                let ident = rest[..ident_len].to_ascii_lowercase();
                let base = ident
                    .strip_prefix("-webkit-")
                    .or(ident.strip_prefix("-moz-"))
                    .or(ident.strip_prefix("-ms-"))
                    .or(ident.strip_prefix("-o-"))
                    .or(ident.strip_prefix("-epub-"))
                    .unwrap_or(&ident)
                    .to_string();
                if !KNOWN_AT_RULES.contains(&base.as_str()) {
                    cx.emit("CSS-902", path, pos(i), format!("unknown at-rule @{ident}"));
                }
                last_at_font_face = base == "font-face";
                at_rule_start = Some((ident, i));
                i += 1 + ident_len;
            }
            b'{' => {
                stack.push((i, std::mem::take(&mut last_at_font_face)));
                at_rule_start = None;
                i += 1;
            }
            b'}' => {
                if stack.pop().is_none() && !reported_syntax {
                    reported_syntax = true;
                    cx.emit("CSS-008", path, pos(i), "unexpected }");
                }
                i += 1;
            }
            b';' => {
                at_rule_start = None;
                i += 1;
            }
            _ => i += 1,
        }
    }
    let _ = at_rule_start;
    if let Some((start, _)) = stack.last() {
        if !reported_syntax {
            cx.emit("CSS-008", path, pos(*start), "unclosed block, } is missing");
        }
    }
    let clean = String::from_utf8_lossy(&clean).to_string();
    // namespace URLs are identifiers, not resources
    let refs_text = NAMESPACE.replace_all(&clean, |c: &regex::Captures| " ".repeat(c[0].len()));

    if let Some(c) = CHARSET.captures(&clean) {
        let cs = c.get(1).or(c.get(2)).map(|m| m.as_str()).unwrap_or("");
        if !cs.eq_ignore_ascii_case("utf-8") {
            cx.emit(
                "CSS-003",
                path,
                pos(0),
                format!("@charset \"{cs}\" is not UTF-8"),
            );
        }
    }
    if let Some(m) = FORBIDDEN_PROPERTY.captures(&clean) {
        let g = m.get(1).expect("group 1");
        cx.emit(
            "CSS-001",
            path,
            pos(g.start()),
            format!(
                "the property \"{}\" must not be used in an EPUB style sheet",
                g.as_str().to_ascii_lowercase()
            ),
        );
    }
    if cx.opts.extended {
        check_declarations(cx, path, &clean, line_base);
    }
    let mut refs: Vec<(usize, String)> = Vec::new();
    for c in IMPORT.captures_iter(&refs_text) {
        if let Some(m) = c.get(1).or(c.get(2)) {
            refs.push((m.start(), m.as_str().to_string()));
        }
    }
    for c in URL.captures_iter(&refs_text) {
        if let Some(m) = c.get(1).or(c.get(2)).or(c.get(3)) {
            if !m.as_str().is_empty() {
                refs.push((m.start(), m.as_str().to_string()));
            }
        }
    }
    for (off, href) in refs {
        if href.starts_with("data:") {
            continue;
        }
        if add_ref(cx, path, pos(off), &href, RefKind::Style) {
            feat.remote = true;
        }
    }
}

// ---------------------------------------------------------------------------
// declarations (extended checks)
// ---------------------------------------------------------------------------

const KNOWN_PROPERTIES: &str = "align-content align-items align-self all animation animation-delay animation-direction animation-duration animation-fill-mode animation-iteration-count animation-name animation-play-state animation-timing-function aspect-ratio azimuth backface-visibility background background-attachment background-blend-mode background-clip background-color background-image background-origin background-position background-position-x background-position-y background-repeat background-size block-size border border-block border-block-color border-block-end border-block-end-color border-block-end-style border-block-end-width border-block-start border-block-start-color border-block-start-style border-block-start-width border-block-style border-block-width border-bottom border-bottom-color border-bottom-left-radius border-bottom-right-radius border-bottom-style border-bottom-width border-collapse border-color border-image border-image-outset border-image-repeat border-image-slice border-image-source border-image-width border-inline border-inline-color border-inline-end border-inline-end-color border-inline-end-style border-inline-end-width border-inline-start border-inline-start-color border-inline-start-style border-inline-start-width border-inline-style border-inline-width border-left border-left-color border-left-style border-left-width border-radius border-right border-right-color border-right-style border-right-width border-spacing border-style border-top border-top-color border-top-left-radius border-top-right-radius border-top-style border-top-width border-width bottom box-decoration-break box-shadow box-sizing break-after break-before break-inside caption-side caret-color clear clip clip-path color color-scheme column-count column-fill column-gap column-rule column-rule-color column-rule-style column-rule-width column-span column-width columns content counter-increment counter-reset counter-set cue cue-after cue-before cursor direction display elevation empty-cells fill filter flex flex-basis flex-direction flex-flow flex-grow flex-shrink flex-wrap float font font-family font-feature-settings font-kerning font-language-override font-optical-sizing font-size font-size-adjust font-stretch font-style font-synthesis font-variant font-variant-alternates font-variant-caps font-variant-east-asian font-variant-ligatures font-variant-numeric font-variant-position font-variation-settings font-weight gap grid grid-area grid-auto-columns grid-auto-flow grid-auto-rows grid-column grid-column-end grid-column-gap grid-column-start grid-gap grid-row grid-row-end grid-row-gap grid-row-start grid-template grid-template-areas grid-template-columns grid-template-rows hanging-punctuation height hyphenate-character hyphens image-orientation image-rendering image-resolution inline-size inset inset-block inset-block-end inset-block-start inset-inline inset-inline-end inset-inline-start isolation justify-content justify-items justify-self left letter-spacing line-break line-height list-style list-style-image list-style-position list-style-type margin margin-block margin-block-end margin-block-start margin-bottom margin-inline margin-inline-end margin-inline-start margin-left margin-right margin-top mask max-block-size max-height max-inline-size max-width min-block-size min-height min-inline-size min-width mix-blend-mode object-fit object-position opacity order orphans outline outline-color outline-offset outline-style outline-width overflow overflow-wrap overflow-x overflow-y padding padding-block padding-block-end padding-block-start padding-bottom padding-inline padding-inline-end padding-inline-start padding-left padding-right padding-top page page-break-after page-break-before page-break-inside pause pause-after pause-before pitch pitch-range place-content place-items place-self play-during pointer-events position quotes richness right rotate ruby-align ruby-position scale scroll-behavior shape-image-threshold shape-margin shape-outside size speak speak-header speak-numeral speak-punctuation speech-rate src stress tab-size table-layout text-align text-align-last text-combine-upright text-decoration text-decoration-color text-decoration-line text-decoration-style text-decoration-thickness text-emphasis text-emphasis-color text-emphasis-position text-emphasis-style text-indent text-justify text-orientation text-overflow text-shadow text-transform text-underline-offset text-underline-position top transform transform-origin transform-style transition transition-delay transition-duration transition-property transition-timing-function translate unicode-bidi unicode-range user-select vertical-align visibility voice-family volume white-space widows width will-change word-break word-spacing word-wrap writing-mode z-index font-display ascent-override descent-override line-gap-override size-adjust";

static KNOWN: std::sync::LazyLock<std::collections::HashSet<&'static str>> =
    std::sync::LazyLock::new(|| KNOWN_PROPERTIES.split_whitespace().collect());

/// Splits at `;` outside of strings and parentheses: (offset in `body`, chunk).
fn split_declarations(body: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let (mut depth, mut quote, mut start) = (0i32, None::<char>, 0usize);
    let mut escaped = false;
    for (i, c) in body.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match (c, quote) {
            ('\\', _) => escaped = true,
            (q, Some(open)) if q == open => quote = None,
            (_, Some(_)) => {}
            ('"' | '\'', None) => quote = Some(c),
            ('(', None) => depth += 1,
            (')', None) => depth -= 1,
            (';', None) if depth <= 0 => {
                out.push((start, &body[start..i]));
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < body.len() {
        out.push((start, &body[start..]));
    }
    out
}

fn is_ident(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with(|c: char| c.is_ascii_digit())
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || !c.is_ascii())
}

/// Problems of one declaration chunk (`property: value`): (rule id, text).
pub fn declaration_problem(chunk: &str) -> Option<(&'static str, String)> {
    let c = chunk.trim();
    if c.is_empty() {
        return None;
    }
    let Some((prop, value)) = c.split_once(':') else {
        return Some((
            "BKX-010",
            format!("\"{}\" is not a declaration: there is no colon", short(c)),
        ));
    };
    let (prop, value) = (prop.trim(), value.trim());
    if !is_ident(prop) {
        return Some((
            "BKX-010",
            format!("\"{}\" is not a valid property name", short(prop)),
        ));
    }
    if value.is_empty() {
        return Some(("BKX-010", format!("the property \"{prop}\" has no value")));
    }
    let opens = value.matches('(').count();
    let closes = value.matches(')').count();
    if opens != closes {
        return Some((
            "BKX-010",
            format!("the value of \"{prop}\" has unbalanced parentheses"),
        ));
    }
    let lower = prop.to_ascii_lowercase();
    if !lower.starts_with('-')
        && !lower.starts_with("mso-")
        && !lower.starts_with("adobe-")
        && !KNOWN.contains(lower.as_str())
    {
        return Some(("BKX-011", format!("\"{prop}\" is not a known CSS property")));
    }
    None
}

fn short(s: &str) -> String {
    let t: String = s.chars().take(40).collect();
    if t.len() < s.len() {
        format!("{t}...")
    } else {
        t
    }
}

/// Checks the declarations of every innermost block of a style sheet (comments already blanked).
pub(super) fn check_declarations(cx: &mut Cx, path: &str, clean: &str, line_base: u32) {
    let b = clean.as_bytes();
    // (block start, boundary before the selector, has nested block)
    let mut stack: Vec<(usize, usize, bool)> = Vec::new();
    let mut boundary = 0usize;
    let mut i = 0;
    let mut reported = 0;
    while i < b.len() {
        match b[i] {
            q @ (b'"' | b'\'') => {
                i += 1;
                while i < b.len() && b[i] != q && b[i] != b'\n' {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'{' => {
                if let Some(top) = stack.last_mut() {
                    top.2 = true;
                }
                stack.push((i, boundary, false));
                boundary = i + 1;
            }
            b'}' => {
                if let Some((start, sel_start, nested)) = stack.pop() {
                    if !nested {
                        let selector = clean[sel_start..start].trim_start();
                        let font_face = selector.starts_with("@font-face");
                        reported += check_block(
                            cx,
                            path,
                            clean,
                            start + 1,
                            i,
                            line_base,
                            font_face,
                            50usize.saturating_sub(reported),
                        );
                    }
                }
                boundary = i + 1;
            }
            b';' => boundary = i + 1,
            _ => {}
        }
        i += 1;
    }
}

#[allow(clippy::too_many_arguments)]
fn check_block(
    cx: &mut Cx,
    path: &str,
    text: &str,
    from: usize,
    to: usize,
    line_base: u32,
    font_face: bool,
    budget: usize,
) -> usize {
    let body = &text[from..to];
    let mut n = 0;
    for (off, chunk) in split_declarations(body) {
        if n >= budget {
            break;
        }
        let lead = chunk.len() - chunk.trim_start().len();
        if let Some((id, msg)) = declaration_problem(chunk) {
            if font_face && id == "BKX-011" {
                continue;
            }
            let at = offset_pos(text, from + off + lead).map(|(l, c)| (l + line_base, c));
            cx.emit(rule_id(id), path, at, msg);
            n += 1;
        }
    }
    n
}

fn rule_id(id: &str) -> &'static str {
    if id == "BKX-011" {
        "BKX-011"
    } else {
        "BKX-010"
    }
}

/// Checks a `style="..."` attribute value found on the line `line`.
pub(super) fn check_declaration_list(cx: &mut Cx, path: &str, value: &str, line: u32) {
    for (_, chunk) in split_declarations(value) {
        if let Some((id, msg)) = declaration_problem(chunk) {
            cx.emit(
                rule_id(id),
                path,
                Some((line, 1)),
                format!("in a style attribute: {msg}"),
            );
        }
    }
}
