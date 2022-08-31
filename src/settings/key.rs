// Bookx - key.rs
// Copyright (C) 2022  Anurag Dhadse <hi@anuragdhadse.com>
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

use strum_macros::*;

#[derive(Display, Debug, Clone, EnumString)]
#[strum(serialize_all = "kebab_case")]
pub enum Key {
    // Client Backend
    ApiLookupDomain,

    // User Interface
    DarkMode,
    Notifications,
    WindowWidth,
    WindowHeight,
    IsMaximized,

    BooksDir,
}