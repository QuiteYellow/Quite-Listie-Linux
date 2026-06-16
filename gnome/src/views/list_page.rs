//! `ListPage` — the open-list view. GNOME counterpart of `qml/ListPage.qml` +
//! Swift `ListView`. Items are grouped into **collapsible label sections** (mirroring
//! the Swift `filteredItemsGroupedByLabel` / `renderSection` model): each section has a
//! header (colour/emoji + name + unchecked count + chevron), the unchecked item rows, a
//! per-section inline "add item" entry, and — depending on the completed-at-bottom
//! setting — either the section's own checked items inline or a trailing "Completed"
//! section.
//!
//! Rows are `adw::ActionRow`s: the prefix checkbox toggles completion, a single click on
//! the row body opens the item editor. The body is rebuilt from
//! [`ListItemModel::grouped_sections`] whenever the model mutates (its store's
//! `items-changed` is used as the change signal).

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::controller::Controller;
use crate::models::{self, LabelSection, ListItemModel, ListItemObject, NO_LABEL};

/// Build the list page for the model's current list.
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

    // Add-item button (Swift ListView's leading "+"): focuses the first section's inline
    // "Add item…" entry, since adds in this port are inline rather than a separate sheet.
    let add_btn = gtk::Button::from_icon_name("list-add-symbolic");
    add_btn.set_tooltip_text(Some("Add Item"));
    header.pack_start(&add_btn);

    // Map toggle (Swift ListView's map/list toggle): pushes the per-list map page. Shown
    // only when the list has map data and at least one pin; visibility is kept in sync by
    // `repopulate` below.
    let map_btn = gtk::Button::from_icon_name("mark-location-symbolic");
    map_btn.set_tooltip_text(Some("Map"));
    map_btn.set_visible(false);
    header.pack_start(&map_btn);
    map_btn.connect_clicked(glib::clone!(
        #[weak]
        controller,
        #[strong]
        list_id,
        move |btn| {
            if let Some(nav) = btn.ancestor(adw::NavigationView::static_type()).and_downcast::<adw::NavigationView>() {
                let title = controller.list_name(&list_id);
                let toggle = crate::views::sidebar_toggle(btn);
                nav.push(&crate::views::map_page::build(&controller, &list_id, &title, toggle));
            }
        }
    ));

    // Search toggle + revealing search bar. The model is already search-aware
    // (`set_search_text` → `grouped_sections` filter), matching Swift's `searchText`.
    let search_btn = gtk::ToggleButton::builder()
        .icon_name("system-search-symbolic")
        .tooltip_text("Search items")
        .build();
    header.pack_start(&search_btn);
    header.pack_end(&actions_menu(controller, &list_model, &list_id));
    toolbar.add_top_bar(&header);

    let search_entry = gtk::SearchEntry::builder().placeholder_text("Search items").build();
    let search_bar = gtk::SearchBar::builder().child(&search_entry).build();
    search_bar.connect_entry(&search_entry);
    search_btn
        .bind_property("active", &search_bar, "search-mode-enabled")
        .bidirectional()
        .sync_create()
        .build();
    toolbar.add_top_bar(&search_bar);
    search_entry.connect_search_changed(glib::clone!(
        #[strong]
        list_model,
        move |e| list_model.set_search_text(e.text().trim())
    ));
    // Hiding the bar clears the filter so the next open isn't silently filtered.
    search_btn.connect_toggled(glib::clone!(
        #[strong]
        list_model,
        #[weak]
        search_entry,
        move |b| {
            if !b.is_active() {
                search_entry.set_text("");
                list_model.set_search_text("");
            }
        }
    ));

    // --- grouped, collapsible sections -------------------------------------
    list_model.set_show_checked_at_bottom(controller.list_completed_at_bottom(&list_id));

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

    // Per-list background gradient (matches Swift `ListView`'s `.background`): paint it
    // behind a transparent scroller via an overlay. Cards/rows keep their own surface;
    // the gradient shows through the section spacing and headers.
    match crate::views::list_settings::page_background(&controller.list_background(&list_id)) {
        Some(bg) => {
            scroller.add_css_class("ql-transparent");
            let overlay = gtk::Overlay::builder().hexpand(true).vexpand(true).build();
            overlay.set_child(Some(&bg));
            overlay.add_overlay(&scroller);
            toolbar.set_content(Some(&overlay));
        }
        None => toolbar.set_content(Some(&scroller)),
    }

    // Section key whose add-entry should regain focus after the next rebuild (so rapid
    // entry isn't interrupted by the full repopulate each add triggers).
    let pending_focus: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

    // Header "+" opens the full "Add Item" screen (Swift's `+` → AddItemView). The inline
    // per-section "Add item…" entries remain for quick text-only adds.
    add_btn.connect_clicked(glib::clone!(
        #[strong]
        list_model,
        move |btn| crate::views::item_editor::open_new(btn, list_model.clone())
    ));

    let repopulate: Rc<dyn Fn()> = {
        let body = body.clone();
        let list_model = list_model.clone();
        let controller = controller.clone();
        let pending_focus = pending_focus.clone();
        let map_btn = map_btn.clone();
        Rc::new(move || {
            while let Some(child) = body.first_child() {
                body.remove(&child);
            }
            // "Pinned Locations" banner → per-list map (Swift ListView map entry). Shown
            // only when the list has map data enabled and at least one item is pinned.
            let list_id = list_model.list_id();
            let mut has_pins = false;
            if controller.list_enable_map_data(&list_id) {
                let pins = models::list_pins(&controller, &list_id);
                if !pins.is_empty() {
                    has_pins = true;
                    body.append(&pinned_locations_banner(&controller, &list_id, pins.len()));
                }
            }
            map_btn.set_visible(has_pins);
            let mut sections = list_model.grouped_sections(controller.hide_empty_labels());
            // Always offer at least one place to add (mirrors No Label being the catch-all).
            if !sections.iter().any(|s| !s.is_completed) {
                sections.insert(0, empty_no_label_section());
            }
            for section in &sections {
                body.append(&build_section(&list_model, &controller, section, &pending_focus));
            }
        })
    };
    repopulate();

    // Re-render once per model mutation. This page owns the model's single change
    // callback, replacing the previous page's — so switching lists doesn't leak
    // handlers and a rebuild fires exactly one repopulate (not one per row).
    list_model.set_on_changed(repopulate.clone());

    adw::NavigationPage::builder()
        .title(title)
        .child(&toolbar)
        .build()
}

