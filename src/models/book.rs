// Bookx - book.rs
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

use crate::{
    config,
    models::format::{self, Format, FormatNotSupportedError},
    settings::{settings_manager, Key},
};
use adw::gdk::{self, gdk_pixbuf};
use epub::doc::{DocError, EpubDoc};
use gtk::glib::{
    self, clone, subclass::Signal, DateTime, Enum, ObjectExt, ParamFlags, ParamSpec,
    ParamSpecBoxed, ParamSpecEnum, ParamSpecInt, ParamSpecObject, ParamSpecString, Sender,
    SignalFlags, ToValue,
};
use gtk::{gdk_pixbuf::Pixbuf, gio::ListStore, prelude::*, subclass::prelude::*};
use log::{debug, error};
use once_cell::{sync::Lazy, unsync::OnceCell};
use serde::{self, Deserialize, Serialize};
use std::{
    cell::{Cell, RefCell},
    collections::{hash_map::Values, HashMap, HashSet},
    path::PathBuf,
};

// must be the same as `CHARACTERS_PER_PAGE` in web/epub-viewer.js
// in 1.x this was 1600, so this was needed to automatically clear the cache
const CHARACTERS_PER_PAGE: i32 = 1024;

// this should be bumped whenever FB2 rendering (see web/webpub.js) is changed
// that way we can clear the cache
const FB2_CONVERTER_VERSION: String = String::from("2.4.0");

#[derive(Default, Debug, Eq, PartialEq, Clone, Serialize, Deserialize)]
pub struct BookxBook {
    pub identifier: String,
    pub title: String,
    pub format: Format,
    pub path: PathBuf,
    pub author: String,
    pub publisher: String,
    pub language: String,
    pub has_annotations: RefCell<bool>,
    pub progress: RefCell<i32>,
    // #[serde(skip_serialializing)]
    // annotations_map: Cell<HashMap<String, BookAnnotation>>,
    // #[serde(skip_serializing)]
    // annotations_list: ListStore,
    // #[serde(skip_serializing)]
    // bookmarks_set: Cell<HashSet<String>>,
    // #[serde(skip_serializing)]
    // bookmarks_list: ListStore,
}

pub enum BookxBookError {
    FormatNotSupportError,
    DocError(DocError),
}

impl BookxBook {
    pub fn load(path: PathBuf) -> Result<BookxBook, BookxBookError> {
        // load a book if found in database otherwise load from it's epub and store the data in
        // database
        // Do we need a db? or store book_data and BookAnnotations and BookMarks in separate JSON
        // files for each Book?
        let mime_type = gio::content_type_get_mime_type(path.to_str().unwrap());
        let format = mime_type.unwrap().as_str().parse::<Format>()?;

        let doc = EpubDoc::new(path.to_str().clone())?;
        let title = doc.metadata.get("title").unwrap().get(0).unwrap().clone();
        let identifier = match doc.metadata.get("identifier") {
            Ok(identifier_list) => identifier_list.get(0).unwrap().clone(),
            Err(e) => title.clone(),
        };
        let author = doc.metadata.get("creator").unwrap().join(", ");
        let publisher = doc
            .metadata
            .get("publisher")
            .unwrap()
            .get(0)
            .unwrap()
            .clone();
        let language = doc
            .metadata
            .get("language")
            .unwrap()
            .get(0)
            .unwrap()
            .to_uppercase();
        let has_annotations = RefCell::new(false);
        let progress = RefCell::new(0);

        let bookx_book = Self {
            identifier,
            title,
            format,
            path,
            author,
            publisher,
            language,
            has_annotations,
            progress,
        };
        Ok(bookx_book)
    }

    // TODO: only call when entering reader mode
    fn load_data(&self) {
        self.annotations_map.get().clear();
        self.annotations_list.remove_all();
        self.bookmarks_set.get().clear();
        self.bookmarks_list.remove_all();
        unimplemented!()
    }

    fn annotations(&self) -> Values<'_, String, String> {
        self.imp().annotations_map.values()
    }

    // returns true if book has annotations
    pub fn has_annotations(&self) -> bool {
        unimplemented!()
    }

    fn annotations_list(&self) -> &ListStore {
        &self.annotations_list
    }

    fn bookmark_list(&self) -> &ListStore {
        &self.bookmarks_list
    }

    pub fn set_progress(&self, current: i32, total: i32) {
        let mut percentage = (current / total) * 100;
        *self.progress = f64::from(percentage);
    }

    pub fn set_metadata(&self) {
        unimplemented!()
    }

    pub fn save_cover(&self, pixbuf: Pixbuf) {
        if !settings_manager::boolean(Key::CacheCovers) {
            return;
        }
        debug!("Saving cover to {:?}", &self.get_cover_path());
        let width = settings_manager::integer(Key::CoverPictureSize);

        let ratio = width / pixbuf.width();
        if ratio == 1 || ratio > 1 {
            pixbuf
                .savev(*self.imp().cover_picture_path, "png", &[])
                .expect("Cannot save cover picture");
        } else {
            let height = pixbuf.height() * ratio;
            match pixbuf.scale_simple(width, height, gdk_pixbuf::InterpType::Bilinear) {
                Some(pixbuf) => {
                    pixbuf
                        .savev(*self.imp().cover_picture_path, "png", &[])
                        .expect("Cannot save cover picture");
                }
                None => error!("Cannot scale the Pixbuf for cover picture"),
            };
        }
    }

    // pub fn get_path(_type: &str, key: &str, extension: String) -> String {
    //     let mut data_dir: PathBuf = PathBuf::new();
    //     if _type == "cache" {
    //         data_dir = glib::user_cache_dir()
    //     } else if _type == "config" {
    //         data_dir = glib::user_config_dir()
    //     } else {
    //         data_dir = glib::user_data_dir()
    //     }
    //     data_dir
    //         .join(config::PKGNAME)
    //         .join(key)
    //         .with_extension(extension.unwrap_or(String::from(extension)))
    //         .to_str()
    //         .to_owned()
    //         .unwrap()
    //         .to_string()
    // }

    pub fn get_data_path(identifier: &String) -> String {
        unimplemented!()
    }

    pub fn get_cover_picture_path(identifier: &str) -> String {
        BookxBook::get_path("cache", identifier, String::from(".png"))
    }
}
