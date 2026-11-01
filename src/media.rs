// SPDX-License-Identifier: GPL-2.0-or-later
//
// Copyright 2026 nico359

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Photo,
    Video,
}

#[derive(Clone, Debug)]
pub struct MediaItem {
    pub path: PathBuf,
    pub kind: MediaKind,
    pub modified: SystemTime,
}

fn media_kind(path: &Path) -> Option<MediaKind> {
    let extension = path.extension().and_then(|extension| extension.to_str())?;
    if extension.eq_ignore_ascii_case("jpg") || extension.eq_ignore_ascii_case("jpeg") {
        Some(MediaKind::Photo)
    } else if extension.eq_ignore_ascii_case("mp4") {
        Some(MediaKind::Video)
    } else {
        None
    }
}

/// All photos and videos in the two Camera directories, newest first.
pub fn scan_media() -> Vec<MediaItem> {
    let mut items = Vec::new();

    for directory in [photo_directory(), video_directory()].into_iter().flatten() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };

        for entry in entries.filter_map(|entry| entry.ok()) {
            let path = entry.path();
            let Some(kind) = media_kind(&path) else {
                continue;
            };
            let Ok(modified) = entry.metadata().and_then(|metadata| metadata.modified()) else {
                continue;
            };
            items.push(MediaItem {
                path,
                kind,
                modified,
            });
        }
    }

    items.sort_by(|left, right| right.modified.cmp(&left.modified));
    items
}

/// Open a media file with the system's default viewer or app chooser.
pub fn launch_external<F>(
    parent: &impl IsA<gtk::Window>,
    path: &Path,
    always_ask: bool,
    on_error: F,
) where
    F: Fn(String) + 'static,
{
    let file = gio::File::for_path(path);
    let launcher = gtk::FileLauncher::new(Some(&file));
    launcher.set_always_ask(always_ask);
    launcher.launch(Some(parent), None::<&gio::Cancellable>, move |result| {
        if let Err(error) = result {
            on_error(error.to_string());
        }
    });
}

/// The directory where stills are stored.  It is not created here.
pub fn photo_directory() -> Result<PathBuf, String> {
    let pictures = glib::user_special_dir(glib::UserDirectory::Pictures)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .ok_or_else(|| "no Pictures directory is configured".to_string())?;

    Ok(pictures.join("Camera"))
}

/// The directory where videos are stored.  It is not created here.
pub fn video_directory() -> Result<PathBuf, String> {
    let videos = glib::user_special_dir(glib::UserDirectory::Videos)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Videos")))
        .ok_or_else(|| "no Videos directory is configured".to_string())?;

    Ok(videos.join("Camera"))
}

fn timestamped_path(directory: &Path, prefix: &str, extension: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;

    let now = glib::DateTime::now_local().map_err(|error| error.to_string())?;
    let stamp = now
        .format("%Y%m%d_%H%M%S")
        .map_err(|error| error.to_string())?;

    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.subsec_millis())
        .unwrap_or(0);

    Ok(directory.join(format!("{prefix}_{stamp}_{millis:03}.{extension}")))
}

/// Build a unique path under the user's Pictures/Camera directory.
pub fn next_photo_path() -> Result<PathBuf, String> {
    timestamped_path(&photo_directory()?, "IMG", "jpg")
}

/// Build a unique path under the user's Videos/Camera directory.
pub fn next_video_path() -> Result<PathBuf, String> {
    timestamped_path(&video_directory()?, "VID", "mp4")
}

/// Newest photo or video in either Camera directory.
pub fn latest_media() -> Option<PathBuf> {
    scan_media().into_iter().next().map(|item| item.path)
}
