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
mod book_manager;
mod library;
mod object_wrapper;
mod storage;
mod utils;

pub use book::Book;
pub use book_manager::BookAction;
pub use library::{BookxLibrary, BookxLibraryStatus};
pub use object_wrapper::ObjectWrapper;