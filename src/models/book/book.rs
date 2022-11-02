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

use crate::models::book::{BookAnnotation, Bookmark};
use crate::models::storage::Storage;
use crate::models::utils;
use crate::models::utils::Format;
use crate::settings::{settings_manager, Key};
use crate::BookxApplication;
use adw::gdk::{self, gdk_pixbuf};
use adw::glib::object::GObject;
use adw::glib::List;
use gio::ListStore;
use gtk::gdk_pixbuf::Pixbuf;
use gtk::glib::{
    self, clone, subclass::Signal, DateTime, Enum, ObjectExt, ParamFlags, ParamSpec,
    ParamSpecBoxed, ParamSpecEnum, ParamSpecInt, ParamSpecObject, ParamSpecString, Sender,
    SignalFlags, ToValue,
};
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use jsondata::Json;
use log::{debug, error};
use once_cell::sync::Lazy;
use once_cell::unsync::OnceCell;
use serde::de::Unexpected::Str;
use serde::{Deserialize, Serialize};
use std::borrow::Borrow;
use std::cell::{Cell, RefCell};
use std::collections::{hash_map::Values, HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::str::FromStr;

// must be the same as `CHARACTERS_PER_PAGE` in web/epub-viewer.js
// in 1.x this was 1600, so this was needed to automatically clear the cache
const CHARACTERS_PER_PAGE: i32 = 1024;

// this should be bumped whenever FB2 rendering (see web/webpub.js) is changed
// that way we can clear the cache
const FB2_CONVERTER_VERSION: String = String::from("2.4.0");

// TODO: switch to serde_json
struct BookData {
    pub metadata: jsondata::Property,
    pub has_annotations: bool,
    pub progress: jsondata::Property,
}

mod imp {
    use super::*;

    #[derive(Default, Debug, Eq, PartialEq, Clone, Serialize, Deserialize)]
    pub struct Book {
        pub identifier: RefCell<String>,
        pub format: RefCell<Format>,
        pub uri: RefCell<String>,
        pub metadata: jsondata::Property,
        pub has_annotations: bool,
        pub cover_picture_path: RefCell<String>,
        // pub view_set: Cell<HashSet<String>>,
        pub book_data: RefCell<BookData>,
        pub storage: RefCell<Storage>,
        pub cache: RefCell<Storage>,
        pub annotations_map: Cell<HashMap<String, BookAnnotation>>,
        pub annotations_list: gio::ListStore,
        pub bookmarks_set: Cell<HashSet<String>>,
        pub bookmarks_list: gio::ListStore,
    }

    // checking if this works
    // https://raw.githubusercontent.com/gtk-rs/examples/master/src/bin/listbox_model.rs
    #[glib::object_subclass]
    impl ObjectSubclass for Book {
        const NAME: &'static str = "Book";
        type ParentType = glib::Object;
        type Class = glib::Class<Self>;

        // glib_object_subclass!();
    }

    impl ObjectImpl for Book {
        fn properties() -> &'static [ParamSpec] {
            static PROPERTIES: Lazy<Vec<ParamSpec>> = Lazy::new(|| {
                vec![ParamSpecObject::new(
                    "annotations_list",
                    "AnnotationsList",
                    "AnnotationsList",
                    glib::List::default(),
                    ParamFlags::READWRITE,
                )]
            });

            PROPERTIES.as_ref()
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &ParamSpec) -> glib::Value {
            match pspec.name() {
                "annotations_list" => self.annotations_list.to_value(),
                _ => unimplemented!(),
            }
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: Lazy<Vec<Signal>> = Lazy::new(|| {
                vec![
                    Signal::builder(
                        "annotation-added",
                        // Types of the values which will be sent to the signal handler
                        // TODO
                        &[BookAnnotation::static_type().into()],
                        // Type of the value the signal handler sends back
                        <()>::static_type().into(),
                    )
                    .flags(SignalFlags::RUN_FIRST)
                    .build(),
                    Signal::builder(
                        "annotation-removed",
                        &[glib::GString::static_type().into()],
                        <()>::static_type().into(),
                    )
                    .flags(SignalFlags::RUN_FIRST)
                    .build(),
                    Signal::builder("externally-modified", &[None], <()>::static_type().into())
                        .flags(SignalFlags::RUN_FIRST)
                        .build(),
                    Signal::builder("cache-modified", &[None], <()>::static_type().into())
                        .flags(SignalFlags::RUN_FIRST)
                        .build(),
                ]
            });
            SIGNALS.as_ref()
        }
    }
}

glib::wrapper! {
    pub struct Book(ObjectSubclass<imp::Book>);
}

