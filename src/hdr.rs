// SPDX-License-Identifier: GPL-2.0-or-later
//
// Copyright 2026 nico359

//! HDR exposure fusion for the three-frame bracketed capture.
//!
//! The engine's C++ helper does the OpenCV alignment and Mertens merge.  It
//! downsizes the brackets before alignment, so the working set stays small,
//! and writes the fused result at the captured resolution.

use std::path::{Path, PathBuf};

#[cfg(feature = "camera2")]
use std::ffi::{c_char, c_int, CString};
#[cfg(feature = "camera2")]
use std::os::unix::ffi::OsStrExt;

/// EV steps submitted to the engine.  The neutral frame supplies the EXIF
/// metadata for the fused result.
pub const EV_BRACKETS: [i32; 3] = [-3, 0, 3];

/// Internal fusion resolution.  The long edge is kept around 1280 px
/// regardless of the selected 12 MP or 20 MP output, which keeps the OpenCV
/// working set below the full-resolution merge.
#[cfg(feature = "camera2")]
const WORKING_LONG_EDGE: c_int = 1717;

#[cfg(feature = "camera2")]
extern "C" {
    fn fc2_hdr_fuse(
        paths: *const *const c_char,
        count: c_int,
        output: *const c_char,
        working_long_edge: c_int,
    ) -> c_int;
}

pub fn process_brackets(paths: &[PathBuf], output: &Path) -> Result<(), String> {
    if paths.len() != EV_BRACKETS.len() {
        return Err(format!(
            "expected {} HDR frames, got {}",
            EV_BRACKETS.len(),
            paths.len()
        ));
    }

    #[cfg(not(feature = "camera2"))]
    {
        let _ = output;
        return Err("HDR requires the Camera2 backend".to_string());
    }

    #[cfg(feature = "camera2")]
    {
        let paths: Vec<CString> = paths
            .iter()
            .map(|path| {
                CString::new(path.as_os_str().as_bytes())
                    .map_err(|_| "HDR path contains a NUL byte".to_string())
            })
            .collect::<Result<_, _>>()?;
        let pointers: Vec<*const c_char> = paths.iter().map(|path| path.as_ptr()).collect();
        let output = CString::new(output.as_os_str().as_bytes())
            .map_err(|_| "HDR output path contains a NUL byte".to_string())?;

        let ok = unsafe {
            fc2_hdr_fuse(
                pointers.as_ptr(),
                pointers.len() as c_int,
                output.as_ptr(),
                WORKING_LONG_EDGE,
            )
        };
        if ok == 0 {
            return Err("OpenCV HDR fusion failed".to_string());
        }
    }

    copy_exif(&paths[1], output);
    Ok(())
}

fn copy_exif(source: &Path, output: &Path) {
    match little_exif::metadata::Metadata::new_from_path(source) {
        Ok(metadata) => {
            if let Err(error) = metadata.write_to_file(output) {
                eprintln!("HDR: could not copy EXIF: {error}");
            }
        }
        Err(error) => eprintln!(
            "HDR: could not read EXIF from {}: {error}",
            source.display()
        ),
    }
}
