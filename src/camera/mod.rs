// SPDX-License-Identifier: GPL-3.0-or-later
//
// Copyright 2026 nico359
//
// Camera abstraction used by the UI.  The actual capture engine is behind the
// `CameraBackend` trait so a future V4L2/libcamera backend can be added without
// touching the window or preview widget.

#[cfg(feature = "camera2")]
mod camera2;

use std::fmt;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Facing {
    Front,
    #[default]
    Back,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraInfo {
    pub id: String,
    pub facing: Facing,
    pub sensor_orientation: i32,
    /// Human-readable label assigned by `CameraSession` ("Back Camera 2", …).
    pub label: String,
}

/// Called after a still has been written (or failed to write).  The callback
/// runs on the camera/writer thread, so the UI layer must marshal it to the
/// main context before touching widgets.
pub type PhotoCallback = Box<dyn Fn(&str, bool) + Send + Sync + 'static>;

/// Shared slot between `CameraSession` and the C trampoline.  It is kept
/// separate from the backend so changing the callback never requires reopening
/// the camera.
pub(crate) type PhotoSink = Arc<Mutex<Option<PhotoCallback>>>;

#[derive(Debug)]
pub enum CameraError {
    Unavailable,
    NotOpen,
    OpenFailed,
    PreviewFailed,
    CaptureFailed,
    InvalidPath,
    Gl(String),
}

impl fmt::Display for CameraError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => f.write_str("no camera backend is available"),
            Self::NotOpen => f.write_str("the camera is not open"),
            Self::OpenFailed => f.write_str("the camera could not be opened"),
            Self::PreviewFailed => f.write_str("the preview could not be started"),
            Self::CaptureFailed => f.write_str("the photo could not be captured"),
            Self::InvalidPath => f.write_str("the destination path is not valid"),
            Self::Gl(message) => write!(f, "OpenGL error: {message}"),
        }
    }
}

impl std::error::Error for CameraError {}

pub trait CameraBackend {
    fn open_first(&mut self) -> Result<Vec<CameraInfo>, CameraError>;
    fn open(&mut self, id: &str) -> Result<(), CameraError>;
    fn close(&mut self);
    fn sensor_orientation(&self) -> i32;
    fn start_preview(&mut self, width: i32, height: i32) -> Result<(), CameraError>;
    fn stop_preview(&mut self);
    fn render_preview(&mut self, width: i32, height: i32, rotation: i32) -> bool;
    fn capture_photo(&mut self, path: &Path) -> Result<(), CameraError>;
    fn set_device_rotation(&mut self, degrees: i32);
    fn set_preview_mirror(&mut self, mirror: bool);
}

fn create_backend(photo_sink: PhotoSink) -> Result<Box<dyn CameraBackend>, CameraError> {
    #[cfg(feature = "camera2")]
    {
        Ok(Box::new(camera2::Camera2Backend::new(photo_sink)?))
    }

    #[cfg(not(feature = "camera2"))]
    {
        let _ = photo_sink;
        Err(CameraError::Unavailable)
    }
}

fn label_cameras(cameras: &mut [CameraInfo]) {
    let mut back = 0;
    let mut front = 0;

    for camera in cameras {
        match camera.facing {
            Facing::Back => {
                back += 1;
                camera.label = if back == 1 {
                    "Back Camera".to_string()
                } else {
                    format!("Back Camera {back}")
                };
            }
            Facing::Front => {
                front += 1;
                camera.label = if front == 1 {
                    "Front Camera".to_string()
                } else {
                    format!("Front Camera {front}")
                };
            }
        }
    }
}

/// Owns the active backend and exposes the small surface the UI needs.
#[derive(Default)]
pub struct CameraSession {
    backend: Option<Box<dyn CameraBackend>>,
    cameras: Vec<CameraInfo>,
    current: usize,
    device_rotation: i32,
    photo_sink: PhotoSink,
}

impl CameraSession {
    pub fn current(&self) -> Option<&CameraInfo> {
        self.cameras.get(self.current)
    }

    pub fn is_open(&self) -> bool {
        self.backend.is_some()
    }

    pub fn set_device_rotation(&mut self, degrees: i32) {
        self.device_rotation = degrees;
        if let Some(backend) = self.backend.as_mut() {
            backend.set_device_rotation(degrees);
        }
    }

    /// Install (or clear) the still-capture result callback.  Safe to call
    /// before or after opening; the sink is shared with any backend created
    /// later.
    pub fn set_photo_callback(&self, callback: Option<PhotoCallback>) {
        *self
            .photo_sink
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = callback;
    }

    /// Open the first back camera (or the first camera if there is no back one).
    pub fn open_first(&mut self) -> Result<(), CameraError> {
        self.close();

        let mut backend = create_backend(Arc::clone(&self.photo_sink))?;
        let mut cameras = backend.open_first()?;
        label_cameras(&mut cameras);

        let index = cameras
            .iter()
            .position(|camera| camera.facing == Facing::Back)
            .unwrap_or(0);

        self.cameras = cameras;
        self.current = index;
        self.backend = Some(backend);
        self.apply_controls();
        Ok(())
    }

    /// Cycle to the next enumerated camera (back, front, back 2, …).
    pub fn switch(&mut self) -> Result<CameraInfo, CameraError> {
        if self.cameras.is_empty() {
            return Err(CameraError::NotOpen);
        }

        let next = (self.current + 1) % self.cameras.len();
        {
            let backend = self.backend.as_mut().ok_or(CameraError::NotOpen)?;
            backend.open(&self.cameras[next].id)?;
        }

        self.current = next;
        self.apply_controls();
        Ok(self.cameras[next].clone())
    }

    fn apply_controls(&mut self) {
        let degrees = self.device_rotation;
        let mirror = self
            .current()
            .map(|camera| camera.facing == Facing::Front)
            .unwrap_or(false);

        if let Some(backend) = self.backend.as_mut() {
            backend.set_device_rotation(degrees);
            backend.set_preview_mirror(mirror);
        }
    }

    pub fn start_preview(&mut self, width: i32, height: i32) -> Result<(), CameraError> {
        self.backend
            .as_mut()
            .ok_or(CameraError::NotOpen)?
            .start_preview(width, height)
    }

    pub fn render_preview(&mut self, width: i32, height: i32, rotation: i32) -> bool {
        self.backend
            .as_mut()
            .map(|backend| backend.render_preview(width, height, rotation))
            .unwrap_or(false)
    }

    pub fn sensor_orientation(&self) -> i32 {
        self.backend
            .as_ref()
            .map(|backend| backend.sensor_orientation())
            .unwrap_or(0)
    }

    pub fn capture_photo(&mut self, path: &Path) -> Result<(), CameraError> {
        self.backend
            .as_mut()
            .ok_or(CameraError::NotOpen)?
            .capture_photo(path)
    }

    pub fn close(&mut self) {
        if let Some(mut backend) = self.backend.take() {
            backend.stop_preview();
            backend.close();
        }
    }
}

impl Drop for CameraSession {
    fn drop(&mut self) {
        self.close();
    }
}
