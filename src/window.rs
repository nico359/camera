/* window.rs
 *
 * Copyright 2026 nico359
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 2 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};

use crate::level_sensor::LevelSensor;
use crate::settings::AppSettings;
use crate::ui::CameraView;

/// 3x3 composition grid, drawn over the preview image itself.
fn draw_grid(cr: &gtk::cairo::Context, width: f64, height: f64) {
    for i in 1..3 {
        let x = width * i as f64 / 3.0;
        cr.move_to(x, 0.0);
        cr.line_to(x, height);
        let _ = cr.stroke();

        let y = height * i as f64 / 3.0;
        cr.move_to(0.0, y);
        cr.line_to(width, y);
        let _ = cr.stroke();
    }
}

/// Render one decoded video frame at up to `max_size` on its longest edge.
fn snapshot_media(
    stream: &gtk::MediaFile,
    max_size: f64,
    renderer: &gtk::gsk::Renderer,
) -> Option<gtk::gdk::Texture> {
    let width = stream.intrinsic_width() as f64;
    let height = stream.intrinsic_height() as f64;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }

    let scale = (max_size / width.max(height)).min(1.0);
    let snapshot = gtk::Snapshot::new();
    stream.snapshot(&snapshot, width * scale, height * scale);
    let node = snapshot.to_node()?;
    Some(renderer.render_texture(&node, None))
}

/// Render a square, centre-cropped video frame for the last-media thumbnail.
fn snapshot_media_square(
    stream: &gtk::MediaFile,
    size: f64,
    renderer: &gtk::gsk::Renderer,
) -> Option<gtk::gdk::Texture> {
    let width = stream.intrinsic_width() as f64;
    let height = stream.intrinsic_height() as f64;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }

    // Cover the square, then crop the overflow.
    let scale = (size / width).max(size / height);
    let draw_width = width * scale;
    let draw_height = height * scale;
    let snapshot = gtk::Snapshot::new();
    snapshot.push_clip(&gtk::graphene::Rect::new(
        0.0,
        0.0,
        size as f32,
        size as f32,
    ));
    snapshot.translate(&gtk::graphene::Point::new(
        ((size - draw_width) / 2.0) as f32,
        ((size - draw_height) / 2.0) as f32,
    ));
    stream.snapshot(&snapshot, draw_width, draw_height);
    snapshot.pop();
    let node = snapshot.to_node()?;
    Some(renderer.render_texture(&node, None))
}

fn draw_level_segments(cr: &gtk::cairo::Context, half: f64, gap: f64) {
    cr.move_to(-half, 0.0);
    cr.line_to(-gap, 0.0);
    let _ = cr.stroke();

    cr.move_to(gap, 0.0);
    cr.line_to(half, 0.0);
    let _ = cr.stroke();
}

/// FuriCam-style rotating level line. `angle` is the absolute roll angle from
/// the accelerometer; the line turns with the device and is green when within
/// 1.5° of horizontal in any of the four orientations.
fn draw_level(cr: &gtk::cairo::Context, width: f64, height: f64, angle: f64) {
    let center_x = width / 2.0;
    let center_y = height / 2.0;
    let half = (width * 0.18).min(90.0);
    let gap = 6.0;

    let normalized = angle.rem_euclid(360.0);
    let nearest = (normalized / 90.0).round() * 90.0;
    let error = (normalized - nearest).abs();
    let level = error < 1.5 || (360.0 - error) < 1.5;

    let _ = cr.save();
    cr.translate(center_x, center_y);
    cr.rotate(angle.to_radians());

    cr.set_source_rgba(0.0, 0.0, 0.0, 0.35);
    cr.set_line_width(4.0);
    draw_level_segments(cr, half, gap);

    if level {
        cr.set_source_rgba(0.2, 0.82, 0.4, 0.95);
    } else {
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.65);
    }
    cr.set_line_width(2.0);
    draw_level_segments(cr, half, gap);

    let _ = cr.restore();
}

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/nico359/camera/window.ui")]
    pub struct CameraWindow {
        #[template_child]
        pub navigation_view: TemplateChild<adw::NavigationView>,

        #[template_child]
        pub camera_view: TemplateChild<CameraView>,

        #[template_child]
        pub preview_frame: TemplateChild<gtk::AspectFrame>,

        #[template_child]
        pub grid_overlay: TemplateChild<gtk::DrawingArea>,

        #[template_child]
        pub level_overlay: TemplateChild<gtk::DrawingArea>,

        #[template_child]
        pub preview_overlay: TemplateChild<gtk::Overlay>,

        #[template_child]
        pub camera_menu_button: TemplateChild<gtk::MenuButton>,

        #[template_child]
        pub flash_menu_button: TemplateChild<gtk::MenuButton>,

        #[template_child]
        pub last_photo_button: TemplateChild<gtk::Button>,

        #[template_child]
        pub last_media_stack: TemplateChild<gtk::Stack>,

        #[template_child]
        pub last_photo_picture: TemplateChild<gtk::Picture>,

        #[template_child]
        pub last_video_picture: TemplateChild<gtk::Picture>,

        #[template_child]
        pub last_video_icon: TemplateChild<gtk::Image>,

        #[template_child]
        pub shutter_button: TemplateChild<gtk::Button>,

        #[template_child]
        pub shutter_image: TemplateChild<gtk::Image>,

        #[template_child]
        pub video_mode_button: TemplateChild<gtk::Button>,

        #[template_child]
        pub video_mode_image: TemplateChild<gtk::Image>,

        #[template_child]
        pub recording_label: TemplateChild<gtk::Label>,

        #[template_child]
        pub zoom_label: TemplateChild<gtk::Label>,

        #[template_child]
        pub focus_reticle: TemplateChild<gtk::Box>,

        #[template_child]
        pub preview_panel_spacer: TemplateChild<gtk::Box>,

        #[template_child]
        pub status_label: TemplateChild<gtk::Label>,

        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,

        #[template_child]
        pub shutter_flash: TemplateChild<gtk::Box>,

        #[template_child]
        pub gps_indicator: TemplateChild<gtk::Image>,

        #[template_child]
        pub manual_controls_button: TemplateChild<gtk::ToggleButton>,

        #[template_child]
        pub manual_revealer: TemplateChild<gtk::Revealer>,

        #[template_child]
        pub iso_scale: TemplateChild<gtk::Scale>,

        #[template_child]
        pub iso_value: TemplateChild<gtk::Label>,

        #[template_child]
        pub shutter_scale: TemplateChild<gtk::Scale>,

        #[template_child]
        pub shutter_value: TemplateChild<gtk::Label>,

        #[template_child]
        pub exposure_auto_button: TemplateChild<gtk::ToggleButton>,

        #[template_child]
        pub ev_control: TemplateChild<gtk::Box>,

        #[template_child]
        pub ev_touch_scale: TemplateChild<gtk::Scale>,

        #[template_child]
        pub ae_lock_button: TemplateChild<gtk::ToggleButton>,

        #[template_child]
        pub awb_lock_button: TemplateChild<gtk::ToggleButton>,

        pub(super) capture_action: RefCell<Option<gio::SimpleAction>>,
        pub(super) switch_action: RefCell<Option<gio::SimpleAction>>,
        pub(super) select_action: RefCell<Option<gio::SimpleAction>>,
        pub(super) video_mode_action: RefCell<Option<gio::SimpleAction>>,
        pub(super) flash_mode: Cell<i32>,
        pub(super) recording_started: Cell<Option<Instant>>,
        pub(super) recording_tick: RefCell<Option<glib::SourceId>>,
        pub(super) shutter_flash_tick: RefCell<Option<glib::SourceId>>,
        pub(super) recording_path: RefCell<Option<PathBuf>>,
        pub(super) zoom_gesture_start: Cell<Option<f32>>,
        pub(super) tap_start: Cell<Option<(f64, f64)>>,
        pub(super) suppress_tap_until: Cell<Option<Instant>>,
        pub(super) focus_tick: RefCell<Option<glib::SourceId>>,
        pub(super) focus_assist_tick: RefCell<Option<glib::SourceId>>,
        pub(super) settings: AppSettings,
        pub(super) level_sensor: RefCell<Option<Rc<LevelSensor>>>,
        pub(super) location_stop: RefCell<Option<Arc<AtomicBool>>>,
        pub(super) location_rx:
            RefCell<Option<std::sync::mpsc::Receiver<crate::location::LocationFix>>>,
        pub(super) location_fix: RefCell<Option<crate::location::LocationFix>>,
        pub(super) manual_limits: RefCell<crate::camera::ManualControls>,
        pub(super) last_video_media: RefCell<Option<gtk::MediaFile>>,
        pub(super) manual_active: Cell<bool>,
        pub(super) manual_syncing: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CameraWindow {
        const NAME: &'static str = "CameraWindow";
        type Type = super::CameraWindow;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for CameraWindow {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().add_css_class("camera-window");
            self.setup_actions();
            self.setup_status();
            self.setup_gestures();
            self.setup_grid();
            self.setup_level();
            self.setup_manual();
            self.setup_location();
            self.obj().load_latest_media();
        }
    }

    impl WidgetImpl for CameraWindow {}
    impl WindowImpl for CameraWindow {}
    impl ApplicationWindowImpl for CameraWindow {}
    impl AdwApplicationWindowImpl for CameraWindow {}

    impl CameraWindow {
        fn setup_grid(&self) {
            let grid = self.grid_overlay.get();
            grid.set_draw_func(|_, cr, width, height| {
                let width = width as f64;
                let height = height as f64;

                cr.set_source_rgba(0.0, 0.0, 0.0, 0.35);
                cr.set_line_width(2.0);
                draw_grid(cr, width, height);

                cr.set_source_rgba(1.0, 1.0, 1.0, 0.55);
                cr.set_line_width(1.0);
                draw_grid(cr, width, height);
            });

            let settings = AppSettings::new();
            grid.set_visible(settings.show_grid());
            settings.gio().bind("show-grid", &grid, "visible").build();
        }

        fn setup_level(&self) {
            let level = self.level_overlay.get();
            let angle = Rc::new(Cell::new(0.0_f64));
            let angle_draw = Rc::clone(&angle);
            level.set_draw_func(move |_, cr, width, height| {
                draw_level(cr, width as f64, height as f64, angle_draw.get());
            });

            let level_draw = level.clone();
            let angle_cb = Rc::clone(&angle);
            let sensor = Rc::new(LevelSensor::new(move |reading| {
                let current = angle_cb.get();
                let mut delta = reading - current;
                while delta > 180.0 {
                    delta -= 360.0;
                }
                while delta < -180.0 {
                    delta += 360.0;
                }
                angle_cb.set((current + delta * 0.25).rem_euclid(360.0));
                level_draw.queue_draw();
            }));

            let show = self.settings.show_level();
            level.set_visible(show);
            if show {
                sensor.set_active(true);
            }
            *self.level_sensor.borrow_mut() = Some(sensor);

            let weak = self.obj().downgrade();
            self.settings
                .gio()
                .connect_changed(Some("show-level"), move |settings, _| {
                    let show = settings.boolean("show-level");
                    if let Some(window) = weak.upgrade() {
                        let imp = window.imp();
                        imp.level_overlay.get().set_visible(show);
                        if let Some(sensor) = imp.level_sensor.borrow().as_ref() {
                            sensor.set_active(show);
                        }
                    }
                });
        }

        fn setup_manual(&self) {
            let weak = self.obj().downgrade();
            self.manual_controls_button.connect_toggled(move |button| {
                if let Some(window) = weak.upgrade() {
                    window
                        .imp()
                        .manual_revealer
                        .set_reveal_child(button.is_active());
                    window.update_preview_panel_margin(button.is_active());
                }
            });

            for scale in [&self.iso_scale, &self.shutter_scale] {
                let weak = self.obj().downgrade();
                scale.connect_value_changed(move |_| {
                    if let Some(window) = weak.upgrade() {
                        if !window.imp().manual_syncing.get() {
                            window.apply_manual_exposure();
                        }
                    }
                });
            }

            let weak = self.obj().downgrade();
            self.exposure_auto_button.connect_clicked(move |button| {
                if let Some(window) = weak.upgrade() {
                    if button.is_active() {
                        window.imp().camera_view.get().set_auto_exposure();
                        window.imp().manual_active.set(false);
                        window.update_manual_sensitivity();
                    } else {
                        window.apply_manual_exposure();
                    }
                }
            });

            let weak = self.obj().downgrade();
            self.ev_touch_scale.connect_value_changed(move |_| {
                if let Some(window) = weak.upgrade() {
                    if !window.imp().manual_syncing.get() {
                        window.apply_ev_touch();
                    }
                }
            });

            let weak = self.obj().downgrade();
            self.ae_lock_button.connect_toggled(move |button| {
                if let Some(window) = weak.upgrade() {
                    window
                        .imp()
                        .camera_view
                        .get()
                        .set_ae_lock(button.is_active());
                }
            });

            let weak = self.obj().downgrade();
            self.awb_lock_button.connect_toggled(move |button| {
                if let Some(window) = weak.upgrade() {
                    window
                        .imp()
                        .camera_view
                        .get()
                        .set_awb_lock(button.is_active());
                }
            });
        }

        fn setup_location(&self) {
            // Poll fixes from the portal thread on the main loop. Keeping the
            // latest fix in memory (never on disk) lets a capture use a fresh
            // position without ever reusing a stale one.
            let weak = self.obj().downgrade();
            glib::timeout_add_local(Duration::from_millis(500), move || {
                if let Some(window) = weak.upgrade() {
                    let imp = window.imp();
                    if let Some(rx) = imp.location_rx.borrow().as_ref() {
                        let mut latest = None;
                        while let Ok(fix) = rx.try_recv() {
                            latest = Some(fix);
                        }
                        if let Some(fix) = latest {
                            *imp.location_fix.borrow_mut() = Some(fix);
                        }
                    }
                    window.update_gps_indicator();
                }
                glib::ControlFlow::Continue
            });

            let weak = self.obj().downgrade();
            self.settings
                .gio()
                .connect_changed(Some("geotag-photos"), move |_, _| {
                    if let Some(window) = weak.upgrade() {
                        if window.imp().settings.geotag_photos() {
                            window.start_location();
                        } else {
                            window.stop_location();
                        }
                    }
                });

            if self.settings.geotag_photos() {
                self.obj().start_location();
            }
        }

        fn setup_actions(&self) {
            let window = self.obj();

            let capture = gio::SimpleAction::new("capture", None);
            capture.set_enabled(false);
            let weak = window.downgrade();
            capture.connect_activate(move |_, _| {
                if let Some(window) = weak.upgrade() {
                    window.shutter();
                }
            });
            window.add_action(&capture);

            let switch = gio::SimpleAction::new("switch-camera", None);
            switch.set_enabled(false);
            let weak = window.downgrade();
            switch.connect_activate(move |_, _| {
                if let Some(window) = weak.upgrade() {
                    window.flip_camera();
                }
            });
            window.add_action(&switch);

            let select = gio::SimpleAction::new("select-camera", Some(glib::VariantTy::INT32));
            select.set_enabled(false);
            let weak = window.downgrade();
            select.connect_activate(move |_, parameter| {
                let Some(index) = parameter.and_then(|value| value.get::<i32>()) else {
                    return;
                };
                if let Some(window) = weak.upgrade() {
                    window.select_camera(index.max(0) as usize);
                }
            });
            window.add_action(&select);

            let video_mode = gio::SimpleAction::new("toggle-video-mode", None);
            video_mode.set_enabled(false);
            let weak = window.downgrade();
            video_mode.connect_activate(move |_, _| {
                if let Some(window) = weak.upgrade() {
                    window.toggle_video_mode();
                }
            });
            window.add_action(&video_mode);

            let flash = gio::SimpleAction::new("set-flash-mode", Some(glib::VariantTy::INT32));
            let weak = window.downgrade();
            flash.connect_activate(move |_, parameter| {
                let Some(mode) = parameter.and_then(|value| value.get::<i32>()) else {
                    return;
                };
                if let Some(window) = weak.upgrade() {
                    window.set_flash_mode(mode);
                }
            });
            window.add_action(&flash);

            let zoom_in = gio::SimpleAction::new("zoom-in", None);
            let weak = window.downgrade();
            zoom_in.connect_activate(move |_, _| {
                if let Some(window) = weak.upgrade() {
                    window.zoom_by(0.5);
                }
            });
            window.add_action(&zoom_in);

            let zoom_out = gio::SimpleAction::new("zoom-out", None);
            let weak = window.downgrade();
            zoom_out.connect_activate(move |_, _| {
                if let Some(window) = weak.upgrade() {
                    window.zoom_by(-0.5);
                }
            });
            window.add_action(&zoom_out);

            let focus_center = gio::SimpleAction::new("focus-center", None);
            let weak = window.downgrade();
            focus_center.connect_activate(move |_, _| {
                if let Some(window) = weak.upgrade() {
                    window.focus_center();
                }
            });
            window.add_action(&focus_center);

            let shortcuts = gio::SimpleAction::new("shortcuts", None);
            let weak = window.downgrade();
            shortcuts.connect_activate(move |_, _| {
                if let Some(window) = weak.upgrade() {
                    window.show_shortcuts();
                }
            });
            window.add_action(&shortcuts);

            let manual = gio::SimpleAction::new("manual-controls", None);
            let weak = window.downgrade();
            manual.connect_activate(move |_, _| {
                if let Some(window) = weak.upgrade() {
                    let button = window.imp().manual_controls_button.get();
                    button.set_active(!button.is_active());
                }
            });
            window.add_action(&manual);

            let thumbnail = gio::SimpleAction::new("thumbnail", None);
            let settings = self.settings.clone();
            let weak = window.downgrade();
            thumbnail.connect_activate(move |_, _| {
                if let Some(window) = weak.upgrade() {
                    if settings.thumbnail_opens_external() {
                        window.open_latest_external();
                    } else {
                        window.show_gallery();
                    }
                }
            });
            window.add_action(&thumbnail);

            let settings_action = gio::SimpleAction::new("settings", None);
            let weak = window.downgrade();
            settings_action.connect_activate(move |_, _| {
                if let Some(window) = weak.upgrade() {
                    window.show_settings();
                }
            });
            window.add_action(&settings_action);

            if let Some(application) = window.application() {
                application.set_accels_for_action("win.capture", &["space"]);
                application.set_accels_for_action("win.switch-camera", &["s"]);
                application.set_accels_for_action("win.toggle-video-mode", &["v"]);
                application.set_accels_for_action("win.zoom-in", &["plus", "equal"]);
                application.set_accels_for_action("win.zoom-out", &["minus"]);
                application.set_accels_for_action("win.focus-center", &["f"]);
                application.set_accels_for_action("win.shortcuts", &["<primary>question"]);
                application.set_accels_for_action("win.settings", &["<primary>comma"]);
            }

            *self.capture_action.borrow_mut() = Some(capture);
            *self.switch_action.borrow_mut() = Some(switch);
            *self.select_action.borrow_mut() = Some(select);
            *self.video_mode_action.borrow_mut() = Some(video_mode);
        }

        fn setup_status(&self) {
            let view = self.camera_view.get();

            let label = self.status_label.get();
            let weak = self.obj().downgrade();

            view.connect_notify_local(Some("ready"), move |view, _| {
                label.set_visible(!view.ready());
                if let Some(window) = weak.upgrade() {
                    if view.ready() {
                        window.update_camera_menu();
                    }
                    window.update_mode_ui();
                    window.update_manual_controls();
                }
            });

            let label = self.status_label.get();
            view.connect_notify_local(Some("status"), move |view, _| {
                label.set_label(&view.status());
            });

            self.status_label.set_visible(!self.camera_view.ready());

            // The camera thread marshals this to the main context before it
            // reaches us, so it can touch widgets directly.
            let weak = self.obj().downgrade();
            view.set_photo_callback(move |path, ok| {
                if let Some(window) = weak.upgrade() {
                    window.on_photo_result(path, ok);
                }
            });
        }

        fn setup_gestures(&self) {
            let view = self.camera_view.get();

            // Tap-to-focus happens on release, not on press.  A pinch or swipe
            // can therefore start first, and only a stationary single-finger
            // tap focuses.
            let click = gtk::GestureClick::new();
            click.set_button(1);
            let weak = self.obj().downgrade();
            click.connect_pressed(move |_, _, x, y| {
                if let Some(window) = weak.upgrade() {
                    window.imp().tap_start.set(Some((x, y)));
                }
            });

            let weak = self.obj().downgrade();
            click.connect_released(move |_, _, x, y| {
                let Some(window) = weak.upgrade() else {
                    return;
                };
                let imp = window.imp();
                let Some((start_x, start_y)) = imp.tap_start.take() else {
                    return;
                };

                // A pinch in progress, or one that just ended, must not focus.
                if imp.zoom_gesture_start.get().is_some()
                    || imp
                        .suppress_tap_until
                        .get()
                        .is_some_and(|until| Instant::now() < until)
                {
                    return;
                }

                // Ignore drags and swipes.
                let dx = x - start_x;
                let dy = y - start_y;
                if dx * dx + dy * dy > 16.0 * 16.0 {
                    return;
                }

                let view = imp.camera_view.get();
                let width = view.width().max(1) as f64;
                let height = view.height().max(1) as f64;
                let nx = (x / width).clamp(0.0, 1.0);
                let ny = (y / height).clamp(0.0, 1.0);
                if view.focus_at(nx, ny).is_ok() {
                    window.show_focus_reticle(x, y);
                    window.start_focus_assist_timeout();
                }
            });

            let weak = self.obj().downgrade();
            click.connect_cancel(move |_, _| {
                if let Some(window) = weak.upgrade() {
                    window.imp().tap_start.set(None);
                }
            });

            view.add_controller(click);

            let zoom = gtk::GestureZoom::new();
            let weak = self.obj().downgrade();
            zoom.connect_begin(move |_, _| {
                if let Some(window) = weak.upgrade() {
                    let view = window.imp().camera_view.get();
                    let imp = window.imp();
                    imp.zoom_gesture_start.set(Some(view.zoom()));
                    imp.tap_start.set(None);
                    imp.suppress_tap_until
                        .set(Some(Instant::now() + Duration::from_millis(300)));
                }
            });

            let weak = self.obj().downgrade();
            zoom.connect_scale_changed(move |gesture, _| {
                if let Some(window) = weak.upgrade() {
                    let view = window.imp().camera_view.get();
                    let start = window.imp().zoom_gesture_start.get().unwrap_or(1.0);
                    let target = start * gesture.scale_delta() as f32;
                    view.set_zoom(target);
                    window.update_zoom_ui();
                }
            });

            let weak = self.obj().downgrade();
            zoom.connect_end(move |_, _| {
                if let Some(window) = weak.upgrade() {
                    let imp = window.imp();
                    imp.zoom_gesture_start.set(None);
                    imp.suppress_tap_until
                        .set(Some(Instant::now() + Duration::from_millis(300)));
                }
            });

            view.add_controller(zoom);
        }
    }
}

fn iso_bounds(limits: crate::camera::ManualControls) -> (f64, f64) {
    let min = limits.iso_min.max(1) as f64;
    let max = limits.iso_max.max(limits.iso_min + 1) as f64;
    (min, max)
}

fn iso_from_slider(limits: crate::camera::ManualControls, t: f64) -> i32 {
    let (min, max) = iso_bounds(limits);
    if max <= min {
        return min as i32;
    }
    (min * (max / min).powf(t.clamp(0.0, 1.0))).round() as i32
}

fn slider_from_iso(limits: crate::camera::ManualControls, iso: i32) -> f64 {
    let (min, max) = iso_bounds(limits);
    if max <= min {
        return 0.5;
    }
    ((iso.max(min as i32) as f64 / min).ln() / (max / min).ln()).clamp(0.0, 1.0)
}

fn ns_bounds(limits: crate::camera::ManualControls) -> (f64, f64) {
    let min = limits.exposure_min_ns.max(1) as f64;
    let max = limits.exposure_max_ns.max(limits.exposure_min_ns + 1) as f64;
    (min, max)
}

fn ns_from_slider(limits: crate::camera::ManualControls, t: f64) -> i64 {
    let (min, max) = ns_bounds(limits);
    if max <= min {
        return min as i64;
    }
    (min * (max / min).powf(t.clamp(0.0, 1.0))).round() as i64
}

fn slider_from_ns(limits: crate::camera::ManualControls, ns: i64) -> f64 {
    let (min, max) = ns_bounds(limits);
    if max <= min {
        return 0.5;
    }
    ((ns.max(min as i64) as f64 / min).ln() / (max / min).ln()).clamp(0.0, 1.0)
}

fn format_shutter(ns: i64) -> String {
    let ns = ns.max(1);
    if ns >= 500_000_000 {
        format!("{:.1} s", ns as f64 / 1_000_000_000.0)
    } else {
        format!("1/{:.0} s", 1_000_000_000.0 / ns as f64)
    }
}

glib::wrapper! {
    pub struct CameraWindow(ObjectSubclass<imp::CameraWindow>)
        @extends gtk::Widget, gtk::Window, gtk::ApplicationWindow, adw::ApplicationWindow,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl CameraWindow {
    pub fn new<P: IsA<gtk::Application>>(application: &P) -> Self {
        glib::Object::builder()
            .property("application", application)
            .build()
    }

    pub fn camera_view(&self) -> CameraView {
        self.imp().camera_view.get()
    }

    pub(crate) fn navigation_view(&self) -> adw::NavigationView {
        self.imp().navigation_view.get()
    }

    pub(crate) fn update_preview_aspect(&self) {
        let view = self.imp().camera_view.get();
        self.imp().preview_frame.set_ratio(view.preview_aspect());
    }

    fn show_shortcuts(&self) {
        let builder = gtk::Builder::from_resource("/io/github/nico359/camera/shortcuts-dialog.ui");
        if let Some(dialog) = builder.object::<adw::Dialog>("shortcuts_dialog") {
            dialog.present(Some(self));
        }
    }

    fn show_settings(&self) {
        crate::ui::present_settings(self);
    }

    pub(crate) fn show_gallery(&self) {
        crate::ui::present_gallery(self);
    }

    fn open_latest_external(&self) {
        let Some(path) = crate::media::latest_media() else {
            self.show_toast(&gettext("No photos or videos yet"));
            return;
        };
        let ask = self.imp().settings.open_with_dialog();
        let weak = self.downgrade();
        crate::media::launch_external(self, &path, ask, move |error| {
            if let Some(window) = weak.upgrade() {
                window.show_toast(&format!("{}: {error}", gettext("Could not open media")));
            }
        });
    }

    fn shutter(&self) {
        let view = self.imp().camera_view.get();
        if view.video_mode() {
            if view.is_recording() {
                self.stop_recording();
            } else {
                self.start_recording();
            }
        } else {
            self.capture_photo();
        }
    }

    fn toggle_video_mode(&self) {
        self.clear_focus_assist();
        let view = self.imp().camera_view.get();
        if view.is_recording() {
            self.show_toast(&gettext("Stop recording first"));
            return;
        }

        let enabled = !view.video_mode();
        match view.set_video_mode(enabled) {
            Ok(()) => {
                self.update_mode_ui();
                self.update_manual_controls();
            }
            Err(error) => {
                self.update_mode_ui();
                self.update_manual_controls();
                self.show_toast(&format!("{}: {error}", gettext("Could not switch mode")));
            }
        }
    }

    pub(crate) fn update_mode_ui(&self) {
        let imp = self.imp();
        let view = imp.camera_view.get();
        let ready = view.ready();
        let video = view.video_mode();
        let recording = view.is_recording();
        if !ready {
            self.stop_focus_assist_timeout();
        }

        if let Some(action) = imp.capture_action.borrow().as_ref() {
            action.set_enabled(ready);
        }
        if let Some(action) = imp.switch_action.borrow().as_ref() {
            action.set_enabled(ready && !recording);
        }
        if let Some(action) = imp.select_action.borrow().as_ref() {
            action.set_enabled(ready && !recording);
        }
        if let Some(action) = imp.video_mode_action.borrow().as_ref() {
            action.set_enabled(ready && !recording);
        }

        imp.camera_menu_button.set_sensitive(ready && !recording);
        imp.flash_menu_button
            .set_sensitive(ready && !view.is_front());
        self.update_zoom_ui();
        self.update_flash_ui();
        imp.preview_frame.set_ratio(view.preview_aspect());

        if video {
            imp.video_mode_image
                .set_icon_name(Some("camera-photo-symbolic"));
            imp.video_mode_button
                .set_tooltip_text(Some(&gettext("Switch to Photo")));
        } else {
            imp.video_mode_image
                .set_icon_name(Some("camera-video-symbolic"));
            imp.video_mode_button
                .set_tooltip_text(Some(&gettext("Switch to Video")));
        }

        if recording {
            imp.shutter_image
                .set_icon_name(Some("media-playback-stop-symbolic"));
            imp.shutter_button
                .set_tooltip_text(Some(&gettext("Stop Recording")));
            imp.shutter_button.remove_css_class("suggested-action");
            imp.shutter_button.add_css_class("destructive-action");
        } else if video {
            imp.shutter_image
                .set_icon_name(Some("media-record-symbolic"));
            imp.shutter_button
                .set_tooltip_text(Some(&gettext("Start Recording")));
            imp.shutter_button.remove_css_class("suggested-action");
            imp.shutter_button.add_css_class("destructive-action");
        } else {
            imp.shutter_image
                .set_icon_name(Some("camera-photo-symbolic"));
            imp.shutter_button
                .set_tooltip_text(Some(&gettext("Take Photo")));
            imp.shutter_button.remove_css_class("destructive-action");
            imp.shutter_button.add_css_class("suggested-action");
        }

        if !recording {
            self.stop_recording_tick();
            imp.recording_started.set(None);
            imp.recording_label.set_visible(false);
        }
    }

    fn set_flash_mode(&self, mode: i32) {
        let view = self.imp().camera_view.get();
        let mode = if view.is_front() {
            0
        } else {
            match mode {
                1 => 1,
                3 => 3,
                _ => 0,
            }
        };

        self.clear_focus_assist();
        view.set_flash_mode(if mode == 1 { 1 } else { 0 });
        view.set_torch(mode == 3);
        self.imp().flash_mode.set(mode);
        self.update_mode_ui();
    }

    fn update_flash_ui(&self) {
        let mode = self.imp().flash_mode.get();
        let menu = gio::Menu::new();

        for (value, label) in [
            (0, gettext("Off")),
            (1, gettext("On")),
            (3, gettext("Torch")),
        ] {
            let label = if mode == value {
                format!("✓ {label}")
            } else {
                label
            };
            let item = gio::MenuItem::new(Some(&label), None);
            item.set_action_and_target_value(Some("win.set-flash-mode"), Some(&value.to_variant()));
            menu.append_item(&item);
        }

        let button = self.imp().flash_menu_button.get();
        button.set_menu_model(Some(&menu));
        for class in ["flash-off", "flash-on", "flash-torch"] {
            button.remove_css_class(class);
        }
        button.add_css_class(match mode {
            1 => "flash-on",
            3 => "flash-torch",
            _ => "flash-off",
        });
    }

    fn zoom_by(&self, delta: f32) {
        let view = self.imp().camera_view.get();
        view.set_zoom(view.zoom() + delta);
        self.update_zoom_ui();
    }

    fn focus_center(&self) {
        let view = self.imp().camera_view.get();
        if view.focus_at(0.5, 0.5).is_ok() {
            self.show_focus_reticle(view.width() as f64 / 2.0, view.height() as f64 / 2.0);
            self.start_focus_assist_timeout();
        }
    }

    fn start_focus_assist_timeout(&self) {
        let view = self.imp().camera_view.get();
        if !view.focus_assist_lit() {
            return;
        }

        self.stop_focus_assist_timeout();
        let weak = self.downgrade();
        let started = Instant::now();
        let id = glib::timeout_add_local(Duration::from_millis(50), move || match weak.upgrade() {
            Some(window) => {
                let view = window.imp().camera_view.get();
                let state = view.af_state();
                let elapsed = started.elapsed();
                let settled = (state == 4 || state == 5) && elapsed >= Duration::from_millis(400);
                if settled || elapsed >= Duration::from_millis(1500) {
                    view.set_torch(false);
                    view.clear_focus_assist();
                    *window.imp().focus_assist_tick.borrow_mut() = None;
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            }
            None => glib::ControlFlow::Break,
        });
        *self.imp().focus_assist_tick.borrow_mut() = Some(id);
    }

    fn stop_focus_assist_timeout(&self) {
        if let Some(id) = self.imp().focus_assist_tick.borrow_mut().take() {
            id.remove();
        }
    }

    fn clear_focus_assist(&self) {
        self.stop_focus_assist_timeout();
        let view = self.imp().camera_view.get();
        if view.focus_assist_lit() {
            view.set_torch(false);
            view.clear_focus_assist();
        }
    }

    fn update_zoom_ui(&self) {
        let zoom = self.imp().camera_view.get().zoom();
        let label = self.imp().zoom_label.get();
        if zoom > 1.005 {
            label.set_text(&format!("{zoom:.1}×"));
            label.set_visible(true);
        } else {
            label.set_visible(false);
        }
    }

    fn show_focus_reticle(&self, x: f64, y: f64) {
        let view = self.imp().camera_view.get();
        let overlay = self.imp().preview_overlay.get();

        // The gesture coordinates are relative to CameraView; the reticle is
        // positioned inside the outer preview overlay.  Translate between them
        // so the reticle lands exactly under the finger.
        let point = view.compute_point(&overlay, &gtk::graphene::Point::new(x as f32, y as f32));
        let (x, y) = point
            .map(|point| (point.x() as f64, point.y() as f64))
            .unwrap_or((x, y));

        let size = 64.0;
        let max_x = (overlay.width() as f64 - size).max(0.0);
        let max_y = (overlay.height() as f64 - size).max(0.0);
        let left = (x - size / 2.0).clamp(0.0, max_x) as i32;
        let top = (y - size / 2.0).clamp(0.0, max_y) as i32;

        let reticle = self.imp().focus_reticle.get();
        reticle.set_margin_start(left);
        reticle.set_margin_top(top);
        reticle.set_visible(true);

        // iOS-style exposure control centred next to the focus rectangle and
        // shown for the same 2 s as the reticle.
        let imp = self.imp();
        let limits = *imp.manual_limits.borrow();
        if limits.ev_max > limits.ev_min && !imp.manual_active.get() {
            let ev = imp.ev_control.get();
            let ev_height = 150.0;
            let ev_left = (left + size as i32 + 8).clamp(0, (overlay.width() - 64).max(0));
            let ev_top = ((top as f64 + size / 2.0 - ev_height / 2.0)
                .clamp(0.0, (overlay.height() as f64 - ev_height).max(0.0)))
                as i32;
            ev.set_margin_start(ev_left);
            ev.set_margin_top(ev_top);
            ev.set_visible(true);
        } else {
            imp.ev_control.set_visible(false);
        }

        self.restart_focus_timeout();
    }

    fn restart_focus_timeout(&self) {
        if let Some(id) = self.imp().focus_tick.borrow_mut().take() {
            id.remove();
        }

        let weak = self.downgrade();
        let id = glib::timeout_add_local(Duration::from_secs(2), move || {
            if let Some(window) = weak.upgrade() {
                window.imp().focus_reticle.set_visible(false);
                window.imp().ev_control.set_visible(false);
                *window.imp().focus_tick.borrow_mut() = None;
            }
            glib::ControlFlow::Break
        });
        *self.imp().focus_tick.borrow_mut() = Some(id);
    }

    fn start_recording(&self) {
        self.clear_focus_assist();
        let path = match crate::media::next_video_path() {
            Ok(path) => path,
            Err(error) => {
                self.show_toast(&format!("{}: {error}", gettext("Could not prepare video")));
                return;
            }
        };

        match self.imp().camera_view.start_recording(&path) {
            Ok(()) => {
                *self.imp().recording_path.borrow_mut() = Some(path);
                self.imp().recording_started.set(Some(Instant::now()));
                self.imp().recording_label.set_label("● 00:00");
                self.imp().recording_label.set_visible(true);
                self.start_recording_tick();
                self.update_mode_ui();
            }
            Err(error) => {
                self.show_toast(&format!(
                    "{}: {error}",
                    gettext("Could not start recording")
                ));
            }
        }
    }

    fn stop_recording(&self) {
        let path = self.imp().recording_path.borrow_mut().take();
        let result = self.imp().camera_view.stop_recording();

        self.stop_recording_tick();
        self.imp().recording_started.set(None);
        self.imp().recording_label.set_visible(false);
        self.update_mode_ui();

        match result {
            Ok(false) => {
                if let Some(path) = path {
                    self.update_media_thumbnail(&path);
                }
                self.show_short_toast(&gettext("Video saved"));
            }
            Ok(true) => self.show_toast(&gettext("Video may be incomplete")),
            Err(error) => {
                self.show_toast(&format!("{}: {error}", gettext("Recording failed")));
            }
        }
    }

    fn start_recording_tick(&self) {
        if self.imp().recording_tick.borrow().is_some() {
            return;
        }

        let weak = self.downgrade();
        let id = glib::timeout_add_local(Duration::from_secs(1), move || match weak.upgrade() {
            Some(window) => {
                let Some(started) = window.imp().recording_started.get() else {
                    return glib::ControlFlow::Break;
                };
                let seconds = started.elapsed().as_secs();
                window.imp().recording_label.set_label(&format!(
                    "● {:02}:{:02}",
                    seconds / 60,
                    seconds % 60
                ));
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });

        *self.imp().recording_tick.borrow_mut() = Some(id);
    }

    fn stop_recording_tick(&self) {
        if let Some(id) = self.imp().recording_tick.borrow_mut().take() {
            id.remove();
        }
    }

    fn capture_photo(&self) {
        self.clear_focus_assist();
        let path = match crate::media::next_photo_path() {
            Ok(path) => path,
            Err(error) => {
                self.show_toast(&format!("{}: {error}", gettext("Could not prepare photo")));
                return;
            }
        };

        match self.imp().camera_view.capture_photo(&path) {
            Ok(()) => self.flash_shutter(),
            Err(error) => self.show_toast(&format!("{}: {error}", gettext("Capture failed"))),
        }
    }

    fn flip_camera(&self) {
        self.clear_focus_assist();
        let was_video = self.imp().camera_view.get().video_mode();
        match self.imp().camera_view.flip_camera() {
            Ok(info) => {
                self.imp().flash_mode.set(0);
                self.update_camera_menu();
                let view = self.imp().camera_view.get();
                self.imp().preview_frame.set_ratio(view.preview_aspect());
                if was_video && !view.video_mode() {
                    self.show_toast(&gettext(
                        "This camera does not support video; using photo mode",
                    ));
                } else {
                    self.show_short_toast(&info.label);
                }
            }
            Err(error) => {
                self.show_toast(&format!("{}: {error}", gettext("Could not switch camera")))
            }
        }
        self.update_mode_ui();
        self.update_manual_controls();
    }

    fn select_camera(&self, index: usize) {
        self.clear_focus_assist();
        let was_video = self.imp().camera_view.get().video_mode();
        match self.imp().camera_view.select_camera(index) {
            Ok(info) => {
                self.imp().flash_mode.set(0);
                self.update_camera_menu();
                let view = self.imp().camera_view.get();
                self.imp().preview_frame.set_ratio(view.preview_aspect());
                if was_video && !view.video_mode() {
                    self.show_toast(&gettext(
                        "This camera does not support video; using photo mode",
                    ));
                } else {
                    self.show_short_toast(&info.label);
                }
            }
            Err(error) => {
                self.show_toast(&format!("{}: {error}", gettext("Could not switch camera")))
            }
        }
        self.update_mode_ui();
    }

    /// Reserve space below the preview for the Pro panel only when the current
    /// preview aspect leaves vertical slack (e.g. 4:3). 16:9 is height-limited,
    /// so the panel overlays it instead of shrinking the image.
    fn update_preview_panel_margin(&self, open: bool) {
        let imp = self.imp();
        let spacer = imp.preview_panel_spacer.get();
        if !open {
            spacer.set_size_request(-1, 0);
            return;
        }

        // Portrait 4:3, 5:4, 1:1 and similar leave vertical slack, so reserve
        // the panel's height and move the preview up. 16:9 is height-limited,
        // so the panel overlays it instead of shrinking the image.
        let aspect = imp.camera_view.get().preview_aspect();
        let reserve = if aspect > 0.65 { 220 } else { 0 };
        spacer.set_size_request(-1, reserve);
    }

    fn update_manual_controls(&self) {
        let imp = self.imp();
        let view = imp.camera_view.get();
        let limits = if view.ready() {
            view.manual_controls()
        } else {
            crate::camera::ManualControls::default()
        };
        *imp.manual_limits.borrow_mut() = limits;

        if !view.ready() {
            imp.iso_scale.set_sensitive(false);
            imp.shutter_scale.set_sensitive(false);
            imp.exposure_auto_button.set_sensitive(false);
            return;
        }

        // A camera or mode change invalidates the old exposure values.
        imp.manual_syncing.set(true);
        imp.exposure_auto_button.set_active(true);
        imp.manual_active.set(false);
        imp.iso_scale.set_value(slider_from_iso(limits, 400));
        imp.shutter_scale
            .set_value(slider_from_ns(limits, 16_666_667));
        imp.ae_lock_button.set_active(false);
        imp.awb_lock_button.set_active(false);
        view.set_auto_exposure();
        view.set_ae_lock(false);
        view.set_awb_lock(false);

        let ev = imp.ev_touch_scale.adjustment();
        ev.set_lower(limits.ev_min as f64);
        ev.set_upper(limits.ev_max as f64);
        ev.set_value(0.0);
        imp.ev_control.set_visible(false);
        imp.manual_syncing.set(false);

        self.update_manual_labels();
        self.update_manual_sensitivity();
        self.update_preview_panel_margin(imp.manual_revealer.reveals_child());
    }

    fn update_manual_sensitivity(&self) {
        let imp = self.imp();
        let limits = *imp.manual_limits.borrow();
        imp.iso_scale.set_sensitive(limits.supported);
        imp.shutter_scale.set_sensitive(limits.supported);
        imp.exposure_auto_button.set_sensitive(limits.supported);
    }

    fn update_manual_labels(&self) {
        let imp = self.imp();
        let limits = *imp.manual_limits.borrow();

        let iso = iso_from_slider(limits, imp.iso_scale.value());
        imp.iso_value.set_label(&format!("ISO {iso}"));

        let ns = ns_from_slider(limits, imp.shutter_scale.value());
        imp.shutter_value.set_label(&format_shutter(ns));
    }

    fn apply_manual_exposure(&self) {
        let imp = self.imp();
        let limits = *imp.manual_limits.borrow();
        let view = imp.camera_view.get();
        if !view.ready() || !limits.supported {
            return;
        }

        let iso = iso_from_slider(limits, imp.iso_scale.value());
        let ns = ns_from_slider(limits, imp.shutter_scale.value());
        view.set_manual_exposure(iso, ns);
        imp.manual_active.set(true);
        imp.exposure_auto_button.set_active(false);
        imp.ev_control.set_visible(false);
        self.update_manual_labels();
        self.update_manual_sensitivity();
    }

    fn apply_ev_touch(&self) {
        let imp = self.imp();
        let limits = *imp.manual_limits.borrow();
        if limits.ev_max <= limits.ev_min || imp.manual_active.get() {
            return;
        }

        // The HAL's EV index runs opposite to the slider: dragging up should
        // brighten, so negate the index while keeping the slider's direction.
        let steps =
            (-(imp.ev_touch_scale.value().round() as i32)).clamp(limits.ev_min, limits.ev_max);
        imp.camera_view.get().set_exposure_compensation(steps);
        self.restart_focus_timeout();
    }

    fn update_camera_menu(&self) {
        let cameras = self.imp().camera_view.cameras();
        let current = self.imp().camera_view.current_camera_index();
        let menu = gio::Menu::new();

        for (index, camera) in cameras.iter().enumerate() {
            let label = if current == Some(index) {
                format!("✓ {}", camera.label)
            } else {
                camera.label.clone()
            };
            let item = gio::MenuItem::new(Some(&label), None);
            item.set_action_and_target_value(
                Some("win.select-camera"),
                Some(&(index as i32).to_variant()),
            );
            menu.append_item(&item);
        }

        self.imp().camera_menu_button.set_menu_model(Some(&menu));
    }

    fn on_photo_result(&self, path: &str, ok: bool) {
        if ok {
            // Tag first; the thumbnail is refreshed after the EXIF write so it
            // never reads a half-rewritten JPEG.
            self.tag_photo_location(Path::new(path));
        } else {
            self.show_toast(&gettext("Could not save photo"));
        }
    }

    fn update_media_thumbnail(&self, path: &Path) {
        let is_video = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"));

        if let Some(media) = self.imp().last_video_media.borrow_mut().take() {
            media.pause();
            self.imp()
                .last_video_picture
                .set_paintable(None::<&gtk::gdk::Paintable>);
        }

        if is_video {
            if let Some(texture) = crate::ui::gallery::cached_video_poster(path) {
                self.imp().last_video_picture.set_paintable(Some(&texture));
                self.imp()
                    .last_media_stack
                    .set_visible_child(&self.imp().last_video_picture.get());
            } else {
                // Show the film icon until the first frame has been decoded.
                self.imp()
                    .last_media_stack
                    .set_visible_child(&self.imp().last_video_icon.get());
                self.start_video_thumbnail(path);
            }
            self.imp().last_photo_button.set_visible(true);
            return;
        }

        match crate::ui::square_texture(path, 48) {
            Some(texture) => {
                self.imp().last_photo_picture.set_paintable(Some(&texture));
                self.imp()
                    .last_media_stack
                    .set_visible_child(&self.imp().last_photo_picture.get());
                self.imp().last_photo_button.set_visible(true);
            }
            None => eprintln!("thumbnail for {} could not be loaded", path.display()),
        }
    }

    /// Decode the first frame of a video that has no cached poster yet, then
    /// cache it for the gallery and show a small cropped thumbnail.
    fn start_video_thumbnail(&self, path: &Path) {
        let Some(surface) = self.native().and_then(|native| native.surface()) else {
            return;
        };

        let media = gtk::MediaFile::for_filename(path);
        media.set_loop(false);

        let path = path.to_path_buf();
        let weak = self.downgrade();
        media.connect_invalidate_contents(move |stream| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let imp = window.imp();
            // Taking the media also makes this run only for the current video.
            let Some(media) = imp.last_video_media.borrow_mut().take() else {
                return;
            };
            let Some(native) = window.native() else {
                return;
            };
            let Some(renderer) = native.renderer() else {
                return;
            };

            if let Some(texture) = snapshot_media_square(stream, 48.0, &renderer) {
                imp.last_video_picture.set_paintable(Some(&texture));
                imp.last_media_stack
                    .set_visible_child(&imp.last_video_picture.get());
            }
            if let Some(poster) = snapshot_media(stream, 1280.0, &renderer) {
                crate::ui::gallery::store_video_poster(&path, &poster);
            }

            media.pause();
        });

        // Keep the stream alive until the first frame arrives.
        *self.imp().last_video_media.borrow_mut() = Some(media.clone());
        media.realize(&surface);
        media.pause();
    }

    fn load_latest_media(&self) {
        if let Some(path) = crate::media::latest_media() {
            self.update_media_thumbnail(&path);
        } else {
            if let Some(media) = self.imp().last_video_media.borrow_mut().take() {
                media.pause();
            }
            self.imp()
                .last_photo_picture
                .set_paintable(None::<&gtk::gdk::Paintable>);
            self.imp()
                .last_video_picture
                .set_paintable(None::<&gtk::gdk::Paintable>);
            self.imp()
                .last_media_stack
                .set_visible_child(&self.imp().last_video_icon.get());
            self.imp().last_photo_button.set_visible(false);
        }
    }

    pub(crate) fn refresh_latest_media(&self) {
        self.load_latest_media();
    }

    /// Brief blackout over the preview, the way most camera apps acknowledge a
    /// shutter press. Replaces the "Saving photo" / "Photo saved" toasts.
    fn flash_shutter(&self) {
        let imp = self.imp();
        if let Some(id) = imp.shutter_flash_tick.borrow_mut().take() {
            id.remove();
        }

        let flash = imp.shutter_flash.get();
        flash.set_opacity(0.6);

        let started = Instant::now();
        let duration = Duration::from_millis(180);
        let weak = self.downgrade();
        let id = glib::timeout_add_local(Duration::from_millis(16), move || {
            let elapsed = started.elapsed().as_secs_f64();
            let progress = elapsed / duration.as_secs_f64();
            if progress >= 1.0 {
                flash.set_opacity(0.0);
                // The source removes itself after returning Break; forget its
                // id so a later shutter press never removes a stale (possibly
                // reused) source id.
                if let Some(window) = weak.upgrade() {
                    window.imp().shutter_flash_tick.borrow_mut().take();
                }
                glib::ControlFlow::Break
            } else {
                flash.set_opacity(0.6 * (1.0 - progress));
                glib::ControlFlow::Continue
            }
        });
        *imp.shutter_flash_tick.borrow_mut() = Some(id);
    }

    pub(crate) fn start_location(&self) {
        let imp = self.imp();
        if imp.location_stop.borrow().is_some() {
            self.update_gps_indicator();
            return;
        }

        let (sender, receiver) = std::sync::mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        crate::location::spawn(sender, Arc::clone(&stop));
        *imp.location_rx.borrow_mut() = Some(receiver);
        *imp.location_stop.borrow_mut() = Some(stop);
        *imp.location_fix.borrow_mut() = None;
        self.update_gps_indicator();
    }

    pub(crate) fn stop_location(&self) {
        let imp = self.imp();
        if let Some(stop) = imp.location_stop.borrow_mut().take() {
            stop.store(true, Ordering::Relaxed);
        }
        *imp.location_rx.borrow_mut() = None;
        *imp.location_fix.borrow_mut() = None;
        self.update_gps_indicator();
    }

    fn update_gps_indicator(&self) {
        let imp = self.imp();
        let indicator = imp.gps_indicator.get();

        if !imp.settings.geotag_photos() {
            indicator.set_visible(false);
            return;
        }

        indicator.set_visible(true);
        let fix = imp.location_fix.borrow();
        let fresh = fix
            .as_ref()
            .filter(|fix| fix.received.elapsed() <= Duration::from_secs(5));
        let usable =
            fresh.is_some_and(|fix| !imp.settings.geotag_high_accuracy() || fix.accuracy <= 10.0);

        let text = match fresh {
            Some(fix) if usable => format!("GPS: ±{:.0} m", fix.accuracy),
            Some(fix) => format!("GPS: ±{:.0} m (not used)", fix.accuracy),
            None => gettext("GPS: no fix"),
        };
        indicator.set_tooltip_text(Some(&text));
        if usable {
            indicator.remove_css_class("dim-label");
        } else {
            indicator.add_css_class("dim-label");
        }
    }

    /// Add GPS EXIF to a saved JPEG if the current fix is fresh and accurate
    /// enough. Invalid or stale positions are silently skipped so a photo is
    /// never tagged with a wrong location.
    fn tag_photo_location(&self, path: &Path) {
        let imp = self.imp();
        let fix = if imp.settings.geotag_photos() {
            imp.location_fix.borrow().clone().filter(|fix| {
                fix.received.elapsed() <= Duration::from_secs(5)
                    && (!imp.settings.geotag_high_accuracy() || fix.accuracy <= 10.0)
            })
        } else {
            None
        };

        let Some(fix) = fix else {
            self.update_media_thumbnail(path);
            return;
        };

        let path = path.to_path_buf();
        let weak = glib::SendWeakRef::from(self.downgrade());
        std::thread::spawn(move || {
            if let Err(error) = crate::location::write_gps(&path, &fix) {
                eprintln!("GPS tag for {}: {error}", path.display());
            }
            glib::idle_add_once(move || {
                if let Some(window) = weak.upgrade() {
                    window.update_media_thumbnail(&path);
                }
            });
        });
    }

    pub(crate) fn show_toast(&self, message: &str) {
        self.imp().toast_overlay.add_toast(adw::Toast::new(message));
    }

    fn show_short_toast(&self, message: &str) {
        let toast = adw::Toast::new(message);
        toast.set_timeout(2);
        self.imp().toast_overlay.add_toast(toast);
    }
}
