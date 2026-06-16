use crate::model::{ListDocument, ListItem, ListLabel};

/// Result of an export: the markdown plus any per-item notes that couldn't be exported.
/// Mirrors Swift `ExportResult`.
#[derive(Debug, Clone, Default)]
pub struct ExportResult {
    pub markdown: String,
    pub warnings: Vec<String>,
}

/// Render a ListDocument as markdown with the export-view options. Mirrors Swift
/// `MarkdownListGenerator.generate` (alphabetical items per label, optional active-only
/// filter, optional notes-as-sublists with heading/image handling + skip warnings).
pub fn generate_markdown_export(doc: &ListDocument, active_only: bool, include_notes: bool) -> ExportResult {
    let items: Vec<&ListItem> = doc
        .active_items()
        .filter(|i| !active_only || !i.checked)
        .collect();
    generate_markdown_export_items(
        &doc.list.name,
        &items,
        &doc.labels,
        &doc.list.label_order,
        active_only,
        include_notes,
    )
}

/// Like [`generate_markdown_export`] but over an explicit item subset (e.g. the items
/// selected in the share sheet). `active_only` only affects the empty-list message wording.
/// Mirrors Swift `MarkdownListGenerator.generate(items:...)`.
pub fn generate_markdown_export_items(
    list_name: &str,
    items: &[&ListItem],
    labels: &[ListLabel],
    label_order: &[String],
    active_only: bool,
    include_notes: bool,
) -> ExportResult {
    let mut markdown = format!("# {list_name}\n\n");
    let mut warnings: Vec<String> = Vec::new();

    if items.is_empty() {
        markdown.push_str(if active_only {
            "*All items are checked!*\n\n"
        } else {
            "*This list is empty.*\n\n"
        });
        return ExportResult { markdown, warnings };
    }

    // Group by label name ("No Label" for unlabeled), then order labels by label_order.
    let label_name = |item: &ListItem| -> String {
        item.label_id
            .as_deref()
            .and_then(|id| labels.iter().find(|l| l.id == id))
            .map(|l| l.name.clone())
            .unwrap_or_else(|| "No Label".to_string())
    };
    let mut group_names: Vec<String> = Vec::new();
    for item in items {
        let name = label_name(item);
        if !group_names.contains(&name) {
            group_names.push(name);
        }
    }
    sort_label_names(&mut group_names, labels, label_order);

    for name in &group_names {
        let mut in_label: Vec<&&ListItem> = items.iter().filter(|i| &label_name(i) == name).collect();
        in_label.sort_by(|a, b| a.note.to_lowercase().cmp(&b.note.to_lowercase()));

        markdown.push_str(&format!("## {name}\n\n"));
        for item in in_label {
            let checkbox = if item.checked { "[x]" } else { "[ ]" };
            let qty = if item.quantity > 1.0 {
                format!("{} ", item.quantity as i64)
            } else {
                String::new()
            };
            markdown.push_str(&format!("- {checkbox} {qty}{}\n", item.note));

            if include_notes {
                if let Some(notes) = item.markdown_notes.as_deref().filter(|n| !n.is_empty()) {
                    let skipped = append_notes_as_sublists(&mut markdown, notes);
                    if skipped > 0 {
                        warnings.push(format!(
                            "'{}' has notes that can't be exported: {skipped} line(s) skipped",
                            item.note
                        ));
                    }
                }
                if let Some(url) = item.source_url.as_deref().filter(|u| !u.is_empty()) {
                    markdown.push_str(&format!("  - [{}]({url})\n", link_label(url)));
                }
            }
        }
        markdown.push('\n');
    }

    ExportResult { markdown, warnings }
}

/// Append an item's markdown notes as nested sublist items. Headings become bold sublist
/// entries at their level's depth; content under a heading nests one level deeper. Images
/// become links; list markers are normalised to "- ". Non-round-trippable block markdown
/// (blockquotes, code fences, rules, tables) is skipped; returns the skip count.
fn append_notes_as_sublists(out: &mut String, notes: &str) -> usize {
    let mut skipped = 0;
    let mut current_depth = 1usize;
    let mut seen_heading = false;

    for line in notes.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !is_exportable_line(trimmed) {
            skipped += 1;
            continue;
        }

        let (depth, text) = if let Some(level) = heading_level(trimmed) {
            seen_heading = true;
            current_depth = level;
            (level, format!("**{}**", heading_text(trimmed)))
        } else {
            let depth = if seen_heading { current_depth + 1 } else { 1 };
            let text = if let Some(link) = image_to_link(trimmed) {
                link
            } else if let Some(rest) = trimmed
                .strip_prefix("- ")
                .or_else(|| trimmed.strip_prefix("* "))
                .or_else(|| trimmed.strip_prefix("+ "))
            {
                rest.to_string()
            } else {
                trimmed.to_string()
            };
            (depth, text)
        };
        out.push_str(&format!("{}- {text}\n", "  ".repeat(depth)));
    }
    skipped
}

/// A line is exportable as a sublist item unless it's a blockquote, code fence,
/// horizontal rule, or table row. Mirrors Swift `isExportableLine`.
fn is_exportable_line(trimmed: &str) -> bool {
    if trimmed.is_empty() {
        return false;
    }
    let first = trimmed.chars().next().unwrap();
    if first == '>' || first == '|' {
        return false;
    }
    if trimmed.starts_with("```")
        || trimmed.starts_with("---")
        || trimmed.starts_with("***")
        || trimmed.starts_with("___")
    {
        return false;
    }
    true
}

/// Heading level (1-6) if the line is an ATX heading followed by a space, else None.
fn heading_level(trimmed: &str) -> Option<usize> {
    if !trimmed.starts_with('#') {
        return None;
    }
    let level = trimmed.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&level) && trimmed.chars().nth(level) == Some(' ') {
        Some(level)
    } else {
        None
    }
}

