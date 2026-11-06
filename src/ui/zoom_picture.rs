// SPDX-License-Identifier: GPL-2.0-or-later
//
// Copyright 2026 nico359

//! Zoomable photo surface for the gallery: pinch to zoom around the fingers,
//! drag to pan while zoomed, and double-tap to toggle between fit and 2.5×.
//! Paging is disabled while zoomed so drags pan instead of changing pages.

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene};

const MAX_SCALE: f64 = 5.0;
const DOUBLE_TAP_SCALE: f64 = 2.5;
const ZOOMED_EPSILON: f64 = 1.01;
const ANIMATION: Duration = Duration::from_millis(180);

mod imp {
    use super::*;

    pub struct ZoomPicture {
        pub texture: RefCell<Option<gdk::Texture>>,
        pub scale: Cell<f64>,
        pub pan_x: Cell<f64>,
        pub pan_y: Cell<f64>,
        pub pinch_start: Cell<Option<f64>>,
        pub drag_start: Cell<(f64, f64)>,
        pub anim: RefCell<Option<glib::SourceId>>,
    }

    impl Default for ZoomPicture {
        fn default() -> Self {
            Self {
                texture: RefCell::new(None),
                scale: Cell::new(1.0),
                pan_x: Cell::new(0.0),
                pan_y: Cell::new(0.0),
                pinch_start: Cell::new(None),
                drag_start: Cell::new((0.0, 0.0)),
                anim: RefCell::new(None),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ZoomPicture {
        const NAME: &'static str = "CameraZoomPicture";
        type Type = super::ZoomPicture;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
        }
    }

    impl ObjectImpl for ZoomPicture {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.set_hexpand(true);
            obj.set_vexpand(true);
            obj.set_overflow(gtk::Overflow::Hidden);
            obj.set_focusable(false);

            let zoom = gtk::GestureZoom::new();
            let weak = obj.downgrade();
            zoom.connect_begin(move |gesture, _| {
                let Some(widget) = weak.upgrade() else {
                    return;
                };
                if widget.imp().texture.borrow().is_none() {
                    gesture.set_state(gtk::EventSequenceState::Denied);
                    return;
                }
                widget.cancel_animation();
                widget.imp().pinch_start.set(Some(widget.scale()));
            });

            let weak = obj.downgrade();
            zoom.connect_scale_changed(move |gesture, _| {
                let Some(widget) = weak.upgrade() else {
                    return;
                };
                let Some(start_scale) = widget.imp().pinch_start.get() else {
                    return;
                };
                let focus = gesture
                    .bounding_box_center()
                    .unwrap_or_else(|| widget.content_center());
                let target = (start_scale * gesture.scale_delta()).clamp(1.0, MAX_SCALE);
                widget.zoom_to(target, Some(focus), false);
            });

            let weak = obj.downgrade();
            zoom.connect_end(move |_, _| {
                let Some(widget) = weak.upgrade() else {
                    return;
                };
                widget.imp().pinch_start.set(None);
                if widget.scale() < 1.03 {
                    widget.animate_to(1.0, (0.0, 0.0));
                }
            });
            obj.add_controller(zoom);

            let drag = gtk::GestureDrag::new();
            let weak = obj.downgrade();
            drag.connect_drag_begin(move |gesture, _, _| {
                let Some(widget) = weak.upgrade() else {
                    return;
                };
                if widget.scale() <= ZOOMED_EPSILON {
                    gesture.set_state(gtk::EventSequenceState::Denied);
                    return;
                }
                widget.cancel_animation();
                widget.imp().drag_start.set(widget.pan());
            });

            let weak = obj.downgrade();
            drag.connect_drag_update(move |_, dx, dy| {
                let Some(widget) = weak.upgrade() else {
                    return;
                };
                let (sx, sy) = widget.imp().drag_start.get();
                widget.set_zoom(widget.scale(), (sx + dx, sy + dy));
            });
            obj.add_controller(drag);

            let click = gtk::GestureClick::new();
            click.set_button(gdk::BUTTON_PRIMARY);
            let weak = obj.downgrade();
            click.connect_pressed(move |_, n_press, x, y| {
                if n_press != 2 {
                    return;
                }
                let Some(widget) = weak.upgrade() else {
                    return;
                };
                if widget.scale() > ZOOMED_EPSILON {
                    widget.animate_to(1.0, (0.0, 0.0));
                } else {
                    let (cx, cy) = widget.content_center();
                    let pan = widget.pan();
                    let scale = widget.scale();
                    let base = (cx + (x - cx - pan.0) / scale, cy + (y - cy - pan.1) / scale);
                    let target = DOUBLE_TAP_SCALE;
                    let pan = (
                        x - cx - (base.0 - cx) * target,
                        y - cy - (base.1 - cy) * target,
                    );
                    let pan = widget.clamp_pan(target, pan);
                    widget.animate_to(target, pan);
                }
            });
            obj.add_controller(click);
        }
    }

