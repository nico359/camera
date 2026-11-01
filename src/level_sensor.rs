// SPDX-License-Identifier: GPL-2.0-or-later
//
// Copyright 2026 nico359

//! Raw accelerometer access for the camera level.
//!
//! iio-sensor-proxy only exposes a coarse orientation/tilt string, so the
//! level uses the SensorFW accelerometer service FuriCam uses on FuriOS.  It
//! polls `xyz` at ~30 Hz while enabled and reports the roll error in degrees
//! relative to the nearest 90° (portrait or landscape).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::gio;
use gtk::gio::prelude::*;
use gtk::glib;
use gtk::glib::variant::ToVariant;

const SERVICE: &str = "com.nokia.SensorService";
const OBJECT: &str = "/SensorManager/accelerometersensor";
const INTERFACE: &str = "local.AccelerometerSensor";

pub struct LevelSensor {
    inner: Rc<Inner>,
    timeout: RefCell<Option<glib::SourceId>>,
}

struct Inner {
    proxy: Option<gio::DBusProxy>,
    session_id: i32,
    pending: Cell<bool>,
    active: Cell<bool>,
    callback: Box<dyn Fn(f64)>,
}

impl LevelSensor {
    pub fn new(callback: impl Fn(f64) + 'static) -> Self {
        let proxy = match gio::DBusProxy::for_bus_sync(
            gio::BusType::System,
            gio::DBusProxyFlags::NONE,
            None,
            SERVICE,
            OBJECT,
            INTERFACE,
            None::<&gio::Cancellable>,
        ) {
            Ok(proxy) => Some(proxy),
            Err(error) => {
                eprintln!("camera: SensorFW accelerometer unavailable: {error}");
                None
            }
        };

        Self {
            inner: Rc::new(Inner {
                proxy,
                session_id: glib::random_int_range(1000, 10_000),
                pending: Cell::new(false),
                active: Cell::new(false),
                callback: Box::new(callback),
            }),
            timeout: RefCell::new(None),
        }
    }

    pub fn set_active(&self, active: bool) {
        if active == self.inner.active.get() {
            return;
        }

        if active {
            let Some(proxy) = self.inner.proxy.as_ref() else {
                return;
            };
            let params = (self.inner.session_id,).to_variant();
            if let Err(error) = proxy.call_sync(
                "start",
                Some(&params),
                gio::DBusCallFlags::NONE,
                500,
                None::<&gio::Cancellable>,
            ) {
                eprintln!("camera: could not start SensorFW accelerometer: {error}");
                return;
            }

            self.inner.active.set(true);
            let inner = Rc::clone(&self.inner);
            let id = glib::timeout_add_local(Duration::from_millis(33), move || {
                if !inner.active.get() {
                    return glib::ControlFlow::Break;
                }
                if inner.pending.get() {
                    return glib::ControlFlow::Continue;
                }
                let Some(proxy) = inner.proxy.as_ref().cloned() else {
                    return glib::ControlFlow::Break;
                };

                inner.pending.set(true);
                let callback_inner = Rc::clone(&inner);
                proxy.call(
                    "org.freedesktop.DBus.Properties.Get",
                    Some(&(INTERFACE, "xyz").to_variant()),
                    gio::DBusCallFlags::NONE,
                    200,
                    None::<&gio::Cancellable>,
                    move |result| {
                        callback_inner.pending.set(false);
                        if !callback_inner.active.get() {
                            return;
                        }
                        let Ok(reply) = result else {
                            return;
                        };
                        let Some((value,)) = reply.get::<(glib::Variant,)>() else {
                            return;
                        };
                        let Some((_timestamp, x, y, _z)) = value.get::<(u64, f64, f64, f64)>()
                        else {
                            return;
                        };

                        let angle = x.atan2(y).to_degrees();
                        let nearest = (angle / 90.0).round() * 90.0;
                        (callback_inner.callback)(angle - nearest);
                    },
                );
                glib::ControlFlow::Continue
            });
            *self.timeout.borrow_mut() = Some(id);
        } else {
            self.inner.active.set(false);
            if let Some(id) = self.timeout.borrow_mut().take() {
                id.remove();
            }
            if let Some(proxy) = self.inner.proxy.as_ref() {
                let params = (self.inner.session_id,).to_variant();
                let _ = proxy.call_sync(
                    "stop",
                    Some(&params),
                    gio::DBusCallFlags::NONE,
                    500,
                    None::<&gio::Cancellable>,
                );
            }
        }
    }
}

impl Drop for LevelSensor {
    fn drop(&mut self) {
        self.set_active(false);
    }
}
