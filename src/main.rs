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

// No console window next to the app on Windows (bookx-cli is a separate, console program).
#![cfg_attr(windows, windows_subsystem = "windows")]
#![allow(
    clippy::type_complexity,
    clippy::large_enum_variant,
    clippy::too_many_arguments
)]

mod a11y;
mod account_model;
mod aids_prefs;
mod annotation_export;
mod annotation_ids;
mod annotation_import;
mod annotation_sync;
mod annotation_sync_ui;
mod app;
mod applog;
mod audiobook;
mod audiobook_link;
mod authors_view;
mod backups;
mod beta;
mod book_formats;
mod bookmark_sync;
mod bulk_fetch;
mod card_layout;
mod chrome_visibility;
mod comic;
mod config;
mod convert_prefs;
mod cover_color;
mod crate_labels;
mod debug_log;
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
mod flow_window;
mod footer_stats;
mod footnotes;
mod fulltext;
mod gestures;
mod goals;
mod goto;
mod grid_columns;
mod grid_fill;
mod hardcover_sync;
mod highlight_goto;
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
mod launch;
mod libraries;
mod library_add;
mod library_cover;
mod library_custom;
mod library_health;
mod library_nav;
mod library_related;
mod library_tagtree;
mod links;
mod look_sync;
mod lookup_prefs;
mod map_thumb_disk;
mod mcp_host;
mod metadata_fetch;
mod mpris;
mod narration;
mod needs_attention;
mod news;
mod news_recipe_form;
mod news_sites;
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
mod report;
mod rsvp_view;
mod safe_mode;
mod saved_searches;
mod secrets;
mod selection_handles;
mod send_notes;
mod series_view;
mod settings_bus;
mod settings_io;
mod sharing;
mod shelves;
mod shortcuts;
mod side_notes;
mod source_priority;
mod sync_activity;
mod sync_bus;
mod sync_live;
mod sync_model;
mod tags;
mod toc_fold;
mod tracker_sync;
mod tts;
mod tts_export;
mod tts_player;
mod tweaks;
mod ui;
mod vocab_marks;
mod vocabulary;
mod voice_download;
mod web_accounts;
mod web_import;
mod work_groups;
mod works;

/// The engine lives in its own crate; this alias keeps `crate::core::…` paths working.
pub use bookx_core as core;

use relm4::RelmApp;

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_ansi(false)
        .with_writer(debug_log::Tee)
        .init();
    applog::init();
    safe_mode::init();

    // File arguments are handled by the app itself, not by GApplication.
    let app = RelmApp::new(config::APP_ID).with_args(vec![]);
    forward_links_to_running_instance();
    icons::install();
    relm4::set_global_css(include_str!("style.css"));
    relm4::gtk::Window::set_default_icon_name(config::APP_ID);
    app.run::<app::App>(());
}

/// A second `bookx` while Bookx runs (`bookx bookx://open?id=3`, Open With, `bookx --doc book.epub`): hands what
/// it was asked to the running process through application actions and leaves.
fn forward_links_to_running_instance() {
    use relm4::gtk::prelude::*;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let req = launch::parse(&args);
    if !req.is_forwardable() {
        return;
    }
    let app = relm4::main_adw_application();
    if app.register(relm4::gtk::gio::Cancellable::NONE).is_ok() && app.is_remote() {
        for l in &req.links {
            app.activate_action("open-uri", Some(&l.to_variant()));
        }
        if !req.files.is_empty() {
            let files: Vec<String> = req.files.iter().map(|p| p.display().to_string()).collect();
            app.activate_action("open-files", Some(&files.to_variant()));
        }
        for d in &req.docs {
            app.activate_action("open-doc", Some(&d.display().to_string().to_variant()));
        }
        // let the message go out before this process ends
        app.activate();
        std::process::exit(0);
    }
}
