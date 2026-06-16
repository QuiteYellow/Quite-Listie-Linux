//! `ListSettings` — per-list configuration. GNOME counterpart of `qml/ListSettings.qml`
//! and Swift `ListSettingsView`: title, emoji icon, favourite, source readout, display
//! toggles (completed-at-bottom, map data), a background-gradient picker, and inline
//! label show/hide.
//!
//! Presented as an `adw::PreferencesDialog`. Most controls apply live against the
//! controller (icon/favourite/background/map/labels/display); the title is committed on
//! close. On close we call [`Controller::notify_list_settings_changed`] so the open list
//! page rebuilds (new title/icon/background) and the sidebar refreshes.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};

use quite_listie_core::model::background;

use crate::controller::Controller;

/// Present the list settings dialog for `list_id` over `parent`.
pub fn present(parent: &impl IsA<gtk::Widget>, controller: &Controller, list_id: &str) {
    let list_id = list_id.to_string();
    let dialog = adw::PreferencesDialog::builder()
        .title("List Settings")
        .build();
    let page = adw::PreferencesPage::new();

    // ── Details ────────────────────────────────────────────────────────────
    let details = adw::PreferencesGroup::builder().title("Details").build();

    let title_row = adw::EntryRow::builder().title("Title").build();
    title_row.set_text(&controller.list_name(&list_id));
    details.add(&title_row);

    // Emoji icon — MenuButton with an EmojiChooser + clear button (same pattern as
    // the label editor). Applied live via `set_list_emoji_icon`.
    let icon_row = adw::ActionRow::builder().title("Icon").build();
    let init_emoji = controller.list_emoji_icon(&list_id);
    let emoji_button = gtk::MenuButton::builder().valign(gtk::Align::Center).build();
    set_emoji_button_label(&emoji_button, &init_emoji);
    let chooser = gtk::EmojiChooser::new();
    emoji_button.set_popover(Some(&chooser));
    let clear_emoji = flat_icon_button("edit-clear-symbolic", "Remove emoji");
    clear_emoji.set_valign(gtk::Align::Center);
    clear_emoji.set_visible(!init_emoji.is_empty());
    chooser.connect_emoji_picked(glib::clone!(
        #[weak]
        emoji_button,
        #[weak]
        clear_emoji,
        #[weak]
        controller,
        #[strong]
        list_id,
        move |_, emoji| {
            set_emoji_button_label(&emoji_button, emoji);
            clear_emoji.set_visible(true);
            controller.set_list_emoji_icon(&list_id, emoji);
        }
    ));
    clear_emoji.connect_clicked(glib::clone!(
        #[weak]
        emoji_button,
        #[weak]
        controller,
        #[strong]
        list_id,
        move |btn| {
            set_emoji_button_label(&emoji_button, "");
            btn.set_visible(false);
            controller.set_list_emoji_icon(&list_id, "");
        }
    ));
    icon_row.add_suffix(&emoji_button);
    icon_row.add_suffix(&clear_emoji);
    details.add(&icon_row);

    let fav_row = adw::SwitchRow::builder()
        .title("Favourite")
        .active(controller.is_favourite(&list_id))
        .build();
    fav_row.connect_active_notify(glib::clone!(
        #[weak]
        controller,
        #[strong]
        list_id,
        move |row| controller.set_favourite(&list_id, row.is_active())
    ));
    details.add(&fav_row);

    let source = controller.list_source_description(&list_id);
    if !source.is_empty() {
        let source_row = adw::ActionRow::builder()
            .title("Source")
            .subtitle(&source)
            .css_classes(["property"])
            .build();
        source_row.add_css_class("dim-label");
        details.add(&source_row);
    }
    page.add(&details);

    // ── Display options ──────────────────────────────────────────────────
    let display = adw::PreferencesGroup::builder().title("Display Options").build();

    let completed_row = adw::SwitchRow::builder()
        .title("Show completed items at bottom")
        .active(controller.list_completed_at_bottom(&list_id))
        .build();
    completed_row.connect_active_notify(glib::clone!(
        #[weak]
        controller,
        #[strong]
        list_id,
        move |row| controller.set_list_completed_at_bottom(&list_id, row.is_active())
    ));
    display.add(&completed_row);

    let map_row = adw::SwitchRow::builder()
        .title("Show map view for this list")
        .active(controller.list_enable_map_data(&list_id))
        .build();
    map_row.connect_active_notify(glib::clone!(
        #[weak]
        controller,
        #[strong]
        list_id,
        move |row| controller.set_list_enable_map_data(&list_id, row.is_active())
    ));
    display.add(&map_row);
    page.add(&display);

    // ── Background gradient ────────────────────────────────────────────────
    let bg_group = adw::PreferencesGroup::builder().title("Background").build();
    let bg_row = adw::ActionRow::builder()
        .title("Background")
        .activatable(true)
        .build();
    let bg_preview = gradient_swatch(40, 24);
    bg_preview.set_valign(gtk::Align::Center);
    let clear_bg = flat_icon_button("edit-clear-symbolic", "Use default background");
    clear_bg.set_valign(gtk::Align::Center);
    bg_row.add_suffix(&bg_preview);
    bg_row.add_suffix(&clear_bg);
    bg_row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    bg_group.add(&bg_row);
    page.add(&bg_group);

    // Shared "currently selected gradient id" used by the row + picker.
    let current_bg = Rc::new(RefCell::new(controller.list_background(&list_id)));
    let refresh_bg_row = {
        let bg_row = bg_row.clone();
        let bg_preview = bg_preview.clone();
        let clear_bg = clear_bg.clone();
        let current_bg = current_bg.clone();
        Rc::new(move || {
            let id = current_bg.borrow().clone();
            match background::find(&id) {
                Some(g) => {
                    bg_row.set_subtitle(g.name);
                    set_swatch_gradient(&bg_preview, Some(g.id));
                    bg_preview.set_visible(true);
                    clear_bg.set_visible(true);
                }
                None => {
                    bg_row.set_subtitle("Default");
                    bg_preview.set_visible(false);
                    clear_bg.set_visible(false);
                }
            }
        })
    };
    refresh_bg_row();

    bg_row.connect_activated(glib::clone!(
        #[weak]
        controller,
        #[strong]
        list_id,
        #[strong]
        current_bg,
        #[strong]
        refresh_bg_row,
        #[weak]
        dialog,
        move |_| {
            present_gradient_picker(
                &dialog,
                &controller,
                &list_id,
                current_bg.clone(),
                refresh_bg_row.clone(),
            );
        }
    ));
    clear_bg.connect_clicked(glib::clone!(
        #[weak]
        controller,
        #[strong]
        list_id,
        #[strong]
        current_bg,
        #[strong]
        refresh_bg_row,
        move |_| {
            *current_bg.borrow_mut() = String::new();
            controller.set_list_background(&list_id, "");
            refresh_bg_row();
        }
    ));

    // ── Labels (manage + show/hide) ────────────────────────────────────────
    let labels_group = adw::PreferencesGroup::builder()
        .title("Show / Hide Labels")
        .build();
    let manage_row = adw::ActionRow::builder()
        .title("Manage Labels…")
        .activatable(true)
        .build();
    manage_row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    manage_row.connect_activated(glib::clone!(
        #[weak]
        controller,
        #[strong]
        list_id,
        move |row| crate::views::label_editor::present(row, &controller, &list_id)
    ));
    labels_group.add(&manage_row);

    let labels = controller.labels(&list_id);
    if labels.is_empty() {
        let empty = adw::ActionRow::builder()
            .title("No labels yet")
            .subtitle("Use Manage Labels… to add some.")
            .build();
        empty.add_css_class("dim-label");
        labels_group.add(&empty);
    } else {
        for label in labels {
            let row = adw::SwitchRow::builder()
                .title(&label.name)
                .active(!controller.is_label_hidden(&list_id, &label.id))
                .build();
            // Prefix: emoji glyph if set, else a coloured dot.
            let emoji = label.emoji_icon.clone().unwrap_or_default();
            if emoji.is_empty() {
                let dot = gtk::DrawingArea::new();
                dot.set_content_width(14);
                dot.set_content_height(14);
                dot.set_valign(gtk::Align::Center);
                let color = label.color.clone();
                dot.set_draw_func(move |_, cr, w, h| {
                    if let Ok(rgba) = gdk::RGBA::parse(&color) {
                        cr.set_source_rgb(
                            rgba.red() as f64,
                            rgba.green() as f64,
                            rgba.blue() as f64,
                        );
                    }
                    let r = (w.min(h) as f64) / 2.0;
                    cr.arc(w as f64 / 2.0, h as f64 / 2.0, r, 0.0, std::f64::consts::TAU);
                    let _ = cr.fill();
                });
                row.add_prefix(&dot);
            } else {
                row.add_prefix(&gtk::Label::new(Some(&emoji)));
            }
            let label_id = label.id.clone();
            row.connect_active_notify(glib::clone!(
                #[weak]
                controller,
                #[strong]
                list_id,
                move |sw| {
                    // active = "shown"; toggle flips presence in hidden_labels. Only
                    // act when the state actually disagrees with storage.
                    let shown = sw.is_active();
                    let hidden = controller.is_label_hidden(&list_id, &label_id);
                    if shown == hidden {
                        controller.toggle_label_hidden(&list_id, &label_id);
                    }
                }
            ));
            labels_group.add(&row);
        }
    }
    page.add(&labels_group);

    dialog.add(&page);

    // Commit the (possibly edited) title and refresh the open page on close.
    dialog.connect_closed(glib::clone!(
        #[weak]
        controller,
        #[strong]
        list_id,
        #[weak]
        title_row,
        move |_| {
            let new_name = title_row.text();
            let new_name = new_name.trim();
            if !new_name.is_empty() && new_name != controller.list_name(&list_id) {
                controller.rename_list(&list_id, new_name);
            }
            controller.notify_list_settings_changed();
        }
    ));

    dialog.present(Some(parent));
}

