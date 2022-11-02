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

use crate::application::{Action, BookxApplication};
use crate::config;
use crate::models::{Book, BookxLibrary, BookxLibraryStatus, ObjectWrapper};
use crate::widgets::library::{BookBox, BookxFlowBox};
use adw::subclass::prelude::*;
use glib::{clone, subclass, Sender};
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib, CompositeTemplate};
use log::info;
use once_cell::unsync::OnceCell;

mod imp {
    use super::*;

    #[derive(Debug, CompositeTemplate)]
    #[template(resource = "/com/adhadse/Bookx/ui/library.ui")]
    pub struct BookxLibraryPage {
        #[template_child]
        pub library_empty_status_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub library_null_status_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub library_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub book_flow_box: TemplateChild<BookxFlowBox>,

        pub library: BookxLibrary,
        pub sender: OnceCell<Sender<Action>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BookxLibraryPage {
        const NAME: &'static str = "BookxLibraryPage";
        type ParentType = adw::Bin;
        type Type = super::BookxLibraryPage;

        fn new() -> Self {
            let library_empty_status_page = TemplateChild::default();
            let library_null_status_page = TemplateChild::default();
            let library_stack = TemplateChild::default();
            let book_flow_box = TemplateChild::default();

            let app = gio::Application::default()
                .unwrap()
                .downcast::<BookxApplication>()
                .unwrap();
            let library = app.library();

            let sender = OnceCell::default();

            Self {
                library_empty_status_page,
                library_null_status_page,
                library_stack,
                book_flow_box,
                library,
                sender,
            }
        }

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for BookxLibraryPage {}
    impl WidgetImpl for BookxLibraryPage {}
    impl BinImpl for BookxLibraryPage {}
}

glib::wrapper! {
    pub struct BookxLibraryPage(ObjectSubclass<imp::BookxLibraryPage>)
    @extends gtk::Widget, adw::Bin;
}

impl BookxLibraryPage {
    pub fn init(&self, sender: Sender<Action>) {
        self.imp().sender.set(sender).unwrap();

        self.setup_widgets();
        self.setup_signals();
    }

    // pub fn set_sorting(&self, sorting: BookxSorting, descending: bool) {
    //     self.imp().content_box.get().set_sorting(sorting, descending);
    // }

    fn setup_widgets(&self) {
        let imp = self.imp();

        // Setup empty status page
        imp.library_empty_status_page
            .set_icon_name(Some(config::APP_ID));
        imp.library_null_status_page
            .set_icon_name(Some(config::APP_ID));

        self.update_library_ui();
    }

    // Update stack page whenever `status` gets updated
    fn setup_signals(&self) {
        self.imp().library.connect_notify_local(
            Some("status"),
            clone!(@weak self as this => move |_, _|
                this.update_library_ui();
            ),
        );
    }

    fn update_library_ui(&self) {
        let imp = self.imp();
        info!(
            "{}",
            format!("Updating stack page to: {:?}", imp.library.status())
        );
        match imp.library.status() {
            BookxLibraryStatus::Loading => imp.library_stack.set_visible_child_name("loading"),
            BookxLibraryStatus::Empty => imp.library_stack.set_visible_child_name("empty"),
            BookxLibraryStatus::Null => imp.library_stack.set_visible_child_name("null"),
            BookxLibraryStatus::Content => {
                // generate list store of Book out of book_init_list of library
                // and bind model of `Book` instances to `book_flow_box`
                imp.library_stack.set_visible_child_name("content");
                let book_list = gio::ListStore::new(Book::static_type());

                // let ctx = glib::MainContext::default();
                // ctx.spawn(async move {
                //     let futures = async move {
                //         articles.into_iter().for_each(|article| {
                //             send!(sender, ArticleAction::Add(article));
                //         })
                //     };
                //     pool.spawn_ok(futures);
                // });

                imp.library
                    .imp()
                    .book_init_list
                    .get()
                    .unwrap()
                    .iter()
                    .for_each(|b_init| {
                        book_list.append(Book::new(
                            b_init.id.clone(),
                            b_init.format,
                            b_init.uri.clone(),
                        ));
                    });
                imp.book_flow_box.bind_model(book_list);
            }
        }
    }
}
