//! App preferences — GNOME counterpart of the Swift `SettingsView`. An
//! [`adw::PreferencesDialog`] whose switches are bound directly to the app `GSettings`
//! (`controller.settings()`), so changes persist immediately and the window's
//! `settings::changed` handlers re-render the affected UI live.
//!
//! Apple-only sections of the source (iCloud, EventKit calendar, CarPlay) are dropped.
//! Map/kanban/quick-add toggles land with their features (maps, kanban) — exposing a
//! toggle whose feature isn't built yet would be a dead switch, so they're omitted here.

use adw::prelude::*;
use gtk::{gio, glib};

use crate::controller::Controller;
use crate::views;

/// Present the preferences dialog over `parent`.
pub fn present(parent: &impl IsA<gtk::Widget>, controller: &Controller) {
    let settings = controller.settings();
    let dialog = adw::PreferencesDialog::new();
    dialog.set_title("Preferences");

    let page = adw::PreferencesPage::new();

    // --- Sidebar group -----------------------------------------------------
    let sidebar = adw::PreferencesGroup::builder().title("Sidebar").build();

    let welcome = adw::SwitchRow::builder()
        .title("Show Welcome List")
        .subtitle("The welcome list explains how to use the app.")
        .build();
    bind_inverted(settings, "hide-welcome-list", &welcome);
    sidebar.add(&welcome);

    let today = switch_row("Show Today Card");
    bind_inverted(settings, "hide-today-card", &today);
    sidebar.add(&today);

    let scheduled = switch_row("Show Scheduled Card");
    bind_inverted(settings, "hide-scheduled-card", &scheduled);
    sidebar.add(&scheduled);

    let locations = switch_row("Show Locations Card");
    bind_inverted(settings, "hide-locations-card", &locations);
    sidebar.add(&locations);

    page.add(&sidebar);

    // --- Lists & Labels group ---------------------------------------------
    let lists = adw::PreferencesGroup::builder().title("Lists & Labels").build();

    let completed = adw::SwitchRow::builder()
        .title("Completed Items at Bottom")
        .subtitle("Move checked items below the unchecked ones.")
        .build();
    settings.bind("show-completed-at-bottom", &completed, "active").build();
    lists.add(&completed);

    let empty_labels = adw::SwitchRow::builder()
        .title("Show Empty Labels")
        .subtitle("Display label sections even when they have no items.")
        .build();
    bind_inverted(settings, "hide-empty-labels", &empty_labels);
    lists.add(&empty_labels);

    page.add(&lists);

    // --- Nextcloud group ---------------------------------------------------
    let nextcloud = adw::PreferencesGroup::builder().title("Nextcloud").build();

    let account = adw::ActionRow::builder()
        .title("Nextcloud Account")
        .activatable(true)
        .build();
    let server = controller.nc_server_url();
    account.set_subtitle(if controller.is_nc_authenticated() && !server.is_empty() {
        &server
    } else {
        "Not connected — tap to connect"
    });
    account.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    let controller_owned = controller.clone();
    account.connect_activated(glib::clone!(
        #[weak]
        dialog,
        move |_| views::nextcloud_setup::present(&dialog, &controller_owned)
    ));
    nextcloud.add(&account);

    page.add(&nextcloud);

    dialog.add(&page);
    dialog.present(Some(parent));
}

fn switch_row(title: &str) -> adw::SwitchRow {
    adw::SwitchRow::builder().title(title).build()
}

/// Bind a `Show X` switch to a `hide-x` GSettings key (active = !hidden).
fn bind_inverted(settings: &gio::Settings, key: &str, row: &adw::SwitchRow) {
    settings.bind(key, row, "active").invert_boolean().build();
}