fn heading_text(trimmed: &str) -> String {
    trimmed.trim_start_matches('#').trim().to_string()
}

/// `![alt](url)` -> `[alt](url)` (empty alt -> "Image link"); None if not an image line.
fn image_to_link(trimmed: &str) -> Option<String> {
    let rest = trimmed.strip_prefix("![")?;
    let close = rest.find("](")?;
    let alt = &rest[..close];
    let after = &rest[close + 2..];
    let url = after.strip_suffix(')')?;
    if url.is_empty() || url.contains(')') {
        return None;
    }
    let label = if alt.is_empty() { "Image link" } else { alt };
    Some(format!("[{label}]({url})"))
}

/// A human label for a source URL link. Mirrors Swift `LocationParser.linkLabel`.
fn link_label(url: &str) -> &'static str {
    let host = url.trim().to_lowercase();
    if host.contains("google.com") || host.contains("goo.gl") {
        "Show on Google Maps"
    } else if host.contains("apple.com") || host.contains("link.maps.apple") {
        "Show on Apple Maps"
    } else {
        "Show on map"
    }
}

/// Order label names by the list's `label_order`; unknown/No-Label names sort last,
/// then alphabetically. Mirrors Swift `sortedLabelNames`.
fn sort_label_names(names: &mut [String], labels: &[ListLabel], label_order: &[String]) {
    names.sort_by(|a, b| {
        let key = |name: &str| -> usize {
            labels
                .iter()
                .find(|l| l.name == name)
                .and_then(|l| label_order.iter().position(|id| id == &l.id))
                .unwrap_or(usize::MAX)
        };
        key(a).cmp(&key(b)).then_with(|| a.to_lowercase().cmp(&b.to_lowercase()))
    });
}

/// Render a ListDocument as a GFM markdown checklist.
pub fn generate_markdown(doc: &ListDocument) -> String {
    let mut out = String::new();

    out.push_str(&format!("# {}\n\n", doc.list.name));

    let label_order = &doc.list.label_order;
    let hidden = &doc.list.hidden_labels;

    // Build sorted label list.
    let mut labels: Vec<&ListLabel> = doc
        .labels
        .iter()
        .filter(|l| !hidden.contains(&l.id))
        .collect();
    labels.sort_by_key(|l| {
        label_order
            .iter()
            .position(|id| id == &l.id)
            .unwrap_or(usize::MAX)
    });

    // Items with a label, grouped by label order.
    for label in &labels {
        let items: Vec<&ListItem> = doc
            .active_items()
            .filter(|i| i.label_id.as_deref() == Some(&label.id))
            .collect();
        if items.is_empty() {
            continue;
        }
        out.push_str(&format!("## {}\n\n", label.name));
        for item in items {
            out.push_str(&format_item(item));
        }
        out.push('\n');
    }

    // Items without a label.
    let unlabeled: Vec<&ListItem> = doc
        .active_items()
        .filter(|i| i.label_id.is_none())
        .collect();
    if !unlabeled.is_empty() {
        for item in unlabeled {
            out.push_str(&format_item(item));
        }
        out.push('\n');
    }

    out
}

fn format_item(item: &ListItem) -> String {
    let check = if item.checked { "x" } else { " " };
    let qty = if item.quantity != 1.0 {
        format!("{}× ", item.display_quantity().unwrap_or_default())
    } else {
        String::new()
    };
    let mut line = format!("- [{}] {}{}\n", check, qty, item.note);
    if let Some(notes) = &item.markdown_notes {
        for note_line in notes.lines() {
            line.push_str(&format!("  {}\n", note_line));
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ListDocument, ListItem, ListLabel};

    #[test]
    fn generates_sections() {
        let mut doc = ListDocument::new("My List");
        doc.labels.push(ListLabel::new("produce", "Produce", "#4CAF50"));
        let mut item = ListItem::new("Apples");
        item.label_id = Some("produce".to_string());
        doc.items.push(item);
        let md = generate_markdown(&doc);
        assert!(md.contains("## Produce"));
        assert!(md.contains("- [ ] Apples"));
    }

    #[test]
    fn export_active_only_filters_checked() {
        let mut doc = ListDocument::new("Shop");
        let mut a = ListItem::new("Apples");
        a.quantity = 2.0;
        let mut b = ListItem::new("Milk");
        b.checked = true;
        doc.items.push(a);
        doc.items.push(b);
        let res = generate_markdown_export(&doc, true, false);
        assert!(res.markdown.contains("- [ ] 2 Apples"));
        assert!(!res.markdown.contains("Milk"));
    }

    #[test]
    fn export_empty_active_message() {
        let mut doc = ListDocument::new("Shop");
        let mut b = ListItem::new("Milk");
        b.checked = true;
        doc.items.push(b);
        let res = generate_markdown_export(&doc, true, false);
        assert!(res.markdown.contains("*All items are checked!*"));
    }

    #[test]
    fn export_notes_skip_blockquote_warns() {
        let mut doc = ListDocument::new("Shop");
        let mut a = ListItem::new("Apples");
        a.markdown_notes = Some("## Sub\nbuy red\n> quoted".to_string());
        doc.items.push(a);
        let res = generate_markdown_export(&doc, false, true);
        assert!(res.markdown.contains("    - **Sub**"));
        assert!(res.markdown.contains("      - buy red"));
        assert_eq!(res.warnings.len(), 1);
    }

    #[test]
    fn checked_items_render_correctly() {
        let mut doc = ListDocument::new("List");
        let mut item = ListItem::new("Milk");
        item.checked = true;
        doc.items.push(item);
        let md = generate_markdown(&doc);
        assert!(md.contains("- [x] Milk"));
    }
}
