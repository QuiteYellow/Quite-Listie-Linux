//! Reusable `libshumate` map widget — the GTK counterpart of the SwiftUI `Map` used in
//! `MapListView` / `GlobalMapView` / `LocationPickerSheet`. Wraps a [`shumate::SimpleMap`]
//! (OpenStreetMap tiles, with the built-in compass / scale / zoom buttons) and a
//! [`shumate::MarkerLayer`] for the pins.
//!
//! Markers are label-tinted (Pango-markup dot, or the label's emoji) like the rest of the
//! app's label affordances; each is a flat button that invokes the selection callback —
//! the GNOME stand-in for MapKit's `Marker` selection + popover anchor.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use shumate::prelude::*;

use crate::models::PinObject;

/// Below this zoom level pin name labels are hidden, leaving just the circles — names
/// appear only once zoomed to roughly street level, to avoid label clutter on overviews
/// (Apple Maps shows marker titles the same way).
const PIN_LABEL_MIN_ZOOM: f64 = 14.0;

/// A configured OSM map plus its marker layer. Hold onto it for the lifetime of the page
/// (the widgets are owned by the returned [`shumate::SimpleMap`]).
pub struct MapView {
    simple: shumate::SimpleMap,
    layer: shumate::MarkerLayer,
    /// Separate layer for the user-location dot so it survives pin rebuilds.
    user_layer: shumate::MarkerLayer,
    /// Pin name labels, toggled by zoom (see [`PIN_LABEL_MIN_ZOOM`]).
    pin_labels: Rc<RefCell<Vec<gtk::Label>>>,
    /// Item id -> the marker's clickable widget, so the pin popover can re-anchor itself to
    /// another pin when cycling to nearby items (Swift `MapListView` prev/next).
    anchors: Rc<RefCell<HashMap<String, gtk::Widget>>>,
}

impl MapView {
    pub fn new() -> Self {
        let simple = shumate::SimpleMap::builder().show_zoom_buttons(true).build();

        // VersaTiles Shortbread vector basemap (graybeard = muted light, shadow = dark) —
        // the GNOME-Maps-like simple look and the GTK equivalent of Swift's `mapStyleMuted`,
        // resolution-independent (no @2x tiles needed). Falls back to muted CARTO raster
        // when the libshumate build lacks vector support. Rebuild on colour-scheme change.
        let style = adw::StyleManager::default();
        let rebuild = glib::clone!(
            #[weak]
            simple,
            move || set_basemap(&simple, adw::StyleManager::default().is_dark())
        );
        rebuild();
        style.connect_dark_notify(glib::clone!(
            #[strong]
            rebuild,
            move |_| rebuild()
        ));
        // Raster fallback only: @2x tiles depend on scale factor (unknown until realized,
        // and it changes when the window moves between monitors of differing scale). Vector
        // tiles are resolution-independent, so they need no scale rebuild.
        if !shumate::VectorRenderer::is_supported() {
            simple.connect_scale_factor_notify(glib::clone!(
                #[strong]
                rebuild,
                move |_| rebuild()
            ));
        }

        let viewport = simple.viewport().expect("SimpleMap always has a viewport");
        let layer = shumate::MarkerLayer::new(&viewport);
        simple.add_overlay_layer(&layer);
        // User-location dot on its own layer (added last so it draws above the pins) and
        // never cleared by `set_pins`.
        let user_layer = shumate::MarkerLayer::new(&viewport);
        simple.add_overlay_layer(&user_layer);

        // Show/hide pin names as the user zooms past the threshold.
        let pin_labels: Rc<RefCell<Vec<gtk::Label>>> = Rc::new(RefCell::new(Vec::new()));
        let update = glib::clone!(
            #[strong]
            pin_labels,
            move |zoom: f64| {
                let show = zoom >= PIN_LABEL_MIN_ZOOM;
                for l in pin_labels.borrow().iter() {
                    l.set_visible(show);
                }
            }
        );
        update(viewport.zoom_level());
        viewport.connect_zoom_level_notify(move |vp| update(vp.zoom_level()));

        Self {
            simple,
            layer,
            user_layer,
            pin_labels,
            anchors: Rc::new(RefCell::new(HashMap::new())),
        }
    }

