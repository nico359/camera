// SPDX-License-Identifier: GPL-2.0-or-later
//
// Copyright 2026 nico359
//
// GtkGLArea wrapper that owns the camera session and draws the live preview.
// Keeping this out of the window keeps the window concerned with actions and
// chrome only.

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::time::Duration;

use gtk::gio::prelude::*;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::camera::{CameraError, CameraInfo, CameraSession, Facing};
use crate::device_rotation::DeviceRotation;

const PREVIEW_WIDTH: i32 = 1280;
const PREVIEW_HEIGHT: i32 = 960;
const VIDEO_WIDTH: i32 = 1920;
const VIDEO_HEIGHT: i32 = 1080;
const VIDEO_PREVIEW_WIDTH: i32 = 1280;
const VIDEO_PREVIEW_HEIGHT: i32 = 720;
const VIDEO_FPS: i32 = 30;
const VIDEO_BITRATE: i32 = 20_000_000;

mod imp {
    use super::*;
    use glib::Properties;

    #[derive(Properties, Default)]
    #[properties(wrapper_type = super::CameraView)]
    pub struct CameraView {
        pub(super) session: RefCell<CameraSession>,

        #[property(get, set)]
        status: RefCell<String>,

        #[property(get, set)]
        ready: Cell<bool>,

        #[property(get, set)]
        device_rotation: Cell<i32>,

        rotation_source: RefCell<Option<DeviceRotation>>,
        rotation_tick: RefCell<Option<glib::SourceId>>,

        pub(super) photo_callback: RefCell<Option<Box<dyn Fn(&str, bool)>>>,
        render_timeout: RefCell<Option<glib::SourceId>>,
        pub(super) video_mode: Cell<bool>,
        pub(super) video_preview_16_9: Cell<bool>,
        pub(super) zoom: Cell<f32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CameraView {
        const NAME: &'static str = "CameraView";
        type Type = super::CameraView;
        type ParentType = gtk::GLArea;
    }

    #[glib::derived_properties]
    impl ObjectImpl for CameraView {
        fn constructed(&self) {
            self.parent_constructed();

            let obj = self.obj();
            obj.set_allowed_apis(gtk::gdk::GLAPI::GLES);
            obj.set_has_depth_buffer(false);
            obj.set_hexpand(true);
            obj.set_vexpand(true);
        }
    }

    impl WidgetImpl for CameraView {
        fn realize(&self) {
            self.parent_realize();

            let obj = self.obj();
            obj.make_current();

            if let Some(error) = obj.error() {
                obj.set_status(format!("OpenGL error: {error}"));
                obj.set_ready(false);
                return;
            }

            let rotation_source = DeviceRotation::new();
            let degrees = rotation_source.degrees();
            obj.set_device_rotation(degrees);
            self.install_photo_bridge();

            let result = {
                let mut session = self.session.borrow_mut();
                session
                    .open_first()
                    .and_then(|()| session.start_preview(PREVIEW_WIDTH, PREVIEW_HEIGHT, true))
            };

            match result {
                Ok(()) => {
                    self.session.borrow_mut().set_device_rotation(degrees);
                    self.zoom.set(1.0);
                    *self.rotation_source.borrow_mut() = Some(rotation_source);
                    self.start_rotation_tick();

                    obj.set_status("");
                    obj.set_ready(true);
                    self.start_tick();
                }
                Err(error) => {
                    obj.set_status(format!("Camera unavailable: {error}"));
                    obj.set_ready(false);
                    self.session.borrow_mut().close();
                }
            }
        }

        fn unrealize(&self) {
            self.stop_tick();

            let obj = self.obj();
            obj.make_current();
            self.session.borrow_mut().close();
            self.stop_rotation_tick();
            self.rotation_source.borrow_mut().take();
            self.video_mode.set(false);
            self.video_preview_16_9.set(false);
            self.zoom.set(1.0);
            obj.set_ready(false);

            self.parent_unrealize();
        }
    }

    impl GLAreaImpl for CameraView {
        fn render(&self, _context: &gtk::gdk::GLContext) -> glib::Propagation {
            let obj = self.obj();
            let scale = obj.scale_factor();

            let mut session = self.session.borrow_mut();
            // The preview is deliberately portrait-locked: only the saved
            // still/video follows the device tilt. FuriCam does the same, and
            // a rotating preview inside a fixed window is disorienting.
            let rotation = (session.sensor_orientation() + 180).rem_euclid(360);
            session.render_preview(obj.width() * scale, obj.height() * scale, rotation);

            glib::Propagation::Stop
        }
    }

