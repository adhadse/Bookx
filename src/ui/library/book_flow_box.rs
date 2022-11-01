// Bookx - book_flow_box.rs
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

use gio::ListStore;
use crate::library::{Book, BookAction, ObjectWrapper};
use crate::ui::library::BookBox;
use gtk::gio;
use gtk::gio::prelude::*;
use gtk::glib::clone;
use gtk::glib::Sender;
use gtk_macros::get_widget;
use gtk_macros::send;
use log::error;

pub struct BookxFlowBox {
    pub widget: adw::Clamp,
    builder: gtk::Builder,
    sender: Sender<BookAction>,
}

impl BookxFlowBox {
    pub fn new(sender: Sender<BookAction>) -> Self {
        let builder = gtk::Builder::from_resource("/com/adhadse/Bookx/library_.ui");
        get_widget!(builder, adw::Clamp, book_flow_box);

        Self {
            builder,
            widget: book_flow_box,
            sender,
        }
    }

    pub fn bind_model(&self, book_list: ListStore) {
        get_widget!(self.builder, gtk::FlowBox, book_flow_box);

        book_flow_box.connect_child_activated(clone!(@strong self.sender as sender => move |_, list_box_row| {
            let book_box = list_box_row.downcast_ref::<BookBox>().unwrap();
            send!(sender, BookAction::Open(book_box.book().clone()));
        }));

        match book_list {
            Some(book_list) => {
                book_flow_box.bind_model(Some(book_list), move |book| {
                    // TODO: maybe downcast_ref is not required as `book` is already GObject?
                    let book: Book = book.downcast_ref::<ObjectWrapper>().unwrap().deserialize();
                    let book_box = BookBox::new(book, self.sender.clone());
                    book_box.upcast::<gtk::Widget>()
                });
            }
            None => {
                error!("Library book_list was empty.")
            }
        }
    }
}
