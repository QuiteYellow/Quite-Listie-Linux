//! `ItemEditor` — edit an existing item. GNOME counterpart of Swift `ItemView`.
//!
//! Presented as a full-width [`adw::NavigationPage`] pushed onto the content
//! navigation stack (the GNOME equivalent of Swift's `fullScreenCover`), so it scales to
//! the window. Layout is adaptive via [`adw::MultiLayoutView`]: on wide windows the
//! fields and the notes editor sit side-by-side; on narrow ones they stack. The notes
//! pane has an Edit/Preview toggle (defaults to Preview): Edit is a multi-line
//! [`gtk::TextView`] with a markdown snippet toolbar (Swift `MarkdownEditorView`); Preview
//! renders the markdown via [`crate::widgets::markdown`] with interactive checkable
//! sublists (Swift `CheckableMarkdownView` parity). Fields: note, quantity, label,
//! completed, reminder + recurrence, source URL, location, list/folder chips, and delete.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use chrono::{Datelike, Local, TimeZone, Timelike, Utc};
use gtk::{gio, glib};

use quite_listie_core::model::{Coordinate, ListItem, ReminderRepeatRule, ReminderRepeatUnit};

use crate::models::{self, ListItemModel};
use crate::views::location_picker;
use crate::widgets::{map, markdown};

/// Open the editor for an existing `item_id`, pushing it onto `anchor`'s navigation view.
pub fn open(anchor: &impl IsA<gtk::Widget>, list_model: Rc<ListItemModel>, item_id: &str) {
    present(anchor, list_model, Some(item_id), None);
}

/// Open the "Add Item" screen (Swift `AddItemView`): the same form with empty defaults,
/// committing a brand-new item on Add.
pub fn open_new(anchor: &impl IsA<gtk::Widget>, list_model: Rc<ListItemModel>) {
    present(anchor, list_model, None, None);
}

/// Open the "Add Item" screen pre-pinned to `location` (Swift `AddItemView(initialLocation:)`,
/// reached from the map's add-at-location gesture).
pub fn open_new_at_location(
    anchor: &impl IsA<gtk::Widget>,
    list_model: Rc<ListItemModel>,
    location: Coordinate,
) {
    present(anchor, list_model, None, Some(location));
}

