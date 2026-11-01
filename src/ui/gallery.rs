// SPDX-License-Identifier: GPL-2.0-or-later
//
// Copyright 2026 nico359

//! A small read-only media browser for the Camera folders.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use adw::prelude::*;
use gettextrs::gettext;
use gtk::gdk_pixbuf::Pixbuf;
use gtk::{gio, glib};

use crate::media::{self, MediaItem, MediaKind};
use crate::window::CameraWindow;

fn format_time(modified: SystemTime) -> String {
    let seconds = modified
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);

    glib::DateTime::from_unix_local(seconds)
        .ok()
        .and_then(|date| date.format("%Y-%m-%d %H:%M").ok())
        .map(|text| text.to_string())
        .unwrap_or_default()
}

fn file_name(item: &MediaItem) -> String {
    item.path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Media")
        .to_string()
}

fn build_tile(item: &MediaItem) -> gtk::FlowBoxChild {
    let media = gtk::Box::new(gtk::Orientation::Vertical, 0);
    media.set_height_request(100);
    media.set_hexpand(true);
    media.add_css_class("media-thumb");

    match item.kind {
        MediaKind::Photo => match Pixbuf::from_file_at_scale(&item.path, 240, 240, true) {
            Ok(pixbuf) => {
                let texture = gtk::gdk::Texture::for_pixbuf(&pixbuf);
                let picture = gtk::Picture::for_paintable(&texture);
                picture.set_content_fit(gtk::ContentFit::Cover);
                picture.set_can_shrink(true);
                picture.set_hexpand(true);
                picture.set_vexpand(true);
                media.append(&picture);
            }
            Err(_) => {
                let icon = gtk::Image::from_icon_name("image-missing-symbolic");
                icon.set_pixel_size(40);
                icon.set_halign(gtk::Align::Center);
                icon.set_valign(gtk::Align::Center);
                icon.set_hexpand(true);
                icon.set_vexpand(true);
                media.append(&icon);
            }
        },
        MediaKind::Video => {
            let icon = gtk::Image::from_icon_name("camera-video-symbolic");
            icon.set_pixel_size(48);
            icon.set_halign(gtk::Align::Center);
            icon.set_valign(gtk::Align::Center);
            icon.set_hexpand(true);
            icon.set_vexpand(true);
            media.append(&icon);
        }
    }

    let tile = gtk::Box::new(gtk::Orientation::Vertical, 4);
    tile.add_css_class("media-tile");
    tile.append(&media);

    let label = gtk::Label::new(Some(&format_time(item.modified)));
    label.add_css_class("caption");
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_max_width_chars(16);
    tile.append(&label);

    let child = gtk::FlowBoxChild::new();
    child.set_child(Some(&tile));
    child.set_tooltip_text(Some(&file_name(item)));
    child
}

fn show_viewer(
    title: &gtk::Label,
    content: &gtk::Box,
    current: &Rc<RefCell<Option<PathBuf>>>,
    item: &MediaItem,
) {
    title.set_label(&file_name(item));
    *current.borrow_mut() = Some(item.path.clone());

    while let Some(child) = content.first_child() {
        content.remove(&child);
    }

    match item.kind {
        MediaKind::Photo => {
            let picture = gtk::Picture::for_filename(&item.path);
            picture.set_content_fit(gtk::ContentFit::Contain);
            picture.set_can_shrink(true);
            picture.set_hexpand(true);
            picture.set_vexpand(true);
            content.append(&picture);
        }
        MediaKind::Video => {
            let placeholder = gtk::Box::new(gtk::Orientation::Vertical, 8);
            placeholder.set_halign(gtk::Align::Center);
            placeholder.set_valign(gtk::Align::Center);
            placeholder.set_hexpand(true);
            placeholder.set_vexpand(true);

            let icon = gtk::Image::from_icon_name("camera-video-symbolic");
            icon.set_pixel_size(96);
            let label = gtk::Label::new(Some(&gettext("Video")));
            label.add_css_class("title-2");
            placeholder.append(&icon);
            placeholder.append(&label);
            content.append(&placeholder);
        }
    }
}