/// The gradient-picker sheet: a scrollable grid of swatches plus a "Default" entry.
/// Selecting one applies it live and updates the parent row.
fn present_gradient_picker(
    parent: &impl IsA<gtk::Widget>,
    controller: &Controller,
    list_id: &str,
    current_bg: Rc<RefCell<String>>,
    refresh_bg_row: Rc<dyn Fn()>,
) {
    let list_id = list_id.to_string();
    let picker = adw::Dialog::builder()
        .title("Choose Background")
        .content_width(560)
        .content_height(520)
        .build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());

    let flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .column_spacing(8)
        .row_spacing(8)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .min_children_per_line(2)
        .max_children_per_line(4)
        .build();

    // "Default" (no gradient) tile first.
    flow.append(&gradient_tile(
        None,
        "Default",
        current_bg.borrow().is_empty(),
        glib::clone!(
            #[weak]
            controller,
            #[weak]
            picker,
            #[strong]
            list_id,
            #[strong]
            current_bg,
            #[strong]
            refresh_bg_row,
            move || {
                *current_bg.borrow_mut() = String::new();
                controller.set_list_background(&list_id, "");
                refresh_bg_row();
                picker.close();
            }
        ),
    ));

    let selected = current_bg.borrow().clone();
    for g in background::ALL {
        flow.append(&gradient_tile(
            Some(g.id),
            g.name,
            selected == g.id,
            glib::clone!(
                #[weak]
                controller,
                #[weak]
                picker,
                #[strong]
                list_id,
                #[strong]
                current_bg,
                #[strong]
                refresh_bg_row,
                move || {
                    *current_bg.borrow_mut() = g.id.to_string();
                    controller.set_list_background(&list_id, g.id);
                    refresh_bg_row();
                    picker.close();
                }
            ),
        ));
    }

    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&flow)
        .build();
    toolbar.set_content(Some(&scroller));
    picker.set_child(Some(&toolbar));
    picker.present(Some(parent));
}