fn present(
    anchor: &impl IsA<gtk::Widget>,
    list_model: Rc<ListItemModel>,
    item_id: Option<&str>,
    initial_location: Option<Coordinate>,
) {
    let is_new = item_id.is_none();
    let item = match item_id {
        Some(id) => match list_model.get_item(id) {
            Some(i) => i,
            None => return,
        },
        None => {
            let mut i = ListItem::new(String::new());
            i.location = initial_location;
            i
        }
    };
    let Some(nav) = anchor
        .ancestor(adw::NavigationView::static_type())
        .and_downcast::<adw::NavigationView>()
    else {
        return;
    };
    let labels = list_model.labels();

    // --- header (back = cancel; Save commits) ------------------------------
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    // Savable screen: hide the window close button so the only exits are Cancel
    // (back) or Save, mirroring the Swift fullScreenCover (no window chrome).
    header.set_show_end_title_buttons(false);
    // Sidebar toggle, like every other content header (the editor is a content drill-in).
    if let Some(toggle) = crate::views::sidebar_toggle(anchor) {
        header.pack_start(&toggle);
    }
    let save_button = gtk::Button::with_label(if is_new { "Add" } else { "Save" });
    save_button.add_css_class("suggested-action");
    header.pack_end(&save_button);
    // Copy Item Link (Swift ItemView's "Copy" button) — a quitelistie:// deeplink. Only
    // meaningful for a saved item, so it's omitted on the Add screen.
    if !is_new {
        let id = item.id.to_string();
        let copy_button = gtk::Button::from_icon_name("edit-copy-symbolic");
        copy_button.set_tooltip_text(Some("Copy Item Link"));
        header.pack_start(&copy_button);
        copy_button.connect_clicked(glib::clone!(
            #[weak]
            header,
            move |btn| {
                header.clipboard().set_text(&format!("quitelistie://item?id={id}"));
                // Transient "Copied!" feedback (Swift's checkmark swap), revert after 2s.
                btn.set_icon_name("object-select-symbolic");
                btn.set_tooltip_text(Some("Copied!"));
                btn.add_css_class("success");
                glib::timeout_add_seconds_local_once(
                    2,
                    glib::clone!(
                        #[weak]
                        btn,
                        move || {
                            btn.set_icon_name("edit-copy-symbolic");
                            btn.set_tooltip_text(Some("Copy Item Link"));
                            btn.remove_css_class("success");
                        }
                    ),
                );
            }
        ));
    }
    toolbar.add_top_bar(&header);

    // --- fields pane -------------------------------------------------------
    let fields = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();

    let note_row = adw::EntryRow::builder().title("Name").text(&item.note).build();
    group.add(&note_row);

    let qty_row = adw::SpinRow::builder()
        .title("Quantity")
        .adjustment(&gtk::Adjustment::new(item.quantity.max(1.0), 1.0, 100.0, 1.0, 10.0, 0.0))
        .digits(0)
        .build();
    group.add(&qty_row);

    let label_model = gtk::StringList::new(&["No Label"]);
    let mut selected_label = 0u32;
    for (i, l) in labels.iter().enumerate() {
        label_model.append(&l.name);
        if item.label_id.as_deref() == Some(l.id.as_str()) {
            selected_label = (i + 1) as u32;
        }
    }
    let label_row = adw::ComboRow::builder()
        .title("Label")
        .model(&label_model)
        .selected(selected_label)
        .build();
    group.add(&label_row);

    // Completed switch (Swift ItemFormView's "Completed" toggle). When on, the name
    // becomes read-only, matching Swift's strikethrough read-only name.
    let checked_row = adw::SwitchRow::builder().title("Completed").active(item.checked).build();
    group.add(&checked_row);
    let apply_checked_style = glib::clone!(
        #[weak]
        note_row,
        #[weak]
        checked_row,
        move || {
            let on = checked_row.is_active();
            note_row.set_editable(!on);
            if on {
                note_row.add_css_class("strikethrough");
            } else {
                note_row.remove_css_class("strikethrough");
            }
        }
    );
    apply_checked_style();
    checked_row.connect_active_notify(glib::clone!(
        #[strong]
        apply_checked_style,
        move |_| apply_checked_style()
    ));

    fields.add(&group);

    // Source URL is managed implicitly (set by paste-location, surfaced as the
    // View-in-Maps link), matching Swift — there is no editable Source URL field.
    let source_url_state = Rc::new(RefCell::new(item.source_url.clone().unwrap_or_default()));

    // Reminder + recurrence.
    let reminder_group = adw::PreferencesGroup::new();
    let reminder_expander = adw::ExpanderRow::builder()
        .title("Reminder")
        .show_enable_switch(true)
        .enable_expansion(item.reminder_date.is_some())
        .build();

    let seed = item
        .reminder_date
        .map(|d| d.with_timezone(&Local))
        .unwrap_or_else(|| Local::now() + chrono::Duration::hours(1));

    let date_row = adw::ActionRow::builder().title("Date").build();
    let calendar = gtk::Calendar::new();
    calendar.select_day(
        &glib::DateTime::new(&glib::TimeZone::local(), seed.year(), seed.month() as i32, seed.day() as i32, 0, 0, 0.0)
            .unwrap(),
    );
    let date_button = gtk::MenuButton::builder().valign(gtk::Align::Center).build();
    let cal_popover = gtk::Popover::new();
    cal_popover.set_child(Some(&calendar));
    date_button.set_popover(Some(&cal_popover));
    set_date_label(&date_button, &calendar);
    calendar.connect_day_selected(glib::clone!(
        #[weak]
        date_button,
        move |cal| set_date_label(&date_button, cal)
    ));
    date_row.add_suffix(&date_button);
    reminder_expander.add_row(&date_row);

    let time_row = adw::ActionRow::builder().title("Time").build();
    let hour = gtk::SpinButton::with_range(0.0, 23.0, 1.0);
    hour.set_wrap(true);
    hour.set_value(seed.hour() as f64);
    two_digit(&hour);
    let minute = gtk::SpinButton::with_range(0.0, 59.0, 1.0);
    minute.set_wrap(true);
    minute.set_value(seed.minute() as f64);
    two_digit(&minute);
    let time_box = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    time_box.set_valign(gtk::Align::Center);
    time_box.append(&hour);
    time_box.append(&gtk::Label::new(Some(":")));
    time_box.append(&minute);
    time_row.add_suffix(&time_box);
    reminder_expander.add_row(&time_row);

    // Don't allow picking a reminder before now (Swift `min(reminderDate, Date())...`);
    // an already-past reminder keeps its loaded value as the floor.
    let floor = item
        .reminder_date
        .map(|d| d.with_timezone(&Local).min(Local::now()))
        .unwrap_or_else(Local::now);
    let clamp_reminder = glib::clone!(
        #[weak]
        calendar,
        #[weak]
        hour,
        #[weak]
        minute,
        move || {
            let d = calendar.date();
            let chosen = Local
                .with_ymd_and_hms(
                    d.year(),
                    d.month() as u32,
                    d.day_of_month() as u32,
                    hour.value() as u32,
                    minute.value() as u32,
                    0,
                )
                .single();
            if let Some(chosen) = chosen {
                if chosen < floor {
                    if let Ok(dt) = glib::DateTime::new(
                        &glib::TimeZone::local(),
                        floor.year(),
                        floor.month() as i32,
                        floor.day() as i32,
                        0,
                        0,
                        0.0,
                    ) {
                        calendar.select_day(&dt);
                    }
                    hour.set_value(floor.hour() as f64);
                    minute.set_value(floor.minute() as f64);
                }
            }
        }
    );
    calendar.connect_day_selected(glib::clone!(
        #[strong]
        clamp_reminder,
        move |_| clamp_reminder()
    ));
    hour.connect_value_changed(glib::clone!(
        #[strong]
        clamp_reminder,
        move |_| clamp_reminder()
    ));
    minute.connect_value_changed(glib::clone!(
        #[strong]
        clamp_reminder,
        move |_| clamp_reminder()
    ));

    // Preset labels mirror Swift's `ReminderRepeatRule.presets.map(displayName)`
    // ("Every 2 Weeks" for biweekly).
    let repeat_model = gtk::StringList::new(&[
        "Never", "Daily", "Weekly", "Every 2 Weeks", "Monthly", "Yearly", "Weekdays", "Custom…",
    ]);
    let repeat_row = adw::ComboRow::builder().title("Repeat").model(&repeat_model).build();
    reminder_expander.add_row(&repeat_row);

    let interval_adj = gtk::Adjustment::new(2.0, 1.0, 365.0, 1.0, 10.0, 0.0);
    let interval_row = adw::SpinRow::builder().title("Every").adjustment(&interval_adj).digits(0).build();
    let unit_row = adw::ComboRow::builder().title("Unit").build();
    reminder_expander.add_row(&interval_row);
    reminder_expander.add_row(&unit_row);

    // Unit labels switch singular/plural by interval (Swift `unit.displayName`/`pluralName`).
    let relabel_units = glib::clone!(
        #[weak]
        unit_row,
        #[weak]
        interval_adj,
        move || {
            let sel = unit_row.selected();
            let model = if interval_adj.value() as i64 == 1 {
                gtk::StringList::new(&["Day", "Week", "Month", "Year", "Weekdays"])
            } else {
                gtk::StringList::new(&["Days", "Weeks", "Months", "Years", "Weekdays"])
            };
            unit_row.set_model(Some(&model));
            unit_row.set_selected(sel);
        }
    );
    relabel_units();
    interval_adj.connect_value_changed(glib::clone!(
        #[strong]
        relabel_units,
        move |_| relabel_units()
    ));

    let mode_model = gtk::StringList::new(&["Fixed Schedule", "After Completion"]);
    let mode_row = adw::ComboRow::builder().title("Mode").model(&mode_model).build();
    reminder_expander.add_row(&mode_row);

    if let Some(rule) = &item.reminder_repeat_rule {
        let (repeat_idx, unit_idx, interval) = rule_to_combo(rule);
        repeat_row.set_selected(repeat_idx);
        unit_row.set_selected(unit_idx);
        interval_row.set_value(interval as f64);
    }
    if matches!(
        item.reminder_repeat_mode,
        Some(quite_listie_core::model::ReminderRepeatMode::AfterComplete)
    ) {
        mode_row.set_selected(1);
    }

    let update_repeat_vis = glib::clone!(
        #[weak]
        repeat_row,
        #[weak]
        interval_row,
        #[weak]
        unit_row,
        #[weak]
        mode_row,
        move || {
            let sel = repeat_row.selected();
            let custom = sel == 7;
            interval_row.set_visible(custom);
            unit_row.set_visible(custom);
            mode_row.set_visible(sel != 0);
        }
    );
    update_repeat_vis();
    repeat_row.connect_selected_notify(glib::clone!(
        #[strong]
        update_repeat_vis,
        move |_| update_repeat_vis()
    ));

    reminder_group.add(&reminder_expander);
    fields.add(&reminder_group);

    // --- location (only when the list has map data enabled) ----------------
    // Current pinned coordinate (read on Save). The source URL lives in `source_url_state`.
    let location_state: Rc<RefCell<Option<Coordinate>>> = Rc::new(RefCell::new(item.location.clone()));
    if list_model.enable_map_data() {
        let location_group = adw::PreferencesGroup::builder().title("Location").build();
        let location_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        location_group.add(&location_box);
        fields.add(&location_group);
        build_location_section(&location_box, &location_state, &source_url_state, &note_row);
    }

    // List (and folder) context chips, edit mode only (Swift EditItemView's chip section).
    if !is_new {
        let chips_group = adw::PreferencesGroup::new();
        let chips_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        chips_box.set_halign(gtk::Align::End);
        chips_box.append(&context_chip(
            &list_model.list_emoji().unwrap_or_default(),
            "view-list-symbolic",
            &list_model.list_name(),
        ));
        if let Some(folder) = list_model.list_folder_name() {
            chips_box.append(&context_chip("", "folder-symbolic", &folder));
        }
        chips_group.add(&chips_box);
        fields.add(&chips_group);
    }

    // Delete only applies to a saved item; the Add screen has no delete.
    let delete_button = (!is_new).then(|| {
        let danger_group = adw::PreferencesGroup::new();
        let b = gtk::Button::builder().label("Delete Item").halign(gtk::Align::Center).margin_top(8).build();
        b.add_css_class("destructive-action");
        b.add_css_class("pill");
        danger_group.add(&b);
        fields.add(&danger_group);
        b
    });

    // --- notes pane (markdown editor + checkable preview) ------------------
    let notes_pane = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    // Heading row: "Notes" + an Edit/Preview linked toggle.
    let notes_header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let notes_heading = gtk::Label::builder().label("Markdown Notes").xalign(0.0).hexpand(true).build();
    notes_heading.add_css_class("heading");
    let edit_toggle = gtk::ToggleButton::with_label("Edit");
    let preview_toggle = gtk::ToggleButton::with_label("Preview");
    preview_toggle.set_group(Some(&edit_toggle));
    let toggle_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    toggle_box.add_css_class("linked");
    toggle_box.append(&edit_toggle);
    toggle_box.append(&preview_toggle);
    notes_header.append(&notes_heading);
    notes_header.append(&toggle_box);

    // Edit view: the raw markdown text (source of truth, read on Save).
    let notes_view = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(8)
        .right_margin(8)
        .build();
    notes_view.buffer().set_text(&item.markdown_notes.clone().unwrap_or_default());
    let notes_scroll = gtk::ScrolledWindow::builder().vexpand(true).child(&notes_view).build();
    notes_scroll.add_css_class("card");

    // Edit pane = snippet toolbar (Swift MarkdownEditorView's accessory bar) + text area.
    let edit_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
    edit_box.append(&markdown_toolbar(&notes_view.buffer()));
    edit_box.append(&notes_scroll);

    // Preview view: rendered markdown, rebuilt from the buffer each time it's shown.
    let preview_scroll = gtk::ScrolledWindow::builder().vexpand(true).build();
    preview_scroll.add_css_class("card");

    let notes_stack = gtk::Stack::new();
    notes_stack.set_vexpand(true);
    notes_stack.add_named(&edit_box, Some("edit"));
    notes_stack.add_named(&preview_scroll, Some("preview"));

    preview_toggle.connect_toggled(glib::clone!(
        #[weak]
        notes_view,
        #[weak]
        preview_scroll,
        #[weak]
        notes_stack,
        move |btn| {
            if btn.is_active() {
                let buffer = notes_view.buffer();
                let (start, end) = buffer.bounds();
                let view: gtk::Widget = if buffer.text(&start, &end, false).trim().is_empty() {
                    // Empty-notes guidance (Swift formRightContent placeholder).
                    let placeholder = gtk::Label::builder()
                        .label("Switch to Edit to add a note, use Markdown for sublists, links, images and more. Sublists can be directly toggled here!")
                        .wrap(true)
                        .xalign(0.0)
                        .yalign(0.0)
                        .build();
                    placeholder.add_css_class("dim-label");
                    placeholder.upcast()
                } else {
                    markdown::checkable_preview(&buffer).upcast()
                };
                view.set_margin_top(8);
                view.set_margin_bottom(8);
                view.set_margin_start(12);
                view.set_margin_end(12);
                preview_scroll.set_child(Some(&view));
                notes_stack.set_visible_child_name("preview");
            } else {
                notes_stack.set_visible_child_name("edit");
            }
        }
    ));
    // Default to Preview (renders the markdown + checkable sublists on open); the handler
    // is now connected, so activating it builds the preview and switches the stack.
    preview_toggle.set_active(true);

    notes_pane.append(&notes_header);
    notes_pane.append(&notes_stack);

    // --- adaptive two-pane via MultiLayoutView -----------------------------
    let mlv = adw::MultiLayoutView::new();

    // 40/60 fields/notes split (Swift `formLeft` 0.4 / `formRight` 0.6). A plain box keeps
    // the natural minimum width — a column-homogeneous grid would inflate it to 5x the
    // widest pane's per-column minimum. A tick callback keeps the fields pane at ~40% of the
    // current width (clamped up to its own minimum when there isn't room); the notes pane
    // carries a left border in place of a separator widget.
    let wide_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let wide_fields = adw::LayoutSlot::new("fields");
    wide_fields.set_vexpand(true);
    let wide_notes = adw::LayoutSlot::new("notes");
    wide_notes.set_hexpand(true);
    wide_notes.set_vexpand(true);
    wide_notes.add_css_class("ql-notes-border");
    wide_box.append(&wide_fields);
    wide_box.append(&wide_notes);
    let fields_weak = wide_fields.downgrade();
    let last_width = std::cell::Cell::new(-1);
    wide_box.add_tick_callback(move |b, _| {
        let w = b.width();
        if w != last_width.get() {
            last_width.set(w);
            if let (true, Some(f)) = (w > 0, fields_weak.upgrade()) {
                f.set_width_request((w as f64 * 0.4) as i32);
            }
        }
        glib::ControlFlow::Continue
    });
    let wide = adw::Layout::new(&wide_box);
    wide.set_name(Some("wide"));

    let narrow_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let narrow_fields = adw::LayoutSlot::new("fields");
    narrow_fields.set_vexpand(true);
    let narrow_notes = adw::LayoutSlot::new("notes");
    narrow_notes.set_vexpand(true);
    narrow_notes.set_size_request(-1, 220);
    narrow_box.append(&narrow_fields);
    narrow_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    narrow_box.append(&narrow_notes);
    let narrow = adw::Layout::new(&narrow_box);
    narrow.set_name(Some("narrow"));

    mlv.add_layout(narrow);
    mlv.add_layout(wide);
    mlv.set_child("fields", &fields);
    mlv.set_child("notes", &notes_pane);
    mlv.set_layout_name("narrow");

    let bin = adw::BreakpointBin::new();
    bin.set_size_request(360, 240);
    bin.set_child(Some(&mlv));
    let breakpoint = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MinWidth,
        600.0,
        adw::LengthUnit::Px,
    ));
    breakpoint.add_setter(&mlv, "layout-name", Some(&"wide".to_value()));
    bin.add_breakpoint(breakpoint);

    toolbar.set_content(Some(&bin));
    let page = adw::NavigationPage::builder()
        .title(if is_new { "Add Item" } else { "Details" })
        .tag("item-editor")
        .child(&toolbar)
        .build();

    // --- actions -----------------------------------------------------------
    let item_id_owned = item.id.to_string();

    // Gate Save/Add on a non-empty name (Swift disables it in both Add and Edit).
    save_button.set_sensitive(!item.note.trim().is_empty());
    note_row.connect_changed(glib::clone!(
        #[weak]
        save_button,
        move |e| save_button.set_sensitive(!e.text().trim().is_empty())
    ));
    let labels_ids: Vec<String> = labels.iter().map(|l| l.id.clone()).collect();

    save_button.connect_clicked(glib::clone!(
        #[strong]
        list_model,
        #[weak]
        nav,
        #[weak]
        note_row,
        #[weak]
        qty_row,
        #[weak]
        label_row,
        #[weak]
        checked_row,
        #[strong]
        source_url_state,
        #[weak]
        notes_view,
        #[weak]
        reminder_expander,
        #[weak]
        calendar,
        #[weak]
        hour,
        #[weak]
        minute,
        #[weak]
        repeat_row,
        #[weak]
        interval_row,
        #[weak]
        unit_row,
        #[weak]
        mode_row,
        #[strong]
        item_id_owned,
        #[strong]
        labels_ids,
        #[strong]
        location_state,
        move |_| {
            let sel = label_row.selected();
            let label_id = if sel == 0 {
                String::new()
            } else {
                labels_ids.get((sel - 1) as usize).cloned().unwrap_or_default()
            };

            let reminder = if reminder_expander.enables_expansion() {
                let d = calendar.date();
                Local
                    .with_ymd_and_hms(
                        d.year(),
                        d.month() as u32,
                        d.day_of_month() as u32,
                        hour.value() as u32,
                        minute.value() as u32,
                        0,
                    )
                    .single()
                    .map(|dt| dt.with_timezone(&Utc))
            } else {
                None
            };
            let rule = reminder
                .and_then(|_| rule_from_combo(repeat_row.selected(), interval_row.value() as u32, unit_row.selected()));
            let mode = rule.as_ref().map(|_| {
                if mode_row.selected() == 0 {
                    quite_listie_core::model::ReminderRepeatMode::Fixed
                } else {
                    quite_listie_core::model::ReminderRepeatMode::AfterComplete
                }
            });

            let buf = notes_view.buffer();
            let (start, end) = buf.bounds();
            let notes = buf.text(&start, &end, false);

            if is_new {
                if note_row.text().trim().is_empty() {
                    return;
                }
                list_model.add_item_full(
                    note_row.text().trim(),
                    qty_row.value(),
                    &label_id,
                    checked_row.is_active(),
                    notes.trim(),
                    source_url_state.borrow().trim(),
                    reminder,
                    rule,
                    mode,
                    location_state.borrow().clone(),
                );
            } else {
                list_model.update_item_fields(
                    &item_id_owned,
                    note_row.text().trim(),
                    qty_row.value(),
                    &label_id,
                    checked_row.is_active(),
                    notes.trim(),
                    source_url_state.borrow().trim(),
                    reminder,
                    rule,
                    mode,
                    location_state.borrow().clone(),
                );
            }
            nav.pop();
        }
    ));

    if let Some(delete_button) = &delete_button {
        delete_button.connect_clicked(glib::clone!(
            #[strong]
            list_model,
            #[weak]
            nav,
            #[strong]
            item_id_owned,
            move |btn| {
                // Confirm before deleting (Swift "Delete Item?" alert).
                let dialog = adw::AlertDialog::new(Some("Delete Item?"), None);
                dialog.add_responses(&[("cancel", "Cancel"), ("delete", "Delete")]);
                dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
                dialog.set_default_response(Some("cancel"));
                dialog.set_close_response("cancel");
                dialog.connect_response(
                    None,
                    glib::clone!(
                        #[strong]
                        list_model,
                        #[weak]
                        nav,
                        #[strong]
                        item_id_owned,
                        move |_, response| {
                            if response == "delete" {
                                list_model.delete_item(&item_id_owned);
                                nav.pop();
                            }
                        }
                    ),
                );
                dialog.present(Some(btn));
            }
        ));
    }

    nav.push(&page);
}

