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

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk_pixbuf::Pixbuf, glib};

mod imp {
    use super::*;
    use gtk::glib::subclass::InitializingObject;

    #[derive(gtk::CompositeTemplate, Default)]
    #[template(resource = "/com/adhadse/Bookx/ui/library_book_image.ui")]
    pub struct BookImage {
        #[template_child]
        pub spinner: TemplateChild<gtk::Spinner>,
        #[template_child]
        pub image: TemplateChild<gtk::Image>,
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
        fn dispose(&self, _obj: &Self::Type) {
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
    pub fn set_pixbuf(&self, pixbuf: &Pixbuf) {
        self.imp().image.set_from_pixbuf(Some(pixbuf));
        self.imp().image.show();
        self.imp().spinner.hide();
    }
}
