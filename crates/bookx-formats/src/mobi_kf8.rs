//! Assembly of KF8 (AZW3) HTML parts from the skeleton and fragment tables, and resolution of
//! `kindle:pos:fid:XXXX:off:YYYYYYYYYY` links into ordinary `#id` anchors.

use std::collections::BTreeMap;

use regex::bytes::Regex;

use crate::mobi_index::{Fragment, Index};

pub struct Assembled {
    /// The HTML files, in reading order.
    pub parts: Vec<Vec<u8>>,
    /// Per fragment (in table order): `(part, start of the fragment inside the part)`.
    pub frag_start: Vec<(usize, usize)>,
}

/// Puts the fragments into their skeletons exactly where the fragment table says.
pub fn assemble(
    flow0: &[u8],
    skeletons: &[(usize, usize, usize)],
    fragments: &[Fragment],
) -> Option<Assembled> {
    let mut parts = Vec::new();
    let mut frag_start = Vec::new();
    let mut fptr = 0usize;
    for (si, &(count, start, len)) in skeletons.iter().enumerate() {
        let mut base = start.checked_add(len)?;
        let mut skeleton = flow0.get(start..base)?.to_vec();
        for _ in 0..count {
            let f = fragments.get(fptr)?;
            let slice = flow0.get(base..base.checked_add(f.length)?)?;
            let mut at = f.insert_pos.saturating_sub(start).min(skeleton.len());
            // a position inside a tag (seen in badly formed files) moves to the end of that tag
            let before = &skeleton[..at];
            if before.iter().rposition(|b| *b == b'<') > before.iter().rposition(|b| *b == b'>') {
                if let Some(gt) = skeleton[at..].iter().position(|b| *b == b'>') {
                    at += gt + 1;
                }
            }
            skeleton.splice(at..at, slice.iter().copied());
            frag_start.push((si, at));
            base += f.length;
            fptr += 1;
        }
        parts.push(skeleton);
    }
    (!parts.is_empty()).then_some(Assembled { parts, frag_start })
}

fn base32(s: &[u8]) -> Option<usize> {
    let mut v = 0usize;
    for c in s {
        let d = (*c as char).to_digit(32)? as usize;
        v = v.checked_mul(32)?.checked_add(d)?;
    }
    Some(v)
}