/// Build (and rebuild on change) the location section: a "Choose/View Location" picker
/// entry, paste-from-clipboard, a coordinate readout, remove, and open-in-maps links.
/// Mirrors the Swift `ItemView` location section + `LocationPickerSheet`.
fn build_location_section(
    container: &gtk::Box,
    location_state: &Rc<RefCell<Option<Coordinate>>>,
    source_url: &Rc<RefCell<String>>,
    note_row: &adw::EntryRow,
) {
    let slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let refresh: Rc<dyn Fn()> = {
        let container = container.clone();
        let location_state = location_state.clone();
        let source_url = source_url.clone();
        let note_row = note_row.clone();
        let slot_inner = slot.clone();
        Rc::new(move || {
            while let Some(child) = container.first_child() {
                container.remove(&child);
            }
            let re = slot_inner.borrow().clone();
            let coord = location_state.borrow().clone();
            match coord {
                Some(c) => container.append(&located_rows(&c, &location_state, &source_url, &re)),
                None => container.append(&empty_rows(&location_state, &source_url, &note_row, &re)),
            }
        })
    };
    *slot.borrow_mut() = Some(refresh.clone());
    refresh();
}

/// UI shown when the item has a pinned coordinate.
fn located_rows(
    coord: &Coordinate,
    location_state: &Rc<RefCell<Option<Coordinate>>>,
    source_url: &Rc<RefCell<String>>,
    refresh: &Option<Rc<dyn Fn()>>,
) -> gtk::Box {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 6);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let view = gtk::Button::builder().label("View Location").has_frame(false).build();
    view.add_css_class("accent");
    view.connect_clicked(glib::clone!(
        #[strong]
        location_state,
        #[strong]
        refresh,
        #[strong(rename_to = lat)]
        coord.latitude,
        #[strong(rename_to = lon)]
        coord.longitude,
        move |b| open_picker(b, Some((lat, lon)), &location_state, &refresh)
    ));
    row.append(&view);

    let readout = gtk::Label::new(Some(&format!("{:.4}°, {:.4}°", coord.latitude, coord.longitude)));
    readout.add_css_class("dim-label");
    readout.set_hexpand(true);
    readout.set_xalign(1.0);
    row.append(&readout);

    let remove = gtk::Button::from_icon_name("edit-clear-symbolic");
    remove.add_css_class("flat");
    remove.set_tooltip_text(Some("Remove location"));
    remove.connect_clicked(glib::clone!(
        #[strong]
        location_state,
        #[strong]
        source_url,
        #[strong]
        refresh,
        move |btn| {
            // Confirm before clearing (Swift "Remove Location?" dialog).
            let dialog = adw::AlertDialog::new(Some("Remove Location?"), None);
            dialog.add_responses(&[("cancel", "Cancel"), ("remove", "Remove")]);
            dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("cancel"));
            dialog.set_close_response("cancel");
            dialog.connect_response(
                None,
                glib::clone!(
                    #[strong]
                    location_state,
                    #[strong]
                    source_url,
                    #[strong]
                    refresh,
                    move |_, response| {
                        if response == "remove" {
                            *location_state.borrow_mut() = None;
                            source_url.borrow_mut().clear(); // Swift clears the source URL too.
                            if let Some(r) = &refresh {
                                r();
                            }
                        }
                    }
                ),
            );
            dialog.present(Some(btn));
        }
    ));
    row.append(&remove);
    outer.append(&row);

    // Source-URL link first (Swift's "View in …" section), if set from a pasted link.
    {
        let url = source_url.borrow();
        let url = url.trim();
        if !url.is_empty() {
            let link = map::maps_link(&map::source_url_label(url), url);
            link.set_halign(gtk::Align::Start);
            outer.append(&link);
        }
    }

    // Navigation links, stacked as their own rows (Swift's "Navigate with …" section).
    // Apple Maps / TomTom don't exist on Linux, so Google Maps + OpenStreetMap stand in.
    let gmaps =
        map::maps_link("Navigate with Google Maps", &map::google_maps_uri(coord.latitude, coord.longitude));
    gmaps.set_halign(gtk::Align::Start);
    outer.append(&gmaps);
    let osm = map::maps_link("Navigate with OpenStreetMap", &map::osm_uri(coord.latitude, coord.longitude));
    osm.set_halign(gtk::Align::Start);
    outer.append(&osm);
    outer
}

