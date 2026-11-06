// SPDX-License-Identifier: GPL-2.0-or-later
//
// Copyright 2026 nico359

//! Swipeable media viewer, pushed as a page inside the camera window.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gdk, gio};

use crate::media::{self, MediaKind};
use crate::window::CameraWindow;

/// Photos are decoded to fit this box instead of holding a full-resolution
/// texture.  A 12 MP JPEG is ~48 MB as an RGBA texture; this keeps each page
/// in the low megabytes and makes swiping cheap.
const PHOTO_MAX: i32 = 2048;

/// Poster frames are only shown while a video prepares, so they stay small.
const POSTER_MAX: f64 = 1280.0;

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Media")
        .to_string()
}

/// Small LRU cache of decoded textures, keyed by file path.
struct TextureCache {
    order: VecDeque<PathBuf>,
    textures: HashMap<PathBuf, gdk::Texture>,
    capacity: usize,
}

impl TextureCache {
    fn new(capacity: usize) -> Self {
        Self {
            order: VecDeque::new(),
            textures: HashMap::new(),
            capacity,
        }
    }

    fn get(&self, path: &Path) -> Option<gdk::Texture> {
        self.textures.get(path).cloned()
    }

    fn insert(&mut self, path: PathBuf, texture: gdk::Texture) {
        if self.textures.insert(path.clone(), texture).is_none() {
            self.order.push_back(path);
        }
        while self.order.len() > self.capacity {
            if let Some(old) = self.order.pop_front() {
                self.textures.remove(&old);
            }
        }
    }
}

thread_local! {
    static PHOTO_CACHE: RefCell<TextureCache> = RefCell::new(TextureCache::new(4));
    static POSTER_CACHE: RefCell<TextureCache> = RefCell::new(TextureCache::new(12));
}

fn load_photo_texture(path: &Path) -> Option<gdk::Texture> {
    let pixbuf = gdk_pixbuf::Pixbuf::from_file_at_scale(path, PHOTO_MAX, PHOTO_MAX, true).ok()?;
    let pixbuf = pixbuf.apply_embedded_orientation().unwrap_or(pixbuf);

    let bytes = pixbuf.read_pixel_bytes();
    let format = if pixbuf.n_channels() == 4 {
        gdk::MemoryFormat::R8g8b8a8
    } else {
        gdk::MemoryFormat::R8g8b8
    };
    let texture = gdk::MemoryTexture::new(
        pixbuf.width(),
        pixbuf.height(),
        format,
        &bytes,
        pixbuf.rowstride() as usize,
    );
    Some(texture.upcast())
}

/// A photo page. The scaled texture is decoded lazily and kept in the shared
/// cache so returning to a page is instant.
struct PhotoPage {
    root: gtk::Box,
    picture: gtk::Picture,
    path: PathBuf,
    loaded: Cell<bool>,
}

impl PhotoPage {
    fn new(path: &Path) -> Rc<Self> {
        let root = page_box();

        let picture = gtk::Picture::new();
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_can_shrink(true);
        picture.set_hexpand(true);
        picture.set_vexpand(true);
        root.append(&picture);

        Rc::new(Self {
            root,
            picture,
            path: path.to_path_buf(),
            loaded: Cell::new(false),
        })
    }

    fn load(&self) {
        if self.loaded.get() {
            return;
        }

        let texture = PHOTO_CACHE
            .with(|cache| cache.borrow().get(&self.path))
            .or_else(|| {
                let texture = load_photo_texture(&self.path)?;
                PHOTO_CACHE.with(|cache| {
                    cache
                        .borrow_mut()
                        .insert(self.path.clone(), texture.clone())
                });
                Some(texture)
            });

        if let Some(texture) = texture {
            self.picture.set_paintable(Some(&texture));
            self.loaded.set(true);
        }
    }

    fn unload(&self) {
        if self.loaded.replace(false) {
            self.picture.set_paintable(None::<&gdk::Paintable>);
        }
    }
}

/// One video page. The media stream is created only while the page is the
/// current one, so swiping never has several decoders running at once. A
/// poster frame is captured from the first rendered frame and reused.
struct VideoPage {
    root: gtk::Overlay,
    picture: gtk::Picture,
    button: gtk::Button,
    spinner: gtk::Spinner,
    path: PathBuf,
    media: RefCell<Option<gtk::MediaFile>>,
    poster: RefCell<Option<gdk::Texture>>,
    realized: Cell<bool>,
    showing_stream: Cell<bool>,
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

        let poster = POSTER_CACHE.with(|cache| cache.borrow().get(path));
        if let Some(poster) = &poster {
            picture.set_paintable(Some(poster));
        }

