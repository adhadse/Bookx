// Bookx - bookx_book.rs
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

use epub::doc::{DocError, EpubDoc};
use relm4::{
    gtk::{self, gdk_pixbuf::Pixbuf, glib, prelude::*},
    ComponentParts, ComponentSender, SimpleComponent,
};
use tracing::error;

use std::fs;
use std::io::Write;
use std::path::PathBuf;

pub struct BookxBook {
    pub title: String,
    pub progress: f64,
    pub pixbuf: Pixbuf,
}

#[relm4_macros::component(pub)]
impl SimpleComponent for BookxBook {
    type Init = Self;
    type Input = ();
    type Output = ();

    view! {
        #[name = "bookx_book"]
        gtk::Box {
            set_tooltip_text: Some(&model.title),
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 1,
            set_halign: gtk::Align::Start,
            set_valign: gtk::Align::End,
            set_width_request: 120,

            gtk::Box {
                set_halign: gtk::Align::Center,
                set_orientation: gtk::Orientation::Vertical,

                gtk::Image {
                    set_from_pixbuf: Some(&model.pixbuf),
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                    set_icon_size: gtk::IconSize::Normal,
                    set_width_request: 160,
                    set_height_request: 200,
                },
                gtk::Box {
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                    gtk::Label {
                        set_label: &format!("{}%", model.progress.clone()),
                        set_margin_end: 10
                    },
                    gtk::LevelBar {
                        set_value: model.progress,
                        set_min_value: 0.0,
                        set_max_value: 100.0,
                        set_width_request: 70,
                        set_orientation: gtk::Orientation::Horizontal
                    }
                }

            }
        }
    }

    fn init(init: Self, root: &Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let model = init;
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }
}

impl BookxBook {
    pub fn load_book(book_path: String) -> Result<Self, DocError> {
        match EpubDoc::new(book_path.clone()) {
            Ok(mut doc) => {
                let identifier = match doc.mdata("identifier") {
                    Some(identifier) => identifier,
                    None => {
                        error!("Cannot find MetaData `identifier` for Book at path: {:?}, skipped loading.", book_path);
                        return Err(DocError::InvalidEpub);
                    }
                };
                let title = match doc.mdata("title") {
                    Some(title) => title,
                    None => {
                        error!(
                            "Cannot find MetaData `title` for Book at path: {:?}. skipped loading.",
                            book_path
                        );
                        return Err(DocError::InvalidEpub);
                    }
                };
                let cover_path: PathBuf = glib::user_cache_dir()
                    .join("cover_images")
                    .join(format!("{}.png", identifier));

                // TODO: not rewrite if exists
                match doc.get_cover() {
                    Some((cover_data, _)) => {
                        let parent = cover_path.parent().unwrap();
                        match fs::create_dir_all(parent) {
                            Ok(()) => {
                                let mut f = fs::File::create(cover_path.clone()).unwrap();
                                let resp = f.write_all(&cover_data);
                                tracing::debug!(
                                    "Writing book cover for: {:?}; Response: {:?}",
                                    book_path,
                                    resp
                                );
                            }
                            Err(e) => {
                                error!(
                                    "Error occurred when creating `cover_images` dir for
                                        bookx cache."
                                );
                            }
                        };
                    }
                    None => {
                        error!("Cannot find cover for Book at path: {:?}", book_path);
                        return Err(DocError::InvalidEpub);
                    }
                };

                let model = BookxBook {
                    title,
                    progress: 40.0,
                    pixbuf: Pixbuf::from_file_at_scale(cover_path, 180, 180, true).unwrap(),
                };

                Ok(model)
            }
            Err(err) => match err {
                DocError::ArchiveError(e) => {
                    error!("Error when loading book: {:?} ", e);
                    Err(DocError::ArchiveError(e))
                }
                DocError::XmlError(e) => {
                    error!("Error when loading book: {:?}", e);
                    Err(DocError::XmlError(e))
                }
                DocError::IOError(e) => {
                    error!("Error when loading book: {:?}", e);
                    Err(DocError::IOError(e))
                }
                DocError::InvalidEpub => {
                    error!("The provided epub:{} is Invalid", book_path);
                    Err(err)
                }
            },
        }
    }
}
