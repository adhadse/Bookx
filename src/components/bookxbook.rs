use epub::doc::EpubDoc;
use gettextrs::gettext;
use gtk::prelude::*;
use relm4::{
    gtk::{
        self,
        gdk_pixbuf::Pixbuf,
        glib::{self},
        prelude::*,
    },
    prelude::*,
    ComponentParts, ComponentSender, SimpleComponent,
};

use std::fs;
use std::io::Write;
use std::{cell::RefCell, path::PathBuf};

pub struct BookxBook {
    pub title: String,
    pub progress: f64,
    pub pixbuf: Pixbuf,
}

#[relm4_macros::component(pub)]
impl SimpleComponent for BookxBook {
    type Init = ();
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
                    set_width_request: 200,
                    set_height_request: 220,
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

    fn init(_: (), root: &Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let mut doc = EpubDoc::new("/home/adhadse/CalibreLibrary/Alex Xu/System Design Interview - An Insider's Guide (29)/System Design Interview - An Insider's Gui - Alex Xu.epub").unwrap();
        // TODO let cover_path = glib::user_cache_dir().join(book_id).join("cover.png");
        let cover_path = glib::user_cache_dir().join("cover.png");
        match doc.get_cover() {
            Some((cover_data, mime_type)) => {
                let mut f = fs::File::create(cover_path.clone()).unwrap();
                let resp = f.write_all(&cover_data);
            }
            None => {
                println!("cannot find cover")
            }
        };

        let model = BookxBook {
            title: String::from("System Design Interview - An Insider's Gui"),
            progress: 40.0,
            pixbuf: Pixbuf::from_file_at_scale(cover_path, 200, 220, true).unwrap(),
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }
}
