// SPDX-License-Identifier: GPL-3.0-or-later
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

use crate::camera::{CameraError, CameraInfo, CameraSession};
use crate::device_rotation::DeviceRotation;

const PREVIEW_WIDTH: i32 = 1280;
const PREVIEW_HEIGHT: i32 = 960;

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
                    .and_then(|()| session.start_preview(PREVIEW_WIDTH, PREVIEW_HEIGHT))
            };

            match result {
                Ok(()) => {
                    self.session.borrow_mut().set_device_rotation(degrees);
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
        if !self.imp().session.borrow().is_open() {
            return Err(CameraError::NotOpen);
        }

        self.make_current();

        let result = {
            let mut session = self.imp().session.borrow_mut();
            change(&mut session).and_then(|info| {
                session
                    .start_preview(PREVIEW_WIDTH, PREVIEW_HEIGHT)
                    .map(|()| info)
            })
        };

        match result {
            Ok(info) => {
                self.set_status("");
                self.set_ready(true);
                Ok(info)
            }
            Err(error) => {
                self.set_status(format!("Camera unavailable: {error}"));
                self.set_ready(false);
                Err(error)
            }
        }
    }

    /// Aspect ratio of the preview area.  The preview is deliberately
    /// portrait-locked, so a 4:3 stream from a 90°/270°-mounted sensor is
    /// shown as 3:4; a 0°/180° sensor stays 4:3.
    pub fn preview_aspect(&self) -> f32 {
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
