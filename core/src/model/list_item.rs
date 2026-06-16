use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

use super::coordinate::Coordinate;
use super::reminder::{ReminderRepeatMode, ReminderRepeatRule};
use super::serde_helpers::{
    iso8601, iso8601_opt, optional_lenient, optional_preserving, quantity, required,
};

// Deserialized manually (see the `Deserialize` impl below) so an unparseable value of a
// known field is preserved across a round-trip rather than failing the whole document parse.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListItem {
    pub id: Uuid,
    pub note: String,

    #[serde(with = "quantity")]
    pub quantity: f64,

    pub checked: bool,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_id: Option<String>,

    #[serde(with = "iso8601")]
    pub modified_at: DateTime<Utc>,

    #[serde(default)]
    pub is_deleted: bool,

    #[serde(default, with = "iso8601_opt", skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<DateTime<Utc>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub markdown_notes: Option<String>,

    #[serde(default, with = "iso8601_opt", skip_serializing_if = "Option::is_none")]
    pub reminder_date: Option<DateTime<Utc>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub reminder_repeat_rule: Option<ReminderRepeatRule>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub reminder_repeat_mode: Option<ReminderRepeatMode>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Coordinate>,

    #[serde(rename = "sourceURL", skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,

    #[serde(default, with = "iso8601_opt", skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<DateTime<Utc>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_change_field: Option<String>,

    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl<'de> Deserialize<'de> for ListItem {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut map = serde_json::Map::<String, serde_json::Value>::deserialize(deserializer)?;
        Ok(ListItem {
            id: required(&mut map, "id")?,
            note: required(&mut map, "note")?,
            quantity: required(&mut map, "quantity")?,
            checked: required(&mut map, "checked")?,
            modified_at: required(&mut map, "modifiedAt")?,
            label_id: optional_preserving(&mut map, "labelId"),
            is_deleted: optional_lenient(&mut map, "isDeleted").unwrap_or(false),
            deleted_at: optional_preserving(&mut map, "deletedAt"),
            markdown_notes: optional_preserving(&mut map, "markdownNotes"),
            reminder_date: optional_preserving(&mut map, "reminderDate"),
            reminder_repeat_rule: optional_preserving(&mut map, "reminderRepeatRule"),
            reminder_repeat_mode: optional_preserving(&mut map, "reminderRepeatMode"),
            location: optional_preserving(&mut map, "location"),
            source_url: optional_preserving(&mut map, "sourceURL"),
            checked_at: optional_preserving(&mut map, "checkedAt"),
            last_change_field: optional_preserving(&mut map, "lastChangeField"),
            extra: map,
        })
    }
}

impl ListItem {
    pub fn new(note: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            note: note.into(),
            quantity: 1.0,
            checked: false,
            label_id: None,
            modified_at: Utc::now(),
            is_deleted: false,
            deleted_at: None,
            markdown_notes: None,
            reminder_date: None,
            reminder_repeat_rule: None,
            reminder_repeat_mode: None,
            location: None,
            source_url: None,
            checked_at: None,
            last_change_field: None,
            extra: Default::default(),
        }
    }

    pub fn touch(&mut self) {
        self.modified_at = Utc::now();
    }

    pub fn soft_delete(&mut self) {
        let now = Utc::now();
        self.is_deleted = true;
        self.deleted_at = Some(now);
        self.modified_at = now;
    }

    pub fn restore(&mut self) {
        self.is_deleted = false;
        self.deleted_at = None;
        self.touch();
    }

    pub fn has_reminder(&self) -> bool {
        self.reminder_date.is_some()
    }

    pub fn has_location(&self) -> bool {
        self.location.is_some()
    }

    pub fn display_quantity(&self) -> Option<String> {
        if self.quantity == 1.0 {
            None
        } else if self.quantity.fract() == 0.0 {
            Some(format!("{}", self.quantity as i64))
        } else {
            Some(format!("{}", self.quantity))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = r#""id":"550e8400-e29b-41d4-a716-446655440000","note":"x","quantity":1,"checked":false,"modifiedAt":"2026-06-15T00:00:00Z""#;

    // A future ReminderRepeatUnit must not fail the parse; the rule round-trips verbatim.
    #[test]
    fn unknown_enum_value_is_preserved() {
        let json = format!("{{{HEAD},\"reminderRepeatRule\":{{\"unit\":\"fortnight\",\"interval\":3}}}}");
        let item: ListItem = serde_json::from_str(&json).unwrap();
        assert!(item.reminder_repeat_rule.is_none());
        let back = serde_json::to_value(&item).unwrap();
        assert_eq!(back["reminderRepeatRule"]["unit"], "fortnight");
        assert_eq!(back["reminderRepeatRule"]["interval"], 3);
    }

    // A malformed value of a known optional field round-trips instead of failing the parse.
    #[test]
    fn malformed_optional_is_preserved() {
        let json = format!("{{{HEAD},\"reminderDate\":\"not-a-date\"}}");
        let item: ListItem = serde_json::from_str(&json).unwrap();
        assert!(item.reminder_date.is_none());
        let back = serde_json::to_value(&item).unwrap();
        assert_eq!(back["reminderDate"], "not-a-date");
    }

    #[test]
    fn unknown_key_round_trips() {
        let json = format!("{{{HEAD},\"futureField\":{{\"a\":1}}}}");
        let item: ListItem = serde_json::from_str(&json).unwrap();
        let back = serde_json::to_value(&item).unwrap();
        assert_eq!(back["futureField"]["a"], 1);
    }

    #[test]
    fn valid_fields_still_parse() {
        let json = format!(
            "{{{HEAD},\"reminderRepeatRule\":{{\"unit\":\"week\",\"interval\":2}},\"sourceURL\":\"http://e.com\"}}"
        );
        let item: ListItem = serde_json::from_str(&json).unwrap();
        assert_eq!(item.reminder_repeat_rule.unwrap().interval, 2);
        assert_eq!(item.source_url.as_deref(), Some("http://e.com"));
        assert!(item.extra.is_empty());
    }
}
