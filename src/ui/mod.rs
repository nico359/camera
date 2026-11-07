// SPDX-License-Identifier: GPL-2.0-or-later
//
// Copyright 2026 nico359

use std::path::Path;

mod camera_view;
pub(crate) mod gallery;
mod settings_dialog;
mod zoom_picture;

/// Load an image and crop its centre to a square texture of `size` px.
pub(crate) fn square_texture(path: &Path, size: i32) -> Option<gtk::gdk::Texture> {
    let (_, width, height) = gtk::gdk_pixbuf::Pixbuf::file_info(path)?;
    // Scale the shorter edge up to `size`, then crop the longer edge.
    let (target_width, target_height) = if width >= height {
        (100_000, size)
    } else {
        (size, 100_000)
    };
    let scaled =
        gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(path, target_width, target_height, true)
            .ok()?;
    let side = scaled.width().min(scaled.height());
    if side <= 0 {
        return None;
    }

    let square = scaled.new_subpixbuf(
        (scaled.width() - side) / 2,
        (scaled.height() - side) / 2,
        side,
        side,
    );
    let thumbnail = if side == size {
        square
    } else {
        square.scale_simple(size, size, gtk::gdk_pixbuf::InterpType::Bilinear)?
    };
    Some(gtk::gdk::Texture::for_pixbuf(&thumbnail))
}

pub use camera_view::CameraView;
pub use gallery::present as present_gallery;
pub use settings_dialog::present as present_settings;

/// Register GTypes that are referenced from GtkBuilder templates.
pub fn init() {
    CameraView::ensure_type();
}
