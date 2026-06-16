//! `MarkdownImport` — import a pasted/opened Markdown checklist into an existing list.
//! GNOME counterpart of Swift `MarkdownListImportView` (paste intent): pick a target list,
//! paste or open a `.md` file, choose merge options, see a live merge summary, and import.
//! Parsing + merge logic live in core (`markdown_parser` / `markdown_import`); the
//! controller's `import_markdown` applies it.
//!
//! Deviations from Swift: no per-item selection or per-item diff lines (all parsed items
//! are imported); the preset-reload (`.preset`) intent isn't wired here. The `quitelistie://import`
//! deeplink routes here pre-filled via [`present_prefilled`]. Target = any open list.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::controller::Controller;

pub fn present(window: &impl IsA<gtk::Widget>, controller: &Controller) {
    present_prefilled(window, controller, "", "");
}

/// Open the import dialog pre-filled from a `quitelistie://import` deeplink.
/// `markdown` seeds the paste area; `preselect_id` (a runtime list id, may be
/// empty) preselects the target combo when it matches an open list.
pub fn present_prefilled(
    window: &impl IsA<gtk::Widget>,
    controller: &Controller,
    markdown: &str,
    preselect_id: &str,
) {
    let lists = controller.importable_lists();

    let dialog = adw::Dialog::builder()
        .title("Import Markdown")
        .content_width(560)
        .content_height(620)
        .build();

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    let import_btn = gtk::Button::builder().label("Import").sensitive(false).build();
    import_btn.add_css_class("suggested-action");
    header.pack_end(&import_btn);
    toolbar.add_top_bar(&header);

    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_start(12)
        .margin_end(12)
        .margin_top(12)
        .margin_bottom(12)
        .build();

    // No open lists -> nothing to import into.
    if lists.is_empty() {
        let status = adw::StatusPage::builder()
            .icon_name("document-edit-symbolic")
            .title("No Lists Open")
            .description("Open or connect a list first, then import Markdown into it.")
            .vexpand(true)
            .build();
        toolbar.set_content(Some(&status));
        dialog.set_child(Some(&toolbar));
        dialog.present(Some(window));
        return;
    }

    // --- target + options --------------------------------------------------
    let names: Vec<&str> = lists.iter().map(|(_, n)| n.as_str()).collect();
    let target_row = adw::ComboRow::builder()
        .title("Import into")
        .model(&gtk::StringList::new(&names))
        .build();
    let replace_row = adw::SwitchRow::builder()
        .title("Replace quantities")
        .subtitle("Off: add to the existing quantity")
        .active(false)
        .build();
    let create_labels_row = adw::SwitchRow::builder()
        .title("Create missing labels")
        .active(true)
        .build();
    let options = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
    options.add_css_class("boxed-list");
    options.append(&target_row);
    options.append(&replace_row);
    options.append(&create_labels_row);
    body.append(&options);

    // --- paste area + open-file -------------------------------------------
    let open_file_btn = gtk::Button::builder()
        .label("Open .md File…")
        .halign(gtk::Align::Start)
        .build();
    body.append(&open_file_btn);

    let text_view = gtk::TextView::builder()
        .monospace(true)
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(8)
        .right_margin(8)
        .build();
    let text_scroller = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .min_content_height(180)
        .child(&text_view)
        .build();
    text_scroller.add_css_class("card");
    body.append(&text_scroller);

    // --- merge summary -----------------------------------------------------
    let summary = gtk::Label::builder().xalign(0.0).wrap(true).build();
    summary.add_css_class("dim-label");
    body.append(&summary);

    toolbar.set_content(Some(&body));

    let buffer = text_view.buffer();

    // Deeplink prefill: seed the paste area and preselect the matching list.
    if !markdown.is_empty() {
        buffer.set_text(markdown);
    }
    if !preselect_id.is_empty() {
        if let Some(idx) = lists.iter().position(|(id, _)| id == preselect_id) {
            target_row.set_selected(idx as u32);
        }
    }

    let refresh: Rc<dyn Fn()> = {
        let controller = controller.clone();
        let lists = Rc::new(lists);
        let target_row = target_row.clone();
        let create_labels_row = create_labels_row.clone();
        let buffer = buffer.clone();
        let summary = summary.clone();
        let import_btn = import_btn.clone();
        Rc::new(move || {
            let md = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false).to_string();
            let has_text = !md.trim().is_empty();
            import_btn.set_sensitive(has_text);
            if !has_text {
                summary.set_text("Paste a Markdown checklist, or open a .md file.");
                return;
            }
            let idx = target_row.selected() as usize;
            let Some((id, _)) = lists.get(idx) else { return };
            let stats = controller.import_preview(id, &md, create_labels_row.is_active());
            summary.set_text(&format!(
                "{} new item{}, {} updated, {} new label{}",
                stats.new_items,
                if stats.new_items == 1 { "" } else { "s" },
                stats.updated_items,
                stats.new_labels,
                if stats.new_labels == 1 { "" } else { "s" },
            ));
        })
    };
    refresh();

    buffer.connect_changed(glib::clone!(
        #[strong]
        refresh,
        move |_| refresh()
    ));
    target_row.connect_selected_notify(glib::clone!(
        #[strong]
        refresh,
        move |_| refresh()
    ));
    create_labels_row.connect_active_notify(glib::clone!(
        #[strong]
        refresh,
        move |_| refresh()
    ));

    open_file_btn.connect_clicked(glib::clone!(
        #[weak]
        buffer,
        #[weak]
        dialog,
        move |btn| open_md_file(&dialog, btn, &buffer)
    ));

    let lists = Rc::new(controller.importable_lists());
    import_btn.connect_clicked(glib::clone!(
        #[weak]
        dialog,
        #[weak]
        controller,
        #[weak]
        buffer,
        #[weak]
        target_row,
        #[weak]
        replace_row,
        #[weak]
        create_labels_row,
        #[strong]
        lists,
        move |_| {
            let md = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false).to_string();
            if md.trim().is_empty() {
                return;
            }
            let Some((id, name)) = lists.get(target_row.selected() as usize) else { return };
            let (new, updated, labels) =
                controller.import_markdown(id, &md, replace_row.is_active(), create_labels_row.is_active());
            controller.show_toast(&format!(
                "Imported into “{name}”: {new} new, {updated} updated, {labels} new label{}",
                if labels == 1 { "" } else { "s" }
            ));
            dialog.close();
        }
    ));

    dialog.set_child(Some(&toolbar));
    dialog.present(Some(window));
}

fn open_md_file(dialog: &adw::Dialog, anchor: &impl IsA<gtk::Widget>, buffer: &gtk::TextBuffer) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Markdown / text"));
    filter.add_suffix("md");
    filter.add_suffix("markdown");
    filter.add_suffix("txt");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);

    let file_dialog = gtk::FileDialog::builder().title("Open Markdown").modal(true).filters(&filters).build();
    let root = anchor.root().and_downcast::<gtk::Window>();
    file_dialog.open(
        root.as_ref(),
        gio::Cancellable::NONE,
        glib::clone!(
            #[weak]
            buffer,
            #[weak]
            dialog,
            move |res| {
                if let Ok(file) = res {
                    match std::fs::read_to_string(file.path().unwrap_or_default()) {
                        Ok(text) => buffer.set_text(&text),
                        Err(e) => dialog.set_title(&format!("Couldn't read file: {e}")),
                    }
                }
            }
        ),
    );
}
