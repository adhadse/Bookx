//! CSS subset (R1, R21, R22): parse author, user and built-in stylesheets with `cssparser`, match selectors on
//! `scraper` elements, cascade and compute a [`ComputedStyle`] for the supported property set.
//!
//! ```
//! use bookx_text::css::StyleSheetSet;
//! let html = scraper::Html::parse_document("<p class='x'>hi</p>");
//! let sheets = StyleSheetSet::new().add_author("p.x { text-indent: 2em }").add_user("p { text-indent: 0 !important }");
//! let p = html.select(&scraper::Selector::parse("p").unwrap()).next().unwrap();
//! assert_eq!(sheets.compute(p).text_indent, 0.0);
//! ```

mod cascade;
mod selector;
mod sheet;
mod values;

pub use cascade::{ComputedStyle, StyleSheetSet, UA_CSS};
pub use selector::{Selector, parse_selector_list};
pub use sheet::{MediaEnv, Rule, media_applies, parse_declaration_text, parse_stylesheet};
pub use values::{
    Color, Decl, Display, Edges, Float, FontStyle, Hyphens, Len, LineHeight, LineHeightSpec,
    PageBreak, Side, Size, TextAlign, TextCombine, TextDecoration, TextOrientation, Val,
    WeightSpec, WritingMode,
};
