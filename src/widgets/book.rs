// Bookx - book_widget.rs
// This implements the Book widget, a child of widgets::BooksFlowBoxWidget, forwards the actions
// of popover menu to models::BooksManager
//
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
    models::{BookAction, BookxBook, BookxBookObject},
    widgets::BookImage,
};
use gtk::{
    glib::{self, clone, subclass::InitializingObject, ParamSpec, ParamSpecObject, Sender, Value},
    prelude::*,
    subclass::prelude::*,
};
use gtk_macros::{action, send, spawn};
use log::error;
use once_cell::sync::{Lazy, OnceCell};
use std::borrow::Borrow;

mod imp {
    use super::*;

    #[derive(gtk::CompositeTemplate, Default)]
    #[template(resource = "/com/adhadse/Bookx/ui/library_book_widget.ui")]
    pub struct BookWidget {
        #[template_child]
        pub book_image: TemplateChild<BookImage>,
        #[template_child]
        pub progress_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub progress_bar: TemplateChild<gtk::ProgressBar>,
        #[template_child]
        pub select: TemplateChild<gtk::Image>,
        // #[template_child]
        // pub book_context_menu: TemplateChild<gio::MenuModel>,
        pub sender: OnceCell<Sender<BookAction>>,
        pub actions: gio::SimpleActionGroup,
        pub book: OnceCell<BookxBookObject>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BookWidget {
        const NAME: &'static str = "BookWidget";
        type Type = super::BookWidget;
        type ParentType = gtk::FlowBoxChild;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for BookWidget {
        fn properties() -> &'static [ParamSpec] {
            static PROPERTIES: Lazy<Vec<ParamSpec>> = Lazy::new(|| {
                vec![
                    ParamSpecObject::builder::<BookxBookObject>("book")
                        .construct_only()
                        .build(),
                    ParamSpecObject::builder::<Sender<BookAction>>("sender")
                        .construct_only()
                        .build(),
                ]
            });
            PROPERTIES.as_ref()
        }

        fn set_property(&self, _id: usize, value: &Value, pspec: &ParamSpec) {
            match pspec.name() {
                "book" => {
                    let book = value.get().unwrap();
                    self.book.set(book).unwrap();
                    // TODO: We can probably set more properties here...
                }
                "sender" => {
                    let sender = value.get().unwrap();
                    self.sender.set(sender).unwrap();
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, _id: usize, pspec: &ParamSpec) -> Value {
            match pspec.name() {
                "book" => self.book.get().to_value(),
                "sender" => self.sender.get().to_value(),
                _ => unimplemented!(),
            }
        }

        fn dispose(&self) {
            self.obj().dispose_template();
        }
    }

    impl WidgetImpl for BookWidget {}

    impl FlowBoxChildImpl for BookWidget {}

    #[gtk::template_callbacks]
    impl BookWidget {
        fn update_load_progress(&self, _pspec: &glib::ParamSpec) {
            self.progress_bar
                .set_fraction(self.book.get().unwrap().book().progress.into_inner() as f64)
        }
    }
}

glib::wrapper! {
    pub struct BookWidget(ObjectSubclass<imp::BookWidget>)
        @extends gtk::Widget, gtk::FlowBoxChild;
}

impl BookWidget {
    pub fn new(book: BookxBookObject, sender: Sender<BookAction>) -> Self {
        let book_widget = glib::Object::builder()
            .property("book", book)
            .property("sender", sender)
            .build();
        book_widget.setup_actions(sender.clone());
        book_widget.action_set_enabled("export-annotations", book.has_annotations());
        book_widget
    }

    fn setup_actions(&self, sender: Sender<BookAction>) {
        // library.show-book-properties
        action!(
            self,
            "show-book-properties",
            clone!(@strong self as this, @strong sender => move |_, _| {
                if let Some(book) = this.book().borrow().clone() {
                    send!(sender, BookAction::ShowBookProperties(book));
                }
            })
        );
        // library.edit-metadata
        action!(
            self,
            "edit-metadata",
            clone!(@strong self as this, @strong sender => move |_, _| {
                if let Some(book) = this.book().borrow().clone() {
                    send!(sender, BookAction::EditMetadata(book));
                }
            })
        );
        // library.delete-book
        action!(
            self,
            "delete-book",
            clone!(@strong self as this, @strong sender => move |_, _| {
                if let Some(book) = this.book().borrow().clone() {
                    send!(sender, BookAction::DeleteBook(book));
                }
            })
        );
        // library.export-annotations
        action!(
            self,
            "export-annotations",
            clone!(@strong self as this, @strong sender => move |_, _| {
                if let Some(book) = this.book().borrow().clone() {
                    send!(sender, BookAction::ExportAnnotations(book));
                }
            })
        );
        // library.send-to-device
        action!(
            self,
            "send-to-device",
            clone!(@strong self as this, @strong sender => move |_, _| {
                if let Some(book) = this.book().borrow().clone() {
                    send!(sender, BookAction::SendToDevice(book));
                }
            })
        );
        // library.open-book
        action!(
            self,
            "open-book",
            clone!(@strong self as this, @strong sender => move |_, _| {
                if let Some(book) = this.book().borrow().clone() {
                    send!(sender, BookAction::OpenBook(book));
                }
            })
        );
    }

    pub fn book(&self) -> BookxBookObject {
        self.property::<BookxBookObject>("book")
    }

    pub fn get_actions(&self) -> &gio::SimpleActionGroup {
        &self.imp().actions
    }
}
