//! `NextcloudSetup` — connect a Nextcloud account. GNOME counterpart of
//! `qml/NextcloudSetup.qml`. An `adw::Dialog` offering two paths:
//!
//! * **Login Flow v2** (recommended): opens the server's sign-in page in the browser
//!   and polls for an app password — supports 2FA/SSO.
//! * **Manual credentials**: server + username + app password + lists path, with a
//!   "Test Connection" step that must succeed before "Connect" is enabled.
//!
//! Drives the controller's `start_nextcloud_login` / `cancel_nextcloud_login` /
//! `test_nextcloud_credentials` / `connect_nextcloud_manual` / `nextcloud_logout`
//! methods and listens for `nc-login-completed`, `nc-test-result`, and
//! `error-occurred`.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::controller::Controller;

const DEFAULT_LISTS_PATH: &str = "/Lists";

/// Present the Nextcloud setup dialog over `parent`.
pub fn present(parent: &impl IsA<gtk::Widget>, controller: &Controller) {
    let dialog = adw::Dialog::builder()
        .title("Connect to Nextcloud")
        .content_width(480)
        .content_height(560)
        .build();

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());

    let page = adw::PreferencesPage::new();

    // --- Server + Login Flow v2 -------------------------------------------
    let server_group = adw::PreferencesGroup::builder()
        .title("Server")
        .description("Address of your Nextcloud server, e.g. https://cloud.example.com")
        .build();

    let server_row = adw::EntryRow::builder().title("Server URL").build();
    server_group.add(&server_row);
    page.add(&server_group);

    let login_group = adw::PreferencesGroup::new();

    // Normal "sign in" button.
    let login_button = gtk::Button::builder()
        .child(&adw::ButtonContent::builder().icon_name("weather-overcast-symbolic").label("Sign in with Nextcloud").build())
        .halign(gtk::Align::Start)
        .margin_top(4)
        .margin_bottom(4)
        .build();
    login_button.add_css_class("suggested-action");

    // Pending state: spinner + explanation + cancel.
    let pending_box = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    pending_box.set_visible(false);
    let spinner = gtk::Spinner::new();
    spinner.set_spinning(true);
    let pending_text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    pending_text.set_hexpand(true);
    let pending_title = gtk::Label::builder().label("Waiting for sign-in…").xalign(0.0).build();
    pending_title.add_css_class("heading");
    let pending_desc = gtk::Label::builder()
        .label("Complete the login in the browser, then return here.")
        .xalign(0.0)
        .wrap(true)
        .build();
    pending_desc.add_css_class("dim-label");
    pending_text.append(&pending_title);
    pending_text.append(&pending_desc);
    let cancel_button = gtk::Button::with_label("Cancel");
    cancel_button.set_valign(gtk::Align::Center);
    pending_box.append(&spinner);
    pending_box.append(&pending_text);
    pending_box.append(&cancel_button);

    let login_hint = gtk::Label::builder()
        .label("Opens your Nextcloud sign-in page in the browser. Supports 2FA and SSO.")
        .xalign(0.0)
        .wrap(true)
        .build();
    login_hint.add_css_class("dim-label");

    let login_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
    login_box.append(&login_button);
    login_box.append(&pending_box);
    login_box.append(&login_hint);
    login_group.add(&login_box);
    page.add(&login_group);

    // --- Manual credentials (collapsible) ---------------------------------
    let manual_group = adw::PreferencesGroup::new();
    let manual_expander = adw::ExpanderRow::builder().title("Sign in manually").build();

    let username_row = adw::EntryRow::builder().title("Username").build();
    let password_row = adw::PasswordEntryRow::builder().title("App Password").build();
    let path_row = adw::EntryRow::builder().title("Lists path").text(DEFAULT_LISTS_PATH).build();
    manual_expander.add_row(&username_row);
    manual_expander.add_row(&password_row);
    manual_expander.add_row(&path_row);

    // Test + Connect controls live in a small box appended as a row.
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_margin_top(6);
    actions.set_margin_bottom(6);
    actions.set_margin_start(12);
    actions.set_margin_end(12);
    let test_button = gtk::Button::with_label("Test Connection");
    let test_spinner = gtk::Spinner::new();
    let test_result = gtk::Label::builder().xalign(0.0).hexpand(true).wrap(true).build();
    let connect_button = gtk::Button::with_label("Connect");
    connect_button.add_css_class("suggested-action");
    connect_button.set_sensitive(false);
    actions.append(&test_button);
    actions.append(&test_spinner);
    actions.append(&test_result);
    actions.append(&connect_button);
    let actions_row = adw::ActionRow::new();
    actions_row.set_child(Some(&actions));
    manual_expander.add_row(&actions_row);

    manual_group.add(&manual_expander);
    page.add(&manual_group);

    // --- Disconnect (only when already connected) -------------------------
    if controller.is_nc_authenticated() {
        let dc_group = adw::PreferencesGroup::new();
        let dc_button = gtk::Button::builder()
            .child(&adw::ButtonContent::builder().icon_name("network-offline-symbolic").label("Disconnect from Nextcloud").build())
            .build();
        dc_button.add_css_class("destructive-action");
        dc_button.connect_clicked(glib::clone!(
            #[weak]
            controller,
            #[weak]
            dialog,
            move |_| {
                controller.nextcloud_logout();
                dialog.close();
            }
        ));
        dc_group.add(&dc_button);
        page.add(&dc_group);
    }

    toolbar.set_content(Some(&page));
    dialog.set_child(Some(&toolbar));

    // --- shared field readers ---------------------------------------------
    let server_text = {
        let row = server_row.clone();
        move || row.text().trim().to_string()
    };

    // --- Login Flow v2 actions --------------------------------------------
    let set_pending = {
        let login_button = login_button.clone();
        let login_hint = login_hint.clone();
        let pending_box = pending_box.clone();
        move |pending: bool| {
            login_button.set_visible(!pending);
            login_hint.set_visible(!pending);
            pending_box.set_visible(pending);
        }
    };

    login_button.connect_clicked(glib::clone!(
        #[weak]
        controller,
        #[strong]
        server_text,
        #[strong]
        set_pending,
        move |_| {
            let server = server_text();
            if server.is_empty() {
                return;
            }
            controller.start_nextcloud_login(&server, DEFAULT_LISTS_PATH);
            set_pending(true);
        }
    ));

    cancel_button.connect_clicked(glib::clone!(
        #[weak]
        controller,
        #[strong]
        set_pending,
        move |_| {
            controller.cancel_nextcloud_login();
            set_pending(false);
        }
    ));

    // --- manual: Test enables Connect on success --------------------------
    let test_ok = Rc::new(Cell::new(false));

    // Re-typing invalidates a prior successful test.
    let invalidate = glib::clone!(
        #[weak]
        connect_button,
        #[weak]
        test_result,
        #[strong]
        test_ok,
        move || {
            test_ok.set(false);
            connect_button.set_sensitive(false);
            test_result.set_text("");
        }
    );
    for row in [&username_row, &path_row] {
        row.connect_changed(glib::clone!(
            #[strong]
            invalidate,
            move |_| invalidate()
        ));
    }
    password_row.connect_changed(glib::clone!(
        #[strong]
        invalidate,
        move |_| invalidate()
    ));

    test_button.connect_clicked(glib::clone!(
        #[weak]
        controller,
        #[weak]
        test_spinner,
        #[weak]
        test_result,
        #[strong]
        server_text,
        #[weak]
        username_row,
        #[weak]
        password_row,
        #[weak]
        path_row,
        move |_| {
            let server = server_text();
            let user = username_row.text().trim().to_string();
            let pass = password_row.text().to_string();
            let path = path_row.text().trim().to_string();
            if server.is_empty() || user.is_empty() || pass.is_empty() {
                return;
            }
            test_result.set_text("");
            test_spinner.set_spinning(true);
            test_spinner.set_visible(true);
            controller.test_nextcloud_credentials(&server, &user, &pass, &path);
        }
    ));

    connect_button.connect_clicked(glib::clone!(
        #[weak]
        controller,
        #[strong]
        server_text,
        #[weak]
        username_row,
        #[weak]
        password_row,
        #[weak]
        path_row,
        move |_| {
            controller.connect_nextcloud_manual(
                &server_text(),
                username_row.text().trim(),
                &password_row.text(),
                path_row.text().trim(),
            );
        }
    ));

    // --- controller signal subscriptions (disconnected on close) ----------
    let login_done = controller.connect_nc_login_completed(glib::clone!(
        #[weak]
        dialog,
        move |_| {
            dialog.close();
        }
    ));

    let test_done = controller.connect_nc_test_result(glib::clone!(
        #[weak]
        test_spinner,
        #[weak]
        test_result,
        #[weak]
        connect_button,
        #[strong]
        test_ok,
        move |_, ok, msg| {
            test_spinner.set_spinning(false);
            test_spinner.set_visible(false);
            test_result.set_text(&msg);
            test_result.remove_css_class("success");
            test_result.remove_css_class("error");
            test_result.add_css_class(if ok { "success" } else { "error" });
            test_ok.set(ok);
            connect_button.set_sensitive(ok);
        }
    ));

    let errored = controller.connect_error_occurred(glib::clone!(
        #[strong]
        set_pending,
        #[weak]
        test_spinner,
        move |_, _msg| {
            // Reset busy states; the window's toast surfaces the message.
            set_pending(false);
            test_spinner.set_spinning(false);
            test_spinner.set_visible(false);
        }
    ));

    // `connect_closed` is an `Fn`, but `disconnect` consumes each id — stash them in a
    // cell and drain on the first (only) close.
    let handlers = std::cell::RefCell::new(Some(vec![login_done, test_done, errored]));
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

    test_spinner.set_visible(false);
    dialog.present(Some(parent));
}
