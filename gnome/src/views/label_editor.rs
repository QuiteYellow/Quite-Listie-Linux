//! `LabelEditor` — manage the labels on a list. GNOME counterpart of
//! `qml/LabelEditor.qml`. An `adw::Dialog` listing the list's labels (colour swatch or
//! emoji, name) with reorder / edit / delete controls, plus "Add Label" and "Grocery
//! Presets" actions. Editing a label opens a sub-dialog with a name field, a native
//! colour picker (`gtk::ColorDialogButton`), and an emoji chooser.
//!
//! All operations are synchronous against the controller (which mutates the cached
//! document and autosaves); the list is rebuilt from `Controller::labels` after each.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};

use crate::controller::Controller;

const DEFAULT_COLOR: &str = "#4CAF50";

/// Present the label editor for `list_id` over `parent`.
pub fn present(parent: &impl IsA<gtk::Widget>, controller: &Controller, list_id: &str) {
    let list_id = list_id.to_string();

    let dialog = adw::Dialog::builder()
        .title("Labels")
        .content_width(420)
        .content_height(560)
        .build();

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();

    let add_button = gtk::Button::from_icon_name("list-add-symbolic");
    add_button.set_tooltip_text(Some("Add Label"));
    header.pack_end(&add_button);

    // Overflow: grocery presets.
    let menu_button = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .tooltip_text("More")
        .build();
    let menu_pop = gtk::Popover::new();
    let presets_button = gtk::Button::builder()
        .label("Add Grocery Presets")
        .has_frame(false)
        .halign(gtk::Align::Fill)
        .build();
    menu_pop.set_child(Some(&presets_button));
    menu_button.set_popover(Some(&menu_pop));
    header.pack_end(&menu_button);
    toolbar.add_top_bar(&header);

    let stack = gtk::Stack::new();
    let empty = adw::StatusPage::builder()
        .icon_name("tag-symbolic")
        .title("No labels")
        .description("Add a label, or apply the grocery presets.")
        .build();
    stack.add_named(&empty, Some("empty"));

    let list_box = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
    list_box.add_css_class("boxed-list");
    let clamp = adw::Clamp::builder().child(&list_box).margin_top(12).margin_bottom(12).build();
    let scroller = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&clamp)
        .build();
    stack.add_named(&scroller, Some("list"));
    toolbar.set_content(Some(&stack));
    dialog.set_child(Some(&toolbar));

    // Rebuild the list from the current label set. Defined as an Rc so the row buttons
    // (and the add/edit/delete/preset handlers) can all trigger a refresh.
    let repopulate: Rc<dyn Fn()> = {
        let list_box = list_box.clone();
        let stack = stack.clone();
        let controller = controller.clone();
        let list_id = list_id.clone();
        let dialog = dialog.clone();
        // Late-bound self-reference so a row handler can request another rebuild.
        let slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let slot2 = slot.clone();
        let f: Rc<dyn Fn()> = Rc::new(move || {
            while let Some(child) = list_box.first_child() {
                list_box.remove(&child);
            }
            let labels = controller.labels(&list_id);
            if labels.is_empty() {
                stack.set_visible_child_name("empty");
                return;
            }
            let count = labels.len();
            let refresh = slot2.borrow().clone();
            for (i, label) in labels.into_iter().enumerate() {
                let row = adw::ActionRow::builder()
                    .title(glib::markup_escape_text(&label.name))
                    .build();

                // Prefix: emoji glyph if set, else a coloured bullet.
                let emoji = label.emoji_icon.clone().unwrap_or_default();
                let prefix = gtk::Label::new(None);
                prefix.set_valign(gtk::Align::Center);
                prefix.set_width_chars(2);
                if emoji.is_empty() {
                    prefix.set_markup(&format!(
                        "<span foreground=\"{}\" size=\"x-large\">●</span>",
                        sanitize_hex(&label.color)
                    ));
                } else {
                    prefix.set_text(&emoji);
                }
                row.add_prefix(&prefix);

                let up = icon_button("go-up-symbolic", "Move up");
                up.set_sensitive(i > 0);
                let down = icon_button("go-down-symbolic", "Move down");
                down.set_sensitive(i + 1 < count);
                let edit = icon_button("document-edit-symbolic", "Edit");
                let delete = icon_button("user-trash-symbolic", "Delete");
                delete.add_css_class("destructive-action");
                row.add_suffix(&up);
                row.add_suffix(&down);
                row.add_suffix(&edit);
                row.add_suffix(&delete);

                let label_id = label.id.clone();
                let label_for_edit = label.clone();

                up.connect_clicked(glib::clone!(
                    #[weak]
                    controller,
                    #[strong]
                    list_id,
                    #[strong]
                    refresh,
                    move |_| {
                        controller.move_label(&list_id, i, i - 1);
                        if let Some(r) = &refresh { r() }
                    }
                ));
                down.connect_clicked(glib::clone!(
                    #[weak]
                    controller,
                    #[strong]
                    list_id,
                    #[strong]
                    refresh,
                    move |_| {
                        controller.move_label(&list_id, i, i + 1);
                        if let Some(r) = &refresh { r() }
                    }
                ));
                delete.connect_clicked(glib::clone!(
                    #[weak]
                    controller,
                    #[strong]
                    list_id,
                    #[strong]
                    refresh,
                    move |_| {
                        controller.delete_label(&list_id, &label_id);
                        if let Some(r) = &refresh { r() }
                    }
                ));
                edit.connect_clicked(glib::clone!(
                    #[weak]
                    controller,
                    #[weak]
                    dialog,
                    #[strong]
                    list_id,
                    #[strong]
                    refresh,
                    move |_| {
                        let on_done = refresh.clone();
                        present_label_edit(
                            &dialog,
                            &controller,
                            &list_id,
                            Some(label_for_edit.clone()),
                            move || {
                                if let Some(r) = &on_done {
                                    r()
                                }
                            },
                        );
                    }
                ));

                list_box.append(&row);
            }
            stack.set_visible_child_name("list");
        });
        *slot.borrow_mut() = Some(f.clone());
        f
    };

    add_button.connect_clicked(glib::clone!(
        #[weak]
        controller,
        #[weak]
        dialog,
        #[strong]
        list_id,
        #[strong]
        repopulate,
        move |_| {
            let on_done = repopulate.clone();
            present_label_edit(&dialog, &controller, &list_id, None, move || on_done());
        }
    ));

    presets_button.connect_clicked(glib::clone!(
        #[weak]
        controller,
        #[weak]
        menu_pop,
        #[strong]
        list_id,
        #[strong]
        repopulate,
        move |_| {
            menu_pop.popdown();
            controller.apply_grocery_presets(&list_id);
            repopulate();
        }
    ));

    repopulate();
    dialog.present(Some(parent));
}

