//! GNOME data models — `gio::ListStore`s of row GObjects, replacing the cxx-qt
//! `QAbstractListModel` bridges. Each list view owns a store; populators rebuild it
//! from the shared `UnifiedProvider`, mirroring the former `rebuild_rows`/`reset`.

mod list_item_object;
mod pin_object;
mod reminder_entry;
mod sidebar_item;

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use gtk::gio;
use gtk::gio::prelude::SettingsExt;
use tokio::sync::Mutex;

use chrono::{DateTime, Local, Utc};
use quite_listie_core::engine::unified_provider::ListSource;
use quite_listie_core::engine::unified_provider::{UnifiedList, UnifiedProvider};
use quite_listie_core::model::{
    Coordinate, ListDocument, ListItem, ListLabel, ReminderRepeatMode, ReminderRepeatRule,
};

pub use list_item_object::ListItemObject;
pub use pin_object::PinObject;
pub use reminder_entry::ReminderEntryObject;
pub use sidebar_item::SidebarItem;

use crate::controller::Controller;
use crate::runtime::spawn_to_main;

type Provider = Arc<Mutex<UnifiedProvider>>;

// ---------------------------------------------------------------------------
// Sidebar
// ---------------------------------------------------------------------------

/// Rebuild the sidebar store from the provider's list index, grouped into the same
/// sections as the Swift `SidebarView`: **Getting Started** (welcome), **Favourites**
/// (star), then one section per folder (alphabetical by folder name, lists alphabetical
/// within). Each section is preceded by a non-selectable header [`SidebarItem`]
/// (`is_header`). Mirrors `SidebarView.body`'s section layout.
pub fn populate_sidebar(store: &gio::ListStore, controller: &Controller) {
    let provider = controller.provider();
    let p = provider.blocking_lock();
    let hide_welcome = controller.settings().boolean("hide-welcome-list");
    let welcome_id = quite_listie_core::engine::welcome::WELCOME_LIST_ID;

    let project = |entry: &UnifiedList| {
        let source_type = match &entry.source {
            ListSource::Nextcloud { .. } => "nextcloud",
            ListSource::ExternalFile { .. } => "external",
        };
        SidebarItem::new(
            &entry.id,
            &entry.name,
            entry.icon.as_deref().unwrap_or("view-list-symbolic"),
            entry.emoji_icon.as_deref().unwrap_or(""),
            entry.unchecked_count as u32,
            entry.is_dirty,
            source_type,
            &entry.folder,
            entry.folder_icon,
            false,
            p.sync_pending_ids.contains(&entry.id),
        )
    };
    let by_name = |a: &&UnifiedList, b: &&UnifiedList| {
        a.name.to_lowercase().cmp(&b.name.to_lowercase())
    };

    store.remove_all();

    // Getting Started — the seeded welcome list (keyed by document UUID, not runtime id).
    if !hide_welcome {
        if let Some(welcome) = p.lists.iter().find(|l| l.original_file_id == welcome_id) {
            store.append(&SidebarItem::header("Getting Started", "book-symbolic"));
            store.append(&project(welcome));
        }
    }

    let is_welcome = |l: &&UnifiedList| l.original_file_id == welcome_id;

    // Favourites — favourited lists (excluding welcome), alphabetical.
    let mut favourites: Vec<&UnifiedList> = p
        .lists
        .iter()
        .filter(|l| !is_welcome(l) && controller.is_favourite(&l.id))
        .collect();
    favourites.sort_by(by_name);
    if !favourites.is_empty() {
        store.append(&SidebarItem::header("Favourites", "starred-symbolic"));
        for entry in favourites {
            store.append(&project(entry));
        }
    }

    // Folder sections — the rest, grouped by folder name, sections alphabetical.
    let mut folders: HashMap<&str, (&'static str, Vec<&UnifiedList>)> = HashMap::new();
    for entry in p.lists.iter() {
        if is_welcome(&entry) || controller.is_favourite(&entry.id) {
            continue;
        }
        folders
            .entry(&entry.folder)
            .or_insert_with(|| (entry.folder_icon, Vec::new()))
            .1
            .push(entry);
    }
    let mut sections: Vec<(&str, &'static str, Vec<&UnifiedList>)> = folders
        .into_iter()
        .map(|(name, (icon, lists))| (name, icon, lists))
        .collect();
    sections.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    for (name, icon, mut lists) in sections {
        lists.sort_by(by_name);
        let sym = match icon {
            "folder-cloud" => "folder-remote-symbolic",
            _ => "folder-symbolic",
        };
        store.append(&SidebarItem::header(name, sym));
        for entry in lists {
            store.append(&project(entry));
        }
    }
}

// ---------------------------------------------------------------------------
// Reminders (cross-list)
// ---------------------------------------------------------------------------

/// Gather every unchecked item with a reminder across all cached lists, sorted by
/// reminder date, then projected into [`ReminderEntryObject`]s carrying their parent-list
/// and label context plus the date group they belong to. Mirrors the Swift
/// `WelcomeViewModel.reminderEntries` + `ReminderListView.filteredEntries`.
///
/// `today_only` (the **Today** smart box) keeps only overdue-or-today reminders, matching
/// the sidebar count in `Controller::recompute_counts_blocking`; otherwise (**Scheduled**)
/// every reminder is returned.
pub fn reminder_entries(controller: &Controller, today_only: bool) -> Vec<ReminderEntryObject> {
    let provider = controller.provider();
    let p = provider.blocking_lock();
    let today = Local::now().date_naive();

    // Collect (item, list, labels) refs first so we can sort by date before projecting.
    let mut pairs: Vec<(&ListItem, &UnifiedList, &[ListLabel])> = Vec::new();
    for (list, doc) in p.all_cached_docs() {
        for item in doc.active_items().filter(|i| !i.checked) {
            let Some(date) = item.reminder_date else { continue };
            if today_only {
                let day: DateTime<Local> = date.into();
                if day.date_naive() > today {
                    continue;
                }
            }
            pairs.push((item, list, &doc.labels));
        }
    }
    pairs.sort_by_key(|(item, _, _)| item.reminder_date);

    pairs
        .into_iter()
        .map(|(item, list, labels)| ReminderEntryObject::from_item(item, labels, list, today))
        .collect()
}

// ---------------------------------------------------------------------------
// Locations (map pins)
// ---------------------------------------------------------------------------

/// Resolve a Maps URL / bare `lat,lng` / place name to coordinates off the main thread,
/// then invoke `cb` on the main thread with `Some((lat, lng, source_url))` or `None`.
/// `source_url` echoes the input only when it parses as a URL. Mirrors the KDE
/// `parse_location_input` + Swift `LocationParser.parseCoordinateWithSource`. Used by the
/// item editor's paste-location and address-search flows.
pub fn resolve_location_async<F>(input: &str, cb: F)
where
    F: FnOnce(Option<(f64, f64, String)>) + 'static,
{
    let raw = input.trim().to_string();
    spawn_to_main(
        async move {
            quite_listie_core::util::location_parser::resolve_location_with_source(&raw)
                .await
                .map(|(c, src)| (c.latitude, c.longitude, src))
        },
        cb,
    );
}

/// Extract a place name from a Maps URL (Swift `LocationParser.parsePlaceName`), used to
/// auto-fill an empty item name after a paste-location. Returns `None` for non-URL input
/// or URLs without a `/place/<NAME>/` segment.
pub fn parse_place_name(source_url: &str) -> Option<String> {
    quite_listie_core::util::location_parser::parse_place_name(source_url)
}

/// All active items in `list_id` that carry a location, projected into [`PinObject`]s.
/// Mirrors the KDE `list_pins_json` / Swift `MapListView.allLocationItems`.
pub fn list_pins(controller: &Controller, list_id: &str) -> Vec<PinObject> {
    let provider = controller.provider();
    let p = provider.blocking_lock();
    let Some(doc) = p.cached_doc(list_id) else {
        return Vec::new();
    };
    let list = p.lists.iter().find(|l| l.id == list_id);
    doc.active_items()
        .filter_map(|item| {
            let l = list?;
            PinObject::from_item(item, &doc.labels, l)
        })
        .collect()
}

/// Every active item with a location across all open lists. Mirrors the Swift
/// `WelcomeViewModel.locationEntries` feeding `GlobalMapView`.
pub fn global_pins(controller: &Controller) -> Vec<PinObject> {
    let provider = controller.provider();
    let p = provider.blocking_lock();
    let mut pins = Vec::new();
    for (list, doc) in p.all_cached_docs() {
        for item in doc.active_items() {
            if let Some(pin) = PinObject::from_item(item, &doc.labels, list) {
                pins.push(pin);
            }
        }
    }
    pins
}

// ---------------------------------------------------------------------------
// List items
// ---------------------------------------------------------------------------

/// Owns the item `gio::ListStore` for the currently-open list and the mutation
/// methods the list page calls. GNOME counterpart of `list_item_model.rs`.
///
/// Held behind an `Rc` so async continuations (which run on the GLib main thread
/// via [`spawn_to_main`]) can recapture it to rebuild the store after a fetch.
/// Special section names, matching the Swift app's grouping keys.
pub const NO_LABEL: &str = "No Label";
pub const COMPLETED: &str = "Completed";

/// One label group in the list view (mirrors a Swift `Section` from `renderSection`).
pub struct LabelSection {
    /// Display name: a label name, [`NO_LABEL`], or [`COMPLETED`].
    pub key: String,
    /// Label id for per-section quick-add (empty for No Label / Completed).
    pub label_id: String,
    /// Hex colour for the header dot (empty for No Label / Completed).
    pub color: String,
    pub emoji: String,
    pub is_completed: bool,
    /// Count shown in the header (unchecked for label sections, checked for Completed).
    pub header_count: usize,
    /// Items rendered first: the unchecked items (or, for Completed, the checked items).
    pub primary_items: Vec<ListItemObject>,
    /// Checked items shown inline beneath the add row when *not* showing a bottom
    /// Completed section. Empty for the Completed section and when completed-at-bottom.
    pub extra_checked: Vec<ListItemObject>,
}

pub struct ListItemModel {
    store: gio::ListStore,
    provider: Provider,
    list_id: RefCell<String>,
    labels: RefCell<Vec<ListLabel>>,
    show_checked_at_bottom: Cell<bool>,
    search: RefCell<String>,
    /// Single "the data changed, re-render" callback owned by the current list page.
    /// Replaced on each page build (so old pages don't leak) and invoked once per
    /// mutation (so there's no per-row signal storm).
    on_changed: RefCell<Option<Rc<dyn Fn()>>>,
}

impl ListItemModel {
    pub fn new(provider: Provider) -> Rc<Self> {
        Rc::new(Self {
            store: gio::ListStore::new::<ListItemObject>(),
            provider,
            list_id: RefCell::new(String::new()),
            labels: RefCell::new(Vec::new()),
            show_checked_at_bottom: Cell::new(false),
            search: RefCell::new(String::new()),
            on_changed: RefCell::new(None),
        })
    }

    /// Register the current page's re-render callback (replaces any previous one).
    pub fn set_on_changed(self: &Rc<Self>, cb: Rc<dyn Fn()>) {
        *self.on_changed.borrow_mut() = Some(cb);
    }

    /// Invoke the change callback (clone the `Rc` out first so the borrow is released
    /// before running it).
    fn notify_changed(&self) {
        let cb = self.on_changed.borrow().clone();
        if let Some(cb) = cb {
            cb();
        }
    }

    pub fn store(&self) -> &gio::ListStore {
        &self.store
    }

    pub fn list_id(&self) -> String {
        self.list_id.borrow().clone()
    }

    pub fn set_show_checked_at_bottom(self: &Rc<Self>, value: bool) {
        self.show_checked_at_bottom.set(value);
        self.reload();
    }

    /// Point the model at a new list: render from cache immediately if available,
    /// then refresh from the network in the background. Mirrors `set_list_id`.
    pub fn set_list_id(self: &Rc<Self>, list_id: &str) {
        *self.list_id.borrow_mut() = list_id.to_string();
        let id = list_id.to_string();

        {
            let p = self.provider.blocking_lock();
            if let Some(doc) = p.cached_doc(&id) {
                let doc = doc.clone();
                drop(p);
                self.rebuild(&doc);
            }
        }

        let provider = self.provider.clone();
        let this = self.clone();
        let id_for_task = id.clone();
        spawn_to_main(
            async move {
                let mut p = provider.lock().await;
                p.fetch_document(&id_for_task).await
            },
            move |result| {
                if let Ok(doc) = result {
                    this.rebuild(&doc);
                }
            },
        );
    }

    pub fn set_search_text(self: &Rc<Self>, text: &str) {
        *self.search.borrow_mut() = text.to_string();
        self.reload();
    }

    /// Re-render from the cached document (no network).
    pub fn reload(self: &Rc<Self>) {
        let id = self.list_id.borrow().clone();
        let p = self.provider.blocking_lock();
        if let Some(doc) = p.cached_doc(&id) {
            let doc = doc.clone();
            drop(p);
            self.rebuild(&doc);
        }
    }

    /// Flatten + filter + sort the document's items and repopulate the store.
    /// Mirrors `rebuild_rows` (label-order grouping, completed-at-bottom).
    fn rebuild(&self, doc: &ListDocument) {
        let search = self.search.borrow().to_lowercase();
        let show_checked_bottom = self.show_checked_at_bottom.get();

        let mut active: Vec<ListItem> = doc
            .active_items()
            .filter(|i| {
                if search.is_empty() {
                    return true;
                }
                i.note.to_lowercase().contains(&search)
                    || i.markdown_notes
                        .as_deref()
                        .map(|n| n.to_lowercase().contains(&search))
                        .unwrap_or(false)
            })
            .cloned()
            .collect();

        let label_order = &doc.list.label_order;
        let labels = &doc.labels;
        let sort_by_label_alpha = |a: &ListItem, b: &ListItem| {
            let ka = label_sort_key(label_order, labels, a.label_id.as_deref());
            let kb = label_sort_key(label_order, labels, b.label_id.as_deref());
            ka.0.cmp(&kb.0)
                .then(ka.1.cmp(&kb.1))
                .then(a.note.to_lowercase().cmp(&b.note.to_lowercase()))
        };

        if show_checked_bottom {
            let mut unchecked: Vec<ListItem> =
                active.iter().filter(|i| !i.checked).cloned().collect();
            let mut checked: Vec<ListItem> = active.iter().filter(|i| i.checked).cloned().collect();
            unchecked.sort_by(sort_by_label_alpha);
            checked.sort_by(|a, b| a.note.to_lowercase().cmp(&b.note.to_lowercase()));
            unchecked.extend(checked);
            active = unchecked;
        } else {
            active.sort_by(sort_by_label_alpha);
        }

        *self.labels.borrow_mut() = doc.labels.clone();

        self.store.remove_all();
        for item in &active {
            self.store
                .append(&ListItemObject::from_item(item, &doc.labels, show_checked_bottom));
        }

        // One coalesced re-render per mutation (not one per appended row).
        self.notify_changed();
    }

    // ----- mutations -------------------------------------------------------

    /// Toggle an item's checked state, advancing or clearing reminders to match
    /// `list_item_model::toggle_checked`.
    pub fn toggle_checked(self: &Rc<Self>, item_id: &str) {
        use quite_listie_core::engine::recurrence::next_reminder_date;
        use chrono::Utc;
        let Ok(id) = item_id.parse::<uuid::Uuid>() else {
            return;
        };
        let list_id = self.list_id.borrow().clone();
        let doc = {
            let mut p = self.provider.blocking_lock();
            let Some(item) = p
                .cached_doc(&list_id)
                .and_then(|doc| doc.items.iter().find(|i| i.id == id).cloned())
            else {
                return;
            };
            let mut updated = item;
            updated.checked = !updated.checked;
            updated.touch();
            updated.checked_at = Some(Utc::now());
            updated.last_change_field = Some("checked".into());

            if updated.checked && updated.reminder_date.is_some() {
                if let (Some(rule), Some(mode), Some(fire_at)) = (
                    updated.reminder_repeat_rule.clone(),
                    updated.reminder_repeat_mode.clone(),
                    updated.reminder_date,
                ) {
                    updated.reminder_date =
                        Some(next_reminder_date(fire_at, &rule, &mode, Utc::now()));
                    updated.checked = false;
                } else {
                    updated.reminder_date = None;
                }
            }

            p.update_item(&list_id, updated.clone());
            p.sync_reminder_for_item(&list_id, &updated);
            p.trigger_autosave(&list_id);
            p.cached_doc(&list_id).cloned()
        };
        if let Some(doc) = doc {
            self.rebuild(&doc);
        }
    }

    /// Add a new item. Pass `NaN` latitude/longitude for no location.
    #[allow(clippy::too_many_arguments)]
    pub fn add_item(
        self: &Rc<Self>,
        note: &str,
        label_id: &str,
        quantity: f64,
        markdown_notes: &str,
        source_url: &str,
        latitude: f64,
        longitude: f64,
    ) {
        let list_id = self.list_id.borrow().clone();
        let mut new_item = ListItem::new(note.to_string());
        new_item.quantity = quantity;
        if !label_id.is_empty() {
            new_item.label_id = Some(label_id.to_string());
        }
        if !markdown_notes.is_empty() {
            new_item.markdown_notes = Some(markdown_notes.to_string());
        }
        if !source_url.is_empty() {
            new_item.source_url = Some(source_url.to_string());
        }
        if (-90.0..=90.0).contains(&latitude) && (-180.0..=180.0).contains(&longitude) {
            new_item.location = Some(quite_listie_core::model::Coordinate {
                latitude,
                longitude,
                extra: Default::default(),
            });
        }
        new_item.last_change_field = Some("added".into());

        let doc = {
            let mut p = self.provider.blocking_lock();
            p.add_item(&list_id, new_item.clone());
            p.sync_reminder_for_item(&list_id, &new_item);
            p.trigger_autosave(&list_id);
            p.cached_doc(&list_id).cloned()
        };
        if let Some(doc) = doc {
            self.rebuild(&doc);
        }
    }

    /// Create a fully-specified item (Swift `AddItemView` → `addItem`). The lightweight
    /// inline path [`add_item`] only carries a note/quantity/label; this mirrors
    /// [`update_item_fields`] for a brand-new item so the "Add Item" screen can set
    /// reminders, location, notes, and source URL on creation.
    #[allow(clippy::too_many_arguments)]
    pub fn add_item_full(
        self: &Rc<Self>,
        note: &str,
        quantity: f64,
        label_id: &str,
        checked: bool,
        markdown_notes: &str,
        source_url: &str,
        reminder_date: Option<DateTime<Utc>>,
        repeat_rule: Option<ReminderRepeatRule>,
        repeat_mode: Option<ReminderRepeatMode>,
        location: Option<Coordinate>,
    ) {
        let list_id = self.list_id.borrow().clone();
        let mut new_item = ListItem::new(note.to_string());
        new_item.quantity = quantity;
        new_item.checked = checked;
        new_item.label_id = (!label_id.is_empty()).then(|| label_id.to_string());
        new_item.markdown_notes = (!markdown_notes.is_empty()).then(|| markdown_notes.to_string());
        new_item.source_url = (!source_url.is_empty()).then(|| source_url.to_string());
        new_item.location = location;
        new_item.reminder_date = reminder_date;
        new_item.reminder_repeat_rule = reminder_date.and(repeat_rule);
        new_item.reminder_repeat_mode = reminder_date.and(repeat_mode);
        new_item.last_change_field = Some("added".into());

        let doc = {
            let mut p = self.provider.blocking_lock();
            p.add_item(&list_id, new_item.clone());
            p.sync_reminder_for_item(&list_id, &new_item);
            p.trigger_autosave(&list_id);
            p.cached_doc(&list_id).cloned()
        };
        if let Some(doc) = doc {
            self.rebuild(&doc);
        }
    }

    /// Look up the current item by id (from the cached document).
    pub fn get_item(&self, item_id: &str) -> Option<ListItem> {
        let list_id = self.list_id.borrow().clone();
        self.provider.blocking_lock().get_item(&list_id, item_id)
    }

    /// The labels of the open list (for the editor's label picker).
    pub fn labels(&self) -> Vec<ListLabel> {
        self.labels.borrow().clone()
    }

    /// The open list's display name (for the item editor's list chip).
    pub fn list_name(&self) -> String {
        let id = self.list_id.borrow().clone();
        self.provider
            .blocking_lock()
            .cached_doc(&id)
            .map(|d| d.list.name.clone())
            .unwrap_or_default()
    }

    /// The open list's emoji icon, if any (for the item editor's list chip).
    pub fn list_emoji(&self) -> Option<String> {
        let id = self.list_id.borrow().clone();
        self.provider.blocking_lock().cached_doc(&id).and_then(|d| d.list.emoji_icon.clone())
    }

    /// For an external `.listie` file, the parent folder's name (Swift `editFolderName`).
    /// `None` for Nextcloud lists.
    pub fn list_folder_name(&self) -> Option<String> {
        let id = self.list_id.borrow().clone();
        let p = self.provider.blocking_lock();
        let list = p.lists.iter().find(|l| l.id == id)?;
        match &list.source {
            ListSource::ExternalFile { path } => {
                path.parent().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().into_owned())
            }
            ListSource::Nextcloud { .. } => None,
        }
    }

    /// Whether the open list has map/location features enabled (per-list setting). Gates
    /// the item editor's location section (matches Swift `enableMapData`).
    pub fn enable_map_data(&self) -> bool {
        let id = self.list_id.borrow().clone();
        self.provider
            .blocking_lock()
            .cached_doc(&id)
            .map(|d| d.list.enable_map_data)
            .unwrap_or(false)
    }

    // ----- grouping (mirrors Swift ListViewModel) --------------------------

    /// Group the open list's items into label sections, matching the Swift
    /// `filteredItemsGroupedByLabel` + `sortedLabelNames` + `renderSection` logic.
    ///
    /// * Items are grouped by label *name* ([`NO_LABEL`] when unlabelled), each group
    ///   sorted by note. Sections follow the list's `label_order` with No Label last.
    /// * `hidden_labels` (per-list) are dropped. When `hide_empty`, label sections with
    ///   no unchecked items are dropped; otherwise every (non-hidden) label is shown.
    /// * When completed-at-bottom is on, checked items collect into a trailing
    ///   [`COMPLETED`] section; otherwise each section shows its own checked items inline.
    pub fn grouped_sections(&self, hide_empty: bool) -> Vec<LabelSection> {
        let id = self.list_id.borrow().clone();
        let p = self.provider.blocking_lock();
        let Some(doc) = p.cached_doc(&id) else {
            return Vec::new();
        };
        let show_bottom = self.show_checked_at_bottom.get();
        let search = self.search.borrow().to_lowercase();
        let labels = &doc.labels;

        // Filter (search) the active items.
        let items: Vec<&ListItem> = doc
            .active_items()
            .filter(|i| {
                search.is_empty()
                    || i.note.to_lowercase().contains(&search)
                    || i.markdown_notes
                        .as_deref()
                        .map(|n| n.to_lowercase().contains(&search))
                        .unwrap_or(false)
            })
            .collect();

        // Group by label name.
        let name_for = |item: &ListItem| -> String {
            item.label_id
                .as_deref()
                .and_then(|lid| labels.iter().find(|l| l.id == lid))
                .map(|l| l.name.clone())
                .unwrap_or_else(|| NO_LABEL.to_string())
        };
        let mut groups: HashMap<String, (Vec<ListItem>, Vec<ListItem>)> = HashMap::new();
        for item in &items {
            let entry = groups.entry(name_for(item)).or_default();
            if item.checked {
                entry.1.push((*item).clone());
            } else {
                entry.0.push((*item).clone());
            }
        }
        let by_note = |a: &ListItem, b: &ListItem| a.note.to_lowercase().cmp(&b.note.to_lowercase());
        for (u, c) in groups.values_mut() {
            u.sort_by(by_note);
            c.sort_by(by_note);
        }

        // Decide which label sections to show, in order.
        let hidden: HashSet<&str> = doc.list.hidden_labels.iter().map(|s| s.as_str()).collect();
        let hidden_names: HashSet<String> = labels
            .iter()
            .filter(|l| hidden.contains(l.id.as_str()))
            .map(|l| l.name.clone())
            .collect();

        let section_names: Vec<String> = if hide_empty {
            // Swift hideEmptyLabels: a label is "empty" only when it has no items at
            // all. With completed-at-bottom on, a label's checked items move to the
            // trailing Completed section, so the label is shown only when it has an
            // unchecked item. With completed shown inline, a label holding only checked
            // items still has rows to display, so keep it.
            let present: Vec<String> = groups
                .iter()
                .filter(|(_, (u, c))| !u.is_empty() || (!show_bottom && !c.is_empty()))
                .map(|(k, _)| k.clone())
                .collect();
            sorted_label_names(&present, labels, &doc.list.label_order)
                .into_iter()
                .filter(|n| !hidden_names.contains(n))
                .collect()
        } else {
            // Every (non-hidden) label, plus No Label when it has items.
            let mut names: Vec<String> = labels.iter().map(|l| l.name.clone()).collect();
            if groups.contains_key(NO_LABEL) {
                names.push(NO_LABEL.to_string());
            }
            sorted_label_names(&names, labels, &doc.list.label_order)
                .into_iter()
                .filter(|n| !hidden_names.contains(n))
                .collect()
        };

        let to_objs = |items: &[ListItem]| -> Vec<ListItemObject> {
            items
                .iter()
                .map(|i| ListItemObject::from_item(i, labels, show_bottom))
                .collect()
        };

        let mut sections = Vec::new();
        for name in section_names {
            let (unchecked, checked) = groups.get(&name).cloned().unwrap_or_default();
            let label = labels.iter().find(|l| l.name == name);
            sections.push(LabelSection {
                key: name.clone(),
                label_id: label.map(|l| l.id.clone()).unwrap_or_default(),
                color: label.map(|l| l.color.clone()).unwrap_or_default(),
                emoji: label.and_then(|l| l.emoji_icon.clone()).unwrap_or_default(),
                is_completed: false,
                header_count: unchecked.len(),
                primary_items: to_objs(&unchecked),
                extra_checked: if show_bottom { Vec::new() } else { to_objs(&checked) },
            });
        }

        // Trailing Completed section (only when completed-at-bottom).
        if show_bottom {
            let mut all_checked: Vec<ListItem> =
                items.iter().filter(|i| i.checked).map(|i| (*i).clone()).collect();
            all_checked.sort_by(by_note);
            if !all_checked.is_empty() {
                sections.push(LabelSection {
                    key: COMPLETED.to_string(),
                    label_id: String::new(),
                    color: String::new(),
                    emoji: String::new(),
                    is_completed: true,
                    header_count: all_checked.len(),
                    primary_items: to_objs(&all_checked),
                    extra_checked: Vec::new(),
                });
            }
        }

        sections
    }

    /// Apply the editor's fields (note/quantity/label/notes/url + reminder) in one
    /// mutation. `reminder_date == None` clears the reminder (and its repeat rule/mode).
    #[allow(clippy::too_many_arguments)]
    pub fn update_item_fields(
        self: &Rc<Self>,
        item_id: &str,
        note: &str,
        quantity: f64,
        label_id: &str,
        checked: bool,
        markdown_notes: &str,
        source_url: &str,
        reminder_date: Option<DateTime<Utc>>,
        repeat_rule: Option<ReminderRepeatRule>,
        repeat_mode: Option<ReminderRepeatMode>,
        location: Option<Coordinate>,
    ) {
        use quite_listie_core::engine::recurrence::next_reminder_date;
        let Ok(id) = item_id.parse::<uuid::Uuid>() else {
            return;
        };
        let list_id = self.list_id.borrow().clone();
        let doc = {
            let mut p = self.provider.blocking_lock();
            let Some(mut item) = p.get_item(&list_id, item_id) else {
                return;
            };
            let _ = id; // id already validated; provider keyed by string
            let was_checked = item.checked;
            item.note = note.to_string();
            item.quantity = quantity;
            item.checked = checked;
            item.label_id = (!label_id.is_empty()).then(|| label_id.to_string());
            item.markdown_notes = (!markdown_notes.is_empty()).then(|| markdown_notes.to_string());
            item.source_url = (!source_url.is_empty()).then(|| source_url.to_string());
            item.location = location;
            item.reminder_date = reminder_date;
            // Repeat rule/mode only make sense alongside a reminder.
            item.reminder_repeat_rule = reminder_date.and(repeat_rule);
            item.reminder_repeat_mode = reminder_date.and(repeat_mode);
            if was_checked != checked {
                item.last_change_field = Some("checked".into());
                item.checked_at = Some(Utc::now());
            } else {
                item.last_change_field = Some("note".into());
            }
            // Checking off an item with a reminder advances a repeat (staying unchecked) or
            // clears a one-off, mirroring Swift updateItem + toggle_checked.
            if !was_checked && checked && item.reminder_date.is_some() {
                if let (Some(rule), Some(mode), Some(fire_at)) = (
                    item.reminder_repeat_rule.clone(),
                    item.reminder_repeat_mode.clone(),
                    item.reminder_date,
                ) {
                    item.reminder_date = Some(next_reminder_date(fire_at, &rule, &mode, Utc::now()));
                    item.checked = false;
                } else {
                    item.reminder_date = None;
                }
            }
            item.touch();
            p.update_item(&list_id, item.clone());
            p.sync_reminder_for_item(&list_id, &item);
            p.trigger_autosave(&list_id);
            p.cached_doc(&list_id).cloned()
        };
        if let Some(doc) = doc {
            self.rebuild(&doc);
        }
    }

    /// Soft-delete an item and cancel any pending reminder.
    /// Mark every (non-deleted) item completed or active in one mutation, advancing or
    /// clearing reminders exactly as [`toggle_checked`] does (Swift `setAllItems`).
    pub fn set_all_checked(self: &Rc<Self>, completed: bool) {
        use chrono::Utc;
        use quite_listie_core::engine::recurrence::next_reminder_date;
        let list_id = self.list_id.borrow().clone();
        let doc = {
            let mut p = self.provider.blocking_lock();
            let Some(items) = p
                .cached_doc(&list_id)
                .map(|d| d.active_items().cloned().collect::<Vec<_>>())
            else {
                return;
            };
            for item in items {
                if item.checked == completed {
                    continue;
                }
                let mut updated = item;
                updated.checked = completed;
                updated.touch();
                updated.checked_at = Some(Utc::now());
                updated.last_change_field = Some("checked".into());
                if completed && updated.reminder_date.is_some() {
                    if let (Some(rule), Some(mode), Some(fire_at)) = (
                        updated.reminder_repeat_rule.clone(),
                        updated.reminder_repeat_mode.clone(),
                        updated.reminder_date,
                    ) {
                        updated.reminder_date =
                            Some(next_reminder_date(fire_at, &rule, &mode, Utc::now()));
                        updated.checked = false;
                    } else {
                        updated.reminder_date = None;
                    }
                }
                p.update_item(&list_id, updated.clone());
                p.sync_reminder_for_item(&list_id, &updated);
            }
            p.trigger_autosave(&list_id);
            p.cached_doc(&list_id).cloned()
        };
        if let Some(doc) = doc {
            self.rebuild(&doc);
        }
    }

    /// Increase the item's quantity by 1 (Swift `incrementQuantity`).
    pub fn increment_quantity(self: &Rc<Self>, item_id: &str) {
        self.adjust_quantity(item_id, 1.0);
    }

    /// Decrease quantity by 1, flooring at 1. Returns `false` when the item was already at
    /// quantity 1 (the caller should offer to delete it), matching Swift `decrementQuantity`.
    pub fn decrement_quantity(self: &Rc<Self>, item_id: &str) -> bool {
        match self.get_item(item_id) {
            Some(item) if item.quantity > 1.0 => {
                self.adjust_quantity(item_id, -1.0);
                true
            }
            _ => false,
        }
    }

    fn adjust_quantity(self: &Rc<Self>, item_id: &str, delta: f64) {
        let list_id = self.list_id.borrow().clone();
        let doc = {
            let mut p = self.provider.blocking_lock();
            let Some(mut item) = p.get_item(&list_id, item_id) else {
                return;
            };
            item.quantity = (item.quantity + delta).max(1.0);
            item.last_change_field = Some("quantity".into());
            item.touch();
            p.update_item(&list_id, item.clone());
            p.trigger_autosave(&list_id);
            p.cached_doc(&list_id).cloned()
        };
        if let Some(doc) = doc {
            self.rebuild(&doc);
        }
    }

    pub fn delete_item(self: &Rc<Self>, item_id: &str) {
        let Ok(id) = item_id.parse::<uuid::Uuid>() else {
            return;
        };
        let list_id = self.list_id.borrow().clone();
        let doc = {
            let mut p = self.provider.blocking_lock();
            p.soft_delete_item(&list_id, id);
            p.reminder_engine.cancel(&id.to_string());
            p.trigger_autosave(&list_id);
            p.cached_doc(&list_id).cloned()
        };
        if let Some(doc) = doc {
            self.rebuild(&doc);
        }
    }
}

/// Order label section names by the list's `label_order` (ids), with unordered labels
/// following alphabetically and [`NO_LABEL`] always last. Mirrors the Swift
/// `sortedLabelNames`.
fn sorted_label_names(names: &[String], labels: &[ListLabel], label_order: &[String]) -> Vec<String> {
    let has_no_label = names.iter().any(|n| n == NO_LABEL);
    let alpha = |a: &String, b: &String| a.to_lowercase().cmp(&b.to_lowercase());

    if label_order.is_empty() {
        let mut v: Vec<String> = names.iter().filter(|n| *n != NO_LABEL).cloned().collect();
        v.sort_by(alpha);
        if has_no_label {
            v.push(NO_LABEL.to_string());
        }
        return v;
    }

    let id_to_name: HashMap<&str, &str> =
        labels.iter().map(|l| (l.id.as_str(), l.name.as_str())).collect();
    let name_set: HashSet<&str> = names.iter().map(|s| s.as_str()).collect();

    let mut ordered: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for id in label_order {
        if let Some(name) = id_to_name.get(id.as_str()) {
            if name_set.contains(name) && !seen.contains(*name) {
                ordered.push((*name).to_string());
                seen.insert((*name).to_string());
            }
        }
    }
    let mut remaining: Vec<String> = names
        .iter()
        .filter(|n| !seen.contains(n.as_str()) && *n != NO_LABEL)
        .cloned()
        .collect();
    remaining.sort_by(alpha);
    ordered.extend(remaining);
    if has_no_label {
        ordered.push(NO_LABEL.to_string());
    }
    ordered
}

/// Sort key `(priority, label_name_lower)` matching `label_sort_key` in the KDE model.
fn label_sort_key(order: &[String], labels: &[ListLabel], label_id: Option<&str>) -> (u32, String) {
    match label_id {
        None => (u32::MAX, String::new()),
        Some(id) => {
            if let Some(pos) = order.iter().position(|o| o == id) {
                return (pos as u32, String::new());
            }
            let name = labels
                .iter()
                .find(|l| l.id == id)
                .map(|l| l.name.to_lowercase())
                .unwrap_or_else(|| id.to_lowercase());
            (u32::MAX - 1, name)
        }
    }
}
