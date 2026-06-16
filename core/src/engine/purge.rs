use chrono::{Duration, Utc};

use crate::model::ListDocument;

const PURGE_AFTER_DAYS: i64 = 30;

/// Hard-delete items that have been soft-deleted for more than 30 days.
///
/// Mirrors Swift `UnifiedListProvider.cleanupOldDeletedItems` (UnifiedListProvider.swift:1109):
/// when `deletedAt` is missing (legacy items soft-deleted before the field existed) fall
/// back to `modifiedAt`, otherwise the item is purged on first save regardless of age.
pub fn purge_old_deleted_items(doc: &mut ListDocument) {
    let cutoff = Utc::now() - Duration::days(PURGE_AFTER_DAYS);
    doc.items.retain(|item| {
        if item.is_deleted {
            let deletion_date = item.deleted_at.unwrap_or(item.modified_at);
            deletion_date >= cutoff
        } else {
            true
        }
    });
}