    /// The map widget — embed this (e.g. in an `adw::ToolbarView` content or an overlay).
    pub fn widget(&self) -> &shumate::SimpleMap {
        &self.simple
    }

    /// The inner [`shumate::Map`] (for camera moves).
    fn map(&self) -> shumate::Map {
        self.simple.map().expect("SimpleMap always has a map")
    }

    /// Replace the markers with one per pin. Each marker invokes `on_select` with its pin
    /// and its button widget (an anchor for a popover).
    pub fn set_pins(&self, pins: &[PinObject], on_select: Rc<dyn Fn(PinObject, gtk::Widget)>) {
        self.layer.remove_all();
        self.pin_labels.borrow_mut().clear();
        self.anchors.borrow_mut().clear();
        let show_labels =
            self.simple.viewport().map(|v| v.zoom_level()).unwrap_or(0.0) >= PIN_LABEL_MIN_ZOOM;
        for pin in pins {
            let marker = shumate::Marker::new();
            let (widget, label, anchor) = pin_widget(pin, &on_select);
            if let Some(label) = label {
                label.set_visible(show_labels);
                self.pin_labels.borrow_mut().push(label);
            }
            self.anchors.borrow_mut().insert(pin.item_id(), anchor);
            marker.set_child(Some(&widget));
            marker.set_location(pin.latitude(), pin.longitude());
            self.layer.add_marker(&marker);
        }
    }

    /// The clickable marker widget for `item_id`, used as a popover anchor when the pin
    /// popover cycles to a nearby pin (Swift prev/next). `None` if the pin isn't shown.
    pub fn anchor_for(&self, item_id: &str) -> Option<gtk::Widget> {
        self.anchors.borrow().get(item_id).cloned()
    }

    /// Subset of `pins` whose coordinate falls inside the currently visible map region
    /// (Swift `viewportItems`). Falls back to all pins before the map is realized/sized.
    pub fn viewport_pins(&self, pins: &[PinObject]) -> Vec<PinObject> {
        let Some(vp) = self.simple.viewport() else {
            return pins.to_vec();
        };
        let (w, h) = (self.simple.width() as f64, self.simple.height() as f64);
        if w <= 0.0 || h <= 0.0 {
            return pins.to_vec();
        }
        let (lat0, lon0) = vp.widget_coords_to_location(&self.simple, 0.0, 0.0);
        let (lat1, lon1) = vp.widget_coords_to_location(&self.simple, w, h);
        let (min_lat, max_lat) = (lat0.min(lat1), lat0.max(lat1));
        let (min_lon, max_lon) = (lon0.min(lon1), lon0.max(lon1));
        pins.iter()
            .filter(|p| {
                (min_lat..=max_lat).contains(&p.latitude())
                    && (min_lon..=max_lon).contains(&p.longitude())
            })
            .cloned()
            .collect()
    }

    /// Geographic coordinate at a point in the map widget's coordinate space (for the
    /// add-at-location right-click, Swift's long-press `proxy.convert`).
    pub fn coord_at_widget(&self, x: f64, y: f64) -> (f64, f64) {
        let vp = self.simple.viewport().expect("viewport");
        vp.widget_coords_to_location(&self.simple, x, y)
    }

