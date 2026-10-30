# Camera

A GTK4/libadwaita camera application for GNOME and Halium-based Linux phones.

The app is deliberately small at this stage: it proves the FuriCam Camera2
engine can drive a native GTK4 preview and full-resolution still capture inside
a Flatpak, and it leaves room for a V4L2/libcamera backend later.

## Status

Implemented:

- live preview through `GtkGLArea`, letterboxed to the still aspect ratio and
  redrawn at the engine's capture rate (30 fps)
- full-resolution JPEG capture (the engine asks the sensor for its largest
  JPEG stream; on the FuriPhone this is 5152×3864)
- asynchronous capture feedback: the UI reports when the JPEG is actually
  written, not when the request is submitted
- a last-shot thumbnail that opens the photo in the default viewer; an
  optional “Ask Which App to Open Photos” menu toggle enables the chooser
- portrait-locked preview with its aspect ratio derived from the active
  sensor orientation
- device-rotation-aware still capture through iio-sensor-proxy
- separate flip (front/back) and camera-picker buttons; the picker includes
  every enumerated camera, including the secondary back camera
- front-camera preview mirroring
- a Flatpak manifest with the permissions the Halium camera stack needs

Not implemented yet:

- video recording, audio, gallery, settings, grid/level, HDR, RAW, manual
  controls.  The engine already exposes most of these through its C API; the UI
  is the remaining work.

## Building

### Desktop / non-Halium

The default build does not compile the Camera2 engine; it uses a no-camera
backend so the GTK UI can be developed and tested without libhybris.

```sh
meson setup build
meson compile -C build
```

### FuriOS / FuriPhone Flatpak

The Flatpak build enables the Camera2 backend:

```sh
sudo flatpak-builder --system --install --force-clean builddir io.github.nico359.camera.json
flatpak run io.github.nico359.camera
```

Use `--user` instead of `--system` if the SDK, runtime and Rust extension are
installed in the user installation.  The SDK and extension are system-wide on
FuriOS, so `--system` is the default here.

Requirements:

- `org.gnome.Sdk//51` and the matching Rust extension.  The SDK metadata
  declares `org.freedesktop.Sdk.Extension` version `26.08`, so install
  `org.freedesktop.Sdk.Extension.rust-stable//26.08`.
- On FuriOS, the Flatpak setup already injects `/android`, `/system`, `/vendor`
  and `/dev/binder` into every sandbox.  The manifest only asks for
  `--device=all`; without that FuriOS integration the app can build but will
  not find the camera HAL.
- The Flatpak is self-hosted on purpose.  `--device=all` and the Halium paths
  make it unsuitable for Flathub.
- The manifest also asks for `--talk-name=org.freedesktop.Flatpak`.  The
  OpenURI portal always shows an app chooser for local files, so the thumbnail
  needs a narrow `flatpak-spawn --host gio open` path to honor “open in the
  default viewer”.  The “Ask Which App to Open Photos” menu toggle switches
  back to the portal chooser.

The camera engine links `libhybris` at runtime through the
`org.freedesktop.Platform.GL.hybris` extension; it does not copy host libraries
into `/app`.

## Configuration

`meson_options.txt` has one option:

```sh
meson setup build -Dcamera2=true
```

`camera2` is only useful on Halium devices.  It compiles the vendored engine in
`src/engine/camera2` and requires EGL headers in the build environment.

## Licensing

The application is **GPL-2.0-or-later**.  The vendored FuriCam Camera2 engine
is **GPL-2.0-only**; see `src/engine/camera2/README.md`.  A binary that links
the engine is distributed under **GPL-2.0**.  The app code alone may also be
used under GPL-3.0-or-later, but distributing the combined work under GPL-3.0
would require permission from the engine copyright holders.

FuriCam itself is based on FuriOS-camera (`GPL-2.0-only`, Furi Labs/Droidian
Project) and remains GPL-2.0-only unless its copyright holders agree otherwise.
This is not legal advice.
