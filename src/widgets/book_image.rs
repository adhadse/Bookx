// Bookx - book_image.rs
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

use crate::models::BookxBookImage;
use gtk::{
    glib::{clone, subclass::InitializingObject, ParamSpec, Value},
    prelude::*,
    subclass::prelude::*,
    {gdk_pixbuf::Pixbuf, glib, prelude::*, subclass::prelude::*},
};
use gtk_macros::spawn;
use once_cell::sync::Lazy;
use std::{cell::RefCell, path::PathBuf};

mod imp {
    use super::*;

    #[derive(gtk::CompositeTemplate, Default)]
    #[template(resource = "/com/adhadse/Bookx/ui/library_book_image.ui")]
    pub struct BookImage {
        #[template_child]
        pub spinner: TemplateChild<gtk::Spinner>,
        #[template_child]
        pub image: TemplateChild<gtk::Image>,

        // this identifier of book is bind when creating `BookWidget` and comes from
        // `library_book_widget.ui`
        pub path: RefCell<Option<PathBuf>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BookImage {
        const NAME: &'static str = "BookImage";
        type Type = super::BookImage;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for BookImage {
        fn properties() -> &'static [ParamSpec] {
            static PROPERTIES: Lazy<Vec<ParamSpec>> =
                Lazy::new(|| vec![glib::ParamSpecString::builder("path").build()]);
            PROPERTIES.as_ref()
        }

        fn property(&self, _id: usize, pspec: &ParamSpec) -> Value {
            match pspec.name() {
                "path" => self.path.borrow().to_value(),
                _ => unimplemented!(),
            }
        }

        fn set_property(&self, _id: usize, value: &Value, pspec: &ParamSpec) {
            match pspec.name() {
                "path" => {
                    let path = value.get().unwrap();
                    self.path.replace(path);

                    spawn!(clone!(@weak self as book_image => async move {
                        match book_image.get_cover_or_download().await {
                            Some(pixbuf) => book_image.set_pixbuf(&pixbuf),
                            _ => book_image.hide(),
                        };
                    }));
                }
                _ => unimplemented!(),
            }
        }

        fn dispose(&self) {
            while let Some(child) = self.instance().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for BookImage {}
}

glib::wrapper! {
    pub struct BookImage(ObjectSubclass<imp::BookImage>)
        @extends gtk::Widget;
}

impl BookImage {
    pub async fn get_cover_picture(&self) -> Option<Pixbuf> {
        let path = self.path.borrow().clone();
        match path {
            Some(path) => {
                let bookx_book_image = BookxBookImage::new(path);
                if !bookx_book_image.exists() {
                    bookx_book_image.download().await.ok()?;
                }
                Pixbuf::from_file(&preview_image.cache).ok()
            }
            None => None,
        }
    }

    pub fn set_pixbuf(&self, pixbuf: &Pixbuf) {
        self.imp().image.set_from_pixbuf(Some(pixbuf));
        self.imp().image.show();
        self.imp().spinner.hide();
    }
}
