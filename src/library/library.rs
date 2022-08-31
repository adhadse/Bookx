// Bookx - library.rs
// Copyright (C) 2022  Anurag Dhadse <hi@anuragdhadse.com>
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

use crate::application::Action;
use crate::library::utils;
use crate::settings::{settings_manager, Key};
use crate::ui::BookxWindow;

use std::cell::RefCell;

use glib::{
    clone, Enum, ObjectExt, ParamFlags, ParamSpec, ParamSpecEnum, ParamSpecObject, Sender, ToValue,
};
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk_pixbuf, glib};
use gtk_macros::*;
use log::{debug, error, info};
use once_cell::sync::Lazy;
use once_cell::unsync::OnceCell;
use strum_macros::*;

#[derive(Display, Copy, Debug, Clone, EnumString, Eq, PartialEq, Enum)]
#[repr(u32)]
#[enum_type(name = "BookxLibraryStatus")]
pub enum BookxLibraryStatus {
    Loading,
    Content,
    Empty, // BooksDir is known, but no ebook found
    Null,  // No BooksDir
}

impl Default for BookxLibraryStatus {
    fn default() -> Self {
        BookxLibraryStatus::Null
    }
}

mod imp {
    use super::*;
    use gio::ListStore;
    use gtk_macros::send;

    #[derive(Debug, Default)]
    pub struct BookxLibrary {
        pub list: Vec<gio::File>,
        pub sender: OnceCell<Sender<Action>>,
        pub status: RefCell<BookxLibraryStatus>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BookxLibrary {
        const NAME: &'static str = "BookxLibrary";
        type Type = super::BookxLibrary;
    }

    impl ObjectImpl for BookxLibrary {
        fn properties() -> &'static [ParamSpec] {
            static PROPERTIES: Lazy<Vec<ParamSpec>> = Lazy::new(|| {
                vec![ParamSpecEnum::new(
                    "status",
                    "Status",
                    "Status",
                    BookxLibraryStatus::static_type(),
                    BookxLibraryStatus::default() as i32,
                    ParamFlags::READABLE,
                )]
            });

            PROPERTIES.as_ref()
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &ParamSpec) -> glib::Value {
            match pspec.name() {
                "status" => obj.status().to_value(),
                _ => unimplemented!(),
            }
        }
    }
}

glib::wrapper! {
    pub struct BookxLibrary(ObjectSubclass<imp::BookxLibrary>);
}

impl BookxLibrary {
    pub fn new(sender: Sender<Action>) -> Self {
        let library = glib::Object::new::<Self>(&[]).unwrap();
        library.imp().sender.set(sender).unwrap();

        library
    }

    pub fn status(&self) -> BookxLibraryStatus {
        *self.imp().status.borrow()
    }

    fn set_status(&self, status: &BookxLibraryStatus) {
        let imp = self.imp();
        *imp.status.borrow_mut() = status.clone();
        self.notify("status");
    }

    // previous signature: `files: &[gio::FIle]`
    pub fn refresh_data(&self) {
        let books_dir = &settings_manager::string(Key::BooksDir);
        debug!(
            "{}",
            format!("Books dir: {:?}", settings_manager::string(Key::BooksDir))
        );

        // if books_dir == ""
        // TODO: uncomment this once we figure out
        // BooksLibraryStatus::Empty
        if books_dir.is_empty() {
            self.set_status(&BookxLibraryStatus::Null);
            return;
        }

        // check if folder itself contains any books
        if books_dir.is_empty() {
            self.set_status(&BookxLibraryStatus::Empty);
            let window = BookxWindow::default();
            window.show_notification("Unable to access files");
            return;
        }

        self.set_status(&BookxLibraryStatus::Loading);
        let model = gio::ListStore::new(gio::File::static_type());
        let files = [gio::File::for_uri(&books_dir)];
        // gio::File::for_uri(&books_dir).
        for f in files {
            model.append(&f);
        }

        self.load_books(model.upcast_ref::<gio::ListModel>());
    }

    fn load_books(&self, model: &gio::ListModel) {
        for pos in 0..model.n_items() {
            let file = model.item(pos).unwrap().downcast::<gio::File>().unwrap();

            if let Ok(info) = file.query_info(
                "standard::name,standard::display-name,standard::type,standard::content-type",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                gio::Cancellable::NONE,
            ) {
                match info.file_type() {
                    gio::FileType::Regular => {
                        if let Some(content_type) = info.content_type() {
                            if gio::content_type_is_mime_type(&content_type, "application/epub+zip")
                            {
                                debug!("Adding file '{}' to the list", file.uri());
                                self.imp().list.to_owned().push(file);
                            }
                        }
                    }
                    gio::FileType::Directory => {
                        debug!("Adding folder '{}' to the list", file.uri());
                        let files = utils::load_files_from_folder(&file, true);
                        self.imp().list.to_owned().extend(files);
                    }
                    _ => (),
                }
            }
        }
        // TODO: do something with this list
        self.set_status(&BookxLibraryStatus::Content);
    }
}
