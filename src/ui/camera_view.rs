// SPDX-License-Identifier: GPL-3.0-or-later
//
// Copyright 2026 nico359
//
// GtkGLArea wrapper that owns the camera session and draws the live preview.
// Keeping this out of the window keeps the window concerned with actions and
// chrome only.

use std::cell::{Cell, RefCell};
use std::path::Path;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::camera::{CameraError, CameraSession, Facing};

const PREVIEW_WIDTH: i32 = 1280;
const PREVIEW_HEIGHT: i32 = 720;

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

        tick: RefCell<Option<gtk::TickCallbackId>>,
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

            let result = {
                let mut session = self.session.borrow_mut();
                session
                    .open(Facing::Back)
                    .and_then(|()| session.start_preview(PREVIEW_WIDTH, PREVIEW_HEIGHT))
            };

            match result {
                Ok(()) => {
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
            obj.set_ready(false);

            self.parent_unrealize();
        }
    }

    impl GLAreaImpl for CameraView {
        fn render(&self, _context: &gtk::gdk::GLContext) -> glib::Propagation {
            let obj = self.obj();
            let scale = obj.scale_factor();

            let mut session = self.session.borrow_mut();
            // Match the upstream camera2_gtk prototype: the sensor is mounted
            // at 90 degrees on the FuriPhone and the renderer expects the
            // extra half-turn to keep the preview upright in portrait.
            let rotation = (session.sensor_orientation() + 180).rem_euclid(360);
            session.render_preview(obj.width() * scale, obj.height() * scale, rotation);

            glib::Propagation::Stop
        }
    }

    impl CameraView {
        fn start_tick(&self) {
            if self.tick.borrow().is_some() {
                return;
            }

            let weak = self.obj().downgrade();
            let id = self.obj().add_tick_callback(move |_, _| match weak.upgrade() {
                Some(view) => {
                    view.queue_render();
                    glib::ControlFlow::Continue
                }
                None => glib::ControlFlow::Break,
            });

            *self.tick.borrow_mut() = Some(id);
        }

        fn stop_tick(&self) {
            if let Some(id) = self.tick.borrow_mut().take() {
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

    pub fn switch_camera(&self) -> Result<(), CameraError> {
        if !self.imp().session.borrow().is_open() {
            return Err(CameraError::NotOpen);
        }

        self.make_current();

        let result = {
            let mut session = self.imp().session.borrow_mut();
            let facing = session.facing().opposite();
            session
                .open(facing)
                .and_then(|()| session.start_preview(PREVIEW_WIDTH, PREVIEW_HEIGHT))
        };

        match result {
            Ok(()) => {
                self.set_status("");
                self.set_ready(true);
                Ok(())
            }
            Err(error) => {
                self.set_status(format!("Camera unavailable: {error}"));
                self.set_ready(false);
                Err(error)
            }
        }
    }

    pub fn capture_photo(&self, path: &Path) -> Result<(), CameraError> {
        self.imp().session.borrow_mut().capture_photo(path)
    }
}

impl Default for CameraView {
    fn default() -> Self {
        Self::new()
    }
}
