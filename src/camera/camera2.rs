// SPDX-License-Identifier: GPL-3.0-or-later
//
// Copyright 2026 nico359
//
// Minimal safe wrapper around FuriCam's backend-neutral `fc2_*` C API.

use std::ffi::{c_char, c_int, c_void, CString};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::ptr;

use super::{CameraBackend, CameraError, Facing};

#[repr(C)]
struct fc2_camera {
    _private: [u8; 0],
}

extern "C" {
    fn fc2_available() -> c_int;
    fn fc2_open(facing: c_int) -> *mut fc2_camera;
    fn fc2_close(camera: *mut fc2_camera);
    fn fc2_sensor_orientation(camera: *mut fc2_camera) -> c_int;
    fn fc2_start_preview(camera: *mut fc2_camera, width: c_int, height: c_int) -> c_int;
    fn fc2_stop_preview(camera: *mut fc2_camera);
    fn fc2_render_preview(
        camera: *mut fc2_camera,
        view_width: c_int,
        view_height: c_int,
        rotation_deg: c_int,
    ) -> c_int;
    fn fc2_set_frame_callback(
        camera: *mut fc2_camera,
        callback: Option<extern "C" fn(*mut c_void)>,
        user_data: *mut c_void,
    );
    fn fc2_capture_photo(camera: *mut fc2_camera, path: *const c_char) -> c_int;
}

pub struct Camera2Backend {
    raw: *mut fc2_camera,
}

impl Camera2Backend {
    pub fn new() -> Result<Self, CameraError> {
        // On a non-Halium build the engine is compiled with its host stub and
        // this returns 0.  If libhybris is simply missing at runtime the
        // subsequent open() fails with OpenFailed instead.
        if unsafe { fc2_available() } == 0 {
            return Err(CameraError::Unavailable);
        }

        Ok(Self {
            raw: ptr::null_mut(),
        })
    }
}

// The engine uses this hook to know whether anyone consumes preview frames.
// GtkGLArea drives the renderer through the render vfunc, so the callback only
// needs to exist; it deliberately does nothing (the same approach as the
// upstream camera2_gtk prototype).
extern "C" fn noop_frame_callback(_user_data: *mut c_void) {}

impl CameraBackend for Camera2Backend {
    fn open(&mut self, facing: Facing) -> Result<(), CameraError> {
        self.close();

        let raw = unsafe { fc2_open(facing.fc2_value()) };
        if raw.is_null() {
            return Err(CameraError::OpenFailed);
        }

        unsafe { fc2_set_frame_callback(raw, Some(noop_frame_callback), ptr::null_mut()) };
        self.raw = raw;
        Ok(())
    }

    fn close(&mut self) {
        if !self.raw.is_null() {
            unsafe { fc2_close(self.raw) };
            self.raw = ptr::null_mut();
        }
    }

    fn sensor_orientation(&self) -> i32 {
        if self.raw.is_null() {
            0
        } else {
            unsafe { fc2_sensor_orientation(self.raw) }
        }
    }

    fn start_preview(&mut self, width: i32, height: i32) -> Result<(), CameraError> {
        if self.raw.is_null() {
            return Err(CameraError::NotOpen);
        }

        if unsafe { fc2_start_preview(self.raw, width, height) } != 0 {
            Ok(())
        } else {
            Err(CameraError::PreviewFailed)
        }
    }

    fn stop_preview(&mut self) {
        if !self.raw.is_null() {
            unsafe { fc2_stop_preview(self.raw) };
        }
    }

    fn render_preview(&mut self, width: i32, height: i32, rotation: i32) -> bool {
        if self.raw.is_null() {
            return false;
        }

        unsafe { fc2_render_preview(self.raw, width, height, rotation) != 0 }
    }

    fn capture_photo(&mut self, path: &Path) -> Result<(), CameraError> {
        if self.raw.is_null() {
            return Err(CameraError::NotOpen);
        }

        let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| CameraError::InvalidPath)?;
        if unsafe { fc2_capture_photo(self.raw, path.as_ptr()) } != 0 {
            Ok(())
        } else {
            Err(CameraError::CaptureFailed)
        }
    }
}

impl Drop for Camera2Backend {
    fn drop(&mut self) {
        self.close();
    }
}