/// UI shown when the item has no location yet.
fn empty_rows(
    location_state: &Rc<RefCell<Option<Coordinate>>>,
    source_url: &Rc<RefCell<String>>,
    note_row: &adw::EntryRow,
    refresh: &Option<Rc<dyn Fn()>>,
) -> gtk::Box {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 4);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let choose = gtk::Button::builder().label("Choose Location").has_frame(false).build();
    choose.add_css_class("accent");
    choose.connect_clicked(glib::clone!(
        #[strong]
        location_state,
        #[strong]
        refresh,
        move |b| open_picker(b, None, &location_state, &refresh)
    ));
    row.append(&choose);

    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    row.append(&spacer);

    let paste = gtk::Button::from_icon_name("edit-paste-symbolic");
    paste.add_css_class("flat");
    paste.set_tooltip_text(Some("Paste an Apple/Google Maps link"));
    paste.connect_clicked(glib::clone!(
        #[strong]
        location_state,
        #[strong]
        source_url,
        #[weak]
        note_row,
        #[strong]
        refresh,
        move |b| paste_location(b, &location_state, &source_url, &note_row, &refresh)
    ));
    row.append(&paste);
    outer.append(&row);

    let hint = gtk::Label::builder()
        .label("Search by address, or paste an Apple or Google Maps link.")
        .wrap(true)
        .xalign(0.0)
        .build();
    hint.add_css_class("caption");
    hint.add_css_class("dim-label");
    outer.append(&hint);
    outer
}

