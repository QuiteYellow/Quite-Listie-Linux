use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use tracing::warn;
use uuid::Uuid;

use crate::engine::nextcloud::{FileGone, NextcloudManager};
use crate::engine::purge::purge_old_deleted_items;
use crate::model::{ListDocument, ListItem, ListLabel};

// ---------------------------------------------------------------------------
// List source
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum ListSource {
    /// Nextcloud WebDAV.
    /// `file_name`: bare filename (e.g. "groceries.listie"), used for NC API calls.
    /// `remote_path`: full path relative to DAV root (e.g. "/Lists/groceries.listie"),
    ///   used for section-name derivation. Empty string means file is in the configured
    ///   lists_remote_path (legacy; migrated lazily).
    Nextcloud { file_name: String, remote_path: String },
    /// A .listie file opened directly from the filesystem (read-only until saved explicitly).
    ExternalFile { path: PathBuf },
}

// ---------------------------------------------------------------------------
// UnifiedList entry (sidebar item)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct UnifiedList {
    /// Runtime ID derived from source (`external:<path>` or `nextcloud:<remote_path>`).
    /// Used as the sidebar key and the cache key — never the document UUID.
    pub id: String,
    /// The document's persistent UUID (`doc.list.id`). Used to detect duplicate-UUID
    /// opens across different sources. Matches Swift's `originalFileId`.
    pub original_file_id: String,
    pub name: String,
    pub icon: Option<String>,
    pub emoji_icon: Option<String>,
    pub source: ListSource,
    pub is_dirty: bool,
    pub unchecked_count: usize,
    /// Section heading for sidebar grouping (e.g. "Lists", "Home", "Documents").
    pub folder: String,
    /// KDE icon name for the section heading.
    pub folder_icon: &'static str,
}

/// Build the sidebar/cache runtime ID from a source. Mirrors Swift's
/// `"external:\(url.path)"` and `"nextcloud:\(remotePath)"` scheme.
pub fn runtime_id_for(source: &ListSource) -> String {
    match source {
        ListSource::Nextcloud { remote_path, .. } => format!("nextcloud:{remote_path}"),
        ListSource::ExternalFile { path } => format!("external:{}", path.display()),
    }
}

