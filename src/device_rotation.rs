// SPDX-License-Identifier: GPL-3.0-or-later
//
// Copyright 2026 nico359
//
// Device orientation from iio-sensor-proxy.  The camera engine needs the
// device tilt to tag JPEG/video orientation; the preview can also use it to
// stay upright when the UI rotates.

use gtk::gio;
use gtk::gio::prelude::*;
use gtk::glib;

pub struct DeviceRotation {
    proxy: Option<gio::DBusProxy>,
}

impl DeviceRotation {
    pub fn new() -> Self {
        let proxy = gio::DBusProxy::for_bus_sync(
            gio::BusType::System,
            gio::DBusProxyFlags::NONE,
            None,
            "net.hadess.SensorProxy",
            "/net/hadess/SensorProxy",
            "net.hadess.SensorProxy",
            None::<&gio::Cancellable>,
        )
        .ok();

        if let Some(proxy) = &proxy {
            let _ = proxy.call_sync(
                "ClaimAccelerometer",
                None::<&glib::Variant>,
                gio::DBusCallFlags::NONE,
                -1,
                None::<&gio::Cancellable>,
            );
        }

        Self { proxy }
    }

    pub fn degrees(&self) -> i32 {
        self.proxy.as_ref().map(Self::degrees_from).unwrap_or(0)
    }

    pub fn degrees_from(proxy: &gio::DBusProxy) -> i32 {
        let Some(value) = proxy.cached_property("AccelerometerOrientation") else {
            return 0;
        };
        let Some(orientation) = value.str() else {
            return 0;
        };

        match orientation {
            "left-up" => 90,
            "bottom-up" => 180,
            "right-up" => 270,
            _ => 0,
        }
    }
}

impl Default for DeviceRotation {
    fn default() -> Self {
        Self::new()
    }
}
