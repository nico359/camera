// SPDX-License-Identifier: GPL-2.0-or-later
//
// Copyright 2026 nico359
//
// Minimal safe wrapper around FuriCam's backend-neutral `fc2_*` C API.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::ptr;

use super::{CameraBackend, CameraError, CameraInfo, Facing, PhotoSink};

#[repr(C)]
struct fc2_camera {
    _private: [u8; 0],
}

#[repr(C)]
#[derive(Copy, Clone)]
struct fc2_camera_info {
    id: [c_char; 64],
    facing: c_int,
    sensor_orientation: c_int,
}

const FC2_FACING_BACK: c_int = 1;

extern "C" {
    fn fc2_available() -> c_int;
    fn fc2_open(facing: c_int) -> *mut fc2_camera;
    fn fc2_list_open_cameras(
        camera: *mut fc2_camera,
        cameras: *mut fc2_camera_info,
        max: c_int,
    ) -> c_int;
    fn fc2_open_id(id: *const c_char) -> *mut fc2_camera;
    fn fc2_close(camera: *mut fc2_camera);
    fn fc2_sensor_orientation(camera: *mut fc2_camera) -> c_int;
    fn fc2_start_preview_with_still(
        camera: *mut fc2_camera,
        width: c_int,
        height: c_int,
        with_still: c_int,
    ) -> c_int;
    fn fc2_stop_preview(camera: *mut fc2_camera);
    fn fc2_render_preview(
        camera: *mut fc2_camera,
        view_width: c_int,
        view_height: c_int,
        rotation_deg: c_int,
    ) -> c_int;
    fn fc2_set_device_rotation(camera: *mut fc2_camera, degrees: c_int);
    fn fc2_set_preview_mirror(camera: *mut fc2_camera, on: c_int);
    fn fc2_set_frame_callback(
        camera: *mut fc2_camera,
        callback: Option<extern "C" fn(*mut c_void)>,
        user_data: *mut c_void,
    );
    fn fc2_capture_photo(camera: *mut fc2_camera, path: *const c_char) -> c_int;
    fn fc2_start_recording(
        camera: *mut fc2_camera,
        path: *const c_char,
        width: c_int,
        height: c_int,
        fps: c_int,
        bitrate: c_int,
    ) -> c_int;
    fn fc2_stop_recording(camera: *mut fc2_camera);
    fn fc2_enter_video_mode(
        camera: *mut fc2_camera,
        width: c_int,
        height: c_int,
        fps: c_int,
        bitrate: c_int,
    ) -> c_int;
    fn fc2_exit_video_mode(camera: *mut fc2_camera);
    fn fc2_prepare_recording(camera: *mut fc2_camera) -> c_int;
    fn fc2_release_recording(camera: *mut fc2_camera);
    fn fc2_is_recording(camera: *mut fc2_camera) -> c_int;
    fn fc2_recording_write_failed(camera: *mut fc2_camera) -> c_int;
    fn fc2_set_photo_callback(
        camera: *mut fc2_camera,
        callback: Option<extern "C" fn(*mut c_void, *const c_char, c_int)>,
        user_data: *mut c_void,
    );
    fn fc2_max_zoom(camera: *mut fc2_camera) -> f32;
    fn fc2_set_zoom(camera: *mut fc2_camera, ratio: f32);
    fn fc2_set_focus_point(camera: *mut fc2_camera, x: f32, y: f32);
    fn fc2_focus_assist_lit(camera: *mut fc2_camera) -> c_int;
    fn fc2_af_state(camera: *mut fc2_camera) -> c_int;
    fn fc2_clear_focus_assist(camera: *mut fc2_camera);
    fn fc2_set_torch(camera: *mut fc2_camera, on: c_int);
    fn fc2_set_flash_mode(camera: *mut fc2_camera, mode: c_int);
}

/// Stable allocation whose address is handed to the C API.  The `Arc` keeps
/// the callback slot alive even while `CameraSession` recreates backends.
struct PhotoSinkFfi {
    sink: PhotoSink,
}

extern "C" fn photo_callback_trampoline(user_data: *mut c_void, path: *const c_char, ok: c_int) {
    if user_data.is_null() || path.is_null() {
        return;
    }

    // The pointer is `&PhotoSinkFfi` owned by the live `Camera2Backend`; C only
    // invokes it while the backend is open.
    let sink = unsafe { &*(user_data as *const PhotoSinkFfi) };
    let path = unsafe { CStr::from_ptr(path) }
        .to_string_lossy()
        .into_owned();
    let callback = sink
        .sink
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(callback) = callback.as_ref() {
        callback(&path, ok != 0);
    }
}

