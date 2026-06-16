//! Shared map-page header controls: a search toggle + revealing search bar and a filter
//! menu (Show Completed + per-label toggles + Clear All Filters). GNOME counterpart of the
//! Swift `MapListView` search-text filter and `GlobalMapView.filterMenu`. Used by both the
//! per-list map (`map_page`) and the cross-list map (`global_map`).

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::models::PinObject;

/// The live filter state the map page reads when projecting its pins.
#[derive(Clone)]
pub struct FilterState {
    pub show_completed: Rc<Cell<bool>>,
    pub selected_labels: Rc<RefCell<HashSet<String>>>,
    pub search_text: Rc<RefCell<String>>,
}

impl FilterState {
    pub fn new() -> Self {
        Self {
            show_completed: Rc::new(Cell::new(false)),
            selected_labels: Rc::new(RefCell::new(HashSet::new())),
            search_text: Rc::new(RefCell::new(String::new())),
        }
    }

    fn has_active_filters(&self) -> bool {
        self.show_completed.get() || !self.selected_labels.borrow().is_empty()
    }
}

/// Apply the live filter state to a page's located pins (Swift `visibleItems`): drop
/// completed (unless shown), then the search-note and selected-label filters.
pub fn filter_pins(mut pins: Vec<PinObject>, state: &FilterState) -> Vec<PinObject> {
    if !state.show_completed.get() {
        pins.retain(|p| !p.checked());
    }
    let query = state.search_text.borrow().to_lowercase();
    if !query.is_empty() {
        pins.retain(|p| p.note().to_lowercase().contains(&query));
    }
    let selected = state.selected_labels.borrow();
    if !selected.is_empty() {
        pins.retain(|p| selected.contains(&p.label_id()));
    }
    pins
}

fn set_indicator(btn: &gtk::MenuButton, active: bool) {
    if active {
        btn.add_css_class("accent");
    } else {
        btn.remove_css_class("accent");
    }
}

/// Add the search bar + filter menu to `header`/`toolbar`. `apply` re-projects the page's
/// pins from the current filter state (called on any control change). Returns a closure
/// that rebuilds the filter menu's label list from the page's unfiltered pins; call it from
/// the page's refresh so newly-used labels appear.
pub fn attach(
    header: &adw::HeaderBar,
    toolbar: &adw::ToolbarView,
    state: &FilterState,
    apply: Rc<dyn Fn()>,
) -> Rc<dyn Fn(&[PinObject])> {
    let state = state.clone();

    // --- search (Swift `searchText` note filter) ---------------------------
    let search_btn = gtk::ToggleButton::builder()
        .icon_name("system-search-symbolic")
        .tooltip_text("Search")
        .build();
    header.pack_start(&search_btn);
    let search_entry = gtk::SearchEntry::builder().placeholder_text("Search items").build();
    let search_bar = gtk::SearchBar::builder().child(&search_entry).build();
    search_bar.connect_entry(&search_entry);
    search_btn
        .bind_property("active", &search_bar, "search-mode-enabled")
        .sync_create()
        .bidirectional()
        .build();
    toolbar.add_top_bar(&search_bar);
    search_entry.connect_search_changed(glib::clone!(
        #[strong(rename_to = search_text)]
        state.search_text,
        #[strong]
        apply,
        move |e| {
            *search_text.borrow_mut() = e.text().trim().to_string();
            apply();
        }
    ));
    search_btn.connect_toggled(glib::clone!(
        #[weak]
        search_entry,
        move |btn| {
            if !btn.is_active() {
                search_entry.set_text("");
            }
        }
    ));

    // --- filter menu (Swift `filterMenu`) ----------------------------------
    let filter_btn = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .tooltip_text("Filter")
        .build();
    let popover = gtk::Popover::new();
    let menu_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(8)
        .margin_end(8)
        .build();
    popover.set_child(Some(&menu_box));
    filter_btn.set_popover(Some(&popover));
    header.pack_end(&filter_btn);

    // Rebuilds the menu from the current unfiltered pins. Held in a slot so "Clear All
    // Filters" can re-run it after resetting the state.
    let last_pins: Rc<RefCell<Vec<PinObject>>> = Rc::new(RefCell::new(Vec::new()));
    let slot: Rc<RefCell<Option<Rc<dyn Fn(&[PinObject])>>>> = Rc::new(RefCell::new(None));

    let refresh: Rc<dyn Fn(&[PinObject])> = Rc::new(glib::clone!(
        #[strong]
        menu_box,
        #[weak]
        filter_btn,
        #[strong]
        apply,
        #[strong]
        last_pins,
        #[strong]
        slot,
        #[strong]
        state,
        move |all: &[PinObject]| {
            *last_pins.borrow_mut() = all.to_vec();
            set_indicator(&filter_btn, state.has_active_filters());

            while let Some(child) = menu_box.first_child() {
                menu_box.remove(&child);
            }

            let completed = gtk::CheckButton::with_label("Show Completed");
            completed.set_active(state.show_completed.get());
            completed.connect_toggled(glib::clone!(
                #[weak]
                filter_btn,
                #[strong]
                apply,
                #[strong]
                state,
                move |c| {
                    state.show_completed.set(c.is_active());
                    set_indicator(&filter_btn, state.has_active_filters());
                    apply();
                }
            ));
            menu_box.append(&completed);

            // Distinct labels actually used by the pins, sorted by name (Swift labelsWithItems).
            let mut seen = HashSet::new();
            let mut labels: Vec<(String, String)> = Vec::new();
            for p in all {
                let id = p.label_id();
                if !id.is_empty() && seen.insert(id.clone()) {
                    labels.push((id, p.label_name()));
                }
            }
            labels.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));

            if !labels.is_empty() {
                menu_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
                for (id, name) in labels {
                    let check = gtk::CheckButton::with_label(&name);
                    check.set_active(state.selected_labels.borrow().contains(&id));
                    check.connect_toggled(glib::clone!(
                        #[weak]
                        filter_btn,
                        #[strong]
                        apply,
                        #[strong]
                        state,
                        #[strong]
                        id,
                        move |c| {
                            if c.is_active() {
                                state.selected_labels.borrow_mut().insert(id.clone());
                            } else {
                                state.selected_labels.borrow_mut().remove(&id);
                            }
                            set_indicator(&filter_btn, state.has_active_filters());
                            apply();
                        }
                    ));
                    menu_box.append(&check);
                }
            }

            if state.has_active_filters() {
                menu_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
                let clear = gtk::Button::with_label("Clear All Filters");
                clear.add_css_class("destructive-action");
                clear.connect_clicked(glib::clone!(
                    #[strong]
                    apply,
                    #[strong]
                    last_pins,
                    #[strong]
                    slot,
                    #[strong]
                    state,
                    move |_| {
                        state.show_completed.set(false);
                        state.selected_labels.borrow_mut().clear();
                        let snap = last_pins.borrow().clone();
                        if let Some(rebuild) = slot.borrow().clone() {
                            rebuild(&snap);
                        }
                        apply();
                    }
                ));
                menu_box.append(&clear);
            }
        }
    ));
    *slot.borrow_mut() = Some(refresh.clone());
    refresh
}
