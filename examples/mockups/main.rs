//! Generates the Bookx design mockups (SVG + PNG).
//!
//! ```sh
//! cargo run --example mockups
//! ```
//!
//! The SVGs are written to `mockups/src/` and rendered to `mockups/*.png` with resvg,
//! so the design documents are produced by the same toolchain as the app.

#![allow(clippy::too_many_arguments, clippy::type_complexity)]

mod banner;
mod dialogs;
mod editor;
mod eink;
mod kit;
mod library;
mod reader;

use std::path::Path;

type Sheet = (&'static str, f64, f64, fn(&mut kit::Svg));

/// Every sheet: file name (without extension), size in pixels, and the function that draws it.
const SHEETS: [Sheet; 6] = [
    ("bookx-banner", 3100.0, 3300.0, banner::run),
    ("bookx-library", 3200.0, 7540.0, library::run),
    ("bookx-reader", 3200.0, 6060.0, reader::run),
    ("bookx-dialogs", 3200.0, 2740.0, dialogs::run),
    ("bookx-eink", 3200.0, 1880.0, eink::run),
    ("bookx-editor", 3200.0, 7600.0, editor::run),
];

/// Draws one sheet to SVG text.
fn draw(w: f64, h: f64, f: fn(&mut kit::Svg)) -> String {
    let mut svg = kit::Svg::new(w, h);
    f(&mut svg);
    svg.finish()
}

fn rasterize(svg: &str) -> resvg::tiny_skia::Pixmap {
    let mut opt = resvg::usvg::Options::default();
    opt.fontdb_mut().load_system_fonts();
    let tree = resvg::usvg::Tree::from_str(svg, &opt).expect("valid svg");
    let size = tree.size().to_int_size();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size.width(), size.height()).unwrap();
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::default(),
        &mut pixmap.as_mut(),
    );
    pixmap
}

fn render(svg: &str, out: &Path) {
    rasterize(svg).save_png(out).expect("write png");
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("mockups");
    std::fs::create_dir_all(root.join("src")).unwrap();

    for (name, w, h, f) in SHEETS {
        let text = draw(w, h, f);
        std::fs::write(root.join("src").join(format!("{name}.svg")), &text).unwrap();
        render(&text, &root.join(format!("{name}.png")));
        println!("wrote {name}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheet_names_are_unique() {
        let mut names: Vec<&str> = SHEETS.iter().map(|s| s.0).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), SHEETS.len());
    }

    #[test]
    fn every_sheet_renders_to_svg_of_the_expected_size() {
        for (name, w, h, f) in SHEETS {
            let svg = draw(w, h, f);
            assert!(svg.starts_with("<?xml"), "{name}: not an XML document");
            assert!(svg.trim_end().ends_with("</svg>"), "{name}: unterminated");
            assert!(
                svg.contains(&format!("width=\"{w}\" height=\"{h}\"")),
                "{name}: size"
            );
            assert!(
                svg.len() > 20_000,
                "{name}: suspiciously small ({} bytes)",
                svg.len()
            );
            assert!(
                !svg.contains("NaN") && !svg.contains("inf"),
                "{name}: bad number"
            );
            let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default())
                .unwrap_or_else(|e| panic!("{name}: invalid svg: {e}"));
            let size = tree.size();
            assert_eq!(
                (size.width(), size.height()),
                (w as f32, h as f32),
                "{name}"
            );
        }
    }

    #[test]
    fn every_sheet_rasterizes_to_a_png_with_content() {
        for (name, w, h, f) in SHEETS {
            let pixmap = rasterize(&draw(w, h, f));
            assert_eq!(
                (pixmap.width(), pixmap.height()),
                (w as u32, h as u32),
                "{name}"
            );
            let png = pixmap.encode_png().expect("encode");
            assert!(
                png.starts_with(&[0x89, b'P', b'N', b'G']),
                "{name}: not a PNG"
            );
            // the page is not blank: many pixels differ from the white background
            let inked = pixmap
                .pixels()
                .iter()
                .filter(|p| (p.red(), p.green(), p.blue()) != (255, 255, 255))
                .count();
            assert!(inked > 50_000, "{name}: only {inked} inked pixels");
        }
    }

    #[test]
    fn sheets_stay_inside_their_canvas() {
        // the last text on every sheet (the footer) sits above the bottom edge
        for (name, w, h, f) in SHEETS {
            let svg = draw(w, h, f);
            let max_y = svg
                .split("y=\"")
                .skip(1)
                .filter_map(|r| r.split('"').next()?.parse::<f64>().ok())
                .fold(0.0, f64::max);
            assert!(
                max_y < h,
                "{name}: content at y={max_y} is below the canvas ({h})"
            );
        }
    }

    #[test]
    fn the_library_sheet_shows_the_device_and_panel_design() {
        let lib = draw(3200.0, 7540.0, library::run);
        for needle in [
            "Kobo Libra 2 connected — Sync reading progress and notes?",
            "Always",
            "Not now",
            "Sync with Kobo Libra 2?",
            "5 highlights go device→Bookx",
            "Nothing is deleted on either side.",
            "Books to send",
            "Send 2 books",
            "EPUB → KEPUB (converted)",
            "+ 3 notes added as a last section",
            "Add my notes to the copy sent to the device",
            "Sync automatically when a device connects",
            "News Subscriptions…",
        ] {
            assert!(lib.contains(needle), "library sheet lacks {needle}");
        }
        // the book menu has the one device entry, not one per device
        assert!(lib.contains("Send to Device…"));
        assert!(!lib.contains("Send to Kobo Libra 2"));
    }

    #[test]
    fn colour_sheets_show_the_new_controls() {
        let reader = draw(3200.0, 6060.0, reader::run);
        for needle in ["Original", "Custom", "#ffcc00", "#fdf6e3", "#cc0000"] {
            assert!(reader.contains(needle), "reader sheet lacks {needle}");
        }
        let editor = draw(3200.0, 7600.0, editor::run);
        for needle in [
            "Check Spelling",
            "wrods",
            "[RSC-007]",
            "[OPF-003]",
            "Custom",
            "Select to Matching Tag",
            "Generate Contents",
            "3 of 12 in this file",
            "Link Stylesheets…",
        ] {
            assert!(editor.contains(needle), "editor sheet lacks {needle}");
        }
    }
}