pub fn present(parent: &CameraWindow) {
    let items = media::scan_media();

    let flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .column_spacing(6)
        .row_spacing(6)
        .min_children_per_line(2)
        .max_children_per_line(3)
        .valign(gtk::Align::Start)
        .build();
    flow.set_margin_top(12);
    flow.set_margin_bottom(12);
    flow.set_margin_start(12);
    flow.set_margin_end(12);

    if items.is_empty() {
        let empty = gtk::Label::new(Some(&gettext("No photos or videos yet")));
        empty.set_wrap(true);
        empty.set_justify(gtk::Justification::Center);
        empty.set_margin_top(48);
        empty.add_css_class("dim-label");
        let child = gtk::FlowBoxChild::new();
        child.set_child(Some(&empty));
        flow.append(&child);
    } else {
        for item in &items {
            flow.append(&build_tile(item));
        }
    }

    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .vexpand(true)
        .build();
    scroller.set_child(Some(&flow));

    let grid_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    grid_page.append(&scroller);

    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.add_named(&grid_page, Some("grid"));

    let viewer_title = gtk::Label::new(None);
    viewer_title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    viewer_title.set_hexpand(true);
    viewer_title.set_xalign(0.0);
    viewer_title.add_css_class("title-4");

    let back = gtk::Button::from_icon_name("go-previous-symbolic");
    back.set_tooltip_text(Some(&gettext("Back")));
    back.add_css_class("flat");

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.set_margin_top(12);
    header.set_margin_bottom(8);
    header.set_margin_start(12);
    header.set_margin_end(12);
    header.append(&back);
    header.append(&viewer_title);

    let viewer_content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    viewer_content.set_hexpand(true);
    viewer_content.set_vexpand(true);
    viewer_content.set_margin_start(12);
    viewer_content.set_margin_end(12);

    let external = gtk::Button::with_label(&gettext("Open Externally"));
    external.set_hexpand(true);
    external.set_margin_top(8);
    external.set_margin_bottom(12);
    external.set_margin_start(12);
    external.set_margin_end(12);

    let viewer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    viewer.set_vexpand(true);
    viewer.append(&header);
    viewer.append(&viewer_content);
    viewer.append(&external);
    stack.add_named(&viewer, Some("viewer"));
    stack.set_visible_child_name("grid");

    let toast = adw::ToastOverlay::new();
    toast.set_child(Some(&stack));

    let items = Rc::new(items);
    let items_cb = Rc::clone(&items);
    let stack_cb = stack.clone();
    let title_cb = viewer_title.clone();
    let content_cb = viewer_content.clone();
    let current = Rc::new(RefCell::new(None));
    let current_cb = Rc::clone(&current);
    flow.connect_child_activated(move |_, child| {
        let Some(item) = items_cb.get(child.index() as usize) else {
            return;
        };
        show_viewer(&title_cb, &content_cb, &current_cb, item);
        stack_cb.set_visible_child_name("viewer");
    });

    let stack_cb = stack.clone();
    back.connect_clicked(move |_| stack_cb.set_visible_child_name("grid"));

    let current_cb = Rc::clone(&current);
    let parent_cb = parent.downgrade();
    let toast_cb = toast.clone();
    external.connect_clicked(move |_| {
        let Some(path) = current_cb.borrow().clone() else {
            return;
        };
        let Some(parent) = parent_cb.upgrade() else {
            return;
        };
        let settings = gio::Settings::new("io.github.nico359.camera");
        let ask = settings.boolean("open-with-dialog");
        let toast = toast_cb.clone();
        media::launch_external(&parent, &path, ask, move |error| {
            toast.add_toast(adw::Toast::new(&format!(
                "{}: {error}",
                gettext("Could not open media")
            )));
        });
    });

    let dialog = adw::Dialog::builder()
        .title(&gettext("Gallery"))
        .content_width(360)
        .content_height(700)
        .build();
    dialog.set_child(Some(&toast));
    dialog.present(Some(parent));
}
