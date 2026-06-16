//! GTK views — GNOME counterparts of the former `qml/*.qml` pages. Built imperatively
//! with gtk4-rs (functionally equivalent to Blueprint `.ui`; Blueprint-ification is an
//! optional later refinement). Each module builds one page/widget.

pub mod global_map;
pub mod item_editor;
pub mod kanban_page;
pub mod label_editor;
pub mod list_page;
pub mod list_settings;
pub mod markdown_export;
pub mod markdown_import;
pub mod location_picker;
pub mod map_controls;
pub mod map_page;
pub mod nextcloud_browser;
pub mod nextcloud_setup;
pub mod pin_popover;
pub mod recent_changes;
pub mod recycle_bin;
pub mod reminder_page;
pub mod settings;
pub mod share_link;

use adw::prelude::*;

/// A sidebar show/hide toggle bound to the enclosing [`adw::OverlaySplitView`]'s
/// `show-sidebar`, found by walking up from `anchor`. Used by content drill-in pages (the
/// item editor, the per-list map) which build their own header but aren't handed a toggle
/// by the window. Returns `None` when `anchor` isn't inside a split.
pub fn sidebar_toggle(anchor: &impl IsA<gtk::Widget>) -> Option<gtk::ToggleButton> {
    let split = anchor
        .ancestor(adw::OverlaySplitView::static_type())
        .and_downcast::<adw::OverlaySplitView>()?;
    let btn = gtk::ToggleButton::builder()
        .icon_name("sidebar-show-symbolic")
        .tooltip_text("Toggle Sidebar")
        .build();
    split.bind_property("show-sidebar", &btn, "active").sync_create().bidirectional().build();
    Some(btn)
}
