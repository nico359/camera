/* window.rs
 *
 * Copyright 2026 nico359
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
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
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

use std::cell::RefCell;

use gettextrs::gettext;
use gtk::prelude::*;
use adw::subclass::prelude::*;
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
        pub status_label: TemplateChild<gtk::Label>,

        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,

        capture_action: RefCell<Option<gio::SimpleAction>>,
        switch_action: RefCell<Option<gio::SimpleAction>>,
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
                    window.capture_photo();
                }
            });
            window.add_action(&capture);

            let switch = gio::SimpleAction::new("switch-camera", None);
            switch.set_enabled(false);
            let weak = window.downgrade();
            switch.connect_activate(move |_, _| {
                if let Some(window) = weak.upgrade() {
                    window.switch_camera();
                }
            });
            window.add_action(&switch);

            if let Some(application) = window.application() {
                application.set_accels_for_action("win.capture", &["space"]);
                application.set_accels_for_action("win.switch-camera", &["s"]);
            }

            *self.capture_action.borrow_mut() = Some(capture);
            *self.switch_action.borrow_mut() = Some(switch);
        }

        fn setup_status(&self) {
            let view = self.camera_view.get();

            let label = self.status_label.get();
            let capture = self.capture_action.borrow().clone();
            let switch = self.switch_action.borrow().clone();

            view.connect_notify_local(Some("ready"), move |view, _| {
                let ready = view.ready();

                if let Some(action) = capture.as_ref() {
                    action.set_enabled(ready);
                }
                if let Some(action) = switch.as_ref() {
                    action.set_enabled(ready);
                }

                label.set_visible(!ready);
            });

            let label = self.status_label.get();
            view.connect_notify_local(Some("status"), move |view, _| {
                label.set_label(&view.status());
            });

            self.status_label
                .set_visible(!self.camera_view.ready());
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

    fn capture_photo(&self) {
        let path = match crate::photo::next_photo_path() {
            Ok(path) => path,
            Err(error) => {
                self.show_toast(&format!("{}: {error}", gettext("Could not prepare photo")));
                return;
            }
        };

        match self.imp().camera_view.capture_photo(&path) {
            Ok(()) => self.show_toast(&format!(
                "{} {}",
                gettext("Capturing photo to"),
                path.display()
            )),
            Err(error) => self.show_toast(&format!("{}: {error}", gettext("Capture failed"))),
        }
    }

    fn switch_camera(&self) {
        if let Err(error) = self.imp().camera_view.switch_camera() {
            self.show_toast(&format!(
                "{}: {error}",
                gettext("Could not switch camera")
            ));
        }
    }

    fn show_toast(&self, message: &str) {
        self.imp().toast_overlay.add_toast(adw::Toast::new(message));
    }
}