    /// Centre the camera to show `pins`: a single pin at street zoom, or the centroid at a
    /// zoom chosen to fit the spread. No-op when empty.
    pub fn fit_to_pins(&self, pins: &[PinObject]) {
        if pins.is_empty() {
            return;
        }
        let (mut min_lat, mut max_lat) = (90.0f64, -90.0f64);
        let (mut min_lon, mut max_lon) = (180.0f64, -180.0f64);
        for p in pins {
            min_lat = min_lat.min(p.latitude());
            max_lat = max_lat.max(p.latitude());
            min_lon = min_lon.min(p.longitude());
            max_lon = max_lon.max(p.longitude());
        }
        let center_lat = (min_lat + max_lat) / 2.0;
        let center_lon = (min_lon + max_lon) / 2.0;
        let span = (max_lat - min_lat).max(max_lon - min_lon);
        self.map().go_to_full(center_lat, center_lon, fit_zoom(span));
    }

    /// Move the camera to an explicit coordinate + zoom.
    pub fn center_on(&self, lat: f64, lon: f64, zoom: f64) {
        self.map().go_to_full(lat, lon, zoom);
    }

    /// Show a single non-interactive marker at `coord`, or clear it when `None`. Used by
    /// the location picker's view mode (Swift `LocationPickerSheet` marker).
    pub fn set_marker(&self, coord: Option<(f64, f64)>) {
        self.layer.remove_all();
        self.pin_labels.borrow_mut().clear();
        self.anchors.borrow_mut().clear();
        if let Some((lat, lon)) = coord {
            let marker = shumate::Marker::new();
            let img = gtk::Image::from_icon_name("mark-location-symbolic");
            img.set_pixel_size(36);
            img.add_css_class("accent");
            marker.set_child(Some(&img));
            marker.set_location(lat, lon);
            self.layer.add_marker(&marker);
        }
    }

    /// Show the user-location dot at `coord`, or clear it when `None` (Swift
    /// `UserAnnotation`). Kept on a dedicated layer so pin rebuilds don't remove it.
    pub fn set_user_location(&self, coord: Option<(f64, f64)>) {
        self.user_layer.remove_all();
        if let Some((lat, lon)) = coord {
            let marker = shumate::Marker::new();
            let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            dot.add_css_class("ql-user-location");
            dot.set_can_target(false);
            marker.set_child(Some(&dot));
            marker.set_location(lat, lon);
            self.user_layer.add_marker(&marker);
        }
    }

    /// The current camera-centre coordinate (used by the picker crosshair).
    pub fn center_coordinate(&self) -> (f64, f64) {
        let vp = self.simple.viewport().expect("viewport");
        (vp.latitude(), vp.longitude())
    }

    /// Subscribe to camera-centre changes (the picker reads the crosshair coordinate).
    pub fn connect_center_changed<F: Fn(f64, f64) + 'static>(&self, f: F) {
        let vp = self.simple.viewport().expect("viewport");
        let f = Rc::new(f);
        vp.connect_latitude_notify(glib::clone!(
            #[strong]
            f,
            move |vp| f(vp.latitude(), vp.longitude())
        ));
        vp.connect_longitude_notify(move |vp| f(vp.latitude(), vp.longitude()));
    }
}

