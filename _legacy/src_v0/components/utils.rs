use core::cmp::Ordering;
use relm4::gtk::{
    gio::{self, prelude::*},
    glib,
};
use std::time::Instant;
use tracing::{debug, info};

pub fn load_files_from_folder(folder: &gio::File, recursive: bool) -> Vec<gio::File> {
    info!("Starting to lad books from folder: {:?}", folder.path());
    let now = Instant::now();

    let res = load_files_from_folder_internal(folder, folder, recursive);
    debug!(
        "Folder enumeration: {} us (recursive: {}), total files: {}",
        now.elapsed().as_micros(),
        recursive,
        res.len()
    );

    res
}

fn load_files_from_folder_internal(
    base: &gio::File,
    folder: &gio::File,
    recursive: bool,
) -> Vec<gio::File> {
    let mut enumerator = folder
        .enumerate_children(
            "standard::name,standard::type,standard::content-type",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            None::<&gio::Cancellable>,
        )
        .expect("Unable to enumerate");

    let mut files = Vec::new();
    while let Some(info) = enumerator.next().and_then(|s| s.ok()) {
        let child = enumerator.child(&info);
        if recursive && info.file_type() == gio::FileType::Directory {
            let mut res = load_files_from_folder_internal(&base, &child, recursive);
            files.append(&mut res);
        } else if info.file_type() == gio::FileType::Regular {
            if let Some(content_type) = info.content_type().map(|t| t.to_string()) {
                match content_type.as_str() {
                    "application/epub+zip" => {
                        // currently only epub is supported
                        files.push(child.clone());
                    }
                    &_ => {
                        info!("File is not supported {:?}", child.path());
                    }
                }
            };
        }
    }

    // gio::FileEnumerator has no guaranteed order, so we should
    // rely on the basename being formatted in a way that gives us an
    // implicit order; if anything, this will queue books in the same
    // order in which they appear in the directory when browsing its
    // contents
    files.sort_by(|a, b| cmp_two_files(Some(base), a, b));

    files
}

pub fn cmp_two_files(base: Option<&gio::File>, a: &gio::File, b: &gio::File) -> Ordering {
    let parent_a = a.parent().unwrap();
    let parent_b = b.parent().unwrap();
    let parent_basename_a = if let Some(base) = base {
        if let Some(path) = base.relative_path(&parent_a) {
            path
        } else {
            parent_a.basename().unwrap()
        }
    } else {
        parent_a.basename().unwrap()
    };
    let parent_basename_b = if let Some(base) = base {
        if let Some(path) = base.relative_path(&parent_b) {
            path
        } else {
            parent_b.basename().unwrap()
        }
    } else {
        parent_b.basename().unwrap()
    };
    let basename_a = a.basename().unwrap();
    let basename_b = b.basename().unwrap();

    let mut order = cmp_like_nautilus(
        &parent_basename_a.to_string_lossy(),
        &parent_basename_b.to_string_lossy(),
    );

    if order.is_eq() {
        order = cmp_like_nautilus(&basename_a.to_string_lossy(), &basename_b.to_string_lossy());
    }

    order
}

fn cmp_like_nautilus(filename_a: &str, filename_b: &str) -> Ordering {
    let order;

    let sort_last_a = filename_a.as_bytes()[0] == b'.' || filename_a.as_bytes()[0] == b'#';
    let sort_last_b = filename_b.as_bytes()[0] == b'.' || filename_b.as_bytes()[0] == b'#';

    if !sort_last_a && sort_last_b {
        order = Ordering::Less;
    } else if sort_last_a && !sort_last_b {
        order = Ordering::Greater;
    } else {
        let key_a = glib::FilenameCollationKey::from(filename_a);
        let key_b = glib::FilenameCollationKey::from(filename_b);
        order = key_a.partial_cmp(&key_b).unwrap();
    }

    order
}