    impl CameraView {
        fn start_tick(&self) {
            if self.render_timeout.borrow().is_some() {
                return;
            }

            // The engine delivers at 30 fps, so redrawing at 60 fps only burns
            // CPU.  A frame-callback-driven repaint would be even better, but
            // this timeout is the low-risk half of that fix.
            let weak = self.obj().downgrade();
            let id =
                glib::timeout_add_local(Duration::from_millis(33), move || match weak.upgrade() {
                    Some(view) => {
                        view.queue_render();
                        glib::ControlFlow::Continue
                    }
                    None => glib::ControlFlow::Break,
                });

            *self.render_timeout.borrow_mut() = Some(id);
        }

        fn stop_tick(&self) {
            if let Some(id) = self.render_timeout.borrow_mut().take() {
                id.remove();
            }
        }

        fn install_photo_bridge(&self) {
            let weak = glib::SendWeakRef::from(self.obj().downgrade());
            let callback = Box::new(move |path: &str, ok: bool| {
                let path = path.to_owned();
                let weak = weak.clone();
                // `idle_add_once` always queues on the default main context,
                // unlike `MainContext::invoke`, which may run the closure on
                // the calling (camera/writer) thread.
                glib::idle_add_once(move || {
                    if let Some(view) = weak.upgrade() {
                        let callback = view.imp().photo_callback.borrow();
                        if let Some(callback) = callback.as_ref() {
                            callback(&path, ok);
                        }
                    }
                });
            });

            self.session.borrow().set_photo_callback(Some(callback));
        }

        fn start_rotation_tick(&self) {
            if self.rotation_tick.borrow().is_some() {
                return;
            }

            let weak = self.obj().downgrade();
            let id =
                glib::timeout_add_local(Duration::from_millis(250), move || match weak.upgrade() {
                    Some(view) => {
                        let degrees = view
                            .imp()
                            .rotation_source
                            .borrow()
                            .as_ref()
                            .map(DeviceRotation::degrees)
                            .unwrap_or(0);
                        view.apply_device_rotation(degrees);
                        glib::ControlFlow::Continue
                    }
                    None => glib::ControlFlow::Break,
                });

            *self.rotation_tick.borrow_mut() = Some(id);
        }

