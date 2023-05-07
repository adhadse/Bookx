// Bookx - BookxBookObject is GObject wrapper around Book struct
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

use gtk::{glib, prelude::*, subclass::prelude::*};

use crate::models::BookxBook;

mod imp {
    use once_cell::sync::OnceCell;

    use super::*;

    #[derive(Default)]
    pub struct BookxBookObject {
        pub book: OnceCell<BookxBook>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BookxBookObject {
        const NAME: &'static str = "BookxBookObject";
        type Type = super::BookxBookObject;
    }

    impl ObjectImpl for BookxBookObject {
        fn properties() -> &'static [glib::ParamSpec] {
            use once_cell::sync::Lazy;
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpecString::builder("title").read_only().build(),
                    glib::ParamSpecUInt::builder("progress").read_only().build(),
                    glib::ParamSpecString::builder("cover-picture-path")
                        .read_only()
                        .build(),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let book = self.book.get().unwrap();
            match pspec.name() {
                "title" => book.title.clone().to_value(),
                "progress" => book.progress.clone().to_value(),
                "cover-picture-path" => book.get_cover_picture_path().clone().to_value(),
                _ => unimplemented!(),
            }
        }
    }
}

glib::wrapper! {
    pub struct BookxBookObject(ObjectSubclass<imp::BookxBookObject>);
}

impl BookxBookObject {
    pub fn new(book: BookxBook) -> Self {
        let obj = glib::Object::builder::<Self>().build();
        obj.imp().book.set(book).unwrap();
        obj
    }

    pub fn book(&self) -> &BookxBook {
        self.imp().book.get().unwrap()
    }
}
