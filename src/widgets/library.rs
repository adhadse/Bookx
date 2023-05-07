// Bookx - library.rs
// This implements BookxLibraryWidget, controlled by widgets::BookxWindow. This is responsible for
// updating status pages as announced by views::BookxLibrary
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

use crate::widgets::BookxWindow;
use crate::{
    application::{Action, BookxApplication},
    config,
    models::BookxBook,
    views::{BookxLibrary, BookxLibraryStatus},
    widgets::BooksFlowBoxWidget,
};
use adw::subclass::prelude::*;
use glib::{clone, subclass, Sender};
use gtk::{
    prelude::*,
    subclass::prelude::*,
    {gio, glib, CompositeTemplate},
};
use gtk_macros::*;
use log::{debug, info};
use once_cell::unsync::OnceCell;

mod imp {
    use super::*;
    use crate::widgets::BookxWindow;
    use adw::glib::subclass::TypeData;
    use adw::glib::Type;
    use std::ptr::NonNull;

    #[derive(Debug, CompositeTemplate)]
    #[template(resource = "/com/adhadse/Bookx/ui/library_widget.ui")]
    pub struct BookxLibraryWidget {
        #[template_child]
        pub books_flow_box_widget: TemplateChild<BooksFlowBoxWidget>,
        #[template_child]
        pub library_error_status_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub library_null_status_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub library_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub book_properties_flap: TemplateChild<adw::Flap>,

        pub sender: OnceCell<Sender<Action>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BookxLibraryWidget {
        const NAME: &'static str = "BookxLibraryWidget";
        type Type = super::BookxLibraryWidget;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn new() -> Self {
            let books_flow_box_widget = TemplateChild::default();
            let library_error_status_page = TemplateChild::default();
            let library_null_status_page = TemplateChild::default();
            let library_stack = TemplateChild::default();
            let book_properties_flap = TemplateChild::default();

            let sender = OnceCell::default();

            Self {
                books_flow_box_widget,
                library_error_status_page,
                library_null_status_page,
                library_stack,
                book_properties_flap,
                sender,
            }
        }
        fn instance_init(obj: &subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for BookxLibraryWidget {}
    impl WidgetImpl for BookxLibraryWidget {}
    impl BinImpl for BookxLibraryWidget {}
}

glib::wrapper! {
    pub struct BookxLibraryWidget(ObjectSubclass<BookxLibraryWidget>)
    @extends gtk::Widget,
    @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl BookxLibraryWidget {
    pub fn init(&self, sender: Sender<Action>) {
        self.imp().sender.set(sender).unwrap();

        self.setup_widgets();
        self.setup_signals();
    }

    fn setup_widgets(&self) {
        let imp = self.imp();

        // Setup empty status page
        imp.library_error_status_page
            .set_icon_name(Some("dialog-error-symbolic")); // TODO: set error icon here
        imp.library_null_status_page
            .set_icon_name(Some(config::APP_ID));

        self.update();
    }

    // Update stack page whenever `status` of BookxLibrary gets updated
    fn setup_signals(&self) {
        BookxWindow::default().library().connect_notify_local(
            Some("status"),
            clone!(@weak self as this => move |_, _| this.update_library()),
        );
    }

    fn update_library(&self) {
        let imp = self.imp();
        info!(
            "{}",
            format!("Updating stack page to: {}", imp.library.status())
        );
        match imp.library.status() {
            BookxLibraryStatus::Loading => imp.library_stack.set_visible_child_name("loading"),
            BookxLibraryStatus::Error => imp.library_stack.set_visible_child_name("error"),
            BookxLibraryStatus::Null => imp.library_stack.set_visible_child_name("null"),
            BookxLibraryStatus::Content => {
                imp.books_flow_box_widget.bind_model(
                    &imp.library.into_inner().unwrap().books_list.borrow(),
                    imp.library.into_inner().unwrap().sender.clone(),
                );
                imp.library_stack.set_visible_child_name("content");
            }
        }
    }
}
