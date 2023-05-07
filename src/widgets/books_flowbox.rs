// books_flowbox.rs
// This file is responsible for creating FlowBox widget and is created by views::BookxLibrary
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

use crate::models::BookxBookObject;
use crate::{models::BookAction, widgets::book::BookWidget};
use gtk::{
    gio::{self, prelude::*},
    glib::{self, clone, subclass::*, ParamSpec, ParamSpecString, Sender, Value},
    prelude::*,
    subclass::{self, prelude::*},
    TemplateChild,
};
use gtk_macros::send;
use log::error;
use once_cell::sync::{Lazy, OnceCell};

mod imp {
    use super::*;

    #[derive(gtk::CompositeTemplate, Default)]
    #[template(resource = "/com/adhadse/Bookx/library_flowbox_widget.ui")]
    pub struct BooksFlowBoxWidget {
        #[template_child]
        pub flow_box: TemplateChild<gtk::FlowBox>,
        pub sender: OnceCell<Sender<BookAction>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BooksFlowBoxWidget {
        const NAME: &'static str = "BooksFlowBoxWidget";
        type Type = super::BooksFlowBoxWidget;
        type ParentType = gtk::Widget;

        fn new() -> Self {
            let flow_box = TemplateChild::default();
            let sender = OnceCell::default();
            Self { flow_box, sender }
        }

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks()
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for BooksFlowBoxWidget {
        fn properties() -> &'static [ParamSpec] {
            static PROPERTIES: Lazy<Vec<ParamSpec>> =
                Lazy::new(|| vec![ParamSpecString::builder("placeholder-icon-name").build()]);
            PROPERTIES.as_ref()
        }

        fn set_property(&self, _id: usize, value: &Value, pspec: &ParamSpec) {
            match pspec.name() {
                "placeholder-icon-name" => {
                    let icon_name = value.get().unwrap();
                    self.empty_status.set_icon_name(icon_name)
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, _id: usize, pspec: &ParamSpec) -> Value {
            match pspec.name() {
                "placeholder-icon-name" => self.empty_status.icon_name().to_value(),
                _ => unimplemented!(),
            }
        }

        fn dispose(&self) {
            self.obj().dispose_template();
        }
    }

    impl WidgetImpl for BooksFlowBoxWidget {}

    #[gtk::template_callbacks]
    impl BooksFlowBoxWidget {
        #[template_callback]
        fn handle_child_activated(&self, book_widget: &BookWidget, _flow_box: &gtk::FlowBox) {
            let sender = self.sender.get().unwrap();
            send!(sender, BookAction::Open(book_widget.book().book().clone()));
        }
    }
}

glib::wrapper! {
    pub struct BooksFlowBoxWidget(ObjectSubclass<imp::BooksFlowBoxWidget>)
        @extends gtk::Widget;
}

impl BooksFlowBoxWidget {
    pub fn init(&self, sender: Sender<BookAction>) {
        self.imp().sender.set(sender).unwrap();
    }

    pub fn bind_model(&self, book_list: &gio::ListStore, sender: Sender<BookAction>) {
        self.imp()
            .flow_box
            .bind_model(Some(book_list), move |book| {
                let book = book.downcast_ref::<BookxBookObject>().unwrap();
                let book_widget = BookWidget::new(book.clone(), sender);
                book_widget.upcast::<gtk::Widget>()
            });
    }
}
