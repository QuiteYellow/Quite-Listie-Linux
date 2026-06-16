//! `PinPopover` — the popover shown when a map marker is tapped (per-list and global
//! maps). GNOME counterpart of Swift `MapPinPopover` / `MapPinSheetContent`: the item
//! note, open-in-maps links, the source URL (if any), and a "Show Details" action that
//! opens the item editor. Prev/next chevrons cycle through the nearest unvisited pins in
//! the current viewport (Swift `MapListView` visit-history cycling).

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::models::PinObject;
use crate::widgets::map::{self, MapView};

/// Show the popover for `pin`, anchored to its marker `anchor`. `pins` is the filtered set
/// currently on the map (the cycling candidates); `on_show_details` opens the editor for
/// the chosen pin. The popover owns a visit-history stack so prev/next walk nearby pins.
pub fn present(
    map: Rc<MapView>,
    pins: Rc<Vec<PinObject>>,
    pin: PinObject,
    anchor: gtk::Widget,
    on_show_details: Rc<dyn Fn(PinObject)>,
) {
    let history = Rc::new(RefCell::new(vec![pin.item_id()]));
    present_pin(map, pins, pin, anchor, on_show_details, history);
}

fn present_pin(
    map: Rc<MapView>,
    pins: Rc<Vec<PinObject>>,
    pin: PinObject,
    anchor: gtk::Widget,
    on_show_details: Rc<dyn Fn(PinObject)>,
    history: Rc<RefCell<Vec<String>>>,
) {
    let popover = gtk::Popover::builder().autohide(true).build();
    popover.set_parent(&anchor);
    // A transient popover parented to a marker that may be rebuilt — drop it cleanly.
    popover.connect_closed(|p| p.unparent());

    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(8)
        .margin_end(8)
        .width_request(240)
        .build();

    // Title row with prev/next chevrons (Swift MapPinPopover header arrows).
    let title_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let prev = gtk::Button::from_icon_name("go-previous-symbolic");
    prev.add_css_class("flat");
    let title = gtk::Label::new(Some(&pin.note()));
    title.add_css_class("heading");
    title.set_wrap(true);
    title.set_hexpand(true);
    title.set_xalign(0.5);
    let next = gtk::Button::from_icon_name("go-next-symbolic");
    next.add_css_class("flat");
    title_row.append(&prev);
    title_row.append(&title);
    title_row.append(&next);
    outer.append(&title_row);

    let visited: HashSet<String> = history.borrow().iter().cloned().collect();
    let next_pin = nearest_unvisited(&map, &pins, &pin, &visited);
    prev.set_sensitive(history.borrow().len() > 1);
    next.set_sensitive(next_pin.is_some());

    // The originating list (helps in the global map).
    let list_name = pin.list_name();
    if !list_name.is_empty() {
        let sub = gtk::Label::new(Some(&list_name));
        sub.add_css_class("caption");
        sub.add_css_class("dim-label");
        sub.set_xalign(0.0);
        outer.append(&sub);
    }

    outer.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

    let (lat, lon) = (pin.latitude(), pin.longitude());
    outer.append(&link("Open in Google Maps", &map::google_maps_uri(lat, lon)));
    outer.append(&link("Open in OpenStreetMap", &map::osm_uri(lat, lon)));

    let url = pin.source_url();
    if !url.is_empty() {
        outer.append(&link(&map::source_url_label(&url), &url));
    }

    outer.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

    let details = link_button("Show Details");
    details.connect_clicked(glib::clone!(
        #[weak]
        popover,
        #[strong]
        on_show_details,
        #[strong]
        pin,
        move |_| {
            popover.popdown();
            on_show_details(pin.clone());
        }
    ));
    outer.append(&details);

    // Cycle to the nearest unvisited pin: re-anchor the popover to its marker.
    next.connect_clicked(glib::clone!(
        #[weak]
        popover,
        #[strong]
        map,
        #[strong]
        pins,
        #[strong]
        on_show_details,
        #[strong]
        history,
        move |_| {
            let Some(target) = next_pin.clone() else { return };
            let id = target.item_id();
            let Some(new_anchor) = map.anchor_for(&id) else { return };
            history.borrow_mut().push(id);
            popover.popdown();
            present_pin(map.clone(), pins.clone(), target, new_anchor, on_show_details.clone(), history.clone());
        }
    ));

    // Step back to the previously visited pin.
    prev.connect_clicked(glib::clone!(
        #[weak]
        popover,
        #[strong]
        map,
        #[strong]
        pins,
        #[strong]
        on_show_details,
        #[strong]
        history,
        move |_| {
            let prev_id = {
                let mut h = history.borrow_mut();
                if h.len() <= 1 {
                    return;
                }
                h.pop();
                h.last().cloned()
            };
            let Some(prev_id) = prev_id else { return };
            let Some(target) = map.viewport_pins(&pins).into_iter().find(|p| p.item_id() == prev_id) else { return };
            let Some(new_anchor) = map.anchor_for(&prev_id) else { return };
            popover.popdown();
            present_pin(map.clone(), pins.clone(), target, new_anchor, on_show_details.clone(), history.clone());
        }
    ));

    popover.set_child(Some(&outer));
    popover.popup();
}

/// Nearest pin (squared planar distance) within the visible viewport that hasn't been
/// visited yet (Swift `nextItem`).
fn nearest_unvisited(
    map: &MapView,
    pins: &[PinObject],
    origin: &PinObject,
    visited: &HashSet<String>,
) -> Option<PinObject> {
    let (olat, olon) = (origin.latitude(), origin.longitude());
    map.viewport_pins(pins)
        .into_iter()
        .filter(|p| !visited.contains(&p.item_id()))
        .min_by(|a, b| dist2(olat, olon, a).total_cmp(&dist2(olat, olon, b)))
}

fn dist2(lat: f64, lon: f64, p: &PinObject) -> f64 {
    let dlat = lat - p.latitude();
    let dlon = lon - p.longitude();
    dlat * dlat + dlon * dlon
}

fn link(label: &str, uri: &str) -> gtk::Button {
    let b = map::maps_link(label, uri);
    b.set_halign(gtk::Align::Fill);
    if let Some(child) = b.child().and_downcast::<gtk::Label>() {
        child.set_xalign(0.0);
    }
    b
}

fn link_button(label: &str) -> gtk::Button {
    gtk::Button::builder().label(label).has_frame(false).halign(gtk::Align::Fill).build()
}
