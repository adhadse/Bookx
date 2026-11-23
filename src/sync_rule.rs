//! The design rule of every sync (issue #39): **sync moves reading position, highlights, notes,
//! bookmarks and similar metadata, never book files, and never deletes a library book, its files
//! or what was made in Bookx because something disappeared on a device** (a book taken off the
//! reader, a wiped device, a deleted KOReader notes folder). Books reach a device only through
//! *Send to Device* and reach the library only through *Add Books*.
//!
//! The behaviour is tested next to each path (`annotation_sync`, `device_sync`, `bookx-serve`'s
//! `tests/sync_rule.rs`). This test is the net under them: the code of the sync paths contains no
//! call that removes a book, trashes a file or deletes a path (outside its tests). A new call in
//! one of them fails here and has to be argued with the owner, not silenced.

#[cfg(test)]
mod tests {
    /// The production part of a source file (everything before its first `#[cfg(test)]`).
    fn production(src: &str) -> &str {
        src.split("#[cfg(test)]").next().unwrap_or(src)
    }

    /// Words that remove a book or a file. `remove_book(s)` of a device is the explicit
    /// *Remove from device* and lives in `devices.rs`, not in a sync path.
    const FORBIDDEN: &[&str] = &[
        "remove_file(",
        "remove_dir_all(",
        "remove_dir(",
        ".remove(&ids",
        ".remove(&[",
        "delete_book(",
        "remove_book(",
        "remove_books(",
        "empty_trash(",
        "books.retain(",
        "books.remove(",
    ];

    #[test]
    fn the_sync_paths_never_remove_books_or_files() {
        let paths: &[(&str, &str)] = &[
            ("annotation_sync.rs", include_str!("annotation_sync.rs")),
            ("bookmark_sync.rs", include_str!("bookmark_sync.rs")),
            ("device_sync.rs", include_str!("device_sync.rs")),
            ("kosync_sync.rs", include_str!("kosync_sync.rs")),
            ("sync_live.rs", include_str!("sync_live.rs")),
            ("look_sync.rs", include_str!("look_sync.rs")),
            ("sync_activity.rs", include_str!("sync_activity.rs")),
        ];
        for (name, src) in paths {
            let code = production(src);
            for bad in FORBIDDEN {
                assert!(
                    !code.contains(bad),
                    "{name} calls `{bad}`: a sync must never remove a book or a file (docs/sync-rules.md)"
                );
            }
        }
    }

    #[test]
    fn the_library_removes_books_only_where_the_user_asks() {
        // the one place that calls Library::remove in the app is the library's own Delete action
        let ui_sync = [
            include_str!("annotation_sync_ui.rs"),
            include_str!("ui/sync_window.rs"),
            include_str!("ui/sync_activity.rs"),
            include_str!("ui/sync_prefs.rs"),
            include_str!("ui/kosync.rs"),
        ];
        for src in ui_sync {
            let code = production(src);
            for bad in [".remove(&ids", "remove_file(", "delete_book("] {
                assert!(!code.contains(bad), "`{bad}` in a sync window");
            }
        }
    }
}