        fn stop_rotation_tick(&self) {
            if let Some(id) = self.rotation_tick.borrow_mut().take() {
                id.remove();
            }
        }
    }
}

glib::wrapper! {
    pub struct CameraView(ObjectSubclass<imp::CameraView>)
        @extends gtk::GLArea, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl CameraView {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    /// Register the GType before a GtkBuilder template references it.
    pub fn ensure_type() {
        let _ = Self::static_type();
    }

    fn apply_device_rotation(&self, degrees: i32) {
        if self.device_rotation() == degrees {
            return;
        }

        self.set_device_rotation(degrees);
        self.imp().session.borrow_mut().set_device_rotation(degrees);
    }

    pub fn flip_camera(&self) -> Result<CameraInfo, CameraError> {
        self.open_camera(CameraSession::flip)
    }

    pub fn select_camera(&self, index: usize) -> Result<CameraInfo, CameraError> {
        if self.imp().session.borrow().current_index() == Some(index) {
            return self
                .imp()
                .session
                .borrow()
                .current()
                .cloned()
                .ok_or(CameraError::NotOpen);
        }
        self.open_camera(|session| session.select(index))
    }

    pub fn cameras(&self) -> Vec<CameraInfo> {
        self.imp().session.borrow().cameras().to_vec()
    }

    pub fn current_camera_index(&self) -> Option<usize> {
        self.imp().session.borrow().current_index()
    }

    fn open_camera<F>(&self, change: F) -> Result<CameraInfo, CameraError>
    where
        F: FnOnce(&mut CameraSession) -> Result<CameraInfo, CameraError>,
    {
        if self.imp().session.borrow().is_recording() {
            return Err(CameraError::Busy);
        }
        if !self.imp().session.borrow().is_open() {
            return Err(CameraError::NotOpen);
        }

        let was_video = self.imp().video_mode.get();
        self.make_current();

        let result = {
            let mut session = self.imp().session.borrow_mut();
            if was_video {
                session.exit_video_mode();
                session.release_recording();
            }

            change(&mut session).and_then(|info| {
                if was_video {
                    match Self::start_video_preview(&mut session) {
                        Ok(sixteen) => {
                            self.imp().video_preview_16_9.set(sixteen);
                            self.imp().video_mode.set(true);
                            Ok(info)
                        }
                        Err(error) => {
                            // The new camera may not support video mode.  Keep
                            // it usable in photo mode instead of declaring the
                            // whole camera unavailable.
                            let restored = Self::restore_photo_preview(&mut session).is_ok();
                            self.imp().video_mode.set(false);
                            self.imp().video_preview_16_9.set(false);
                            if restored {
                                Ok(info)
                            } else {
                                Err(error)
                            }
                        }
                    }
                } else {
                    session
                        .start_preview(PREVIEW_WIDTH, PREVIEW_HEIGHT, true)
                        .map(|()| info)
                }
            })
        };

        match result {
            Ok(info) => {
                {
                    let mut session = self.imp().session.borrow_mut();
                    session.set_zoom(1.0);
                    session.set_torch(false);
                    session.set_flash_mode(0);
                }
                self.imp().zoom.set(1.0);
                self.set_status("");
                self.set_ready(true);
                Ok(info)
            }
            Err(error) => {
                if was_video {
                    self.imp().video_mode.set(false);
                    self.imp().video_preview_16_9.set(false);
                }
                self.set_status(format!("Camera unavailable: {error}"));
                self.set_ready(false);
                Err(error)
            }
        }
    }

    pub fn set_video_mode(&self, enabled: bool) -> Result<(), CameraError> {
        if enabled == self.imp().video_mode.get() {
            return Ok(());
        }
        if self.imp().session.borrow().is_recording() {
            return Err(CameraError::Busy);
        }
        if !self.imp().session.borrow().is_open() {
            return Err(CameraError::NotOpen);
        }

        let mut session = self.imp().session.borrow_mut();
        if enabled {
            let sixteen = Self::start_video_preview(&mut session)?;
            self.imp().video_preview_16_9.set(sixteen);
            self.imp().video_mode.set(true);
        } else {
            session.exit_video_mode();
            session.release_recording();
            let restored = Self::restore_photo_preview(&mut session).is_ok();
            self.imp().video_preview_16_9.set(!restored);
            self.imp().video_mode.set(false);
            if !restored {
                return Err(CameraError::PreviewFailed);
            }
        }

        Ok(())
    }

    fn start_video_preview(session: &mut CameraSession) -> Result<bool, CameraError> {
        // Video mode does not need the full-resolution JPEG output.  Start a
        // still-capture-free 16:9 preview; that removes one heavy stream and
        // lets the HAL deliver frames at the requested rate.
        let sixteen = session
            .start_preview(VIDEO_PREVIEW_WIDTH, VIDEO_PREVIEW_HEIGHT, false)
            .is_ok();
        if !sixteen
            && session
                .start_preview(PREVIEW_WIDTH, PREVIEW_HEIGHT, false)
                .is_err()
        {
            let _ = Self::restore_photo_preview(session);
            return Err(CameraError::PreviewFailed);
        }

        if let Err(error) =
            session.enter_video_mode(VIDEO_WIDTH, VIDEO_HEIGHT, VIDEO_FPS, VIDEO_BITRATE)
        {
            let _ = Self::restore_photo_preview(session);
            return Err(error);
        }

        // Warm the mic so the first clip does not lose its opening seconds to
        // GStreamer/AudioCodec startup.
        let _ = session.prepare_recording();
        Ok(sixteen)
    }

    fn restore_photo_preview(session: &mut CameraSession) -> Result<(), CameraError> {
        session.start_preview(PREVIEW_WIDTH, PREVIEW_HEIGHT, true)
    }

    pub fn video_mode(&self) -> bool {
        self.imp().video_mode.get()
    }

    pub fn is_recording(&self) -> bool {
        self.imp().session.borrow().is_recording()
    }

    pub fn start_recording(&self, path: &Path) -> Result<(), CameraError> {
        if !self.imp().video_mode.get() || self.imp().session.borrow().is_recording() {
            return Err(CameraError::Busy);
        }
        self.make_current();
        let mut session = self.imp().session.borrow_mut();
        session.start_recording(path, VIDEO_WIDTH, VIDEO_HEIGHT, VIDEO_FPS, VIDEO_BITRATE)?;
        Ok(())
    }

    /// Stop recording and return whether the muxer reported a write failure.
    pub fn stop_recording(&self) -> Result<bool, CameraError> {
        if !self.imp().session.borrow().is_recording() {
            return Err(CameraError::Busy);
        }
        let mut session = self.imp().session.borrow_mut();
        session.stop_recording();
        let write_failed = session.recording_write_failed();
        Ok(write_failed)
    }

    pub fn zoom(&self) -> f32 {
        self.imp().zoom.get()
    }

    pub fn max_zoom(&self) -> f32 {
        self.imp().session.borrow().max_zoom().max(1.0)
    }

    pub fn set_zoom(&self, zoom: f32) {
        let zoom = zoom.clamp(1.0, self.max_zoom());
        self.imp().session.borrow_mut().set_zoom(zoom);
        self.imp().zoom.set(zoom);
    }

    pub fn is_front(&self) -> bool {
        self.imp()
            .session
            .borrow()
            .current()
            .map(|camera| camera.facing == Facing::Front)
            .unwrap_or(false)
    }

    pub fn set_torch(&self, on: bool) {
        if self.is_front() {
            return;
        }
        self.imp().session.borrow_mut().set_torch(on);
    }

    pub fn set_flash_mode(&self, mode: i32) {
        self.imp().session.borrow_mut().set_flash_mode(mode);
    }

    /// Map a view-normalized tap to the sensor's active array, then ask the
    /// engine to focus there.  This is the inverse of the preview rotation plus
    /// the front-camera mirror (the same mapping FuriCam uses).
    pub fn focus_at(&self, x: f64, y: f64) -> Result<(), CameraError> {
        if !self.ready() {
            return Err(CameraError::NotOpen);
        }

        let (rotation, mirrored) = {
            let session = self.imp().session.borrow();
            let rotation = (session.sensor_orientation() + 180).rem_euclid(360);
            let mirrored = session
                .current()
                .map(|camera| camera.facing == Facing::Front)
                .unwrap_or(false);
            (rotation, mirrored)
        };

        let rad = rotation as f64 * std::f64::consts::PI / 180.0;
        let sin = rad.sin();
        let cos = rad.cos();
        let nx = x.clamp(0.0, 1.0) - 0.5;
        let ny = y.clamp(0.0, 1.0) - 0.5;
        let mut sx = cos * nx - sin * ny + 0.5;
        let sy = sin * nx + cos * ny + 0.5;
        if mirrored {
            sx = 1.0 - sx;
        }

        self.imp()
            .session
            .borrow_mut()
            .set_focus_point(sx.clamp(0.0, 1.0) as f32, sy.clamp(0.0, 1.0) as f32);
        Ok(())
    }

    pub fn focus_assist_lit(&self) -> bool {
        self.imp().session.borrow().focus_assist_lit()
    }

    pub fn af_state(&self) -> i32 {
        self.imp().session.borrow().af_state()
    }

    pub fn clear_focus_assist(&self) {
        self.imp().session.borrow_mut().clear_focus_assist();
    }

    /// Aspect ratio of the preview area.  The preview is deliberately
    /// portrait-locked, so a 4:3 stream from a 90°/270°-mounted sensor is
    /// shown as 3:4; a 16:9 video stream becomes 9:16.  A 0°/180° sensor
    /// stays 4:3 in photo mode.
    pub fn preview_aspect(&self) -> f32 {
        if self.imp().video_preview_16_9.get() {
            return 9.0 / 16.0;
        }

        let sensor = self
            .imp()
            .session
            .borrow()
            .sensor_orientation()
            .rem_euclid(360);
        if sensor % 180 == 90 {
            3.0 / 4.0
        } else {
            4.0 / 3.0
        }
    }

    /// Set the main-thread callback invoked after a still is written (or
    /// fails).  The camera thread marshals through `install_photo_bridge`.
    pub fn set_photo_callback<F>(&self, callback: F)
    where
        F: Fn(&str, bool) + 'static,
    {
        *self.imp().photo_callback.borrow_mut() = Some(Box::new(callback));
    }

    pub fn capture_photo(&self, path: &Path) -> Result<(), CameraError> {
        let mut session = self.imp().session.borrow_mut();
        session.set_device_rotation(self.device_rotation());
        session.capture_photo(path)
    }
}

impl Default for CameraView {
    fn default() -> Self {
        Self::new()
    }
}
