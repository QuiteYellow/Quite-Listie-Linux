use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListLabel {
    pub id: String,
    pub name: String,
    /// Hex color string, e.g. "#4CAF50"
    pub color: String,
    /// Optional icon name (KDE/FreeDesktop icon name)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,

    /// Optional cross-platform emoji glyph (renders natively on every OS).
    /// KDE prefers this over `symbol`; iOS prefers `symbol` and falls back to this.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji_icon: Option<String>,

    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl ListLabel {
    pub fn new(id: impl Into<String>, name: impl Into<String>, color: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            color: color.into(),
            symbol: None,
            emoji_icon: None,
            extra: Default::default(),
        }
    }
}
