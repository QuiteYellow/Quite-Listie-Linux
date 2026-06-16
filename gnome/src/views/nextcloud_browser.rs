//! `NextcloudBrowser` — browse the remote `.listie` files on a connected Nextcloud
//! account and open or create lists. GNOME counterpart of `qml/NextcloudBrowser.qml`.
//!
//! A single `adw::Dialog` holding a directory listing that is rebuilt on each
//! navigation. Drives `Controller::browse_nextcloud_at` and reacts to the
//! `remote-files-ready` signal (a JSON array of entries); opening / creating routes
//! through `open_remote_list` / `create_list_at`, after which `lists-updated` triggers
//! a refresh so the "already open" flags stay current.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::controller::Controller;

/// One entry as emitted by `Controller::browse_nextcloud_at`.
#[derive(serde::Deserialize)]
struct RemoteFile {
    name: String,
    #[serde(rename = "isDirectory")]
    is_directory: bool,
    #[serde(rename = "alreadyOpen")]
    already_open: bool,
    #[serde(rename = "displayName")]
    display_name: String,
    #[serde(rename = "remotePath")]
    remote_path: String,
}

struct State {
    current_path: String,
    path_stack: Vec<String>,
}

/// Present the Nextcloud browser over `parent`. Assumes an authenticated account.
pub fn present(parent: &impl IsA<gtk::Widget>, controller: &Controller) {
    let dialog = adw::Dialog::builder()
        .title("Nextcloud")
        .content_width(480)
        .content_height(560)
        .build();

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();

    let back_button = gtk::Button::from_icon_name("go-previous-symbolic");
    back_button.set_tooltip_text(Some("Back"));
    back_button.set_visible(false);
    header.pack_start(&back_button);

    let refresh_button = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh_button.set_tooltip_text(Some("Refresh"));
    header.pack_end(&refresh_button);

    let new_button = gtk::Button::from_icon_name("list-add-symbolic");
    new_button.set_tooltip_text(Some("New List Here"));
    header.pack_end(&new_button);
    toolbar.add_top_bar(&header);

    // Body: a stack of {loading spinner, empty placeholder, file list}.
    let stack = gtk::Stack::new();

    let spinner = gtk::Spinner::new();
    spinner.set_spinning(true);
    let loading = adw::StatusPage::builder().title("Loading…").child(&spinner).build();
    stack.add_named(&loading, Some("loading"));

    let empty = adw::StatusPage::builder()
        .icon_name("folder-symbolic")
        .title("No files found")
        .description("This folder has no .listie files or subfolders.")
        .build();
    stack.add_named(&empty, Some("empty"));

    let list_box = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .build();
    list_box.add_css_class("boxed-list");
    let list_clamp = adw::Clamp::builder().child(&list_box).margin_top(12).margin_bottom(12).build();
    let scroller = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&list_clamp)
        .build();
    stack.add_named(&scroller, Some("list"));

    stack.set_visible_child_name("loading");
    toolbar.set_content(Some(&stack));
    dialog.set_child(Some(&toolbar));

    let state = Rc::new(RefCell::new(State {
        current_path: "/".to_string(),
        path_stack: Vec::new(),
    }));

    // ----- navigation ------------------------------------------------------
    let load = {
        let controller = controller.clone();
        let stack = stack.clone();
        let state = state.clone();
        Rc::new(move || {
            stack.set_visible_child_name("loading");
            let path = state.borrow().current_path.clone();
            controller.browse_nextcloud_at(&path);
        })
    };

    let update_title = {
        let dialog = dialog.clone();
        let back_button = back_button.clone();
        let state = state.clone();
        Rc::new(move || {
            let st = state.borrow();
            let title = st
                .current_path
                .trim_matches('/')
                .rsplit('/')
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or("Nextcloud");
            dialog.set_title(title);
            back_button.set_visible(!st.path_stack.is_empty());
        })
    };
    update_title();

    refresh_button.connect_clicked(glib::clone!(
        #[strong]
        load,
        move |_| load()
    ));

    back_button.connect_clicked(glib::clone!(
        #[strong]
        load,
        #[strong]
        update_title,
        #[strong]
        state,
        move |_| {
            // Bind the pop result to a local so the `borrow_mut()` temporary is dropped
            // before the next borrow (an `if let` scrutinee borrow lives through the body).
            let prev = state.borrow_mut().path_stack.pop();
            if let Some(prev) = prev {
                state.borrow_mut().current_path = prev;
            }
            update_title();
            load();
        }
    ));

    new_button.connect_clicked(glib::clone!(
        #[weak]
        controller,
        #[weak]
        dialog,
        #[strong]
        state,
        move |_| {
            present_new_list_dialog(&dialog, &controller, state.borrow().current_path.clone());
        }
    ));

    // ----- populate on remote-files-ready ----------------------------------
    let populate = {
        let list_box = list_box.clone();
        let stack = stack.clone();
        let controller = controller.clone();
        let load = load.clone();
        let update_title = update_title.clone();
        let state = state.clone();
        move |json: String| {
            // Clear existing rows.
            while let Some(child) = list_box.first_child() {
                list_box.remove(&child);
            }
            let files: Vec<RemoteFile> = serde_json::from_str(&json).unwrap_or_default();
            if files.is_empty() {
                stack.set_visible_child_name("empty");
                return;
            }
            for f in files {
                let row = adw::ActionRow::builder().title(if f.is_directory {
                    glib::markup_escape_text(&f.name).to_string()
                } else {
                    glib::markup_escape_text(&f.display_name).to_string()
                }).build();
                let icon = gtk::Image::from_icon_name(if f.is_directory {
                    "folder-symbolic"
                } else {
                    "text-x-generic-symbolic"
                });
                row.add_prefix(&icon);

                if f.is_directory {
                    row.set_activatable(true);
                    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
                    let remote_path = f.remote_path.clone();
                    row.connect_activated(glib::clone!(
                        #[strong]
                        load,
                        #[strong]
                        update_title,
                        #[strong]
                        state,
                        move |_| {
                            {
                                let mut st = state.borrow_mut();
                                let cur = st.current_path.clone();
                                st.path_stack.push(cur);
                                st.current_path = remote_path.clone();
                            }
                            update_title();
                            load();
                        }
                    ));
                } else {
                    row.set_sensitive(!f.already_open);
                    let open_button = gtk::Button::builder()
                        .label(if f.already_open { "Open" } else { "Add" })
                        .valign(gtk::Align::Center)
                        .build();
                    if f.already_open {
                        row.set_subtitle("Already open");
                    } else {
                        open_button.add_css_class("suggested-action");
                    }
                    let remote_path = f.remote_path.clone();
                    open_button.connect_clicked(glib::clone!(
                        #[weak]
                        controller,
                        move |_| controller.open_remote_list(&remote_path)
                    ));
                    row.add_suffix(&open_button);
                }
                list_box.append(&row);
            }
            stack.set_visible_child_name("list");
        }
    };

    // ----- controller subscriptions (disconnected on close) ----------------
    let files_ready = controller.connect_remote_files_ready(glib::clone!(
        #[strong]
        populate,
        move |_, json| populate(json)
    ));

    // Opening/creating a list refreshes the index — reload to update flags.
    let lists_updated = controller.connect_lists_updated(glib::clone!(
        #[strong]
        load,
        move |_| load()
    ));

    // On error, drop the spinner; the window's toast shows the message.
    let errored = controller.connect_error_occurred(glib::clone!(
        #[weak]
        stack,
        #[weak]
        list_box,
        move |_, _msg| {
            // Show whatever (possibly empty) list we have rather than spin forever.
            let name = if list_box.first_child().is_some() { "list" } else { "empty" };
            stack.set_visible_child_name(name);
        }
    ));

    let handlers = RefCell::new(Some(vec![files_ready, lists_updated, errored]));
    dialog.connect_closed(glib::clone!(
        #[weak]
        controller,
        move |_| {
            if let Some(ids) = handlers.borrow_mut().take() {
                for id in ids {
                    controller.disconnect(id);
                }
            }
        }
    ));

    dialog.present(Some(parent));
    load();
}

/// Ask for a name and create a list in `folder`.
fn present_new_list_dialog(parent: &adw::Dialog, controller: &Controller, folder: String) {
    let dialog = adw::AlertDialog::new(
        Some("New List"),
        Some(&format!(
            "Create a new list in: {}",
            if folder == "/" { "Nextcloud (root)" } else { &folder }
        )),
    );
    let entry = gtk::Entry::builder().placeholder_text("List name").build();
    dialog.set_extra_child(Some(&entry));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("create", "Create");
    dialog.set_response_appearance("create", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("create"));
    dialog.set_close_response("cancel");

    let do_create = glib::clone!(
        #[weak]
        controller,
        #[weak]
        entry,
        move || {
            let name = entry.text();
            let name = name.trim();
            if !name.is_empty() {
                controller.create_list_at(&folder, name);
            }
        }
    );
    entry.connect_activate(glib::clone!(
        #[weak]
        dialog,
        #[strong]
        do_create,
        move |_| {
            do_create();
            dialog.close();
        }
    ));
    dialog.connect_response(None, move |_, response| {
        if response == "create" {
            do_create();
        }
    });
    dialog.present(Some(parent));
}
