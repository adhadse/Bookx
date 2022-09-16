// Bookx - utils.rs
// Copyright (C) 2022  Anurag Dhadse <hello@adhadse.com>
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

use core::cmp::Ordering;
use std::path::PathBuf;

use enum_map::{enum_map, Enum, EnumArray, EnumMap};
use gtk::{gdk, gio, glib, prelude::*};
use log::{debug, warn};

#[derive(Debug, Enum)]
pub enum EBook {
    Epub,
}

pub fn get_ebook_mime(ebook: EBook) -> String {
    match ebook {
        EBook::Epub => String::from("application/epub+zip"),
    }
}

pub fn load_files_from_folder(folder: &gio::File, recursive: bool) -> Vec<gio::File> {
    use std::time::Instant;

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
            "standard::name,standard::type",
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
            files.push(child.clone());
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
