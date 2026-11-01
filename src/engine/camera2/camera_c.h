/* SPDX-License-Identifier: GPL-2.0-only
 * Copyright (C) 2026 Sean Pollard <spollard08@gmail.com>
 *
 * camera_c.h — a backend-neutral C API over the camera engine.
 *
 * This is the seam that lets non-C++/non-Qt frontends (GTK, Rust, Flutter, …)
 * drive the same engine the Qt app uses.  The vocabulary is deliberately
 * generic ("open camera / start preview / set control / capture / record")
 * rather than Camera2-specific, so a future libcamera- or V4L2-backed engine
 * could implement the same API without changing any frontend.
 */

#ifndef FURICAM_CAMERA_C_H
#define FURICAM_CAMERA_C_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct fc2_camera fc2_camera;

#define FC2_FACING_BACK  1
#define FC2_FACING_FRONT 0

/* 1 if a real camera backend is reachable (0 on a non-Halium host stub build). */
int fc2_available(void);

/* One camera reported by fc2_list_cameras(). */
typedef struct fc2_camera_info {
    char id[64];
    int  facing;              /* FC2_FACING_* */
    int  sensor_orientation;  /* degrees clockwise from natural portrait */
} fc2_camera_info;

/* Open the engine and the first camera matching `facing`.  NULL on failure. */
fc2_camera* fc2_open(int facing);

/* List the cameras discovered by an open camera's session.  Call after
 * fc2_open()/fc2_open_id(); returns the number of entries written (up to
 * `max`).  Enumerating through a separate temporary session destabilises some
 * Halium camera HALs, so this deliberately reuses the open session. */
int fc2_list_open_cameras(fc2_camera* c, fc2_camera_info* out, int max);

/* Open a specific camera by the id returned from fc2_list_cameras(). */
fc2_camera* fc2_open_id(const char* id);
void        fc2_close(fc2_camera* c);

/* Sensor mount orientation in degrees — the frontend uses it to rotate preview. */
int fc2_sensor_orientation(fc2_camera* c);

/* Preview (PRIVATE/GPU buffers for the zero-copy GL path).  1 on success.
 * `fc2_start_preview_with_still` can omit the full-resolution JPEG stream,
 * which is not needed while recording and can cap the capture rate. */
int  fc2_start_preview(fc2_camera* c, int width, int height);
int  fc2_start_preview_with_still(fc2_camera* c, int width, int height, int with_still);
void fc2_stop_preview(fc2_camera* c);

/* Draw the latest preview frame into the CURRENT GL context, filling
 * [0,0,view_w,view_h] rotated `rotation_deg`.  Returns 1 if a frame was drawn. */
int  fc2_render_preview(fc2_camera* c, int view_w, int view_h, int rotation_deg);

/* Same, but show only the centred crop_x/crop_y (0..1) sub-rect of the preview
 * stream, so a full-FOV 4:3 stream can fill any photo aspect WYSIWYG. */
int  fc2_render_preview_cropped(fc2_camera* c, int view_w, int view_h, int rotation_deg,
                                float crop_x, float crop_y);

/* Device tilt in degrees clockwise from natural portrait (0/90/180/270).
 * The session uses it to set the JPEG/video orientation tag; call it whenever
 * the device rotates, before capturing. */
void fc2_set_device_rotation(fc2_camera* c, int degrees);

/* Mirror the preview horizontally, for the front/selfie camera. */
void fc2_set_preview_mirror(fc2_camera* c, int on);

/* Called on a camera thread when a new frame is ready (schedule a redraw). */
void fc2_set_frame_callback(fc2_camera* c, void (*cb)(void* user), void* user);

/* Controls. */
void  fc2_set_white_balance(fc2_camera* c, int app_mode); /* 0=Auto 1=Daylight 2=Cloudy 3=Fluor 4=Incand */
void  fc2_set_zoom(fc2_camera* c, float ratio);           /* 1 .. fc2_max_zoom */
float fc2_max_zoom(fc2_camera* c);
void  fc2_set_torch(fc2_camera* c, int on);
void  fc2_set_flash_mode(fc2_camera* c, int mode); /* 0=off 1=on 2=auto */
void  fc2_set_ae_lock(fc2_camera* c, int lock);
void  fc2_set_awb_lock(fc2_camera* c, int lock);
void  fc2_set_auto_exposure(fc2_camera* c);
void  fc2_set_manual_exposure(fc2_camera* c, int iso, int64_t exposure_ns);
void  fc2_set_focus_point(fc2_camera* c, float x, float y); /* normalized [0,1] */

/* Still JPEG output sizes and quality.  Sizes are the camera's advertised
 * output configurations; (0,0) requests the largest.  Quality is [1,100]. */
typedef struct fc2_size {
    int width;
    int height;
} fc2_size;

int  fc2_jpeg_sizes(fc2_camera* c, fc2_size* out, int max);
void fc2_set_jpeg_size(fc2_camera* c, int width, int height);
void fc2_set_jpeg_quality(fc2_camera* c, int quality);

/* AF-assist state used by tap-to-focus.  The frontend polls af_state() and
 * calls set_torch(false) + clear_focus_assist() once focus settles. */
int  fc2_focus_assist_lit(fc2_camera* c);
int  fc2_af_state(fc2_camera* c);
void fc2_clear_focus_assist(fc2_camera* c);

/* Capture / record.  Capture and recording finish asynchronously. */
int  fc2_capture_photo(fc2_camera* c, const char* path);

/* Simultaneous preview + record mode.  Enter it before recording so the
 * viewfinder keeps running; exit it to release the encoder and return to a
 * preview-only session.  Returns 1 on success. */
int  fc2_enter_video_mode(fc2_camera* c, int width, int height, int fps, int bitrate);
void fc2_exit_video_mode(fc2_camera* c);
int  fc2_prepare_recording(fc2_camera* c);   /* warm the mic; 1 on success */
void fc2_release_recording(fc2_camera* c);
int  fc2_is_recording(fc2_camera* c);
int  fc2_recording_write_failed(fc2_camera* c);

/* Result of a still capture.  Called on a camera thread after the JPEG has
 * been written (or after the write failed).  `ok` is 1 on success. */
typedef void (*fc2_photo_callback)(void* user, const char* path, int ok);
void fc2_set_photo_callback(fc2_camera* c, fc2_photo_callback cb, void* user);
int  fc2_start_recording(fc2_camera* c, const char* path, int width, int height, int fps, int bitrate);
void fc2_stop_recording(fc2_camera* c);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* FURICAM_CAMERA_C_H */