/// Open the map picker, committing the chosen coordinate back into `location_state`.
fn open_picker(
    anchor: &impl IsA<gtk::Widget>,
    initial: Option<(f64, f64)>,
    location_state: &Rc<RefCell<Option<Coordinate>>>,
    refresh: &Option<Rc<dyn Fn()>>,
) {
    let location_state = location_state.clone();
    let refresh = refresh.clone();
    location_picker::present(
        anchor,
        initial,
        Rc::new(move |lat, lon| {
            *location_state.borrow_mut() = Some(Coordinate { latitude: lat, longitude: lon, extra: Default::default() });
            if let Some(r) = &refresh {
                r();
            }
        }),
    );
}

/// Read the clipboard, resolve it to a coordinate, and pin it (and its source URL).
/// Drives `button` through a loading spinner → success/failed icon (Swift's
/// `PasteLocationFeedback`), reverting after 2s.
fn paste_location(
    button: &gtk::Button,
    location_state: &Rc<RefCell<Option<Coordinate>>>,
    source_url_state: &Rc<RefCell<String>>,
    note_row: &adw::EntryRow,
    refresh: &Option<Rc<dyn Fn()>>,
) {
    let clipboard = button.clipboard();
    let button = button.clone();
    let location_state = location_state.clone();
    let source_url_state = source_url_state.clone();
    let note_row = note_row.clone();
    let refresh = refresh.clone();

    let spinner = gtk::Spinner::new();
    spinner.start();
    button.set_child(Some(&spinner));
    button.set_sensitive(false);

    clipboard.read_text_async(gio::Cancellable::NONE, move |res| {
        let text = match res {
            Ok(Some(t)) if !t.trim().is_empty() => t.to_string(),
            _ => {
                paste_feedback(&button, "process-stop-symbolic", "error");
                return;
            }
        };
        models::resolve_location_async(&text, move |result| {
            if let Some((lat, lon, source_url)) = result {
                *location_state.borrow_mut() =
                    Some(Coordinate { latitude: lat, longitude: lon, extra: Default::default() });
                if !source_url.is_empty() {
                    *source_url_state.borrow_mut() = source_url.clone();
                }
                // Auto-fill an empty name from the pasted place (Swift pasteLocation).
                if note_row.text().trim().is_empty() {
                    if let Some(name) = models::parse_place_name(&source_url) {
                        note_row.set_text(&name);
                    }
                }
                paste_feedback(&button, "object-select-symbolic", "success");
                if let Some(r) = &refresh {
                    r();
                }
            } else {
                paste_feedback(&button, "process-stop-symbolic", "error");
            }
        });
    });
}

