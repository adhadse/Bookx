// Bookx — a modern e-book library, reader and editor written in Rust.
// Copyright (C) 2023-2026  Anurag Dhadse <hello@adhadse.com>
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

#![allow(
    clippy::type_complexity,
    clippy::large_enum_variant,
    clippy::too_many_arguments
)]

mod a11y;
mod annotation_export;
mod annotation_ids;
mod annotation_import;
mod annotation_sync;
mod annotation_sync_ui;
mod app;
mod audiobook;
mod audiobook_link;
mod authors_view;
mod backups;
mod book_formats;
mod bulk_fetch;
mod card_layout;
mod chrome_visibility;
mod comic;
mod config;
mod cover_color;
mod crate_labels;
mod desktop_integration;
mod device_covers;
mod device_sync;
mod devices;
mod devtools;
mod diagnostics;
mod dict_auto;
mod dict_catalog;
mod dict_install;
mod dictionaries;
mod duplicates;
mod editor_chars;
mod editor_complete;
mod editor_keymap;
mod editor_marks;
mod editor_prefs;
mod editor_search;
mod editor_search_fn;
mod editor_search_fuzzy;
mod editor_search_modes;
mod editor_search_saved;
mod editor_snippets;
mod editor_spell;
mod editor_spell_dicts;
mod editor_state;
mod editor_structure;
mod editor_tools;
mod eink;
mod emailing;
mod export;
mod footer_stats;
mod footnotes;
mod fulltext;
mod gestures;
mod goals;
mod goto;
mod grid_columns;
mod grid_fill;
mod highlight_repair;
mod i18n;
mod icons;
mod image_view;
mod import;
mod jobs;
mod jump_history;
mod kobo_real_check;
mod kosync_queue;
mod kosync_sync;
mod libraries;
mod library_health;
mod library_nav;
mod links;
mod look_sync;
mod map_thumb_disk;
mod mcp_host;
mod metadata_fetch;
mod mpris;
mod narration;
mod needs_attention;
mod news;
mod notes_view;
mod opds_browse;
mod overlay_play;
mod page_list;
mod palette;
mod pdf;
mod plugins;
mod preflight;
mod privacy;
mod reader_cards;
mod reader_themes;
mod reading_dwell;
mod reading_profiles;
mod reading_ruler;
mod reading_stats;
mod readwise_sync;
mod release_consistency;
mod remove_undo;
mod rsvp_view;
mod saved_searches;
mod secrets;
mod selection_handles;
mod send_notes;
mod series_view;
mod settings_bus;
mod sharing;
mod shelves;
mod shortcuts;
mod side_notes;
mod sync_model;
mod tags;
mod tracker_sync;
mod tts;
mod tts_export;
mod tts_player;
mod tweaks;
mod ui;
mod vocab_marks;
mod vocabulary;
mod voice_download;
mod web_import;
mod work_groups;
mod works;

/// The engine lives in its own crate; this alias keeps `crate::core::…` paths working.
pub use bookx_core as core;

use relm4::RelmApp;

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    // File arguments are handled by the app itself, not by GApplication.
    let app = RelmApp::new(config::APP_ID).with_args(vec![]);
    forward_links_to_running_instance();
    icons::install();
    relm4::set_global_css(include_str!("style.css"));
    relm4::gtk::Window::set_default_icon_name(config::APP_ID);
    app.run::<app::App>(());
}

/// `bookx bookx://open?id=3` while Bookx runs: hand the link to that process and leave.
fn forward_links_to_running_instance() {
    use relm4::gtk::prelude::*;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let links = links::from_args(&args);
    if links.is_empty() {
        return;
    }
    let app = relm4::main_adw_application();
    if app.register(relm4::gtk::gio::Cancellable::NONE).is_ok() && app.is_remote() {
        for l in links {
            app.activate_action("open-uri", Some(&l.to_variant()));
        }
        // let the message go out before this process ends
        app.activate();
        std::process::exit(0);
    }
}
