// Bookx - book_box.rs
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

use crate::models::{Book, BookAction};
use crate::widgets::library::BookImage;
use epub::doc::EpubDoc;
use glib::subclass::InitializingObject;
use gtk::gdk_pixbuf;
use gtk::glib::{self, clone, Sender};
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk_macros::{action, send, spawn};
use log::error;
use once_cell::sync::OnceCell;
use std::borrow::Borrow;
use std::fs;
use std::io::Write;

mod imp {
    use super::*;

    #[derive(gtk::CompositeTemplate, Default)]
    #[template(resource = "/com/adhadse/Bookx/ui/library_book_box.ui")]
    pub struct BookBox {
        #[template_child]
        pub book_image: TemplateChild<BookImage>,
        // #[template_child]
        // pub progress_bar: TemplateChild<gtk::ProgressBar>,
        #[template_child]
        pub progress_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub menu_button: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub popover_menu: TemplateChild<gtk::PopoverMenu>,
        #[template_child]
        pub select: TemplateChild<gtk::Image>,

        pub book: OnceCell<Book>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BookBox {
        const NAME: &'static str = "BookBox";
        type Type = super::BookBox;
        type ParentType = gtk::FlowBoxChild;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for BookBox {
        fn dispose(&self, _obj: &Self::Type) {
            while let Some(child) = self.instance().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for BookBox {}

    impl FlowBoxChildImpl for BookBox {}
}

glib::wrapper! {
    pub struct BookBox(ObjectSubclass<imp::BookBox>)
        @extends gtk::Widget, gtk::FlowBoxChild;
}

impl BookBox {
    pub fn new(book: Book, sender: Sender<BookAction>) -> Self {
        let book_box: Self = glib::Object::new(&[]).unwrap();
        let actions = gio::ActionGroup::new();
        book_box.init(book);
        book_box.insert_action_group("library", actions);
        book_box.setup_actions(sender.clone());
        book_box.action_set_enabled("export-annotations", book.has_annotations());
        book_box
    }

    pub fn book(&self) -> &Book {
        self.imp().book.get().unwrap()
    }

    fn init(&self, book: Book) {
        let imp = self.imp();
        imp.book.set(book).unwrap();

        // if let Some(title) = &self.article().title {
        //     imp.title_label.set_text(title);
        // }
        //
        // match self.article().get_article_info(false) {
        //     Some(article_info) => imp.info_label.set_text(&article_info),
        //     None => {
        //         imp.info_label.hide();
        //     }
        // };
        //
        // if let Ok(Some(preview)) = self.article().get_preview() {
        //     imp.content_label.set_text(&preview);
        // }

        self.imp().progress_label.set_label(&book.get_progress());

        let book = self.book().clone();
        let book_image = imp.book_image.clone();

        // check if cover doesn't exist and then download it;
        // TODO: maybe move creating and saving of cover_picture to book.rs
        spawn!(async move {
            // src/models/preview_image.rs
            // src/models/article.rs#L168
            let exists = std::path::Path::new(book.imp().cover_picture_path)
                .try_exists()
                .unwrap_or_else(|_| {
                    // image does not exist; get cover from ebook and save
                    let doc = EpubDoc::new(book.imp().uri).unwrap();
                    let cover_data: Vec<u8> = doc.get_cover().unwrap();
                    let mut f = fs::File::create(book.imp().cover_picture_path).unwrap();
                    let resp = f.write_all(&cover_data);
                });
            if exists {
                match gdk_pixbuf::Pixbuf::from_file(book.imp().cover_picture_path) {
                    Ok(Some(pixbuf)) => book_image.set_pixbuf(&pixbuf),
                    _ => {
                        error!(
                            "Cannot create pixbuf for cover picture: {}",
                            book.imp().cover_picture_path
                        );
                        book_image.hide();
                    }
                };
            } else {
                // TODO: when even ebook don't have cover,
                // download and save_cover() and try loading that
            }
        });
    }

    fn setup_actions(&self, sender: Sender<BookAction>) {
        // library.book-details
        action!(
            self,
            "book-details",
            clone!(@strong self as this, @strong sender => move |_, _| {
                if let Some(book) = this.book().borrow().clone() {
                    send!(sender, BookAction::BookDetails(book));
                }
            })
        );
        // library.edit-metadata
        action!(
            self,
            "edit-metadata",
            clone!(@strong self as this, @strong sender => move |_, _| {
                if let Some(book) = this.book().borrow().clone() {
                    send!(sender, BookAction::EditMetadata(book));
                }
            })
        );
        // library.delete-book
        action!(
            self,
            "delete-book",
            clone!(@strong self as this, @strong sender => move |_, _| {
                if let Some(book) = this.book().borrow().clone() {
                    send!(sender, BookAction::DeleteBook(book));
                }
            })
        );
        // library.export-annotations
        action!(
            self,
            "export-annotations",
            clone!(@strong self as this, @strong sender => move |_, _| {
                if let Some(book) = this.book().borrow().clone() {
                    send!(sender, BookAction::ExportAnnotations(book));
                }
            })
        );
        // library.send-to-device
        action!(
            self,
            "send-to-device",
            clone!(@strong self as this, @strong sender => move |_, _| {
                if let Some(book) = this.book().borrow().clone() {
                    send!(sender, BookAction::SendToDevice(book));
                }
            })
        );
        // library.open-book
        action!(
            self,
            "open-book",
            clone!(@strong self as this, @strong sender => move |_, _| {
                if let Some(book) = this.book().borrow().clone() {
                    send!(sender, BookAction::OpenBook(book));
                }
            })
        );
    }
}
