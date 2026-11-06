// SPDX-License-Identifier: GPL-2.0-or-later
//
// Copyright 2026 nico359

//! Location portal watcher and GPS EXIF writing.
//!
//! Fixes are kept in memory only and expire quickly; we never persist a fix or
//! reuse an old one. A photo is tagged only when the latest fix is fresh and,
//! by default, accurate to 10 metres or better.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Instant;

use ashpd::desktop::location::{Accuracy, CreateSessionOptions, LocationProxy};
use futures_util::StreamExt;
use little_exif::exif_tag::ExifTag;
use little_exif::metadata::Metadata;
use little_exif::rational::uR64;

/// A location fix received from the portal.
#[derive(Clone, Debug)]
pub struct LocationFix {
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: Option<f64>,
    pub accuracy: f64,
    pub received: Instant,
}

/// Start watching the XDG location portal in a background thread.
pub fn spawn(sender: Sender<LocationFix>, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();
        let Ok(runtime) = runtime else {
            eprintln!("Location: could not create the async runtime");
            return;
        };

        runtime.block_on(async {
            if let Err(error) = watch(sender, stop).await {
                eprintln!("Location portal error: {error}");
            }
        });
    });
}

async fn watch(sender: Sender<LocationFix>, stop: Arc<AtomicBool>) -> ashpd::Result<()> {
    let proxy = LocationProxy::new().await?;
    let session = proxy
        .create_session(
            CreateSessionOptions::default()
                .set_accuracy(Accuracy::Exact)
                .set_distance_threshold(0u32)
                .set_time_threshold(1u32),
        )
        .await?;

    // Subscribe before starting so the first update is not lost.
    let mut stream = proxy.receive_location_updated().await?;
    proxy.start(&session, None, Default::default()).await?;

    while let Some(location) = stream.next().await {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        let fix = LocationFix {
            latitude: location.latitude(),
            longitude: location.longitude(),
            altitude: location.altitude(),
            accuracy: location.accuracy().max(0.0),
            received: Instant::now(),
        };
        if sender.send(fix).is_err() {
            break;
        }
    }

    Ok(())
}

fn degrees_minutes_seconds(value: f64) -> (f64, f64, f64) {
    let value = value.abs();
    let degrees = value.trunc();
    let minutes_total = (value - degrees) * 60.0;
    let minutes = minutes_total.trunc();
    let seconds = (minutes_total - minutes) * 60.0;
    (degrees, minutes, seconds)
}

/// Add GPS tags to a JPEG, preserving its existing EXIF data.
pub fn write_gps(path: &Path, fix: &LocationFix) -> Result<(), Box<dyn std::error::Error>> {
    let mut metadata = Metadata::new_from_path(path)?;

    let (lat_d, lat_m, lat_s) = degrees_minutes_seconds(fix.latitude);
    metadata.set_tag(ExifTag::GPSLatitude(vec![
        uR64::from(lat_d),
        uR64::from(lat_m),
        uR64::from(lat_s),
    ]));
    metadata.set_tag(ExifTag::GPSLatitudeRef(
        if fix.latitude >= 0.0 { "N" } else { "S" }.to_string(),
    ));

    let (lon_d, lon_m, lon_s) = degrees_minutes_seconds(fix.longitude);
    metadata.set_tag(ExifTag::GPSLongitude(vec![
        uR64::from(lon_d),
        uR64::from(lon_m),
        uR64::from(lon_s),
    ]));
    metadata.set_tag(ExifTag::GPSLongitudeRef(
        if fix.longitude >= 0.0 { "E" } else { "W" }.to_string(),
    ));

    if let Some(altitude) = fix.altitude {
        metadata.set_tag(ExifTag::GPSAltitudeRef(vec![u8::from(altitude < 0.0)]));
        metadata.set_tag(ExifTag::GPSAltitude(vec![uR64::from(altitude.abs())]));
    }

    metadata.set_tag(ExifTag::GPSHPositioningError(vec![uR64::from(
        fix.accuracy,
    )]));

    if let Ok(now) = gtk::glib::DateTime::now_utc() {
        if let Ok(date) = now.format("%Y:%m:%d") {
            metadata.set_tag(ExifTag::GPSDateStamp(date.to_string()));
        }
        metadata.set_tag(ExifTag::GPSTimeStamp(vec![
            uR64::from(now.hour() as f64),
            uR64::from(now.minute() as f64),
            uR64::from(now.second() as f64),
        ]));
    }

    metadata.write_to_file(path)?;
    Ok(())
}
