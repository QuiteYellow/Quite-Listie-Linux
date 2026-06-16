use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use crate::model::{ListDocument, ListItem, ListLabel};

/// Three-way merge of a local (in-memory) document and a remote (server) document.
/// Both sides are assumed to share a common ancestor that we don't have; we
/// resolve conflicts using per-item `modified_at` timestamps.
pub fn merge_documents(local: ListDocument, remote: ListDocument) -> ListDocument {
    // --- Items ---
    // Start with all remote items keyed by id.
    let mut merged: HashMap<Uuid, ListItem> =
        remote.items.into_iter().map(|i| (i.id, i)).collect();

    for local_item in local.items {
        match merged.get(&local_item.id) {
            None => {
                // Item only exists locally — keep it.
                merged.insert(local_item.id, local_item);
            }
            Some(remote_item) => {
                // Newer modifiedAt wins; remote wins on tie (Swift `ListDocument.merge`).
                // Deletions carry their own timestamp, so they merge by the same rule.
                if local_item.modified_at > remote_item.modified_at {
                    merged.insert(local_item.id, local_item);
                }
            }
        }
    }

    // --- Labels ---
    // Local is the base (local wins on conflict), remote adds labels the user hasn't seen yet.
    // Matches Swift: labelsById starts from local, server only inserts missing IDs.
    let mut merged_labels: HashMap<String, ListLabel> =
        local.labels.into_iter().map(|l| (l.id.clone(), l)).collect();
    for remote_label in remote.labels {
        merged_labels
            .entry(remote_label.id.clone())
            .or_insert(remote_label);
    }

    // --- Deleted label tombstones ---
    let mut deleted_label_ids: HashSet<String> =
        local.deleted_label_ids.into_iter().collect();
    deleted_label_ids.extend(remote.deleted_label_ids);

    // Remove labels that have been tombstoned.
    for id in &deleted_label_ids {
        merged_labels.remove(id);
    }

    // --- List summary ---
    let merged_summary = if local.list.modified_at > remote.list.modified_at {
        local.list
    } else {
        remote.list
    };

    ListDocument {
        version: 2,
        list: merged_summary,
        items: merged.into_values().collect(),
        labels: merged_labels.into_values().collect(),
        deleted_label_ids: deleted_label_ids.into_iter().collect(),
        extra: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ListDocument, ListItem};
    use chrono::{Duration, Utc};

    fn make_item(note: &str, modified_offset_secs: i64) -> ListItem {
        let mut item = ListItem::new(note);
        item.modified_at = Utc::now() + Duration::seconds(modified_offset_secs);
        item
    }

    #[test]
    fn local_only_item_is_kept() {
        let mut local = ListDocument::new("Test");
        local.items.push(make_item("local-only", 0));
        let remote = ListDocument::new("Test");
        let merged = merge_documents(local.clone(), remote);
        assert_eq!(merged.items.len(), 1);
        assert_eq!(merged.items[0].note, "local-only");
    }

    #[test]
    fn remote_newer_wins() {
        let item_local = make_item("item", -10);
        let mut item_remote = item_local.clone();
        item_remote.note = "updated-remote".into();
        item_remote.modified_at = Utc::now();

        let mut local = ListDocument::new("Test");
        local.items.push(item_local);
        let mut remote = ListDocument::new("Test");
        remote.items.push(item_remote);

        let merged = merge_documents(local, remote);
        assert_eq!(merged.items[0].note, "updated-remote");
    }

    #[test]
    fn local_deletion_propagates() {
        let mut item = make_item("item", 0);
        let id = item.id;
        let mut remote = ListDocument::new("Test");
        remote.items.push(item.clone());

        item.soft_delete();
        let mut local = ListDocument::new("Test");
        local.items.push(item);

        let merged = merge_documents(local, remote);
        let found = merged.items.iter().find(|i| i.id == id).unwrap();
        assert!(found.is_deleted);
    }

    #[test]
    fn remote_deletion_wins_when_newer() {
        // Remote deleted more recently than local's last edit → deletion propagates.
        let item = make_item("item", 0);
        let id = item.id;
        let mut deleted = item.clone();
        deleted.soft_delete(); // sets modified_at = now, which is after item's modified_at

        let mut local = ListDocument::new("Test");
        local.items.push(item);
        let mut remote = ListDocument::new("Test");
        remote.items.push(deleted);

        let merged = merge_documents(local, remote);
        let found = merged.items.iter().find(|i| i.id == id).unwrap();
        assert!(found.is_deleted);
    }

    #[test]
    fn local_restore_wins_when_newer() {
        // Item deleted remotely at t=-10, restored locally at t=+10 → restore wins.
        let id = make_item("item", 0).id;

        let mut remote_item = make_item("item", -10);
        remote_item.id = id;
        remote_item.is_deleted = true;
        remote_item.deleted_at = Some(Utc::now() - Duration::seconds(10));

        let mut local_item = make_item("item", 10);
        local_item.id = id;
        local_item.is_deleted = false;
        local_item.deleted_at = None;

        let mut local = ListDocument::new("Test");
        local.items.push(local_item);
        let mut remote = ListDocument::new("Test");
        remote.items.push(remote_item);

        let merged = merge_documents(local, remote);
        let found = merged.items.iter().find(|i| i.id == id).unwrap();
        assert!(!found.is_deleted, "newer restore should beat older deletion");
    }
}
