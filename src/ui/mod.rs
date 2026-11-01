// SPDX-License-Identifier: GPL-2.0-or-later
//
// Copyright 2026 nico359

mod camera_view;
mod gallery;
mod settings_dialog;

pub use camera_view::CameraView;
pub use gallery::present as present_gallery;
pub use settings_dialog::present as present_settings;

/// Register GTypes that are referenced from GtkBuilder templates.
pub fn init() {
    CameraView::ensure_type();
}
