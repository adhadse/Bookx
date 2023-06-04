// Bookx - library.rs
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
use gettextrs::gettext;
use gtk::prelude::*;
use relm4::Component;
use relm4::{
    gtk::{self, gio},
    ComponentController, ComponentParts, ComponentSender, SimpleComponent,
};

// responsible for displaying
pub struct BookxLibrary {}

#[relm4_macros::component(pub)]
impl SimpleComponent for BookxLibrary {
    type Init = String;
    type Input = ();
    type Output = ();

    view! {
        #[name = "library"]
        gtk::FlowBox {
            set_activate_on_single_click: true,
            set_column_spacing: 12,
            set_row_spacing: 12,
            set_focus_on_click: true,
            set_selection_mode: gtk::SelectionMode::None,
            set_visible: true,
            set_valign: gtk::Align::Start,
            set_max_children_per_line: 100,
        }
    }

    fn init(
        content_dir: Self::Init,
        root: &Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = BookxLibrary {};
        let widgets = view_output!();
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
}

// TODO:
// Then send message for updated status (let main_container update the status page),
// load these book using flowbox widget inside library
