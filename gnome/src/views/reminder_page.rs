//! `ReminderPage` — the cross-list reminder view reached from the **Today** /
//! **Scheduled** smart boxes. GNOME counterpart of Swift `ReminderListView`.
//!
//! Unchecked items that carry a reminder are gathered across every open list
//! ([`crate::models::reminder_entries`]), sorted by date, and rendered in date-bucketed
//! sections (Overdue / Today / Tomorrow / future dates). Each row mirrors the list page's
//! item row — a checkbox to complete the item, a reminder chip, plus chips naming the
//! parent list and the item's label — and a single click opens the same item editor.
//!
//! Mutations run through a per-list [`ListItemModel`] cache (the GNOME equivalent of
//! Swift's `viewModelCache`): each cached model's change callback re-gathers the entries,
//! so completing or editing an item refreshes the view and the smart-box counts.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::controller::Controller;
use crate::models::{self, ListItemModel, ReminderEntryObject};

/// Build the reminder page. `today_only` selects the Today bucket (overdue + today);
/// otherwise every scheduled reminder is shown.
pub fn build(
    controller: &Controller,
    today_only: bool,
    sidebar_toggle: &gtk::ToggleButton,
) -> adw::NavigationPage {
    let title = if today_only { "Today" } else { "Scheduled" };

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.pack_start(sidebar_toggle);
    toolbar.add_top_bar(&header);

    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .margin_top(12)
        .margin_bottom(18)
        .build();
    let clamp = adw::Clamp::builder().maximum_size(700).child(&body).build();
    let scroller = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&clamp)
        .build();
    toolbar.set_content(Some(&scroller));

    // Per-list models, created on demand and reused across re-gathers so a list's items
    // (and its change callback) are loaded once. Mirrors Swift's viewModelCache.
    let model_cache: Rc<RefCell<HashMap<String, Rc<ListItemModel>>>> =
        Rc::new(RefCell::new(HashMap::new()));

    let repopulate: Rc<dyn Fn()> = {
        let body = body.clone();
        let controller = controller.clone();
        let model_cache = model_cache.clone();
        // Late-bound self-reference so a model's change callback can trigger a re-gather.
        let slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let slot_outer = slot.clone();
        let f: Rc<dyn Fn()> = Rc::new(move || {
            while let Some(child) = body.first_child() {
                body.remove(&child);
            }

            let entries = models::reminder_entries(&controller, today_only);
            if entries.is_empty() {
                body.append(&empty_state(today_only));
                return;
            }

            let re_gather = slot.borrow().clone();
            let mut idx = 0;
            while idx < entries.len() {
                let key = entries[idx].group_key();
                let mut group: Vec<ReminderEntryObject> = Vec::new();
                while idx < entries.len() && entries[idx].group_key() == key {
                    group.push(entries[idx].clone());
                    idx += 1;
                }
                body.append(&build_group(&controller, &model_cache, &re_gather, &group));
            }
        });
        *slot_outer.borrow_mut() = Some(f.clone());
        f
    };
    repopulate();

    adw::NavigationPage::builder().title(title).child(&toolbar).build()
}

/// Build one date-bucket section: a coloured header plus a boxed list of rows.
fn build_group(
    controller: &Controller,
    model_cache: &Rc<RefCell<HashMap<String, Rc<ListItemModel>>>>,
    re_gather: &Option<Rc<dyn Fn()>>,
    group: &[ReminderEntryObject],
) -> gtk::Box {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 4);

    let first = &group[0];
    let header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_top(2)
        .margin_bottom(2)
        .build();
    let icon = gtk::Image::from_icon_name(group_icon(first.proximity()));
    apply_proximity_class(&icon, first.proximity());
    header.append(&icon);
    let title = gtk::Label::builder().label(&first.group_title()).xalign(0.0).hexpand(true).build();
    title.add_css_class("heading");
    apply_proximity_class(&title, first.proximity());
    header.append(&title);
    container.append(&header);

    let list_box = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
    list_box.add_css_class("boxed-list");
    for entry in group {
        list_box.append(&build_row(controller, model_cache, re_gather, entry));
    }
    container.append(&list_box);

    container
}

/// Get (or create) the cached [`ListItemModel`] for `list_id`, wiring its change callback
/// to re-gather the reminder list.
fn model_for(
    controller: &Controller,
    cache: &Rc<RefCell<HashMap<String, Rc<ListItemModel>>>>,
    re_gather: &Option<Rc<dyn Fn()>>,
    list_id: &str,
) -> Rc<ListItemModel> {
    if let Some(m) = cache.borrow().get(list_id) {
        return m.clone();
    }
    let model = ListItemModel::new(controller.provider());
    model.set_list_id(list_id);
    if let Some(cb) = re_gather.clone() {
        // Any mutation through this model re-gathers entries and refreshes the counts.
        let controller = controller.clone();
        model.set_on_changed(Rc::new(move || {
            cb();
            controller.refresh_counts();
        }));
    }
    cache.borrow_mut().insert(list_id.to_string(), model.clone());
    model
}

