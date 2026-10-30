# Vendored FuriCam Camera2 engine

This directory contains the Qt-free camera engine from FuriCam:

- `CameraSession.*` — Android Camera2 NDK lifecycle, capture and recording
- `PreviewRenderer.*` — zero-copy AImage → EGLImage preview renderer
- `VideoEncoder.*` / `AudioEncoder.*` — MediaCodec/GStreamer recording
- `DngWriter.*` — dependency-free DNG (TIFF/EP) writer for RAW16
- `camera_c.*` — the backend-neutral C API used by this app
- `Camera2NDK.*` / `Camera2NDKShim.c` — the NDK declarations and libhybris shim
- `host/hybris/` — the vendored libhybris binding headers
- `hybris_loader.cpp` — runtime loader for the Flatpak GL extension's libhybris

Upstream: <https://github.com/nico359/furicam>, `src/camera2/`.

## Licensing

The engine files carry `SPDX-License-Identifier: GPL-2.0-only` and are
copyright Sean Pollard and contributors.  This application is currently
`GPL-3.0-or-later`.  **GPL-2.0-only and GPL-3.0-or-later are not compatible
without the copyright holders' permission.**  This vendored copy is for local
experimentation while the app is being validated; before distributing a build
that links it, either obtain permission to relicense the engine or move the
combined work to a GPL-2.0-compatible license.

## Keeping it in sync

The copy is intentionally unmodified apart from this README and the added
`hybris_loader.cpp`.  When the engine changes upstream, re-copy the files and
re-check the build flags in `build.rs`.
