//! What the converter (`bookx-formats`, `comic.rs`) writes into the book about a comic, and how
//! the reader finds the pages again.
//!
//! The book is an EPUB with one `images/page-NNNN.ext` per page. The OPF carries
//! `<meta name="bookx:comic" content="1"/>`, `bookx:comic-double` (indexes of double pages) and
//! `bookx:comic-types` (page types of ComicInfo other than `Story`, `index:Type;index:Type`).

/// Page-type information from ComicInfo.xml.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PageInfo {
    pub double: Vec<usize>,
    pub types: Vec<(usize, String)>,
}

/// The `<meta>` lines the converter adds to the OPF (empty page info gives only the marker).
pub fn opf_meta(info: &PageInfo) -> String {
    let mut s = String::from("    <meta name=\"bookx:comic\" content=\"1\"/>\n");
    if !info.double.is_empty() {
        let v: Vec<String> = info.double.iter().map(|i| i.to_string()).collect();
        s.push_str(&format!(
            "    <meta name=\"bookx:comic-double\" content=\"{}\"/>\n",
            v.join(",")
        ));
    }
    if !info.types.is_empty() {
        let v: Vec<String> = info
            .types
            .iter()
            .map(|(i, t)| {
                format!(
                    "{i}:{}",
                    t.chars()
                        .filter(|c| c.is_ascii_alphanumeric())
                        .collect::<String>()
                )
            })
            .collect();
        s.push_str(&format!(
            "    <meta name=\"bookx:comic-types\" content=\"{}\"/>\n",
            v.join(";")
        ));
    }
    s
}

fn meta_content(opf: &str, name: &str) -> Option<String> {
    let key = format!("name=\"{name}\"");
    let at = opf.find(&key)?;
    let tag_start = opf[..at].rfind('<')?;
    let tag_end = at + opf[at..].find('>')?;
    let tag = &opf[tag_start..tag_end];
    let c = tag.find("content=\"")? + "content=\"".len();
    let end = tag[c..].find('"')?;
    Some(tag[c..c + end].to_string())
}

/// The page info stored in an OPF (None when it is not a comic made by Bookx).
pub fn read_opf(opf: &str) -> Option<PageInfo> {
    meta_content(opf, "bookx:comic")?;
    let double = meta_content(opf, "bookx:comic-double")
        .map(|s| s.split(',').filter_map(|v| v.trim().parse().ok()).collect())
        .unwrap_or_default();
    let types = meta_content(opf, "bookx:comic-types")
        .map(|s| {
            s.split(';')
                .filter_map(|e| {
                    let (i, t) = e.split_once(':')?;
                    Some((i.parse().ok()?, t.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    Some(PageInfo { double, types })
}

/// Whether a book is a comic: the marker, or (for books imported before the marker existed) a
/// first chapter that is nothing but a page image.
pub fn is_comic(opf: &str, first_chapter: Option<&str>) -> bool {
    read_opf(opf).is_some()
        || first_chapter.is_some_and(|c| {
            c.contains("<div class=\"page\"><img src=\"images/page-0001.") && c.len() < 600
        })
}

/// The page images of a comic book in reading order, from the names in the archive.
pub fn page_paths(names: &[String]) -> Vec<String> {
    let mut v: Vec<(usize, &String)> = names
        .iter()
        .filter_map(|n| {
            let rest = n.strip_prefix("OEBPS/images/page-")?;
            let (num, ext) = rest.split_once('.')?;
            if !matches!(ext, "jpg" | "png" | "gif" | "webp") {
                return None;
            }
            Some((num.parse().ok()?, n))
        })
        .collect();
    v.sort_by_key(|(i, _)| *i);
    v.into_iter().map(|(_, n)| n.clone()).collect()
}

/// The type ComicInfo gave a page ("Story" when none).
pub fn type_of(info: &PageInfo, page: usize) -> &str {
    info.types
        .iter()
        .find(|(i, _)| *i == page)
        .map(|(_, t)| t.as_str())
        .unwrap_or("Story")
}

/// `double[i]` for every page.
pub fn double_flags(info: &PageInfo, count: usize) -> Vec<bool> {
    let mut v = vec![false; count];
    for i in &info.double {
        if let Some(f) = v.get_mut(*i) {
            *f = true;
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_through_the_opf() {
        let info = PageInfo {
            double: vec![3, 8],
            types: vec![(0, "FrontCover".into()), (12, "Back Cover".into())],
        };
        let opf = format!("<metadata>\n{}</metadata>", opf_meta(&info));
        let back = read_opf(&opf).unwrap();
        assert_eq!(back.double, vec![3, 8]);
        assert_eq!(
            back.types,
            vec![(0, "FrontCover".to_string()), (12, "BackCover".to_string())],
            "only letters and digits are kept"
        );
        assert_eq!(type_of(&back, 0), "FrontCover");
        assert_eq!(type_of(&back, 5), "Story");
        assert!(double_flags(&back, 10)[3]);
        assert_eq!(double_flags(&back, 2), vec![false, false]);
    }

    #[test]
    fn marker_only_and_foreign_books() {
        let opf = format!("<metadata>{}</metadata>", opf_meta(&PageInfo::default()));
        assert_eq!(read_opf(&opf), Some(PageInfo::default()));
        assert!(read_opf("<metadata><meta name=\"other\" content=\"1\"/></metadata>").is_none());
        assert!(is_comic(&opf, None));
        assert!(!is_comic("<package/>", Some("<p>A novel chapter</p>")));
    }

    #[test]
    fn books_imported_before_the_marker_are_still_recognised() {
        let ch = "<html><body><div class=\"page\"><img src=\"images/page-0001.jpg\" alt=\"Page 1\"/></div>\n</body></html>";
        assert!(is_comic("<package/>", Some(ch)));
        let long = format!("{ch}{}", "x".repeat(700));
        assert!(!is_comic("<package/>", Some(&long)));
    }

    #[test]
    fn page_paths_are_in_numeric_order_and_ignore_other_files() {
        let names: Vec<String> = [
            "OEBPS/images/page-0010.jpg",
            "OEBPS/images/page-0002.png",
            "OEBPS/images/page-0001.jpg",
            "OEBPS/images/cover.jpg",
            "OEBPS/images/page-0003.bmp",
            "OEBPS/chapter-1.xhtml",
            "../evil/OEBPS/images/page-0004.jpg",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            page_paths(&names),
            vec![
                "OEBPS/images/page-0001.jpg",
                "OEBPS/images/page-0002.png",
                "OEBPS/images/page-0010.jpg"
            ]
        );
    }

    #[test]
    fn garbage_meta_does_not_panic() {
        assert!(read_opf("name=\"bookx:comic\"").is_none());
        let opf = "<meta name=\"bookx:comic\" content=\"1\"/><meta name=\"bookx:comic-double\" content=\"a,,5,\"/><meta name=\"bookx:comic-types\" content=\"x;3:\"/>";
        let i = read_opf(opf).unwrap();
        assert_eq!(i.double, vec![5]);
        assert_eq!(i.types, vec![(3, String::new())]);
    }
}
