//! Quite Listie — native GNOME (GTK4 + libadwaita) front-end entry point.

mod application;
mod controller;
mod geo;
mod models;
mod runtime;
mod views;
mod widgets;
mod window;

use adw::prelude::*;
use application::QuiteListieApplication;
use gtk::glib;

/// Application ID — reused unchanged from the KDE port so existing desktop
/// integration (icons, MIME associations) carries over.
pub const APP_ID: &str = "com.quiteyellow.QuiteListie";

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // The shared core does its blocking/network I/O on a tokio runtime; UI runs on
    // the GLib main loop. runtime::spawn_to_main bridges the two.
    runtime::init();

    // Headless smoke test: construct the controller, load the disk cache, print the
    // smart-box counts, and exit. Used to verify the data layer without a display.
    // Run with: QL_SMOKE=1 GSETTINGS_SCHEMA_DIR=<dir-with-compiled-schema> quite-listie
    if std::env::var("QL_SMOKE").is_ok() {
        return smoke_test();
    }

    let app = QuiteListieApplication::new();
    app.run()
}

fn smoke_test() -> glib::ExitCode {
    // gio::Settings needs the GLib type system initialised.
    gtk::init().expect("gtk init");
    let controller = controller::Controller::new();
    controller.connect_lists_updated(|c| {
        tracing::info!(
            "lists-updated: today={} scheduled={} location={} nc_auth={}",
            c.today_count(),
            c.scheduled_count(),
            c.location_count(),
            c.is_nc_authenticated(),
        );
    });
    controller.connect_error_occurred(|_, msg| tracing::error!("error: {msg}"));

    // Exercise the sidebar + list-item models off the same provider.
    let sidebar_store = gtk::gio::ListStore::new::<models::SidebarItem>();
    let list_model = models::ListItemModel::new(controller.provider());
    controller.connect_lists_updated(glib::clone!(
        #[weak]
        sidebar_store,
        #[strong]
        list_model,
        move |c| {
            models::populate_sidebar(&sidebar_store, c);
            tracing::info!("sidebar store now has {} row(s)", sidebar_store.n_items());
            // Open the first list, if any, and report its item count. Bind the id in
            // its own statement so the provider lock is released before set_list_id
            // (which locks again) — avoids a reentrant deadlock.
            let first = c.provider().blocking_lock().lists.first().map(|l| l.id.clone());
            if let Some(first) = first {
                list_model.set_list_id(&first);
                tracing::info!(
                    "opened list {first}: {} item row(s) after cache render",
                    list_model.store().n_items()
                );
            }
        }
    ));

    // Drive one refresh cycle, then quit the main loop shortly after.
    controller.refresh_lists();
    let n = controller.provider().blocking_lock().lists.len();
    tracing::info!("disk cache loaded: {n} list(s) in the sidebar index");

    let main_loop = glib::MainLoop::new(None, false);
    glib::timeout_add_seconds_local(
        2,
        glib::clone!(
            #[strong]
            main_loop,
            move || {
                main_loop.quit();
                glib::ControlFlow::Break
            }
        ),
    );
    main_loop.run();
    glib::ExitCode::SUCCESS
}
