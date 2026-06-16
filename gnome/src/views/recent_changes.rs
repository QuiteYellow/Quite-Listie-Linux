//! `RecentChanges` — a per-list change history. GNOME counterpart of Swift
//! `RecentChangesView`: a "Sync Activity" header (source + state) followed by the 30 most
//! recently changed items, each with a kind icon, a change description + relative time, and
//! an Undo action for checked/deleted changes. Clicking a (non-deleted) row opens the item
//! editor. Backed by the controller's `recent_changes` / `undo_change` / `list_sync_state`.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::controller::{Controller, RecentChangeRow};
use crate::models::ListItemModel;

pub fn present(anchor: &impl IsA<gtk::Widget>, controller: &Controller, list_id: &str) {
    let dialog = adw::Dialog::builder()
        .title("Recent Changes")
        .content_width(480)
        .content_height(580)
        .build();

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());

    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_start(12)
        .margin_end(12)
        .margin_top(12)
        .margin_bottom(12)
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&body)
        .build();
    toolbar.set_content(Some(&scroller));

    // One model for this list; edits through it (and undo's rebuild) refresh the view.
    let model = ListItemModel::new(controller.provider());
    model.set_list_id(list_id);

    let rebuild: Rc<dyn Fn()> = {
        let body = body.clone();
        let controller = controller.clone();
        let list_id = list_id.to_string();
        let model = model.clone();
        Rc::new(move || {
            while let Some(child) = body.first_child() {
                body.remove(&child);
            }

            // --- Sync Activity ---------------------------------------------
            let sync_box = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
            sync_box.add_css_class("boxed-list");
            sync_box.append(&info_row("Source", &controller.list_source_description(&list_id)));
            sync_box.append(&info_row("State", &controller.list_sync_state(&list_id)));
            let sync_label = section_label("Sync Activity");
            body.append(&sync_label);
            body.append(&sync_box);

            // --- Changes ---------------------------------------------------
            let rows = controller.recent_changes(&list_id);
            if rows.is_empty() {
                let status = adw::StatusPage::builder()
                    .icon_name("document-open-recent-symbolic")
                    .title("No Recent Changes")
                    .description("Changes to items will appear here.")
                    .vexpand(true)
                    .build();
                body.append(&status);
                return;
            }
            body.append(&section_label("Changes"));
            let list_box = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
            list_box.add_css_class("boxed-list");
            for row in &rows {
                list_box.append(&build_row(&controller, &list_id, &model, row));
            }
            body.append(&list_box);
        })
    };
    rebuild();
    // Edits made through the editor re-render the history.
    model.set_on_changed(rebuild.clone());

    dialog.set_child(Some(&toolbar));
    dialog.present(Some(anchor));
}

fn section_label(text: &str) -> gtk::Label {
    let l = gtk::Label::builder().label(text).xalign(0.0).build();
    l.add_css_class("heading");
    l
}

fn info_row(title: &str, value: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(title).build();
    let value = gtk::Label::new(Some(value));
    value.add_css_class("dim-label");
    value.set_valign(gtk::Align::Center);
    value.set_selectable(true);
    row.add_suffix(&value);
    row
}

fn build_row(
    controller: &Controller,
    list_id: &str,
    model: &Rc<ListItemModel>,
    row: &RecentChangeRow,
) -> adw::ActionRow {
    let subtitle = format!("{} · {}", change_description(&row.change, row.checked), relative_time(row.modified_ts));
    let action_row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&row.note))
        .subtitle(&subtitle)
        .activatable(!row.is_deleted)
        .build();

    let icon = gtk::Image::from_icon_name(icon_name(&row.change, row.checked));
    if let Some(class) = icon_color_class(&row.change, row.checked) {
        icon.add_css_class(class);
    }
    action_row.add_prefix(&icon);

    if can_undo(&row.change, row.is_deleted) {
        let undo = gtk::Button::builder().label("Undo").valign(gtk::Align::Center).build();
        undo.add_css_class("flat");
        undo.connect_clicked(glib::clone!(
            #[weak]
            controller,
            #[strong(rename_to = list_id)]
            list_id.to_string(),
            #[strong(rename_to = item_id)]
            row.id.clone(),
            move |_| controller.undo_change(&list_id, &item_id)
        ));
        action_row.add_suffix(&undo);
    }

    if !row.is_deleted {
        action_row.connect_activated(glib::clone!(
            #[strong]
            model,
            #[strong(rename_to = item_id)]
            row.id.clone(),
            move |r| crate::views::item_editor::open(r, model.clone(), &item_id)
        ));
    }
    action_row
}

fn can_undo(change: &str, is_deleted: bool) -> bool {
    match change {
        "checked" => !is_deleted,
        "deleted" => true,
        _ => false,
    }
}

fn icon_name(change: &str, checked: bool) -> &'static str {
    match change {
        "added" => "list-add-symbolic",
        "checked" => {
            if checked {
                "object-select-symbolic"
            } else {
                "edit-undo-symbolic"
            }
        }
        "note" => "document-edit-symbolic",
        "quantity" => "view-list-symbolic",
        "label" => "view-list-bullet-symbolic",
        "reminder" => "alarm-symbolic",
        "location" => "mark-location-symbolic",
        "subitems" => "view-list-symbolic",
        "deleted" => "user-trash-symbolic",
        "restored" => "edit-undo-symbolic",
        _ => "dialog-question-symbolic",
    }
}

fn icon_color_class(change: &str, checked: bool) -> Option<&'static str> {
    match change {
        "added" => Some("success"),
        "checked" => Some(if checked { "success" } else { "warning" }),
        "deleted" => Some("error"),
        "restored" => Some("accent"),
        _ => None,
    }
}

fn change_description(change: &str, checked: bool) -> &'static str {
    match change {
        "added" => "Added",
        "checked" => {
            if checked {
                "Checked off"
            } else {
                "Unchecked"
            }
        }
        "note" => "Name updated",
        "quantity" => "Quantity changed",
        "label" => "Category changed",
        "reminder" => "Reminder updated",
        "location" => "Location updated",
        "subitems" => "Sub-items updated",
        "deleted" => "Deleted",
        "restored" => "Restored",
        _ => "Modified",
    }
}

/// A short relative time from a unix timestamp, e.g. "5m ago", "2h ago", "3d ago".
fn relative_time(ts: i64) -> String {
    let now = chrono::Utc::now().timestamp();
    let secs = (now - ts).max(0);
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else if secs < 604_800 {
        format!("{}d ago", secs / 86_400)
    } else {
        format!("{}w ago", secs / 604_800)
    }
}
