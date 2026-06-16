use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::{info, warn};
use zbus::Connection;

use crate::model::ListItem;

// ---------------------------------------------------------------------------
// Pending reminder registry (persisted in session state)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingReminder {
    pub notification_id: u32,
    pub item_id: String,
    pub list_id: String,
    pub fire_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// D-Bus notification proxy
// ---------------------------------------------------------------------------

#[zbus::proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
trait Notifications {
    async fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: std::collections::HashMap<&str, zbus::zvariant::Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;

    #[zbus(signal)]
    fn action_invoked(&self, id: u32, action_key: String) -> zbus::Result<()>;
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

pub struct ReminderEngine {
    pub pending: HashMap<String, PendingReminder>,
    /// Reverse map: notification_id → item_id (for ActionInvoked lookups).
    notif_to_item: HashMap<u32, String>,
}

impl ReminderEngine {
    pub fn new() -> Self {
        Self {
            pending: HashMap::new(),
            notif_to_item: HashMap::new(),
        }
    }

    /// Schedule (or reschedule) a reminder for `item` in `list_name`.
    pub fn schedule(&mut self, item: &ListItem, list_id: &str, list_name: &str) {
        let Some(fire_at) = item.reminder_date else { return };
        self.pending.insert(
            item.id.to_string(),
            PendingReminder {
                notification_id: 0,
                item_id: item.id.to_string(),
                list_id: list_id.to_string(),
                fire_at,
            },
        );
        info!(
            "scheduled reminder for '{}' in '{}' at {}",
            item.note, list_name, fire_at
        );
    }

    pub fn cancel(&mut self, item_id: &str) {
        if let Some(r) = self.pending.remove(item_id) {
            self.notif_to_item.remove(&r.notification_id);
        }
    }

    /// Called periodically (every ~30 s). Fires overdue notifications via D-Bus.
    pub async fn tick(&mut self, get_item: impl Fn(&str, &str) -> Option<ListItem>) {
        let now = Utc::now();
        let overdue: Vec<PendingReminder> = self
            .pending
            .values()
            .filter(|r| r.fire_at <= now)
            .cloned()
            .collect();

        if overdue.is_empty() {
            return;
        }

        let conn = match Connection::session().await {
            Ok(c) => c,
            Err(e) => {
                warn!("D-Bus session connection failed: {e}");
                return;
            }
        };
        let proxy = match NotificationsProxy::new(&conn).await {
            Ok(p) => p,
            Err(e) => {
                warn!("could not connect to org.freedesktop.Notifications: {e}");
                return;
            }
        };

        for reminder in overdue {
            let item = get_item(&reminder.list_id, &reminder.item_id);
            let body = item
                .as_ref()
                .map(|i| {
                    if let Some(qty) = i.display_quantity() {
                        format!("{} × {}", qty, i.note)
                    } else {
                        i.note.clone()
                    }
                })
                .unwrap_or_default();

            match proxy
                .notify(
                    "Quite Listie",
                    0,
                    "checkmark",
                    &reminder.list_id,
                    &body,
                    &["complete", "Mark complete"],
                    std::collections::HashMap::new(),
                    -1,
                )
                .await
            {
                Ok(notif_id) => {
                    self.notif_to_item.insert(notif_id, reminder.item_id.clone());
                    // Mirror Swift UNUserNotificationCenter: a scheduled notification
                    // fires once and is removed; recurring items don't auto-advance
                    // on fire. The next occurrence is set when the user taps
                    // "Mark complete" (see complete_item_from_notification in
                    // unified_provider.rs). If the user dismisses without acting,
                    // the item just shows as overdue in the UI until they handle
                    // it manually — matching ReminderManager.swift's behaviour.
                    let _ = item; // keep for parity with prior signature
                    self.pending.remove(&reminder.item_id);
                }
                Err(e) => warn!("failed to send notification: {e}"),
            }
        }
    }

    /// Look up the (list_id, item_id) for a notification_id (for ActionInvoked).
    pub fn resolve_notification(&self, notif_id: u32) -> Option<(&str, &str)> {
        let item_id = self.notif_to_item.get(&notif_id)?;
        let reminder = self.pending.get(item_id.as_str())?;
        Some((&reminder.list_id, &reminder.item_id))
    }
}

// ---------------------------------------------------------------------------
// Background task: start tick loop + ActionInvoked listener
// ---------------------------------------------------------------------------

/// Spawn the reminder tick loop and ActionInvoked D-Bus listener.
/// `on_complete(list_id, item_id)` is called when the user taps "Mark complete".
pub fn start_reminder_tasks(
    provider: Arc<Mutex<crate::engine::unified_provider::UnifiedProvider>>,
    on_complete: impl Fn(String, String) + Send + Sync + 'static,
) {
    let on_complete = Arc::new(on_complete);

    // Tick loop — fires every 30 seconds.
    let provider_tick = provider.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            // Snapshot overdue items before taking the mutable borrow for tick().
            let items: HashMap<(String, String), crate::model::ListItem> = {
                let p = provider_tick.lock().await;
                let now = chrono::Utc::now();
                p.reminder_engine.pending.values()
                    .filter(|r| r.fire_at <= now)
                    .filter_map(|r| {
                        p.get_item(&r.list_id, &r.item_id)
                            .map(|i| ((r.list_id.clone(), r.item_id.clone()), i))
                    })
                    .collect()
            };
            let mut p = provider_tick.lock().await;
            p.reminder_engine.tick(|list_id, item_id| {
                items.get(&(list_id.to_string(), item_id.to_string())).cloned()
            }).await;
        }
    });

    // ActionInvoked listener.
    let provider_action = provider.clone();
    let on_complete_action = on_complete;
    tokio::spawn(async move {
        let conn = match Connection::session().await {
            Ok(c) => c,
            Err(e) => {
                warn!("ActionInvoked: D-Bus connect failed: {e}");
                return;
            }
        };
        let proxy = match NotificationsProxy::new(&conn).await {
            Ok(p) => p,
            Err(e) => {
                warn!("ActionInvoked: proxy failed: {e}");
                return;
            }
        };
        let mut stream = match proxy.receive_action_invoked().await {
            Ok(s) => s,
            Err(e) => {
                warn!("ActionInvoked: signal subscribe failed: {e}");
                return;
            }
        };

        use futures_util::StreamExt;
        while let Some(signal) = stream.next().await {
            if let Ok(args) = signal.args() {
                if args.action_key() == "complete" {
                    let (list_id, item_id) = {
                        let p = provider_action.lock().await;
                        match p.reminder_engine.resolve_notification(*args.id()) {
                            Some((l, i)) => (l.to_string(), i.to_string()),
                            None => continue,
                        }
                    };
                    {
                        let mut p = provider_action.lock().await;
                        p.complete_item_from_notification(&list_id, &item_id);
                    }
                    on_complete_action(list_id, item_id);
                }
            }
        }
    });
}