/// Show a transient outcome icon on the paste button, reverting to the paste icon after 2s.
fn paste_feedback(button: &gtk::Button, icon: &str, css: &'static str) {
    button.set_icon_name(icon);
    button.add_css_class(css);
    button.set_sensitive(true);
    glib::timeout_add_seconds_local_once(
        2,
        glib::clone!(
            #[weak]
            button,
            move || {
                button.set_icon_name("edit-paste-symbolic");
                button.remove_css_class(css);
            }
        ),
    );
}

/// Markdown editing snippets (Swift `MarkdownSnippet`).
#[derive(Clone, Copy)]
enum Snippet {
    Bold,
    Italic,
    Code,
    CodeBlock,
    Blockquote,
    Link,
    Image,
    Task,
    UnorderedList,
    OrderedList,
    Heading(u8),
}

/// Apply a snippet to `buffer` at the current selection/cursor (Swift `MarkdownSnippet.apply`).
fn apply_snippet(buffer: &gtk::TextBuffer, snippet: Snippet) {
    match snippet {
        Snippet::Bold => wrap_inline(buffer, "**", "**", "bold"),
        Snippet::Italic => wrap_inline(buffer, "*", "*", "italic"),
        Snippet::Code => wrap_inline(buffer, "`", "`", "code"),
        Snippet::CodeBlock => wrap_inline(buffer, "```\n", "\n```", "code"),
        Snippet::Link => insert_link(buffer, false),
        Snippet::Image => insert_link(buffer, true),
        Snippet::Task => line_prefix(buffer, "- [ ] "),
        Snippet::UnorderedList => line_prefix(buffer, "- "),
        Snippet::OrderedList => line_prefix(buffer, "1. "),
        Snippet::Blockquote => line_prefix(buffer, "> "),
        Snippet::Heading(n) => line_prefix(buffer, &format!("{} ", "#".repeat(n as usize))),
    }
    buffer.set_modified(true);
}

