// Bookx - library.rs
// This is Library component, responsible for setting BookxLibraryStatus, reading the
// library folder and initializing FlowBoxWidget.
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
    application::Action,
    models::format::{Format, FormatNotSupportedError},
    models::{BookxBook, BookxBookError, BookxBookObject},
    settings::{settings_manager, Key},
    widgets::BooksFlowBoxWidget,
};
use epub::doc::DocError;
use std::cell::{Ref, RefCell};
use std::fmt::{write, Formatter};
use std::fs::File;
use std::iter::Once;
use std::{error::Error, fmt};

use crate::models::BookAction;
use crate::views::utils;
use anyhow::__private::kind::TraitKind;
use futures::TryFutureExt;
use gtk::{
    glib::{self, Enum, Sender},
    prelude::*,
    subclass::prelude::*,
};
use gtk_macros::send;
use log::{debug, error, info};
use once_cell::{sync::Lazy, unsync::OnceCell};
use std::path::PathBuf;
use strum_macros::*;

#[derive(Display, Copy, Debug, Clone, EnumString, Eq, PartialEq, Enum)]
#[repr(u32)]
#[enum_type(name = "BookxLibraryStatus")]
pub enum BookxLibraryStatus {
    Loading,
    Content,
    Error, // BooksDir is known, but no ebook found; or an error occurred
    Null,  // No BooksDir
}

impl Default for BookxLibraryStatus {
    fn default() -> Self {
        BookxLibraryStatus::Null
    }
}

#[derive(Debug)]
struct BookxLibraryError {
    details: String,
}

impl BookxLibraryError {
    fn new(msg: &str) -> BookxLibraryError {
        BookxLibraryError {
            details: msg.to_string(),
        }
    }
}

impl fmt::Display for BookxLibraryError {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{}", self.details)
    }
}

impl Error for BookxLibraryError {
    fn description(&self) -> &str {
        &self.details
    }
}

#[derive(Debug, Default)]
pub struct BookxLibrary {
    pub books_list: RefCell<gio::ListStore>,
    pub sender: Sender<BookAction>,
    pub status: RefCell<BookxLibraryStatus>,
    pub monitor: RefCell<Option<gio::FileMonitor>>,
}

impl BookxLibrary {
    pub fn new(sender: Sender<BookAction>) -> Self {
        let books_list = RefCell::new(gio::ListStore::new(BookxBookObject::static_type()));
        let status = RefCell::new(BookxLibraryStatus::default());
        let monitor = RefCell::new(*gio::FileMonitor::NONE);
        let library = Self {
            books_list,
            sender,
            status,
            monitor,
        };
        // TODO handle error here (basically do nothing); Make it async
        library.refresh_data();
        library
    }

