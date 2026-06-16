//! `ReminderEntryObject` — a row GObject for the cross-list reminder view
//! (`views/reminder_page.rs`). Pairs an unchecked item that has a reminder with its
//! parent-list context (name/icon) and label metadata, plus the precomputed date-group
//! the row belongs to. GNOME counterpart of the Swift `ReminderEntry` struct.

use std::cell::{Cell, RefCell};

use chrono::{Days, Local, NaiveDate};
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use quite_listie_core::engine::unified_provider::UnifiedList;
use quite_listie_core::model::{ListItem, ListLabel};

mod imp {
    use super::*;

    #[derive(glib::Properties, Default)]
    #[properties(wrapper_type = super::ReminderEntryObject)]
    pub struct ReminderEntryObject {
        #[property(get, set)]
        pub item_id: RefCell<String>,
        #[property(get, set)]
        pub list_id: RefCell<String>,
        #[property(get, set)]
        pub list_name: RefCell<String>,
        #[property(get, set)]
        pub list_emoji: RefCell<String>,
        #[property(get, set)]
        pub list_icon: RefCell<String>,
        #[property(get, set)]
        pub note: RefCell<String>,
        #[property(get, set)]
        pub quantity_display: RefCell<String>,
        #[property(get, set)]
        pub checked: Cell<bool>,
        #[property(get, set)]
        pub label_name: RefCell<String>,
        #[property(get, set)]
        pub label_color: RefCell<String>,
        #[property(get, set)]
        pub label_emoji: RefCell<String>,
        #[property(get, set)]
        pub reminder_display: RefCell<String>,
        #[property(get, set)]
        pub reminder_is_repeating: Cell<bool>,
        /// 0=overdue 1=today 2=tomorrow 3=future — drives the chip + group colour.
        #[property(get, set)]
        pub proximity: Cell<i32>,
        /// Stable key used to group consecutive rows (the sorted list is already in
        /// group order, so equal keys are contiguous).
        #[property(get, set)]
        pub group_key: RefCell<String>,
        #[property(get, set)]
        pub group_title: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ReminderEntryObject {
        const NAME: &'static str = "QuiteListieReminderEntryObject";
        type Type = super::ReminderEntryObject;
    }

    #[glib::derived_properties]
    impl ObjectImpl for ReminderEntryObject {}
}

glib::wrapper! {
    pub struct ReminderEntryObject(ObjectSubclass<imp::ReminderEntryObject>);
}

impl ReminderEntryObject {
    /// Build an entry from an item that has a reminder, resolving its label against
    /// `labels` and carrying `list`'s name/icon for the row's list chip. `today` is the
    /// local date used to bucket the reminder into overdue/today/tomorrow/future.
    pub fn from_item(item: &ListItem, labels: &[ListLabel], list: &UnifiedList, today: NaiveDate) -> Self {
        let label = item
            .label_id
            .as_deref()
            .and_then(|id| labels.iter().find(|l| l.id == id));

        let date = item.reminder_date.expect("from_item requires a reminder_date");
        let local_d: chrono::DateTime<Local> = date.into();
        let day = local_d.date_naive();
        let time_str = local_d.format("%-I:%M %p").to_string();

        let (proximity, group_key, group_title) = if day < today {
            (0, "overdue".to_string(), "Overdue".to_string())
        } else if day == today {
            (1, "today".to_string(), "Today".to_string())
        } else if day == today + Days::new(1) {
            (2, "tomorrow".to_string(), "Tomorrow".to_string())
        } else {
            (3, day.to_string(), local_d.format("%b %-d, %Y").to_string())
        };

        // Row chip text: the day bucket plus the time (overdue keeps the date).
        let reminder_display = match proximity {
            0 => format!("Overdue {time_str}"),
            1 => format!("Today {time_str}"),
            2 => format!("Tomorrow {time_str}"),
            _ => format!("{} {time_str}", local_d.format("%b %-d")),
        };

        glib::Object::builder()
            .property("item-id", item.id.to_string())
            .property("list-id", &list.id)
            .property("list-name", &list.name)
            .property("list-emoji", list.emoji_icon.clone().unwrap_or_default())
            .property("list-icon", list.icon.clone().unwrap_or_else(|| "view-list-symbolic".into()))
            .property("note", &item.note)
            .property("quantity-display", item.display_quantity().unwrap_or_default())
            .property("checked", item.checked)
            .property("label-name", label.map(|l| l.name.clone()).unwrap_or_default())
            .property("label-color", label.map(|l| l.color.clone()).unwrap_or_default())
            .property("label-emoji", label.and_then(|l| l.emoji_icon.clone()).unwrap_or_default())
            .property("reminder-display", reminder_display)
            .property("reminder-is-repeating", item.reminder_repeat_rule.is_some())
            .property("proximity", proximity)
            .property("group-key", group_key)
            .property("group-title", group_title)
            .build()
    }
}
