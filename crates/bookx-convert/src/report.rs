//! The conversion report: what the chosen output format cannot hold, and what the transforms
//! changed. Shown by `bookx-cli convert --report`.

use bookx_core::book::OpenBook;
use serde::Serialize;

use crate::inspect::Digest;

/// What an output format can carry.
#[derive(Debug, Clone, Copy)]
pub struct Caps {
    pub images: bool,
    pub cover: bool,
    pub toc: bool,
    pub nested_toc: bool,
    pub links: bool,
    pub series: bool,
}

const ALL: Caps = Caps {
    images: true,
    cover: true,
    toc: true,
    nested_toc: true,
    links: true,
    series: true,
};

/// Capabilities of an output extension (as given to the pipeline).
pub fn caps(ext: &str) -> Caps {
    match ext {
        "txt" | "md" | "markdown" => Caps {
            images: false,
            cover: false,
            toc: false,
            nested_toc: false,
            links: false,
            series: false,
        },
        "txtz" => Caps {
            images: false,
            cover: true,
            toc: false,
            nested_toc: false,
            links: false,
            series: true,
        },
        "pdb" | "rtf" => Caps {
            images: ext == "pdb",
            cover: false,
            toc: ext == "pdb",
            nested_toc: false,
            links: false,
            series: false,
        },
        "pmlz" | "rb" | "snb" => Caps {
            nested_toc: false,
            series: false,
            ..ALL
        },
        "html" | "htm" => Caps {
            images: false,
            cover: false,
            toc: false,
            nested_toc: false,
            links: true,
            series: false,
        },
        "htmlz" => Caps {
            toc: false,
            nested_toc: false,
            ..ALL
        },
        "docx" => Caps {
            cover: false,
            ..ALL
        },
        "fb2" | "mobi" => Caps {
            nested_toc: ext != "fb2",
            ..ALL
        },
        _ => ALL,
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ConversionReport {
    pub output: String,
    /// What the output format cannot hold of what the book has.
    pub lost: Vec<String>,
    /// What the transforms changed (their own notes).
    pub changes: Vec<String>,
    /// Statistics of the book that was written.
    pub words: usize,
    pub images: usize,
    pub toc_entries: usize,
    pub broken_links: usize,
}

impl ConversionReport {
    pub fn to_text(&self) -> String {
        let mut s = format!(
            "Conversion report ({})\n  {} words, {} images, {} contents entries\n",
            self.output, self.words, self.images, self.toc_entries
        );
        if self.lost.is_empty() && self.changes.is_empty() && self.broken_links == 0 {
            s.push_str("  Nothing was lost or changed.\n");
        }
        for l in &self.lost {
            s.push_str(&format!("  lost: {l}\n"));
        }
        if self.broken_links > 0 {
            s.push_str(&format!(
                "  warning: {} internal links point nowhere (already so in the source)\n",
                self.broken_links
            ));
        }
        for c in &self.changes {
            s.push_str(&format!("  changed: {c}\n"));
        }
        s
    }
}

pub fn build_report(
    book: &OpenBook,
    ext: &str,
    reports: &[(String, bookx_core::contracts::TransformReport)],
) -> ConversionReport {
    let d = Digest::of(book);
    let c = caps(ext);
    let mut lost = Vec::new();
    if !c.images && d.images > 0 {
        lost.push(format!("{} images (this format has no pictures)", d.images));
    }
    if !c.cover && d.has_cover {
        lost.push("the cover image".into());
    }
    if !c.toc && !d.toc.is_empty() {
        lost.push(format!("the table of contents ({} entries)", d.toc.len()));
    } else if !c.nested_toc && d.toc.iter().any(|t| t.0 > 0) {
        lost.push("the nesting of the table of contents (it becomes one flat list)".into());
    }
    if !c.links && d.internal_links > 0 {
        lost.push(format!(
            "{} internal links and footnote links",
            d.internal_links
        ));
    }
    if !c.series && !d.series.is_empty() {
        lost.push("the series name and number".into());
    }
    let changes = reports
        .iter()
        .filter(|(_, r)| r.files_changed > 0 || !r.notes.is_empty())
        .flat_map(|(id, r)| {
            let mut v = vec![format!("{id}: {} file(s)", r.files_changed)];
            v.extend(r.notes.iter().map(|n| format!("{id}: {n}")));
            v
        })
        .collect();
    ConversionReport {
        output: ext.to_string(),
        lost,
        changes,
        words: d.words.len(),
        images: d.images,
        toc_entries: d.toc.len(),
        broken_links: d.broken_links,
    }
}