/// One selectable swatch tile (gradient preview + name), wrapped in a flat button.
fn gradient_tile(
    gradient_id: Option<&'static str>,
    name: &str,
    selected: bool,
    on_click: impl Fn() + 'static,
) -> gtk::FlowBoxChild {
    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let swatch = gradient_swatch(120, 56);
    set_swatch_gradient(&swatch, gradient_id);
    vbox.append(&swatch);
    let label = gtk::Label::builder()
        .label(name)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["caption"])
        .build();
    vbox.append(&label);

    let button = gtk::Button::builder().has_frame(false).child(&vbox).build();
    if selected {
        button.add_css_class("suggested-action");
    }
    button.connect_clicked(move |_| on_click());

    gtk::FlowBoxChild::builder().child(&button).build()
}

/// A rounded gradient swatch (for the settings row preview + picker tiles). The gradient
/// id is stored as widget data and re-read on each draw so it tracks the active scheme.
fn gradient_swatch(w: i32, h: i32) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(w);
    area.set_content_height(h);
    area.set_draw_func(|area, cr, w, h| draw_stored_gradient(area, cr, w as f64, h as f64, 8.0));
    area
}

/// A full-bleed (square-cornered, expanding) gradient background for the open list page.
/// Returns `None` unless `gradient_id` names a known gradient.
pub fn page_background(gradient_id: &str) -> Option<gtk::DrawingArea> {
    background::find(gradient_id)?;
    let area = gtk::DrawingArea::new();
    area.set_hexpand(true);
    area.set_vexpand(true);
    set_swatch_gradient(&area, Some(gradient_id));
    area.set_draw_func(|area, cr, w, h| draw_stored_gradient(area, cr, w as f64, h as f64, 0.0));
    Some(area)
}

