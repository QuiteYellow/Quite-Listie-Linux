//! `MarkdownExport` — export the open list as a Markdown checklist. GNOME counterpart of
//! Swift `MarkdownExportView`: an `adw::Dialog` with "Include completed" / "Include notes"
//! toggles, a Raw/Preview switch, a warnings banner for notes that can't round-trip, and
//! Copy / Save-to-file actions. The markdown itself is produced by the controller
//! (`export_markdown` -> core `generate_markdown_export`).

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::controller::Controller;

pub fn present(anchor: &impl IsA<gtk::Widget>, controller: &Controller, list_id: &str) {
    let dialog = adw::Dialog::builder()
        .title("Export Markdown")
        .content_width(560)
        .content_height(640)
        .build();

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();

    // Raw/Preview toggle (start, next to the auto close button).
    let raw_toggle = gtk::ToggleButton::builder()
        .icon_name("view-paged-symbolic")
        .tooltip_text("Show raw markdown")
        .build();
    header.pack_start(&raw_toggle);

    // Copy + Save (end).
    let copy_btn = gtk::Button::builder().icon_name("edit-copy-symbolic").tooltip_text("Copy").build();
    let save_btn = gtk::Button::builder().icon_name("document-save-symbolic").tooltip_text("Save to file…").build();
    header.pack_end(&save_btn);
    header.pack_end(&copy_btn);
    toolbar.add_top_bar(&header);

    // --- options row -------------------------------------------------------
    let include_completed = adw::SwitchRow::builder().title("Include completed").active(true).build();
    let include_notes = adw::SwitchRow::builder().title("Include notes").active(false).build();
    let options = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
    options.add_css_class("boxed-list");
    options.append(&include_completed);
    options.append(&include_notes);

    // --- content (raw text view + rendered preview, one visible at a time) --
    let raw_view = gtk::TextView::builder()
        .editable(false)
        .monospace(true)
        .wrap_mode(gtk::WrapMode::WordChar)
        .left_margin(8)
        .right_margin(8)
        .top_margin(8)
        .bottom_margin(8)
        .build();
    let preview_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let content_stack = gtk::Stack::new();
    content_stack.add_named(&preview_box, Some("preview"));
    content_stack.add_named(&raw_view, Some("raw"));
    let content_scroller = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&content_stack)
        .build();

    // --- warnings banner ---------------------------------------------------
    let warnings_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .margin_top(6)
        .margin_bottom(6)
        .visible(false)
        .build();

    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_start(12)
        .margin_end(12)
        .margin_top(12)
        .margin_bottom(12)
        .build();
    body.append(&options);
    body.append(&warnings_box);
    body.append(&content_scroller);
    toolbar.set_content(Some(&body));

    // Holds the latest rendered markdown for Copy / Save.
    let current_md: Rc<std::cell::RefCell<String>> = Rc::new(std::cell::RefCell::new(String::new()));

    let regenerate: Rc<dyn Fn()> = {
        let controller = controller.clone();
        let list_id = list_id.to_string();
        let include_completed = include_completed.clone();
        let include_notes = include_notes.clone();
        let raw_view = raw_view.clone();
        let preview_box = preview_box.clone();
        let warnings_box = warnings_box.clone();
        let current_md = current_md.clone();
        Rc::new(move || {
            let res = controller.export_markdown(&list_id, !include_completed.is_active(), include_notes.is_active());

            raw_view.buffer().set_text(&res.markdown);
            *current_md.borrow_mut() = res.markdown.clone();

            // Rebuild the rendered preview from a throwaway buffer (reuses the notes renderer).
            while let Some(child) = preview_box.first_child() {
                preview_box.remove(&child);
            }
            let buf = gtk::TextBuffer::new(None);
            buf.set_text(&res.markdown);
            preview_box.append(&crate::widgets::markdown::checkable_preview(&buf));

            // Warnings banner.
            while let Some(child) = warnings_box.first_child() {
                warnings_box.remove(&child);
            }
            if res.warnings.is_empty() {
                warnings_box.set_visible(false);
            } else {
                let head = gtk::Label::builder()
                    .label(format!(
                        "{} item(s) had notes that couldn't be exported",
                        res.warnings.len()
                    ))
                    .xalign(0.0)
                    .wrap(true)
                    .build();
                head.add_css_class("warning");
                head.add_css_class("caption-heading");
                warnings_box.append(&head);
                for w in &res.warnings {
                    let l = gtk::Label::builder().label(w).xalign(0.0).wrap(true).build();
                    l.add_css_class("dim-label");
                    l.add_css_class("caption");
                    warnings_box.append(&l);
                }
                warnings_box.set_visible(true);
            }
        })
    };
    regenerate();

    include_completed.connect_active_notify(glib::clone!(
        #[strong]
        regenerate,
        move |_| regenerate()
    ));
    include_notes.connect_active_notify(glib::clone!(
        #[strong]
        regenerate,
        move |_| regenerate()
    ));

    raw_toggle.connect_toggled(glib::clone!(
        #[weak]
        content_stack,
        move |t| {
            content_stack.set_visible_child_name(if t.is_active() { "raw" } else { "preview" });
            t.set_tooltip_text(Some(if t.is_active() { "Show preview" } else { "Show raw markdown" }));
        }
    ));

    copy_btn.connect_clicked(glib::clone!(
        #[weak]
        dialog,
        #[strong]
        current_md,
        move |btn| {
            btn.clipboard().set_text(&current_md.borrow());
            // Brief confirmation via the dialog title.
            dialog.set_title("Copied to clipboard");
            let dialog = dialog.clone();
            glib::timeout_add_seconds_local_once(2, move || dialog.set_title("Export Markdown"));
        }
    ));

    let list_name = controller.list_name(list_id);
    save_btn.connect_clicked(glib::clone!(
        #[weak]
        dialog,
        #[strong]
        current_md,
        #[strong]
        list_name,
        move |btn| {
            save_to_file(&dialog, btn, &list_name, current_md.borrow().clone());
        }
    ));

    dialog.set_child(Some(&toolbar));
    dialog.present(Some(anchor));
}

/// Sanitise a list name for use as a filename (Swift `sanitizeFilename`).
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| if ":/\\?%*|\"<>".contains(c) { '-' } else { c })
        .collect()
}

fn save_to_file(dialog: &adw::Dialog, anchor: &impl IsA<gtk::Widget>, list_name: &str, md: String) {
    let file_dialog = gtk::FileDialog::builder()
        .title("Save Markdown")
        .initial_name(format!("{}.md", sanitize_filename(list_name)))
        .modal(true)
        .build();
    let root = anchor.root().and_downcast::<gtk::Window>();
    file_dialog.save(
        root.as_ref(),
        gio::Cancellable::NONE,
        glib::clone!(
            #[weak]
            dialog,
            move |res| {
                if let Ok(file) = res {
                    if let Err(e) = std::fs::write(file.path().unwrap_or_default(), md.as_bytes()) {
                        dialog.set_title(&format!("Save failed: {e}"));
                    }
                }
            }
        ),
    );
}
