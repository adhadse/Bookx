//! Tiny syntax highlighter for the code editor (XML/HTML and CSS) using `GtkTextTag`s.

use relm4::gtk::{self, prelude::*};

pub const TAGS: [&str; 7] = [
    "syn-tag",
    "syn-attr",
    "syn-string",
    "syn-comment",
    "syn-entity",
    "syn-prop",
    "syn-sel",
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Xml,
    Css,
    Plain,
}

impl Lang {
    pub fn for_path(path: &str) -> Lang {
        match crate::core::util::extension(path).as_str() {
            "xhtml" | "html" | "htm" | "xht" | "xml" | "opf" | "ncx" | "svg" => Lang::Xml,
            "css" => Lang::Css,
            _ => Lang::Plain,
        }
    }
}

pub fn make_tag_table(dark: bool) -> gtk::TextTagTable {
    let table = gtk::TextTagTable::new();
    update_colors(&table, dark);
    table
}

pub fn update_colors(table: &gtk::TextTagTable, dark: bool) {
    let colors: [(&str, &str, bool, bool); 7] = if dark {
        [
            ("syn-tag", "#78aeed", false, false),
            ("syn-attr", "#8ff0a4", false, false),
            ("syn-string", "#f9f06b", false, false),
            ("syn-comment", "#9a9996", false, true),
            ("syn-entity", "#ffbe6f", false, false),
            ("syn-prop", "#99c1f1", false, false),
            ("syn-sel", "#dc8add", true, false),
        ]
    } else {
        [
            ("syn-tag", "#1a5fb4", false, false),
            ("syn-attr", "#26a269", false, false),
            ("syn-string", "#986a00", false, false),
            ("syn-comment", "#77767b", false, true),
            ("syn-entity", "#c64600", false, false),
            ("syn-prop", "#1c71d8", false, false),
            ("syn-sel", "#813d9c", true, false),
        ]
    };
    if table.lookup("marked-region").is_none() {
        let t = gtk::TextTag::new(Some("marked-region"));
        t.set_background(Some("rgba(53,132,228,0.25)"));
        table.add(&t);
    }
    for (name, color, bold, italic) in colors {
        let tag = table.lookup(name).unwrap_or_else(|| {
            let t = gtk::TextTag::new(Some(name));
            table.add(&t);
            t
        });
        tag.set_foreground(Some(color));
        tag.set_weight(if bold { 700 } else { 400 });
        tag.set_style(if italic {
            gtk::pango::Style::Italic
        } else {
            gtk::pango::Style::Normal
        });
    }
}

