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

/// Bubble level. `angle` is the roll error in degrees relative to the nearest
/// 90° orientation; the bubble is green when within 1.5°.
fn draw_level(cr: &gtk::cairo::Context, width: f64, height: f64, angle: f64) {
    let center_x = width / 2.0;
    let center_y = height / 2.0;
    let half = (width * 0.32).min(150.0);
    let offset = (angle * (half / 30.0)).clamp(-half, half);
    let level = angle.abs() < 1.5;

    cr.set_source_rgba(0.0, 0.0, 0.0, 0.35);
    cr.set_line_width(2.0);
    cr.move_to(center_x - half, center_y);
    cr.line_to(center_x + half, center_y);
    let _ = cr.stroke();

    cr.set_source_rgba(1.0, 1.0, 1.0, 0.55);
    cr.set_line_width(1.0);
    cr.move_to(center_x - half, center_y);
    cr.line_to(center_x + half, center_y);
    let _ = cr.stroke();

    cr.move_to(center_x, center_y - 6.0);
    cr.line_to(center_x, center_y + 6.0);
    let _ = cr.stroke();

    let bubble_x = center_x + offset;
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.5);
    cr.arc(bubble_x, center_y, 9.0, 0.0, std::f64::consts::TAU);
    let _ = cr.fill();

    if level {
        cr.set_source_rgba(0.2, 0.82, 0.4, 0.95);
    } else {
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.9);
    }
    cr.arc(bubble_x, center_y, 7.0, 0.0, std::f64::consts::TAU);
    let _ = cr.fill();

    cr.set_source_rgba(0.0, 0.0, 0.0, 0.6);
    cr.set_line_width(1.0);
    cr.arc(bubble_x, center_y, 7.0, 0.0, std::f64::consts::TAU);
    let _ = cr.stroke();
}

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/nico359/camera/window.ui")]
    pub struct CameraWindow {
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
        pub status_label: TemplateChild<gtk::Label>,

        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,

        pub(super) capture_action: RefCell<Option<gio::SimpleAction>>,
        pub(super) switch_action: RefCell<Option<gio::SimpleAction>>,
        pub(super) select_action: RefCell<Option<gio::SimpleAction>>,
        pub(super) video_mode_action: RefCell<Option<gio::SimpleAction>>,
        pub(super) flash_mode: Cell<i32>,
        pub(super) recording_started: Cell<Option<Instant>>,
        pub(super) recording_tick: RefCell<Option<glib::SourceId>>,
        pub(super) recording_path: RefCell<Option<PathBuf>>,
        pub(super) zoom_gesture_start: Cell<Option<f32>>,
        pub(super) tap_start: Cell<Option<(f64, f64)>>,
        pub(super) suppress_tap_until: Cell<Option<Instant>>,
        pub(super) focus_tick: RefCell<Option<glib::SourceId>>,
        pub(super) focus_assist_tick: RefCell<Option<glib::SourceId>>,
        pub(super) settings: AppSettings,
        pub(super) level_sensor: RefCell<Option<Rc<LevelSensor>>>,
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
            let sensor = Rc::new(LevelSensor::new(move |error| {
                let smoothed = angle_cb.get() * 0.75 + error * 0.25;
                angle_cb.set(smoothed);
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

            let gallery = gio::SimpleAction::new("gallery", None);
            let weak = window.downgrade();
            gallery.connect_activate(move |_, _| {
                if let Some(window) = weak.upgrade() {
                    window.show_gallery();
                }
            });
            window.add_action(&gallery);

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

    pub(crate) fn update_preview_aspect(&self) {
        let view = self.imp().camera_view.get();
        self.imp().preview_frame.set_ratio(view.preview_aspect());
    }

    fn show_settings(&self) {
        crate::ui::present_settings(self);
    }

    pub(crate) fn show_gallery(&self) {
        crate::ui::present_gallery(self);
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
            }
            Err(error) => {
                self.update_mode_ui();
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

        if let Some(id) = self.imp().focus_tick.borrow_mut().take() {
            id.remove();
        }

        let weak = self.downgrade();
        let id = glib::timeout_add_local(Duration::from_millis(1200), move || {
            if let Some(window) = weak.upgrade() {
                window.imp().focus_reticle.set_visible(false);
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
            Ok(()) => {
                // Keep this short so the result toast is not stuck behind it.
                let toast = adw::Toast::new(&gettext("Saving photo…"));
                toast.set_timeout(2);
                self.imp().toast_overlay.add_toast(toast);
            }
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
            self.update_media_thumbnail(Path::new(path));
            self.show_short_toast(&gettext("Photo saved"));
        } else {
            self.show_toast(&gettext("Could not save photo"));
        }
    }

    fn update_media_thumbnail(&self, path: &Path) {
        let is_video = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"));

        if is_video {
            self.imp()
                .last_media_stack
                .set_visible_child(&self.imp().last_video_icon.get());
            self.imp().last_photo_button.set_visible(true);
            return;
        }

        match gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(path, 48, 48, true) {
            Ok(pixbuf) => {
                let texture = gtk::gdk::Texture::for_pixbuf(&pixbuf);
                self.imp().last_photo_picture.set_paintable(Some(&texture));
                self.imp()
                    .last_media_stack
                    .set_visible_child(&self.imp().last_photo_picture.get());
                self.imp().last_photo_button.set_visible(true);
            }
            Err(error) => eprintln!("thumbnail for {}: {error}", path.display()),
        }
    }

    fn load_latest_media(&self) {
        if let Some(path) = crate::media::latest_media() {
            self.update_media_thumbnail(&path);
        }
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
