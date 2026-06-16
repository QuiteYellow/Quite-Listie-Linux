//! The application controller — GNOME counterpart of the KDE `AppController`
//! (`kde/src/bridge/app_controller.rs`). A plain [`glib::Object`] holding the shared
//! [`UnifiedProvider`], exposing GObject **properties** (bound by the views) and
//! **signals** (which drive model refreshes), plus methods that mirror the former
//! `#[qinvokable]`s.
//!
//! Async work runs on the tokio runtime and is marshalled back to the GLib main loop
//! via [`crate::runtime::spawn_to_main`].

use std::cell::{Cell, OnceCell, RefCell};
use std::sync::Arc;

use chrono::Utc;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use tokio::sync::Mutex;

use quite_listie_core::engine::nextcloud::NextcloudCredentials;
use quite_listie_core::engine::unified_provider::{ListSource, UnifiedProvider};
use quite_listie_core::model::ListLabel;

use crate::runtime::{runtime, spawn_to_main};

type Provider = Arc<Mutex<UnifiedProvider>>;

/// A soft-deleted item projected for the recycle-bin view.
#[derive(Debug, Clone)]
pub struct DeletedItemRow {
    pub id: String,
    pub note: String,
    /// Whole days since the item was deleted (clamped at 0).
    pub days_ago: i64,
    /// Deletion timestamp (unix seconds), used for newest-first ordering.
    pub deleted_ts: i64,
}

/// An item projected for the share-link picker, in label-grouped order.
#[derive(Debug, Clone)]
pub struct ShareItem {
    pub id: String,
    pub note: String,
    pub quantity: f64,
    pub checked: bool,
    /// Resolved label name ("No Label" when unlabeled) — also the group heading.
    pub label_name: String,
}

/// A recently-changed item projected for the recent-changes view.
#[derive(Debug, Clone)]
pub struct RecentChangeRow {
    pub id: String,
    pub note: String,
    /// The `last_change_field` kind: added/checked/note/quantity/label/reminder/location/
    /// subitems/deleted/restored.
    pub change: String,
    pub checked: bool,
    pub is_deleted: bool,
    /// Modification timestamp (unix seconds), used for newest-first ordering + relative time.
    pub modified_ts: i64,
}

/// Normalise a user-entered server address: trim, drop a trailing slash, and default
/// to `https://` when no scheme is given (otherwise reqwest rejects the URL with a
/// "builder error"). An empty input stays empty.
fn normalize_server_url(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() || trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    }
}

mod imp {
    use super::*;
    use glib::subclass::Signal;
    use std::sync::OnceLock;

    #[derive(glib::Properties)]
    #[properties(wrapper_type = super::Controller)]
    pub struct Controller {
        #[property(get, set)]
        pub current_list_id: RefCell<String>,
        #[property(get, set)]
        pub today_count: Cell<i32>,
        #[property(get, set)]
        pub scheduled_count: Cell<i32>,
        #[property(get, set)]
        pub location_count: Cell<i32>,
        #[property(get, set)]
        pub is_nc_authenticated: Cell<bool>,
        /// "idle" | "syncing" | "error" | "offline"
        #[property(get, set)]
        pub sync_status: RefCell<String>,

        pub provider: OnceCell<Provider>,
        pub settings: OnceCell<gtk::gio::Settings>,
        pub reminder_tasks_started: Cell<bool>,
        /// Abort handle for an in-flight Nextcloud Login-Flow-v2 poll, so the setup
        /// dialog can cancel it.
        pub login_task: RefCell<Option<tokio::task::AbortHandle>>,
    }

    impl Default for Controller {
        fn default() -> Self {
            Self {
                current_list_id: RefCell::new(String::new()),
                today_count: Cell::new(0),
                scheduled_count: Cell::new(0),
                location_count: Cell::new(0),
                is_nc_authenticated: Cell::new(false),
                sync_status: RefCell::new("idle".to_string()),
                provider: OnceCell::new(),
                settings: OnceCell::new(),
                reminder_tasks_started: Cell::new(false),
                login_task: RefCell::new(None),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Controller {
        const NAME: &'static str = "QuiteListieController";
        type Type = super::Controller;
        type ParentType = glib::Object;
    }

    #[glib::derived_properties]
    impl ObjectImpl for Controller {
        fn constructed(&self) {
            self.parent_constructed();
            let provider = quite_listie_core::engine::provider_singleton::get();
            let authed = provider.blocking_lock().nc.is_authenticated();
            self.provider.set(provider).ok();
            self.settings
                .set(gtk::gio::Settings::new(crate::APP_ID))
                .ok();
            self.obj().set_is_nc_authenticated(authed);
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    // The list index changed (create/delete/sync).
                    Signal::builder("lists-updated").build(),
                    // The selected list changed (user navigation).
                    Signal::builder("current-list-changed").build(),
                    // An async op failed; carries a user-facing message.
                    Signal::builder("error-occurred")
                        .param_types([str::static_type()])
                        .build(),
                    // The open list's document was changed by a background sync.
                    Signal::builder("current-list-externally-changed").build(),
                    // Nextcloud Login Flow v2 completed.
                    Signal::builder("nc-login-completed").build(),
                    // A "Test connection" finished; carries (ok, message).
                    Signal::builder("nc-test-result")
                        .param_types([bool::static_type(), str::static_type()])
                        .build(),
                    // A Nextcloud directory listing is ready; carries a JSON array of
                    // entries (see `browse_nextcloud_at`).
                    Signal::builder("remote-files-ready")
                        .param_types([str::static_type()])
                        .build(),
                    // List settings (title/icon/favourite/background) were saved;
                    // the open list page should rebuild. Mirrors Swift's
                    // `.listSettingsChanged` notification.
                    Signal::builder("list-settings-changed").build(),
                    // A `quitelistie://import?...` deeplink was decoded. Carries
                    // (target_runtime_id_or_empty, markdown, preview); the window
                    // opens the Markdown import dialog pre-filled.
                    Signal::builder("deeplink-import")
                        .param_types([str::static_type(), str::static_type(), bool::static_type()])
                        .build(),
                    // A `quitelistie://item?id=...` deeplink resolved to an open
                    // list. Carries (list_id, item_id); the window selects the
                    // list and opens the item editor.
                    Signal::builder("deeplink-item")
                        .param_types([str::static_type(), str::static_type()])
                        .build(),
                ]
            })
        }
    }
}

glib::wrapper! {
    pub struct Controller(ObjectSubclass<imp::Controller>);
}

