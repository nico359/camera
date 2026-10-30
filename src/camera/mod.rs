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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Facing {
    Front,
    #[default]
    Back,
}

impl Facing {
    pub fn opposite(self) -> Self {
        match self {
            Self::Front => Self::Back,
            Self::Back => Self::Front,
        }
    }

    /// Value expected by FuriCam's `FC2_FACING_*` API.
    #[cfg(feature = "camera2")]
    fn fc2_value(self) -> i32 {
        match self {
            // FC2_FACING_FRONT is 0, FC2_FACING_BACK is 1.
            Self::Front => 0,
            Self::Back => 1,
        }
    }
}

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
    fn open(&mut self, facing: Facing) -> Result<(), CameraError>;
    fn close(&mut self);
    fn sensor_orientation(&self) -> i32;
    fn start_preview(&mut self, width: i32, height: i32) -> Result<(), CameraError>;
    fn stop_preview(&mut self);
    fn render_preview(&mut self, width: i32, height: i32, rotation: i32) -> bool;
    fn capture_photo(&mut self, path: &Path) -> Result<(), CameraError>;
}

fn create_backend() -> Result<Box<dyn CameraBackend>, CameraError> {
    #[cfg(feature = "camera2")]
    {
        Ok(Box::new(camera2::Camera2Backend::new()?))
    }

    #[cfg(not(feature = "camera2"))]
    {
        Err(CameraError::Unavailable)
    }
}

/// Owns the active backend and exposes the small surface the UI needs.
#[derive(Default)]
pub struct CameraSession {
    backend: Option<Box<dyn CameraBackend>>,
    facing: Facing,
}

impl CameraSession {
    pub fn facing(&self) -> Facing {
        self.facing
    }

    pub fn is_open(&self) -> bool {
        self.backend.is_some()
    }

    /// Open `facing`, replacing any previously open camera.
    pub fn open(&mut self, facing: Facing) -> Result<(), CameraError> {
        self.close();

        let mut backend = create_backend()?;
        backend.open(facing)?;

        self.facing = facing;
        self.backend = Some(backend);
        Ok(())
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
