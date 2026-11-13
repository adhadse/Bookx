#![forbid(unsafe_code)]
//! GUI-free logic behind the Bookx e-book editor (roadmap track T7).

pub mod accessibility;
pub mod book_tools;
pub mod charinfo;
pub mod checkpoints;
pub mod compare;
pub mod files;
pub mod filter_styles;
pub mod fixes;
pub mod fontlayout;
pub mod fonts;
pub mod images;
pub mod inline_toc;
pub mod links;
pub mod live_css;
pub mod markup;
pub mod opf_edit;
pub mod preview_map;
pub mod problems;
pub mod reports;
pub mod reports_tables;
pub mod scroll_sync;
pub mod search_ignore;
pub mod semantics;
pub mod spell;
pub mod spell_import;
pub mod spell_session;
pub mod spell_user;
pub mod split_merge;
pub mod tag_search;
pub mod tags;
pub mod toc_gen;
pub mod transform_css;
pub mod upgrade;
pub mod validate;

#[doc(hidden)]
pub mod testepub;
#[doc(hidden)]
pub mod testfont;

#[cfg(test)]
mod testutil;
