// SPDX-License-Identifier: GPL-2.0-or-later
//
// Copyright 2026 nico359

//! Typed access to the app's persistent settings.

use std::collections::HashMap;

use gtk::gio;
use gtk::gio::prelude::*;
use gtk::glib::variant::ToVariant;

const SCHEMA_ID: &str = "io.github.nico359.camera";

#[derive(Clone)]
pub struct AppSettings {
    settings: gio::Settings,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self::new()
    }
}

impl AppSettings {
    pub fn new() -> Self {
        Self {
            settings: gio::Settings::new(SCHEMA_ID),
        }
    }

    /// The raw settings object, for widgets that bind a property directly.
    pub fn gio(&self) -> &gio::Settings {
        &self.settings
    }

    pub fn open_with_dialog(&self) -> bool {
        self.settings.boolean("open-with-dialog")
    }

    pub fn jpeg_quality(&self) -> i32 {
        self.settings.int("jpeg-quality").clamp(70, 100)
    }

    pub fn set_jpeg_quality(&self, quality: i32) {
        let _ = self
            .settings
            .set_int("jpeg-quality", quality.clamp(70, 100));
    }

    /// The saved JPEG size for a camera, or `None` for its full resolution.
    pub fn photo_size(&self, camera_id: &str) -> Option<(i32, i32)> {
        let map = self
            .settings
            .get::<HashMap<String, String>>("photo-resolutions");
        map.get(camera_id).and_then(|value| parse_size(value))
    }

    /// Save a camera's JPEG size, or clear it to use the full resolution.
    pub fn set_photo_size(&self, camera_id: &str, size: Option<(i32, i32)>) {
        let mut map = self
            .settings
            .get::<HashMap<String, String>>("photo-resolutions");
        match size {
            Some((width, height)) => {
                map.insert(camera_id.to_string(), format!("{width}x{height}"));
            }
            None => {
                map.remove(camera_id);
            }
        }
        let _ = self
            .settings
            .set_value("photo-resolutions", &map.to_variant());
    }
}

fn parse_size(value: &str) -> Option<(i32, i32)> {
    let (width, height) = value.split_once('x')?;
    let width = width.parse::<i32>().ok()?;
    let height = height.parse::<i32>().ok()?;
    (width > 0 && height > 0).then_some((width, height))
}
