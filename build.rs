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

    // AudioEncoder.cpp builds a stub when FURICAM_HAVE_AUDIO is not defined,
    // which keeps GStreamer out of the build.  Recording is video-only for now.
    build.compile("furicam_camera");

    println!("cargo:rustc-link-lib=EGL");
    println!("cargo:rustc-link-lib=pthread");
    println!("cargo:rustc-link-lib=dl");
}