pub struct Camera2Backend {
    raw: *mut fc2_camera,
    device_rotation: i32,
    mirror: bool,
    photo_sink: Box<PhotoSinkFfi>,
}

impl Camera2Backend {
    pub fn new(photo_sink: PhotoSink) -> Result<Self, CameraError> {
        // On a non-Halium build the engine is compiled with its host stub and
        // this returns 0.  If libhybris is simply missing at runtime the
        // subsequent open() fails with OpenFailed instead.
        if unsafe { fc2_available() } == 0 {
            return Err(CameraError::Unavailable);
        }

        Ok(Self {
            raw: ptr::null_mut(),
            device_rotation: 0,
            mirror: false,
            photo_sink: Box::new(PhotoSinkFfi { sink: photo_sink }),
        })
    }

    fn photo_sink_ptr(&self) -> *mut c_void {
        &*self.photo_sink as *const PhotoSinkFfi as *mut c_void
    }

    fn register_callbacks(&self, raw: *mut fc2_camera) {
        if raw.is_null() {
            return;
        }
        unsafe {
            fc2_set_frame_callback(raw, Some(noop_frame_callback), ptr::null_mut());
            fc2_set_device_rotation(raw, self.device_rotation);
            fc2_set_preview_mirror(raw, self.mirror as c_int);
            fc2_set_photo_callback(raw, Some(photo_callback_trampoline), self.photo_sink_ptr());
        }
    }

    fn read_cameras(&self) -> Vec<CameraInfo> {
        if self.raw.is_null() {
            return Vec::new();
        }

        const MAX_CAMERAS: usize = 8;
        let mut infos: [fc2_camera_info; MAX_CAMERAS] = std::array::from_fn(|_| fc2_camera_info {
            id: [0; 64],
            facing: 0,
            sensor_orientation: 0,
        });

        let count =
            unsafe { fc2_list_open_cameras(self.raw, infos.as_mut_ptr(), MAX_CAMERAS as c_int) };
        infos[..count.max(0) as usize]
            .iter()
            .map(camera_info_from)
            .collect()
    }
}

fn camera_info_from(info: &fc2_camera_info) -> CameraInfo {
    let bytes: Vec<u8> = info
        .id
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();

    CameraInfo {
        id: String::from_utf8_lossy(&bytes).into_owned(),
        facing: if info.facing == 0 {
            Facing::Front
        } else {
            Facing::Back
        },
        sensor_orientation: info.sensor_orientation,
        label: String::new(),
    }
}

// The engine uses this hook to know whether anyone consumes preview frames.
// GtkGLArea drives the renderer through the render vfunc, so the callback only
// needs to exist; it deliberately does nothing (the same approach as the
// upstream camera2_gtk prototype).
extern "C" fn noop_frame_callback(_user_data: *mut c_void) {}

impl CameraBackend for Camera2Backend {
    fn open_first(&mut self) -> Result<Vec<CameraInfo>, CameraError> {
        self.close();

        let raw = unsafe { fc2_open(FC2_FACING_BACK) };
        if raw.is_null() {
            return Err(CameraError::OpenFailed);
        }

        self.raw = raw;
        self.register_callbacks(raw);

        let cameras = self.read_cameras();
        if cameras.is_empty() {
            self.close();
            return Err(CameraError::Unavailable);
        }
        Ok(cameras)
    }

    fn open(&mut self, id: &str) -> Result<(), CameraError> {
        self.close();

        let id = CString::new(id).map_err(|_| CameraError::OpenFailed)?;
        let raw = unsafe { fc2_open_id(id.as_ptr()) };
        if raw.is_null() {
            return Err(CameraError::OpenFailed);
        }

        self.raw = raw;
        self.register_callbacks(raw);
        Ok(())
    }

