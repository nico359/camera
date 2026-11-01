// SPDX-License-Identifier: GPL-2.0-or-later
//
// Copyright 2026 nico359

//! Swipeable media viewer, pushed as a page inside the camera window.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
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

/// One video page. The media stream is created only while the page is the
/// current one, so swiping never has several decoders running at once.
struct VideoPage {
    root: gtk::Overlay,
    picture: gtk::Picture,
    button: gtk::Button,
    spinner: gtk::Spinner,
    path: PathBuf,
    media: RefCell<Option<gtk::MediaFile>>,
    realized: Cell<bool>,
}

impl VideoPage {
    fn new(path: &Path) -> Rc<Self> {
        let root = gtk::Overlay::new();
        root.add_css_class("gallery-page");
        root.set_hexpand(true);
        root.set_vexpand(true);

        let picture = gtk::Picture::new();
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_can_shrink(true);
        picture.set_hexpand(true);
        picture.set_vexpand(true);
        picture.add_css_class("gallery-video");
        root.set_child(Some(&picture));

        let spinner = gtk::Spinner::new();
        spinner.set_halign(gtk::Align::Center);
        spinner.set_valign(gtk::Align::Center);
        spinner.set_size_request(48, 48);
        spinner.set_visible(false);
        root.add_overlay(&spinner);

        let button = gtk::Button::from_icon_name("media-playback-start-symbolic");
        button.add_css_class("circular");
        button.add_css_class("osd");
        button.add_css_class("gallery-play");
        button.set_halign(gtk::Align::Center);
        button.set_valign(gtk::Align::Center);
        button.set_size_request(72, 72);
        button.set_visible(false);
        root.add_overlay(&button);

        let page = Rc::new(Self {
            root,
            picture,
            button,
            spinner,
            path: path.to_path_buf(),
            media: RefCell::new(None),
            realized: Cell::new(false),
        });

        let weak = Rc::downgrade(&page);
        page.button.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.toggle();
            }
        });

        // A GtkMediaFile only produces video frames once it is realized with
        // the widget's GdkSurface; GtkVideo does this internally, a plain
        // GtkPicture does not.
        let weak = Rc::downgrade(&page);
        page.root.connect_map(move |_| {
            if let Some(page) = weak.upgrade() {
                page.realize_stream();
            }
        });

        let weak = Rc::downgrade(&page);
        page.root.connect_unmap(move |_| {
            if let Some(page) = weak.upgrade() {
                page.unrealize_stream();
            }
        });

        page
    }

    fn realize_stream(self: &Rc<Self>) {
        if self.realized.get() {
            return;
        }
        let stream = self.media.borrow().as_ref().cloned();
        let Some(stream) = stream else {
            return;
        };
        let Some(surface) = self.root.native().and_then(|native| native.surface()) else {
            return;
        };
        stream.realize(&surface);
        self.realized.set(true);
    }

    fn unrealize_stream(&self) {
        if !self.realized.get() {
            return;
        }
        if let Some(stream) = self.media.borrow().as_ref() {
            stream.pause();
            if let Some(surface) = self.root.native().and_then(|native| native.surface()) {
                stream.unrealize(&surface);
            }
            self.realized.set(false);
        }
    }

    /// Start loading the paused first frame. Called when the page becomes the
    /// current carousel page.
    fn load(self: &Rc<Self>) {
        if self.media.borrow().is_some() {
            return;
        }

        self.button.set_visible(true);
        self.update_icon(false);
        self.spinner.set_visible(false);

        let stream = gtk::MediaFile::for_filename(&self.path);
        stream.set_loop(false);

        let weak = Rc::downgrade(self);
        stream.connect_prepared_notify(move |stream| {
            let Some(page) = weak.upgrade() else {
                return;
            };
            if stream.is_prepared() {
                page.spinner.stop();
                page.spinner.set_visible(false);
                page.button.set_visible(stream.error().is_none());
                page.update_icon(stream.is_playing());
            }
        });

        let weak = Rc::downgrade(self);
        stream.connect_playing_notify(move |stream| {
            if let Some(page) = weak.upgrade() {
                if stream.is_playing() {
                    page.spinner.stop();
                    page.spinner.set_visible(false);
                }
                page.update_icon(stream.is_playing());
            }
        });

        let weak = Rc::downgrade(self);
        stream.connect_error_notify(move |stream| {
            if let Some(page) = weak.upgrade() {
                if stream.error().is_some() {
                    page.spinner.stop();
                    page.spinner.set_visible(false);
                    page.button.set_visible(false);
                }
            }
        });

        stream.connect_ended_notify(move |stream| {
            if stream.is_ended() {
                stream.seek(0);
            }
        });

        self.picture.set_paintable(Some(&stream));
        *self.media.borrow_mut() = Some(stream);
        self.realize_stream();
    }

    /// Stop playback and release the decoder when the page is left.
    fn unload(&self) {
        self.unrealize_stream();
        if let Some(stream) = self.media.borrow_mut().take() {
            stream.pause();
            self.picture.set_paintable(None::<&gdk::Paintable>);
        }
        self.spinner.stop();
        self.spinner.set_visible(false);
        self.button.set_visible(false);
    }

    fn toggle(self: &Rc<Self>) {
        let stream = self.media.borrow().as_ref().cloned();
        let Some(stream) = stream else {
            self.load();
            return;
        };
        if stream.is_playing() {
            stream.pause();
        } else {
            if !stream.is_prepared() {
                self.spinner.set_visible(true);
                self.spinner.start();
            }
            stream.play();
        }
    }

    fn update_icon(&self, playing: bool) {
        if playing {
            self.button.set_icon_name("media-playback-pause-symbolic");
            self.button.set_tooltip_text(Some(&gettext("Pause")));
        } else {
            self.button.set_icon_name("media-playback-start-symbolic");
            self.button.set_tooltip_text(Some(&gettext("Play")));
        }
    }
}

