//! `RecycleBin` — restore or permanently delete a list's soft-deleted items. GNOME
//! counterpart of Swift `RecycleBinView`: a dialog listing deleted items with a
//! days-since / auto-deletes-in-30-days countdown, per-row Restore and Delete-Forever, and
//! Restore-All / Delete-All header actions. Backed by the controller's recycle-bin methods.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::controller::{Controller, DeletedItemRow};

/// Items are purged automatically after this many days (matches Swift).
const RETENTION_DAYS: i64 = 30;

pub fn present(anchor: &impl IsA<gtk::Widget>, controller: &Controller, list_id: &str) {
    let dialog = adw::Dialog::builder()
        .title("Recycle Bin")
        .content_width(480)
        .content_height(560)
        .build();

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    let restore_all = gtk::Button::builder().label("Restore All").build();
    let delete_all = gtk::Button::builder().label("Delete All").build();
    delete_all.add_css_class("destructive-action");
    header.pack_end(&delete_all);
    header.pack_end(&restore_all);
    toolbar.add_top_bar(&header);

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

    // `rebuild` re-fetches the deleted items and rebuilds the list. Defined via a RefCell
    // cell so the per-row handlers it creates can themselves call back into it.
    let rebuild_cell: Rc<std::cell::RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(std::cell::RefCell::new(None));
    let rebuild: Rc<dyn Fn()> = {
        let body = body.clone();
        let controller = controller.clone();
        let list_id = list_id.to_string();
        let restore_all = restore_all.clone();
        let delete_all = delete_all.clone();
        let rebuild_cell = rebuild_cell.clone();
        Rc::new(move || {
            while let Some(child) = body.first_child() {
                body.remove(&child);
            }
            let rows = controller.deleted_items(&list_id);
            restore_all.set_visible(!rows.is_empty());
            delete_all.set_visible(!rows.is_empty());

            if rows.is_empty() {
                let status = adw::StatusPage::builder()
                    .icon_name("user-trash-symbolic")
                    .title("Recycle Bin Empty")
                    .description("Deleted items appear here and are removed automatically after 30 days.")
                    .vexpand(true)
                    .build();
                body.append(&status);
                return;
            }

            let caption = gtk::Label::builder()
                .label("Items are automatically deleted after 30 days")
                .xalign(0.0)
                .wrap(true)
                .build();
            caption.add_css_class("dim-label");
            caption.add_css_class("caption");
            body.append(&caption);

            let again = rebuild_cell.borrow().clone().expect("rebuild set");
            let list_box = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
            list_box.add_css_class("boxed-list");
            for row in &rows {
                list_box.append(&build_row(&controller, &list_id, row, &again));
            }
            body.append(&list_box);
        })
    };
    *rebuild_cell.borrow_mut() = Some(rebuild.clone());
    rebuild();

    restore_all.connect_clicked(glib::clone!(
        #[weak]
        controller,
        #[strong]
        rebuild,
        #[strong(rename_to = list_id)]
        list_id.to_string(),
        move |btn| {
            confirm(
                btn,
                "Restore All Items?",
                "All deleted items will be restored to the list.",
                "Restore All",
                false,
                glib::clone!(
                    #[weak]
                    controller,
                    #[strong]
                    rebuild,
                    #[strong]
                    list_id,
                    move || {
                        controller.restore_all_deleted(&list_id);
                        rebuild();
                    }
                ),
            );
        }
    ));
    delete_all.connect_clicked(glib::clone!(
        #[weak]
        controller,
        #[strong]
        rebuild,
        #[strong(rename_to = list_id)]
        list_id.to_string(),
        move |btn| {
            confirm(
                btn,
                "Delete All Items Forever?",
                "All items will be permanently deleted and cannot be recovered.",
                "Delete All",
                true,
                glib::clone!(
                    #[weak]
                    controller,
                    #[strong]
                    rebuild,
                    #[strong]
                    list_id,
                    move || {
                        controller.purge_all_deleted(&list_id);
                        rebuild();
                    }
                ),
            );
        }
    ));

    dialog.set_child(Some(&toolbar));
    dialog.present(Some(anchor));
}

fn build_row(
    controller: &Controller,
    list_id: &str,
    row: &DeletedItemRow,
    rebuild: &Rc<dyn Fn()>,
) -> adw::ActionRow {
    let days_remaining = RETENTION_DAYS - row.days_ago;
    let subtitle = if days_remaining > 0 {
        format!(
            "Deleted {} day{} ago • Auto-deletes in {} day{}",
            row.days_ago,
            plural(row.days_ago),
            days_remaining,
            plural(days_remaining)
        )
    } else {
        format!("Deleted {} day{} ago • Will be auto-deleted soon", row.days_ago, plural(row.days_ago))
    };

    let action_row = adw::ActionRow::builder().title(glib::markup_escape_text(&row.note)).subtitle(&subtitle).build();
    if days_remaining <= 0 {
        action_row.add_css_class("error");
    } else if days_remaining <= 7 {
        action_row.add_css_class("warning");
    }

    let restore = gtk::Button::builder()
        .icon_name("edit-undo-symbolic")
        .tooltip_text("Restore")
        .valign(gtk::Align::Center)
        .build();
    restore.add_css_class("flat");
    let purge = gtk::Button::builder()
        .icon_name("user-trash-full-symbolic")
        .tooltip_text("Delete Forever")
        .valign(gtk::Align::Center)
        .build();
    purge.add_css_class("flat");
    purge.add_css_class("error");

    restore.connect_clicked(glib::clone!(
        #[weak]
        controller,
        #[strong(rename_to = list_id)]
        list_id.to_string(),
        #[strong(rename_to = item_id)]
        row.id.clone(),
        #[strong]
        rebuild,
        move |_| {
            controller.restore_item(&list_id, &item_id);
            rebuild();
        }
    ));
    purge.connect_clicked(glib::clone!(
        #[weak]
        controller,
        #[strong(rename_to = list_id)]
        list_id.to_string(),
        #[strong(rename_to = item_id)]
        row.id.clone(),
        #[strong]
        rebuild,
        move |btn| {
            confirm(
                btn,
                "Delete Forever?",
                "This item will be permanently deleted and cannot be recovered.",
                "Delete",
                true,
                glib::clone!(
                    #[weak]
                    controller,
                    #[strong]
                    list_id,
                    #[strong]
                    item_id,
                    #[strong]
                    rebuild,
                    move || {
                        controller.permanently_delete_item(&list_id, &item_id);
                        rebuild();
                    }
                ),
            );
        }
    ));

    action_row.add_suffix(&restore);
    action_row.add_suffix(&purge);
    action_row
}

fn plural(n: i64) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

fn confirm<F: Fn() + 'static>(
    anchor: &impl IsA<gtk::Widget>,
    heading: &str,
    msg: &str,
    confirm_label: &str,
    destructive: bool,
    on_confirm: F,
) {
    let dialog = adw::AlertDialog::new(Some(heading), Some(msg));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("ok", confirm_label);
    dialog.set_response_appearance(
        "ok",
        if destructive {
            adw::ResponseAppearance::Destructive
        } else {
            adw::ResponseAppearance::Suggested
        },
    );
    dialog.set_close_response("cancel");
    dialog.connect_response(None, move |_, resp| {
        if resp == "ok" {
            on_confirm();
        }
    });
    dialog.present(Some(anchor));
}