/// Build the pin widget: a label-coloured circle (gradient fill, dark stroke, drop shadow
/// + inset highlight for depth) with the label's emoji inside it, and the item name on a
/// small pill below — like an Apple Maps marker. Clicking the circle selects the pin.
fn pin_widget(
    pin: &PinObject,
    on_select: &Rc<dyn Fn(PinObject, gtk::Widget)>,
) -> (gtk::Widget, Option<gtk::Label>, gtk::Widget) {
    let color = if pin.label_color().is_empty() {
        "#3584e4".to_string()
    } else {
        sanitize_hex(&pin.label_color())
    };

    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    body.set_halign(gtk::Align::Center);
    body.set_valign(gtk::Align::Center);
    body.add_css_class("ql-map-pin-body");
    body.add_css_class(&pin_color_class(&color));

    let emoji = pin.label_emoji();
    if !emoji.is_empty() {
        let glyph = gtk::Label::new(Some(&emoji));
        glyph.add_css_class("ql-map-pin-emoji");
        // Let the label fill the body on both axes and centre its own text — a GtkBox
        // ignores a non-expanding child's main-axis alignment, so expand + x/y-align is the
        // reliable way to truly centre the glyph in the circle.
        glyph.set_hexpand(true);
        glyph.set_vexpand(true);
        glyph.set_halign(gtk::Align::Fill);
        glyph.set_valign(gtk::Align::Fill);
        glyph.set_xalign(0.5);
        glyph.set_yalign(0.5);
        body.append(&glyph);
    }

    let button = gtk::Button::builder()
        .child(&body)
        // Keep the circle its natural size and centred over the (wider) name label,
        // so the emoji stays centred in the pin.
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .has_frame(false)
        .tooltip_text(&pin.note())
        .css_classes(["ql-map-pin"])
        .build();
    button.connect_clicked(glib::clone!(
        #[strong]
        on_select,
        #[strong]
        pin,
        move |b| on_select(pin.clone(), b.clone().upcast())
    ));

    let container = gtk::Box::new(gtk::Orientation::Vertical, 1);
    container.set_halign(gtk::Align::Center);
    container.append(&button);

    // Apple-Maps-style pointer under the circle, in the label colour.
    let tail = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    tail.set_halign(gtk::Align::Center);
    tail.add_css_class("ql-map-pin-tail");
    tail.add_css_class(&pin_color_class(&color));
    container.append(&tail);

    let note = pin.note();
    let label = (!note.is_empty()).then(|| {
        let label = gtk::Label::new(Some(&note));
        label.add_css_class("ql-map-pin-label");
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_max_width_chars(14);
        container.append(&label);
        label
    });
    (container.upcast(), label, button.upcast())
}

/// VersaTiles Shortbread vector basemap (bundled style JSON; tiles/glyphs/sprites fetched
/// from tiles.versatiles.org). neutrino = muted light, eclipse = dark. Falls back to the
/// CARTO raster basemap when the libshumate build lacks vector support.
fn set_basemap(simple: &shumate::SimpleMap, dark: bool) {
    if shumate::VectorRenderer::is_supported() {
        let (id, style_json) = if dark {
            ("versatiles-shadow", include_str!("../../data/styles/versatiles-shadow.json"))
        } else {
            ("versatiles-graybeard", include_str!("../../data/styles/versatiles-graybeard.json"))
        };
        match shumate::VectorRenderer::new(id, style_json) {
            Ok(source) => {
                simple.set_map_source(Some(&source));
                if let Some(vp) = simple.viewport() {
                    vp.set_reference_map_source(Some(&source));
                }
                return;
            }
            Err(e) => tracing::warn!("vector basemap failed ({e}); falling back to CARTO raster"),
        }
    }
    set_basemap_raster(simple, dark);
}

/// Muted CARTO raster basemap for the colour scheme, using @2x (512 px) tiles on HiDPI
/// displays so the raster stays crisp. `tile_size` stays 256 (the logical web-mercator tile
/// size that drives zoom/geo addressing); the renderer decodes each tile to a
/// native-resolution texture, so a 512 px @2x image renders sharp on a 2× monitor rather
/// than being upscaled from 256. Fallback for libshumate builds without vector support.
fn set_basemap_raster(simple: &shumate::SimpleMap, dark: bool) {
    let retina = simple.scale_factor() >= 2;
    let style = if dark { "dark_all" } else { "light_all" };
    let suffix = if retina { "@2x" } else { "" };
    let url = format!("https://a.basemaps.cartocdn.com/{style}/{{z}}/{{x}}/{{y}}{suffix}.png");
    // Distinct id per (scheme, density) so the on-disk tile cache never mixes 256/512 px.
    let id = format!("carto-{style}{}", if retina { "-2x" } else { "" });
    let name = if dark { "CARTO Dark Matter" } else { "CARTO Positron" };

    let source = shumate::RasterRenderer::new_full_from_url(
        &id,
        name,
        "© OpenStreetMap contributors, © CARTO",
        "https://carto.com/attributions",
        0,
        20,
        256,
        shumate::MapProjection::Mercator,
        &url,
    );
    simple.set_map_source(Some(&source));
    if let Some(vp) = simple.viewport() {
        vp.set_reference_map_source(Some(&source));
    }
}

