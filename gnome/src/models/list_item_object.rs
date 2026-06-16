//! `ListItemObject` — a row GObject for the list-item `gio::ListStore`. Carries the
//! flattened, display-ready fields the KDE `list_item_model.rs` exposed as roles
//! (`note`, `checked`, `labelColor`, `reminderProximity`, `sectionName`, …).

use std::cell::{Cell, RefCell};

use chrono::Utc;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use quite_listie_core::model::{ListItem, ListLabel};

mod imp {
    use super::*;

    #[derive(glib::Properties, Default)]
    #[properties(wrapper_type = super::ListItemObject)]
    pub struct ListItemObject {
        #[property(get, set)]
        pub item_id: RefCell<String>,
        #[property(get, set)]
        pub note: RefCell<String>,
        #[property(get, set)]
        pub quantity: Cell<f64>,
        #[property(get, set)]
        pub quantity_display: RefCell<String>,
        #[property(get, set)]
        pub checked: Cell<bool>,
        #[property(get, set)]
        pub label_id: RefCell<String>,
        #[property(get, set)]
        pub label_color: RefCell<String>,
        #[property(get, set)]
        pub label_name: RefCell<String>,
        #[property(get, set)]
        pub label_emoji: RefCell<String>,
        #[property(get, set)]
        pub has_reminder: Cell<bool>,
        #[property(get, set)]
        pub reminder_display: RefCell<String>,
        /// 0=overdue 1=today 2=tomorrow 3=future -1=none
        #[property(get, set)]
        pub reminder_proximity: Cell<i32>,
        #[property(get, set)]
        pub reminder_is_repeating: Cell<bool>,
        #[property(get, set)]
        pub has_location: Cell<bool>,
        #[property(get, set)]
        pub markdown_notes: RefCell<String>,
        /// Section grouping key (label name, or "Completed").
        #[property(get, set)]
        pub section_name: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ListItemObject {
        const NAME: &'static str = "QuiteListieListItemObject";
        type Type = super::ListItemObject;
    }

    #[glib::derived_properties]
    impl ObjectImpl for ListItemObject {}
}

glib::wrapper! {
    pub struct ListItemObject(ObjectSubclass<imp::ListItemObject>);
}

impl ListItemObject {
    /// Build a row object from a `ListItem`, resolving label fields against `labels`
    /// and computing reminder proximity/display — mirrors `list_item_model::data`.
    pub fn from_item(item: &ListItem, labels: &[ListLabel], show_checked_at_bottom: bool) -> Self {
        let label = item
            .label_id
            .as_deref()
            .and_then(|id| labels.iter().find(|l| l.id == id));

        let (proximity, display) = match item.reminder_date {
            None => (-1, String::new()),
            Some(d) => {
                let now = Utc::now();
                let today = now.date_naive();
                let date = d.date_naive();
                let local_d: chrono::DateTime<chrono::Local> = d.into();
                let time_str = local_d.format("%-I:%M %p").to_string();
                // Overdue is an instant comparison (Swift `reminderDate < now`), so a
                // reminder earlier today reads Overdue rather than "Today HH:MM".
                if d < now {
                    (0, "Overdue".to_string())
                } else if date == today {
                    (1, format!("Today {time_str}"))
                } else if date == today + chrono::Days::new(1) {
                    (2, format!("Tomorrow {time_str}"))
                } else {
                    let days = (date - today).num_days();
                    (3, format!("In {} day{}", days, if days == 1 { "" } else { "s" }))
                }
            }
        };

        let section_name = if show_checked_at_bottom && item.checked {
            "Completed".to_string()
        } else {
            label.map(|l| l.name.clone()).unwrap_or_default()
        };

        glib::Object::builder()
            .property("item-id", item.id.to_string())
            .property("note", &item.note)
            .property("quantity", item.quantity)
            .property("quantity-display", item.display_quantity().unwrap_or_default())
            .property("checked", item.checked)
            .property("label-id", item.label_id.clone().unwrap_or_default())
            .property("label-color", label.map(|l| l.color.clone()).unwrap_or_default())
            .property("label-name", label.map(|l| l.name.clone()).unwrap_or_default())
            .property(
                "label-emoji",
                label.and_then(|l| l.emoji_icon.clone()).unwrap_or_default(),
            )
            .property("has-reminder", item.has_reminder())
            .property("reminder-display", display)
            .property("reminder-proximity", proximity)
            .property("reminder-is-repeating", item.reminder_repeat_rule.is_some())
            .property("has-location", item.has_location())
            .property("markdown-notes", item.markdown_notes.clone().unwrap_or_default())
            .property("section-name", section_name)
            .build()
    }
}