/// The list-actions menu, shared by the list and kanban pages. Mirrors the Swift
/// `ListView` overflow menu: a top-level of common actions plus two nested submenus
/// ("Presets & Sharing" and "Mark All Items As…") to keep the top level short.
pub(crate) fn actions_menu(
    controller: &Controller,
    list_model: &Rc<ListItemModel>,
    list_id: &str,
) -> gtk::MenuButton {
    let menu = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .tooltip_text("List actions")
        .build();
    let list_id = list_id.to_string();

    let group = gio::SimpleActionGroup::new();
    let add_action = |name: &str, callback: Box<dyn Fn()>| {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate(move |_, _| callback());
        group.add_action(&action);
    };

    add_action(
        "refresh",
        Box::new(glib::clone!(
            #[weak]
            controller,
            move || controller.refresh_lists()
        )),
    );
    add_action(
        "view-mode",
        Box::new(glib::clone!(
            #[weak]
            controller,
            #[strong]
            list_id,
            move || {
                let next = if controller.list_view_mode(&list_id) == "kanban" { "list" } else { "kanban" };
                controller.set_list_view_mode(&list_id, next);
            }
        )),
    );
    add_action(
        "completed-bottom",
        Box::new(glib::clone!(
            #[weak]
            controller,
            #[strong]
            list_id,
            move || {
                let now = !controller.list_completed_at_bottom(&list_id);
                controller.set_list_completed_at_bottom(&list_id, now);
            }
        )),
    );
    add_action(
        "mark-complete",
        Box::new(glib::clone!(
            #[strong]
            list_model,
            move || list_model.set_all_checked(true)
        )),
    );
    add_action(
        "mark-active",
        Box::new(glib::clone!(
            #[strong]
            list_model,
            move || list_model.set_all_checked(false)
        )),
    );
    add_action(
        "export",
        Box::new(glib::clone!(
            #[weak]
            controller,
            #[weak]
            menu,
            #[strong]
            list_id,
            move || crate::views::markdown_export::present(&menu, &controller, &list_id)
        )),
    );
    add_action(
        "share",
        Box::new(glib::clone!(
            #[weak]
            controller,
            #[weak]
            menu,
            #[strong]
            list_id,
            move || crate::views::share_link::present(&menu, &controller, &list_id)
        )),
    );
    add_action(
        "labels",
        Box::new(glib::clone!(
            #[weak]
            controller,
            #[weak]
            menu,
            #[strong]
            list_id,
            move || crate::views::label_editor::present(&menu, &controller, &list_id)
        )),
    );
    add_action(
        "settings",
        Box::new(glib::clone!(
            #[weak]
            controller,
            #[weak]
            menu,
            #[strong]
            list_id,
            move || crate::views::list_settings::present(&menu, &controller, &list_id)
        )),
    );
    add_action(
        "rename",
        Box::new(glib::clone!(
            #[weak]
            controller,
            #[weak]
            menu,
            #[strong]
            list_id,
            move || present_rename_dialog(&menu, &controller, &list_id)
        )),
    );
    add_action(
        "recycle",
        Box::new(glib::clone!(
            #[weak]
            controller,
            #[weak]
            menu,
            #[strong]
            list_id,
            move || crate::views::recycle_bin::present(&menu, &controller, &list_id)
        )),
    );
    add_action(
        "changes",
        Box::new(glib::clone!(
            #[weak]
            controller,
            #[weak]
            menu,
            #[strong]
            list_id,
            move || crate::views::recent_changes::present(&menu, &controller, &list_id)
        )),
    );
    add_action(
        "close",
        Box::new(glib::clone!(
            #[weak]
            controller,
            #[strong]
            list_id,
            move || {
                controller.exclude_list(&list_id);
                controller.select_list("");
            }
        )),
    );
    add_action(
        "delete",
        Box::new(glib::clone!(
            #[weak]
            controller,
            #[weak]
            menu,
            #[strong]
            list_id,
            move || present_delete_confirm(&menu, &controller, &list_id)
        )),
    );

    let is_kanban = controller.list_view_mode(&list_id) == "kanban";
    let at_bottom = controller.list_completed_at_bottom(&list_id);

    let model = gio::Menu::new();

    let refresh_section = gio::Menu::new();
    refresh_section.append(Some("Refresh"), Some("list.refresh"));
    model.append_section(None, &refresh_section);

    let view_section = gio::Menu::new();
    view_section.append(Some(if is_kanban { "List View" } else { "Kanban View" }), Some("list.view-mode"));
    view_section.append(
        Some(if at_bottom { "Show Completed Inline" } else { "Show Completed at Bottom" }),
        Some("list.completed-bottom"),
    );
    let mark_all = gio::Menu::new();
    mark_all.append(Some("Complete"), Some("list.mark-complete"));
    mark_all.append(Some("Active"), Some("list.mark-active"));
    view_section.append_submenu(Some("Mark All Items As…"), &mark_all);
    model.append_section(None, &view_section);

    let tools_section = gio::Menu::new();
    let presets = gio::Menu::new();
    presets.append(Some("Export Markdown…"), Some("list.export"));
    presets.append(Some("Share as Link…"), Some("list.share"));
    tools_section.append_submenu(Some("Presets & Sharing"), &presets);
    tools_section.append(Some("Labels…"), Some("list.labels"));
    tools_section.append(Some("List Settings…"), Some("list.settings"));
    tools_section.append(Some("Rename…"), Some("list.rename"));
    tools_section.append(Some("Recycle Bin…"), Some("list.recycle"));
    tools_section.append(Some("Recent Changes…"), Some("list.changes"));
    model.append_section(None, &tools_section);

    let close_section = gio::Menu::new();
    close_section.append(Some("Close List"), Some("list.close"));
    close_section.append(Some("Delete List…"), Some("list.delete"));
    model.append_section(None, &close_section);

    menu.insert_action_group("list", Some(&group));
    menu.set_menu_model(Some(&model));
    menu
}