thread_local! {
    /// CSS classes already registered for marker colours (one provider per distinct hex).
    static PIN_COLOR_CLASSES: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

/// Ensure a `.ql-pin-<hex>` rule exists that colours the marker ring (circle border) and
/// the pointer (tail) in `hex`, and return the class name. GTK has no per-widget inline
/// colour, so we register one tiny provider per distinct label colour (cached so repeats
/// are free). The class is applied to both the circle body and the tail.
fn pin_color_class(hex: &str) -> String {
    let safe = sanitize_hex(hex);
    let class = format!("ql-pin-{}", &safe[1..]);
    PIN_COLOR_CLASSES.with(|seen| {
        if seen.borrow_mut().insert(class.clone()) {
            let provider = gtk::CssProvider::new();
            // Circle: the label colour is the ring (border). Tail: only the top border is
            // coloured (left/right stay transparent so the triangle shape is preserved).
            provider.load_from_string(&format!(
                ".ql-map-pin-body.{class} {{ border-color: {safe}; }}\n\
                 .ql-map-pin-tail.{class} {{ border-top-color: {safe}; }}"
            ));
            if let Some(display) = gtk::gdk::Display::default() {
                gtk::style_context_add_provider_for_display(
                    &display,
                    &provider,
                    gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
                );
            }
        }
    });
    class
}

// --- shared maps helpers (used by the item editor + pin popover) -----------

/// A flat link-style button that opens `uri` in the default handler (browser).
pub fn maps_link(label: &str, uri: &str) -> gtk::Button {
    let button = gtk::Button::builder().label(label).has_frame(false).build();
    button.add_css_class("accent");
    let uri = uri.to_string();
    button.connect_clicked(move |_| {
        let _ = gio::AppInfo::launch_default_for_uri(&uri, gio::AppLaunchContext::NONE);
    });
    button
}

pub fn google_maps_uri(lat: f64, lon: f64) -> String {
    format!("https://www.google.com/maps/search/?api=1&query={lat},{lon}")
}

pub fn osm_uri(lat: f64, lon: f64) -> String {
    format!("https://www.openstreetmap.org/?mlat={lat}&mlon={lon}#map=16/{lat}/{lon}")
}

/// Label for a source-URL link, by host substring (mirrors Swift `sourceURLLabel`).
pub fn source_url_label(url: &str) -> String {
    let u = url.to_lowercase();
    if u.contains("google.com") || u.contains("goo.gl") {
        "View in Google Maps".to_string()
    } else if u.contains("apple.com") || u.contains("link.maps.apple") {
        "View in Apple Maps".to_string()
    } else {
        "View in Maps".to_string()
    }
}

/// Pick a zoom level that fits a lat/lon span (degrees) into the viewport. Single-point
/// spans get street-level zoom; wider spreads zoom out, clamped to a sane range.
fn fit_zoom(span_deg: f64) -> f64 {
    if span_deg <= 0.0001 {
        return 14.0;
    }
    // 360° spans the world at zoom 0; halve the span per zoom level. Pad by 1.5×.
    let zoom = (360.0 / (span_deg * 1.5)).log2();
    zoom.clamp(2.0, 16.0)
}

/// Only allow a `#rrggbb`/`#rgb` hex value through to Pango markup.
fn sanitize_hex(hex: &str) -> String {
    if hex.starts_with('#') && hex.len() <= 9 && hex[1..].chars().all(|c| c.is_ascii_hexdigit()) {
        hex.to_string()
    } else {
        "#3584e4".to_string()
    }
}