fn link_regex() -> Regex {
    Regex::new(r#"kindle:pos:fid:([0-9A-Va-v]{4}):off:([0-9A-Va-v]{10})"#).expect("static regex")
}

/// Where a `(fragment, offset)` link points: `(part, byte position inside the part)`.
pub fn locate(asm: &Assembled, fid: usize, off: usize) -> Option<(usize, usize)> {
    let (part, start) = *asm.frag_start.get(fid)?;
    let len = asm.parts.get(part)?.len();
    Some((part, (start + off).min(len.saturating_sub(1))))
}

/// Gives every link target an `id` (reusing the one the element has) and turns the
/// `kindle:pos` links into `#id` links. `extra` are additional `(fid, off)` targets (the
/// table of contents); the returned map names the id of each target.
pub fn resolve_links(
    asm: &mut Assembled,
    extra: &[(usize, usize)],
) -> BTreeMap<(usize, usize), String> {
    let re = link_regex();
    let mut wanted: Vec<(usize, usize)> = extra.to_vec();
    for p in &asm.parts {
        for c in re.captures_iter(p) {
            if let (Some(f), Some(o)) = (base32(&c[1]), base32(&c[2])) {
                wanted.push((f, o));
            }
        }
    }
    wanted.sort_unstable();
    wanted.dedup();
    // (part, position of the element's tag) -> id, applied from the end of each part
    let mut by_part: BTreeMap<usize, BTreeMap<usize, Vec<(usize, usize)>>> = BTreeMap::new();
    for &(f, o) in &wanted {
        let Some((part, pos)) = locate(asm, f, o) else {
            continue;
        };
        let p = &asm.parts[part];
        let from = pos.min(p.len());
        // the tag the position lies in, or the next one that opens after it
        let tag_open = match p[from..].iter().position(|b| *b == b'>') {
            Some(gt) => {
                let end = from + gt;
                match p[..end].iter().rposition(|b| *b == b'<') {
                    Some(open)
                        if p.get(open + 1)
                            .is_some_and(|c| !matches!(*c, b'/' | b'!' | b'?')) =>
                    {
                        open
                    }
                    _ => match p[end..].iter().position(|b| *b == b'<') {
                        Some(n) => end + n,
                        None => continue,
                    },
                }
            }
            None => continue,
        };
        by_part
            .entry(part)
            .or_default()
            .entry(tag_open)
            .or_default()
            .push((f, o));
    }
    let mut names: BTreeMap<(usize, usize), String> = BTreeMap::new();
    let id_re = regex::Regex::new(r#"\sid\s*=\s*["']([^"']+)["']"#).expect("static regex");
    for (part, tags) in by_part {
        let bytes = &mut asm.parts[part];
        for (open, targets) in tags.into_iter().rev() {
            let Some(close) = bytes[open..].iter().position(|b| *b == b'>') else {
                continue;
            };
            let tag = String::from_utf8_lossy(&bytes[open..open + close + 1]).into_owned();
            let existing = id_re.captures(&tag).map(|c| c[1].to_string());
            let id = match existing {
                Some(id) => id,
                None => {
                    let id = format!("kf8-{}-{}", targets[0].0, targets[0].1);
                    let name_end = tag
                        .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
                        .unwrap_or(tag.len());
                    let ins = open + name_end;
                    for (i, b) in format!(" id=\"{id}\"").bytes().enumerate() {
                        bytes.insert(ins + i, b);
                    }
                    id
                }
            };
            for t in targets {
                names.insert(t, id.clone());
            }
        }
    }
    for p in &mut asm.parts {
        *p = re
            .replace_all(p, |c: &regex::bytes::Captures| {
                let target = base32(&c[1]).zip(base32(&c[2]));
                match target.and_then(|t| names.get(&t)) {
                    Some(id) => format!("#{id}").into_bytes(),
                    None => b"#".to_vec(),
                }
            })
            .into_owned();
    }
    names
}

/// Number of fragments, for sanity checks.
pub fn fragment_count(index: &Index) -> usize {
    index.entries.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frag(insert_pos: usize, length: usize) -> Fragment {
        Fragment {
            insert_pos,
            file: 0,
            length,
        }
    }

    #[test]
    fn fragments_go_into_their_skeletons() {
        // skeleton "<html><body></body></html>" (26 bytes) with two fragments inserted after <body>
        let skel = b"<html><body></body></html>";
        let a = b"<p id=\"a\">one</p>";
        let b = b"<p id=\"b\">two</p>";
        let mut flow0 = skel.to_vec();
        flow0.extend_from_slice(a);
        flow0.extend_from_slice(b);
        let insert = 12; // after <html><body>
        let asm = assemble(
            &flow0,
            &[(2, 0, skel.len())],
            &[frag(insert, a.len()), frag(insert + a.len(), b.len())],
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(asm.parts[0].clone()).unwrap(),
            "<html><body><p id=\"a\">one</p><p id=\"b\">two</p></body></html>"
        );
        assert_eq!(asm.frag_start, vec![(0, 12), (0, 12 + a.len())]);
    }

    #[test]
    fn two_files_and_bad_tables() {
        let flow0 = b"<a></a><b></b>";
        let asm = assemble(flow0, &[(0, 0, 7), (0, 7, 7)], &[]).unwrap();
        assert_eq!(asm.parts.len(), 2);
        assert!(assemble(flow0, &[(1, 0, 7)], &[]).is_none());
        assert!(assemble(flow0, &[(0, 10, 99)], &[]).is_none());
        assert!(assemble(flow0, &[], &[]).is_none());
    }

    #[test]
    fn links_become_anchors_and_ids_are_reused() {
        let flow0 = b"<html><body><p id=\"x\">t</p><p>u <a href=\"kindle:pos:fid:0000:off:0000000000\">go</a> <a href=\"kindle:pos:fid:0000:off:0000000013\">go2</a></p></body></html>";
        let mut asm = assemble(flow0, &[(0, 0, flow0.len())], &[]).unwrap();
        // no fragments: fid 0 does not exist -> links degrade to "#"
        let names = resolve_links(&mut asm, &[]);
        assert!(names.is_empty());
        let s = String::from_utf8(asm.parts[0].clone()).unwrap();
        assert!(!s.contains("kindle:pos"));

        // with a fragment starting at the body content
        let body_at = 12;
        let skel = b"<html><body></body></html>";
        let frag_html = b"<p id=\"x\">t</p><p>u <a href=\"kindle:pos:fid:0000:off:0000000000\">go</a> <a href=\"kindle:pos:fid:0000:off:0000000011\">go2</a></p>";
        let mut flow = skel.to_vec();
        flow.extend_from_slice(frag_html);
        let mut asm = assemble(
            &flow,
            &[(1, 0, skel.len())],
            &[frag(body_at, frag_html.len())],
        )
        .unwrap();
        let names = resolve_links(&mut asm, &[]);
        let s = String::from_utf8(asm.parts[0].clone()).unwrap();
        assert_eq!(names[&(0, 0)], "x", "{s}");
        assert!(s.contains("href=\"#x\""), "{s}");
        assert!(
            s.contains("href=\"#kf8-0-17\"") || s.contains("href=\"#x\""),
            "{s}"
        );
    }
}
