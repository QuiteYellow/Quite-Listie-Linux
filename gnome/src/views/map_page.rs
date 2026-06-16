//! `MapPage` — the per-list pinned-locations map (Swift `MapListView`, reached from the
//! list page's "Pinned Locations" banner). Shows the open list's located items as
//! label-tinted markers; tapping a marker opens the shared pin popover, whose "Show
//! Details" pushes the item editor. A search bar + filter menu (Show Completed, per-label
//! toggles, Clear All Filters) mirror Swift's filters; a right-click on the map adds an
//! item at that point (Swift's long-press add). The map re-gathers its pins whenever the
//! page becomes visible again (e.g. after the editor pops), the GNOME stand-in for Swift's
//! onDismiss.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use quite_listie_core::model::Coordinate;

use crate::controller::Controller;
use crate::geo;
use crate::models::{self, ListItemModel, PinObject};
use crate::views::map_controls::{self, FilterState};
use crate::views::{item_editor, pin_popover};
use crate::widgets::map::MapView;

/// Build the map page for `list_id`.
pub fn build(
    controller: &Controller,
    list_id: &str,
    title: &str,
    sidebar_toggle: Option<gtk::ToggleButton>,
) -> adw::NavigationPage {
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    if let Some(toggle) = &sidebar_toggle {
        header.pack_start(toggle);
    }

    let recenter = gtk::Button::from_icon_name("zoom-fit-best-symbolic");
    recenter.set_tooltip_text(Some("Recenter"));
    header.pack_start(&recenter);

    // Locate toggle (Swift focus cycle, 2-state on desktop): on -> centre on the user and
    // show the location dot; off -> hide it.
    let locate = gtk::ToggleButton::builder()
        .icon_name("find-location-symbolic")
        .tooltip_text("My Location")
        .build();
    header.pack_start(&locate);
    toolbar.add_top_bar(&header);

    let map = Rc::new(MapView::new());

    // Map / empty-state swap (Swift's `allLocationItems.isEmpty` branch).
    let stack = gtk::Stack::new();
    stack.add_named(map.widget(), Some("map"));
    let empty = adw::StatusPage::builder()
        .icon_name("mark-location-symbolic")
        .title("No Locations")
        .description("No items are pinned yet. Open an item and tap Location to get started.")
        .build();
    stack.add_named(&empty, Some("empty"));
    toolbar.set_content(Some(&stack));

    // A model for the list, used to open the editor and add items at a point. Kept alive by
    // the closures held by the page's `showing` handler for the page's lifetime.
    let model = ListItemModel::new(controller.provider());
    model.set_list_id(list_id);

    let state = FilterState::new();
    let did_fit = Rc::new(Cell::new(false));

    // Re-project the markers + empty state from the current filter state.
    let apply: Rc<dyn Fn()> = {
        let map = map.clone();
        let controller = controller.clone();
        let model = model.clone();
        let list_id = list_id.to_string();
        let state = state.clone();
        let did_fit = did_fit.clone();
        let stack = stack.clone();
        Rc::new(move || {
            let all = models::list_pins(&controller, &list_id);
            stack.set_visible_child_name(if all.is_empty() { "empty" } else { "map" });
            let pins = Rc::new(map_controls::filter_pins(all, &state));

            let on_show_details: Rc<dyn Fn(PinObject)> = {
                let model = model.clone();
                let map = map.clone();
                Rc::new(move |pin: PinObject| {
                    item_editor::open(map.widget(), model.clone(), &pin.item_id());
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
            // Fit once on first show; later refreshes keep the user's camera.
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
        #[strong(rename_to = list_id)]
        list_id.to_string(),
        move |_| {
            let pins = map_controls::filter_pins(models::list_pins(&controller, &list_id), &state);
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

    // Right-click to add an item at that point (Swift's long-press `onAddAtLocation`).
    let click = gtk::GestureClick::new();
    click.set_button(gtk::gdk::BUTTON_SECONDARY);
    click.connect_pressed(glib::clone!(
        #[strong]
        map,
        #[strong]
        model,
        move |_, _, x, y| {
            let (lat, lon) = map.coord_at_widget(x, y);
            let popover = gtk::Popover::new();
            popover.set_parent(map.widget());
            popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            popover.connect_closed(|p| p.unparent());
            let add = gtk::Button::builder().label("Add item here").has_frame(false).build();
            add.connect_clicked(glib::clone!(
                #[weak]
                popover,
                #[strong]
                map,
                #[strong]
                model,
                move |_| {
                    popover.popdown();
                    item_editor::open_new_at_location(
                        map.widget(),
                        model.clone(),
                        Coordinate { latitude: lat, longitude: lon, extra: Default::default() },
                    );
                }
            ));
            popover.set_child(Some(&add));
            popover.popup();
        }
    ));
    map.widget().add_controller(click);

    let page = adw::NavigationPage::builder().title(title).child(&toolbar).build();
    // Refresh on every show: initial display, and after the editor pops back.
    page.connect_showing(glib::clone!(
        #[strong]
        apply,
        #[strong]
        refresh_menu,
        #[weak]
        controller,
        #[strong(rename_to = list_id)]
        list_id.to_string(),
        move |_| {
            refresh_menu(&models::list_pins(&controller, &list_id));
            apply();
        }
    ));
    page
}
