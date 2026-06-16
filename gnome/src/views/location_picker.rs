//! `LocationPicker` — pick or refine an item's pinned coordinate on a map. GNOME
//! counterpart of the Swift `LocationPickerSheet`. Two modes mirror Swift:
//! * **View** (opened for an existing pin): a marker at the coordinate, no crosshair or
//!   search, with an "Edit" button.
//! * **Edit** (fresh pick, or after tapping Edit): a fixed centre crosshair the map pans
//!   under, an address search bar, a live coordinate readout, and Save.
//! The crosshair-under-a-panning-map model replaces MapKit's `onMapCameraChange` tracking.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::geo;
use crate::models;
use crate::widgets::map::MapView;

/// Present the picker. `initial` pre-centres the map (street zoom) and opens in view mode;
/// `None` starts zoomed out in edit mode. `on_done` receives the chosen `(lat, lon)` on Save.
pub fn present(
    parent: &impl IsA<gtk::Widget>,
    initial: Option<(f64, f64)>,
    on_done: Rc<dyn Fn(f64, f64)>,
) {
    let had_initial = initial.is_some();
    let dialog = adw::Dialog::builder().content_width(560).content_height(560).build();

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    // Trailing button is "Save" in edit mode, "Edit" in view mode.
    let action_button = gtk::Button::new();
    action_button.add_css_class("suggested-action");
    header.pack_end(&action_button);
    toolbar.add_top_bar(&header);

    // --- map + centre crosshair + coordinate readout -----------------------
    let map = Rc::new(MapView::new());
    let overlay = gtk::Overlay::builder().hexpand(true).vexpand(true).build();
    overlay.set_child(Some(map.widget()));

    // Fixed centre crosshair (the map pans beneath it) — edit mode only.
    let crosshair = gtk::Image::from_icon_name("find-location-symbolic");
    crosshair.set_pixel_size(36);
    crosshair.set_halign(gtk::Align::Center);
    crosshair.set_valign(gtk::Align::Center);
    crosshair.set_can_target(false);
    crosshair.add_css_class("accent");
    overlay.add_overlay(&crosshair);

    // Live coordinate readout, bottom-centre.
    let readout = gtk::Label::new(None);
    readout.add_css_class("ql-coord-readout");
    readout.set_halign(gtk::Align::Center);
    readout.set_valign(gtk::Align::End);
    readout.set_margin_bottom(18);
    readout.set_can_target(false);
    overlay.add_overlay(&readout);

    let set_readout = {
        let readout = readout.clone();
        Rc::new(move |lat: f64, lon: f64| readout.set_text(&format!("{lat:.4}°, {lon:.4}°")))
    };

    // Current committed coordinate (the view-mode marker / edit-mode start point).
    let coord = Rc::new(Cell::new(initial));
    let is_editing = Rc::new(Cell::new(initial.is_none()));

    // In view mode the readout shows the marker coordinate, not the (pannable) centre.
    map.connect_center_changed(glib::clone!(
        #[strong]
        set_readout,
        #[strong]
        is_editing,
        move |lat, lon| {
            if is_editing.get() {
                set_readout(lat, lon);
            }
        }
    ));

    toolbar.set_content(Some(&overlay));

    // --- address search bar (top) — edit mode only -------------------------
    let search = gtk::SearchEntry::builder().placeholder_text("Search address").hexpand(true).build();
    let search_btn = gtk::Button::with_label("Search");
    search_btn.add_css_class("suggested-action");
    let searching = Rc::new(Cell::new(false));
    let do_search = glib::clone!(
        #[weak]
        search,
        #[strong]
        map,
        #[strong]
        set_readout,
        #[strong]
        searching,
        move || {
            let query = search.text().trim().to_string();
            if query.is_empty() || searching.get() {
                return;
            }
            searching.set(true);
            search.remove_css_class("error");
            let map = map.clone();
            let set_readout = set_readout.clone();
            let searching = searching.clone();
            let search = search.clone();
            models::resolve_location_async(&query, move |result| {
                searching.set(false);
                if let Some((lat, lon, _)) = result {
                    map.center_on(lat, lon, 15.0);
                    set_readout(lat, lon);
                } else {
                    // Failed geocode (Swift's red error indicator), cleared on next edit.
                    search.add_css_class("error");
                }
            });
        }
    );
    search.connect_activate(glib::clone!(
        #[strong]
        do_search,
        move |_| do_search()
    ));
    search.connect_search_changed(glib::clone!(
        #[weak]
        search,
        move |_| search.remove_css_class("error")
    ));
    search_btn.connect_clicked(glib::clone!(
        #[strong]
        do_search,
        move |_| do_search()
    ));
    let search_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(6)
        .margin_end(6)
        .build();
    // "Use my location" (Swift MapUserLocationButton): centre the crosshair on the device.
    let locate_btn = gtk::Button::from_icon_name("find-location-symbolic");
    locate_btn.set_tooltip_text(Some("Use my location"));
    locate_btn.connect_clicked(glib::clone!(
        #[weak]
        locate_btn,
        #[strong]
        map,
        #[strong]
        set_readout,
        move |_| {
            locate_btn.set_sensitive(false);
            geo::current_location(glib::clone!(
                #[weak]
                locate_btn,
                #[strong]
                map,
                #[strong]
                set_readout,
                move |coord| {
                    locate_btn.set_sensitive(true);
                    if let Some((lat, lon)) = coord {
                        map.center_on(lat, lon, 15.0);
                        set_readout(lat, lon);
                    }
                }
            ));
        }
    ));
    search_box.append(&search);
    search_box.append(&search_btn);
    search_box.append(&locate_btn);
    toolbar.add_top_bar(&search_box);

    // --- mode application --------------------------------------------------
    let apply_mode = glib::clone!(
        #[weak]
        dialog,
        #[weak]
        crosshair,
        #[weak]
        search_box,
        #[weak]
        action_button,
        #[strong]
        map,
        #[strong]
        coord,
        #[strong]
        is_editing,
        #[strong]
        set_readout,
        move || {
            let editing = is_editing.get();
            crosshair.set_visible(editing);
            search_box.set_visible(editing);
            if editing {
                map.set_marker(None);
                action_button.set_label("Save");
                dialog.set_title(if had_initial { "Update Location" } else { "Choose Location" });
                if let Some((lat, lon)) = coord.get() {
                    map.center_on(lat, lon, 15.0);
                    set_readout(lat, lon);
                }
            } else {
                action_button.set_label("Edit");
                dialog.set_title("View Location");
                if let Some((lat, lon)) = coord.get() {
                    map.set_marker(Some((lat, lon)));
                    map.center_on(lat, lon, 15.0);
                    set_readout(lat, lon);
                }
            }
        }
    );

    action_button.connect_clicked(glib::clone!(
        #[weak]
        dialog,
        #[strong]
        map,
        #[strong]
        is_editing,
        #[strong]
        on_done,
        #[strong]
        apply_mode,
        move |_| {
            if is_editing.get() {
                let (lat, lon) = map.center_coordinate();
                on_done(lat, lon);
                dialog.close();
            } else {
                is_editing.set(true);
                apply_mode();
            }
        }
    ));

    apply_mode();

    dialog.set_child(Some(&toolbar));
    dialog.present(Some(parent));
}
