// SPDX-License-Identifier: GPL-2.0-or-later
//
// Copyright 2026 nico359

//! Swipeable media viewer, pushed as a page inside the camera window.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant, UNIX_EPOCH};

use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gdk, gio};
use image::ImageDecoder;

use crate::media::{self, MediaKind};
use crate::ui::zoom_picture::ZoomPicture;
use crate::window::CameraWindow;

/// Photos are decoded to fit this box instead of holding a full-resolution
/// texture.  A 12 MP JPEG is ~48 MB as an RGBA texture; this keeps each page
/// in the low megabytes and makes swiping cheap.
const PHOTO_MAX: i32 = 1600;

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

/// Return a square, cropped 48 px thumbnail from the persistent video poster.
pub(crate) fn cached_video_poster(path: &Path) -> Option<gdk::Texture> {
    let cache_path = poster_cache_path(path)?;
    crate::ui::square_texture(&cache_path, 48)
}

/// Keep a decoded poster in memory and on disk for the gallery and the
/// last-media thumbnail.
pub(crate) fn store_video_poster(path: &Path, texture: &gdk::Texture) {
    POSTER_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .insert(path.to_path_buf(), texture.clone())
    });

    if let Some(cache_path) = poster_cache_path(path) {
        if let Some(directory) = cache_path.parent() {
            let _ = std::fs::create_dir_all(directory);
        }
        let _ = texture.save_to_png(&cache_path);
    }
}

/// Persistent poster frame for a video, keyed on the file path and mtime.
fn poster_cache_path(path: &Path) -> Option<PathBuf> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let seconds = modified.duration_since(UNIX_EPOCH).ok()?.as_secs();

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    seconds.hash(&mut hasher);

    Some(
        gtk::glib::user_cache_dir()
            .join("camera")
            .join("vidthumbs")
            .join(format!("{:016x}.png", hasher.finish())),
    )
}

struct DecodedPhoto {
    width: i32,
    height: i32,
    bytes: Vec<u8>,
}

/// Decode on a worker thread. Pure Rust, so it is safe to call from any
/// thread (gdk-pixbuf decoding from a sandboxed worker thread hangs).
fn decode_photo(path: PathBuf) -> Option<DecodedPhoto> {
    let reader = image::ImageReader::open(&path)
        .ok()?
        .with_guessed_format()
        .ok()?;
    let mut decoder = reader.into_decoder().ok()?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut image = image::DynamicImage::from_decoder(decoder).ok()?;
    image.apply_orientation(orientation);

    let thumbnail = image.thumbnail(PHOTO_MAX as u32, PHOTO_MAX as u32);
    let rgba = thumbnail.to_rgba8();
    let (width, height) = rgba.dimensions();

    Some(DecodedPhoto {
        width: width as i32,
        height: height as i32,
        bytes: rgba.into_raw(),
    })
}

/// A photo page. The scaled texture is decoded lazily and kept in the shared
/// cache so returning to a page is instant.
struct PhotoPage {
    root: gtk::Overlay,
    picture: ZoomPicture,
    spinner: gtk::Spinner,
    path: PathBuf,
    loaded: Cell<bool>,
    loading: Cell<bool>,
}

impl PhotoPage {
    fn new(path: &Path) -> Rc<Self> {
        let root = gtk::Overlay::new();
        root.add_css_class("gallery-page");
        root.set_hexpand(true);
        root.set_vexpand(true);

        let picture = ZoomPicture::new();
        root.set_child(Some(&picture));

        let spinner = gtk::Spinner::new();
        spinner.set_halign(gtk::Align::Center);
        spinner.set_valign(gtk::Align::Center);
        spinner.set_size_request(48, 48);
        spinner.set_visible(false);
        root.add_overlay(&spinner);

        Rc::new(Self {
            root,
            picture,
            spinner,
            path: path.to_path_buf(),
            loaded: Cell::new(false),
            loading: Cell::new(false),
        })
    }

