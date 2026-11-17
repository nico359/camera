// SPDX-License-Identifier: GPL-2.0-or-later
//
// Copyright 2026 nico359
//
// Device orientation from iio-sensor-proxy.  The camera engine needs the
// device tilt to tag JPEG/video orientation; the preview can also use it to
// stay upright when the UI rotates.

use gtk::gio;
use gtk::gio::prelude::*;
use gtk::glib;
use gtk::glib::variant::ToVariant;

pub struct DeviceRotation {
    proxy: Option<gio::DBusProxy>,
    connection: Option<gio::DBusConnection>,
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

        let connection = gio::bus_get_sync(gio::BusType::System, None::<&gio::Cancellable>).ok();

        Self { proxy, connection }
    }

    /// Current device tilt.  FuriCam queries the properties interface fresh
    /// before every capture because the proxy's cached property can lag behind
    /// the sensor; do the same here so a rotated phone never gets a stale JPEG
    /// orientation.
    pub fn degrees(&self) -> i32 {
        if let Some(connection) = &self.connection {
            if let Some(orientation) = fresh_orientation(connection) {
                return parse_orientation(&orientation);
            }
        }

        self.proxy
            .as_ref()
            .and_then(|proxy| proxy.cached_property("AccelerometerOrientation"))
            .and_then(|value| value.str().map(parse_orientation))
            .unwrap_or(0)
    }
}

fn fresh_orientation(connection: &gio::DBusConnection) -> Option<String> {
    let parameters = ("net.hadess.SensorProxy", "AccelerometerOrientation").to_variant();
    let reply = connection
        .call_sync(
            Some("net.hadess.SensorProxy"),
            "/net/hadess/SensorProxy",
            "org.freedesktop.DBus.Properties",
            "Get",
            Some(&parameters),
            None,
            gio::DBusCallFlags::NONE,
            -1,
            None::<&gio::Cancellable>,
        )
        .ok()?;
    let value = reply.get::<(glib::Variant,)>()?.0;
    value.str().map(|orientation| orientation.to_string())
}

fn parse_orientation(orientation: &str) -> i32 {
    match orientation {
        "left-up" => 90,
        "bottom-up" => 180,
        "right-up" => 270,
        _ => 0,
    }
}

impl Default for DeviceRotation {
    fn default() -> Self {
        Self::new()
    }
}
