use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use crate::model::{ListDocument, ListItem, ListLabel, ListSummary};

/// Parse a markdown string into a ListDocument.
///
/// Headings (# / ## / ###) become label sections.
/// The first heading is used as the list title if the document has no name yet.
/// Task list items (- [ ] / - [x]) and plain list items become ListItems.
/// Nested list items under a top-level item become that item's markdown_notes.
/// Quantities are extracted from patterns like "2x Milk" or "2 Milk".
pub fn parse_markdown(text: &str, existing_name: Option<&str>) -> ListDocument {
    let opts = Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH;
    let parser = Parser::new_ext(text, opts);

    let mut doc = ListDocument::new(existing_name.unwrap_or("Imported List"));
    let mut current_label_name: Option<String> = None;
    let mut first_heading = true;
    let mut in_list_item = false;
    let mut current_item_text = String::new();
    let mut current_item_checked = false;
    let mut current_item_notes: Vec<String> = Vec::new();
    let mut depth = 0usize;

    let events: Vec<Event> = parser.collect();
    let mut i = 0;

    while i < events.len() {
        match &events[i] {
            Event::Start(Tag::Heading { level: _, .. }) => {
                flush_item(
                    &mut doc,
                    &mut in_list_item,
                    &mut current_item_text,
                    &mut current_item_checked,
                    &mut current_item_notes,
                    &current_label_name,
                );
                i += 1;
                let mut heading_text = String::new();
                while i < events.len() {
                    match &events[i] {
                        Event::Text(t) => heading_text.push_str(t),
                        Event::End(TagEnd::Heading(_)) => break,
                        _ => {}
                    }
                    i += 1;
                }
                let heading_text = heading_text.trim().to_string();
                if first_heading && existing_name.is_none() {
                    doc.list = ListSummary::new(&heading_text);
                    first_heading = false;
                } else {
                    first_heading = false;
                    current_label_name = Some(heading_text.clone());
                    // Ensure a label exists for this heading.
                    if !doc.labels.iter().any(|l| l.name == heading_text) {
                        let id = slug(&heading_text);
                        let color = label_color_for_name(&heading_text);
                        doc.labels.push(ListLabel::new(id, &heading_text, color));
                    }
                }
            }

            Event::Start(Tag::List(_)) => {
                depth += 1;
            }
            Event::End(TagEnd::List(_)) => {
                if depth > 0 { depth -= 1; }
                if depth == 0 {
                    flush_item(
                        &mut doc,
                        &mut in_list_item,
                        &mut current_item_text,
                        &mut current_item_checked,
                        &mut current_item_notes,
                        &current_label_name,
                    );
                }
            }

            Event::Start(Tag::Item) => {
                if depth == 1 {
                    flush_item(
                        &mut doc,
                        &mut in_list_item,
                        &mut current_item_text,
                        &mut current_item_checked,
                        &mut current_item_notes,
                        &current_label_name,
                    );
                    in_list_item = true;
                    current_item_checked = false;
                    current_item_text.clear();
                    current_item_notes.clear();
                }
            }

            Event::TaskListMarker(checked) => {
                current_item_checked = *checked;
            }

            Event::Text(text) if in_list_item => {
                if depth == 1 {
                    current_item_text.push_str(text);
                } else {
                    current_item_notes.push(text.to_string());
                }
            }

            Event::End(TagEnd::Item) => {
                // will be flushed at next Item or end of list
            }

            _ => {}
        }
        i += 1;
    }

    flush_item(
        &mut doc,
        &mut in_list_item,
        &mut current_item_text,
        &mut current_item_checked,
        &mut current_item_notes,
        &current_label_name,
    );

    doc
}

fn flush_item(
    doc: &mut ListDocument,
    in_list_item: &mut bool,
    text: &mut String,
    checked: &mut bool,
    notes: &mut Vec<String>,
    label_name: &Option<String>,
) {
    if !*in_list_item || text.trim().is_empty() {
        return;
    }

    let (quantity, note) = extract_quantity(text.trim());
    let label_id = label_name.as_deref().and_then(|name| {
        doc.labels.iter().find(|l| l.name == name).map(|l| l.id.clone())
    });

    let mut item = ListItem::new(note);
    item.quantity = quantity;
    item.checked = *checked;
    item.label_id = label_id;
    if !notes.is_empty() {
        item.markdown_notes = Some(notes.join("\n"));
    }
    doc.items.push(item);

    *in_list_item = false;
    text.clear();
    *checked = false;
    notes.clear();
}

/// Extract a leading quantity from text like "2x Milk", "3 apples", "0.5 kg butter".
fn extract_quantity(text: &str) -> (f64, String) {
    let re_patterns = [
        // "2x Milk" or "2X Milk"
        (r"^(\d+(?:\.\d+)?)[xX]\s+(.+)$", 1, 2),
        // "2 Milk" — only when number is followed by space then non-digit
        (r"^(\d+(?:\.\d+)?)\s+([A-Za-z].+)$", 1, 2),
    ];

    for (_pattern, _qty_group, _text_group) in re_patterns {
        // Minimal regex without the regex crate: hand-parse.
        if let Some((qty, rest)) = try_parse_leading_quantity(text) {
            return (qty, rest.to_string());
        }
    }

    (1.0, text.to_string())
}

fn try_parse_leading_quantity(text: &str) -> Option<(f64, &str)> {
    let mut end = 0;
    let mut has_dot = false;
    for (i, ch) in text.char_indices() {
        if ch.is_ascii_digit() {
            end = i + 1;
        } else if ch == '.' && !has_dot {
            has_dot = true;
            end = i + 1;
        } else {
            break;
        }
    }
    if end == 0 {
        return None;
    }
    let qty: f64 = text[..end].parse().ok()?;
    let rest = text[end..].trim_start_matches(['x', 'X']).trim_start();
    if rest.is_empty() || rest.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    Some((qty, rest))
}

fn slug(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-')
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn label_color_for_name(name: &str) -> &'static str {
    // Assign a deterministic color from the preset palette.
    let colors = [
        "#4CAF50", "#2196F3", "#F44336", "#FF9800",
        "#00BCD4", "#9C27B0", "#FFEB3B", "#795548",
        "#607D8B", "#E91E63",
    ];
    let idx = name.bytes().fold(0usize, |acc, b| acc.wrapping_add(b as usize));
    colors[idx % colors.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_headings_as_labels() {
        let md = "# Grocery List\n## Produce\n- Apples\n- Bananas\n## Dairy\n- [x] Milk\n";
        let doc = parse_markdown(md, None);
        assert_eq!(doc.list.name, "Grocery List");
        assert_eq!(doc.labels.len(), 2);
        assert!(doc.items.iter().any(|i| i.note == "Milk" && i.checked));
    }

    #[test]
    fn extracts_quantity() {
        let md = "# List\n- 3x Apples\n";
        let doc = parse_markdown(md, None);
        let apple = doc.items.iter().find(|i| i.note == "Apples").unwrap();
        assert_eq!(apple.quantity, 3.0);
    }
}
