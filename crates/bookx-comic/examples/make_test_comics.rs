//! Writes the synthetic comics the UI scenarios read: `make_test_comics DIR` creates
//! `DIR/story.cbz`, `manga.cbz`, `saga/Saga v01.cbz`, `saga/Saga v02.cbz`, `webtoon.cbz`, `spreads.cbz`, `bomb.cbz` and `corrupt.cbz`.

use bookx_comic::testgen::{bomb_png, png, small_comic, zip_of, Layout};

fn main() {
    let dir = std::env::args()
        .nth(1)
        .expect("usage: make_test_comics DIR");
    std::fs::create_dir_all(&dir).expect("create the folder");
    let w =
        |name: &str, bytes: Vec<u8>| std::fs::write(format!("{dir}/{name}"), bytes).expect("write");

    w(
        "story.cbz",
        small_comic(Some(
            r#"<?xml version="1.0"?><ComicInfo><Title>Gutter Tales</Title><Series>Panel Park</Series><Number>1</Number><Writer>Ada Inker</Writer><Year>2020</Year><Pages><Page Image="0" Type="FrontCover"/><Page Image="3" DoublePage="true"/><Page Image="5" Type="Preview"/></Pages></ComicInfo>"#,
        )),
    );
    let manga_pages: Vec<(String, Vec<u8>)> = vec![
        ("m01.png".into(), png(400, 600, Layout::Splash, 255)),
        ("m02.png".into(), png(400, 600, Layout::Grid(2, 2), 255)),
        ("m03.png".into(), png(400, 600, Layout::Mixed, 255)),
        ("m04.png".into(), png(400, 600, Layout::Grid(2, 3), 255)),
        (
            "ComicInfo.xml".into(),
            br#"<?xml version="1.0"?><ComicInfo><Title>Right Way</Title><Series>Reverse Road</Series><Number>2</Number><Writer>Ken Tate</Writer><Manga>YesAndRightToLeft</Manga></ComicInfo>"#.to_vec(),
        ),
    ];
    w("manga.cbz", zip_of(&manga_pages));
    // two volumes with no ComicInfo, in a folder that names the series (`saga/Saga v01.cbz`)
    std::fs::create_dir_all(format!("{dir}/saga")).expect("create the series folder");
    for (n, tint) in [(1, 255), (2, 254)] {
        let pages: Vec<(String, Vec<u8>)> = (1..=6)
            .map(|i| (format!("p{i}.png"), png(400, 600, Layout::Grid(2, 2), tint)))
            .collect();
        std::fs::write(format!("{dir}/saga/Saga v0{n}.cbz"), zip_of(&pages)).expect("write");
    }
    let webtoon: Vec<(String, Vec<u8>)> = (1..=4)
        .map(|i| {
            (
                format!("w{i:02}.png"),
                png(400, 1600, Layout::Grid(1, 4), 255),
            )
        })
        .collect();
    w("webtoon.cbz", zip_of(&webtoon));
    let mut spreads: Vec<(String, Vec<u8>)> = (1..=7)
        .map(|i| {
            (
                format!("s{i:02}.png"),
                png(400, 600, Layout::Grid(2, 2), 255),
            )
        })
        .collect();
    spreads[3].1 = png(800, 600, Layout::Grid(3, 1), 255);
    w("spreads.cbz", zip_of(&spreads));
    w(
        "bomb.cbz",
        zip_of(&[
            ("1.png".to_string(), png(400, 600, Layout::Grid(2, 2), 255)),
            ("2.png".to_string(), bomb_png(40_000, 40_000)),
        ]),
    );
    let mut broken = png(400, 600, Layout::Grid(2, 2), 255);
    broken.truncate(broken.len() / 2);
    w(
        "corrupt.cbz",
        zip_of(&[
            ("1.png".to_string(), png(400, 600, Layout::Grid(2, 2), 255)),
            ("2.png".to_string(), broken),
            ("3.png".to_string(), png(400, 600, Layout::Mixed, 255)),
        ]),
    );
}