/// Build one collapsible label section.
fn build_section(
    list_model: &Rc<ListItemModel>,
    controller: &Controller,
    section: &LabelSection,
    pending_focus: &Rc<RefCell<Option<String>>>,
) -> gtk::Box {
    let list_id = list_model.list_id();
    let container = gtk::Box::new(gtk::Orientation::Vertical, 4);

    // --- header (flat, click toggles the section) --------------------------
    let header = gtk::Button::builder().has_frame(false).build();
    let hbox = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_top(2)
        .margin_bottom(2)
        .build();
    let expanded = controller.section_expanded(&list_id, &section.key);
    let chevron = gtk::Image::from_icon_name(chevron_icon(expanded));
    hbox.append(&chevron);

    if !section.emoji.is_empty() {
        let e = gtk::Label::new(Some(&section.emoji));
        hbox.append(&e);
    } else if !section.color.is_empty() {
        let dot = gtk::Label::new(None);
        dot.set_markup(&format!(
            "<span foreground=\"{}\" size=\"large\">●</span>",
            sanitize_hex(&section.color)
        ));
        hbox.append(&dot);
    }

    let name = gtk::Label::builder().label(&section.key).xalign(0.0).hexpand(true).build();
    name.add_css_class("heading");
    hbox.append(&name);
    let count = gtk::Label::new(Some(&section.header_count.to_string()));
    count.add_css_class("dim-label");
    hbox.append(&count);
    header.set_child(Some(&hbox));
    container.append(&header);

    // --- revealer body -----------------------------------------------------
    let revealer = gtk::Revealer::builder()
        .reveal_child(expanded)
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .build();
    let inner = gtk::Box::new(gtk::Orientation::Vertical, 6);

    if !section.primary_items.is_empty() {
        inner.append(&item_list(list_model, &section.primary_items));
    }

    if !section.is_completed {
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

        // Restore focus to this section's entry after a rebuild triggered by its add.
        if pending_focus.borrow().as_deref() == Some(section.key.as_str()) {
            *pending_focus.borrow_mut() = None;
            let entry = entry.clone();
            glib::idle_add_local_once(move || {
                entry.grab_focus();
            });
        }
        inner.append(&entry);
    }

    if !section.extra_checked.is_empty() {
        inner.append(&item_list(list_model, &section.extra_checked));
    }

    revealer.set_child(Some(&inner));
    container.append(&revealer);

    header.connect_clicked(glib::clone!(
        #[weak]
        controller,
        #[weak]
        revealer,
        #[weak]
        chevron,
        #[strong]
        list_id,
        #[strong(rename_to = key)]
        section.key,
        move |_| {
            let now = !controller.section_expanded(&list_id, &key);
            controller.set_section_expanded(&list_id, &key, now);
            revealer.set_reveal_child(now);
            chevron.set_icon_name(Some(chevron_icon(now)));
        }
    ));

    container
}

