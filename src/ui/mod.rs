// SPDX-License-Identifier: GPL-3.0-or-later
//
// Copyright 2026 nico359

mod camera_view;

pub use camera_view::CameraView;

/// Register GTypes that are referenced from GtkBuilder templates.
pub fn init() {
    CameraView::ensure_type();
}
