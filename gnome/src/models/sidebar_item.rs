//! `SidebarItem` — a row GObject for the sidebar `gio::ListStore`. Projects a
//! `UnifiedList` plus derived flags (section start, sync pending). GNOME counterpart
//! of the role data emitted by `kde/src/bridge/sidebar_model.rs`.

use std::cell::{Cell, RefCell};

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

mod imp {
    use super::*;

    #[derive(glib::Properties, Default)]
    #[properties(wrapper_type = super::SidebarItem)]
    pub struct SidebarItem {
        #[property(get, set)]
        pub list_id: RefCell<String>,
        #[property(get, set)]
        pub name: RefCell<String>,
        #[property(get, set)]
        pub icon: RefCell<String>,
        #[property(get, set)]
        pub emoji_icon: RefCell<String>,
        #[property(get, set)]
        pub unchecked_count: Cell<u32>,
        #[property(get, set)]
        pub is_dirty: Cell<bool>,
        /// "nextcloud" | "external"
        #[property(get, set)]
        pub source_type: RefCell<String>,
        #[property(get, set)]
        pub folder: RefCell<String>,
        #[property(get, set)]
        pub folder_icon: RefCell<String>,
        #[property(get, set)]
        pub is_section_start: Cell<bool>,
        #[property(get, set)]
        pub is_sync_pending: Cell<bool>,
        /// True for a non-selectable section-header pseudo-row (Favourites, a folder
        /// name, "Getting Started"). Headers carry their title in `name` and icon in `icon`.
        #[property(get, set)]
        pub is_header: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SidebarItem {
        const NAME: &'static str = "QuiteListieSidebarItem";
        type Type = super::SidebarItem;
    }

    #[glib::derived_properties]
    impl ObjectImpl for SidebarItem {}
}

glib::wrapper! {
    pub struct SidebarItem(ObjectSubclass<imp::SidebarItem>);
}

impl SidebarItem {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        list_id: &str,
        name: &str,
        icon: &str,
        emoji_icon: &str,
        unchecked_count: u32,
        is_dirty: bool,
        source_type: &str,
        folder: &str,
        folder_icon: &str,
        is_section_start: bool,
        is_sync_pending: bool,
    ) -> Self {
        glib::Object::builder()
            .property("list-id", list_id)
            .property("name", name)
            .property("icon", icon)
            .property("emoji-icon", emoji_icon)
            .property("unchecked-count", unchecked_count)
            .property("is-dirty", is_dirty)
            .property("source-type", source_type)
            .property("folder", folder)
            .property("folder-icon", folder_icon)
            .property("is-section-start", is_section_start)
            .property("is-sync-pending", is_sync_pending)
            .build()
    }

    /// A non-selectable section header row carrying `title` + a themed `icon` name.
    pub fn header(title: &str, icon: &str) -> Self {
        glib::Object::builder()
            .property("name", title)
            .property("icon", icon)
            .property("is-header", true)
            .build()
    }
}
