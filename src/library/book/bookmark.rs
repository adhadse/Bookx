// Bookx - bookmark.rs
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

use gtk::glib::{ParamFlags, ParamSpec, ParamSpecString};
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use once_cell::sync::Lazy;
use std::cell::RefCell;

mod imp {
    use super::*;

    #[derive(Default, Eq, PartialEq, Debug, Clone, Serialize, Deserialize)]
    pub struct Bookmark {
        pub cfi: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Bookmark {
        const NAME: &'static str = "Bookmark";
        type ParentType = glib::Object;
        type Class = glib::Class<Self>;

        glib_object_subclass!();
    }

    impl ObjectImpl for Bookmark {
        fn properties() -> &'static [ParamSpec] {
            static PROPERTIES: Lazy<Vec<ParamSpec>> = Lazy::new(|| {
                vec![ParamSpecString::new(
                    "cfi",
                    "CFI",
                    "CFI",
                    String::default(),
                    ParamFlags::READWRITE | ParamFlags::CONSTRUCT_ONLY,
                )]
            });
            PROPERTIES.as_ref()
        }
    }
}

glib::wrapper! {
    pub struct Bookmark(ObjectSubclass<imp::Bookmark>)
    @extends glib::Object;
}

impl Bookmark {
    pub fn new(cfi: String) -> Self {
        let bookmark = glib::Object::new::<Self>(&[]).unwrap();
        *bookmark.imp().cfi = cfi;
        bookmark
    }
}