/// A boxed-list of item rows.
fn item_list(list_model: &Rc<ListItemModel>, items: &[ListItemObject]) -> gtk::ListBox {
    let list_box = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
    list_box.add_css_class("boxed-list");
    for obj in items {
        list_box.append(&build_item_row(list_model, obj));
    }
    list_box
}

/// One item row: prefix checkbox toggles completion; activating the row opens the editor.
pub(crate) fn build_item_row(list_model: &Rc<ListItemModel>, obj: &ListItemObject) -> adw::ActionRow {
    let checked = obj.checked();
    let note = glib::markup_escape_text(&obj.note());
    let row = adw::ActionRow::builder()
        .activatable(true)
        .title(if checked { format!("<s>{note}</s>") } else { note.to_string() })
        .build();
    if checked {
        row.add_css_class("dim-label");
    }

    let check = gtk::CheckButton::builder().active(checked).valign(gtk::Align::Center).build();
    check.connect_toggled(glib::clone!(
        #[strong]
        list_model,
        #[strong(rename_to = item_id)]
        obj.item_id(),
        move |_| list_model.toggle_checked(&item_id)
    ));
    row.add_prefix(&check);

    let q = obj.quantity_display();
    if !q.is_empty() {
        let badge = gtk::Label::new(Some(&format!("×{q}")));
        badge.add_css_class("ql-quantity-badge");
        badge.set_valign(gtk::Align::Center);
        row.add_suffix(&badge);
    }

    if obj.has_reminder() {
        // Chip = proximity bell icon + label (+ repeat glyph when recurring), matching
        // Swift's ReminderChipView and the KDE ItemRow.
        let chip = gtk::Box::new(gtk::Orientation::Horizontal, 3);
        chip.add_css_class("caption");
        chip.add_css_class("ql-reminder-chip");
        chip.set_valign(gtk::Align::Center);
        let bell = gtk::Image::from_icon_name(if obj.reminder_proximity() == 0 {
            "appointment-missed-symbolic"
        } else {
            "alarm-symbolic"
        });
        bell.set_pixel_size(12);
        chip.append(&bell);
        chip.append(&gtk::Label::new(Some(&obj.reminder_display())));
        if obj.reminder_is_repeating() {
            let repeat = gtk::Image::from_icon_name("media-playlist-repeat-symbolic");
            repeat.set_pixel_size(12);
            chip.append(&repeat);
        }
        apply_reminder_class(&chip, obj.reminder_proximity());
        row.add_suffix(&chip);
    }

    if obj.has_location() {
        let pin = gtk::Image::from_icon_name("mark-location-symbolic");
        pin.add_css_class("dim-label");
        pin.set_valign(gtk::Align::Center);
        row.add_suffix(&pin);
    }

    row.connect_activated(glib::clone!(
        #[strong]
        list_model,
        #[strong(rename_to = item_id)]
        obj.item_id(),
        move |r| crate::views::item_editor::open(r, list_model.clone(), &item_id)
    ));

    // Right-click context menu (Swift `itemContextMenu`; KDE `ItemRow.qml`): Edit,
    // Copy Item Link, Increase/Decrease Quantity, Delete.
    let menu_gesture = gtk::GestureClick::builder().button(gtk::gdk::BUTTON_SECONDARY).build();
    menu_gesture.connect_pressed(glib::clone!(
        #[strong]
        list_model,
        #[weak]
        row,
        #[strong(rename_to = item_id)]
        obj.item_id(),
        #[strong(rename_to = quantity)]
        obj.quantity(),
        move |_, _, x, y| {
            let popover = item_context_menu(&list_model, &item_id, quantity, &row);
            popover.set_parent(&row);
            popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            popover.connect_closed(|p| p.unparent());
            popover.popup();
        }
    ));
    row.add_controller(menu_gesture);
    row
}

