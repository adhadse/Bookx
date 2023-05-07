// Bookx - mod.rs
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

mod book;
mod book_image;
mod book_object;
mod books_manager;
pub mod format;

pub use book::{BookxBook, BookxBookError};
pub use book_image::BookxBookImage;
pub use book_object::BookxBookObject;
pub use books_manager::{BookAction, BooksManager};
pub use format::{Format, FormatNotSupportedError};
