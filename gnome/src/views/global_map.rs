//! `GlobalMap` — every pinned item across all open lists on one map, reached from the
//! sidebar's **Locations** smart box. GNOME counterpart of Swift `GlobalMapView`. Tapping
//! a marker opens the shared pin popover; "Show Details" opens the item editor in the
//! pin's source list. A search bar + filter menu (Show Completed, per-label toggles, Clear
//! All Filters) mirror the Swift filters. The map re-gathers whenever the page becomes
//! visible again.
//!
//! Deferred vs. Swift: the user-location focus cycle (needs GeoClue) and the long-press
//! add-at-location (the global map has no single target list); the core all-pins map +
//! filtering + navigation parity is here.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::controller::Controller;
use crate::geo;
use crate::models::{self, ListItemModel, PinObject};
use crate::views::map_controls::{self, FilterState};
use crate::views::{item_editor, pin_popover};
use crate::widgets::map::MapView;

/// Build the global map page.
pub fn build(controller: &Controller, sidebar_toggle: &gtk::ToggleButton) -> adw::NavigationPage {
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.pack_start(sidebar_toggle);

    let recenter = gtk::Button::from_icon_name("zoom-fit-best-symbolic");
    recenter.set_tooltip_text(Some("Recenter"));
    header.pack_start(&recenter);

    // Locate toggle (Swift focus cycle, 2-state on desktop): on -> centre on the user and
    // show the location dot; off -> hide it. Heading-follow needs a magnetometer, so it has
    // no desktop equivalent.
    let locate = gtk::ToggleButton::builder()
        .icon_name("find-location-symbolic")
        .tooltip_text("My Location")
        .build();
    header.pack_start(&locate);
    toolbar.add_top_bar(&header);

    let map = Rc::new(MapView::new());

    // Map / empty-state swap (Swift's `locationEntries.isEmpty` branch).
    let stack = gtk::Stack::new();
    stack.add_named(map.widget(), Some("map"));
    let empty = adw::StatusPage::builder()
        .icon_name("mark-location-symbolic")
        .title("No Locations")
        .description("Paste a Google Maps or Apple Maps link on an item to pin it here.")
        .build();
    stack.add_named(&empty, Some("empty"));
    toolbar.set_content(Some(&stack));

    let state = FilterState::new();
    let did_fit = Rc::new(Cell::new(false));

    let apply: Rc<dyn Fn()> = {
        let map = map.clone();
        let controller = controller.clone();
        let state = state.clone();
        let did_fit = did_fit.clone();
        let stack = stack.clone();
        Rc::new(move || {
            let all = models::global_pins(&controller);
            stack.set_visible_child_name(if all.is_empty() { "empty" } else { "map" });
            let pins = Rc::new(map_controls::filter_pins(all, &state));

            let on_show_details: Rc<dyn Fn(PinObject)> = {
                let controller = controller.clone();
                let map = map.clone();
                Rc::new(move |pin: PinObject| {
                    // Open the editor in the pin's source list (fresh per-list model).
                    let model = ListItemModel::new(controller.provider());
                    model.set_list_id(&pin.list_id());
                    item_editor::open(map.widget(), model, &pin.item_id());
                })
            };
            let on_select: Rc<dyn Fn(PinObject, gtk::Widget)> = {
                let map = map.clone();
                let pins = pins.clone();
                let on_show_details = on_show_details.clone();
                Rc::new(move |pin, anchor: gtk::Widget| {
                    pin_popover::present(map.clone(), pins.clone(), pin, anchor, on_show_details.clone());
                })
            };
            map.set_pins(pins.as_slice(), on_select);
            if !did_fit.get() {
                map.fit_to_pins(pins.as_slice());
                did_fit.set(true);
            }
        })
    };

    let refresh_menu = map_controls::attach(&header, &toolbar, &state, apply.clone());

    recenter.connect_clicked(glib::clone!(
        #[strong]
        map,
        #[weak]
        controller,
        #[strong]
        state,
        move |_| {
            let pins = map_controls::filter_pins(models::global_pins(&controller), &state);
            map.fit_to_pins(&pins);
        }
    ));

    locate.connect_toggled(glib::clone!(
        #[strong]
        map,
        move |btn| {
            if !btn.is_active() {
                map.set_user_location(None);
                return;
            }
            btn.set_sensitive(false);
            geo::current_location(glib::clone!(
                #[weak]
                btn,
                #[strong]
                map,
                move |coord| {
                    btn.set_sensitive(true);
                    match coord {
                        Some((lat, lon)) => {
                            map.set_user_location(Some((lat, lon)));
                            map.center_on(lat, lon, 12.0);
                        }
                        None => btn.set_active(false),
                    }
                }
            ));
        }
    ));

    let page = adw::NavigationPage::builder().title("Locations").child(&toolbar).build();
    page.connect_showing(glib::clone!(
        #[strong]
        apply,
        #[strong]
        refresh_menu,
        #[weak]
        controller,
        move |_| {
            refresh_menu(&models::global_pins(&controller));
            apply();
        }
    ));
    page
}
