//! `PinObject` — a row GObject for an item that carries a pinned location, used by both
//! the per-list map (`views/map_page.rs`) and the cross-list global map. Carries the
//! coordinate plus the display + navigation context the map markers and pin popover need.
//! GNOME counterpart of the Swift `MapListView` marker / `LocationEntry`.

use std::cell::{Cell, RefCell};

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use quite_listie_core::engine::unified_provider::UnifiedList;
use quite_listie_core::model::{ListItem, ListLabel};

mod imp {
    use super::*;

    #[derive(glib::Properties, Default)]
    #[properties(wrapper_type = super::PinObject)]
    pub struct PinObject {
        #[property(get, set)]
        pub item_id: RefCell<String>,
        #[property(get, set)]
        pub list_id: RefCell<String>,
        #[property(get, set)]
        pub list_name: RefCell<String>,
        #[property(get, set)]
        pub note: RefCell<String>,
        #[property(get, set)]
        pub latitude: Cell<f64>,
        #[property(get, set)]
        pub longitude: Cell<f64>,
        #[property(get, set)]
        pub checked: Cell<bool>,
        #[property(get, set)]
        pub source_url: RefCell<String>,
        #[property(get, set)]
        pub label_id: RefCell<String>,
        #[property(get, set)]
        pub label_name: RefCell<String>,
        /// Hex marker tint; falls back to the accent colour when empty.
        #[property(get, set)]
        pub label_color: RefCell<String>,
        #[property(get, set)]
        pub label_emoji: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PinObject {
        const NAME: &'static str = "QuiteListiePinObject";
        type Type = super::PinObject;
    }

    #[glib::derived_properties]
    impl ObjectImpl for PinObject {}
}

glib::wrapper! {
    pub struct PinObject(ObjectSubclass<imp::PinObject>);
}

impl PinObject {
    /// Build a pin from an item that has a location, resolving its label against `labels`
    /// and carrying `list`'s id/name for navigation back to the source list.
    pub fn from_item(item: &ListItem, labels: &[ListLabel], list: &UnifiedList) -> Option<Self> {
        let coord = item.location.as_ref()?;
        let label = item
            .label_id
            .as_deref()
            .and_then(|id| labels.iter().find(|l| l.id == id));
        Some(
            glib::Object::builder()
                .property("item-id", item.id.to_string())
                .property("list-id", &list.id)
                .property("list-name", &list.name)
                .property("note", &item.note)
                .property("latitude", coord.latitude)
                .property("longitude", coord.longitude)
                .property("checked", item.checked)
                .property("source-url", item.source_url.clone().unwrap_or_default())
                .property("label-id", item.label_id.clone().unwrap_or_default())
                .property("label-name", label.map(|l| l.name.clone()).unwrap_or_default())
                .property("label-color", label.map(|l| l.color.clone()).unwrap_or_default())
                .property("label-emoji", label.and_then(|l| l.emoji_icon.clone()).unwrap_or_default())
                .build(),
        )
    }
}
