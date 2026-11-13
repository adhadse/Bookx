//! Read-later end to end: web page → article → fetched images → EPUB, validated independently.

use std::io::Read;

use bookx_core::archive::Archive;
use bookx_core::book::OpenBook;
use bookx_sources::cover::generate_cover;
use bookx_sources::fetch::{MockFetcher, MockReply};
use bookx_sources::readability::{article_to_epub, extract, fetch_images, ImageLimits};

const URL: &str = "https://news.example.org/2024/story/";

fn page() -> String {
    r##"<!doctype html><html lang="fr"><head><title>La Une | Le Monde Libre</title>
<meta property="og:title" content="La Une">
<meta property="og:site_name" content="Le Monde Libre">
<meta property="og:image" content="/media/lead.png">
<meta name="author" content="Zoé Müller">
<meta property="article:published_time" content="2024-05-06T07:08:09Z">
</head><body>
<nav><a href="/">Accueil</a></nav>
<article>
<h1>La Une</h1>
<p>Un texte d'introduction, avec des virgules, des accents (é, è, ê) et un peu de longueur pour compter.</p>
<p><img src="/media/photo.png" alt="Une photo"> Suite du texte, avec <a href="/autre">un lien</a>, et encore des mots pour la densité.</p>
<p><img src="/media/missing.png" alt="Absente"></p>
<p>日本語の段落もあります、そして終わりです。これは十分に長い文章です、ええ。</p>
</article>
<div class="comments"><p>Commentaire inutile, avec des virgules, et du texte, pour être sûr qu'il est ignoré.</p></div>
</body></html>"##
        .to_string()
}

fn zip_names_and_method(bytes: &[u8]) -> (Vec<String>, u16) {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("valid zip");
    let names: Vec<String> = (0..z.len())
        .map(|i| z.by_index(i).unwrap().name().unwrap().to_string())
        .collect();
    let first = z.by_index(0).unwrap();
    let method = match first.compression() {
        zip::CompressionMethod::Stored => 0,
        _ => 8,
    };
    (names, method)
}

fn read(bytes: &[u8], name: &str) -> Vec<u8> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut f = z.by_name(name).unwrap_or_else(|_| panic!("{name} missing"));
    let mut v = Vec::new();
    f.read_to_end(&mut v).unwrap();
    v
}

#[test]
fn page_to_epub() {
    let lead = generate_cover("Lead", "Image", "", 300, 450).unwrap();
    let photo = generate_cover("Photo", "Inline", "", 200, 200).unwrap();
    let fetcher = MockFetcher::new()
        .with_body("https://news.example.org/media/lead.png", &lead)
        .with_body("https://news.example.org/media/photo.png", &photo)
        .on(
            "https://news.example.org/media/missing.png",
            MockReply::Status(404),
        );

    let article = extract(&page(), URL).unwrap();
    assert_eq!(article.title, "La Une");
    assert_eq!(article.author, "Zoé Müller");
    assert_eq!(article.language, "fr");
    assert_eq!(
        article.images,
        vec![
            "https://news.example.org/media/photo.png",
            "https://news.example.org/media/missing.png"
        ]
    );
    assert!(article.content.contains("日本語の段落"));
    assert!(!article.content.contains("Commentaire") && !article.content.contains("Accueil"));

    let images = fetch_images(&fetcher, &article.all_image_urls(), &ImageLimits::default());
    assert_eq!(images.len(), 2, "the 404 image is skipped");
    let epub = article_to_epub(&article, &images).unwrap();
    let bytes = epub.to_bytes().unwrap();

    // --- independent structural validation ---
    let (names, method) = zip_names_and_method(&bytes);
    assert_eq!(names[0], "mimetype");
    assert_eq!(method, 0, "mimetype is stored");
    assert_eq!(read(&bytes, "mimetype"), b"application/epub+zip");
    let container = String::from_utf8(read(&bytes, "META-INF/container.xml")).unwrap();
    let cdoc = roxmltree::Document::parse(&container).unwrap();
    let opf_path = cdoc
        .descendants()
        .find(|n| n.tag_name().name() == "rootfile")
        .unwrap()
        .attribute("full-path")
        .unwrap()
        .to_string();
    let opf_text = String::from_utf8(read(&bytes, &opf_path)).unwrap();
    let opf = roxmltree::Document::parse(&opf_text).unwrap();
    let dir = opf_path
        .rsplit_once('/')
        .map(|(d, _)| format!("{d}/"))
        .unwrap_or_default();
    let mut ids = std::collections::HashSet::new();
    for item in opf.descendants().filter(|n| n.tag_name().name() == "item") {
        let href = item.attribute("href").unwrap();
        let path = format!("{dir}{href}");
        assert!(
            names.contains(&path),
            "manifest item {href} is not in the archive"
        );
        ids.insert(item.attribute("id").unwrap().to_string());
        let data = read(&bytes, &path);
        match item.attribute("media-type").unwrap() {
            "image/png" => assert!(data.starts_with(b"\x89PNG"), "{href} is not a PNG"),
            "image/jpeg" => assert!(data.starts_with(&[0xFF, 0xD8])),
            "application/xhtml+xml" => {
                let text = String::from_utf8(data).unwrap();
                let o = roxmltree::ParsingOptions {
                    allow_dtd: true,
                    ..Default::default()
                };
                roxmltree::Document::parse_with_options(&text, o)
                    .unwrap_or_else(|e| panic!("{href}: {e}"));
            }
            _ => {}
        }
    }
    for r in opf
        .descendants()
        .filter(|n| n.tag_name().name() == "itemref")
    {
        assert!(ids.contains(r.attribute("idref").unwrap()));
    }

    // --- what the app would see ---
    let book = OpenBook::from_archive(Archive::from_bytes(bytes).unwrap()).unwrap();
    assert_eq!(book.metadata().title, "La Une");
    assert_eq!(book.metadata().authors, vec!["Zoé Müller"]);
    assert_eq!(book.metadata().language, "fr");
    assert_eq!(book.metadata().published, "2024-05-06");
    assert!(book
        .cover_bytes()
        .is_some_and(|c| c.starts_with(b"\x89PNG")));
    let ch = book.chapter_source(&book.spine_paths()[0]).unwrap();
    assert!(ch.contains("images/img1.png"));
    assert!(
        !ch.contains("missing.png"),
        "the failed image is gone from the text"
    );
    assert!(
        ch.contains("Un texte d'introduction")
            || ch.contains("Un texte d&#39;introduction")
            || ch.contains("introduction")
    );
}

#[test]
fn page_that_cannot_be_read_gives_a_clear_error() {
    let e = extract("<html><body><script>1</script></body></html>", URL).unwrap_err();
    assert!(e.to_string().contains("no readable content"));
}
