//! First-run welcome list seeding.
//!
//! Mirrors Swift `ExampleData.welcomeList` (Local/ExampleData.swift): an
//! 8-label list with onboarding content, keyed by the stable id
//! `example-welcome-list` so the same document round-trips between iOS and KDE.
//!
//! Strategy: at first launch we write a `.listie` file to
//! `~/.local/share/quite-listie/example-welcome-list.listie` and add it to
//! ExternalOpenedFiles. After that, the file exists like any other and the
//! user can rename/edit/delete it. A small "seeded" sentinel under the state
//! directory ensures we don't recreate it if the user explicitly deleted it.
//!
//! The KDE port keeps Swift's label IDs and colours so a synced welcome list
//! merges cleanly across platforms.
//!
//! KDE-specific content (right-click context menus instead of swipes, no
//! iCloud private lists) lives in this file rather than being faithfully
//! ported from Swift, per the project memory `feedback_swift_match_full.md`:
//! match Swift fully *when porting Swift views*, but adapt platform-specific
//! help copy.

use std::path::PathBuf;

use chrono::Utc;

use crate::model::{ListDocument, ListItem, ListLabel, ListSummary};

/// Stable id of the seeded welcome list (matches Swift `ExampleData.welcomeList`).
pub const WELCOME_LIST_ID: &str = "example-welcome-list";
const WELCOME_FILE_NAME: &str = "example-welcome-list.listie";
const SEEDED_FLAG: &str = "welcome-seeded";

fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap().join(".local/share"))
        .join("quite-listie")
}

/// Directory holding local `.listie` files (the welcome list and user-created local
/// lists). Used by `UnifiedProvider::create_local_list`.
pub fn local_data_dir() -> PathBuf {
    data_dir()
}

fn state_dir() -> PathBuf {
    dirs::state_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap().join(".local/state"))
        .join("quite-listie")
}

fn seeded_flag_path() -> PathBuf {
    state_dir().join(SEEDED_FLAG)
}

/// Seed the welcome list at first run. Returns the on-disk path of the
/// welcome `.listie` file when one was just written (so the caller can add
/// it to ExternalOpenedFiles), or `None` if the flag was already set or
/// writing failed. Idempotent.
pub fn seed_welcome_list_if_first_run() -> Option<PathBuf> {
    let flag = seeded_flag_path();
    if flag.exists() {
        return None;
    }
    let _ = std::fs::create_dir_all(state_dir());
    let _ = std::fs::create_dir_all(data_dir());

    let target = data_dir().join(WELCOME_FILE_NAME);
    if target.exists() {
        // File already on disk (e.g. user restored from a backup); just set
        // the flag so we don't try again.
        let _ = std::fs::write(&flag, b"1");
        return None;
    }

    let doc = build_welcome_document();
    let bytes = match serde_json::to_vec_pretty(&doc) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("welcome seed: encode failed: {e}");
            return None;
        }
    };
    if let Err(e) = std::fs::write(&target, &bytes) {
        tracing::warn!("welcome seed: write {target:?} failed: {e}");
        return None;
    }
    let _ = std::fs::write(&flag, b"1");
    tracing::info!("welcome seed: wrote {target:?}");
    Some(target)
}

