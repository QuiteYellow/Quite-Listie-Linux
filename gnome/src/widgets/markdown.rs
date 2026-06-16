//! Markdown rendering for item notes — the GNOME counterpart of the Swift app's
//! `CheckableMarkdownView` (ItemView.swift:813). Renders a markdown string as a column
//! of widgets where task-list lines (`- [ ]` / `- [x]`) become interactive checkboxes
//! that toggle the underlying source text, and every other non-empty line is rendered
//! as inline-formatted Pango markup. Inline formatting (bold/italic/code/links/strike)
//! is parsed with `pulldown-cmark` and emitted as Pango markup for a `GtkLabel`.

use adw::prelude::*;
use gtk::glib;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

/// Shown in the preview when there are no notes yet (matches the Swift placeholder).
pub const EMPTY_PLACEHOLDER: &str =
    "Click “Edit” to add a note — use Markdown for sublists, links and more. \
     Sublists (- [ ]) can be toggled right here.";

/// Build a live preview of `buffer`'s markdown. Returns a vertical box that renders the
/// current text and re-renders itself whenever a task checkbox is toggled (toggling also
/// rewrites the checkbox marker in `buffer`, so the change is saved with the item).
pub fn checkable_preview(buffer: &gtk::TextBuffer) -> gtk::Box {
    let container = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .build();
    render(&container, buffer);
    container
}

/// Re-render `container` from the current `buffer` text. Re-runs `render` on toggle.
fn render(container: &gtk::Box, buffer: &gtk::TextBuffer) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }

    let text = buffer_text(buffer);
    if text.trim().is_empty() {
        let placeholder = gtk::Label::builder()
            .label(EMPTY_PLACEHOLDER)
            .wrap(true)
            .xalign(0.0)
            .build();
        placeholder.add_css_class("dim-label");
        container.append(&placeholder);
        return;
    }

    for (index, line) in text.split('\n').enumerate() {
        match check_state(line) {
            Some(checked) => container.append(&task_row(buffer, container, index, line, checked)),
            None if !line.trim().is_empty() => container.append(&block_label(line)),
            None => {
                // Blank line — a small spacer so paragraph breaks read as breaks.
                let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
                spacer.set_height_request(6);
                container.append(&spacer);
            }
        }
    }
}

/// A checkbox + inline-rendered label for a `- [ ]` / `- [x]` line.
fn task_row(
    buffer: &gtk::TextBuffer,
    container: &gtk::Box,
    index: usize,
    line: &str,
    checked: bool,
) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let check = gtk::CheckButton::builder()
        .active(checked)
        .valign(gtk::Align::Start)
        .build();
    check.connect_toggled(glib::clone!(
        #[weak]
        container,
        #[strong]
        buffer,
        move |_| {
            toggle_task_line(&buffer, index);
            // Defer the rebuild: we're removing this very checkbox mid-signal.
            glib::idle_add_local_once(glib::clone!(
                #[weak]
                container,
                #[strong]
                buffer,
                move || render(&container, &buffer)
            ));
        }
    ));

    let mut markup = inline_to_pango(label_text(line));
    if checked {
        markup = format!("<s>{markup}</s>");
    }
    let label = markup_label(&markup);
    label.set_hexpand(true);
    if checked {
        label.add_css_class("dim-label");
    }

    row.append(&check);
    row.append(&label);
    row
}

/// Render a non-task line: headings, bullets, and blockquotes get block treatment;
/// everything else is inline markup.
fn block_label(line: &str) -> gtk::Label {
    let trimmed = line.trim_start();

    // ATX headings: 1–6 leading `#`, scaled + bold.
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
        let content = inline_to_pango(trimmed[hashes + 1..].trim_start());
        let size = match hashes {
            1 => "x-large",
            2 => "large",
            _ => "medium",
        };
        let label = markup_label(&format!("<span size=\"{size}\" weight=\"bold\">{content}</span>"));
        label.set_margin_top(4);
        return label;
    }

    // Blockquote.
    if let Some(rest) = trimmed.strip_prefix("> ") {
        let label = markup_label(&format!("<i>{}</i>", inline_to_pango(rest)));
        label.add_css_class("dim-label");
        label.set_margin_start(8);
        return label;
    }

    // Plain bullet (`- ` / `* `, not a task — those are handled in render).
    if let Some(rest) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
        return markup_label(&format!("• {}", inline_to_pango(rest)));
    }

    markup_label(&inline_to_pango(line))
}

/// A left-aligned, wrapping label carrying Pango markup, with link clicks enabled.
fn markup_label(markup: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .use_markup(true)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .xalign(0.0)
        .halign(gtk::Align::Start)
        .selectable(true)
        .build();
    label.set_markup(markup);
    label
}

// --- task-line helpers (mirror Swift CheckableMarkdownView) -------------------

/// `Some(true)` for `- [x]`, `Some(false)` for `- [ ]`, `None` for a non-task line.
fn check_state(line: &str) -> Option<bool> {
    let trimmed = line.trim_start_matches([' ', '\t']);
    if trimmed.starts_with("- [x] ") || trimmed.starts_with("- [X] ") {
        Some(true)
    } else if trimmed.starts_with("- [ ] ") {
        Some(false)
    } else {
        None
    }
}