impl Default for Controller {
    fn default() -> Self {
        Self::new()
    }
}

impl Controller {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn provider(&self) -> Provider {
        self.imp().provider.get().expect("provider set in constructed").clone()
    }

    pub fn settings(&self) -> &gtk::gio::Settings {
        self.imp().settings.get().expect("settings set in constructed")
    }

    // ----- signal emit + connect helpers -----------------------------------

    fn emit_lists_updated(&self) {
        self.emit_by_name::<()>("lists-updated", &[]);
    }
    fn emit_error(&self, message: &str) {
        self.emit_by_name::<()>("error-occurred", &[&message]);
    }
    /// Surface a transient message to the user (shown as a toast by the window). Shares the
    /// `error-occurred` channel, which the window already wires to its `ToastOverlay`.
    pub fn show_toast(&self, message: &str) {
        self.emit_by_name::<()>("error-occurred", &[&message]);
    }
    fn emit_current_list_changed(&self) {
        self.emit_by_name::<()>("current-list-changed", &[]);
    }
    fn emit_nc_login_completed(&self) {
        self.emit_by_name::<()>("nc-login-completed", &[]);
    }

    pub fn connect_lists_updated<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "lists-updated",
            false,
            glib::closure_local!(move |obj: &Self| f(obj)),
        )
    }
    pub fn connect_current_list_changed<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "current-list-changed",
            false,
            glib::closure_local!(move |obj: &Self| f(obj)),
        )
    }
    pub fn connect_error_occurred<F: Fn(&Self, String) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "error-occurred",
            false,
            glib::closure_local!(move |obj: &Self, msg: String| f(obj, msg)),
        )
    }
    pub fn connect_current_list_externally_changed<F: Fn(&Self) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "current-list-externally-changed",
            false,
            glib::closure_local!(move |obj: &Self| f(obj)),
        )
    }
    pub fn connect_nc_login_completed<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "nc-login-completed",
            false,
            glib::closure_local!(move |obj: &Self| f(obj)),
        )
    }
    pub fn connect_nc_test_result<F: Fn(&Self, bool, String) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "nc-test-result",
            false,
            glib::closure_local!(move |obj: &Self, ok: bool, msg: String| f(obj, ok, msg)),
        )
    }
    pub fn connect_remote_files_ready<F: Fn(&Self, String) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "remote-files-ready",
            false,
            glib::closure_local!(move |obj: &Self, json: String| f(obj, json)),
        )
    }
    pub fn connect_list_settings_changed<F: Fn(&Self) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "list-settings-changed",
            false,
            glib::closure_local!(move |obj: &Self| f(obj)),
        )
    }
    pub fn connect_deeplink_import<F: Fn(&Self, String, String, bool) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "deeplink-import",
            false,
            glib::closure_local!(move |obj: &Self, target: String, md: String, preview: bool| f(
                obj, target, md, preview
            )),
        )
    }
    pub fn connect_deeplink_item<F: Fn(&Self, String, String) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "deeplink-item",
            false,
            glib::closure_local!(move |obj: &Self, list_id: String, item_id: String| f(
                obj, list_id, item_id
            )),
        )
    }

    /// Decode a `quitelistie://` (or legacy `listie://`) URL and route it.
    /// Import URLs emit `deeplink-import` (target resolved against open lists);
    /// item URLs emit `deeplink-item` when the item is found in an open list.
    /// Mirrors the KDE `AppController::import_deeplink`.
    pub fn handle_deeplink(&self, url: &str) {
        use quite_listie_core::util::deeplink::{parse_url, DeeplinkAction};
        let action = match parse_url(url) {
            Ok(a) => a,
            Err(e) => {
                self.emit_error(&format!("Invalid Quite Listie link: {e}"));
                return;
            }
        };
        match action {
            DeeplinkAction::Import { list_id, markdown, preview } => {
                // Resolve the list hint against open lists by runtime id or
                // document UUID (matches Swift); empty target -> show picker.
                let provider = self.provider();
                let target = {
                    let p = provider.blocking_lock();
                    list_id
                        .as_deref()
                        .and_then(|hint| {
                            p.lists
                                .iter()
                                .find(|l| l.id == hint || l.original_file_id == hint)
                                .map(|l| l.id.clone())
                        })
                        .unwrap_or_default()
                };
                self.emit_by_name::<()>("deeplink-import", &[&target, &markdown, &preview]);
            }
            DeeplinkAction::Item { item_id } => {
                let provider = self.provider();
                let resolved = {
                    let p = provider.blocking_lock();
                    p.lists.iter().find_map(|l| {
                        let doc = p.cached_doc(&l.id)?;
                        doc.items
                            .iter()
                            .find(|i| !i.is_deleted && i.id.to_string() == item_id)
                            .map(|_| l.id.clone())
                    })
                };
                match resolved {
                    Some(list_id) => {
                        self.emit_by_name::<()>("deeplink-item", &[&list_id, &item_id])
                    }
                    None => self.emit_error(&format!(
                        "Item {item_id} was not found in any open list."
                    )),
                }
            }
        }
    }

    // ----- smart-box counts (mirror compute_smart_counts) -------------------

    fn recompute_counts_blocking(&self) {
        let provider = self.provider();
        let (tc, sc, lc) = {
            let p = provider.blocking_lock();
            let today = Utc::now().date_naive();
            let (mut t, mut s, mut l) = (0i32, 0i32, 0i32);
            for (_list, doc) in p.all_cached_docs() {
                for item in doc.active_items().filter(|i| !i.checked) {
                    if let Some(d) = item.reminder_date {
                        if d.date_naive() <= today {
                            t += 1;
                        } else {
                            s += 1;
                        }
                    }
                    if item.has_location() {
                        l += 1;
                    }
                }
            }
            (t, s, l)
        };
        self.set_today_count(tc);
        self.set_scheduled_count(sc);
        self.set_location_count(lc);
    }

    /// Recompute counts and notify the sidebar — equivalent to the KDE
    /// `update_counts_and_notify`.
    fn refresh_counts_and_notify(&self) {
        self.recompute_counts_blocking();
        self.emit_lists_updated();
    }

    /// Public entry point for views (e.g. the reminder page) that mutate items through
    /// their own `ListItemModel` and need the smart-box counts + sidebar badges refreshed.
    pub fn refresh_counts(&self) {
        self.refresh_counts_and_notify();
    }

    // ----- invokables -------------------------------------------------------

    /// Restore the previously-opened lists from disk, then sync just those lists from
    /// Nextcloud in the background.
    ///
    /// Unlike the source app, we deliberately do **not** auto-discover every file in the
    /// remote lists folder — the sidebar holds only the NC lists the user has explicitly
    /// opened (persisted in `nc-opened-files.json`) plus external files. Discovery and
    /// opening happen on demand through the Nextcloud browser.
    pub fn refresh_lists(&self) {
        if !self.imp().reminder_tasks_started.get() {
            self.imp().reminder_tasks_started.set(true);
            self.start_reminder_tasks();
        }

        // Phase 1: synchronous disk load (no network) — restores the opened lists.
        {
            let provider = self.provider();
            let mut p = provider.blocking_lock();
            p.load_nc_opened_from_disk();
            p.load_external_opened_from_disk();
            p.reconcile_reminders();
        }
        self.refresh_counts_and_notify();

        // Phase 2: background sync of the already-open NC lists (no discovery).
        let provider = self.provider();
        spawn_to_main(
            async move {
                let mut p = provider.lock().await;
                let changed = p.sync_all_lists().await;
                p.reconcile_reminders();
                changed
            },
            glib::clone!(
                #[weak(rename_to = this)]
                self,
                move |changed: Vec<String>| {
                    this.refresh_counts_and_notify();
                    // If the list currently on screen changed on the server, reload it.
                    if changed.iter().any(|id| *this.current_list_id() == *id) {
                        this.emit_by_name::<()>("current-list-externally-changed", &[]);
                    }
                }
            ),
        );
    }

    /// Wire the reminder engine's fire callback to `current-list-externally-changed`.
    fn start_reminder_tasks(&self) {
        let (tx, rx) = async_channel::unbounded::<String>();
        let provider = self.provider();
        quite_listie_core::engine::reminder_engine::start_reminder_tasks(
            provider,
            move |list_id, _item_id| {
                let _ = tx.try_send(list_id);
            },
        );
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = this)]
            self,
            async move {
                while let Ok(list_id) = rx.recv().await {
                    if *this.current_list_id() == list_id {
                        this.emit_by_name::<()>("current-list-externally-changed", &[]);
                    }
                }
            }
        ));
    }

    /// Select a list by id.
    pub fn select_list(&self, list_id: &str) {
        self.set_current_list_id(list_id.to_string());
        self.emit_current_list_changed();
    }

    /// Create a new list (optionally with an emoji icon) and navigate to it.
    /// Create a new **local** list (Swift `NewListView` makes a private list; Nextcloud
    /// lists are created in the file browser via [`create_list_at`]). No account required.
    pub fn create_local_list(&self, name: &str, emoji: &str) {
        let provider = self.provider();
        let name = name.to_string();
        let emoji = emoji.to_string();
        spawn_to_main(
            async move {
                let mut p = provider.lock().await;
                let res = p.create_local_list(&name);
                if let Ok(id) = &res {
                    if !emoji.is_empty() {
                        p.set_list_emoji_icon(id, Some(emoji));
                    }
                }
                res
            },
            glib::clone!(
                #[weak(rename_to = this)]
                self,
                move |result| match result {
                    Ok(id) => {
                        this.select_list(&id);
                        this.refresh_counts_and_notify();
                    }
                    Err(e) => this.emit_error(&e.to_string()),
                }
            ),
        );
    }

    /// Permanently delete a list and its underlying file.
    pub fn delete_list_permanently(&self, list_id: &str) {
        let id = list_id.to_string();
        if *self.current_list_id() == id {
            self.set_current_list_id(String::new());
            self.emit_current_list_changed();
        }
        let provider = self.provider();
        spawn_to_main(
            async move {
                let mut p = provider.lock().await;
                p.delete_list_permanently(&id).await
            },
            glib::clone!(
                #[weak(rename_to = this)]
                self,
                move |result| match result {
                    Ok(()) => this.refresh_counts_and_notify(),
                    Err(e) => this.emit_error(&e.to_string()),
                }
            ),
        );
    }

    /// Rename a list (no-op on empty/whitespace names).
    pub fn rename_list(&self, list_id: &str, new_name: &str) {
        if new_name.trim().is_empty() {
            return;
        }
        {
            let provider = self.provider();
            let mut p = provider.blocking_lock();
            p.rename_list(list_id, new_name);
            p.trigger_autosave(list_id);
        }
        self.refresh_counts_and_notify();
    }

    /// Close/exclude a list from the sidebar (keeps the underlying file).
    pub fn exclude_list(&self, list_id: &str) {
        self.provider().blocking_lock().exclude_list(list_id);
        self.refresh_counts_and_notify();
    }

    // ----- list metadata getters -------------------------------------------

    pub fn list_name(&self, list_id: &str) -> String {
        let provider = self.provider();
        let p = provider.blocking_lock();
        p.lists
            .iter()
            .find(|l| l.id == list_id)
            .map(|l| l.name.clone())
            .unwrap_or_default()
    }

    pub fn list_emoji_icon(&self, list_id: &str) -> String {
        let provider = self.provider();
        let p = provider.blocking_lock();
        p.lists
            .iter()
            .find(|l| l.id == list_id)
            .and_then(|l| l.emoji_icon.clone())
            .unwrap_or_default()
    }

    // ----- Nextcloud account flows -----------------------------------------

    /// Begin Nextcloud Login Flow v2: opens the server's browser login page and polls
    /// for the resulting app password. On success, stores the credentials and emits
    /// `nc-login-completed`; on failure, emits `error-occurred`. Cancellable via
    /// [`cancel_nextcloud_login`].
    pub fn start_nextcloud_login(&self, server_url: &str, lists_path: &str) {
        let server = normalize_server_url(server_url);
        let path = lists_path.to_string();
        let (tx, rx) = async_channel::bounded::<anyhow::Result<NextcloudCredentials>>(1);
        let handle = runtime().spawn(async move {
            let result =
                quite_listie_core::engine::nextcloud::login_flow_v2(&server, &path).await;
            let _ = tx.send(result).await;
        });
        self.imp().login_task.replace(Some(handle.abort_handle()));
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = this)]
            self,
            async move {
                if let Ok(result) = rx.recv().await {
                    this.imp().login_task.replace(None);
                    match result {
                        Ok(creds) => {
                            this.provider().blocking_lock().nc.set_credentials(creds);
                            this.set_is_nc_authenticated(true);
                            this.emit_nc_login_completed();
                        }
                        Err(e) => this.emit_error(&e.to_string()),
                    }
                }
            }
        ));
    }

    /// Abort an in-flight Login Flow v2 poll started by [`start_nextcloud_login`].
    pub fn cancel_nextcloud_login(&self) {
        if let Some(h) = self.imp().login_task.replace(None) {
            h.abort();
        }
    }

    /// Verify manual credentials without saving them. Result is delivered via the
    /// `nc-test-result` signal as `(ok, message)`.
    pub fn test_nextcloud_credentials(
        &self,
        server: &str,
        username: &str,
        password: &str,
        lists_path: &str,
    ) {
        let creds = NextcloudCredentials {
            server_url: normalize_server_url(server),
            username: username.to_string(),
            app_password: password.to_string(),
            lists_remote_path: lists_path.to_string(),
        };
        spawn_to_main(
            async move {
                match creds.test_connection().await {
                    Ok(()) => (true, "Connected successfully".to_string()),
                    Err(e) => (false, e.to_string()),
                }
            },
            glib::clone!(
                #[weak(rename_to = this)]
                self,
                move |(ok, msg): (bool, String)| {
                    this.emit_by_name::<()>("nc-test-result", &[&ok, &msg]);
                }
            ),
        );
    }

    /// Connect using manually entered credentials: persists them, marks the account
    /// authenticated, and emits `nc-login-completed`.
    pub fn connect_nextcloud_manual(
        &self,
        server: &str,
        username: &str,
        password: &str,
        lists_path: &str,
    ) {
        let creds = NextcloudCredentials {
            server_url: normalize_server_url(server),
            username: username.to_string(),
            app_password: password.to_string(),
            lists_remote_path: lists_path.to_string(),
        };
        match creds.save() {
            Ok(()) => {
                self.provider().blocking_lock().nc.set_credentials(creds);
                self.set_is_nc_authenticated(true);
                self.emit_nc_login_completed();
            }
            Err(e) => self.emit_error(&e.to_string()),
        }
    }

    /// Log out of Nextcloud: clears credentials and removes NC lists from the sidebar.
    pub fn nextcloud_logout(&self) {
        {
            let provider = self.provider();
            let mut p = provider.blocking_lock();
            p.nc.logout();
            p.clear_nc_lists();
        }
        self.set_is_nc_authenticated(false);
        self.refresh_counts_and_notify();
    }

    pub fn nc_server_url(&self) -> String {
        self.provider().blocking_lock().nc.server_url().to_string()
    }

    pub fn nc_remote_path(&self) -> String {
        self.provider().blocking_lock().nc.lists_remote_path().to_string()
    }

    pub fn set_nc_remote_path(&self, path: &str) {
        self.provider()
            .blocking_lock()
            .nc
            .update_lists_remote_path(path);
    }

    // ----- Nextcloud browser + opening -------------------------------------

    /// List the contents of a Nextcloud directory (relative to the DAV root). The
    /// result is delivered via the `remote-files-ready` signal as a JSON array of
    /// `{name, isDirectory, alreadyOpen, displayName, remotePath}` objects.
    pub fn browse_nextcloud_at(&self, path: &str) {
        let path = path.to_string();
        let provider = self.provider();
        spawn_to_main(
            async move {
                let p = provider.lock().await;
                if !p.nc.is_authenticated() {
                    anyhow::bail!("not authenticated");
                }
                let entries = p.nc.list_files_at(&path).await?;
                // Remote paths already open in the sidebar, to flag duplicates.
                let open_paths: std::collections::HashSet<String> = p
                    .lists
                    .iter()
                    .filter_map(|l| match &l.source {
                        ListSource::Nextcloud { remote_path, .. } => Some(remote_path.clone()),
                        _ => None,
                    })
                    .collect();
                let cache_names = p.cache_names();
                let base = if path.trim_end_matches('/').is_empty() {
                    ""
                } else {
                    path.trim_end_matches('/')
                };
                let items: Vec<String> = entries
                    .iter()
                    .map(|entry| {
                        let remote_path = format!("{}/{}", base, entry.name);
                        let already_open = open_paths.contains(&remote_path);
                        let runtime_id = format!("nextcloud:{remote_path}");
                        let display = cache_names
                            .get(&runtime_id)
                            .cloned()
                            .unwrap_or_else(|| {
                                entry.name.trim_end_matches(".listie").to_string()
                            });
                        format!(
                            r#"{{"name":{},"isDirectory":{},"alreadyOpen":{},"displayName":{},"remotePath":{}}}"#,
                            serde_json::to_string(&entry.name).unwrap_or_default(),
                            entry.is_directory,
                            already_open,
                            serde_json::to_string(&display).unwrap_or_default(),
                            serde_json::to_string(&remote_path).unwrap_or_default(),
                        )
                    })
                    .collect();
                Ok(format!("[{}]", items.join(",")))
            },
            glib::clone!(
                #[weak(rename_to = this)]
                self,
                move |result: anyhow::Result<String>| match result {
                    Ok(json) => this.emit_by_name::<()>("remote-files-ready", &[&json]),
                    Err(e) => this.emit_error(&e.to_string()),
                }
            ),
        );
    }

    /// Open an existing remote `.listie` file (by bare file name) and navigate to it.
    pub fn open_remote_list(&self, file_name: &str) {
        let file_name = file_name.to_string();
        let provider = self.provider();
        spawn_to_main(
            async move {
                let mut p = provider.lock().await;
                p.open_remote_list(&file_name).await
            },
            glib::clone!(
                #[weak(rename_to = this)]
                self,
                move |result| match result {
                    Ok(id) => {
                        this.select_list(&id);
                        this.refresh_counts_and_notify();
                    }
                    Err(e) => this.emit_error(&e.to_string()),
                }
            ),
        );
    }

    /// Create a new `.listie` file in a remote folder, then open and navigate to it.
    pub fn create_list_at(&self, remote_folder: &str, name: &str) {
        let folder = remote_folder.to_string();
        let name = name.to_string();
        let provider = self.provider();
        spawn_to_main(
            async move {
                let file_name = format!("{}.listie", name.trim());
                let remote_path = if folder.trim_end_matches('/').is_empty() {
                    format!("/{}", file_name)
                } else {
                    format!("{}/{}", folder.trim_end_matches('/'), file_name)
                };
                let doc = quite_listie_core::model::ListDocument::new(&name);
                let mut p = provider.lock().await;
                p.nc.save_file(&file_name, &remote_path, &doc).await?;
                p.open_remote_list(&file_name).await
            },
            glib::clone!(
                #[weak(rename_to = this)]
                self,
                move |result: anyhow::Result<String>| match result {
                    Ok(id) => {
                        this.select_list(&id);
                        this.refresh_counts_and_notify();
                    }
                    Err(e) => this.emit_error(&e.to_string()),
                }
            ),
        );
    }

    /// Open a local `.listie` file from the filesystem and navigate to it. Accepts a
    /// plain path or a `file://` URI (as handed back by `gtk::FileDialog`).
    pub fn open_external_file(&self, path: &str) {
        let path = if path.starts_with("file://") {
            gtk::gio::File::for_uri(path)
                .path()
                .unwrap_or_else(|| std::path::PathBuf::from(path))
        } else {
            std::path::PathBuf::from(path)
        };
        let result = self.provider().blocking_lock().open_external_file(path);
        match result {
            Ok(id) => {
                self.select_list(&id);
                self.refresh_counts_and_notify();
            }
            Err(e) => self.emit_error(&e.to_string()),
        }
    }

    // ----- labels ----------------------------------------------------------

    /// The labels defined on a list, in `label_order`.
    pub fn labels(&self, list_id: &str) -> Vec<ListLabel> {
        self.provider()
            .blocking_lock()
            .cached_doc(list_id)
            .map(|d| d.labels.clone())
            .unwrap_or_default()
    }

    /// Add a new label (a fresh UUID) to a list. `emoji` may be empty.
    pub fn add_label(&self, list_id: &str, name: &str, color: &str, emoji: &str) {
        let mut label = ListLabel::new(uuid::Uuid::new_v4().to_string(), name, color);
        if !emoji.is_empty() {
            label.emoji_icon = Some(emoji.to_string());
        }
        self.mutate_labels(list_id, |p| p.add_label(list_id, label));
    }

    /// Update an existing label in place (matched by `id`).
    pub fn update_label(&self, list_id: &str, id: &str, name: &str, color: &str, emoji: &str) {
        let mut label = ListLabel::new(id, name, color);
        if !emoji.is_empty() {
            label.emoji_icon = Some(emoji.to_string());
        }
        self.mutate_labels(list_id, |p| p.update_label(list_id, label));
    }

    /// Delete a label from a list.
    pub fn delete_label(&self, list_id: &str, id: &str) {
        let id = id.to_string();
        self.mutate_labels(list_id, |p| p.delete_label(list_id, &id));
    }

    /// Reorder a label within `label_order` (no-op on equal/negative indices).
    pub fn move_label(&self, list_id: &str, from: usize, to: usize) {
        if from == to {
            return;
        }
        self.mutate_labels(list_id, |p| p.move_label_order(list_id, from, to));
    }

    /// Add the built-in grocery preset labels to a list.
    pub fn apply_grocery_presets(&self, list_id: &str) {
        self.mutate_labels(list_id, |p| {
            for preset in quite_listie_core::presets::GROCERY_LABELS {
                p.add_label(list_id, ListLabel::new(preset.id, preset.name, preset.color));
            }
        });
    }

    /// Run a label mutation against the provider, autosave the list, and notify so the
    /// open list page re-renders item label dots.
    fn mutate_labels<F: FnOnce(&mut UnifiedProvider)>(&self, list_id: &str, f: F) {
        {
            let provider = self.provider();
            let mut p = provider.blocking_lock();
            f(&mut p);
            p.trigger_autosave(list_id);
        }
        if *self.current_list_id() == *list_id {
            self.emit_by_name::<()>("current-list-externally-changed", &[]);
        }
    }

    // ----- markdown export -------------------------------------------------

    /// Render the list as markdown for the export view. Mirrors Swift `MarkdownExportView`
    /// (which calls `MarkdownListGenerator.generate`). Returns the markdown and any
    /// per-item notes that couldn't be exported.
    pub fn export_markdown(
        &self,
        list_id: &str,
        active_only: bool,
        include_notes: bool,
    ) -> quite_listie_core::util::markdown_generator::ExportResult {
        let provider = self.provider();
        let p = provider.blocking_lock();
        match p.cached_doc(list_id) {
            Some(doc) => quite_listie_core::util::markdown_generator::generate_markdown_export(
                doc,
                active_only,
                include_notes,
            ),
            None => Default::default(),
        }
    }

    // ----- markdown import -------------------------------------------------

    /// Open lists the user can import into, as `(id, name)` sorted by name. Mirrors the
    /// Swift import list-picker (paste intent: "Pick a list to import into").
    pub fn importable_lists(&self) -> Vec<(String, String)> {
        let provider = self.provider();
        let p = provider.blocking_lock();
        let mut out: Vec<(String, String)> =
            p.lists.iter().map(|l| (l.id.clone(), l.name.clone())).collect();
        out.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
        out
    }

    /// Preview stats for importing `markdown` into a list (no mutation). Mirrors Swift
    /// `MarkdownImportLogic.mergeStats`.
    pub fn import_preview(
        &self,
        target_list_id: &str,
        markdown: &str,
        create_labels: bool,
    ) -> quite_listie_core::util::markdown_import::MergeStats {
        let parsed = quite_listie_core::util::markdown_parser::parse_markdown(markdown, None);
        let provider = self.provider();
        let p = provider.blocking_lock();
        let Some(doc) = p.cached_doc(target_list_id) else {
            return Default::default();
        };
        quite_listie_core::util::markdown_import::merge_stats(&parsed, &doc.items, &doc.labels, create_labels)
    }

    /// Apply a markdown import into the target list: create missing labels (when enabled),
    /// then add new items / merge into matching ones (quantity add-or-replace, re-activate,
    /// re-label). Returns `(new_items, updated_items, new_labels)`. Mirrors Swift
    /// `MarkdownListImportView.importItems`.
    pub fn import_markdown(
        &self,
        target_list_id: &str,
        markdown: &str,
        replace_quantities: bool,
        create_labels: bool,
    ) -> (usize, usize, usize) {
        use quite_listie_core::model::ListItem;
        use quite_listie_core::util::markdown_import::{match_existing, parsed_label_name};

        let parsed = quite_listie_core::util::markdown_parser::parse_markdown(markdown, None);

        let provider = self.provider();
        let mut p = provider.blocking_lock();
        let Some(doc) = p.cached_doc(target_list_id) else {
            return (0, 0, 0);
        };
        let existing_items = doc.items.clone();
        let existing_labels = doc.labels.clone();

        // Map each parsed label name -> target label id (existing or freshly created).
        let mut label_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let mut new_labels = 0usize;
        for item in &parsed.items {
            let Some(name) = parsed_label_name(&parsed, item) else { continue };
            if label_map.contains_key(&name) {
                continue;
            }
            if let Some(existing) = existing_labels.iter().find(|l| l.name.to_lowercase() == name.to_lowercase()) {
                label_map.insert(name, existing.id.clone());
            } else if create_labels {
                let color = parsed
                    .labels
                    .iter()
                    .find(|l| l.name == name)
                    .map(|l| l.color.clone())
                    .unwrap_or_else(|| "#607D8B".to_string());
                let new_label = ListLabel::new(uuid::Uuid::new_v4().to_string(), &name, &color);
                let new_id = new_label.id.clone();
                p.add_label(target_list_id, new_label);
                label_map.insert(name, new_id);
                new_labels += 1;
            }
        }

        let mut new_items = 0usize;
        let mut updated_items = 0usize;
        for parsed_item in &parsed.items {
            let target_label = parsed_label_name(&parsed, parsed_item).and_then(|n| label_map.get(&n).cloned());
            match match_existing(&parsed_item.note, &existing_items) {
                Some(existing) => {
                    let mut updated = existing.clone();
                    updated.quantity = if replace_quantities || existing.checked {
                        parsed_item.quantity
                    } else {
                        existing.quantity + parsed_item.quantity
                    };
                    updated.checked = false;
                    if let Some(label_id) = target_label {
                        updated.label_id = Some(label_id);
                    }
                    if parsed_item.markdown_notes.is_some() {
                        updated.markdown_notes = parsed_item.markdown_notes.clone();
                    }
                    p.update_item(target_list_id, updated);
                    updated_items += 1;
                }
                None => {
                    let mut item = ListItem::new(&parsed_item.note);
                    item.quantity = parsed_item.quantity;
                    item.checked = parsed_item.checked;
                    item.label_id = target_label;
                    item.markdown_notes = parsed_item.markdown_notes.clone();
                    p.add_item(target_list_id, item);
                    new_items += 1;
                }
            }
        }

        p.trigger_autosave(target_list_id);
        drop(p);

        if *self.current_list_id() == *target_list_id {
            self.emit_by_name::<()>("current-list-externally-changed", &[]);
        }
        self.emit_lists_updated();
        (new_items, updated_items, new_labels)
    }

    // ----- recycle bin -----------------------------------------------------

    /// Soft-deleted items for a list, newest first, with the days-since-deletion used by
    /// the recycle-bin auto-delete countdown. Mirrors Swift `RecycleBinView`.
    pub fn deleted_items(&self, list_id: &str) -> Vec<DeletedItemRow> {
        let provider = self.provider();
        let p = provider.blocking_lock();
        let Some(doc) = p.cached_doc(list_id) else {
            return Vec::new();
        };
        let now = Utc::now();
        let mut rows: Vec<DeletedItemRow> = doc
            .deleted_items()
            .map(|i| {
                let when = i.deleted_at.unwrap_or(i.modified_at);
                DeletedItemRow {
                    id: i.id.to_string(),
                    note: i.note.clone(),
                    days_ago: (now - when).num_days().max(0),
                    deleted_ts: when.timestamp(),
                }
            })
            .collect();
        rows.sort_by(|a, b| b.deleted_ts.cmp(&a.deleted_ts));
        rows
    }

    /// Restore a soft-deleted item back into the active list.
    pub fn restore_item(&self, list_id: &str, item_id: &str) {
        if let Ok(id) = uuid::Uuid::parse_str(item_id) {
            self.mutate_items(list_id, |p| p.restore_item(list_id, id));
        }
    }

    /// Permanently remove a soft-deleted item (cannot be undone).
    pub fn permanently_delete_item(&self, list_id: &str, item_id: &str) {
        if let Ok(id) = uuid::Uuid::parse_str(item_id) {
            self.mutate_items(list_id, |p| p.permanently_delete_item(list_id, id));
        }
    }

    /// Restore every soft-deleted item in the list.
    pub fn restore_all_deleted(&self, list_id: &str) {
        let ids: Vec<uuid::Uuid> = self
            .deleted_items(list_id)
            .iter()
            .filter_map(|r| uuid::Uuid::parse_str(&r.id).ok())
            .collect();
        self.mutate_items(list_id, |p| {
            for id in ids {
                p.restore_item(list_id, id);
            }
        });
    }

    /// Permanently delete every soft-deleted item in the list.
    pub fn purge_all_deleted(&self, list_id: &str) {
        let ids: Vec<uuid::Uuid> = self
            .deleted_items(list_id)
            .iter()
            .filter_map(|r| uuid::Uuid::parse_str(&r.id).ok())
            .collect();
        self.mutate_items(list_id, |p| {
            for id in ids {
                p.permanently_delete_item(list_id, id);
            }
        });
    }

    // ----- share link ------------------------------------------------------

    /// The list's active items for the share picker, sorted by label order then note.
    /// Mirrors Swift `ShareLinkSheet.groupedItems` (flattened; the view inserts headers).
    pub fn share_items(&self, list_id: &str) -> Vec<ShareItem> {
        let provider = self.provider();
        let p = provider.blocking_lock();
        let Some(doc) = p.cached_doc(list_id) else {
            return Vec::new();
        };
        let order = &doc.list.label_order;
        let label_name = |item: &quite_listie_core::model::ListItem| -> String {
            item.label_id
                .as_deref()
                .and_then(|id| doc.labels.iter().find(|l| l.id == id))
                .map(|l| l.name.clone())
                .unwrap_or_else(|| "No Label".to_string())
        };
        let label_index = |item: &quite_listie_core::model::ListItem| -> usize {
            item.label_id
                .as_deref()
                .and_then(|id| order.iter().position(|o| o == id))
                .unwrap_or(usize::MAX)
        };
        let mut indexed: Vec<(usize, ShareItem)> = doc
            .active_items()
            .map(|i| {
                (
                    label_index(i),
                    ShareItem {
                        id: i.id.to_string(),
                        note: i.note.clone(),
                        quantity: i.quantity,
                        checked: i.checked,
                        label_name: label_name(i),
                    },
                )
            })
            .collect();
        // Sort by label order, then label name, then note (matches the export ordering).
        indexed.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| a.1.label_name.to_lowercase().cmp(&b.1.label_name.to_lowercase()))
                .then_with(|| a.1.note.to_lowercase().cmp(&b.1.note.to_lowercase()))
        });
        indexed.into_iter().map(|(_, r)| r).collect()
    }

    /// Build a `quitelistie://import` share URL for the selected items. Mirrors Swift
    /// `ShareLinkSheet.generateShareURL` (markdown of the selected items, then zlib/base64
    /// encoded). Returns an empty string when nothing is selected.
    pub fn build_share_url(
        &self,
        list_id: &str,
        selected_ids: &[String],
        include_comments: bool,
        compress: bool,
    ) -> String {
        if selected_ids.is_empty() {
            return String::new();
        }
        let provider = self.provider();
        let p = provider.blocking_lock();
        let Some(doc) = p.cached_doc(list_id) else {
            return String::new();
        };
        let selected: std::collections::HashSet<&str> = selected_ids.iter().map(|s| s.as_str()).collect();
        let items: Vec<&quite_listie_core::model::ListItem> = doc
            .items
            .iter()
            .filter(|i| !i.is_deleted && selected.contains(i.id.to_string().as_str()))
            .collect();
        let result = quite_listie_core::util::markdown_generator::generate_markdown_export_items(
            &doc.list.name,
            &items,
            &doc.labels,
            &doc.list.label_order,
            false,
            include_comments,
        );
        quite_listie_core::util::deeplink::build_import_url(list_id, &result.markdown, true, compress)
            .unwrap_or_default()
    }

    // ----- recent changes --------------------------------------------------

    /// The 30 most recently changed items (those with a `last_change_field`), newest first.
    /// Mirrors Swift `RecentChangesView.loadRecentItems`.
    pub fn recent_changes(&self, list_id: &str) -> Vec<RecentChangeRow> {
        let provider = self.provider();
        let p = provider.blocking_lock();
        let Some(doc) = p.cached_doc(list_id) else {
            return Vec::new();
        };
        let mut rows: Vec<RecentChangeRow> = doc
            .items
            .iter()
            .filter(|i| i.last_change_field.is_some())
            .map(|i| RecentChangeRow {
                id: i.id.to_string(),
                note: i.note.clone(),
                change: i.last_change_field.clone().unwrap_or_default(),
                checked: i.checked,
                is_deleted: i.is_deleted,
                modified_ts: i.modified_at.timestamp(),
            })
            .collect();
        rows.sort_by(|a, b| b.modified_ts.cmp(&a.modified_ts));
        rows.truncate(30);
        rows
    }

    /// Undo a recent change: toggle a `checked` change back, or restore a `deleted` one.
    /// Other change kinds aren't undoable (matches Swift `canUndo`).
    pub fn undo_change(&self, list_id: &str, item_id: &str) {
        let Ok(id) = uuid::Uuid::parse_str(item_id) else {
            return;
        };
        self.mutate_items(list_id, |p| {
            let field = p
                .cached_doc(list_id)
                .and_then(|d| d.items.iter().find(|i| i.id == id))
                .and_then(|i| i.last_change_field.clone());
            match field.as_deref() {
                Some("checked") => {
                    let updated = p
                        .cached_doc(list_id)
                        .and_then(|d| d.items.iter().find(|i| i.id == id))
                        .cloned();
                    if let Some(mut item) = updated {
                        item.checked = !item.checked;
                        item.modified_at = Utc::now();
                        item.checked_at = Some(Utc::now());
                        item.last_change_field = Some("checked".into());
                        p.update_item(list_id, item);
                    }
                }
                Some("deleted") => p.restore_item(list_id, id),
                _ => {}
            }
        });
    }

    /// A short human description of the list's source + sync state, for the recent-changes
    /// "Sync Activity" section. Mirrors Swift `sourceDescription` / `syncStateDescription`.
    pub fn list_sync_state(&self, list_id: &str) -> String {
        let provider = self.provider();
        let p = provider.blocking_lock();
        if p.sync_pending_ids.contains(list_id) {
            return "Pending sync (offline)".to_string();
        }
        match p.lists.iter().find(|l| l.id == list_id) {
            Some(l) if l.is_dirty => "Unsaved changes".to_string(),
            Some(_) => "Synced".to_string(),
            None => "Unknown".to_string(),
        }
    }

    /// Run an item mutation, autosave, and refresh the open list + sidebar counts.
    fn mutate_items<F: FnOnce(&mut UnifiedProvider)>(&self, list_id: &str, f: F) {
        {
            let provider = self.provider();
            let mut p = provider.blocking_lock();
            f(&mut p);
            p.trigger_autosave(list_id);
        }
        if *self.current_list_id() == *list_id {
            self.emit_by_name::<()>("current-list-externally-changed", &[]);
        }
        self.emit_lists_updated();
    }

    // ----- settings convenience --------------------------------------------

    pub fn show_completed_at_bottom(&self) -> bool {
        self.settings().boolean("show-completed-at-bottom")
    }

    pub fn set_show_completed_at_bottom(&self, value: bool) {
        let _ = self.settings().set_boolean("show-completed-at-bottom", value);
    }

    pub fn hide_empty_labels(&self) -> bool {
        self.settings().boolean("hide-empty-labels")
    }

    pub fn set_hide_empty_labels(&self, value: bool) {
        let _ = self.settings().set_boolean("hide-empty-labels", value);
    }

    // ----- per-section expand/collapse persistence -------------------------
    // Stored in the `sections-json` setting as `{ "<listId>": { "<section>": bool } }`,
    // mirroring the Swift app's per-list `expandedSections` in UserDefaults. Sections
    // default to expanded.

    pub fn section_expanded(&self, list_id: &str, section: &str) -> bool {
        serde_json::from_str::<serde_json::Value>(&self.settings().string("sections-json"))
            .ok()
            .and_then(|v| v.get(list_id)?.get(section)?.as_bool())
            .unwrap_or(true)
    }

    pub fn set_section_expanded(&self, list_id: &str, section: &str, value: bool) {
        let mut v: serde_json::Value =
            serde_json::from_str(&self.settings().string("sections-json")).unwrap_or_default();
        if !v.is_object() {
            v = serde_json::json!({});
        }
        v[list_id][section] = serde_json::Value::Bool(value);
        let _ = self.settings().set_string("sections-json", &v.to_string());
    }

    // ----- per-list view mode (list/kanban) persistence --------------------
    // Stored in `view-modes-json` as `{ "<listId>": "list"|"kanban" }`, mirroring the
    // Swift `ListViewModel.viewMode` (`listViewMode` UserDefaults dict). Map mode is
    // reached via the per-list map page, not persisted here. Defaults to "list".

    pub fn list_view_mode(&self, list_id: &str) -> String {
        serde_json::from_str::<serde_json::Value>(&self.settings().string("view-modes-json"))
            .ok()
            .and_then(|v| v.get(list_id)?.as_str().map(String::from))
            .filter(|m| m == "kanban")
            .unwrap_or_else(|| "list".to_string())
    }

    pub fn set_list_view_mode(&self, list_id: &str, mode: &str) {
        let mut v: serde_json::Value =
            serde_json::from_str(&self.settings().string("view-modes-json")).unwrap_or_default();
        if !v.is_object() {
            v = serde_json::json!({});
        }
        if mode == "list" {
            if let Some(obj) = v.as_object_mut() {
                obj.remove(list_id);
            }
        } else {
            v[list_id] = serde_json::Value::String(mode.to_string());
        }
        let _ = self.settings().set_string("view-modes-json", &v.to_string());
        // Rebuild the open page (window listens on `list-settings-changed`).
        self.emit_by_name::<()>("list-settings-changed", &[]);
    }

    // ----- per-list "completed at bottom" persistence ----------------------
    // Stored in `completed-at-bottom-json` as `{ "<listId>": bool }`. Mirrors the Swift
    // per-list `ListViewModel.showCompletedAtBottom`. Lists without an entry fall back to
    // the global `show-completed-at-bottom` default.

    pub fn list_completed_at_bottom(&self, list_id: &str) -> bool {
        serde_json::from_str::<serde_json::Value>(&self.settings().string("completed-at-bottom-json"))
            .ok()
            .and_then(|v| v.get(list_id)?.as_bool())
            .unwrap_or_else(|| self.show_completed_at_bottom())
    }

    pub fn set_list_completed_at_bottom(&self, list_id: &str, value: bool) {
        let mut v: serde_json::Value =
            serde_json::from_str(&self.settings().string("completed-at-bottom-json")).unwrap_or_default();
        if !v.is_object() {
            v = serde_json::json!({});
        }
        v[list_id] = serde_json::Value::Bool(value);
        let _ = self.settings().set_string("completed-at-bottom-json", &v.to_string());
        self.emit_by_name::<()>("list-settings-changed", &[]);
    }

    // ----- list settings (title/icon/favourite/background/map/labels) -------
    // Backs `views::list_settings`. Mirrors the Swift `ListSettingsView`: title +
    // icon + favourite + source on the provider/GSettings, display + background
    // preferences in GSettings, label show/hide on the provider.

    /// Human-readable source of a list (Nextcloud remote path, or external file
    /// path). Mirrors the KDE `AppController::list_source_description`.
    pub fn list_source_description(&self, list_id: &str) -> String {
        let provider = self.provider();
        let p = provider.blocking_lock();
        p.lists
            .iter()
            .find(|l| l.id == list_id)
            .map(|l| match &l.source {
                ListSource::Nextcloud { remote_path, .. } => remote_path.clone(),
                ListSource::ExternalFile { path } => path.display().to_string(),
            })
            .unwrap_or_default()
    }

    /// Set (or clear, when empty) the list's emoji icon.
    pub fn set_list_emoji_icon(&self, list_id: &str, emoji: &str) {
        let emoji = (!emoji.is_empty()).then(|| emoji.to_string());
        self.mutate_labels(list_id, |p| p.set_list_emoji_icon(list_id, emoji));
    }

    pub fn list_enable_map_data(&self, list_id: &str) -> bool {
        let provider = self.provider();
        let p = provider.blocking_lock();
        p.cached_doc(list_id)
            .map(|d| d.list.enable_map_data)
            .unwrap_or(false)
    }

    pub fn set_list_enable_map_data(&self, list_id: &str, enabled: bool) {
        self.mutate_labels(list_id, |p| p.set_list_enable_map_data(list_id, enabled));
    }

    pub fn is_label_hidden(&self, list_id: &str, label_id: &str) -> bool {
        let provider = self.provider();
        let p = provider.blocking_lock();
        p.is_label_hidden(list_id, label_id)
    }

    pub fn toggle_label_hidden(&self, list_id: &str, label_id: &str) {
        self.mutate_labels(list_id, |p| p.toggle_label_hidden(list_id, label_id));
    }

    /// Favourites are persisted as a JSON array of list ids in the
    /// `favourite-list-ids-json` GSettings key (matches the KDE port).
    pub fn is_favourite(&self, list_id: &str) -> bool {
        serde_json::from_str::<Vec<String>>(&self.settings().string("favourite-list-ids-json"))
            .unwrap_or_default()
            .iter()
            .any(|id| id == list_id)
    }

    pub fn set_favourite(&self, list_id: &str, value: bool) {
        let mut ids: Vec<String> =
            serde_json::from_str(&self.settings().string("favourite-list-ids-json"))
                .unwrap_or_default();
        let present = ids.iter().any(|id| id == list_id);
        if value && !present {
            ids.push(list_id.to_string());
        } else if !value {
            ids.retain(|id| id != list_id);
        }
        let _ = self
            .settings()
            .set_string("favourite-list-ids-json", &serde_json::to_string(&ids).unwrap_or_default());
    }

    /// The chosen background gradient id for a list (empty = default), stored in
    /// the `backgrounds-json` GSettings object `{ "<listId>": "<gradientId>" }`.
    pub fn list_background(&self, list_id: &str) -> String {
        serde_json::from_str::<serde_json::Value>(&self.settings().string("backgrounds-json"))
            .ok()
            .and_then(|v| v.get(list_id)?.as_str().map(String::from))
            .unwrap_or_default()
    }

    pub fn set_list_background(&self, list_id: &str, gradient_id: &str) {
        let mut v: serde_json::Value =
            serde_json::from_str(&self.settings().string("backgrounds-json")).unwrap_or_default();
        if !v.is_object() {
            v = serde_json::json!({});
        }
        if gradient_id.is_empty() {
            if let Some(obj) = v.as_object_mut() {
                obj.remove(list_id);
            }
        } else {
            v[list_id] = serde_json::Value::String(gradient_id.to_string());
        }
        let _ = self.settings().set_string("backgrounds-json", &v.to_string());
    }

    /// Notify that list settings were saved: rebuilds the open list page and
    /// refreshes the sidebar (title/icon). Emits both `list-settings-changed`
    /// and `lists-updated`.
    pub fn notify_list_settings_changed(&self) {
        self.emit_by_name::<()>("list-settings-changed", &[]);
        self.emit_lists_updated();
    }
}
