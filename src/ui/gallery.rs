// SPDX-License-Identifier: GPL-2.0-or-later
//
// Copyright 2026 nico359

//! Full-screen swipeable media viewer for the Camera folders.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gdk, gio};

use crate::media::{self, MediaKind};
use crate::window::CameraWindow;

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Media")
        .to_string()
}

pub fn present(parent: &CameraWindow) {
    let items = Rc::new(media::scan_media());

    let window = adw::Window::builder()
        .transient_for(parent)
        .modal(true)
        .default_width(360)
        .default_height(760)
        .build();
    window.set_application(parent.application().as_ref());
    window.set_title(Some(&gettext("Gallery")));

    let header = adw::HeaderBar::new();

    let close = gtk::Button::from_icon_name("window-close-symbolic");
    close.set_tooltip_text(Some(&gettext("Close")));
    close.add_css_class("flat");
    header.pack_start(&close);

    let title = adw::WindowTitle::new(&gettext("Gallery"), "");
    header.set_title_widget(Some(&title));

    let external = gtk::Button::from_icon_name("external-link-symbolic");
    external.set_tooltip_text(Some(&gettext("Open Externally")));
    external.add_css_class("flat");
    header.pack_end(&external);

    let carousel = adw::Carousel::new();
    carousel.set_hexpand(true);
    carousel.set_vexpand(true);
    carousel.set_allow_mouse_drag(true);
    carousel.set_interactive(true);
    carousel.set_focusable(true);

    let mut video_widgets = Vec::new();
    let mut pages = Vec::new();

    if items.is_empty() {
        let empty = gtk::Label::new(Some(&gettext("No photos or videos yet")));
        empty.add_css_class("title-2");
        empty.add_css_class("dim-label");
        empty.set_halign(gtk::Align::Center);
        empty.set_valign(gtk::Align::Center);
        empty.set_hexpand(true);
        empty.set_vexpand(true);

        let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        page.add_css_class("gallery-page");
        page.set_hexpand(true);
        page.set_vexpand(true);
        page.append(&empty);
        carousel.append(&page);
        pages.push(page);
    } else {
        for item in items.iter() {
            let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
            page.add_css_class("gallery-page");
            page.set_hexpand(true);
            page.set_vexpand(true);

            match item.kind {
                MediaKind::Photo => {
                    let picture = gtk::Picture::for_filename(&item.path);
                    picture.set_content_fit(gtk::ContentFit::Contain);
                    picture.set_can_shrink(true);
                    picture.set_hexpand(true);
                    picture.set_vexpand(true);
                    page.append(&picture);
                    video_widgets.push(None);
                }
                MediaKind::Video => {
                    let file = gio::File::for_path(&item.path);
                    let video = gtk::Video::for_file(Some(&file));
                    video.set_autoplay(false);
                    video.set_loop(false);
                    video.set_hexpand(true);
                    video.set_vexpand(true);
                    video.add_css_class("gallery-video");
                    page.append(&video);
                    video_widgets.push(Some(video));
                }
            }

            carousel.append(&page);
            pages.push(page);
        }
    }

    let videos = Rc::new(video_widgets);
    let pages = Rc::new(pages);
    let dots = adw::CarouselIndicatorDots::new();
    dots.set_carousel(Some(&carousel));
    dots.set_margin_top(8);
    dots.set_margin_bottom(12);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("gallery-viewer");
    content.append(&carousel);
    content.append(&dots);

    let toast = adw::ToastOverlay::new();
    toast.set_child(Some(&content));

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&toast));
    window.set_content(Some(&toolbar));

    let current = Rc::new(RefCell::new(None));
    if let Some(item) = items.first() {
        title.set_subtitle(&file_name(&item.path));
        *current.borrow_mut() = Some(item.path.clone());
    }

    let items_cb = Rc::clone(&items);
    let videos_cb = Rc::clone(&videos);
    let title_cb = title.clone();
    let current_cb = Rc::clone(&current);
    carousel.connect_page_changed(move |_, index| {
        let index = index as usize;
        if let Some(item) = items_cb.get(index) {
            title_cb.set_subtitle(&file_name(&item.path));
            *current_cb.borrow_mut() = Some(item.path.clone());
        }

        for (page, video) in videos_cb.iter().enumerate() {
            if page == index {
                continue;
            }
            if let Some(video) = video {
                if let Some(stream) = video.media_stream() {
                    stream.pause();
                }
            }
        }
    });

    let window_cb = window.downgrade();
    close.connect_clicked(move |_| {
        if let Some(window) = window_cb.upgrade() {
            window.close();
        }
    });

    let pages_cb = Rc::clone(&pages);
    let videos_cb = Rc::clone(&videos);
    let carousel_cb = carousel.clone();
    let window_cb = window.downgrade();
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed(move |_, keyval, _, _| {
        if keyval == gdk::Key::Escape {
            if let Some(window) = window_cb.upgrade() {
                window.close();
            }
            return gtk::glib::Propagation::Stop;
        }

        let current = carousel_cb.position().round().max(0.0) as usize;

        if matches!(keyval, gdk::Key::space | gdk::Key::Return) {
            if let Some(Some(video)) = videos_cb.get(current) {
                if let Some(stream) = video.media_stream() {
                    if stream.is_playing() {
                        stream.pause();
                    } else {
                        stream.play();
                    }
                }
            }
            return gtk::glib::Propagation::Stop;
        }
        let target = if keyval == gdk::Key::Left {
            current.checked_sub(1)
        } else if keyval == gdk::Key::Right {
            (current + 1 < pages_cb.len()).then_some(current + 1)
        } else {
            None
        };

        let Some(target) = target else {
            return gtk::glib::Propagation::Proceed;
        };
        carousel_cb.scroll_to(&pages_cb[target], true);
        gtk::glib::Propagation::Stop
    });
    window.add_controller(keys);

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

    let carousel_focus = carousel.clone();
    window.connect_map(move |_| {
        carousel_focus.grab_focus();
    });

    window.present();
    window.fullscreen();
}
