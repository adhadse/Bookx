// Bookx - bookx_library.rs
// Copyright (C) 2023  Anurag Dhadse <hello@adhadse.com>
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

use crate::components::library::BookxBook;
use crate::components::utils;
use crate::config::APP_ID;
use gettextrs::gettext;
use gtk::prelude::*;
use relm4::{
    gtk::{self, gio},
    Component, ComponentController, ComponentParts, ComponentSender,
};

// responsible for displaying
pub struct BookxLibrary {}

#[derive(Debug)]
pub enum BookxLibraryMessage {
    OpenBookxReader,
}

#[relm4_macros::component(pub)]
impl Component for BookxLibrary {
    type CommandOutput = ();
    type Init = ();
    type Input = ();
    type Output = BookxLibraryMessage;

    view! {
        gtk::ScrolledWindow {
            set_hscrollbar_policy: gtk::PolicyType::Never,
            gtk::Viewport {
                set_vexpand: true,
                set_scroll_to_focus: true,

                #[name = "library"]
                #[wrap(Some)]
                set_child= &gtk::FlowBox {
                    set_activate_on_single_click: false,
                    set_column_spacing: 12,
                    set_row_spacing: 12,
                    set_focus_on_click: true,
                    set_selection_mode: gtk::SelectionMode::Single,
                    set_visible: true,
                    set_valign: gtk::Align::Start,
                    set_max_children_per_line: 100,
                    connect_child_activated[sender] => move |_, _| {
                        sender.output(BookxLibraryMessage::OpenBookxReader).unwrap()
                    }
                }
            }
        }
    }

    fn init(
        _init: Self::Init,
        root: &Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = BookxLibrary {};
        let widgets = view_output!();

        // let settings = gio::Settings::new(APP_ID);
        // let content_dir = settings.string("books-dir");

        // if content_dir == ""
        // if content_dir.is_empty() {
        //     self.set_status(&BookxLibraryStatus::Null);
        //     send!(
        //         self.sender,
        //         Action::Notification(
        //             "Books Directory string is empty. \
        //             Set it to an existing path in Preferences."
        //                 .to_string()
        //         )
        //     );
        //     return Err(BookxLibraryError::new("BooksDir string is empty"));
        // }

        let content_dir = "/home/adhadse/Documents/sample_dir";

        let book_files = utils::load_files_from_folder(&gio::File::for_path(content_dir), true);

        for book_file in book_files {
            if let Ok(bookx_book) =
                BookxBook::load_book(book_file.path().unwrap().display().to_string())
            {
                let bookx_book_comp = BookxBook::builder().launch(bookx_book).detach();
                let library = &widgets.library;
                library.append(bookx_book_comp.widget());
            }
        }
        ComponentParts { model, widgets }
    }

    // fn update_with_view(
    //     &mut self,
    //     widgets: &mut Self::Widgets,
    //     message: Self::Input,
    //     sender: ComponentSender<Self>,
    //     _root: &Self::Root,
    // ) {
    //     match message {

    //     }
    // }
}

// TODO:
// Then send message for updated status (let main_container update the status page),