/// Wrap the selection (or a placeholder) in `prefix`/`suffix`, leaving the inner text selected.
fn wrap_inline(buffer: &gtk::TextBuffer, prefix: &str, suffix: &str, placeholder: &str) {
    let (start, end) = match buffer.selection_bounds() {
        Some(b) => b,
        None => {
            let it = buffer.iter_at_mark(&buffer.get_insert());
            (it, it)
        }
    };
    let start_off = start.offset();
    let inner = buffer.text(&start, &end, false).to_string();
    let inner = if inner.is_empty() { placeholder.to_string() } else { inner };
    let snippet = format!("{prefix}{inner}{suffix}");
    let (mut s, mut e) = (start, end);
    buffer.delete(&mut s, &mut e);
    let mut ins = buffer.iter_at_offset(start_off);
    buffer.insert(&mut ins, &snippet);
    let inner_start = start_off + prefix.chars().count() as i32;
    let inner_end = inner_start + inner.chars().count() as i32;
    buffer.select_range(&buffer.iter_at_offset(inner_start), &buffer.iter_at_offset(inner_end));
}

/// Insert a `[text](URL)` (or `![alt](URL)`) link, selecting URL when text was supplied,
/// otherwise the placeholder text (Swift `MarkdownSnippet` link/image).
fn insert_link(buffer: &gtk::TextBuffer, is_image: bool) {
    let (start, end) = match buffer.selection_bounds() {
        Some(b) => b,
        None => {
            let it = buffer.iter_at_mark(&buffer.get_insert());
            (it, it)
        }
    };
    let start_off = start.offset();
    let bang = if is_image { "!" } else { "" };
    let selected = buffer.text(&start, &end, false).to_string();
    let has_sel = !selected.is_empty();
    let text = if has_sel {
        selected
    } else if is_image {
        "alt".to_string()
    } else {
        "text".to_string()
    };
    let snippet = format!("{bang}[{text}](URL)");
    let (mut s, mut e) = (start, end);
    buffer.delete(&mut s, &mut e);
    let mut ins = buffer.iter_at_offset(start_off);
    buffer.insert(&mut ins, &snippet);
    let bang_len = bang.chars().count() as i32;
    let (sel_start, sel_len) = if has_sel {
        // Select "URL": past `![text](`.
        (start_off + bang_len + 1 + text.chars().count() as i32 + 2, 3)
    } else {
        // Select the placeholder text: past `![`.
        (start_off + bang_len + 1, text.chars().count() as i32)
    };
    buffer.select_range(&buffer.iter_at_offset(sel_start), &buffer.iter_at_offset(sel_start + sel_len));
}

/// Insert `prefix` at the start of the line containing the cursor (Swift `insertLinePrefix`).
fn line_prefix(buffer: &gtk::TextBuffer, prefix: &str) {
    let cursor = buffer.iter_at_mark(&buffer.get_insert());
    let mut line_start = cursor;
    line_start.set_line_offset(0);
    buffer.insert(&mut line_start, prefix);
}

