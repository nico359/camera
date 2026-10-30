// SPDX-License-Identifier: GPL-3.0-or-later
//
// Copyright 2026 nico359

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use gtk::glib;

/// Build a unique path under the user's Pictures/Camera directory.
pub fn next_photo_path() -> Result<PathBuf, String> {
    let pictures = glib::user_special_dir(glib::UserDirectory::Pictures)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .ok_or_else(|| "no Pictures directory is configured".to_string())?;

    let directory = pictures.join("Camera");
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;

    let now = glib::DateTime::now_local().map_err(|error| error.to_string())?;
    let stamp = now
        .format("%Y%m%d_%H%M%S")
        .map_err(|error| error.to_string())?;

    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.subsec_millis())
        .unwrap_or(0);

    Ok(directory.join(format!("IMG_{stamp}_{millis:03}.jpg")))
}