/// Build the per-item right-click menu. Mirrors Swift `ListView.itemContextMenu`.
fn item_context_menu(
    list_model: &Rc<ListItemModel>,
    item_id: &str,
    quantity: f64,
    row: &adw::ActionRow,
) -> gtk::Popover {
    let popover = gtk::Popover::new();
    let menu_box = gtk::Box::new(gtk::Orientation::Vertical, 0);

    let edit_btn = flat_menu_button("Edit Item…");
    let copy_btn = flat_menu_button("Copy Item Link");
    let increase_btn = flat_menu_button("Increase Quantity");
    menu_box.append(&edit_btn);
    menu_box.append(&copy_btn);
    menu_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    menu_box.append(&increase_btn);

    // qty >= 2 gets a "Decrease Quantity" entry; deletion is always its own destructive row.
    let decrease_btn = (quantity >= 2.0).then(|| {
        let b = flat_menu_button("Decrease Quantity");
        menu_box.append(&b);
        b
    });
    menu_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let delete_btn = flat_menu_button("Delete Item…");
    delete_btn.add_css_class("destructive-action");
    menu_box.append(&delete_btn);

    popover.set_child(Some(&menu_box));

    edit_btn.connect_clicked(glib::clone!(
        #[strong]
        list_model,
        #[weak]
        popover,
        #[weak]
        row,
        #[strong(rename_to = id)]
        item_id.to_string(),
        move |_| {
            popover.popdown();
            crate::views::item_editor::open(&row, list_model.clone(), &id);
        }
    ));
    copy_btn.connect_clicked(glib::clone!(
        #[weak]
        popover,
        #[weak]
        row,
        #[strong(rename_to = id)]
        item_id.to_string(),
        move |_| {
            popover.popdown();
            row.clipboard().set_text(&format!("quitelistie://item?id={id}"));
        }
    ));
    increase_btn.connect_clicked(glib::clone!(
        #[strong]
        list_model,
        #[weak]
        popover,
        #[strong(rename_to = id)]
        item_id.to_string(),
        move |_| {
            popover.popdown();
            list_model.increment_quantity(&id);
        }
    ));
    if let Some(decrease_btn) = decrease_btn {
        decrease_btn.connect_clicked(glib::clone!(
            #[strong]
            list_model,
            #[weak]
            popover,
            #[strong(rename_to = id)]
            item_id.to_string(),
            move |_| {
                popover.popdown();
                list_model.decrement_quantity(&id);
            }
        ));
    }
    delete_btn.connect_clicked(glib::clone!(
        #[strong]
        list_model,
        #[weak]
        popover,
        #[weak]
        row,
        #[strong(rename_to = id)]
        item_id.to_string(),
        move |_| {
            popover.popdown();
            present_delete_item_confirm(&row, &list_model, &id);
        }
    ));

    popover
}

