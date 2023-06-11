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
    Component, ComponentParts, ComponentSender,
};
use tracing::error;

// serve as the main container for library, reader component
// add Toast messages
pub struct BookxMainContainer {
    library: Controller<BookxLibrary>,
}

#[derive(Debug)]
pub enum BookxMainContainerMessage {
    OpenBookxReader,
    CloseBookxReader,
}

#[relm4_macros::component(pub)]
impl Component for BookxMainContainer {
    type CommandOutput = ();
    type Init = ();
    type Input = BookxMainContainerMessage;
    type Output = BookxMainContainerMessage;

    view! {
        #[name = "main_stack"]
        gtk::Stack {
            set_transition_type: gtk::StackTransitionType::SlideLeftRight,
            set_transition_duration: 250,

            add_named: (model.library.widget(), Some("bookx_library")),
        }
    }

    fn init(_: (), root: &Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let library = BookxLibrary::builder()
            .launch(())
            .forward(sender.input_sender(), |msg| match msg {
                BookxLibraryMessage::OpenBookxReader => BookxMainContainerMessage::OpenBookxReader,
            });
        let model = Self { library };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            BookxMainContainerMessage::OpenBookxReader => {
                sender
                    .output(BookxMainContainerMessage::OpenBookxReader)
                    .unwrap();

                // Build the bookx_reader widget and then add it named;
                // this avoids building widget when it's not supposed to be used.

                let bookx_reader = gtk::Box::builder()
                    .orientation(gtk::Orientation::Vertical)
                    .build();
                bookx_reader.append(&gtk::Label::builder().label("This is Bookx Reader").build());
                widgets
                    .main_stack
                    .add_named(&bookx_reader, Some("bookx_reader"));

                widgets.main_stack.set_visible_child_name("bookx_reader");
            }
            BookxMainContainerMessage::CloseBookxReader => {
                // app.rs will tell if want to close the Reader
                widgets.main_stack.set_visible_child_name("bookx_library");
            }
        }
    }
}
