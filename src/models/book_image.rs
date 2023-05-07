// Bookx - book_image.rs
// This file implements BookxBookImage and is responsible for getting the cover image of book
// from epub or fetching it from the internet
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

use std::path::PathBuf;

use anyhow::Result;
use crypto::{digest::Digest, sha1::Sha1};
use gtk::glib;
// use isahc::prelude::*;
use crate::config;
use epub::doc::{DocError, EpubDoc};
use log::info;
use once_cell::sync::Lazy;
use std::{fs, io::Write};

pub static CACHE_DIR: Lazy<PathBuf> = Lazy::new(|| glib::user_cache_dir().join(config::PKGNAME));

pub struct BookxBookImage {
    pub path: PathBuf,
    pub cache: PathBuf,
}

impl BookxBookImage {
    pub fn new(path: PathBuf) -> Self {
        let identifier = Self.get_identifier();
        let cache = BookxBookImage::get_cache_of(identifier.clone().as_str());
        Self { path, cache }
    }

    pub fn get_cache_of(path: &str) -> PathBuf {
        let mut hasher = Sha1::new();
        hasher.input_str(path);
        let cache: PathBuf = CACHE_DIR.join(&hasher.result_str()).join(".png");
        cache
    }

    pub fn exists(&self) -> bool {
        self.cache.exists()
    }

    async fn get_cover_or_download(self, pathbuf: &PathBuf) -> Result<()> {
        let mut doc = EpubDoc::new(pathbuf).unwrap();
        let cover_data = doc.get_cover();
        if let Ok(cover_data) = cover_data {
            let mut f = fs::File::create(self.cache).unwrap();
            let resp = f.write_all(&cover_data);
            Ok(())
        } else {
            // if cover isn't in epub then search on internet and download
            Self.download();
            Ok(())
        }
    }

    async fn download(&self) -> Result<()> {
        // if let Ok(mut resp) = isahc::get_async(&self.url.to_string()).await {
        //     info!(
        //         "Downloading preview image {:#?} into {:#?}",
        //         self.path, self.cache
        //     );
        //     let body = resp.bytes().await?;
        //     async_std::fs::write(self.cache.clone(), body).await?;
        // }
        Ok(())
    }

    fn get_identifier(&self) -> &String {
        let doc = EpubDoc::new(self.path.to_str()).unwrap();
        let title = doc.metadata.get("title").unwrap().get(0).unwrap();
        let identifier = match doc.metadata.get("identifier") {
            Ok(identifier_list) => identifier_list.get(0).unwrap(),
            Err(e) => title,
        };
        identifier
    }
}
