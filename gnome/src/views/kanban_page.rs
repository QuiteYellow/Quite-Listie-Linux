//! `KanbanPage` — the per-list **Kanban board** view mode. GNOME counterpart of Swift
//! `KanbanBoardView`: each label becomes a fixed-width column laid out side by side in a
//! horizontal scroller. Columns are fed by the same [`ListItemModel::grouped_sections`]
//! data as the list view (so completed-at-bottom, hidden/empty labels and the No-Label /
//! Completed catch-alls all behave identically); each column has a header (colour/emoji +
//! name + unchecked count), the item rows, and a per-column inline "add item" entry.
//!
//! Reached via the list-actions menu's **Kanban View** toggle (persisted per list in the
//! controller's `view-modes-json`); the window rebuilds the open page on toggle.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::controller::Controller;
use crate::models::{LabelSection, ListItemModel};
use crate::views::list_page;

/// Column width — matches the Swift "normal" default (400pt).
const COLUMN_WIDTH: i32 = 400;

pub fn build(
    controller: &Controller,
    list_model: Rc<ListItemModel>,
    title: &str,
    sidebar_toggle: &gtk::ToggleButton,
) -> adw::NavigationPage {
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.pack_start(sidebar_toggle);
    let list_id = list_model.list_id();
    header.pack_end(&list_page::actions_menu(controller, &list_model, &list_id));
    toolbar.add_top_bar(&header);

    list_model.set_show_checked_at_bottom(controller.list_completed_at_bottom(&list_id));

    let columns_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .margin_start(12)
        .margin_end(12)
        .margin_top(12)
        .margin_bottom(12)
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hexpand(true)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .child(&columns_box)
        .build();
    toolbar.set_content(Some(&scroller));

    // Section key whose add-entry should regain focus after the next rebuild.
    let pending_focus: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

    let repopulate: Rc<dyn Fn()> = {
        let columns_box = columns_box.clone();
        let list_model = list_model.clone();
        let controller = controller.clone();
        let pending_focus = pending_focus.clone();
        Rc::new(move || {
            while let Some(child) = columns_box.first_child() {
                columns_box.remove(&child);
            }
            let sections = list_model.grouped_sections(controller.hide_empty_labels());
            for section in &sections {
                columns_box.append(&build_column(&list_model, section, &pending_focus));
            }
        })
    };
    repopulate();
    list_model.set_on_changed(repopulate.clone());

    adw::NavigationPage::builder().title(title).child(&toolbar).build()
}

/// One Kanban column for a label section.
fn build_column(
    list_model: &Rc<ListItemModel>,
    section: &LabelSection,
    pending_focus: &Rc<RefCell<Option<String>>>,
) -> gtk::Box {
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .width_request(COLUMN_WIDTH)
        .build();

    // --- header (not collapsible, unlike the list view's sections) ---------
    let hbox = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_start(4)
        .margin_end(4)
        .build();
    if !section.emoji.is_empty() {
        hbox.append(&gtk::Label::new(Some(&section.emoji)));
    } else if !section.color.is_empty() {
        let dot = gtk::Label::new(None);
        dot.set_markup(&format!(
            "<span foreground=\"{}\" size=\"large\">●</span>",
            list_page::sanitize_hex(&section.color)
        ));
        hbox.append(&dot);
    }
    let name = gtk::Label::builder().label(&section.key).xalign(0.0).hexpand(true).build();
    name.add_css_class("heading");
    hbox.append(&name);
    let count = gtk::Label::new(Some(&section.header_count.to_string()));
    count.add_css_class("dim-label");
    hbox.append(&count);
    column.append(&hbox);

    // --- scrolling column body --------------------------------------------
    let inner = gtk::Box::new(gtk::Orientation::Vertical, 6);

    if !section.primary_items.is_empty() {
        let list_box = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
        list_box.add_css_class("boxed-list");
        for obj in &section.primary_items {
            list_box.append(&list_page::build_item_row(list_model, obj));
        }
        inner.append(&list_box);
    }

    if !section.is_completed {
        inner.append(&inline_add(list_model, section, pending_focus));
    }

    if !section.extra_checked.is_empty() {
        let list_box = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
        list_box.add_css_class("boxed-list");
        for obj in &section.extra_checked {
            list_box.append(&list_page::build_item_row(list_model, obj));
        }
        inner.append(&list_box);
    }

    let body_scroller = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&inner)
        .build();
    column.append(&body_scroller);

    column
}

/// The per-column inline add entry (Enter or the trailing icon adds an item to this label).
fn inline_add(
    list_model: &Rc<ListItemModel>,
    section: &LabelSection,
    pending_focus: &Rc<RefCell<Option<String>>>,
) -> gtk::Entry {
    let entry = gtk::Entry::builder()
        .placeholder_text("Add item…")
        .secondary_icon_name("list-add-symbolic")
        .build();
    let label_id = section.label_id.clone();
    let key = section.key.clone();
    let do_add = glib::clone!(
        #[strong]
        list_model,
        #[weak]
        entry,
        #[strong]
        pending_focus,
        move || {
            let text = entry.text();
            let text = text.trim();
            if !text.is_empty() {
                *pending_focus.borrow_mut() = Some(key.clone());
                list_model.add_item(text, &label_id, 1.0, "", "", f64::NAN, f64::NAN);
            }
        }
    );
    entry.connect_activate(glib::clone!(
        #[strong]
        do_add,
        move |_| do_add()
    ));
    entry.connect_icon_release(glib::clone!(
        #[strong]
        do_add,
        move |_, pos| {
            if pos == gtk::EntryIconPosition::Secondary {
                do_add();
            }
        }
    ));

    if pending_focus.borrow().as_deref() == Some(section.key.as_str()) {
        *pending_focus.borrow_mut() = None;
        let entry = entry.clone();
        glib::idle_add_local_once(move || {
            entry.grab_focus();
        });
    }
    entry
}