fn build_welcome_document() -> ListDocument {
    let labels = vec![
        label("welcome-getting-started", "Start Here",         "#34C759"),
        label("welcome-items",           "Items & Editing",    "#FF9500"),
        label("welcome-labels",          "Labels & Organisation","#AF52DE"),
        label("welcome-views",           "Views & Layout",     "#5AC8FA"),
        label("welcome-reminders",       "Reminders",          "#FF3B30"),
        label("welcome-import-export",   "Import & Export",    "#007AFF"),
        label("welcome-collaboration",   "Collaboration",      "#FF2D55"),
        label("welcome-shortcuts",       "Keyboard Shortcuts", "#8E8E93"),
    ];

    let label_order: Vec<String> = labels.iter().map(|l| l.id.clone()).collect();

    let items = vec![
        item("Welcome to Quite Listie",
             "welcome-getting-started",
             "## Welcome!\n\nQuite Listie keeps your lists in plain `.listie` files — share them via Nextcloud, drop them in a synced folder, or keep them local. You can browse, edit, and reorganise items right here in the sidebar.\n\n### What to try next\n- Tick this item off (or right-click for more options)\n- Open **List Settings** in the sidebar context menu to rename or theme this list\n- Use the **+** button at the top of the sidebar to start a fresh list"),
        item("Items: edit, drag, recycle",
             "welcome-items",
             "Click an item to open the editor. From there you can change the name, quantity, notes (Markdown!), label, location, and reminders. Deleted items go to the **Recycle Bin** for 30 days before they're auto-cleaned — open it from the sidebar to restore anything you missed."),
        item("Labels organise everything",
             "welcome-labels",
             "Labels are how items group themselves. Manage them from **List Settings → Manage Labels…**: rename, recolour, set an emoji icon. Items render in label order — drag a label up/down to reshuffle the whole list."),
        item("List, Kanban, and Map views",
             "welcome-views",
             "The toolbar lets you switch between flat list, kanban board (by label), and a map view (for items with locations). Pick what fits the moment — the choice is per-list."),
        item("Reminders that respect your time",
             "welcome-reminders",
             "Toggle **Reminder** in the item editor and pick a date + time. Repeating reminders support daily/weekly/monthly/yearly plus custom intervals. The notification carries a **Mark complete** action — for repeating items it advances to the next occurrence; for one-offs it ticks the item and clears the reminder."),
        item("Import and export Markdown",
             "welcome-import-export",
             "Use the overflow menu on any list to **Export as Markdown** or **Share Link**. Inbound markdown goes through **Import** — paste a `# Heading` + `- [ ] item` checklist and Quite Listie figures out the labels."),
        item("Collaborate with Nextcloud",
             "welcome-collaboration",
             "Connect to a Nextcloud server from **Settings → Nextcloud** and your lists sync automatically. Conflicts are resolved field-by-field, so two people editing the same list rarely lose work."),
        item("Keyboard shortcuts",
             "welcome-shortcuts",
             "**Ctrl+F** focuses the search bar. More shortcuts are on the way — meanwhile, every action lives in the toolbar or the right-click context menu (KDE convention)."),
        item("Done with this list?",
             "welcome-getting-started",
             "Right-click it in the sidebar and choose **Delete List** to remove it from disk — this welcome doc won't reappear afterwards. Or **Close File** to keep it on disk but hide it."),
    ];

    let summary = ListSummary {
        id: WELCOME_LIST_ID.to_string(),
        name: "Welcome to Quite Listie".to_string(),
        modified_at: Utc::now(),
        icon: Some("book".to_string()),
        emoji_icon: Some("👋".to_string()),
        hidden_labels: Vec::new(),
        label_order,
        enable_map_data: false,
        extra: Default::default(),
    };

    ListDocument {
        version: 2,
        list: summary,
        items,
        labels,
        deleted_label_ids: Vec::new(),
        extra: Default::default(),
    }
}

fn label(id: &str, name: &str, color: &str) -> ListLabel {
    ListLabel::new(id, name, color)
}

fn item(note: &str, label_id: &str, markdown_notes: &str) -> ListItem {
    let mut it = ListItem::new(note.to_string());
    it.label_id = Some(label_id.to_string());
    it.markdown_notes = Some(markdown_notes.to_string());
    it
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn welcome_document_is_well_formed() {
        let doc = build_welcome_document();
        assert_eq!(doc.list.id, WELCOME_LIST_ID);
        assert_eq!(doc.labels.len(), 8);
        assert!(doc.items.len() >= 8);
        for item in &doc.items {
            let lid = item.label_id.as_deref().unwrap();
            assert!(
                doc.labels.iter().any(|l| l.id == lid),
                "item references unknown label {lid}"
            );
        }
        // round-trips through JSON
        let json = serde_json::to_string(&doc).unwrap();
        let _: ListDocument = serde_json::from_str(&json).unwrap();
    }
}
