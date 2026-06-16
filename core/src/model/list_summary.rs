use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};

use super::serde_helpers::{iso8601, optional_lenient, optional_preserving, required};

// Deserialized manually (see the `Deserialize` impl below) so an unparseable value of a
// known field is preserved across a round-trip rather than failing the whole document parse.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListSummary {
    pub id: String,
    pub name: String,

    #[serde(with = "iso8601")]
    pub modified_at: DateTime<Utc>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji_icon: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_labels: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub label_order: Vec<String>,

    #[serde(default)]
    pub enable_map_data: bool,

    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl<'de> Deserialize<'de> for ListSummary {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut map = serde_json::Map::<String, serde_json::Value>::deserialize(deserializer)?;
        Ok(ListSummary {
            id: required(&mut map, "id")?,
            name: required(&mut map, "name")?,
            modified_at: required(&mut map, "modifiedAt")?,
            icon: optional_preserving(&mut map, "icon"),
            emoji_icon: optional_preserving(&mut map, "emojiIcon"),
            hidden_labels: optional_preserving(&mut map, "hiddenLabels").unwrap_or_default(),
            label_order: optional_preserving(&mut map, "labelOrder").unwrap_or_default(),
            enable_map_data: optional_lenient(&mut map, "enableMapData").unwrap_or(false),
            extra: map,
        })
    }
}

impl ListSummary {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            modified_at: Utc::now(),
            icon: None,
            emoji_icon: None,
            hidden_labels: Vec::new(),
            label_order: Vec::new(),
            enable_map_data: false,
            extra: Default::default(),
        }
    }

    pub fn touch(&mut self) {
        self.modified_at = Utc::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = r#""id":"abc","name":"L","modifiedAt":"2026-06-15T00:00:00Z""#;

    // A malformed value of a known field and an unknown key both round-trip.
    #[test]
    fn malformed_field_and_unknown_key_preserved() {
        let json = format!("{{{HEAD},\"labelOrder\":\"oops\",\"futureFlag\":true}}");
        let s: ListSummary = serde_json::from_str(&json).unwrap();
        assert!(s.label_order.is_empty());
        let back = serde_json::to_value(&s).unwrap();
        assert_eq!(back["labelOrder"], "oops");
        assert_eq!(back["futureFlag"], true);
    }

    #[test]
    fn valid_summary_parses() {
        let json = format!("{{{HEAD},\"icon\":\"cart\",\"hiddenLabels\":[\"a\"],\"enableMapData\":true}}");
        let s: ListSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(s.icon.as_deref(), Some("cart"));
        assert_eq!(s.hidden_labels, vec!["a"]);
        assert!(s.enable_map_data);
        assert!(s.extra.is_empty());
    }
}