    impl WidgetImpl for ZoomPicture {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let obj = self.obj();
            let Some(texture) = self.texture.borrow().clone() else {
                return;
            };
            let (base, size) = obj.fit_rect(&texture);
            let (cx, cy) = obj.content_center();
            let scale = self.scale.get();
            let pan = (self.pan_x.get(), self.pan_y.get());

            snapshot.save();
            snapshot.translate(&graphene::Point::new(
                (cx + pan.0) as f32,
                (cy + pan.1) as f32,
            ));
            snapshot.scale(scale as f32, scale as f32);
            snapshot.translate(&graphene::Point::new(-cx as f32, -cy as f32));
            snapshot.append_texture(
                &texture,
                &graphene::Rect::new(base.0 as f32, base.1 as f32, size.0 as f32, size.1 as f32),
            );
            snapshot.restore();
        }

        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let natural = self
                .texture
                .borrow()
                .as_ref()
                .map(|texture| match orientation {
                    gtk::Orientation::Horizontal => texture.width(),
                    _ => texture.height(),
                })
                .unwrap_or(0);
            (0, natural, -1, -1)
        }
    }
}

glib::wrapper! {
    pub struct ZoomPicture(ObjectSubclass<imp::ZoomPicture>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ZoomPicture {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn set_texture(&self, texture: Option<&gdk::Texture>) {
        *self.imp().texture.borrow_mut() = texture.cloned();
        self.reset();
        self.queue_draw();
    }

    pub fn clear(&self) {
        *self.imp().texture.borrow_mut() = None;
        self.reset();
        self.queue_draw();
    }

    fn scale(&self) -> f64 {
        self.imp().scale.get()
    }

    fn pan(&self) -> (f64, f64) {
        (self.imp().pan_x.get(), self.imp().pan_y.get())
    }

    fn content_center(&self) -> (f64, f64) {
        (self.width() as f64 / 2.0, self.height() as f64 / 2.0)
    }

    fn fit_rect(&self, texture: &gdk::Texture) -> ((f64, f64), (f64, f64)) {
        let width = self.width() as f64;
        let height = self.height() as f64;
        let tw = texture.width() as f64;
        let th = texture.height() as f64;
        if tw <= 0.0 || th <= 0.0 || width <= 0.0 || height <= 0.0 {
            return ((0.0, 0.0), (0.0, 0.0));
        }
        let fit = (width / tw).min(height / th);
        let w = tw * fit;
        let h = th * fit;
        (((width - w) / 2.0, (height - h) / 2.0), (w, h))
    }

    fn clamp_pan(&self, scale: f64, pan: (f64, f64)) -> (f64, f64) {
        let Some(texture) = self.imp().texture.borrow().clone() else {
            return pan;
        };
        let (base, size) = self.fit_rect(&texture);
        let (cx, cy) = self.content_center();
        let widget_w = self.width() as f64;
        let widget_h = self.height() as f64;

        let mut x = pan.0;
        let content_w = size.0 * scale;
        if content_w <= widget_w {
            x = (widget_w - content_w) / 2.0 - cx - (base.0 - cx) * scale;
        } else {
            let x0 = cx + x + (base.0 - cx) * scale;
            let clamped = x0.clamp(widget_w - content_w, 0.0);
            x += clamped - x0;
        }

        let mut y = pan.1;
        let content_h = size.1 * scale;
        if content_h <= widget_h {
            y = (widget_h - content_h) / 2.0 - cy - (base.1 - cy) * scale;
        } else {
            let y0 = cy + y + (base.1 - cy) * scale;
            let clamped = y0.clamp(widget_h - content_h, 0.0);
            y += clamped - y0;
        }

        (x, y)
    }

    fn set_zoom(&self, scale: f64, pan: (f64, f64)) {
        let scale = scale.clamp(1.0, MAX_SCALE);
        let pan = self.clamp_pan(scale, pan);
        self.imp().scale.set(scale);
        self.imp().pan_x.set(pan.0);
        self.imp().pan_y.set(pan.1);

        if let Some(carousel) = self
            .ancestor(adw::Carousel::static_type())
            .and_downcast::<adw::Carousel>()
        {
            carousel.set_interactive(scale <= ZOOMED_EPSILON);
        }
        self.queue_draw();
    }

    /// Zoom keeping the widget point `focus` fixed under the fingers.
    fn zoom_to(&self, scale: f64, focus: Option<(f64, f64)>, animate: bool) {
        if !animate {
            self.cancel_animation();
        }
        let scale = scale.clamp(1.0, MAX_SCALE);
        let (cx, cy) = self.content_center();
        let current_scale = self.scale();
        let current_pan = self.pan();
        let focus = focus.unwrap_or((cx, cy));
        let base = (
            cx + (focus.0 - cx - current_pan.0) / current_scale,
            cy + (focus.1 - cy - current_pan.1) / current_scale,
        );
        let pan = (
            focus.0 - cx - (base.0 - cx) * scale,
            focus.1 - cy - (base.1 - cy) * scale,
        );

        if animate {
            self.animate_to(scale, pan);
        } else {
            self.set_zoom(scale, pan);
        }
    }

    fn animate_to(&self, scale: f64, pan: (f64, f64)) {
        self.cancel_animation();

        let start_scale = self.scale();
        let start_pan = self.pan();
        let (to_scale, to_pan) = (scale, pan);
        let started = Instant::now();
        let weak = self.downgrade();
        let id = glib::timeout_add_local(Duration::from_millis(16), move || {
            let Some(widget) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let t = (started.elapsed().as_secs_f64() / ANIMATION.as_secs_f64()).min(1.0);
            let eased = 1.0 - (1.0 - t).powi(3);
            let scale = start_scale + (to_scale - start_scale) * eased;
            let x = start_pan.0 + (to_pan.0 - start_pan.0) * eased;
            let y = start_pan.1 + (to_pan.1 - start_pan.1) * eased;
            widget.set_zoom(scale, (x, y));

            if t >= 1.0 {
                widget.imp().anim.borrow_mut().take();
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        *self.imp().anim.borrow_mut() = Some(id);
    }

    fn cancel_animation(&self) {
        if let Some(id) = self.imp().anim.borrow_mut().take() {
            id.remove();
        }
    }

    pub fn reset(&self) {
        self.cancel_animation();
        self.set_zoom(1.0, (0.0, 0.0));
    }

    pub fn zoom_in(&self) {
        self.zoom_to((self.scale() * 1.5).min(MAX_SCALE), None, true);
    }

    pub fn zoom_out(&self) {
        let target = self.scale() / 1.5;
        if target <= 1.03 {
            self.animate_to(1.0, (0.0, 0.0));
        } else {
            self.zoom_to(target, None, true);
        }
    }
}
