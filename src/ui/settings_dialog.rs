// SPDX-License-Identifier: GPL-2.0-or-later
//
// Copyright 2026 nico359

//! The window's Settings dialog: photo resolution, JPEG quality and the
//! media-open preference.  These are deliberately separate from the controls
//! that are changed while shooting (camera, flash, zoom, focus).

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gettextrs::gettext;
use gtk::StringList;

use crate::settings::{AppSettings, JpegQuality};
use crate::window::CameraWindow;

fn megapixels_label(width: i32, height: i32) -> String {
    let mp = (width as f64 * height as f64) / 1_000_000.0;
    if mp >= 10.0 {
        format!("{mp:.0} MP")
    } else {
        format!("{mp:.1} MP")
    }
}

fn aspect_label(width: i32, height: i32) -> Option<&'static str> {
    let ratio = width as f32 / height as f32;
    [
        (4.0 / 3.0, "4:3"),
        (16.0 / 9.0, "16:9"),
        (3.0 / 2.0, "3:2"),
        (1.0, "1:1"),
        (5.0 / 4.0, "5:4"),
    ]
    .into_iter()
    .find(|(candidate, _)| (ratio - candidate).abs() < 0.02)
    .map(|(_, label)| label)
}

fn resolution_subtitle(camera: &str, size: Option<(i32, i32)>) -> String {
    match size.and_then(|(width, height)| aspect_label(width, height)) {
        Some(aspect) => format!("{camera} · {aspect}"),
        None => camera.to_string(),
    }
}

fn quality_label(profile: JpegQuality) -> String {
    match profile {
        JpegQuality::Original => gettext("Original"),
        JpegQuality::Balanced => gettext("Balanced"),
        JpegQuality::Medium => gettext("Medium"),
        JpegQuality::Small => gettext("Small"),
    }
}