        let page = Rc::new(Self {
            root,
            picture,
            button,
            spinner,
            path: path.to_path_buf(),
            media: RefCell::new(None),
            poster: RefCell::new(poster),
            realized: Cell::new(false),
            showing_stream: Cell::new(false),
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

    /// Snapshot the first rendered frame and keep it as this page's poster.
    fn capture_poster(self: &Rc<Self>, stream: &gtk::MediaFile) {
        if self.poster.borrow().is_some() {
            return;
        }

        let width = stream.intrinsic_width() as f64;
        let height = stream.intrinsic_height() as f64;
        if width <= 0.0 || height <= 0.0 {
            return;
        }

        let scale = (POSTER_MAX / width.max(height)).min(1.0);
        let snapshot = gtk::Snapshot::new();
        stream.snapshot(&snapshot, width * scale, height * scale);
        let Some(node) = snapshot.to_node() else {
            return;
        };
        let Some(renderer) = self.root.native().and_then(|native| native.renderer()) else {
            return;
        };
        let texture = renderer.render_texture(&node, None);

        POSTER_CACHE.with(|cache| {
            cache
                .borrow_mut()
                .insert(self.path.clone(), texture.clone())
        });
        *self.poster.borrow_mut() = Some(texture);
        self.button.set_visible(true);
    }

    /// Start loading the paused first frame. Called when the page becomes the
    /// current carousel page.
    fn load(self: &Rc<Self>) {
        if self.media.borrow().is_some() {
            return;
        }

        if let Some(poster) = self.poster.borrow().as_ref() {
            self.picture.set_paintable(Some(poster));
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

        let weak = Rc::downgrade(self);
        stream.connect_invalidate_contents(move |stream| {
            let Some(page) = weak.upgrade() else {
                return;
            };
            page.capture_poster(stream);
            if !page.showing_stream.get() {
                page.showing_stream.set(true);
                page.picture.set_paintable(Some(stream));
            }
        });

        stream.connect_ended_notify(move |stream| {
            if stream.is_ended() {
                stream.seek(0);
            }
        });

        // With a cached poster, keep showing it until the first real frame
        // arrives; without one, show the stream directly as before.
        if self.poster.borrow().is_some() {
            self.showing_stream.set(false);
        } else {
            self.picture.set_paintable(Some(&stream));
            self.showing_stream.set(true);
        }
        *self.media.borrow_mut() = Some(stream);
        self.realize_stream();
    }

    /// Stop playback and release the decoder when the page is left, leaving
    /// the poster frame visible.
    fn unload(&self) {
        self.unrealize_stream();
        if let Some(stream) = self.media.borrow_mut().take() {
            stream.pause();
        }
        self.showing_stream.set(false);

        if let Some(poster) = self.poster.borrow().as_ref() {
            self.picture.set_paintable(Some(poster));
        } else {
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

enum PageMedia {
    Photo(Rc<PhotoPage>),
    Video(Rc<VideoPage>),
    Empty,
}

struct PageEntry {
    widget: gtk::Widget,
    media: PageMedia,
}

impl PageEntry {
    fn load(&self) {
        match &self.media {
            PageMedia::Photo(photo) => photo.load(),
            PageMedia::Video(video) => video.load(),
            PageMedia::Empty => {}
        }
    }

    fn unload(&self) {
        match &self.media {
            PageMedia::Photo(photo) => photo.unload(),
            PageMedia::Video(video) => video.unload(),
            PageMedia::Empty => {}
        }
    }
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

    let mut entries: Vec<PageEntry> = Vec::new();

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
        entries.push(PageEntry {
            widget: page.upcast(),
            media: PageMedia::Empty,
        });
    } else {
        for item in items.iter() {
            match item.kind {
                MediaKind::Photo => {
                    let photo = PhotoPage::new(&item.path);
                    carousel.append(&photo.root);
                    entries.push(PageEntry {
                        widget: photo.root.clone().upcast(),
                        media: PageMedia::Photo(photo),
                    });
                }
                MediaKind::Video => {
                    let video = VideoPage::new(&item.path);
                    carousel.append(&video.root);
                    entries.push(PageEntry {
                        widget: video.root.clone().upcast(),
                        media: PageMedia::Video(video),
                    });
                }
            }
        }
    }

    let entries = Rc::new(entries);

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

    let first = carousel.position().round().max(0.0) as usize;
    if let Some(entry) = entries.get(first) {
        entry.load();
    }
    if let Some(item) = items.first() {
        title.set_subtitle(&file_name(&item.path));
        *current.borrow_mut() = Some(item.path.clone());
    }

    let items_cb = Rc::clone(&items);
    let entries_cb = Rc::clone(&entries);
    let title_cb = title.clone();
    let current_cb = Rc::clone(&current);
    carousel.connect_page_changed(move |_, index| {
        let index = index as usize;
        if let Some(item) = items_cb.get(index) {
            title_cb.set_subtitle(&file_name(&item.path));
            *current_cb.borrow_mut() = Some(item.path.clone());
        }

        for (page, entry) in entries_cb.iter().enumerate() {
            if page == index {
                entry.load();
            } else {
                entry.unload();
            }
        }
    });

    let entries_cb = Rc::clone(&entries);
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
            if let Some(entry) = entries_cb.get(current) {
                if let PageMedia::Video(video) = &entry.media {
                    video.toggle();
                }
            }
            return gtk::glib::Propagation::Stop;
        }

        let target = if keyval == gdk::Key::Left {
            current.checked_sub(1)
        } else if keyval == gdk::Key::Right {
            (current + 1 < entries_cb.len()).then_some(current + 1)
        } else {
            None
        };

        let Some(target) = target else {
            return gtk::glib::Propagation::Proceed;
        };
        carousel_cb.scroll_to(&entries_cb[target].widget, true);
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