/// Confirm before soft-deleting an item (Swift "Delete Item?" alert → Recycle Bin).
fn present_delete_item_confirm(anchor: &impl IsA<gtk::Widget>, list_model: &Rc<ListItemModel>, item_id: &str) {
    let dialog = adw::AlertDialog::new(
        Some("Delete Item?"),
        Some("Item will be moved to the Recycle Bin and automatically deleted after 30 days."),
    );
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("delete", "Delete");
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_close_response("cancel");
    dialog.connect_response(
        None,
        glib::clone!(
            #[strong]
            list_model,
            #[strong(rename_to = id)]
            item_id.to_string(),
            move |_, response| {
                if response == "delete" {
                    list_model.delete_item(&id);
                }
            }
        ),
    );
    dialog.present(Some(anchor));
}

/// A "Pinned Locations" banner card that drills into the per-list map. Mirrors the Swift
/// `ListView` map entry (icon + title + "N items with location" + chevron).
fn pinned_locations_banner(controller: &Controller, list_id: &str, count: usize) -> gtk::ListBox {
    let row = adw::ActionRow::builder()
        .activatable(true)
        .title("Pinned Locations")
        .subtitle(&format!("{count} item{} with location", if count == 1 { "" } else { "s" }))
        .build();
    row.add_prefix(&gtk::Image::from_icon_name("mark-location-symbolic"));
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    row.connect_activated(glib::clone!(
        #[weak]
        controller,
        #[strong(rename_to = list_id)]
        list_id.to_string(),
        move |r| {
            if let Some(nav) = r.ancestor(adw::NavigationView::static_type()).and_downcast::<adw::NavigationView>() {
                let title = controller.list_name(&list_id);
                let toggle = crate::views::sidebar_toggle(r);
                nav.push(&crate::views::map_page::build(&controller, &list_id, &title, toggle));
            }
        }
    ));
    let list_box = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
    list_box.add_css_class("boxed-list");
    list_box.append(&row);
    list_box
}

fn empty_no_label_section() -> LabelSection {
    LabelSection {
        key: NO_LABEL.to_string(),
        label_id: String::new(),
        color: String::new(),
        emoji: String::new(),
        is_completed: false,
        header_count: 0,
        primary_items: Vec::new(),
        extra_checked: Vec::new(),
    }
}

fn chevron_icon(expanded: bool) -> &'static str {
    if expanded {
        "pan-down-symbolic"
    } else {
        "pan-end-symbolic"
    }
}

fn flat_menu_button(label: &str) -> gtk::Button {
    gtk::Button::builder()
        .label(label)
        .has_frame(false)
        .halign(gtk::Align::Fill)
        .build()
}

fn present_rename_dialog(anchor: &impl IsA<gtk::Widget>, controller: &Controller, list_id: &str) {
    let dialog = adw::AlertDialog::new(Some("Rename List"), None);
    let entry = gtk::Entry::builder().text(controller.list_name(list_id)).build();
    dialog.set_extra_child(Some(&entry));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("rename", "Rename");
    dialog.set_response_appearance("rename", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("rename"));
    dialog.set_close_response("cancel");

    let id = list_id.to_string();
    dialog.connect_response(
        None,
        glib::clone!(
            #[weak]
            controller,
            #[weak]
            entry,
            move |_, response| {
                if response == "rename" {
                    let name = entry.text();
                    let name = name.trim();
                    if !name.is_empty() {
                        controller.rename_list(&id, name);
                    }
                }
            }
        ),
    );
    dialog.present(Some(anchor));
}

pub(crate) fn present_delete_confirm(anchor: &impl IsA<gtk::Widget>, controller: &Controller, list_id: &str) {
    let name = controller.list_name(list_id);
    let dialog = adw::AlertDialog::new(
        Some("Delete List?"),
        Some(&format!("“{name}” will be permanently deleted. This cannot be undone.")),
    );
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("delete", "Delete");
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_close_response("cancel");

    let id = list_id.to_string();
    dialog.connect_response(
        None,
        glib::clone!(
            #[weak]
            controller,
            move |_, response| {
                if response == "delete" {
                    controller.delete_list_permanently(&id);
                }
            }
        ),
    );
    dialog.present(Some(anchor));
}

fn apply_reminder_class(widget: &impl IsA<gtk::Widget>, proximity: i32) {
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
pub(crate) fn sanitize_hex(hex: &str) -> String {
    if hex.starts_with('#') && hex.len() <= 9 && hex[1..].chars().all(|c| c.is_ascii_hexdigit()) {
        hex.to_string()
    } else {
        "gray".to_string()
    }
}
