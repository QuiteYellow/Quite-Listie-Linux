//! Pure merge logic for importing parsed markdown into an existing list. Mirrors Swift
//! `MarkdownImportLogic` (the testable helpers behind `MarkdownListImportView`): name-based
//! item matching and merge statistics. Kept free of UI so it can be unit-tested.

use crate::model::{ListDocument, ListItem, ListLabel};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeStats {
    pub new_items: usize,
    pub updated_items: usize,
    pub new_labels: usize,
    pub matched_labels: usize,
    pub unmatched_labels: usize,
}

/// Find the existing item a parsed item should merge into: case-insensitive note match.
///
/// (Swift also does a UUID-first match using a preset snapshot; that only applies to the
/// preset-reload intent, not the paste/link import flows ported here, so we match by note.)
pub fn match_existing<'a>(parsed_note: &str, existing: &'a [ListItem]) -> Option<&'a ListItem> {
    let key = parsed_note.to_lowercase();
    existing.iter().find(|i| !i.is_deleted && i.note.to_lowercase() == key)
}

/// The label name a parsed item belongs to (via the parsed document's own labels).
pub fn parsed_label_name(parsed: &ListDocument, item: &ListItem) -> Option<String> {
    item.label_id
        .as_deref()
        .and_then(|id| parsed.labels.iter().find(|l| l.id == id))
        .map(|l| l.name.clone())
}

/// Merge statistics for a preview: how many items would be added vs updated, and how many
/// labels match / are missing (and would be created). Mirrors Swift `mergeStats`.
pub fn merge_stats(
    parsed: &ListDocument,
    existing_items: &[ListItem],
    existing_labels: &[ListLabel],
    create_unmatched_labels: bool,
) -> MergeStats {
    let mut stats = MergeStats::default();

    for item in &parsed.items {
        if match_existing(&item.note, existing_items).is_some() {
            stats.updated_items += 1;
        } else {
            stats.new_items += 1;
        }
    }

    let existing_names: std::collections::HashSet<String> =
        existing_labels.iter().map(|l| l.name.to_lowercase()).collect();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for item in &parsed.items {
        if let Some(name) = parsed_label_name(parsed, item) {
            if !seen.insert(name.to_lowercase()) {
                continue;
            }
            if existing_names.contains(&name.to_lowercase()) {
                stats.matched_labels += 1;
            } else {
                stats.unmatched_labels += 1;
            }
        }
    }
    stats.new_labels = if create_unmatched_labels { stats.unmatched_labels } else { 0 };
    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::markdown_parser::parse_markdown;

    #[test]
    fn merge_counts_new_and_updated() {
        let parsed = parse_markdown("# Shop\n## Produce\n- Apples\n- Bananas\n", None);
        let mut existing_apple = ListItem::new("apples"); // case-insensitive match
        existing_apple.quantity = 1.0;
        let existing_items = vec![existing_apple];
        let existing_labels: Vec<ListLabel> = vec![];

        let stats = merge_stats(&parsed, &existing_items, &existing_labels, true);
        assert_eq!(stats.updated_items, 1); // Apples
        assert_eq!(stats.new_items, 1); // Bananas
        assert_eq!(stats.unmatched_labels, 1); // Produce
        assert_eq!(stats.new_labels, 1);
    }

    #[test]
    fn matched_label_not_created() {
        let parsed = parse_markdown("# Shop\n## Produce\n- Apples\n", None);
        let existing_labels = vec![ListLabel::new("p", "produce", "#4CAF50")];
        let stats = merge_stats(&parsed, &[], &existing_labels, true);
        assert_eq!(stats.matched_labels, 1);
        assert_eq!(stats.new_labels, 0);
    }
}
