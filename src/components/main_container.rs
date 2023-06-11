// Bookx - main_container.rs
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

use crate::components::library::{BookxLibrary, BookxLibraryMessage};
use gettextrs::gettext;
use relm4::{
    gtk::{self, prelude::*},
    prelude::*,
    ComponentParts, ComponentSender, SimpleComponent,
};
use tracing::error;

// serve as the main container for library, reader component
// status page of library, add Toast messages
pub struct BookxMainContainer {
    library: Controller<BookxLibrary>,
}

#[derive(Debug)]
pub enum BookxMainContainerMessage {
    OpenBookxReader,
}

#[relm4_macros::component(pub)]
impl SimpleComponent for BookxMainContainer {
    type Init = ();
    type Input = BookxMainContainerMessage;
    type Output = BookxMainContainerMessage;

    view! {
        #[name = "main_container"]
        gtk::ScrolledWindow {
            set_hscrollbar_policy: gtk::PolicyType::Never,
            gtk::Viewport {
                set_vexpand: true,
                set_scroll_to_focus: true,
                set_child: Some(model.library.widget())
            }
        }
    }

    fn init(_: (), root: &Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let library = BookxLibrary::builder()
            .launch(String::from("/home/adhadse/Documents/sample_dir"))
            .forward(sender.input_sender(), |msg| match msg {
                BookxLibraryMessage::OpenBookxReader => BookxMainContainerMessage::OpenBookxReader,
            });
        let model = Self { library };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            BookxMainContainerMessage::OpenBookxReader => sender
                .output(BookxMainContainerMessage::OpenBookxReader)
                .unwrap(),
        }
    }
}