    fn load(self: &Rc<Self>) {
        if self.loaded.get() {
            return;
        }

        if let Some(texture) = PHOTO_CACHE.with(|cache| cache.borrow().get(&self.path)) {
            self.picture.set_texture(Some(&texture));
            self.loaded.set(true);
            return;
        }

        if self.loading.replace(true) {
            return;
        }

        self.spinner.set_visible(true);
        self.spinner.start();

        let path = self.path.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(decode_photo(path));
        });

        let weak = Rc::downgrade(self);
        gtk::glib::timeout_add_local(Duration::from_millis(10), move || {
            let decoded = match receiver.try_recv() {
                Ok(decoded) => decoded,
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    return gtk::glib::ControlFlow::Continue;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if let Some(page) = weak.upgrade() {
                        page.loading.set(false);
                        page.spinner.stop();
                        page.spinner.set_visible(false);
                    }
                    return gtk::glib::ControlFlow::Break;
                }
            };

            if let Some(page) = weak.upgrade() {
                page.loading.set(false);
                page.spinner.stop();
                page.spinner.set_visible(false);
                if let Some(decoded) = decoded {
                    page.apply_decoded(decoded);
                }
            }
            gtk::glib::ControlFlow::Break
        });
    }

    fn apply_decoded(&self, decoded: DecodedPhoto) {
        let bytes = gtk::glib::Bytes::from(&decoded.bytes);
        let texture: gdk::Texture = gdk::MemoryTexture::new(
            decoded.width,
            decoded.height,
            gdk::MemoryFormat::R8g8b8a8,
            &bytes,
            decoded.width as usize * 4,
        )
        .upcast();
        PHOTO_CACHE.with(|cache| {
            cache
                .borrow_mut()
                .insert(self.path.clone(), texture.clone())
        });
        self.picture.set_texture(Some(&texture));
        self.loaded.set(true);
    }

    fn unload(&self) {
        if self.loaded.replace(false) {
            self.picture.clear();
        }
        self.spinner.stop();
        self.spinner.set_visible(false);
    }

    fn zoom_in(&self) {
        self.picture.zoom_in();
    }

    fn zoom_out(&self) {
        self.picture.zoom_out();
    }
}

/// One video page. The media stream is created only while the page is the
/// current one, so swiping never has several decoders running at once. A
/// poster frame is captured from the first rendered frame and reused.
struct VideoPage {
    root: gtk::Overlay,
    picture: gtk::Picture,
    button: gtk::Button,
    controls: gtk::Box,
    control_button: gtk::Button,
    seek: gtk::Scale,
    time_label: gtk::Label,
    spinner: gtk::Spinner,
    path: PathBuf,
    media: RefCell<Option<gtk::MediaFile>>,
    poster: RefCell<Option<gdk::Texture>>,
    realized: Cell<bool>,
    showing_stream: Cell<bool>,
    scrubbing: Cell<bool>,
    updating: Cell<bool>,
    tick: RefCell<Option<gtk::glib::SourceId>>,
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

        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        controls.add_css_class("gallery-controls");
        controls.set_halign(gtk::Align::Fill);
        controls.set_valign(gtk::Align::End);
        controls.set_margin_start(12);
        controls.set_margin_end(12);
        controls.set_margin_bottom(12);
        controls.set_visible(false);

        let control_button = gtk::Button::from_icon_name("media-playback-start-symbolic");
        control_button.add_css_class("flat");
        control_button.add_css_class("circular");
        control_button.set_tooltip_text(Some(&gettext("Play")));
        control_button.set_valign(gtk::Align::Center);
        control_button.set_size_request(40, 40);

