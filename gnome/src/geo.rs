//! Device location via the XDG **Location portal** (`org.freedesktop.portal.Location`),
//! using `ashpd`. The portal is GeoClue-backed and is the only path that works both
//! natively and inside the Flatpak sandbox (direct GeoClue2 D-Bus is refused for sandboxed
//! peers). GNOME counterpart of the Swift `LocationPermissionManager` + MapKit
//! `UserAnnotation` / `MapUserLocationButton`.
//!
//! Desktops have no GPS or magnetometer: fixes come from WiFi/IP (city-to-street accuracy)
//! and heading is unavailable, so the Swift heading-follow focus state has no equivalent.

use ashpd::desktop::location::{Accuracy, CreateSessionOptions, LocationProxy};
use futures_util::StreamExt;

use crate::runtime::spawn_to_main;

/// Fetch the current device location once via the portal, delivering `(lat, lon)` to
/// `on_main` on the GLib main thread. `None` on failure or denied permission. The portal
/// shows its own one-time permission prompt on first use.
pub fn current_location<F: FnOnce(Option<(f64, f64)>) + 'static>(on_main: F) {
    spawn_to_main(fetch(), on_main);
}

async fn fetch() -> Option<(f64, f64)> {
    let proxy = LocationProxy::new().await.ok()?;
    let session = proxy
        .create_session(CreateSessionOptions::default().set_accuracy(Accuracy::Exact))
        .await
        .ok()?;
    let mut stream = proxy.receive_location_updated().await.ok()?;
    // `start` only resolves once the session is active; the first fix arrives on the
    // stream, so await both together (per the ashpd Location example).
    let (start, location) =
        futures_util::join!(proxy.start(&session, None, Default::default()), stream.next());
    let coord = match (start, location) {
        (Ok(_), Some(l)) => Some((l.latitude(), l.longitude())),
        _ => None,
    };
    let _ = session.close().await;
    coord
}
