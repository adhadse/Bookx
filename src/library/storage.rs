// Bookx - storage.rs
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

use crate::config;
use crate::glib::{Error, GString};
use adw::glib::subclass::SignalId;
use gio::{File, FileInfo};
use gtk::glib::{
    clone, subclass::Signal, ObjectExt, ParamFlags, ParamSpec, ParamSpecEnum, ParamSpecInt,
    ParamSpecObject, ParamSpecString, Sender, SignalFlags, ToValue,
};
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use jsondata::Json;
use log::{debug, error};
use once_cell::sync::Lazy;
use once_cell::unsync::OnceCell;
use std::borrow::{Borrow, BorrowMut};
use std::cell::{Cell, RefCell};
use std::fs;
use std::io::{ErrorKind, Write};
use std::iter::Once;
use std::path;
use futures::TryFutureExt;
use url::form_urlencoded;

mod imp {
    use super::*;

    #[derive(Default, Debug, Clone)]
    pub struct Storage {
        pub file: RefCell<File>,
        pub monitor: RefCell<gio::FileMonitor>,
        pub data: RefCell<Json>,
        pub modified: RefCell<u64>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Storage {
        const NAME: &'static str = "Storage";
        type ParentType = glib::Object;
        type Class = glib::Class<Self>;

        // glib_object_subclass!();
    }

    impl ObjectImpl for Storage {
        fn properties() -> &'static [ParamSpec] {
            static PROPERTIES: Lazy<Vec<ParamSpec>> = Lazy::new(|| {
                vec![ParamSpecObject::new(
                    "file",
                    "File",
                    "File",
                    File::static_type(),
                    ParamFlags::READWRITE,
                )]
            });
            PROPERTIES.as_ref()
        }

        fn signals() -> &'static [Signal] {
            static SIGNAL: Lazy<Vec<Signal>> = Lazy::new(|| {
                vec![
                    Signal::builder("modified", &[None], <()>::static_type().into())
                        .flags(SignalFlags::RUN_FIRST)
                        .build(),
                    Signal::builder("externally-modified", &[None], <()>::static_type().into())
                        .flags(SignalFlags::RUN_FIRST)
                        .build(),
                ]
            });
            SIGNAL.as_ref()
        }

        fn property(&self, _obj: &Self::Type, _id: usize, pspec: &ParamSpec) -> glib::Value {
            match pspec.name() {
                "file" => self.file.to_value(),
                _ => unimplemented!(),
            }
        }
    }
}

glib::wrapper! {
    pub struct Storage(ObjectSubclass<imp::Storage>)
    @extends glib::Object;
}

impl Storage {
    pub fn new(path: String) -> Self {
        let storage = glib::Object::new::<Self>(&[]).unwrap();
        storage.imp().file.into_inner() = File::for_path(path.clone());
        storage.imp().data.into_inner() = storage.read(path.clone());
        storage.imp().monitor.into_innner() = storage
            .imp()
            .file
            .get()
            .monitor(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
            .expect("Cannot create monitor")
            .connect("changed", false, {
                if Self::get_modified() > storage.imp().modified {
                    debug!("Externally modified: {}", &storage.imp().file.get_path());
                    storage.imp().data.into_inner() = storage.read(path.clone());
                    storage.emit_by_name("externally-modified");
                }
            });

        storage
    }

    fn read(&self, path: String) -> Json {
        *self.imp().modified = self.get_modified();
        let data = fs::read_to_string(&path).unwrap_or_else(|error| {
            if error.kind() == ErrorKind::NotFound {
                let _ = fs::File::create(&path).unwrap_or_else(|error| {
                     panic!("Problem creating the File: {:?}", error);
                });
                return fs::read_to_string(&path).unwrap_or_else( |error| {
                    panic!("Problem reading the File: {:?}", error);
            })
            } else {
                panic!("Unknown problem while reading the File: {:?}", error);
            }
        });
        return data.parse::<Json>().unwrap();
    }

    fn get_modified(&self) -> u64 {
        match self.imp().file.borrow().query_info(
            "time::modified",
            gio::FileQueryInfoFlags::NONE,
            gio::Cancellable::NONE,
        ) {
            Ok(info) => info.attribute_uint64("time::modified"),
            Err(_) => {
                debug!("Failed to get file info");
                *self.imp().data = RefCell::default();
                self.emit_by_name("externally-modified");
                0
            }
        }
    }

    pub fn get_path(_type: &str, key: String, extension: Option<String>) -> Option<&str> {
        let mut data_dir: path::PathBuf = path::PathBuf::new();
        if _type == "cache" {
            data_dir = glib::user_cache_dir()
        } else if _type == "config" {
            data_dir = glib::user_config_dir()
        } else {
            data_dir = glib::user_data_dir()
        }
        data_dir
            .join(config::PKGNAME)
            .join(form_urlencoded::Serializer::new(key))
            .with_extension(extension.unwrap_or("json"))
            .to_str()
    }

    fn write(&self, data: Json) {
        let parent = self.imp().file.borrow().parent().unwrap().path();
        debug!("Writing to {:?}", self.imp().file.borrow().path());
        let mkdirp =
            glib::mkdir_with_parents(self.imp().file.borrow().parent().unwrap().path(), 755);
        if mkdirp == 0 {
            if let Ok(success) = self.imp().file.try_borrow_mut().unwrap().replace_contents(
                data.to_string().as_ref(),
                None,
                false,
                gio::FileCreateFlags::REPLACE_DESTINATION,
            ) {
                self.imp().modified.replace(self.get_modified());
                self.emit_by_name("modified");
            }
        } else {
            error!("Could not save file")
        }
    }

    pub fn get(&self, property: &str) -> jsondata::Result<Json> {
        self.imp().data.borrow().get(property)
    }

    pub fn set(&mut self, property: &str, value: Json) {
        self.imp().data.into_inner().set(property, value)
            .expect(format!("Cannot set `data` for file: {}", self.imp().file.into_inner()).as_str());
    }

    pub fn clear(&self) {
        if let Err(_) = self.imp().file.into_inner().delete() {
            ()
        }
    }
}