type Span = (usize, usize, &'static str);

/// Returns (start_char, end_char, tag) triples. Offsets are in characters.
pub fn scan(text: &str, lang: Lang) -> Vec<Span> {
    match lang {
        Lang::Xml => scan_xml(text),
        Lang::Css => scan_css(text),
        Lang::Plain => Vec::new(),
    }
}

fn scan_xml(text: &str) -> Vec<Span> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut out = Vec::new();
    let mut i = 0;
    let starts = |i: usize, s: &str| -> bool {
        let sc: Vec<char> = s.chars().collect();
        i + sc.len() <= n && chars[i..i + sc.len()] == sc[..]
    };
    while i < n {
        if chars[i] == '<' {
            if starts(i, "<!--") {
                let mut j = i + 4;
                while j < n && !starts(j, "-->") {
                    j += 1;
                }
                let end = (j + 3).min(n);
                out.push((i, end, "syn-comment"));
                i = end;
                continue;
            }
            // tag
            let tag_start = i;
            let mut j = i + 1;
            if j < n && (chars[j] == '/' || chars[j] == '?' || chars[j] == '!') {
                j += 1;
            }
            while j < n && !chars[j].is_whitespace() && chars[j] != '>' && chars[j] != '/' {
                j += 1;
            }
            out.push((tag_start, j, "syn-tag"));
            // attributes
            while j < n && chars[j] != '>' {
                if chars[j] == '"' || chars[j] == '\'' {
                    let q = chars[j];
                    let s = j;
                    j += 1;
                    while j < n && chars[j] != q {
                        j += 1;
                    }
                    j = (j + 1).min(n);
                    out.push((s, j, "syn-string"));
                } else if chars[j].is_alphanumeric()
                    || chars[j] == '-'
                    || chars[j] == ':'
                    || chars[j] == '_'
                {
                    let s = j;
                    while j < n
                        && (chars[j].is_alphanumeric() || matches!(chars[j], '-' | ':' | '_' | '.'))
                    {
                        j += 1;
                    }
                    out.push((s, j, "syn-attr"));
                } else {
                    j += 1;
                }
            }
            if j < n {
                // closing '>' (with optional '/' or '?' before)
                let s = if j > 0 && matches!(chars[j - 1], '/' | '?') {
                    j - 1
                } else {
                    j
                };
                out.push((s.max(tag_start), j + 1, "syn-tag"));
                j += 1;
            }
            i = j;
        } else if chars[i] == '&' {
            let mut j = i + 1;
            while j < n && j - i < 12 && (chars[j].is_alphanumeric() || chars[j] == '#') {
                j += 1;
            }
            if j < n && chars[j] == ';' {
                out.push((i, j + 1, "syn-entity"));
                i = j + 1;
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    out
}

fn scan_css(text: &str) -> Vec<Span> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut out = Vec::new();
    let mut i = 0;
    let mut depth = 0i32;
    let mut seg_start = 0usize;
    let mut in_value = false;
    while i < n {
        let c = chars[i];
        if c == '/' && i + 1 < n && chars[i + 1] == '*' {
            let mut j = i + 2;
            while j + 1 < n && !(chars[j] == '*' && chars[j + 1] == '/') {
                j += 1;
            }
            let end = (j + 2).min(n);
            out.push((i, end, "syn-comment"));
            i = end;
            seg_start = i;
            continue;
        }
        if c == '"' || c == '\'' {
            let mut j = i + 1;
            while j < n && chars[j] != c {
                j += 1;
            }
            let end = (j + 1).min(n);
            out.push((i, end, "syn-string"));
            i = end;
            continue;
        }
        match c {
            '{' => {
                if depth == 0 && i > seg_start {
                    out.push((seg_start, i, "syn-sel"));
                }
                depth += 1;
                in_value = false;
                seg_start = i + 1;
            }
            '}' => {
                depth = (depth - 1).max(0);
                in_value = false;
                seg_start = i + 1;
            }
            ':' if depth > 0 && !in_value => {
                if i > seg_start {
                    out.push((seg_start, i, "syn-prop"));
                }
                in_value = true;
            }
            ';' => {
                in_value = false;
                seg_start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out
}

pub fn apply(buffer: &gtk::TextBuffer, lang: Lang) {
    let (s, e) = buffer.bounds();
    for t in TAGS {
        buffer.remove_tag_by_name(t, &s, &e);
    }
    if lang == Lang::Plain {
        return;
    }
    let text = buffer.text(&s, &e, false);
    if text.len() > 600_000 {
        return;
    }
    for (a, b, tag) in scan(&text, lang) {
        if b > a {
            let sa = buffer.iter_at_offset(a as i32);
            let sb = buffer.iter_at_offset(b as i32);
            buffer.apply_tag_by_name(tag, &sa, &sb);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_tokens() {
        let spans = scan("<a href=\"x\">t &amp; u</a>", Lang::Xml);
        assert!(spans.iter().any(|s| s.2 == "syn-attr"));
        assert!(spans.iter().any(|s| s.2 == "syn-string"));
        assert!(spans.iter().any(|s| s.2 == "syn-entity"));
    }

    #[test]
    fn css_tokens() {
        let spans = scan("p.a { color: red; } /* c */", Lang::Css);
        assert!(spans.iter().any(|s| s.2 == "syn-sel"));
        assert!(spans.iter().any(|s| s.2 == "syn-prop"));
        assert!(spans.iter().any(|s| s.2 == "syn-comment"));
    }
}