/// Paint the gradient named by the widget's `ql-gradient-id` data, optionally clipped to
/// rounded corners. A no-op when the data is unset/unknown.
fn draw_stored_gradient(area: &gtk::DrawingArea, cr: &gtk::cairo::Context, w: f64, h: f64, radius: f64) {
    let id = unsafe {
        area.data::<String>("ql-gradient-id")
            .map(|p| p.as_ref().clone())
    };
    let Some(g) = id.as_deref().and_then(background::find) else {
        return;
    };
    let (from, to) = g.stops(adw::StyleManager::default().is_dark());
    if radius > 0.0 {
        rounded_rect(cr, 0.0, 0.0, w, h, radius);
        cr.clip();
    }
    let grad = gtk::cairo::LinearGradient::new(0.0, 0.0, w, h);
    if let Ok(c) = gdk::RGBA::parse(from) {
        grad.add_color_stop_rgb(0.0, c.red() as f64, c.green() as f64, c.blue() as f64);
    }
    if let Ok(c) = gdk::RGBA::parse(to) {
        grad.add_color_stop_rgb(1.0, c.red() as f64, c.green() as f64, c.blue() as f64);
    }
    let _ = cr.set_source(&grad);
    let _ = cr.paint();
}

/// Point a [`gradient_swatch`] at a gradient id (or clear it) and repaint.
fn set_swatch_gradient(area: &gtk::DrawingArea, gradient_id: Option<&str>) {
    unsafe {
        match gradient_id {
            Some(id) => area.set_data("ql-gradient-id", id.to_string()),
            None => {
                let _ = area.steal_data::<String>("ql-gradient-id");
            }
        }
    }
    area.queue_draw();
}

fn rounded_rect(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::FRAC_PI_2;
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, std::f64::consts::PI);
    cr.arc(x + r, y + r, r, std::f64::consts::PI, 3.0 * FRAC_PI_2);
    cr.close_path();
}

fn flat_icon_button(icon: &str, tooltip: &str) -> gtk::Button {
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
