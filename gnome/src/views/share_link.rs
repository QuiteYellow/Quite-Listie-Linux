//! `ShareLink` — generate a `quitelistie://import` deeplink for a list's items. GNOME
//! counterpart of Swift `ShareLinkSheet` (share mode): Compress / Comments toggles, an
//! All/Active/None item picker grouped by label, a live character count with long-URL
//! warnings, Copy Link, and a selectable URL preview. The markdown + encoding is produced
//! by the controller (`build_share_url` -> core `generate_markdown_export_items` +
//! `build_import_url`).
//!
//! Deviation: the preset modes (save/edit `SharePreset`) and the native share sheet aren't
//! ported — there's no `SharePreset`/ManagePresets in the GNOME port yet, and the desktop
//! flow is copy-to-clipboard.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::controller::Controller;

const WARN_CHARS: usize = 2000;
const ERROR_CHARS: usize = 4000;

pub fn present(anchor: &impl IsA<gtk::Widget>, controller: &Controller, list_id: &str) {
    let items = controller.share_items(list_id);

    let dialog = adw::Dialog::builder()
        .title("Share as Link")
        .content_width(520)
        .content_height(640)
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

    // Intro.
    let intro = gtk::Label::builder()
        .label("Anyone with this link can import a copy of these items into Quite Listie.")
        .xalign(0.0)
        .wrap(true)
        .build();
    intro.add_css_class("dim-label");
    body.append(&intro);

    // Options.
    let compress = adw::SwitchRow::builder().title("Compress").active(true).build();
    let comments = adw::SwitchRow::builder().title("Comments").subtitle("Include item notes").active(false).build();
    let opts = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
    opts.add_css_class("boxed-list");
    opts.append(&compress);
    opts.append(&comments);
    body.append(&opts);

    // Selection state, seeded to the active (unchecked) items (Swift default).
    let selected: Rc<RefCell<HashSet<String>>> =
        Rc::new(RefCell::new(items.iter().filter(|i| !i.checked).map(|i| i.id.clone()).collect()));

    // Picker controls: All / Active / None + counter.
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let all_btn = gtk::Button::with_label("All");
    let active_btn = gtk::Button::with_label("Active");
    let none_btn = gtk::Button::with_label("None");
    for b in [&all_btn, &active_btn, &none_btn] {
        b.add_css_class("pill");
    }
    let counter = gtk::Label::new(None);
    counter.add_css_class("dim-label");
    counter.set_hexpand(true);
    counter.set_halign(gtk::Align::End);
    controls.append(&all_btn);
    controls.append(&active_btn);
    controls.append(&none_btn);
    controls.append(&counter);
    body.append(&controls);

    // Item rows grouped by label. Keep (id, check, item_checked) to drive bulk buttons.
    let checks: Rc<Vec<(String, gtk::CheckButton, bool)>> = {
        let mut v = Vec::new();
        let list_box_holder = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let mut current_label: Option<String> = None;
        let mut current_box: Option<gtk::ListBox> = None;
        for item in &items {
            if current_label.as_deref() != Some(item.label_name.as_str()) {
                let header = gtk::Label::builder().label(&item.label_name).xalign(0.0).build();
                header.add_css_class("heading");
                header.set_margin_top(4);
                list_box_holder.append(&header);
                let lb = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
                lb.add_css_class("boxed-list");
                list_box_holder.append(&lb);
                current_box = Some(lb);
                current_label = Some(item.label_name.clone());
            }
            let title = glib::markup_escape_text(&item.note);
            let title = if item.checked { format!("<s>{title}</s>") } else { title.to_string() };
            let row = adw::ActionRow::builder().title(&title).build();
            if item.checked {
                row.add_css_class("dim-label");
            }
            let check = gtk::CheckButton::builder()
                .active(selected.borrow().contains(&item.id))
                .valign(gtk::Align::Center)
                .build();
            row.add_prefix(&check);
            if item.quantity > 1.0 {
                let badge = gtk::Label::new(Some(&format!("×{}", item.quantity as i64)));
                badge.add_css_class("ql-quantity-badge");
                badge.set_valign(gtk::Align::Center);
                row.add_suffix(&badge);
            }
            row.set_activatable_widget(Some(&check));
            current_box.as_ref().unwrap().append(&row);
            v.push((item.id.clone(), check, item.checked));
        }
        if items.is_empty() {
            let empty = gtk::Label::builder().label("This list has no items to share.").xalign(0.0).build();
            empty.add_css_class("dim-label");
            list_box_holder.append(&empty);
        }
        body.append(&list_box_holder);
        Rc::new(v)
    };

    // Details + warning + URL preview.
    let details = gtk::Label::builder().xalign(0.0).wrap(true).build();
    let char_count = gtk::Label::new(None);
    char_count.set_halign(gtk::Align::End);
    let detail_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    details.set_hexpand(true);
    detail_row.append(&details);
    detail_row.append(&char_count);
    body.append(&detail_row);

    let warning = gtk::Label::builder().xalign(0.0).wrap(true).visible(false).build();
    warning.add_css_class("caption");
    body.append(&warning);

    let copy_btn = gtk::Button::with_label("Copy Link");
    copy_btn.add_css_class("suggested-action");
    copy_btn.set_halign(gtk::Align::Start);
    body.append(&copy_btn);

    let preview = gtk::Label::builder().xalign(0.0).wrap(true).selectable(true).build();
    preview.add_css_class("caption");
    preview.add_css_class("monospace");
    body.append(&preview);

    // Holds the latest URL for Copy.
    let current_url: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));

    let refresh: Rc<dyn Fn()> = {
        let controller = controller.clone();
        let list_id = list_id.to_string();
        let selected = selected.clone();
        let compress = compress.clone();
        let comments = comments.clone();
        let counter = counter.clone();
        let details = details.clone();
        let char_count = char_count.clone();
        let warning = warning.clone();
        let preview = preview.clone();
        let copy_btn = copy_btn.clone();
        let current_url = current_url.clone();
        let total = items.len();
        Rc::new(move || {
            let ids: Vec<String> = selected.borrow().iter().cloned().collect();
            counter.set_text(&format!("{} / {}", ids.len(), total));
            let url = controller.build_share_url(&list_id, &ids, comments.is_active(), compress.is_active());
            let n = url.chars().count();
            *current_url.borrow_mut() = url.clone();
            copy_btn.set_sensitive(!ids.is_empty());

            details.set_text(&format!(
                "{} item{}{}",
                ids.len(),
                if ids.len() == 1 { "" } else { "s" },
                if comments.is_active() { " with comments" } else { "" }
            ));
            char_count.set_text(&format!("{n} characters"));

            for c in ["dim-label", "warning", "error"] {
                char_count.remove_css_class(c);
            }
            if n >= ERROR_CHARS {
                char_count.add_css_class("error");
                warning.set_label("URL is very long — over 4,000 characters may not work on all platforms. Try compression, drop comments, or deselect items.");
                warning.set_visible(true);
                warning.remove_css_class("warning");
                warning.add_css_class("error");
            } else if n >= WARN_CHARS {
                char_count.add_css_class("warning");
                warning.set_label("URL is getting long — over 2,000 characters may be truncated by some apps. Consider compression or deselecting items.");
                warning.set_visible(true);
                warning.remove_css_class("error");
                warning.add_css_class("warning");
            } else {
                char_count.add_css_class("dim-label");
                warning.set_visible(false);
            }

            preview.set_text(if url.is_empty() { "No URL — no items selected." } else { &url });
        })
    };
    refresh();

    // Per-item toggles update the selection set.
    for (id, check, _) in checks.iter() {
        check.connect_toggled(glib::clone!(
            #[strong]
            selected,
            #[strong(rename_to = id)]
            id.clone(),
            #[strong]
            refresh,
            move |c| {
                if c.is_active() {
                    selected.borrow_mut().insert(id.clone());
                } else {
                    selected.borrow_mut().remove(&id);
                }
                refresh();
            }
        ));
    }

    // Bulk buttons set the selection + sync the checkboxes (which fire refresh).
    let bulk = {
        let selected = selected.clone();
        let checks = checks.clone();
        let refresh = refresh.clone();
        move |which: Bulk| {
            {
                let mut sel = selected.borrow_mut();
                sel.clear();
                for (id, _, checked) in checks.iter() {
                    let want = match which {
                        Bulk::All => true,
                        Bulk::Active => !checked,
                        Bulk::None => false,
                    };
                    if want {
                        sel.insert(id.clone());
                    }
                }
            }
            // Sync without triggering N nested refreshes: set active, then one refresh.
            for (id, check, _) in checks.iter() {
                check.set_active(selected.borrow().contains(id));
            }
            refresh();
        }
    };
    all_btn.connect_clicked(glib::clone!(
        #[strong]
        bulk,
        move |_| bulk(Bulk::All)
    ));
    active_btn.connect_clicked(glib::clone!(
        #[strong]
        bulk,
        move |_| bulk(Bulk::Active)
    ));
    none_btn.connect_clicked(glib::clone!(
        #[strong]
        bulk,
        move |_| bulk(Bulk::None)
    ));

    compress.connect_active_notify(glib::clone!(
        #[strong]
        refresh,
        move |_| refresh()
    ));
    comments.connect_active_notify(glib::clone!(
        #[strong]
        refresh,
        move |_| refresh()
    ));

    copy_btn.connect_clicked(glib::clone!(
        #[strong]
        current_url,
        move |btn| {
            let url = current_url.borrow().clone();
            if !url.is_empty() {
                btn.clipboard().set_text(&url);
                btn.set_label("Copied!");
                let btn = btn.clone();
                glib::timeout_add_seconds_local_once(2, move || btn.set_label("Copy Link"));
            }
        }
    ));

    dialog.set_child(Some(&toolbar));
    dialog.present(Some(anchor));
}

#[derive(Clone, Copy)]
enum Bulk {
    All,
    Active,
    None,
}