    pub async fn refresh_data(&self) -> Result<(), BookxLibraryError> {
        self.set_status(&BookxLibraryStatus::Loading);
        let books_dir = gio::File::for_uri(&settings_manager::string(Key::BooksDir));
        debug!(
            "{}",
            format!("Books dir: {:?}", settings_manager::string(Key::BooksDir))
        );

        // if books_dir == ""
        if books_dir.is_empty() {
            self.set_status(&BookxLibraryStatus::Null);
            send!(
                self.sender,
                Action::Notification(
                    "Books Directory string is empty. \
                    Set it to an existing path in Preferences."
                        .to_string()
                )
            );
            return Err(BookxLibraryError::new("BooksDir string is empty"));
        }

        let monitor = books_dir
            .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
            .expect(&format!(
                "Cannot create FileMonitor for `books_dir`{}",
                Key::BooksDir
            ));
        monitor.connect_changed(
            glib::clone!(@weak self as obj => move |_monitor, file_a, file_b, event| {
                obj.file_monitor_cb(event, file_a, file_b);
            }),
        );
        self.monitor.replace(Some(monitor)).unwrap();
        let book_pathbufs = self.get_book_pathbufs(books_dir);

        let mut new_books_list = gio::ListStore::new(BookxBookObject::static_type());
        // books_list.append(&Format::get_filter());
        // books_list.extend(book_pathbufs);

        return if !book_pathbufs.is_empty() {
            // Load books from Book Pathbufs of accepted MIME type and update it in
            // BookxLibrary.books_list: gio::ListStore::<BookxBookObject>
            for book_pathbuf in book_pathbufs.into_iter() {
                let book = match BookxBook::load(book_pathbuf) {
                    Ok(book) => book,
                    Err(e) => match e.anyhow_kind() {
                        BookxBookError::FormatNotSupportedError => {
                            send!(
                                self.sender,
                                Action::Notification(format!(
                                    "{} is not supported by Bookx",
                                    book_pathbuf.to_str().unwrap()
                                ))
                            );
                            continue;
                        }
                        BookxBookError::DocError(DocError::ArchiveError) => {
                            send!(
                                self.sender,
                                Action::Notification(format!(
                                    "Bookx failed to load {}. ArchiveError occurred.",
                                    book_pathbuf.to_str().unwrap()
                                ))
                            );
                            error!(
                                "ArchiveError for {}. Source: {}",
                                book_pathbuf.to_str().unwrap(),
                                e.source().unwrap()
                            );
                            continue;
                        }
                        BookxBookError::DocError(DocError::XmlError) => {
                            send!(
                                self.sender,
                                Action::Notification(format!(
                                    "Bookx failed to load {}. XmlError occurred.",
                                    book_pathbuf.to_str().unwrap()
                                ))
                            );
                            error!(
                                "XmlError for {}. Source: {}",
                                book_pathbuf.to_str().unwrap(),
                                e.source().unwrap()
                            );
                            continue;
                        }
                        BookxBookError::DocError(DocError::IOError) => {
                            send!(
                                self.sender,
                                Action::Notification(format!(
                                    "Bookx failed to load {}. IOError occurred.",
                                    book_pathbuf.to_str().unwrap()
                                ))
                            );
                            error!(
                                "IOError for {}. Source: {}",
                                book_pathbuf.to_str().unwrap(),
                                e.source().unwrap()
                            );
                            continue;
                        }
                        BookxBookError::DocError(DocError::InvalidEpub) => {
                            send!(
                                self.sender,
                                Action::Notification(format!(
                                    "Bookx failed to load {}. InvalidEpub error occurred.",
                                    book_pathbuf.to_str().unwrap()
                                ))
                            );
                            error!(
                                "InvalidEpub error for {}. Source: {}",
                                book_pathbuf.to_str().unwrap(),
                                e.source().unwrap()
                            );
                            continue;
                        }
                    },
                };
                new_books_list.append(&BookxBookObject::new(book));
            }

            if new_books_list.n_items() != 0 {
                // We found some readable books
                self.books_list.replace_with(new_books_list);
                self.set_status(&BookxLibraryStatus::Content);
                Ok(())
            } else {
                // No Readable books in listStore/library
                self.set_status(&BookxLibraryStatus::Error);
                send!(
                    self.sender,
                    Action::Notification("Library has no readable Books.".to_string())
                );
                Err(BookxLibraryError::new("Library has no readable Books."))
            }
        } else {
            self.set_status(&BookxLibraryStatus::Error);
            send!(
                self.sender,
                Action::Notification("Library folder is Empty".to_string())
            );
            Err(BookxLibraryError::new("Library Folder is Empty."))
        };
    }

    fn get_book_pathbufs<'a>(&self, file: gio::File) -> Vec<PathBuf> {
        let mut files_list = vec![];

        if let Ok(info) = file.query_info(
            "standard::name,standard::display-name,standard::type,standard::content-type",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            gio::Cancellable::NONE,
        ) {
            match info.file_type() {
                gio::FileType::Regular => {
                    if let Some(content_type) = info.content_type() {
                        debug!("Content type: {}", content_type);
                        // Checks whether the content mime type is acceptable as EBook
                        debug!("Adding file '{}' to the list", file.uri());
                        files_list.push(file);
                        file.path()
                    }
                }
                gio::FileType::Directory => {
                    debug!("Adding folder '{}' to the list", file.uri());
                    let files = utils::load_files_from_folder(&file, true);
                    files_list.extend(files);
                }
                _ => (),
            }
        }

        return files_list.into_iter().map(|f| f.path().unwrap()).collect();
    }

    pub fn status(&self) -> BookxLibraryStatus {
        *self.status.borrow()
    }

    fn set_status(&self, status: &BookxLibraryStatus) {
        *self.status.borrow_mut() = status.clone();
        self.notify("status");
    }
}
