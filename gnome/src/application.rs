//! The [`adw::Application`] subclass. Owns app-level actions, the single-instance
//! guard, and (in later phases) `quitelistie://` deeplink + `.listie` file opening
//! via `gio::Application::HANDLES_OPEN`.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};

use crate::window::QuiteListieWindow;
use crate::APP_ID;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct QuiteListieApplication;

    #[glib::object_subclass]
    impl ObjectSubclass for QuiteListieApplication {
        const NAME: &'static str = "QuiteListieApplication";
        type Type = super::QuiteListieApplication;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for QuiteListieApplication {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.setup_actions();
        }
    }

    impl ApplicationImpl for QuiteListieApplication {
        fn startup(&self) {
            self.parent_startup();
            // Dev convenience: an uninstalled `cargo` run has no icon on the theme
            // search path (the hicolor PNGs are only copied there by `meson install`),
            // so the app logo is missing. In debug builds, add the repo's icon tree so
            // the app-id resolves. Release/installed builds use the installed icons.
            #[cfg(debug_assertions)]
            if let Some(display) = gtk::gdk::Display::default() {
                let theme = gtk::IconTheme::for_display(&display);
                theme.add_search_path(concat!(env!("CARGO_MANIFEST_DIR"), "/data/icons"));
            }
        }

        fn activate(&self) {
            self.obj().ensure_window().present();
        }

        // Files / URIs handed to the app (`HANDLES_OPEN`): `.listie` files and
        // `quitelistie://` deeplinks. A second invocation routes here through the
        // single primary instance, so we reuse (or create) the one window.
        fn open(&self, files: &[gio::File], _hint: &str) {
            let window = self.obj().ensure_window();
            window.present();
            for file in files {
                window.open_file(file);
            }
        }
    }

    impl GtkApplicationImpl for QuiteListieApplication {}
    impl AdwApplicationImpl for QuiteListieApplication {}
}

glib::wrapper! {
    pub struct QuiteListieApplication(ObjectSubclass<imp::QuiteListieApplication>)
        @extends adw::Application, gtk::Application, gio::Application,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl QuiteListieApplication {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("application-id", APP_ID)
            .property("flags", gio::ApplicationFlags::HANDLES_OPEN)
            .build()
    }

    /// Return the single app window, creating it if this is the first activation.
    fn ensure_window(&self) -> QuiteListieWindow {
        self.active_window()
            .and_downcast::<QuiteListieWindow>()
            .unwrap_or_else(|| QuiteListieWindow::new(self))
    }

    fn setup_actions(&self) {
        let quit = gio::ActionEntry::builder("quit")
            .activate(|app: &Self, _, _| app.quit())
            .build();
        // New Window (Swift "New Window", QuiteListie.swift:191). Each window owns its
        // own Controller but shares the singleton provider, so they show the same data.
        let new_window = gio::ActionEntry::builder("new-window")
            .activate(|app: &Self, _, _| QuiteListieWindow::new(app).present())
            .build();
        self.add_action_entries([quit, new_window]);
        self.set_accels_for_action("app.quit", &["<primary>q"]);
        self.set_accels_for_action("app.new-window", &["<primary><shift>n"]);
    }
}

impl Default for QuiteListieApplication {
    fn default() -> Self {
        Self::new()
    }
}