    fn close(&mut self) {
        if !self.raw.is_null() {
            unsafe {
                fc2_set_photo_callback(self.raw, None, ptr::null_mut());
                fc2_close(self.raw);
            }
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

    fn start_preview(
        &mut self,
        width: i32,
        height: i32,
        with_still: bool,
    ) -> Result<(), CameraError> {
        if self.raw.is_null() {
            return Err(CameraError::NotOpen);
        }

        if unsafe { fc2_start_preview_with_still(self.raw, width, height, with_still as c_int) }
            != 0
        {
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

        let path =
            CString::new(path.as_os_str().as_bytes()).map_err(|_| CameraError::InvalidPath)?;
        if unsafe { fc2_capture_photo(self.raw, path.as_ptr()) } != 0 {
            Ok(())
        } else {
            Err(CameraError::CaptureFailed)
        }
    }

    fn set_device_rotation(&mut self, degrees: i32) {
        self.device_rotation = degrees;
        if !self.raw.is_null() {
            unsafe { fc2_set_device_rotation(self.raw, degrees) };
        }
    }

    fn set_preview_mirror(&mut self, mirror: bool) {
        self.mirror = mirror;
        if !self.raw.is_null() {
            unsafe { fc2_set_preview_mirror(self.raw, mirror as c_int) };
        }
    }

    fn enter_video_mode(
        &mut self,
        width: i32,
        height: i32,
        fps: i32,
        bitrate: i32,
    ) -> Result<(), CameraError> {
        if self.raw.is_null() {
            return Err(CameraError::NotOpen);
        }
        if unsafe { fc2_enter_video_mode(self.raw, width, height, fps, bitrate) } != 0 {
            Ok(())
        } else {
            Err(CameraError::VideoModeFailed)
        }
    }

    fn exit_video_mode(&mut self) {
        if !self.raw.is_null() {
            unsafe { fc2_exit_video_mode(self.raw) };
        }
    }

    fn prepare_recording(&mut self) -> bool {
        !self.raw.is_null() && unsafe { fc2_prepare_recording(self.raw) } != 0
    }

    fn release_recording(&mut self) {
        if !self.raw.is_null() {
            unsafe { fc2_release_recording(self.raw) };
        }
    }

    fn start_recording(
        &mut self,
        path: &Path,
        width: i32,
        height: i32,
        fps: i32,
        bitrate: i32,
    ) -> Result<(), CameraError> {
        if self.raw.is_null() {
            return Err(CameraError::NotOpen);
        }
        let path =
            CString::new(path.as_os_str().as_bytes()).map_err(|_| CameraError::InvalidPath)?;
        if unsafe { fc2_start_recording(self.raw, path.as_ptr(), width, height, fps, bitrate) } != 0
        {
            Ok(())
        } else {
            Err(CameraError::RecordingFailed)
        }
    }

    fn stop_recording(&mut self) {
        if !self.raw.is_null() {
            unsafe { fc2_stop_recording(self.raw) };
        }
    }

    fn is_recording(&self) -> bool {
        !self.raw.is_null() && unsafe { fc2_is_recording(self.raw) } != 0
    }

    fn recording_write_failed(&self) -> bool {
        !self.raw.is_null() && unsafe { fc2_recording_write_failed(self.raw) } != 0
    }

    fn max_zoom(&self) -> f32 {
        if self.raw.is_null() {
            1.0
        } else {
            unsafe { fc2_max_zoom(self.raw) }.max(1.0)
        }
    }

    fn set_zoom(&mut self, ratio: f32) {
        if !self.raw.is_null() {
            unsafe { fc2_set_zoom(self.raw, ratio) };
        }
    }

    fn set_focus_point(&mut self, x: f32, y: f32) {
        if !self.raw.is_null() {
            unsafe { fc2_set_focus_point(self.raw, x, y) };
        }
    }

    fn focus_assist_lit(&self) -> bool {
        !self.raw.is_null() && unsafe { fc2_focus_assist_lit(self.raw) } != 0
    }

    fn af_state(&self) -> i32 {
        if self.raw.is_null() {
            0
        } else {
            unsafe { fc2_af_state(self.raw) }
        }
    }

    fn clear_focus_assist(&mut self) {
        if !self.raw.is_null() {
            unsafe { fc2_clear_focus_assist(self.raw) };
        }
    }

    fn set_torch(&mut self, on: bool) {
        if !self.raw.is_null() {
            unsafe { fc2_set_torch(self.raw, on as c_int) };
        }
    }

    fn set_flash_mode(&mut self, mode: i32) {
        if !self.raw.is_null() {
            unsafe { fc2_set_flash_mode(self.raw, mode) };
        }
    }
}

impl Drop for Camera2Backend {
    fn drop(&mut self) {
        self.close();
    }
}