/// Build the markdown snippet toolbar (Swift MarkdownEditorView's keyboard/glass bar):
/// Task / List / Heading / Link, plus an overflow menu for the rest.
fn markdown_toolbar(buffer: &gtk::TextBuffer) -> gtk::Box {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);

    let snip_button = |icon: &str, tip: &str, snippet: Snippet| {
        let b = gtk::Button::from_icon_name(icon);
        b.add_css_class("flat");
        b.set_tooltip_text(Some(tip));
        b.connect_clicked(glib::clone!(
            #[weak]
            buffer,
            move |_| apply_snippet(&buffer, snippet)
        ));
        b
    };

    bar.append(&snip_button("checkbox-checked-symbolic", "Task", Snippet::Task));
    bar.append(&snip_button("view-list-symbolic", "List", Snippet::UnorderedList));

    // Heading H1-H3 menu.
    let heading_pop = gtk::Popover::new();
    let heading_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
    for n in 1..=3u8 {
        let b = gtk::Button::builder().label(format!("Heading {n}")).has_frame(false).build();
        b.connect_clicked(glib::clone!(
            #[weak]
            buffer,
            #[weak]
            heading_pop,
            move |_| {
                apply_snippet(&buffer, Snippet::Heading(n));
                heading_pop.popdown();
            }
        ));
        heading_box.append(&b);
    }
    heading_pop.set_child(Some(&heading_box));
    let heading_btn = gtk::MenuButton::builder()
        .icon_name("format-text-rich-symbolic")
        .tooltip_text("Heading")
        .popover(&heading_pop)
        .build();
    heading_btn.add_css_class("flat");
    bar.append(&heading_btn);

    bar.append(&snip_button("insert-link-symbolic", "Link", Snippet::Link));

    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    bar.append(&spacer);

    // Overflow menu for the remaining snippets.
    let more_pop = gtk::Popover::new();
    let more_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let entries = [
        ("Bold", Snippet::Bold),
        ("Italic", Snippet::Italic),
        ("Inline Code", Snippet::Code),
        ("Code Block", Snippet::CodeBlock),
        ("Blockquote", Snippet::Blockquote),
        ("Image", Snippet::Image),
        ("Ordered List", Snippet::OrderedList),
    ];
    for (label, snippet) in entries {
        let b = gtk::Button::builder().label(label).has_frame(false).build();
        b.connect_clicked(glib::clone!(
            #[weak]
            buffer,
            #[weak]
            more_pop,
            move |_| {
                apply_snippet(&buffer, snippet);
                more_pop.popdown();
            }
        ));
        more_box.append(&b);
    }
    more_pop.set_child(Some(&more_box));
    let more_btn = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .tooltip_text("More")
        .popover(&more_pop)
        .build();
    more_btn.add_css_class("flat");
    bar.append(&more_btn);

    bar
}

/// A pill chip showing an emoji (or fallback icon) + text (Swift `ItemFormChip`).
fn context_chip(emoji: &str, fallback_icon: &str, text: &str) -> gtk::Box {
    let chip = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    chip.add_css_class("ql-item-chip");
    if emoji.is_empty() {
        chip.append(&gtk::Image::from_icon_name(fallback_icon));
    } else {
        chip.append(&gtk::Label::new(Some(emoji)));
    }
    let label = gtk::Label::builder().label(text).ellipsize(gtk::pango::EllipsizeMode::End).build();
    label.add_css_class("caption");
    chip.append(&label);
    chip
}

fn set_date_label(button: &gtk::MenuButton, calendar: &gtk::Calendar) {
    let d = calendar.date();
    button.set_label(&format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day_of_month()));
}

/// Make a 0–59/0–23 spin button show two digits.
fn two_digit(spin: &gtk::SpinButton) {
    spin.connect_output(|s| {
        s.set_text(&format!("{:02}", s.value() as i32));
        glib::Propagation::Stop
    });
}

/// Map the repeat combo selection (+ custom interval/unit) to a rule. `0` = Never.
fn rule_from_combo(sel: u32, interval: u32, unit_sel: u32) -> Option<ReminderRepeatRule> {
    use ReminderRepeatUnit::*;
    match sel {
        0 => None,
        1 => Some(ReminderRepeatRule::daily()),
        2 => Some(ReminderRepeatRule::weekly()),
        3 => Some(ReminderRepeatRule::biweekly()),
        4 => Some(ReminderRepeatRule::monthly()),
        5 => Some(ReminderRepeatRule::yearly()),
        6 => Some(ReminderRepeatRule::weekdays()),
        _ => {
            let unit = match unit_sel {
                0 => Day,
                1 => Week,
                2 => Month,
                3 => Year,
                _ => Weekdays,
            };
            let interval = if matches!(unit, Weekdays) { 1 } else { interval.max(1) };
            Some(ReminderRepeatRule { unit, interval, extra: Default::default() })
        }
    }
}

/// Map a rule to `(repeat index, unit index, interval)` for initialising the combos.
fn rule_to_combo(rule: &ReminderRepeatRule) -> (u32, u32, u32) {
    use ReminderRepeatUnit::*;
    let repeat_idx = match (&rule.unit, rule.interval) {
        (Day, 1) => 1,
        (Week, 1) => 2,
        (Week, 2) => 3,
        (Month, 1) => 4,
        (Year, 1) => 5,
        (Weekdays, _) => 6,
        _ => 7,
    };
    let unit_idx = match rule.unit {
        Day => 0,
        Week => 1,
        Month => 2,
        Year => 3,
        Weekdays => 4,
    };
    (repeat_idx, unit_idx, rule.interval)
}