pub fn present(window: &CameraWindow) {
    let view = window.camera_view();
    let settings = AppSettings::new();

    let page = adw::PreferencesPage::new();

    let photo_group = adw::PreferencesGroup::new();
    photo_group.set_title(&gettext("Photos"));

    let sizes = view.photo_sizes();
    let camera_label = view
        .current_camera()
        .map(|camera| camera.label)
        .unwrap_or_else(|| gettext("Current camera"));

    let resolution = adw::ComboRow::new();
    resolution.set_title(&gettext("Photo Resolution"));

    let saved_size = view
        .current_camera_id()
        .and_then(|id| settings.photo_size(&id));
    let selected = saved_size
        .and_then(|size| sizes.iter().position(|candidate| *candidate == size))
        .unwrap_or(0) as u32;
    let initial_size = sizes.get(selected as usize).copied();
    resolution.set_subtitle(&resolution_subtitle(&camera_label, initial_size));

    if sizes.is_empty() {
        let model = StringList::new(&[]);
        model.append(&gettext("No photo sizes available"));
        resolution.set_model(Some(&model));
        resolution.set_selected(0);
        resolution.set_subtitle(&gettext("No camera available"));
        resolution.set_sensitive(false);
    } else {
        let model = StringList::new(&[]);
        for (width, height) in &sizes {
            model.append(&format!(
                "{} ({} × {})",
                megapixels_label(*width, *height),
                width,
                height
            ));
        }
        resolution.set_model(Some(&model));
        resolution.set_selected(selected);
        resolution.set_sensitive(view.ready() && !view.is_recording());

        let previous = Rc::new(Cell::new(selected));
        let updating = Rc::new(Cell::new(false));
        let previous_cb = Rc::clone(&previous);
        let updating_cb = Rc::clone(&updating);
        let view_cb = view.clone();
        let settings_cb = settings.clone();
        let sizes_cb = sizes.clone();
        let camera_label_cb = camera_label.clone();
        let weak = window.downgrade();
        resolution.connect_selected_notify(move |row| {
            if updating_cb.get() {
                return;
            }
            let index = row.selected() as usize;
            let Some(size) = sizes_cb.get(index).copied() else {
                return;
            };

            match view_cb.set_photo_size(size.0, size.1) {
                Ok(()) => {
                    if let Some(id) = view_cb.current_camera_id() {
                        let full_resolution = sizes_cb.first().copied() == Some(size);
                        settings_cb
                            .set_photo_size(&id, if full_resolution { None } else { Some(size) });
                    }
                    row.set_subtitle(&resolution_subtitle(&camera_label_cb, Some(size)));
                    if let Some(window) = weak.upgrade() {
                        window.update_preview_aspect();
                    }
                    previous_cb.set(row.selected());
                }
                Err(error) => {
                    updating_cb.set(true);
                    row.set_selected(previous_cb.get());
                    updating_cb.set(false);
                    if let Some(window) = weak.upgrade() {
                        window.show_toast(&format!(
                            "{}: {error}",
                            gettext("Could not change photo resolution")
                        ));
                    }
                }
            }
        });
    }
    photo_group.add(&resolution);

    let quality = adw::ComboRow::new();
    quality.set_title(&gettext("JPEG Quality"));
    quality.set_subtitle(&gettext("Lower tiers use less space"));
    let quality_model = StringList::new(&[]);
    for profile in JpegQuality::ALL {
        quality_model.append(&quality_label(profile));
    }
    quality.set_model(Some(&quality_model));
    let current_profile = settings.jpeg_quality_profile();
    let quality_selected = JpegQuality::ALL
        .iter()
        .position(|profile| *profile == current_profile)
        .unwrap_or(1) as u32;
    quality.set_selected(quality_selected);
    let settings_cb = settings.clone();
    let view_cb = view.clone();
    quality.connect_selected_notify(move |row| {
        let Some(profile) = JpegQuality::ALL.get(row.selected() as usize).copied() else {
            return;
        };
        settings_cb.set_jpeg_quality_profile(profile);
        view_cb.set_jpeg_quality(profile.hal_value());
    });
    photo_group.add(&quality);

    page.add(&photo_group);

    let composition_group = adw::PreferencesGroup::new();
    composition_group.set_title(&gettext("Composition"));

    let grid = adw::SwitchRow::new();
    grid.set_title(&gettext("Show 3×3 Grid"));
    grid.set_subtitle(&gettext("Composition guide over the preview"));
    grid.set_active(settings.show_grid());
    settings.gio().bind("show-grid", &grid, "active").build();
    composition_group.add(&grid);
    page.add(&composition_group);

    let video_group = adw::PreferencesGroup::new();
    video_group.set_title(&gettext("Video"));

    let video_resolution = adw::ComboRow::new();
    video_resolution.set_title(&gettext("Video Resolution"));
    video_resolution.set_subtitle(&gettext("4K may be unstable"));
    let video_model = StringList::new(&[]);
    video_model.append(&gettext("1080p"));
    video_model.append(&gettext("4K"));
    video_resolution.set_model(Some(&video_model));
    let video_selected = if settings.video_size().0 >= 3000 {
        1
    } else {
        0
    };
    video_resolution.set_selected(video_selected);
    video_resolution.set_sensitive(!view.is_recording());
    let weak = window.downgrade();
    let view_cb = view.clone();
    let settings_cb = settings.clone();
    video_resolution.connect_selected_notify(move |row| {
        let (width, height) = if row.selected() == 1 {
            (3840, 2160)
        } else {
            (1920, 1080)
        };
        settings_cb.set_video_size(width, height);
        if let Some(window) = weak.upgrade() {
            if let Err(error) = view_cb.apply_video_settings() {
                window.show_toast(&format!(
                    "{}: {error}",
                    gettext("Could not change video resolution")
                ));
            }
            window.update_mode_ui();
            window.update_preview_aspect();
        }
    });
    video_group.add(&video_resolution);

    let stabilization = adw::SwitchRow::new();
    stabilization.set_title(&gettext("Video Stabilization (EIS)"));
    stabilization.set_subtitle(&gettext("Off gives a wider field of view"));
    stabilization.set_active(settings.video_stabilization());
    stabilization.set_sensitive(!view.is_recording());
    let weak = window.downgrade();
    let view_cb = view.clone();
    let settings_cb = settings.clone();
    stabilization.connect_active_notify(move |row| {
        settings_cb.set_video_stabilization(row.is_active());
        if let Some(window) = weak.upgrade() {
            if let Err(error) = view_cb.apply_video_settings() {
                window.show_toast(&format!(
                    "{}: {error}",
                    gettext("Could not change video stabilization")
                ));
            }
            window.update_mode_ui();
            window.update_preview_aspect();
        }
    });
    video_group.add(&stabilization);
    page.add(&video_group);

    let media_group = adw::PreferencesGroup::new();
    media_group.set_title(&gettext("Media"));

    let open_with = adw::SwitchRow::new();
    open_with.set_title(&gettext("Ask Which App to Open Media"));
    open_with.set_subtitle(&gettext(
        "Show the app chooser instead of the default viewer",
    ));
    open_with.set_active(settings.open_with_dialog());
    settings
        .gio()
        .bind("open-with-dialog", &open_with, "active")
        .build();
    media_group.add(&open_with);
    page.add(&media_group);

    let dialog = adw::PreferencesDialog::builder()
        .title(&gettext("Settings"))
        .content_width(360)
        .search_enabled(false)
        .build();
    dialog.add(&page);
    dialog.present(Some(window));
}