        let seek = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 1.0);
        seek.set_draw_value(false);
        seek.set_hexpand(true);
        seek.set_valign(gtk::Align::Center);
        seek.set_round_digits(0);
        seek.set_sensitive(false);
        seek.add_css_class("gallery-seek");

        let time_label = gtk::Label::new(Some("0:00 / 0:00"));
        time_label.add_css_class("caption");
        time_label.add_css_class("numeric");
        time_label.set_xalign(1.0);
        time_label.set_width_chars(11);
        time_label.set_valign(gtk::Align::Center);

        controls.append(&control_button);
        controls.append(&seek);
        controls.append(&time_label);
        root.add_overlay(&controls);

        let poster = POSTER_CACHE
            .with(|cache| cache.borrow().get(path))
            .or_else(|| {
                let cache_path = poster_cache_path(path)?;
                gdk::Texture::from_filename(&cache_path).ok()
            });
        if let Some(poster) = &poster {
            picture.set_paintable(Some(poster));
        }

        let page = Rc::new(Self {
            root,
            picture,
            button,
            controls,
            control_button,
            seek,
            time_label,
            spinner,
            path: path.to_path_buf(),
            media: RefCell::new(None),
            poster: RefCell::new(poster),
            realized: Cell::new(false),
            showing_stream: Cell::new(false),
            scrubbing: Cell::new(false),
            updating: Cell::new(false),
            tick: RefCell::new(None),
        });

        let weak = Rc::downgrade(&page);
        page.button.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.toggle();
            }
        });

        let weak = Rc::downgrade(&page);
        page.control_button.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.toggle();
            }
        });

        let weak = Rc::downgrade(&page);
        page.seek.connect_value_changed(move |scale| {
            let Some(page) = weak.upgrade() else {
                return;
            };
            if page.updating.get() || page.scrubbing.get() {
                return;
            }
            page.seek_to(scale.value());
        });

        let drag = gtk::GestureDrag::new();
        drag.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&page);
        drag.connect_drag_begin(move |_, _, _| {
            if let Some(page) = weak.upgrade() {
                page.scrubbing.set(true);
            }
        });
        let weak = Rc::downgrade(&page);
        drag.connect_drag_end(move |_, _, _| {
            if let Some(page) = weak.upgrade() {
                page.scrubbing.set(false);
                page.seek_to(page.seek.value());
            }
        });
        page.seek.add_controller(drag);

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

        store_video_poster(&self.path, &texture);

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
        self.controls.set_visible(true);
        self.start_ticking();
        self.update_controls();

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
                page.update_controls();
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
                page.update_controls();
            }
        });

        let weak = Rc::downgrade(self);
        stream.connect_error_notify(move |stream| {
            if let Some(page) = weak.upgrade() {
                if stream.error().is_some() {
                    page.spinner.stop();
                    page.spinner.set_visible(false);
                    page.button.set_visible(false);
                    page.controls.set_visible(false);
                    page.stop_ticking();
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
        self.stop_ticking();
        self.controls.set_visible(false);
        self.scrubbing.set(false);
        self.updating.set(false);
    }

    fn start_ticking(self: &Rc<Self>) {
        if self.tick.borrow().is_some() {
            return;
        }

        let weak = Rc::downgrade(self);
        let id = gtk::glib::timeout_add_local(Duration::from_millis(250), move || {
            let Some(page) = weak.upgrade() else {
                return gtk::glib::ControlFlow::Break;
            };
            if page.media.borrow().is_none() {
                page.tick.borrow_mut().take();
                return gtk::glib::ControlFlow::Break;
            }
            page.update_controls();
            gtk::glib::ControlFlow::Continue
        });
        *self.tick.borrow_mut() = Some(id);
    }

    fn stop_ticking(&self) {
        if let Some(id) = self.tick.borrow_mut().take() {
            id.remove();
        }
    }

    fn seek_to(&self, seconds: f64) {
        if let Some(stream) = self.media.borrow().as_ref() {
            stream.seek((seconds * 1_000_000.0).round() as i64);
        }
    }

    fn update_controls(&self) {
        let stream = self.media.borrow().as_ref().cloned();
        let Some(stream) = stream else {
            return;
        };

        let duration = stream.duration();
        let timestamp = stream.timestamp().max(0);
        let seekable = stream.is_seekable() && duration > 0;
        self.seek.set_sensitive(seekable);

        if seekable && !self.scrubbing.get() {
            self.updating.set(true);
            self.seek.set_range(0.0, duration as f64 / 1_000_000.0);
            self.seek.set_value(timestamp as f64 / 1_000_000.0);
            self.updating.set(false);
        }

        let duration_label = if duration > 0 {
            format_time(duration)
        } else {
            "--:--".to_string()
        };
        self.time_label
            .set_text(&format!("{} / {}", format_time(timestamp), duration_label));
        self.update_icon(stream.is_playing());
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
        let (icon, tooltip) = if playing {
            ("media-playback-pause-symbolic", gettext("Pause"))
        } else {
            ("media-playback-start-symbolic", gettext("Play"))
        };
        self.button.set_icon_name(icon);
        self.button.set_tooltip_text(Some(&tooltip));
        self.button.set_visible(!playing);
        self.control_button.set_icon_name(icon);
        self.control_button.set_tooltip_text(Some(&tooltip));
    }
}

fn format_time(microseconds: i64) -> String {
    let total = (microseconds.max(0) / 1_000_000) as u64;
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
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

    /// Warm a neighbouring photo while the carousel is idle. Videos are not
    /// prefetched so only one decoder ever runs.
    fn prefetch(&self) {
        if let PageMedia::Photo(photo) = &self.media {
            photo.load();
        }
    }
}

/// Warm the neighbouring photos. Decoding happens on worker threads, so this
/// is safe to call while the carousel is still moving.
fn prefetch_neighbors(entries: &[PageEntry], index: usize) {
    if let Some(entry) = index.checked_sub(1).and_then(|i| entries.get(i)) {
        entry.prefetch();
    }
    if let Some(entry) = entries.get(index + 1) {
        entry.prefetch();
    }
}

struct WarmVideo {
    index: usize,
    video: Rc<VideoPage>,
    started: Instant,
}

/// Generate missing video posters in the background, one decoder at a time and
/// nearest page first. Mirrors FuriCam's thumbnail queue.
fn start_poster_prewarm(
    carousel: adw::Carousel,
    entries: Rc<Vec<PageEntry>>,
    first: usize,
) -> Rc<RefCell<Option<gtk::glib::SourceId>>> {
    let mut indices: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| match &entry.media {
            PageMedia::Video(video) if video.poster.borrow().is_none() => Some(index),
            _ => None,
        })
        .collect();
    indices.sort_by_key(|index| index.abs_diff(first));

    let queue = Rc::new(RefCell::new(VecDeque::from(indices)));
    let warming: Rc<RefCell<Option<WarmVideo>>> = Rc::new(RefCell::new(None));
    let tick: Rc<RefCell<Option<gtk::glib::SourceId>>> = Rc::new(RefCell::new(None));

    let tick_cb = Rc::clone(&tick);
    let id = gtk::glib::timeout_add_local(Duration::from_millis(300), move || {
        let current = carousel.position().round().max(0.0) as usize;

        let warm = { warming.borrow_mut().take() };
        if let Some(warm) = warm {
            let done = warm.video.poster.borrow().is_some()
                || warm.started.elapsed() > Duration::from_secs(4);
            if !done {
                *warming.borrow_mut() = Some(warm);
                return gtk::glib::ControlFlow::Continue;
            }
            if current != warm.index {
                warm.video.unload();
            }
        }

        loop {
            let next = { queue.borrow_mut().pop_front() };
            let Some(index) = next else {
                tick_cb.borrow_mut().take();
                return gtk::glib::ControlFlow::Break;
            };
            if index == current {
                continue;
            }
            let PageMedia::Video(video) = &entries[index].media else {
                continue;
            };
            if video.poster.borrow().is_some() {
                continue;
            }

            video.load();
            *warming.borrow_mut() = Some(WarmVideo {
                index,
                video: Rc::clone(video),
                started: Instant::now(),
            });
            return gtk::glib::ControlFlow::Continue;
        }
    });
    *tick.borrow_mut() = Some(id);
    tick
}

/// The navigation view's swipe tracker is a capture-phase drag gesture. It
/// still claims horizontal drags when the visible page has `can-pop` disabled,
/// which cancels panning in the photo viewer. The gallery is the only page in
/// this navigation view, so disable the tracker's drag/scroll controllers and
/// use the explicit Back button instead.
fn disable_navigation_swipe(navigation: &adw::NavigationView) {
    let controllers = navigation.observe_controllers();
    for index in 0..controllers.n_items() {
        let Some(controller) = controllers.item(index) else {
            continue;
        };
        let Ok(controller) = controller.downcast::<gtk::EventController>() else {
            continue;
        };
        if controller.is::<gtk::GestureDrag>() || controller.is::<gtk::EventControllerScroll>() {
            controller.set_propagation_phase(gtk::PropagationPhase::None);
        }
    }
}

pub fn present(parent: &CameraWindow) {
    let navigation = parent.navigation_view();
    if navigation.find_page("gallery").is_some() {
        return;
    }

    disable_navigation_swipe(&navigation);

    let items = Rc::new(media::scan_media());

    let header = adw::HeaderBar::new();

    let back = gtk::Button::from_icon_name("go-previous-symbolic");
    back.add_css_class("flat");
    back.set_tooltip_text(Some(&gettext("Back")));
    let nav_weak = navigation.downgrade();
    back.connect_clicked(move |_| {
        if let Some(navigation) = nav_weak.upgrade() {
            navigation.pop();
        }
    });
    header.pack_start(&back);

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

    let pager = gtk::Label::new(None);
    pager.add_css_class("gallery-pager");
    pager.set_halign(gtk::Align::Center);
    pager.set_margin_top(6);
    pager.set_margin_bottom(10);
    pager.set_visible(items.len() > 1);
    if items.len() > 1 {
        pager.set_label(&format!("1 / {}", items.len()));
    }

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("gallery-viewer");
    content.append(&carousel);
    content.append(&pager);

    let toast = adw::ToastOverlay::new();
    toast.set_child(Some(&content));

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&toast));

    let page = adw::NavigationPage::new(&toolbar, &gettext("Gallery"));
    page.set_tag(Some("gallery"));
    // Paging is handled by the carousel; the navigation swipe-back would
    // compete with panning a zoomed photo, so use the Back button instead.
    page.set_can_pop(false);

    let current = Rc::new(RefCell::new(None));

    let first = carousel.position().round().max(0.0) as usize;
    if let Some(entry) = entries.get(first) {
        entry.load();
    }

    // Warm the next few photos while the first one decodes.
    for entry in entries.iter().skip(first + 1).take(2) {
        entry.prefetch();
    }
    if let Some(item) = items.first() {
        title.set_subtitle(&file_name(&item.path));
        *current.borrow_mut() = Some(item.path.clone());
    }

    let items_cb = Rc::clone(&items);
    let entries_cb = Rc::clone(&entries);
    let title_cb = title.clone();
    let current_cb = Rc::clone(&current);
    let pager_cb = pager.clone();
    let carousel_focus = carousel.clone();
    let total = items.len();
    carousel.connect_page_changed(move |_, index| {
        let index = index as usize;
        carousel_focus.grab_focus();
        if total > 1 {
            pager_cb.set_label(&format!("{} / {}", index + 1, total));
        }
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

        prefetch_neighbors(&entries_cb, index);
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

        if matches!(keyval, gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add) {
            if let Some(entry) = entries_cb.get(current) {
                if let PageMedia::Photo(photo) = &entry.media {
                    photo.zoom_in();
                }
            }
            return gtk::glib::Propagation::Stop;
        }

        if matches!(keyval, gdk::Key::minus | gdk::Key::KP_Subtract) {
            if let Some(entry) = entries_cb.get(current) {
                if let PageMedia::Photo(photo) = &entry.media {
                    photo.zoom_out();
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

    let warm_tick = start_poster_prewarm(carousel.clone(), Rc::clone(&entries), first);
    toolbar.connect_unmap(move |_| {
        if let Some(id) = warm_tick.borrow_mut().take() {
            id.remove();
        }
    });
}
