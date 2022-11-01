// Bookx - library.rs
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

use gtk::glib;
use gtk::glib::{Receiver, Sender};
use gtk_macros::send;
use log::error;
use std::cell::RefCell;
use std::rc::Rc;

use super::Book;
use crate::application::Action;

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum BookAction {
    BookDetails(Book),
    EditMetadata(Book),
    DeleteBook(Book),
    ExportAnnotations(Book),
    SendToDevice(Book),
    OpenBook(Book),
}

pub struct BooksManager {
    main_sender: Sender<Action>,
    pub sender: Sender<BookAction>,
    receiver: RefCell<Option<Receiver<BookAction>>>,
}

impl BooksManager {
    pub fn new(main_sender: Sender<Action>) -> Rc<Self> {
        let (sender, r) = glib::MainContext::channel(glib::PRIORITY_DEFAULT);
        let receiver = RefCell::new(Some(r));

        let manager = Rc::new(Self {
            main_sender,
            sender,
            receiver,
        });

        manager.init(manager.clone());
        manager
    }

    fn init(&self, manager: Rc<Self>) {
        let receiver = self.receiver.borrow_mut().take().unwrap();
        receiver.attach(None, move |action| manager.do_action(action));
    }

    fn do_action(&self, action: BookAction) -> glib::Continue {
        match action {
            BookAction::BookDetails(book) => self.book_details(book),
            BookAction::EditMetadata(book) => self.edit_metadata(book),
            BookAction::DeleteBook(book) => self.delete_book(book),
            BookAction::ExportAnnotations(book) => self.export_annotations(book),
            BookAction::SendToDevice(book) => self.send_to_device(book), // Update Book values by their ID.
            BookAction::OpenBook(book) => self.open_book(book),
        };
        glib::Continue(true)
    }

    fn book_details(&self, book: Book) {
        send!(self.main_sender, Action::Books(Box::new(BookAction::BookDetails(book))));
    }

    fn edit_metadata(&self, book: Book) {
        send!(self.main_sender, Action::Books(Box::new(BookAction::EditMetadata(book))));
    }

    fn delete_book(&self, book: Book) {
        send!(self.main_sender, Action::Books(Box::new(BookAction::DeleteBook(book))));
    }

    fn export_annotations(&self, book: Book) {
        send!(self.main_sender, Action::Books(Box::new(BookAction::ExportAnnotations(book))));
    }

    fn send_to_device(&self, book: Book) {
        send!(self.main_sender, Action::Books(Box::new(BookAction::SendToDevice(book))));
    }

    fn open_book(&self, book: Book) {
        send!(self.main_sender, Action::Books(Box::new(BookAction::OpenBook(book))));
    }

}
