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
use std::time::{Duration, Instant};

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};

use crate::ui::CameraView;

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
        pub camera_menu_button: TemplateChild<gtk::MenuButton>,

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
        pub status_label: TemplateChild<gtk::Label>,

        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,

        pub(super) last_media_path: RefCell<Option<PathBuf>>,
        pub(super) capture_action: RefCell<Option<gio::SimpleAction>>,
        pub(super) switch_action: RefCell<Option<gio::SimpleAction>>,
        pub(super) select_action: RefCell<Option<gio::SimpleAction>>,
        pub(super) video_mode_action: RefCell<Option<gio::SimpleAction>>,
        pub(super) last_media_action: RefCell<Option<gio::SimpleAction>>,
        pub(super) recording_started: Cell<Option<Instant>>,
        pub(super) recording_tick: RefCell<Option<glib::SourceId>>,
        pub(super) recording_path: RefCell<Option<PathBuf>>,
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
            self.setup_actions();
            self.setup_status();
            self.obj().load_latest_media();
        }
    }

    impl WidgetImpl for CameraWindow {}
    impl WindowImpl for CameraWindow {}
    impl ApplicationWindowImpl for CameraWindow {}
    impl AdwApplicationWindowImpl for CameraWindow {}

    impl CameraWindow {
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

            let last_media = gio::SimpleAction::new("open-last-media", None);
            last_media.set_enabled(false);
            let weak = window.downgrade();
            last_media.connect_activate(move |_, _| {
                if let Some(window) = weak.upgrade() {
                    window.open_last_media();
                }
            });
            window.add_action(&last_media);

            let settings = gio::Settings::new("io.github.nico359.camera");
            let ask_open_with = gio::SimpleAction::new_stateful(
                "ask-open-with",
                None,
                &settings.boolean("open-with-dialog").to_variant(),
            );
            let settings = settings.clone();
            let weak = window.downgrade();
            ask_open_with.connect_change_state(move |action, value| {
                let Some(value) = value.and_then(|value| value.get::<bool>()) else {
                    return;
                };
                let _ = settings.set_boolean("open-with-dialog", value);
                action.set_state(&value.to_variant());
                if let Some(window) = weak.upgrade() {
                    let message = if value {
                        gettext("Media will open with an app chooser")
                    } else {
                        gettext("Media will open in the default viewer")
                    };
                    window.show_toast(&message);
                }
            });
            window.add_action(&ask_open_with);

            if let Some(application) = window.application() {
                application.set_accels_for_action("win.capture", &["space"]);
                application.set_accels_for_action("win.switch-camera", &["s"]);
                application.set_accels_for_action("win.toggle-video-mode", &["v"]);
            }

            *self.capture_action.borrow_mut() = Some(capture);
            *self.switch_action.borrow_mut() = Some(switch);
            *self.select_action.borrow_mut() = Some(select);
            *self.video_mode_action.borrow_mut() = Some(video_mode);
            *self.last_media_action.borrow_mut() = Some(last_media);
        }

        fn setup_status(&self) {
            let view = self.camera_view.get();

            let label = self.status_label.get();
            let weak = self.obj().downgrade();

            view.connect_notify_local(Some("ready"), move |view, _| {
                label.set_visible(!view.ready());
                if let Some(window) = weak.upgrade() {
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

    fn update_mode_ui(&self) {
        let imp = self.imp();
        let view = imp.camera_view.get();
        let ready = view.ready();
        let video = view.video_mode();
        let recording = view.is_recording();

        if let Some(action) = imp.capture_action.borrow().as_ref() {
            action.set_enabled(ready);
        }
        if let Some(action) = imp.switch_action.borrow().as_ref() {
            action.set_enabled(ready && !video && !recording);
        }
        if let Some(action) = imp.select_action.borrow().as_ref() {
            action.set_enabled(ready && !video && !recording);
        }
        if let Some(action) = imp.video_mode_action.borrow().as_ref() {
            action.set_enabled(ready && !recording);
        }

        imp.camera_menu_button
            .set_sensitive(ready && !video && !recording);
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

    fn start_recording(&self) {
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
        match self.imp().camera_view.flip_camera() {
            Ok(info) => {
                self.update_camera_menu();
                let view = self.imp().camera_view.get();
                self.imp().preview_frame.set_ratio(view.preview_aspect());
                self.show_short_toast(&info.label);
            }
            Err(error) => {
                self.show_toast(&format!("{}: {error}", gettext("Could not switch camera")))
            }
        }
    }

    fn select_camera(&self, index: usize) {
        match self.imp().camera_view.select_camera(index) {
            Ok(info) => {
                self.update_camera_menu();
                let view = self.imp().camera_view.get();
                self.imp().preview_frame.set_ratio(view.preview_aspect());
                self.show_short_toast(&info.label);
            }
            Err(error) => {
                self.show_toast(&format!("{}: {error}", gettext("Could not switch camera")))
            }
        }
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
            *self.imp().last_media_path.borrow_mut() = Some(path.to_path_buf());
            if let Some(action) = self.imp().last_media_action.borrow().as_ref() {
                action.set_enabled(true);
            }
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
                *self.imp().last_media_path.borrow_mut() = Some(path.to_path_buf());
                if let Some(action) = self.imp().last_media_action.borrow().as_ref() {
                    action.set_enabled(true);
                }
            }
            Err(error) => eprintln!("thumbnail for {}: {error}", path.display()),
        }
    }

    fn load_latest_media(&self) {
        if let Some(path) = crate::media::latest_media() {
            self.update_media_thumbnail(&path);
        }
    }

    fn open_last_media(&self) {
        let Some(path) = self.imp().last_media_path.borrow().clone() else {
            return;
        };
        if !path.exists() {
            self.show_toast(&gettext("Media not found"));
            return;
        }

        let settings = gio::Settings::new("io.github.nico359.camera");
        let ask = settings.boolean("open-with-dialog");

        // The OpenURI portal deliberately shows an app chooser for local
        // files, regardless of its `ask` option.  When the user did not ask
        // for the chooser, launch the host's default handler instead.  This
        // needs --talk-name=org.freedesktop.Flatpak in the manifest.
        if !ask {
            if let Err(error) = crate::media::open_with_host(&path) {
                eprintln!("host open failed, falling back to the portal: {error}");
            } else {
                return;
            }
        }

        let file = gio::File::for_path(&path);
        let launcher = gtk::FileLauncher::new(Some(&file));
        launcher.set_always_ask(ask);

        let weak = self.downgrade();
        launcher.launch(Some(self), None::<&gio::Cancellable>, move |result| {
            if let Err(error) = result {
                if let Some(window) = weak.upgrade() {
                    window.show_toast(&format!("{}: {error}", gettext("Could not open media")));
                }
            }
        });
    }

    fn show_toast(&self, message: &str) {
        self.imp().toast_overlay.add_toast(adw::Toast::new(message));
    }

    fn show_short_toast(&self, message: &str) {
        let toast = adw::Toast::new(message);
        toast.set_timeout(2);
        self.imp().toast_overlay.add_toast(toast);
    }
}