impl Book {
    pub fn new(identifier: String, format: Format, uri: String) -> Self {
        // If this doesn't work then look how BookxApplication GObject is created
        // Line 188, application.rs
        let book = glib::Object::new::<Self>(&[]).unwrap();
        *book.imp().identifier = identifier.clone();
        *book.imp().format = format;
        *book.imp().uri = uri;
        *book.imp().storage = Storage::new(Self::get_data_path(identifier.borrow()));
        *book.imp().cache = Storage::new(Self::get_cache_path(identifier.borrow()));
        *book.imp().cover_picture_path = Self::get_cover_path(identifier.borrow());

        // TODO: only call when entering reader mode
        book.load_data();

        // TODO: save progress and has_annotations as property
        // TODO: get metadata when calling win.show_book_details() | memory conservation

        // connect storage and cache with signals
        book.imp().storage.borrow().connect("modified", false, {
            BookxApplication::default().library().update(identifier, {
                // TODO: WTF
                // identifier,
                // metadata: this._storage.get('metadata', {}),
                // hasAnnotations: this._annotationsMap.size > 0,
                // progress: this._storage.get('progress', []),
                // modified: new Date()
            })
        });

        book.imp()
            .storage
            .borrow()
            .connect("externally-modified", false, {
                book.load_data();
                book.emit_by_name("externally-modified");
            });
        book.imp()
            .cache
            .borrow()
            .connect("externally-modified", false, {
                book.emit_by_name("cache-modified");
            });
        book
    }

    fn load_data(&self) {
        self.imp().annotations_map.get().clear();
        self.imp().annotations_list.remove_all();
        self.imp().bookmarks_set.get().clear();
        self.imp().bookmarks_list.remove_all();

        // self.imp().storage.get('annotations', [])
        //     .sort((a, b) => EpubCFI.compare(a.value, b.value))
        //     .forEach(({ value, color, text, note }) =>
        //         this.addAnnotation(new EpubViewAnnotation({
        //             cfi: value,
        //             color: color || 'yellow',
        //             text: text || '',
        //             note: note || ''
        //         }), true))
        //
        // self.imp()._storage.get('bookmarks', [])
        //     .forEach(cfi => this.addBookmark(cfi, true))
    }