/// One reminder row: prefix checkbox completes the item; activating opens the editor;
/// suffix chips show the reminder, the parent list, and the item's label.
fn build_row(
    controller: &Controller,
    model_cache: &Rc<RefCell<HashMap<String, Rc<ListItemModel>>>>,
    re_gather: &Option<Rc<dyn Fn()>>,
    entry: &ReminderEntryObject,
) -> adw::ActionRow {
    let note = glib::markup_escape_text(&entry.note());
    let row = adw::ActionRow::builder().activatable(true).title(note.as_str()).build();

    // Checkbox → toggle completion in the item's own list.
    let check = gtk::CheckButton::builder().valign(gtk::Align::Center).build();
    check.connect_toggled(glib::clone!(
        #[weak]
        controller,
        #[strong]
        model_cache,
        #[strong]
        re_gather,
        #[strong(rename_to = list_id)]
        entry.list_id(),
        #[strong(rename_to = item_id)]
        entry.item_id(),
        move |_| {
            let model = model_for(&controller, &model_cache, &re_gather, &list_id);
            model.toggle_checked(&item_id);
        }
    ));
    row.add_prefix(&check);

    // Reminder chip (coloured by proximity).
    let chip = gtk::Label::new(Some(&entry.reminder_display()));
    chip.add_css_class("caption");
    chip.set_valign(gtk::Align::Center);
    apply_proximity_class(&chip, entry.proximity());
    row.add_suffix(&chip);

    // List chip — which list the item lives in (emoji-or-icon + name).
    let list_chip = gtk::Box::new(gtk::Orientation::Horizontal, 3);
    list_chip.set_valign(gtk::Align::Center);
    let emoji = entry.list_emoji();
    if emoji.is_empty() {
        list_chip.append(&gtk::Image::from_icon_name(&entry.list_icon()));
    } else {
        list_chip.append(&gtk::Label::new(Some(&emoji)));
    }
    let list_name = gtk::Label::new(Some(&entry.list_name()));
    list_name.add_css_class("caption");
    list_name.add_css_class("dim-label");
    list_chip.append(&list_name);
    row.add_suffix(&list_chip);

    // Label chip (coloured dot + name), if the item has a label.
    let label_name = entry.label_name();
    if !label_name.is_empty() {
        let label_chip = gtk::Label::new(None);
        label_chip.add_css_class("caption");
        label_chip.set_valign(gtk::Align::Center);
        let dot = if !entry.label_color().is_empty() {
            format!("<span foreground=\"{}\">●</span> ", sanitize_hex(&entry.label_color()))
        } else {
            String::new()
        };
        label_chip.set_markup(&format!("{dot}{}", glib::markup_escape_text(&label_name)));
        row.add_suffix(&label_chip);
    }

    // Single click opens the same editor the list page uses.
    row.connect_activated(glib::clone!(
        #[weak]
        controller,
        #[strong]
        model_cache,
        #[strong]
        re_gather,
        #[strong(rename_to = list_id)]
        entry.list_id(),
        #[strong(rename_to = item_id)]
        entry.item_id(),
        move |r| {
            let model = model_for(&controller, &model_cache, &re_gather, &list_id);
            crate::views::item_editor::open(r, model, &item_id);
        }
    ));

    row
}

fn empty_state(today_only: bool) -> adw::StatusPage {
    adw::StatusPage::builder()
        .icon_name(if today_only { "alarm-symbolic" } else { "month-symbolic" })
        .title(if today_only { "No Reminders Today" } else { "No Scheduled Reminders" })
        .description("Items with reminders will appear here")
        .vexpand(true)
        .build()
}

fn group_icon(proximity: i32) -> &'static str {
    match proximity {
        0 => "alarm-symbolic",
        _ => "month-symbolic",
    }
}

fn apply_proximity_class(widget: &impl IsA<gtk::Widget>, proximity: i32) {
    for c in ["ql-overdue", "ql-today", "ql-tomorrow", "ql-future"] {
        widget.remove_css_class(c);
    }
    widget.add_css_class(match proximity {
        0 => "ql-overdue",
        1 => "ql-today",
        2 => "ql-tomorrow",
        _ => "ql-future",
    });
}

/// Only allow a `#rrggbb`/`#rgb` hex value through to Pango markup.
fn sanitize_hex(hex: &str) -> String {
    if hex.starts_with('#') && hex.len() <= 9 && hex[1..].chars().all(|c| c.is_ascii_hexdigit()) {
        hex.to_string()
    } else {
        "gray".to_string()
    }
}