/// The text of a task line after the `- [ ] ` / `- [x] ` marker.
fn label_text(line: &str) -> &str {
    let trimmed = line.trim_start_matches([' ', '\t']);
    for marker in ["- [x] ", "- [X] ", "- [ ] "] {
        if let Some(rest) = trimmed.strip_prefix(marker) {
            return rest;
        }
    }
    line
}

/// Flip the checkbox marker on line `index` and write the result back to `buffer`.
fn toggle_task_line(buffer: &gtk::TextBuffer, index: usize) {
    buffer.set_text(&toggle_in_text(&buffer_text(buffer), index));
}

/// Pure form of [`toggle_task_line`]: returns `text` with line `index`'s marker flipped.
fn toggle_in_text(text: &str, index: usize) -> String {
    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    if let Some(line) = lines.get_mut(index) {
        *line = match check_state(line) {
            Some(true) => replace_first(line, "- [x] ", "- [ ] ")
                .or_else(|| replace_first(line, "- [X] ", "- [ ] "))
                .unwrap_or_else(|| line.clone()),
            Some(false) => {
                replace_first(line, "- [ ] ", "- [x] ").unwrap_or_else(|| line.clone())
            }
            None => return text.to_string(),
        };
    }
    lines.join("\n")
}

fn replace_first(s: &str, from: &str, to: &str) -> Option<String> {
    s.find(from).map(|pos| {
        let mut out = String::with_capacity(s.len());
        out.push_str(&s[..pos]);
        out.push_str(to);
        out.push_str(&s[pos + from.len()..]);
        out
    })
}

fn buffer_text(buffer: &gtk::TextBuffer) -> String {
    let (start, end) = buffer.bounds();
    buffer.text(&start, &end, false).to_string()
}

// --- inline markdown → Pango markup ------------------------------------------

/// Convert a single line of inline markdown to Pango markup (bold, italic, code,
/// strikethrough, links). Block constructs are handled by the caller; here a stray
/// paragraph wrapper from the parser is simply ignored.
pub fn inline_to_pango(line: &str) -> String {
    let opts = Options::ENABLE_STRIKETHROUGH;
    let parser = Parser::new_ext(line, opts);
    let mut out = String::new();

    for event in parser {
        match event {
            Event::Text(t) => out.push_str(&escape(&t)),
            Event::Code(t) => {
                out.push_str("<tt>");
                out.push_str(&escape(&t));
                out.push_str("</tt>");
            }
            Event::Start(Tag::Strong) => out.push_str("<b>"),
            Event::End(TagEnd::Strong) => out.push_str("</b>"),
            Event::Start(Tag::Emphasis) => out.push_str("<i>"),
            Event::End(TagEnd::Emphasis) => out.push_str("</i>"),
            Event::Start(Tag::Strikethrough) => out.push_str("<s>"),
            Event::End(TagEnd::Strikethrough) => out.push_str("</s>"),
            Event::Start(Tag::Link { dest_url, .. }) => {
                out.push_str(&format!("<a href=\"{}\">", escape(&dest_url)));
            }
            Event::End(TagEnd::Link) => out.push_str("</a>"),
            Event::SoftBreak | Event::HardBreak => out.push(' '),
            _ => {}
        }
    }

    // A line that produced no inline output (e.g. only a paragraph wrapper) falls back
    // to the escaped raw text so nothing silently vanishes.
    if out.is_empty() {
        escape(line)
    } else {
        out
    }
}

/// Escape the five XML/Pango-significant characters.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_task_lines() {
        assert_eq!(check_state("- [ ] buy milk"), Some(false));
        assert_eq!(check_state("  - [x] done"), Some(true));
        assert_eq!(check_state("- [X] DONE"), Some(true));
        assert_eq!(check_state("- plain bullet"), None);
        assert_eq!(check_state("not a task"), None);
    }

    #[test]
    fn strips_task_marker() {
        assert_eq!(label_text("- [ ] buy milk"), "buy milk");
        assert_eq!(label_text("  - [x] done"), "done");
    }

    #[test]
    fn toggles_marker_in_place() {
        let text = "- [ ] a\n- [x] b\nplain";
        let text = toggle_in_text(text, 0);
        assert_eq!(text, "- [x] a\n- [x] b\nplain");
        let text = toggle_in_text(&text, 1);
        assert_eq!(text, "- [x] a\n- [ ] b\nplain");
        let text = toggle_in_text(&text, 2); // non-task: no change
        assert_eq!(text, "- [x] a\n- [ ] b\nplain");
    }

    #[test]
    fn renders_inline_markup() {
        assert_eq!(inline_to_pango("**bold**"), "<b>bold</b>");
        assert_eq!(inline_to_pango("a `code` b"), "a <tt>code</tt> b");
        assert_eq!(
            inline_to_pango("[site](https://x.com)"),
            "<a href=\"https://x.com\">site</a>"
        );
    }

    #[test]
    fn escapes_special_chars() {
        assert_eq!(inline_to_pango("a < b & c"), "a &lt; b &amp; c");
    }
}
