// SPDX-License-Identifier: GPL-2.0-or-later
//
// Copyright 2026 nico359

//! Build script for the optional FuriCam Camera2 backend.
//!
//! The engine is vendored under `src/engine/camera2` and is compiled
//! with the `cc` crate so the whole application remains a normal Cargo project.
//! It is deliberately only built when the `camera2` feature is enabled: desktop
//! builds keep their system dependencies small, and the Halium/FuriPhone
//! Flatpak is the only place that needs the Android NDK shim and libhybris.

fn main() {
    println!("cargo:rerun-if-changed=src/engine/camera2");

    if std::env::var_os("CARGO_FEATURE_CAMERA2").is_none() {
        return;
    }

    let engine = "src/engine/camera2";

    // Camera2NDKShim.c has to be compiled as C: its libhybris macros assign a
    // void* to function pointers, which is valid C but ill-formed C++ (the
    // upstream engine documents the same constraint).  Compile just that file
    // with the C compiler and fold the object into the C++ archive.
    let out_dir = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let shim_object = out_dir.join("Camera2NDKShim.o");
    let compiler = cc::Build::new().get_compiler();
    let mut cmd = compiler.to_command();
    cmd.arg("-c")
        .arg(format!("{engine}/Camera2NDKShim.c"))
        .arg("-o")
        .arg(&shim_object)
        .arg("-I")
        .arg(engine)
        .arg("-I")
        .arg(format!("{engine}/host"))
        .arg("-O2")
        .arg("-fPIC")
        .arg("-pthread");
    let status = cmd.status().expect("failed to run the C compiler");
    assert!(status.success(), "C compiler failed for Camera2NDKShim.c");

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .include(engine)
        .include(format!("{engine}/host"))
        .flag_if_supported("-fPIC")
        .flag_if_supported("-pthread")
        .warnings(false)
        .object(&shim_object)
        // The engine's C API is the only surface the Rust side touches.
        .file(format!("{engine}/CameraSession.cpp"))
        .file(format!("{engine}/DngWriter.cpp"))
        .file(format!("{engine}/VideoEncoder.cpp"))
        .file(format!("{engine}/AudioEncoder.cpp"))
        .file(format!("{engine}/PreviewRenderer.cpp"))
        .file(format!("{engine}/camera_c.cpp"))
        .file(format!("{engine}/hybris_loader.cpp"));

    // Enable AAC audio recording when the GStreamer development files are
    // available.  The Flatpak SDK ships them; desktop builds without them fall
    // back to the video-only AudioEncoder stub.
    let gst = pkg_config::probe_library("gstreamer-1.0");
    let gst_app = pkg_config::probe_library("gstreamer-app-1.0");
    let mut audio_libs = Vec::new();
    if let (Ok(gst), Ok(gst_app)) = (gst, gst_app) {
        build.includes(gst.include_paths);
        build.includes(gst_app.include_paths);
        build.define("FURICAM_HAVE_AUDIO", None);
        audio_libs.extend(gst.libs);
        audio_libs.extend(gst_app.libs);
        println!("cargo:warning=GStreamer found: audio recording enabled");
    } else {
        println!("cargo:warning=GStreamer dev files not found: video-only recording");
    }

    build.compile("furicam_camera");

    // The cc archive is linked after Cargo's native-lib list, so emit the
    // GStreamer libs again here to satisfy the static engine's symbols
    // (otherwise --as-needed drops them before it sees the archive).
    audio_libs.sort();
    audio_libs.dedup();
    for lib in audio_libs {
        println!("cargo:rustc-link-lib=dylib={lib}");
    }

    println!("cargo:rustc-link-lib=EGL");
    println!("cargo:rustc-link-lib=pthread");
    println!("cargo:rustc-link-lib=dl");
}
