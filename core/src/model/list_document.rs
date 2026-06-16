use serde::{Deserialize, Serialize};

use super::{ListItem, ListLabel, ListSummary};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDocument {
    #[serde(default = "default_version")]
    pub version: u32,
    pub list: ListSummary,
    pub items: Vec<ListItem>,
    pub labels: Vec<ListLabel>,
    #[serde(rename = "deletedLabelIDs", default, skip_serializing_if = "Vec::is_empty")]
    pub deleted_label_ids: Vec<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

fn default_version() -> u32 {
    2
}

impl ListDocument {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            version: 2,
            list: ListSummary::new(name),
            items: Vec::new(),
            labels: Vec::new(),
            deleted_label_ids: Vec::new(),
            extra: Default::default(),
        }
    }

    pub fn active_items(&self) -> impl Iterator<Item = &ListItem> {
        self.items.iter().filter(|i| !i.is_deleted)
    }

    pub fn deleted_items(&self) -> impl Iterator<Item = &ListItem> {
        self.items.iter().filter(|i| i.is_deleted)
    }
}