    fn annotations(&self) -> Values<'_, String, String> {
        self.imp().annotations_map.values()
    }

    // returns true if book has annotations
    pub fn has_annotations(&self) -> bool {
        let annotations_array = self
            .imp()
            .storage
            .borrow()
            .get("annotations")
            .unwrap()
            .to_array();
        !annotations_array.unwrap().is_empty()
    }

    // returns annotation corresponding to a CFI
    pub fn get_annotations(&self, cfi: &str) -> Option<&BookAnnotation> {
        self.imp().annotations_map.get().get(cfi)
    }

    fn annotations_list(&self) -> &ListStore {
        &self.imp().annotations_list
    }

    fn bookmark_list(&self) -> &ListStore {
        &self.imp().bookmarks_list
    }

    // return true if book has a bookmark corresponding to CFI
    pub fn has_bookmark(&self, cfi: &str) -> bool {
        self.imp().bookmarks_set.get().contains(cfi)
    }

    // Get Last location: f64 otherwise return 0
    // TODO: return a EpubCFI of beginning or propagate err
    pub fn get_last_location(&self) -> String {
        match self.imp().storage.borrow().get("last_location") {
            Some(location) => location.to_string(),
            None => String::from(""),
        }
    }

    pub fn set_last_location(&self, location: f64) {
        self.imp()
            .storage
            .borrow()
            .set("last_location", Json::new::<f64>(location))
    }

    pub fn set_progress(&self, current: i128, total: i128) {
        let mut percentage = Json::new::<i128>((current / total) * 100);
        // if let Err(E) = js.append("", Json::new(current)) {
        //     error!(format!(
        //         "Cannot set 'current' value: {} when trying to `set_progress()`",
        //         current
        //     ))
        // };
        // if let Err(E) = js.append("", Json::new(total)) {
        //     error!(format!(
        //         "Cannot set 'total' value: {} when trying to `set_progress()`",
        //         total
        //     ))
        // };
        self.imp().storage.borrow().set("progress", percentage);
    }

    pub fn get_progress(self) -> String {
        let progress = self.imp().storage.borrow().get("progress");
        match progress {
            Ok(progress) => {
                let fraction = progress.to_integer().unwrap_or_else(|error| {
                    error!(
                        "Cannot parse progress for book: {:?}",
                        self.imp().identifier
                    );
                    return 0;
                });
                format!("{}%", fraction)
            }
            Err(_) => {
                error!(
                    "Cannot find progress property for book: {:?}",
                    self.imp().identifier
                )
            }
        }
    }

    pub fn set_metadata(&self, metadata: Json) {
        self.imp().storage.borrow().set("metadata", metadata)
    }

    // pub fn get_locations(&self) -> Option<List<T>> {
    //     if self.format == Format::FB2 {
    //         let convert_version = self.cache.get("converter_version");
    //         return if convert_version == FB2_CONVERTER_VERSION {
    //             Some(self.cache.get("locations"))
    //         } else {
    //             None
    //         };
    //     }
    //
    //     let locations_chars = self.cache.get("locations_chars");
    //     return if locations_chars == CHARACTERS_PER_PAGE {
    //         Some(self.cache.get("locations"))
    //     } else {
    //         None
    //     };
    // }
    //
    // pub fn set_locations(&self, locations: ) {
    //     if *self.imp().format == Format::FB2 {
    //         self.imp()
    //             .cache
    //             .borrow_mut()
    //             .set("convert_version", FB2_CONVERTER_VERSION);
    //     }
    //     self.imp().cache.borrow_mut().set(
    //         "locations_chars",
    //         Json::new::<f64>(CHARACTERS_PER_PAGE as f64),
    //     );
    //     self.imp().cache.borrow_mut().set("locations", locations);
    // }

    // pub fn add_annotation(&self, annotation: BookAnnotation, init: bool) {
    //     let cfi = annotation.imp().cfi.take();
    //     if self.imp().annotations_map.contains_key(cfi.borrow()) {
    //         self.emit_by_name(
    //             "annotation-added",
    //             &[-&self.imp().annotations_map.get().get(cfi.borrow())],
    //         )
    //     } else {
    //         self.imp()
    //             .annotations_map
    //             .get()
    //             .insert(cfi.clone(), annotation.clone());
    //
    //         if init {
    //             self.imp().annotations_list.append(annotation.clone());
    //         } else {
    //             self.imp()
    //                 .annotations_list
    //                 .insert_sorted(annotation.clone(), EpubCFI::compare);
    //         }
    //     }
    // }

    // pub fn remove_annotation(&self, annotation: &BookAnnotation) {
    //     let cfi = annotation.imp().cfi.clone().take();
    //     self.emit_by_name("annotation-removed", &cfi);
    //     self.imp().annotations_map.get().remove(&cfi);
    //     let store = &self.imp().annotations_list;
    //     match store.find(annotation) {
    //         Some(position) => store.remove(position),
    //         None => (),
    //     };
    //     self.on_annotations_changed();
    // }

    // fn on_annotations_changed(&self) {
    //     // FIXME: can we fix with Rust iterators?
    //     let mut annotations = Json::new::<Vec<String>>(Vec::new());
    //     for (_, annotations) in self.imp().annotations_list {
    //         annotations.append("", Json::new(annotation.cfi.to_string()))
    //     }
    //     self.imp()
    //         .storage
    //         .borrow_mut()
    //         .set("annotations", annotations);
    // }

    // pub fn add_bookmark(&self, cfi: String, init: bool) {
    //     self.imp().bookmarks_set.get().insert(&cfi);
    //     self.imp().bookmarks_list.append(Bookmark::new(&cfi));
    //     if !init {
    //         self.on_bookmarks_changed();
    //     }
    // }
    //
    // pub fn remove_bookmark(&self, cfi: String) {
    //     self.imp().bookmarks_set.get().remove(cfi);
    //     let store = &self.imp().bookmarks_list;
    //     match store.find(cfi) {
    //         Some(position) => store.remove(position),
    //         None => (),
    //     };
    //     self.on_bookmarks_changed();
    // }

    fn on_bookmarks_changed(&self) {
        // TODO: can we fix with Rust iterators?
        // let bookmarks: Vec<String> = self.imp().bookmarks_set.into_iter().collect();
        // let bookmarks = Vec::from_iter(self.imp().bookmarks_set.iter()));
        let mut bookmarks = Json::new::<Vec<String>>(Vec::new());
        for (_, bookmark) in self.imp().bookmarks_set.into_iter() {
            bookmarks.append("", Json::new(bookmark.cfi.to_string()))
        }
        self.imp().storage.borrow_mut().set("bookmarks", bookmarks);
    }

    fn clear_cache(&self) {
        self.cache.clear();
        match gio::File::for_path(*self.imp().cover_picture_path).delete(gio::Cancellable::NONE) {
            Ok(_) => (),
            Err(_) => error!("Cannot delete cache for Book"),
        }
    }

    fn disconnect_all(&self) {
        for annotation in self.imp().annotations {
            // disconnect everyone
        }
    }

    // fn disconnect_all_handles(&self, object: glib::Object, signal: glib::) {
    // TODO: where is `GObject.signal_parse_name() ?
    // }

    // TODO: uncomment when `view_set` is required
    // fn add_view(&self, view) {
    //     self.imp().view_set.get().insert(view);
    // }
    //
    // fn delete_view(&self, view) {
    //     self.imp().view_set.get().remove(view);
    //     if self.imp().view_set.get().len() == 0 {
    //         // TODO: this needs to be handled by library
    //         // dataMap.delete(this._identifier)
    //     }
    // }

    pub fn get_data(&self) -> Json {
        *self.imp().storage.borrow().imp().data
    }

    pub fn save_cover(&self, pixbuf: Pixbuf) {
        if !settings_manager::boolean(Key::CacheCovers) {
            return;
        }
        debug!("Saving cover to {:?}", &self.imp().cover_picture_path);
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

    pub fn get_data_path(identifier: &str) -> String {
        Storage::get_path("data", identifier, None)
    }

    pub fn get_cache_path(identifier: &str) -> String {
        Storage::get_path("cache", identifier, None)
    }

    pub fn get_cover_path(identifier: &str) -> String {
        Storage::get_path("cache", identifier, Some(String::from(".png")))
    }
}