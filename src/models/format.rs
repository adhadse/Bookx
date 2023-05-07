// Bookx - utils.rs
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

use crate::models::Format::EPUB;
use core::cmp::Ordering;
use gtk::{
    gio,
    glib::{self, Enum},
    prelude::*,
};
use log::debug;
use std::fmt::{self, format, Formatter};
use std::str::FromStr;
use strum_macros::EnumIter;

#[derive(Debug)]
pub struct FormatNotSupportedError {
    details: String,
}

impl FormatNotSupportedError {
    fn new(msg: &str) -> FormatNotSupportedError {
        FormatNotSupportedError {
            details: msg.to_string(),
        }
    }
}

impl fmt::Display for FormatNotSupportedError {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{}", self.details)
    }
}

// Formats that gets accepted as EBook by Bookx
// TODO: remove EnumIter is needed
#[derive(Debug, Enum, Clone, Copy, EnumIter)]
#[enum_type(name = "Format")]
pub enum Format {
    EPUB,
}

impl Default for Format {
    fn default() -> Self {
        Format::Epub
    }
}

impl Format {
    pub fn iterator() -> impl Iterator<Item = Format> {
        [EPUB].iter().copied()
    }
}

// impl ToValue for Format {
//     fn to_value(&self) -> Value {
//         self.to_value()
//     }

// fn value_type(&self) -> Type {
//     Format
// }
// }

impl FromStr for Format {
    type Err = FormatNotSupportedError;
    fn from_str(mime: &str) -> Result<Self, Self::Err> {
        match mime {
            String::from("application/epub+zip") => Ok(EPUB),
            // String::from("application/x-mobipocket-ebook") => Format::MOBI,
            // String::from("application/vnd.amazon.mobi8-ebook") => Format::KINDLE,
            // String::from("application/x-mobi8-ebook") => Format::KINDLEALIAS,
            // String::from("text/fb2+xml") => Format::FB2,
            _ => Err(FormatNotSupportedError {
                details: format!("{} is not supported format for book", mime),
            }),
        }
    }
}

impl Format {
    // pub fn get_filter() -> gtk::FileFilter {
    //     let filter = gtk::FileFilter::new();
    //     filter.set_property("name", &String::from("Supported book files"));
    //
    //     for format in Format::iterator() {
    //         filter.add_mime_type(&format.get_mime());
    //     }
    //     filter
    // }
    pub fn get_mime(self) -> String {
        match self {
            EPUB => String::from("application/epub+zip"),
            // Format::MOBI => String::from("application/x-mobipocket-ebook"),
            // Format::KINDLE => String::from("application/vnd.amazon.mobi8-ebook"),
            // Format::KINDLEALIAS => String::from("application/x-mobi8-ebook"),
            // Format::FB2 => String::from("text/fb2+xml"),
        }
    }

    // formats that allow user to add annotations without warning
    pub fn can_annotate() -> Vec<Format> {
        Vec::from([
            EPUB,
            // Format::MOBI,
            // Format::KINDLE,
            // Format::KINDLEALIAS,
        ])
    }
}
