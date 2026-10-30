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
copyright Sean Pollard and contributors.  The application is
`GPL-2.0-or-later`.  The combined work is distributed under **GPL-2.0**: the
application code can also be used under GPL-3.0-or-later on its own, but a
binary that links this GPL-2.0-only engine cannot be distributed under GPL-3.0
without permission from the engine copyright holders.

## Keeping it in sync

The copy has local modifications for the GTK frontend: `camera_c.{h,cpp}`
expose asynchronous still-capture results, and `hybris_loader.cpp` is added.
When the engine changes upstream, re-copy the files, re-apply those changes,
and re-check the build flags in `build.rs`.