fn page_box() -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    page.add_css_class("gallery-page");
    page.set_hexpand(true);
    page.set_vexpand(true);
    page
}

pub fn present(parent: &CameraWindow) {
    let navigation = parent.navigation_view();
    if navigation.find_page("gallery").is_some() {
        return;
    }

    let items = Rc::new(media::scan_media());

    let header = adw::HeaderBar::new();

    let title = adw::WindowTitle::new(&gettext("Gallery"), "");
    header.set_title_widget(Some(&title));

    let external = gtk::Button::from_icon_name("window-new-symbolic");
    external.set_tooltip_text(Some(&gettext("Open Externally")));
    external.add_css_class("flat");
    header.pack_end(&external);

    let carousel = adw::Carousel::new();
    carousel.set_hexpand(true);
    carousel.set_vexpand(true);
    carousel.set_allow_mouse_drag(true);
    carousel.set_allow_long_swipes(true);
    carousel.set_interactive(true);
    carousel.set_focusable(true);

    let mut videos = Vec::new();
    let mut pages: Vec<gtk::Widget> = Vec::new();

    if items.is_empty() {
        let empty = gtk::Label::new(Some(&gettext("No photos or videos yet")));
        empty.add_css_class("title-2");
        empty.add_css_class("dim-label");
        empty.set_halign(gtk::Align::Center);
        empty.set_valign(gtk::Align::Center);
        empty.set_hexpand(true);
        empty.set_vexpand(true);

        let page = page_box();
        page.append(&empty);
        carousel.append(&page);
        pages.push(page.upcast());
        videos.push(None);
    } else {
        for item in items.iter() {
            match item.kind {
                MediaKind::Photo => {
                    let page = page_box();
                    let picture = gtk::Picture::for_filename(&item.path);
                    picture.set_content_fit(gtk::ContentFit::Contain);
                    picture.set_can_shrink(true);
                    picture.set_hexpand(true);
                    picture.set_vexpand(true);
                    page.append(&picture);
                    carousel.append(&page);
                    pages.push(page.upcast());
                    videos.push(None);
                }
                MediaKind::Video => {
                    let video = VideoPage::new(&item.path);
                    carousel.append(&video.root);
                    pages.push(video.root.clone().upcast());
                    videos.push(Some(video));
                }
            }
        }
    }

    let videos = Rc::new(videos);
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

    let page = adw::NavigationPage::new(&toolbar, &gettext("Gallery"));
    page.set_tag(Some("gallery"));

    let current = Rc::new(RefCell::new(None));

    // Load the first page's video (if any) after the carousel is ready.
    let first = carousel.position().round().max(0.0) as usize;
    if let Some(Some(video)) = videos.get(first) {
        video.load();
    }
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

        // Only the visible page keeps a decoder; everything else is released
        // so fast swipes over videos stay light.
        for (page, video) in videos_cb.iter().enumerate() {
            let Some(video) = video else {
                continue;
            };
            if page == index {
                video.load();
            } else if video.media.borrow().is_some() {
                video.unload();
            }
        }
    });

    let pages_cb = Rc::clone(&pages);
    let videos_cb = Rc::clone(&videos);
    let carousel_cb = carousel.clone();
    let navigation_cb = navigation.downgrade();
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed(move |_, keyval, _, _| {
        if keyval == gdk::Key::Escape {
            if let Some(navigation) = navigation_cb.upgrade() {
                navigation.pop();
            }
            return gtk::glib::Propagation::Stop;
        }

        let current = carousel_cb.position().round().max(0.0) as usize;

        if matches!(keyval, gdk::Key::space | gdk::Key::Return) {
            if let Some(Some(video)) = videos_cb.get(current) {
                video.toggle();
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
    toolbar.add_controller(keys);

    let carousel_focus = carousel.clone();
    toolbar.connect_map(move |_| {
        carousel_focus.grab_focus();
    });

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

    navigation.push(&page);
}