/// Human-readable description of a source for error messages.
fn describe_source(source: &ListSource) -> String {
    match source {
        ListSource::Nextcloud { remote_path, .. } => format!("Nextcloud {remote_path}"),
        ListSource::ExternalFile { path } => path.display().to_string(),
    }
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

pub struct UnifiedProvider {
    pub nc: NextcloudManager,
    pub lists: Vec<UnifiedList>,
    /// In-memory document cache, keyed by list id.
    cache: HashMap<String, ListDocument>,
    /// Handles for in-progress autosave tasks, keyed by list id.
    autosave_handles: HashMap<String, tokio::task::JoinHandle<()>>,
    /// Nextcloud list IDs the user has explicitly removed — never re-added by sync.
    excluded_nc_ids: std::collections::HashSet<String>,
    /// NC list IDs currently serving cached data because the server was unreachable.
    pub sync_pending_ids: std::collections::HashSet<String>,
    /// Reminder engine — drives D-Bus notifications.
    pub reminder_engine: crate::engine::reminder_engine::ReminderEngine,
}

impl Default for UnifiedProvider {
    fn default() -> Self {
        Self {
            nc: NextcloudManager::new(),
            lists: Vec::new(),
            cache: HashMap::new(),
            autosave_handles: HashMap::new(),
            excluded_nc_ids: ExcludedIds::load().0,
            sync_pending_ids: std::collections::HashSet::new(),
            reminder_engine: crate::engine::reminder_engine::ReminderEngine::new(),
        }
    }
}

impl UnifiedProvider {
    // -----------------------------------------------------------------------
    // Listing
    // -----------------------------------------------------------------------

    /// Refresh the list of all lists from Nextcloud (and any open external files).
    pub async fn refresh_list_index(&mut self) -> Result<()> {
        if !self.nc.is_authenticated() {
            return Ok(());
        }

        // Check reachability before making requests; if unreachable we serve from disk cache.
        let reachable = self.nc.is_server_reachable().await;

        let remote_files = if reachable {
            self.nc.list_remote_files().await?
        } else {
            warn!("NC server unreachable during refresh — serving from disk cache");
            Vec::new()
        };
        let lists_path = self.nc.lists_remote_path().trim_end_matches('/').to_string();

        for (file_name, _etag) in remote_files {
            if self.excluded_nc_ids.contains(&file_name) {
                continue;
            }

            let remote_path = format!("{}/{}", lists_path, file_name);
            let runtime_id = format!("nextcloud:{remote_path}");
            if self.lists.iter().any(|l| l.id == runtime_id) {
                continue;
            }
            let nc_folder = nc_folder_from_path(&remote_path);

            // Use open_file (cache-first) rather than always downloading.
            match self.nc.open_file_at(&file_name, Some(&remote_path)).await {
                Ok((doc, from_cache)) => {
                    if from_cache {
                        self.spawn_nc_background_sync(file_name.clone(), remote_path.clone());
                    }
                    let new_uuid = doc.list.id.clone();
                    if let Some(existing) = self.lists.iter().find(|l| l.original_file_id == new_uuid) {
                        warn!(
                            "skipping NC file {file_name}: UUID {new_uuid} already loaded as \"{}\"",
                            existing.name
                        );
                        continue;
                    }
                    let unchecked = doc.active_items().filter(|i| !i.checked).count();
                    let source = ListSource::Nextcloud { file_name, remote_path };
                    self.lists.push(UnifiedList {
                        id: runtime_id.clone(),
                        original_file_id: new_uuid,
                        name: doc.list.name.clone(),
                        icon: doc.list.icon.clone(),
                        emoji_icon: doc.list.emoji_icon.clone(),
                        source,
                        is_dirty: false,
                        unchecked_count: unchecked,
                        folder: nc_folder,
                        folder_icon: "folder-cloud",
                    });
                    self.cache.insert(runtime_id, doc);
                }
                Err(e) => {
                    warn!("failed to fetch {file_name} during index refresh: {e}");
                    // Attempt disk cache fallback before showing a placeholder.
                    if let Some(doc) = self.nc.open_from_disk_cache(&remote_path) {
                        let new_uuid = doc.list.id.clone();
                        if let Some(existing) = self.lists.iter().find(|l| l.original_file_id == new_uuid) {
                            warn!(
                                "skipping NC disk-cache {file_name}: UUID {new_uuid} already loaded as \"{}\"",
                                existing.name
                            );
                            continue;
                        }
                        let unchecked = doc.active_items().filter(|i| !i.checked).count();
                        let source = ListSource::Nextcloud { file_name, remote_path };
                        self.lists.push(UnifiedList {
                            id: runtime_id.clone(),
                            original_file_id: new_uuid,
                            name: doc.list.name.clone(),
                            icon: doc.list.icon.clone(),
                            emoji_icon: doc.list.emoji_icon.clone(),
                            source,
                            is_dirty: false,
                            unchecked_count: unchecked,
                            folder: nc_folder.clone(),
                            folder_icon: "folder-cloud",
                        });
                        self.cache.insert(runtime_id.clone(), doc);
                        self.sync_pending_ids.insert(runtime_id);
                    } else {
                        // Placeholder — no doc available, original_file_id unknown until first fetch.
                        let source = ListSource::Nextcloud { file_name: file_name.clone(), remote_path };
                        self.lists.push(UnifiedList {
                            id: runtime_id,
                            original_file_id: String::new(),
                            name: file_name.trim_end_matches(".listie").to_string(),
                            icon: None,
                            emoji_icon: None,
                            source,
                            is_dirty: false,
                            unchecked_count: 0,
                            folder: nc_folder,
                            folder_icon: "folder-cloud",
                        });
                    }
                }
            }
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Open / fetch
    // -----------------------------------------------------------------------

    pub async fn fetch_document(&mut self, list_id: &str) -> Result<ListDocument> {
        let entry = self
            .lists
            .iter()
            .find(|l| l.id == list_id)
            .ok_or_else(|| anyhow::anyhow!("list not found: {list_id}"))?
            .clone();

        match &entry.source {
            ListSource::Nextcloud { file_name, remote_path } => {
                let (doc, from_cache) = self.nc.open_file_at(file_name, Some(remote_path)).await?;
                if from_cache {
                    self.spawn_nc_background_sync(file_name.clone(), remote_path.clone());
                }
                self.update_cache(list_id, doc.clone());
                Ok(doc)
            }
            ListSource::ExternalFile { path } => {
                let bytes = std::fs::read(path)?;
                let mut doc: ListDocument = serde_json::from_slice(&bytes)?;
                purge_old_deleted_items(&mut doc);
                self.update_cache(list_id, doc.clone());
                Ok(doc)
            }
        }
    }

    /// `remote_path` may be a full path ("/Lists/groceries.listie") or bare filename.
    pub async fn open_remote_list(&mut self, remote_path: &str) -> anyhow::Result<String> {
        // Split into bare filename and full path.
        let file_name = remote_path.rsplit('/').next().unwrap_or(remote_path).to_string();
        let full_path = if remote_path.starts_with('/') {
            remote_path.to_string()
        } else {
            format!("{}/{}", self.nc.lists_remote_path().trim_end_matches('/'), remote_path)
        };
        if self.excluded_nc_ids.remove(&file_name) {
            ExcludedIds(self.excluded_nc_ids.clone()).save();
        }
        let runtime_id = format!("nextcloud:{full_path}");
        if self.lists.iter().any(|l| l.id == runtime_id) {
            return Ok(runtime_id);
        }
        let (doc, from_cache) = self.nc.open_file_at(&file_name, Some(&full_path)).await?;
        if from_cache {
            self.spawn_nc_background_sync(file_name.clone(), full_path.clone());
        }
        let new_uuid = doc.list.id.clone();
        if let Some(existing) = self.lists.iter().find(|l| l.original_file_id == new_uuid) {
            anyhow::bail!(
                "Duplicate list ID: \"{}\" is already open from {}",
                existing.name,
                describe_source(&existing.source)
            );
        }
        let nc_folder = nc_folder_from_path(&full_path);
        let unchecked = doc.active_items().filter(|i| !i.checked).count();
        let source = ListSource::Nextcloud { file_name: file_name.clone(), remote_path: full_path.clone() };
        self.lists.push(UnifiedList {
            id: runtime_id.clone(),
            original_file_id: new_uuid,
            name: doc.list.name.clone(),
            icon: doc.list.icon.clone(),
            emoji_icon: doc.list.emoji_icon.clone(),
            source,
            is_dirty: false,
            unchecked_count: unchecked,
            folder: nc_folder,
            folder_icon: "folder-cloud",
        });
        self.cache.insert(runtime_id.clone(), doc);
        // Persist so this file reloads on the next launch.
        NcOpenedFiles::load().add(&file_name, &full_path, self.nc.server_url());
        Ok(runtime_id)
    }

    /// Load previously-opened NC files from disk at startup (fast, no network).
    /// Returns the list of file_names that were loaded from disk cache.
    /// Caller should trigger a background network refresh for each.
    pub fn load_nc_opened_from_disk(&mut self) {
        let records = NcOpenedFiles::load();
        let lists_path = self.nc.lists_remote_path().trim_end_matches('/').to_string();
        for record in records.0 {
            let file_name = record.file_name.clone();
            if self.excluded_nc_ids.contains(&file_name) {
                continue;
            }
            // Use stored remote_path if present; fall back for old records that predate this field.
            let remote_path = if record.remote_path.is_empty() {
                format!("{}/{}", lists_path, file_name)
            } else {
                record.remote_path.clone()
            };
            let runtime_id = format!("nextcloud:{remote_path}");
            if self.lists.iter().any(|l| l.id == runtime_id) {
                continue;
            }
            let nc_folder = nc_folder_from_path(&remote_path);
            if let Some(doc) = self.nc.open_from_disk_cache(&remote_path) {
                let new_uuid = doc.list.id.clone();
                if let Some(existing) = self.lists.iter().find(|l| l.original_file_id == new_uuid) {
                    warn!(
                        "skipping NC disk entry {file_name}: UUID {new_uuid} already loaded as \"{}\"",
                        existing.name
                    );
                    continue;
                }
                let unchecked = doc.active_items().filter(|i| !i.checked).count();
                let source = ListSource::Nextcloud { file_name, remote_path };
                self.lists.push(UnifiedList {
                    id: runtime_id.clone(),
                    original_file_id: new_uuid,
                    name: doc.list.name.clone(),
                    icon: doc.list.icon.clone(),
                    emoji_icon: doc.list.emoji_icon.clone(),
                    source,
                    is_dirty: false,
                    unchecked_count: unchecked,
                    folder: nc_folder,
                    folder_icon: "folder-cloud",
                });
                self.cache.insert(runtime_id.clone(), doc);
                // Mark sync-pending until a network refresh confirms the server state.
                self.sync_pending_ids.insert(runtime_id);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Create / save
    // -----------------------------------------------------------------------

    pub async fn create_list(&mut self, name: impl Into<String>) -> Result<String> {
        let doc = ListDocument::new(name);
        let doc_uuid = doc.list.id.clone();
        let file_name = format!("{}.listie", doc_uuid);
        let lists_path = self.nc.lists_remote_path().to_string();
        let remote_path = format!("{}/{}", lists_path.trim_end_matches('/'), file_name);
        let nc_folder = nc_folder_name(&lists_path);
        let runtime_id = format!("nextcloud:{remote_path}");
        let source = ListSource::Nextcloud { file_name, remote_path };
        self.lists.push(UnifiedList {
            id: runtime_id.clone(),
            original_file_id: doc_uuid,
            name: doc.list.name.clone(),
            icon: doc.list.icon.clone(),
            emoji_icon: doc.list.emoji_icon.clone(),
            source,
            is_dirty: false,
            unchecked_count: 0,
            folder: nc_folder,
            folder_icon: "folder-cloud",
        });
        self.update_cache(&runtime_id, doc.clone());
        self.save_list(&runtime_id).await?;
        Ok(runtime_id)
    }

    /// Create a new local list as a `.listie` file in the app data directory and add it
    /// to the sidebar (Swift `NewListView`'s private list — no Nextcloud required).
    /// Returns the runtime id.
    pub fn create_local_list(&mut self, name: impl Into<String>) -> Result<String> {
        let doc = ListDocument::new(name);
        let dir = crate::engine::welcome::local_data_dir();
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.listie", doc.list.id));
        std::fs::write(&path, serde_json::to_vec_pretty(&doc)?)?;
        self.open_external_file(path)
    }

    /// Open an external .listie file and add it to the sidebar.
    pub fn open_external_file(&mut self, path: PathBuf) -> Result<String> {
        let runtime_id = format!("external:{}", path.display());

        // Same source already loaded → no-op, just refresh persistence.
        if self.lists.iter().any(|l| l.id == runtime_id) {
            ExternalOpenedFiles::load().add(&path);
            return Ok(runtime_id);
        }

        let bytes = std::fs::read(&path)?;
        let mut doc: ListDocument = serde_json::from_slice(&bytes)?;
        purge_old_deleted_items(&mut doc);

        // Same UUID, different source → refuse (matches Swift "Duplicate list ID").
        let new_uuid = doc.list.id.clone();
        if let Some(existing) = self.lists.iter().find(|l| l.original_file_id == new_uuid) {
            anyhow::bail!(
                "Duplicate list ID: \"{}\" is already open from {}",
                existing.name,
                describe_source(&existing.source)
            );
        }

        let name = doc.list.name.clone();
        let icon = doc.list.icon.clone();
        let emoji_icon = doc.list.emoji_icon.clone();
        let folder = external_folder_name(&path);
        let source = ListSource::ExternalFile { path: path.clone() };
        self.lists.push(UnifiedList {
            id: runtime_id.clone(),
            original_file_id: new_uuid,
            name,
            icon,
            emoji_icon,
            source,
            is_dirty: false,
            unchecked_count: 0,
            folder,
            folder_icon: "folder",
        });
        self.update_cache(&runtime_id, doc);
        ExternalOpenedFiles::load().add(&path);
        Ok(runtime_id)
    }

    /// Load previously-opened external files at startup.
    /// Skips records whose file is no longer readable or fails to parse.
    pub fn load_external_opened_from_disk(&mut self) {
        // First-run welcome seed: if the sentinel is missing, write the
        // welcome `.listie` file to ~/.local/share and register it as an
        // opened external file so it shows up in the sidebar like any
        // other list. Subsequent launches see the sentinel and skip.
        if let Some(path) = crate::engine::welcome::seed_welcome_list_if_first_run() {
            let mut opened = ExternalOpenedFiles::load();
            opened.add(&path);
        }

        let records = ExternalOpenedFiles::load();
        for path in records.0 {
            let runtime_id = format!("external:{}", path.display());
            if self.lists.iter().any(|l| l.id == runtime_id) {
                continue;
            }
            if let Ok(bytes) = std::fs::read(&path) {
                if let Ok(mut doc) = serde_json::from_slice::<ListDocument>(&bytes) {
                    purge_old_deleted_items(&mut doc);
                    let new_uuid = doc.list.id.clone();
                    if let Some(existing) = self.lists.iter().find(|l| l.original_file_id == new_uuid) {
                        warn!(
                            "skipping external file {}: UUID {new_uuid} already loaded as \"{}\"",
                            path.display(),
                            existing.name
                        );
                        continue;
                    }
                    let name = doc.list.name.clone();
                    let icon = doc.list.icon.clone();
                    let emoji_icon = doc.list.emoji_icon.clone();
                    let folder = external_folder_name(&path);
                    let unchecked = doc.active_items().filter(|i| !i.checked).count();
                    let source = ListSource::ExternalFile { path };
                    self.lists.push(UnifiedList {
                        id: runtime_id.clone(),
                        original_file_id: new_uuid,
                        name,
                        icon,
                        emoji_icon,
                        source,
                        is_dirty: false,
                        unchecked_count: unchecked,
                        folder,
                        folder_icon: "folder",
                    });
                    self.update_cache(&runtime_id, doc);
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Mutations
    // -----------------------------------------------------------------------

    pub fn add_item(&mut self, list_id: &str, item: ListItem) {
        if let Some(doc) = self.cache.get_mut(list_id) {
            doc.items.push(item);
            self.mark_dirty(list_id);
        }
    }

    pub fn update_item(&mut self, list_id: &str, item: ListItem) {
        if let Some(doc) = self.cache.get_mut(list_id) {
            if let Some(existing) = doc.items.iter_mut().find(|i| i.id == item.id) {
                *existing = item;
                self.mark_dirty(list_id);
            }
        }
    }

    pub fn soft_delete_item(&mut self, list_id: &str, item_id: Uuid) {
        if let Some(doc) = self.cache.get_mut(list_id) {
            if let Some(item) = doc.items.iter_mut().find(|i| i.id == item_id) {
                item.soft_delete();
                item.last_change_field = Some("deleted".into());
                self.mark_dirty(list_id);
            }
        }
    }

    pub fn restore_item(&mut self, list_id: &str, item_id: Uuid) {
        if let Some(doc) = self.cache.get_mut(list_id) {
            if let Some(item) = doc.items.iter_mut().find(|i| i.id == item_id) {
                item.restore();
                item.last_change_field = Some("restored".into());
                self.mark_dirty(list_id);
            }
        }
    }

    pub fn permanently_delete_item(&mut self, list_id: &str, item_id: Uuid) {
        if let Some(doc) = self.cache.get_mut(list_id) {
            doc.items.retain(|i| i.id != item_id);
            self.mark_dirty(list_id);
        }
    }

    /// Rename a list. Mirrors Swift `UnifiedListProvider.updateList(name:...)`
    /// (UnifiedListProvider.swift:927-942): updates the in-memory document, the
    /// sidebar entry, touches `modifiedAt`, and schedules autosave. The on-disk
    /// filename never changes — for both NC and external files the file is keyed
    /// by UUID, not display name.
    pub fn rename_list(&mut self, list_id: &str, new_name: &str) {
        let trimmed = new_name.trim();
        if trimmed.is_empty() {
            return;
        }
        if let Some(doc) = self.cache.get_mut(list_id) {
            if doc.list.name == trimmed {
                return;
            }
            doc.list.name = trimmed.to_string();
            doc.list.touch();
            if let Some(entry) = self.lists.iter_mut().find(|l| l.id == list_id) {
                entry.name = trimmed.to_string();
            }
            self.mark_dirty(list_id);
        }
    }

    /// Permanently delete a list and its underlying file. Mirrors Swift
    /// `UnifiedListProvider.deleteList` (UnifiedListProvider.swift:961-975): for
    /// `.nextcloud` issues a WebDAV DELETE and wipes per-file caches; for
    /// `.external` removes the on-disk `.listie` file. In both cases the entry
    /// is dropped from the sidebar, the document cache, sync state, and the
    /// persisted opened-files index. Reminders for items in the list are
    /// cancelled. Unlike `exclude_list`, the NC file is not added to the
    /// excluded-ids set — if a file with the same name reappears later it
    /// should re-import normally.
    pub async fn delete_list_permanently(&mut self, list_id: &str) -> anyhow::Result<()> {
        let Some(entry) = self.lists.iter().find(|l| l.id == list_id).cloned() else {
            return Ok(());
        };

        if let Some(handle) = self.autosave_handles.remove(list_id) {
            handle.abort();
        }

        let item_ids: Vec<String> = self
            .cache
            .get(list_id)
            .map(|doc| doc.items.iter().map(|i| i.id.to_string()).collect())
            .unwrap_or_default();
        for id in item_ids {
            self.reminder_engine.cancel(&id);
        }

        match &entry.source {
            ListSource::Nextcloud { file_name, remote_path } => {
                self.nc.delete_remote(remote_path).await?;
                NcOpenedFiles::load().remove(file_name);
            }
            ListSource::ExternalFile { path } => {
                if path.exists() {
                    std::fs::remove_file(path)?;
                }
                ExternalOpenedFiles::load().remove(path);
            }
        }

        self.lists.retain(|l| l.id != list_id);
        self.cache.remove(list_id);
        self.sync_pending_ids.remove(list_id);
        Ok(())
    }

    pub fn set_list_emoji_icon(&mut self, list_id: &str, emoji: Option<String>) {
        if let Some(doc) = self.cache.get_mut(list_id) {
            doc.list.emoji_icon = emoji.clone();
            doc.list.touch();
            if let Some(entry) = self.lists.iter_mut().find(|l| l.id == list_id) {
                entry.emoji_icon = emoji;
            }
            self.mark_dirty(list_id);
        }
    }

    /// Flip whether `label_id` is in `hidden_labels` for the list. Mirrors
    /// `ExternalFileStore.updateList(hiddenLabels:)` (ExternalFileStore.swift:898).
    pub fn toggle_label_hidden(&mut self, list_id: &str, label_id: &str) {
        if let Some(doc) = self.cache.get_mut(list_id) {
            let pos = doc.list.hidden_labels.iter().position(|id| id == label_id);
            match pos {
                Some(i) => {
                    doc.list.hidden_labels.remove(i);
                }
                None => {
                    doc.list.hidden_labels.push(label_id.to_string());
                }
            }
            doc.list.touch();
            self.mark_dirty(list_id);
        }
    }

    pub fn is_label_hidden(&self, list_id: &str, label_id: &str) -> bool {
        self.cache
            .get(list_id)
            .map(|doc| doc.list.hidden_labels.iter().any(|id| id == label_id))
            .unwrap_or(false)
    }

    pub fn set_list_enable_map_data(&mut self, list_id: &str, enabled: bool) {
        if let Some(doc) = self.cache.get_mut(list_id) {
            doc.list.enable_map_data = enabled;
            doc.list.touch();
            self.mark_dirty(list_id);
        }
    }

    pub fn add_label(&mut self, list_id: &str, label: ListLabel) {
        if let Some(doc) = self.cache.get_mut(list_id) {
            doc.labels.push(label);
            doc.list.touch();
            self.mark_dirty(list_id);
        }
    }

    pub fn update_label(&mut self, list_id: &str, label: ListLabel) {
        if let Some(doc) = self.cache.get_mut(list_id) {
            if let Some(existing) = doc.labels.iter_mut().find(|l| l.id == label.id) {
                // Update only the fields the editor manages. `symbol` (the Swift SF Symbol)
                // and `extra` (preserved unknown keys) have no GNOME editor, so carry them
                // over — a wholesale replace would wipe a round-tripped Swift label's icon.
                existing.name = label.name;
                existing.color = label.color;
                existing.emoji_icon = label.emoji_icon;
                doc.list.touch();
                self.mark_dirty(list_id);
            }
        }
    }

    pub fn move_label_order(&mut self, list_id: &str, from: usize, to: usize) {
        if let Some(doc) = self.cache.get_mut(list_id) {
            let order = &mut doc.list.label_order;
            if from < order.len() && to < order.len() {
                let id = order.remove(from);
                order.insert(to, id);
                doc.list.touch();
                self.mark_dirty(list_id);
            }
        }
    }

    pub fn delete_label(&mut self, list_id: &str, label_id: &str) {
        if let Some(doc) = self.cache.get_mut(list_id) {
            doc.labels.retain(|l| l.id != label_id);
            doc.deleted_label_ids.push(label_id.to_string());
            // Remove label reference from all items.
            for item in doc.items.iter_mut() {
                if item.label_id.as_deref() == Some(label_id) {
                    item.label_id = None;
                    item.touch();
                }
            }
            doc.list.touch();
            self.mark_dirty(list_id);
        }
    }

    // -----------------------------------------------------------------------
    // Save
    // -----------------------------------------------------------------------

    /// Trigger a debounced autosave (500 ms) for the given list.
    pub fn trigger_autosave(&mut self, list_id: &str) {
        if let Some(handle) = self.autosave_handles.remove(list_id) {
            handle.abort();
        }
        tracing::info!("[autosave] scheduled for {list_id}");
        let list_id = list_id.to_string();
        let provider = crate::engine::provider_singleton::get();
        let task_id = list_id.clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            tracing::info!("[autosave] firing save_list for {task_id}");

            // Phase 1: snapshot what we need to save, then release the lock.
            let snapshot = {
                let p = provider.lock().await;
                p.lists.iter().find(|l| l.id == task_id).and_then(|entry| {
                    p.cache.get(&task_id).map(|doc| {
                        (entry.source.clone(), doc.clone(), p.nc.clone())
                    })
                })
            };

            let Some((source, doc, mut nc)) = snapshot else { return };

            // Phase 2: network I/O outside the lock.
            let result = match &source {
                ListSource::Nextcloud { file_name, remote_path } => {
                    nc.save_file(file_name, remote_path, &doc).await
                        .map(|_| Some((file_name.clone(), nc)))
                }
                ListSource::ExternalFile { path } => {
                    let json = serde_json::to_vec_pretty(&doc)
                        .map_err(anyhow::Error::from)
                        .and_then(|b| atomic_write(path, &b));
                    json.map(|_| None)
                }
            };

            // Phase 3: write results back under a brief lock.
            let mut p = provider.lock().await;
            match result {
                Ok(Some((file_name, nc_done))) => {
                    p.nc.absorb(nc_done);
                    let unchecked = p.cache.get(&task_id)
                        .map(|d| d.active_items().filter(|i| !i.checked).count())
                        .unwrap_or(0);
                    if let Some(e) = p.lists.iter_mut().find(|l| l.id == task_id) {
                        e.is_dirty = false;
                        e.unchecked_count = unchecked;
                    }
                    tracing::info!("[autosave] save OK for {file_name}");
                }
                Ok(None) => {
                    if let Some(e) = p.lists.iter_mut().find(|l| l.id == task_id) {
                        e.is_dirty = false;
                    }
                }
                Err(e) => {
                    tracing::warn!("[autosave] save FAILED for {task_id}: {e}");
                }
            }
        });
        self.autosave_handles.insert(list_id, handle);
    }

    pub async fn save_list(&mut self, list_id: &str) -> Result<()> {
        let entry = self
            .lists
            .iter_mut()
            .find(|l| l.id == list_id)
            .ok_or_else(|| anyhow::anyhow!("list not found: {list_id}"))?;

        let doc = self
            .cache
            .get(list_id)
            .ok_or_else(|| anyhow::anyhow!("no cached document for {list_id}"))?
            .clone();

        let unchecked_count = doc.active_items().filter(|i| !i.checked).count();

        match &entry.source.clone() {
            ListSource::Nextcloud { file_name, remote_path } => {
                tracing::info!("[save_list] NC  file={file_name}  remote_path={remote_path}");
                self.nc.save_file(file_name, remote_path, &doc).await?;
            }
            ListSource::ExternalFile { path } => {
                tracing::info!("[save_list] external  path={}", path.display());
                let json = serde_json::to_vec_pretty(&doc)?;
                atomic_write(path, &json)?;
            }
        }

        if let Some(e) = self.lists.iter_mut().find(|l| l.id == list_id) {
            e.is_dirty = false;
            e.unchecked_count = unchecked_count;
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn update_cache(&mut self, list_id: &str, doc: ListDocument) {
        let unchecked = doc.active_items().filter(|i| !i.checked).count();
        if let Some(entry) = self.lists.iter_mut().find(|l| l.id == list_id) {
            entry.name = doc.list.name.clone();
            entry.icon = doc.list.icon.clone();
            entry.emoji_icon = doc.list.emoji_icon.clone();
            entry.original_file_id = doc.list.id.clone();
            entry.unchecked_count = unchecked;
        }
        self.cache.insert(list_id.to_string(), doc);
    }

    fn mark_dirty(&mut self, list_id: &str) {
        if let Some(entry) = self.lists.iter_mut().find(|l| l.id == list_id) {
            entry.is_dirty = true;
            if let Some(doc) = self.cache.get(list_id) {
                entry.unchecked_count = doc.active_items().filter(|i| !i.checked).count();
            }
        }
    }

    pub fn cached_doc(&self, list_id: &str) -> Option<&ListDocument> {
        self.cache.get(list_id)
    }

    /// Apply one sync result: updates cache and dirty state, returns true if the
    /// doc actually changed (so callers know whether to notify the UI).
    pub fn apply_sync_result(&mut self, list_id: &str, doc: ListDocument) -> bool {
        let prev_modified = self.cache.get(list_id).map(|d| d.list.modified_at);
        let did_change = prev_modified.map(|t| t != doc.list.modified_at).unwrap_or(true);
        self.update_cache(list_id, doc);
        did_change
    }

    pub fn cache_names(&self) -> std::collections::HashMap<String, String> {
        self.cache.iter()
            .map(|(id, doc)| (id.clone(), doc.list.name.clone()))
            .collect()
    }

    /// Mark an NC list as user-excluded so sync never re-adds it.
    /// Also removes it from the in-memory list, cache, and persisted opened-files list.
    pub fn exclude_list(&mut self, id: &str) {
        if let Some(entry) = self.lists.iter().find(|l| l.id == id) {
            match &entry.source {
                ListSource::Nextcloud { file_name, .. } => {
                    self.excluded_nc_ids.insert(file_name.clone());
                    ExcludedIds(self.excluded_nc_ids.clone()).save();
                    NcOpenedFiles::load().remove(file_name);
                }
                ListSource::ExternalFile { path } => {
                    ExternalOpenedFiles::load().remove(path);
                }
            }
        }
        self.lists.retain(|l| l.id != id);
        self.cache.remove(id);
        self.sync_pending_ids.remove(id);
    }

    /// Sync all NC lists: retry pending uploads, then run bidirectional sync for each.
    /// Call on window focus / periodic timer.
    /// Re-attempt NC lists that failed at startup (network down, timeout).
    /// 404 errors are treated as permanent; other errors leave the entry for the next retry.
    pub async fn retry_unavailable_nc_lists(&mut self) {
        let unavailable: Vec<(String, String, String)> = self.lists.iter()
            .filter_map(|l| {
                if let ListSource::Nextcloud { file_name, remote_path } = &l.source {
                    if !self.cache.contains_key(&l.id) && self.nc.load_from_disk(remote_path).is_none() {
                        return Some((l.id.clone(), file_name.clone(), remote_path.clone()));
                    }
                }
                None
            })
            .collect();

        for (list_id, file_name, remote_path) in unavailable {
            match self.nc.open_file_at(&file_name, Some(&remote_path)).await {
                Ok((doc, from_cache)) => {
                    if from_cache {
                        self.spawn_nc_background_sync(file_name.clone(), remote_path.clone());
                    }
                    self.sync_pending_ids.remove(&list_id);
                    self.update_cache(&list_id, doc);
                }
                Err(e) => {
                    let msg = e.to_string();
                    if e.is::<FileGone>() || msg.contains("404") || msg.contains("Not Found") {
                        // Permanently gone — remove from opened files and sidebar.
                        self.lists.retain(|l| l.id != list_id);
                        NcOpenedFiles::load().remove(&file_name);
                    }
                    // Other errors: leave for next retry.
                }
            }
        }
    }

    /// Returns the list IDs whose cached document actually changed (server had a newer version).
    pub async fn sync_all_lists(&mut self) -> Vec<String> {
        self.retry_unavailable_nc_lists().await;
        if let Err(e) = self.nc.retry_pending_uploads().await {
            warn!("retry_pending_uploads error: {e}");
        }

        let nc_list_ids: Vec<(String, String, String)> = self.lists.iter()
            .filter_map(|l| {
                if let ListSource::Nextcloud { file_name, remote_path } = &l.source {
                    Some((l.id.clone(), file_name.clone(), remote_path.clone()))
                } else {
                    None
                }
            })
            .collect();

        let mut changed_ids = Vec::new();
        for (list_id, file_name, remote_path) in nc_list_ids {
            let prev_modified = self.cache.get(&list_id).map(|d| d.list.modified_at);
            match self.nc.sync_file(&file_name, &remote_path).await {
                Ok(mut doc) => {
                    self.sync_pending_ids.remove(&list_id);
                    purge_old_deleted_items(&mut doc);
                    let did_change = prev_modified.map(|t| t != doc.list.modified_at).unwrap_or(true);
                    self.update_cache(&list_id, doc);
                    if did_change {
                        changed_ids.push(list_id);
                    }
                }
                Err(e) if e.is::<FileGone>() => {
                    // Confirmed deleted on the server — drop locally (Swift removes the list
                    // on `nextcloudFileNotFound`).
                    warn!("sync_all_lists: {file_name} gone on server — removing locally");
                    self.lists.retain(|l| l.id != list_id);
                    self.cache.remove(&list_id);
                    self.sync_pending_ids.remove(&list_id);
                    NcOpenedFiles::load().remove(&file_name);
                }
                Err(e) => {
                    warn!("sync_all_lists: sync failed for {file_name}: {e}");
                    self.sync_pending_ids.insert(list_id);
                }
            }
        }
        changed_ids
    }

    /// Iterate all cached list documents together with their list metadata.
    pub fn all_cached_docs(&self) -> impl Iterator<Item = (&UnifiedList, &ListDocument)> {
        self.lists.iter().filter_map(move |l| self.cache.get(&l.id).map(|doc| (l, doc)))
    }

    /// Retrieve a single item by list_id + item_id string (for reminder tick callback).
    pub fn get_item(&self, list_id: &str, item_id: &str) -> Option<crate::model::ListItem> {
        let id: uuid::Uuid = item_id.parse().ok()?;
        self.cache.get(list_id)?.items.iter().find(|i| i.id == id).cloned()
    }

    /// Rebuild the in-memory pending-reminders registry from the items cache. Mirrors
    /// Swift `ReminderManager.reconcileWithBudget` (ReminderManager.swift:172-242):
    /// the source of truth is the items on disk, not the OS notification queue. Called
    /// after the cache is populated at launch and after each NC sync so newly-arrived
    /// items get scheduled and previously-scheduled items survive a restart.
    pub fn reconcile_reminders(&mut self) {
        // Snapshot what needs to be scheduled, then mutate engine state.
        let now = chrono::Utc::now();
        let snapshot: Vec<(ListItem, String, String)> = self.lists.iter()
            .filter_map(|l| self.cache.get(&l.id).map(|doc| (l.clone(), doc.clone())))
            .flat_map(|(l, doc)| {
                doc.items.into_iter()
                    .filter(|i| !i.checked && !i.is_deleted && i.reminder_date.map_or(false, |d| d > now))
                    .map(move |i| (i, l.id.clone(), l.name.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();

        // Drop any pending entries whose item no longer qualifies (e.g. was completed
        // or deleted while the app was closed). Compare by item id.
        let valid_ids: std::collections::HashSet<String> = snapshot.iter()
            .map(|(i, _, _)| i.id.to_string())
            .collect();
        let stale: Vec<String> = self.reminder_engine.pending.keys()
            .filter(|k| !valid_ids.contains(*k))
            .cloned()
            .collect();
        for id in stale {
            self.reminder_engine.cancel(&id);
        }

        // Schedule (or re-schedule) the valid ones.
        for (item, list_id, list_name) in snapshot {
            self.reminder_engine.schedule(&item, &list_id, &list_name);
        }
        tracing::info!("[reconcile_reminders] pending count = {}", self.reminder_engine.pending.len());
    }

    /// Schedule or cancel an OS-level reminder for `item`. Mirrors Swift's
    /// `ListViewModel`-side scheduling (e.g. ListViewModel.swift:202-205, 259-264, 383-389):
    /// schedule when the item has a future reminder and isn't checked; cancel otherwise.
    /// Called by bridge mutators (`add_item`, `toggle_checked`, `edit_item`,
    /// `delete_item`) so the OS notification queue stays in sync with the model.
    pub fn sync_reminder_for_item(&mut self, list_id: &str, item: &ListItem) {
        let list_name = self.lists.iter()
            .find(|l| l.id == list_id)
            .map(|l| l.name.clone())
            .unwrap_or_default();
        let id_str = item.id.to_string();
        let should_schedule = !item.checked
            && !item.is_deleted
            && item.reminder_date.map_or(false, |d| d > chrono::Utc::now());
        if should_schedule {
            self.reminder_engine.schedule(item, list_id, &list_name);
        } else {
            self.reminder_engine.cancel(&id_str);
        }
    }

    /// Mark an item as checked (used internally).
    pub fn mark_item_complete(&mut self, list_id: &str, item_id: &str) {
        let Some(id) = item_id.parse::<uuid::Uuid>().ok() else { return };
        let Some(doc) = self.cache.get(list_id) else { return };
        let Some(item) = doc.items.iter().find(|i| i.id == id).cloned() else { return };
        let mut updated = item;
        updated.checked = true;
        updated.touch();
        self.update_item(list_id, updated);
        self.trigger_autosave(list_id);
    }

    /// Handle the "Mark complete" notification action. Mirrors Swift
    /// `ReminderManager.completeItemFromNotification` (ReminderManager.swift:312-371):
    /// - Repeating: advance reminder_date to next occurrence, keep unchecked, reschedule.
    /// - One-off:   set checked=true, clear reminder_date, cancel.
    pub fn complete_item_from_notification(&mut self, list_id: &str, item_id: &str) {
        let Some(id) = item_id.parse::<uuid::Uuid>().ok() else { return };
        let Some(item) = self.cache.get(list_id)
            .and_then(|doc| doc.items.iter().find(|i| i.id == id).cloned()) else { return };

        let now = chrono::Utc::now();
        let mut updated = item.clone();
        updated.touch();

        if let (Some(rule), Some(_), Some(fire_at)) = (
            &item.reminder_repeat_rule,
            &item.reminder_repeat_mode,
            item.reminder_date,
        ) {
            let mode = item.reminder_repeat_mode.clone().unwrap_or(crate::model::reminder::ReminderRepeatMode::Fixed);
            let next = crate::engine::recurrence::next_reminder_date(fire_at, rule, &mode, now);
            updated.checked = false;
            updated.reminder_date = Some(next);
        } else {
            updated.checked = true;
            updated.reminder_date = None;
        }

        self.update_item(list_id, updated.clone());
        self.sync_reminder_for_item(list_id, &updated);
        self.trigger_autosave(list_id);
    }

    /// Remove all NC lists and clear exclusions (called on logout).
    pub fn clear_nc_lists(&mut self) {
        let nc_ids: std::collections::HashSet<String> = self.lists.iter()
            .filter(|l| matches!(&l.source, ListSource::Nextcloud { .. }))
            .map(|l| l.id.clone())
            .collect();
        self.lists.retain(|l| !matches!(&l.source, ListSource::Nextcloud { .. }));
        for id in &nc_ids {
            self.cache.remove(id);
        }
        self.excluded_nc_ids.clear();
        ExcludedIds::default().save();
        NcOpenedFiles::default().save();
        self.sync_pending_ids.clear();
    }

    /// Spawn a fire-and-forget background sync for an NC file that was just served from
    /// cache (mem or disk). Mirrors Swift `NextcloudManager.openFile`
    /// (NextcloudManager.swift:215, 225). The task runs on a clone of `self.nc` outside
    /// the provider lock; if the server has a newer version it's absorbed into the
    /// provider's mem_cache via `absorb_mem_entry` (targeted, not the wholesale
    /// `absorb` — avoids racing with other concurrent background tasks).
    fn spawn_nc_background_sync(&self, file_name: String, remote_path: String) {
        let mut nc = self.nc.clone();
        tokio::spawn(async move {
            match nc.background_sync(&file_name, &remote_path).await {
                Ok(true) => {
                    if let Some(new_doc) = nc.mem_cache_get(&remote_path) {
                        let provider = crate::engine::provider_singleton::get();
                        let mut p = provider.lock().await;
                        p.nc.absorb_mem_entry(&remote_path, new_doc);
                    }
                }
                Ok(false) => {} // unchanged or unreachable
                Err(e) if e.is::<FileGone>() => {
                    // Confirmed deleted on the server — drop locally (Swift posts
                    // `nextcloudFileNotFound`, which removes the list).
                    let runtime_id = format!("nextcloud:{remote_path}");
                    let provider = crate::engine::provider_singleton::get();
                    let mut p = provider.lock().await;
                    p.lists.retain(|l| l.id != runtime_id);
                    p.cache.remove(&runtime_id);
                    p.sync_pending_ids.remove(&runtime_id);
                    NcOpenedFiles::load().remove(&file_name);
                    tracing::warn!("[bg_sync] {file_name} gone on server — removed locally");
                }
                Err(e) => tracing::warn!("[bg_sync] failed for {file_name}: {e}"),
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Atomic write
// ---------------------------------------------------------------------------

fn atomic_write(path: &PathBuf, data: &[u8]) -> Result<()> {
    let tmp = path.with_extension("listie.tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Persisted list of NC files the user has explicitly opened
// Mirrors Swift's "com.listie.nextcloud-files" UserDefaults entry.
// ---------------------------------------------------------------------------

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct NcOpenedFiles(Vec<NcFileRecord>);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct NcFileRecord {
    file_name: String,
    server_url: String,
    /// Full remote path from the DAV root, e.g. "/Lists/Tests/groceries.listie".
    /// Empty string in records written before this field was added (fall back to
    /// lists_remote_path reconstruction in that case).
    #[serde(default)]
    remote_path: String,
}

impl NcOpenedFiles {
    fn path() -> std::path::PathBuf {
        dirs::state_dir()
            .unwrap_or_else(|| dirs::home_dir().unwrap().join(".local/state"))
            .join("quite-listie")
            .join("nc-opened-files.json")
    }

    fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|d| serde_json::from_str(&d).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        let path = Self::path();
        if let Some(p) = path.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        if let Ok(json) = serde_json::to_string(self) {
            let _ = std::fs::write(path, json);
        }
    }

    fn add(&mut self, file_name: &str, remote_path: &str, server_url: &str) {
        // Update existing record (remote_path may have changed) or insert new.
        if let Some(existing) = self.0.iter_mut().find(|r| r.file_name == file_name) {
            if existing.remote_path != remote_path {
                existing.remote_path = remote_path.to_string();
                self.save();
            }
        } else {
            self.0.push(NcFileRecord {
                file_name: file_name.to_string(),
                server_url: server_url.to_string(),
                remote_path: remote_path.to_string(),
            });
            self.save();
        }
    }

    fn remove(&mut self, file_name: &str) {
        let before = self.0.len();
        self.0.retain(|r| r.file_name != file_name);
        if self.0.len() != before {
            self.save();
        }
    }
}

// ---------------------------------------------------------------------------
// Persisted list of external .listie files the user has opened.
// Stored on disk so they reappear in the sidebar across launches.
// ---------------------------------------------------------------------------

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct ExternalOpenedFiles(Vec<PathBuf>);

impl ExternalOpenedFiles {
    fn path() -> PathBuf {
        dirs::state_dir()
            .unwrap_or_else(|| dirs::home_dir().unwrap().join(".local/state"))
            .join("quite-listie")
            .join("external-opened-files.json")
    }

    fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|d| serde_json::from_str(&d).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        let path = Self::path();
        if let Some(p) = path.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        if let Ok(json) = serde_json::to_string(self) {
            let _ = std::fs::write(path, json);
        }
    }

    fn add(&mut self, path: &Path) {
        if !self.0.iter().any(|p| p == path) {
            self.0.push(path.to_path_buf());
            self.save();
        }
    }

    fn remove(&mut self, path: &Path) {
        let before = self.0.len();
        self.0.retain(|p| p != path);
        if self.0.len() != before {
            self.save();
        }
    }
}

// ---------------------------------------------------------------------------
// Excluded NC IDs (persisted across runs)
// ---------------------------------------------------------------------------

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct ExcludedIds(std::collections::HashSet<String>);

impl ExcludedIds {
    fn path() -> std::path::PathBuf {
        dirs::state_dir()
            .unwrap_or_else(|| dirs::home_dir().unwrap().join(".local/state"))
            .join("quite-listie")
            .join("nc-excluded.json")
    }

    fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|d| serde_json::from_str(&d).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        let path = Self::path();
        if let Some(p) = path.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        if let Ok(json) = serde_json::to_string(self) {
            let _ = std::fs::write(path, json);
        }
    }
}

// ---------------------------------------------------------------------------
// Folder name helpers (match Swift sidebar section logic)
// ---------------------------------------------------------------------------

/// Derive a human-readable folder name from the Nextcloud lists remote path.
/// e.g. "/Lists" → "Lists", "/" → "Nextcloud", "" → "Nextcloud"
pub fn nc_folder_name(lists_remote_path: &str) -> String {
    lists_remote_path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("Nextcloud")
        .to_string()
}

/// Derive section name from a specific file's full remote path — matches Swift's nextcloudFolderName.
/// "/Lists/groceries.listie" → "Lists", "/Lists/Work/project.listie" → "Work", "/groceries.listie" → "Nextcloud"
pub fn nc_folder_from_path(remote_path: &str) -> String {
    let parts: Vec<&str> = remote_path.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() >= 2 {
        parts[parts.len() - 2].to_string()
    } else {
        "Nextcloud".to_string()
    }
}

/// Derive a human-readable folder name from a local file path.
/// Files in the home directory → "Home"; others → parent directory name.
pub fn external_folder_name(path: &std::path::Path) -> String {
    let parent = path.parent();
    let home = dirs::home_dir();
    if let (Some(p), Some(h)) = (parent, home.as_deref()) {
        if p == h {
            return "Home".to_string();
        }
    }
    parent
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("Local Files")
        .to_string()
}
