use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tracing::warn;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionState {
    /// IDs of lists open at last quit.
    pub last_open_list_ids: Vec<String>,
    /// Currently selected list.
    pub selected_list_id: Option<String>,
    /// Collapse state per list: { list_id -> { label_id -> is_expanded } }
    pub expanded_sections: HashMap<String, HashMap<String, bool>>,
    /// View mode per list: "list" | "kanban" | "map"
    pub view_modes: HashMap<String, String>,
    /// Whether completed items appear inline or in a bottom section, per list.
    pub completed_at_bottom: HashMap<String, bool>,
    /// Kanban completed-items visible per list+column: { list_id -> { label_id -> visible } }
    pub kanban_completed_visible: HashMap<String, HashMap<String, bool>>,
}

impl SessionState {
    fn path() -> PathBuf {
        dirs::state_dir()
            .unwrap_or_else(|| dirs::home_dir().unwrap().join(".local/state"))
            .join("quite-listie")
            .join("session.json")
    }

    pub fn load() -> Self {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_else(|e| {
                warn!("failed to parse session state: {e}");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(self) {
            Ok(data) => {
                if let Err(e) = std::fs::write(&path, data) {
                    warn!("failed to save session state: {e}");
                }
            }
            Err(e) => warn!("failed to serialize session state: {e}"),
        }
    }
}