/// The add/edit label sub-dialog. `existing` is `None` for a new label.
fn present_label_edit(
    parent: &impl IsA<gtk::Widget>,
    controller: &Controller,
    list_id: &str,
    existing: Option<quite_listie_core::model::ListLabel>,
    on_done: impl Fn() + 'static,
) {
    let list_id = list_id.to_string();
    let editing_id = existing.as_ref().map(|l| l.id.clone());
    let init_name = existing.as_ref().map(|l| l.name.clone()).unwrap_or_default();
    let init_color = existing
        .as_ref()
        .map(|l| l.color.clone())
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| DEFAULT_COLOR.to_string());
    let init_emoji = existing.as_ref().and_then(|l| l.emoji_icon.clone()).unwrap_or_default();

    let dialog = adw::Dialog::builder()
        .title(if editing_id.is_some() { "Edit Label" } else { "New Label" })
        .content_width(380)
        .build();
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    let save_button = gtk::Button::with_label("Save");
    save_button.add_css_class("suggested-action");
    header.pack_end(&save_button);
    toolbar.add_top_bar(&header);

    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();

    let name_row = adw::EntryRow::builder().title("Name").text(&init_name).build();
    group.add(&name_row);

    // Colour picker.
    let color_row = adw::ActionRow::builder().title("Colour").build();
    let color_button = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::new()));
    color_button.set_valign(gtk::Align::Center);
    if let Ok(rgba) = gdk::RGBA::parse(&init_color) {
        color_button.set_rgba(&rgba);
    }
    color_row.add_suffix(&color_button);
    group.add(&color_row);

    // Emoji chooser.
    let emoji_value = Rc::new(RefCell::new(init_emoji.clone()));
    let emoji_row = adw::ActionRow::builder().title("Emoji").build();
    let emoji_button = gtk::MenuButton::builder().valign(gtk::Align::Center).build();
    set_emoji_button_label(&emoji_button, &init_emoji);
    let chooser = gtk::EmojiChooser::new();
    emoji_button.set_popover(Some(&chooser));
    let clear_emoji = icon_button("edit-clear-symbolic", "Remove emoji");
    clear_emoji.set_valign(gtk::Align::Center);
    clear_emoji.set_visible(!init_emoji.is_empty());

    chooser.connect_emoji_picked(glib::clone!(
        #[weak]
        emoji_button,
        #[weak]
        clear_emoji,
        #[strong]
        emoji_value,
        move |_, emoji| {
            *emoji_value.borrow_mut() = emoji.to_string();
            set_emoji_button_label(&emoji_button, emoji);
            clear_emoji.set_visible(true);
        }
    ));
    clear_emoji.connect_clicked(glib::clone!(
        #[weak]
        emoji_button,
        #[strong]
        emoji_value,
        move |btn| {
            emoji_value.borrow_mut().clear();
            set_emoji_button_label(&emoji_button, "");
            btn.set_visible(false);
        }
    ));
    emoji_row.add_suffix(&emoji_button);
    emoji_row.add_suffix(&clear_emoji);
    group.add(&emoji_row);

    page.add(&group);
    toolbar.set_content(Some(&page));
    dialog.set_child(Some(&toolbar));

    save_button.connect_clicked(glib::clone!(
        #[weak]
        controller,
        #[weak]
        dialog,
        #[weak]
        name_row,
        #[weak]
        color_button,
        #[strong]
        emoji_value,
        #[strong]
        list_id,
        move |_| {
            let name = name_row.text();
            let name = name.trim();
            if name.is_empty() {
                return;
            }
            let color = rgba_to_hex(&color_button.rgba());
            let emoji = emoji_value.borrow().clone();
            match &editing_id {
                Some(id) => controller.update_label(&list_id, id, name, &color, &emoji),
                None => controller.add_label(&list_id, name, &color, &emoji),
            }
            on_done();
            dialog.close();
        }
    ));

    name_row.connect_entry_activated(glib::clone!(
        #[weak]
        save_button,
        move |_| save_button.emit_clicked()
    ));

    dialog.present(Some(parent));
}

fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.set_tooltip_text(Some(tooltip));
    b.add_css_class("flat");
    b
}

fn set_emoji_button_label(button: &gtk::MenuButton, emoji: &str) {
    if emoji.is_empty() {
        button.set_label("Choose…");
    } else {
        button.set_label(emoji);
    }
}

fn rgba_to_hex(c: &gdk::RGBA) -> String {
    format!(
        "#{:02X}{:02X}{:02X}",
        (c.red() * 255.0).round() as u8,
        (c.green() * 255.0).round() as u8,
        (c.blue() * 255.0).round() as u8,
    )
}

/// Only let a `#rrggbb`/`#rgb` hex value through to Pango markup.
fn sanitize_hex(hex: &str) -> String {
    if hex.starts_with('#') && hex.len() <= 9 && hex[1..].chars().all(|c| c.is_ascii_hexdigit()) {
        hex.to_string()
    } else {
        "gray".to_string()
    }
}
