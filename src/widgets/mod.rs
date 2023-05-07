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
pub mod book_image;
mod books_flowbox;
pub mod library;
mod preferences;
mod window;

pub use window::BookxView;
pub use window::BookxWindow;

pub use book_image::BookImage;
pub use books_flowbox::BooksFlowBoxWidget;
pub use library::BookxLibraryWidget;
pub use preferences::BookxPreferencesWindow;
