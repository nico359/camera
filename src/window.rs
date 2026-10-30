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
use std::path::{Path, PathBuf};

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
        pub controls: TemplateChild<gtk::CenterBox>,

        #[template_child]
        pub last_photo_button: TemplateChild<gtk::Button>,

        #[template_child]
        pub last_photo_picture: TemplateChild<gtk::Picture>,

        #[template_child]
        pub status_label: TemplateChild<gtk::Label>,

        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,

        pub(super) last_photo_path: RefCell<Option<PathBuf>>,
        capture_action: RefCell<Option<gio::SimpleAction>>,
        switch_action: RefCell<Option<gio::SimpleAction>>,
        pub(super) last_photo_action: RefCell<Option<gio::SimpleAction>>,
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
            self.setup_layout();
            self.obj().update_layout();
            self.obj().load_latest_photo();
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

            let last_photo = gio::SimpleAction::new("open-last-photo", None);
            last_photo.set_enabled(false);
            let weak = window.downgrade();
            last_photo.connect_activate(move |_, _| {
                if let Some(window) = weak.upgrade() {
                    window.show_last_photo();
                }
            });
            window.add_action(&last_photo);

            if let Some(application) = window.application() {
                application.set_accels_for_action("win.capture", &["space"]);
                application.set_accels_for_action("win.switch-camera", &["s"]);
            }

            *self.capture_action.borrow_mut() = Some(capture);
            *self.switch_action.borrow_mut() = Some(switch);
            *self.last_photo_action.borrow_mut() = Some(last_photo);
        }

        fn setup_status(&self) {
            let view = self.camera_view.get();

            let label = self.status_label.get();
            let frame = self.preview_frame.get();
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

                if ready {
                    frame.set_ratio(view.preview_aspect());
                }
                label.set_visible(!ready);
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

        fn setup_layout(&self) {
            let window = self.obj();
            window.connect_notify_local(Some("width"), |window, _| window.update_layout());
            window.connect_notify_local(Some("height"), |window, _| window.update_layout());
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
            Ok(()) => {
                // Keep this short so the result toast is not stuck behind it.
                let toast = adw::Toast::new(&gettext("Saving photo…"));
                toast.set_timeout(2);
                self.imp().toast_overlay.add_toast(toast);
            }
            Err(error) => self.show_toast(&format!("{}: {error}", gettext("Capture failed"))),
        }
    }

    fn switch_camera(&self) {
        match self.imp().camera_view.switch_camera() {
            Ok(info) => {
                let view = self.imp().camera_view.get();
                self.imp().preview_frame.set_ratio(view.preview_aspect());
                self.show_toast(&info.label);
            }
            Err(error) => {
                self.show_toast(&format!("{}: {error}", gettext("Could not switch camera")))
            }
        }
    }

    fn on_photo_result(&self, path: &str, ok: bool) {
        if ok {
            self.update_thumbnail(Path::new(path));
            self.show_toast(&gettext("Photo saved"));
        } else {
            self.show_toast(&gettext("Could not save photo"));
        }
    }

    fn update_thumbnail(&self, path: &Path) {
        match gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(path, 48, 48, true) {
            Ok(pixbuf) => {
                let texture = gtk::gdk::Texture::for_pixbuf(&pixbuf);
                self.imp().last_photo_picture.set_paintable(Some(&texture));
                self.imp().last_photo_button.set_visible(true);
                *self.imp().last_photo_path.borrow_mut() = Some(path.to_path_buf());
                if let Some(action) = self.imp().last_photo_action.borrow().as_ref() {
                    action.set_enabled(true);
                }
            }
            Err(error) => eprintln!("thumbnail for {}: {error}", path.display()),
        }
    }

    fn load_latest_photo(&self) {
        let Ok(directory) = crate::photo::photo_directory() else {
            return;
        };
        let Ok(entries) = std::fs::read_dir(&directory) else {
            return;
        };

        let latest = entries
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("jpg"))
            })
            .max_by_key(|entry| {
                entry
                    .metadata()
                    .and_then(|metadata| metadata.modified())
                    .ok()
            });

        if let Some(entry) = latest {
            self.update_thumbnail(&entry.path());
        }
    }

    fn show_last_photo(&self) {
        let Some(path) = self.imp().last_photo_path.borrow().clone() else {
            return;
        };

        let title = gettext("Photo");
        let dialog = adw::Dialog::builder()
            .title(title.clone())
            .content_width(360)
            .content_height(640)
            .build();
        let toolbar = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&gtk::Label::new(Some(title.as_str()))));

        let open = gtk::Button::from_icon_name("external-link-symbolic");
        open.set_tooltip_text(Some(&gettext("Open in Image Viewer")));
        let file = gio::File::for_path(&path);
        let parent = self.clone();
        open.connect_clicked(move |_| {
            let launcher = gtk::FileLauncher::new(Some(&file));
            launcher.launch(Some(&parent), None::<&gio::Cancellable>, |result| {
                if let Err(error) = result {
                    eprintln!("open photo: {error}");
                }
            });
        });
        header.pack_end(&open);

        toolbar.add_top_bar(&header);
        let picture = gtk::Picture::for_filename(&path);
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_hexpand(true);
        picture.set_vexpand(true);
        toolbar.set_content(Some(&picture));

        dialog.set_child(Some(&toolbar));
        dialog.present(Some(self));
    }

    fn update_layout(&self) {
        let landscape = self.width() > self.height();
        let controls = self.imp().controls.get();

        if landscape {
            controls.set_halign(gtk::Align::End);
            controls.set_valign(gtk::Align::Center);
            controls.set_margin_start(0);
            controls.set_margin_end(28);
            controls.set_margin_top(0);
            controls.set_margin_bottom(0);
        } else {
            controls.set_halign(gtk::Align::Fill);
            controls.set_valign(gtk::Align::End);
            controls.set_margin_start(24);
            controls.set_margin_end(24);
            controls.set_margin_top(0);
            controls.set_margin_bottom(28);
        }
    }

    fn show_toast(&self, message: &str) {
        self.imp().toast_overlay.add_toast(adw::Toast::new(message));
    }
}
