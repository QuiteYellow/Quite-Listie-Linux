use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::engine::merge::merge_documents;
use crate::model::ListDocument;

// ---------------------------------------------------------------------------
// Credentials
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NextcloudCredentials {
    pub server_url: String,
    pub username: String,
    pub app_password: String,
    pub lists_remote_path: String,
}

impl NextcloudCredentials {
    fn path() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| dirs::home_dir().unwrap().join(".local/share"))
            .join("quite-listie")
            .join("nc-credentials.json")
    }

    pub fn load() -> Option<Self> {
        let data = std::fs::read_to_string(Self::path()).ok()?;
        serde_json::from_str(&data).ok()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::path();
        std::fs::create_dir_all(path.parent().unwrap())?;
        let json = serde_json::to_string_pretty(self)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true).mode(0o600);
            opts.open(&path)?.write_all(json.as_bytes())?;
        }
        #[cfg(not(unix))]
        std::fs::write(&path, json)?;
        Ok(())
    }

    /// WebDAV root for this account: `server/remote.php/dav/files/username`
    /// Does NOT include lists_remote_path — use this for file-browser operations.
    pub fn dav_root(&self) -> String {
        format!(
            "{}/remote.php/dav/files/{}",
            self.server_url.trim_end_matches('/'),
            self.username,
        )
    }

    /// WebDAV base including the configured lists directory.
    /// Used for flat listing and building file URLs within the configured path.
    pub fn dav_base(&self) -> String {
        format!(
            "{}/remote.php/dav/files/{}{}",
            self.server_url.trim_end_matches('/'),
            self.username,
            self.lists_remote_path,
        )
    }

    /// Full WebDAV URL for an arbitrary remote path (relative to dav_root).
    /// `remote_path` should start with `/`, e.g. `/Lists/groceries.listie`.
    /// Path segments are percent-encoded so filenames with spaces and special
    /// characters are transmitted correctly.
    pub fn dav_url_for(&self, remote_path: &str) -> String {
        // Encode each path segment individually (leaves '/' separators intact).
        let encoded: String = remote_path
            .split('/')
            .map(|seg| urlencoding::encode(seg).into_owned())
            .collect::<Vec<_>>()
            .join("/");
        let clean = if encoded.starts_with('/') { encoded } else { format!("/{}", encoded) };
        format!("{}{}", self.dav_root(), clean)
    }

    /// Verify these credentials by issuing a depth-0 PROPFIND against the configured
    /// lists directory. Returns `Ok(())` if the server accepts them, otherwise an error
    /// describing the failure. Used by the "Test connection" button in manual setup.
    pub async fn test_connection(&self) -> anyhow::Result<()> {
        let client = Client::builder()
            .user_agent("QuiteListie/0.1")
            .timeout(std::time::Duration::from_secs(10))
            .build()?;
        let body = r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:"><d:prop><d:getetag/></d:prop></d:propfind>"#;
        let resp = client
            .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), self.dav_base())
            .basic_auth(&self.username, Some(&self.app_password))
            .header("Depth", "0")
            .header("Content-Type", "application/xml")
            .body(body)
            .send()
            .await?;
        let status = resp.status();
        if status.is_success() || status.as_u16() == 207 {
            Ok(())
        } else {
            anyhow::bail!("Server returned {}", status)
        }
    }

    /// Human-readable account identifier: `username@hostname`
    pub fn account_id(&self) -> String {
        let host = url::Url::parse(&self.server_url)
            .ok()
            .and_then(|u| u.host_str().map(|h| h.to_string()))
            .unwrap_or_else(|| self.server_url.clone());
        format!("{}@{}", self.username, host)
    }
}

// ---------------------------------------------------------------------------
// Login Flow v2
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct LoginFlowResponse {
    login: String,
    poll: LoginFlowPoll,
}

#[derive(Debug, Deserialize)]
struct LoginFlowPoll {
    token: String,
    endpoint: String,
}

#[derive(Debug, Deserialize)]
struct LoginFlowResult {
    server: String,
    #[serde(rename = "loginName")]
    login_name: String,
    #[serde(rename = "appPassword")]
    app_password: String,
}

/// Initiate Login Flow v2: open the browser URL and poll until the user completes auth.
pub async fn login_flow_v2(
    server_url: &str,
    lists_path: &str,
) -> anyhow::Result<NextcloudCredentials> {
    let client = Client::builder()
        .user_agent("QuiteListie-KDE/0.1")
        .build()?;

    let flow_url = format!(
        "{}/index.php/login/v2",
        server_url.trim_end_matches('/')
    );
    let resp: LoginFlowResponse = client.post(&flow_url).send().await?.json().await?;

    open::that(&resp.login)?;
    info!("Opened browser for Nextcloud login: {}", resp.login);

    // Poll every 2 seconds; 20-minute timeout (matching Swift).
    let timeout = tokio::time::Instant::now() + Duration::from_secs(20 * 60);
    loop {
        if tokio::time::Instant::now() > timeout {
            anyhow::bail!("Login Flow v2 timed out");
        }
        tokio::time::sleep(Duration::from_secs(2)).await;

        let result = client
            .post(&resp.poll.endpoint)
            .form(&[("token", &resp.poll.token)])
            .send()
            .await;

        match result {
            Ok(r) if r.status().is_success() => {
                let creds: LoginFlowResult = r.json().await?;
                return Ok(NextcloudCredentials {
                    server_url: creds.server,
                    username: creds.login_name,
                    app_password: creds.app_password,
                    lists_remote_path: lists_path.to_string(),
                });
            }
            Ok(_) => continue, // 404 = still waiting
            Err(e) => warn!("poll error: {e}"),
        }
    }
}

// ---------------------------------------------------------------------------
// ETag store
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct ETagStore(HashMap<String, String>);

impl ETagStore {
    fn path() -> PathBuf {
        dirs::state_dir()
            .unwrap_or_else(|| dirs::home_dir().unwrap().join(".local/state"))
            .join("quite-listie")
            .join("nc-etags.json")
    }

    fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|d| serde_json::from_str(&d).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        let path = Self::path();
        if let Some(p) = path.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        if let Ok(json) = serde_json::to_string(self) {
            let _ = std::fs::write(path, json);
        }
    }
}

// ---------------------------------------------------------------------------
// Pending uploads
// ---------------------------------------------------------------------------

/// Set of full remote paths (relative to the DAV root) with unsaved local changes. The
/// remote path is both the upload URL source and the cache key, so a bare set suffices
/// (mirrors Swift `pendingUploads: Set<String>`).
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct PendingUploads(HashSet<String>);

impl PendingUploads {
    fn path() -> PathBuf {
        dirs::state_dir()
            .unwrap_or_else(|| dirs::home_dir().unwrap().join(".local/state"))
            .join("quite-listie")
            .join("nc-pending.json")
    }

    fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|d| serde_json::from_str(&d).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        let path = Self::path();
        if let Some(p) = path.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        if let Ok(json) = serde_json::to_string(self) {
            let _ = std::fs::write(path, json);
        }
    }
}

// ---------------------------------------------------------------------------
// Remote file entry (for directory listing)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct RemoteEntry {
    pub name: String,
    pub etag: String,
    pub is_directory: bool,
}

// ---------------------------------------------------------------------------
// Confirmed-deletion error
// ---------------------------------------------------------------------------

/// Returned by change-detection / download calls when the server has confirmed the file
/// is gone — a 404 on the file while the account root is still reachable. Distinguished
/// from transient unreachability so the provider can drop the list locally instead of
/// retrying forever (Swift `NCError.notFound` + the `nextcloudFileNotFound` notification).
#[derive(Debug)]
pub struct FileGone(pub String);

impl std::fmt::Display for FileGone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "file gone on server: {}", self.0)
    }
}

impl std::error::Error for FileGone {}

// ---------------------------------------------------------------------------
// NextcloudManager
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct NextcloudManager {
    client: Client,
    credentials: Option<NextcloudCredentials>,
    etags: ETagStore,
    mem_cache: HashMap<String, ListDocument>,
    pending: PendingUploads,
    /// Per-account disk cache directory.
    cache_dir: PathBuf,
}

impl NextcloudManager {
    pub fn new() -> Self {
        let client = Client::builder()
            .user_agent("QuiteListie-KDE/0.1")
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client");

        let credentials = NextcloudCredentials::load();
        let cache_dir = Self::make_cache_dir(credentials.as_ref());

        Self {
            client,
            credentials,
            etags: ETagStore::load(),
            mem_cache: HashMap::new(),
            pending: PendingUploads::load(),
            cache_dir,
        }
    }

    fn make_cache_dir(creds: Option<&NextcloudCredentials>) -> PathBuf {
        let base = dirs::data_dir()
            .unwrap_or_else(|| dirs::home_dir().unwrap().join(".local/share"))
            .join("quite-listie")
            .join("nc-cache");
        if let Some(c) = creds {
            let host = url::Url::parse(&c.server_url)
                .ok()
                .and_then(|u| u.host_str().map(|h| h.to_string()))
                .unwrap_or_else(|| "unknown".to_string());
            base.join(format!("{}_{}", host, c.username))
        } else {
            base.join("default")
        }
    }

    /// Cache key for a file: its full remote path, or one constructed under the configured
    /// lists directory when only the bare name is known. Scoping mem/disk/etag/pending
    /// state by the full path stops same-named files in different folders from colliding
    /// (Swift keys by `accountId:remotePath`; the per-account `cache_dir` is the account
    /// scope here).
    fn resolve_path(&self, file_name: &str, remote_path: Option<&str>) -> String {
        match remote_path {
            Some(p) => p.to_string(),
            None => format!("{}/{}", self.lists_remote_path().trim_end_matches('/'), file_name),
        }
    }

    /// On-disk cache file for a remote path: the path flattened into a single filename
    /// (Swift `localCacheURL`).
    fn disk_cache_path(&self, remote_path: &str) -> PathBuf {
        let mut name = remote_path.replace('/', "_").replace(':', "-");
        if !(name.ends_with(".listie") || name.ends_with(".json")) {
            name.push_str(".listie");
        }
        self.cache_dir.join(name)
    }

    /// Build the right error for a 404: a confirmed [`FileGone`] when the account root is
    /// still reachable, otherwise a transient "unreachable" error so the caller retries
    /// rather than dropping the list (Swift `checkFileChanged` 404 disambiguation).
    async fn not_found_error(&self, remote_path: &str) -> anyhow::Error {
        if self.is_server_reachable().await {
            anyhow::Error::new(FileGone(remote_path.to_string()))
        } else {
            anyhow::anyhow!("server unreachable (404 on {remote_path})")
        }
    }

    fn ensure_cache_dir(&self) {
        let _ = std::fs::create_dir_all(&self.cache_dir);
    }

    pub fn is_authenticated(&self) -> bool {
        self.credentials.is_some()
    }

    pub fn server_url(&self) -> &str {
        self.credentials.as_ref().map(|c| c.server_url.as_str()).unwrap_or("")
    }

    pub fn lists_remote_path(&self) -> &str {
        self.credentials.as_ref().map(|c| c.lists_remote_path.as_str()).unwrap_or("/")
    }

    pub fn update_lists_remote_path(&mut self, path: &str) {
        if let Some(creds) = &mut self.credentials {
            creds.lists_remote_path = path.to_string();
            if let Err(e) = creds.save() {
                warn!("failed to save NC credentials: {e}");
            }
        }
    }

    pub fn set_credentials(&mut self, creds: NextcloudCredentials) {
        if let Err(e) = creds.save() {
            warn!("failed to persist NC credentials: {e}");
        }
        self.cache_dir = Self::make_cache_dir(Some(&creds));
        self.credentials = Some(creds);
    }

    pub fn logout(&mut self) {
        self.credentials = None;
        let _ = std::fs::remove_file(NextcloudCredentials::path());
    }

    /// Merge mutable NC state (etags, mem_cache, pending) back from a clone that
    /// was used for network I/O outside the provider lock. Credentials and cache_dir
    /// are not touched — they can only change through explicit set_credentials/logout.
    pub fn absorb(&mut self, other: Self) {
        self.etags = other.etags;
        self.mem_cache = other.mem_cache;
        self.pending = other.pending;
    }

    /// Targeted absorb for background syncs: merges a single mem_cache entry from a
    /// clone back into self without disturbing other entries. Avoids the wholesale
    /// overwrite race that bare `absorb` has when multiple background tasks race.
    pub fn absorb_mem_entry(&mut self, remote_path: &str, doc: ListDocument) {
        self.mem_cache.insert(remote_path.to_string(), doc);
    }

    pub fn mem_cache_get(&self, remote_path: &str) -> Option<ListDocument> {
        self.mem_cache.get(remote_path).cloned()
    }

    fn creds(&self) -> anyhow::Result<&NextcloudCredentials> {
        self.credentials
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("not authenticated"))
    }

    // -----------------------------------------------------------------------
    // Disk cache helpers
    // -----------------------------------------------------------------------

    fn save_to_disk(&self, remote_path: &str, doc: &ListDocument) {
        self.ensure_cache_dir();
        let path = self.disk_cache_path(remote_path);
        match serde_json::to_vec_pretty(doc) {
            Ok(bytes) => {
                let tmp = path.with_extension("listie.tmp");
                if std::fs::write(&tmp, &bytes).is_ok() {
                    let _ = std::fs::rename(&tmp, &path);
                }
            }
            Err(e) => warn!("disk cache encode error for {remote_path}: {e}"),
        }
    }

    pub fn load_from_disk(&self, remote_path: &str) -> Option<ListDocument> {
        let path = self.disk_cache_path(remote_path);
        let bytes = std::fs::read(&path).ok()?;
        match serde_json::from_slice(&bytes) {
            Ok(doc) => Some(doc),
            Err(e) => {
                warn!("disk cache decode error for {remote_path}: {e}");
                None
            }
        }
    }

    /// Open from disk cache only (no network). Updates memCache if found. Keyed by the
    /// file's full remote path.
    pub fn open_from_disk_cache(&mut self, remote_path: &str) -> Option<ListDocument> {
        if let Some(cached) = self.mem_cache.get(remote_path) {
            return Some(cached.clone());
        }
        let doc = self.load_from_disk(remote_path)?;
        self.mem_cache.insert(remote_path.to_string(), doc.clone());
        Some(doc)
    }

    // -----------------------------------------------------------------------
    // open_file — cache-first, non-blocking (background sync triggered separately)
    // -----------------------------------------------------------------------

    /// Serves from memCache → disk cache → network download.
    /// Returns `(doc, from_cache)`. When `from_cache` is true the caller should
    /// trigger a background sync (mirrors Swift `NextcloudManager.openFile`,
    /// NextcloudManager.swift:215, 225).
    pub async fn open_file(&mut self, file_name: &str) -> anyhow::Result<(ListDocument, bool)> {
        self.open_file_at(file_name, None).await
    }

    /// Like open_file but downloads from `remote_path` (full path relative to DAV root)
    /// instead of constructing the URL from lists_remote_path. Cache key is still file_name.
    pub async fn open_file_at(
        &mut self,
        file_name: &str,
        remote_path: Option<&str>,
    ) -> anyhow::Result<(ListDocument, bool)> {
        let key = self.resolve_path(file_name, remote_path);

        // 1. memCache
        if let Some(cached) = self.mem_cache.get(&key) {
            return Ok((cached.clone(), true));
        }

        // 2. disk cache
        if let Some(doc) = self.load_from_disk(&key) {
            self.mem_cache.insert(key, doc.clone());
            return Ok((doc, true));
        }

        // 3. network
        let doc = self.download_at(file_name, remote_path, true).await?;
        Ok((doc, false))
    }

    // -----------------------------------------------------------------------
    // save_file — offline-first, two-stage merge
    // -----------------------------------------------------------------------

    /// `remote_path` is the full path from the DAV root (e.g. `/Lists/groceries.listie`).
    /// `file_name` is the bare filename used as the cache/disk key (e.g. `groceries.listie`).
    pub async fn save_file(&mut self, file_name: &str, remote_path: &str, doc: &ListDocument) -> anyhow::Result<()> {
        if self.credentials.is_none() {
            anyhow::bail!("not authenticated");
        }
        let full_url = self.creds().map(|c| c.dav_url_for(remote_path)).unwrap_or_default();
        info!("[save_file] start  file={file_name}  remote_path={remote_path}  url={full_url}");

        // Stage 1: merge with whatever is currently in memCache.
        let mut working = doc.clone();
        if let Some(cached) = self.mem_cache.get(remote_path).cloned() {
            info!("[save_file] stage1 merging with memCache: {file_name}");
            working = merge_documents(working, cached);
        }

        self.mem_cache.insert(remote_path.to_string(), working.clone());
        self.save_to_disk(remote_path, &working);
        self.pending.0.insert(remote_path.to_string());
        self.pending.save();
        info!("[save_file] written to disk cache and marked pending: {file_name}");

        // Stage 2: check whether the server has a version beyond what we have locally.
        let server_changed = match self.fetch_etag(remote_path).await {
            Ok(remote_etag) => {
                let cached_etag = self.etags.0.get(remote_path).cloned();
                let changed = cached_etag.as_deref() != Some(&remote_etag);
                info!("[save_file] stage2 etag check: cached={:?} remote={remote_etag} changed={changed}", cached_etag);
                changed
            }
            Err(e) if e.is::<FileGone>() => {
                // Server file was deleted; the upload below recreates it (Swift `saveFile`
                // swallows the 404 and uploads).
                info!("[save_file] stage2: server file gone — recreating on upload: {file_name}");
                false
            }
            Err(e) => {
                warn!("[save_file] stage2 etag fetch failed (offline?) — queued for retry: {file_name}: {e}");
                return Ok(());
            }
        };

        if server_changed {
            info!("[save_file] server changed — downloading for merge: {file_name}");
            match self.download_at(file_name, Some(remote_path), false).await {
                Ok(server_doc) => {
                    working = merge_documents(working, server_doc);
                    self.mem_cache.insert(remote_path.to_string(), working.clone());
                    self.save_to_disk(remote_path, &working);
                    info!("[save_file] merge complete: {file_name}");
                }
                Err(e) => {
                    // Match Swift NextcloudManager.saveFile (NextcloudManager.swift:282–297):
                    // on Stage-2 download failure, abort the upload and leave the file in
                    // `pending` for retry. Uploading the local-only version here would
                    // silently overwrite the server's newer changes (we already know the
                    // server has a version we haven't merged in).
                    warn!("[save_file] download for merge failed, upload aborted (queued for retry): {file_name}: {e}");
                    return Ok(());
                }
            }
        }

        info!("[save_file] uploading to {full_url}");
        match self.upload(remote_path, &working).await {
            Ok(()) => {
                if let Ok(etag) = self.fetch_etag(remote_path).await {
                    self.etags.0.insert(remote_path.to_string(), etag.clone());
                    self.etags.save();
                    info!("[save_file] upload OK, new etag={etag}: {file_name}");
                }
                self.pending.0.remove(remote_path);
                self.pending.save();
            }
            Err(e) => {
                warn!("[save_file] upload FAILED, queued for retry: {file_name}: {e}");
            }
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // sync_file — explicit bidirectional sync (called on foreground focus)
    // -----------------------------------------------------------------------

    /// Three-way sync:
    /// - both pending + server changed → download, 3-way merge, upload
    /// - pending only → upload
    /// - server changed only → download
    /// - neither → return cache
    pub async fn sync_file(
        &mut self,
        file_name: &str,
        remote_path: &str,
    ) -> anyhow::Result<ListDocument> {
        if self.credentials.is_none() {
            anyhow::bail!("not authenticated");
        }

        let remote_etag = match self.fetch_etag(remote_path).await {
            Ok(e) => e,
            // Confirmed deletion propagates so the caller can drop the list.
            Err(e) if e.is::<FileGone>() => return Err(e),
            Err(e) => {
                // Unreachable — return whatever we have.
                warn!("sync_file: cannot reach server for {file_name}: {e}");
                if let Some(cached) = self.mem_cache.get(remote_path).cloned() {
                    return Ok(cached);
                }
                return self.open_from_disk_cache(remote_path)
                    .ok_or_else(|| anyhow::anyhow!("server unreachable and no cache: {file_name}"));
            }
        };

        let cached_etag = self.etags.0.get(remote_path).cloned();
        let has_pending = self.pending.0.contains(remote_path);
        let server_changed = cached_etag.as_deref() != Some(&remote_etag);

        if has_pending && server_changed {
            // Conflict: 3-way merge.
            let local_doc = self.mem_cache.get(remote_path).cloned()
                .or_else(|| self.load_from_disk(remote_path));
            let server_doc = self.download_at(file_name, Some(remote_path), true).await?;
            let merged = if let Some(local) = local_doc {
                merge_documents(local, server_doc)
            } else {
                server_doc
            };
            self.upload(remote_path, &merged).await?;
            if let Ok(etag) = self.fetch_etag(remote_path).await {
                self.etags.0.insert(remote_path.to_string(), etag);
                self.etags.save();
            }
            self.pending.0.remove(remote_path);
            self.pending.save();
            self.mem_cache.insert(remote_path.to_string(), merged.clone());
            self.save_to_disk(remote_path, &merged);
            Ok(merged)

        } else if has_pending {
            // Upload local.
            let doc = self.mem_cache.get(remote_path).cloned()
                .or_else(|| self.load_from_disk(remote_path));
            if let Some(doc) = doc {
                self.upload(remote_path, &doc).await?;
                if let Ok(etag) = self.fetch_etag(remote_path).await {
                    self.etags.0.insert(remote_path.to_string(), etag);
                    self.etags.save();
                }
                self.pending.0.remove(remote_path);
                self.pending.save();
                Ok(doc)
            } else {
                self.pending.0.remove(remote_path);
                self.pending.save();
                self.open_file_at(file_name, Some(remote_path)).await.map(|(d, _)| d)
            }

        } else if server_changed {
            // Download server version.
            self.download_at(file_name, Some(remote_path), true).await

        } else {
            // No change — return cache.
            if let Some(cached) = self.mem_cache.get(remote_path).cloned() {
                Ok(cached)
            } else {
                self.open_file_at(file_name, Some(remote_path)).await.map(|(d, _)| d)
            }
        }
    }

    // -----------------------------------------------------------------------
    // background_sync — ETag check + download without updating etagStore
    // -----------------------------------------------------------------------

    /// Checks if the server version is newer and downloads it into the cache.
    /// Deliberately does NOT update `etagStore` so `save_file` Stage 2 still detects
    /// the divergence and merges on the next save.
    /// Returns `true` if the file was refreshed, `false` if unchanged or unreachable.
    /// Returns `Err([FileGone])` when the file is confirmed deleted on the server (404 with
    /// the account root reachable) so the caller can drop the list.
    pub async fn background_sync(&mut self, file_name: &str, remote_path: &str) -> anyhow::Result<bool> {
        let remote_etag = match self.fetch_etag(remote_path).await {
            Ok(e) => e,
            Err(e) if e.is::<FileGone>() => return Err(e), // confirmed deleted
            Err(_) => return Ok(false),                    // transient/unreachable — ignore
        };

        let cached_etag = self.etags.0.get(remote_path).cloned();
        if cached_etag.as_deref() == Some(&remote_etag) {
            return Ok(false); // unchanged
        }

        // Download WITHOUT updating etagStore (pass update_etag=false).
        let doc = self.download_at(file_name, Some(remote_path), false).await?;
        self.mem_cache.insert(remote_path.to_string(), doc.clone());
        self.save_to_disk(remote_path, &doc);
        info!("background refresh: new server version cached (etag held): {file_name}");
        Ok(true)
    }

    // -----------------------------------------------------------------------
    // retry_pending_uploads
    // -----------------------------------------------------------------------

    /// Retries all pending uploads. Call when network becomes available.
    pub async fn retry_pending_uploads(&mut self) -> anyhow::Result<()> {
        if self.pending.0.is_empty() || self.credentials.is_none() {
            return Ok(());
        }
        let pending: Vec<String> = self.pending.0.iter().cloned().collect();
        let mut succeeded = Vec::new();
        for remote_path in &pending {
            let doc = self.mem_cache.get(remote_path).cloned()
                .or_else(|| self.load_from_disk(remote_path));
            let Some(doc) = doc else {
                warn!("no cached doc for pending upload: {remote_path}");
                continue;
            };
            match self.upload(remote_path, &doc).await {
                Ok(()) => {
                    if let Ok(etag) = self.fetch_etag(remote_path).await {
                        self.etags.0.insert(remote_path.to_string(), etag);
                    }
                    succeeded.push(remote_path.clone());
                    info!("retry upload succeeded: {remote_path}");
                }
                Err(e) => warn!("retry upload failed: {remote_path}: {e}"),
            }
        }
        if !succeeded.is_empty() {
            for p in &succeeded {
                self.pending.0.remove(p);
            }
            self.pending.save();
            self.etags.save();
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Directory listing
    // -----------------------------------------------------------------------

    /// Lists the configured `lists_remote_path` — returns only .listie entries (no dirs).
    /// Used by `refresh_list_index`.
    pub async fn list_remote_files(&self) -> anyhow::Result<Vec<(String, String)>> {
        let creds = self.creds()?;
        let url = creds.dav_base();
        let entries = self.propfind_at(&url, false).await?;
        Ok(entries
            .into_iter()
            .filter(|e| !e.is_directory && (e.name.ends_with(".listie") || e.name.ends_with(".json")))
            .map(|e| (e.name, e.etag))
            .collect())
    }

    /// Lists any remote path (for the file browser). Returns directories and .listie files.
    pub async fn list_files_at(&self, remote_path: &str) -> anyhow::Result<Vec<RemoteEntry>> {
        let creds = self.creds()?;
        let url = creds.dav_url_for(remote_path);
        let mut entries = self.propfind_at(&url, true).await?;
        // Filter: keep directories and listie/json files; hide dot-files.
        entries.retain(|e| {
            !e.name.starts_with('.') && (e.is_directory || e.name.ends_with(".listie") || e.name.ends_with(".json"))
        });
        // Sort: directories first, then alphabetically.
        entries.sort_by(|a, b| {
            b.is_directory.cmp(&a.is_directory)
                .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(entries)
    }

    /// Reachability check: lightweight Depth:0 PROPFIND on the DAV root.
    pub async fn is_server_reachable(&self) -> bool {
        let Ok(creds) = self.creds() else { return false };
        let url = creds.dav_root();
        let result = self.client
            .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), &url)
            .basic_auth(&creds.username, Some(&creds.app_password))
            .header("Depth", "0")
            .header("Content-Type", "application/xml")
            .body(r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:"><d:prop><d:getetag/></d:prop></d:propfind>"#)
            .timeout(Duration::from_secs(5))
            .send()
            .await;
        match result {
            Ok(r) => r.status().is_success() || r.status().as_u16() == 207,
            Err(_) => false,
        }
    }

    pub fn mark_pending(&mut self, remote_path: &str) {
        self.pending.0.insert(remote_path.to_string());
        self.pending.save();
    }

    pub fn has_pending_upload(&self, remote_path: &str) -> bool {
        self.pending.0.contains(remote_path)
    }

    // -----------------------------------------------------------------------
    // HTTP primitives
    // -----------------------------------------------------------------------

    async fn fetch_etag(&self, remote_path: &str) -> anyhow::Result<String> {
        let creds = self.creds()?;
        let url = creds.dav_url_for(remote_path);
        info!("[fetch_etag] PROPFIND {url}");
        let body = r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:"><d:prop><d:getetag/></d:prop></d:propfind>"#;

        let resp = self
            .client
            .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), &url)
            .basic_auth(&creds.username, Some(&creds.app_password))
            .header("Depth", "0")
            .header("Content-Type", "application/xml")
            .body(body)
            .send()
            .await?;

        let status = resp.status();
        info!("[fetch_etag] → HTTP {status} for {url}");
        if status.as_u16() == 404 {
            return Err(self.not_found_error(remote_path).await);
        }
        if !status.is_success() && status.as_u16() != 207 {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("PROPFIND failed: HTTP {status} — {body}");
        }

        let xml = resp.text().await?;
        parse_etag(&xml).ok_or_else(|| anyhow::anyhow!("no ETag in PROPFIND response"))
    }

    /// Download a file. Accepts an explicit full remote path (relative to the DAV root) or
    /// constructs one under `lists_remote_path` when `None`; that resolved path is the cache
    /// key. `update_etag`: set false for background refreshes so `save_file` Stage 2 still
    /// detects divergence on the next save.
    async fn download_at(
        &mut self,
        file_name: &str,
        remote_path: Option<&str>,
        update_etag: bool,
    ) -> anyhow::Result<ListDocument> {
        let key = self.resolve_path(file_name, remote_path);
        let creds = self.creds()?;
        let url = creds.dav_url_for(&key);

        let resp = self
            .client
            .get(&url)
            .basic_auth(&creds.username, Some(&creds.app_password))
            .send()
            .await?;

        if resp.status().as_u16() == 404 {
            return Err(self.not_found_error(&key).await);
        }

        let etag = resp
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim_matches('"').to_string());

        let bytes = resp.bytes().await?;
        let doc: ListDocument = serde_json::from_slice(&bytes)?;

        self.mem_cache.insert(key.clone(), doc.clone());
        self.save_to_disk(&key, &doc);

        if update_etag {
            if let Some(etag) = etag {
                self.etags.0.insert(key.clone(), etag);
                self.etags.save();
            } else {
                // Fallback: PROPFIND for ETag if not in response headers.
                if let Ok(etag) = self.fetch_etag(&key).await {
                    self.etags.0.insert(key.clone(), etag);
                    self.etags.save();
                }
            }
        }

        Ok(doc)
    }

    async fn upload(&self, remote_path: &str, doc: &ListDocument) -> anyhow::Result<()> {
        let creds = self.creds()?;
        let url = creds.dav_url_for(remote_path);
        info!("[upload] PUT {url}");
        let body = serde_json::to_vec_pretty(doc)?;
        let resp = self.client
            .put(&url)
            .basic_auth(&creds.username, Some(&creds.app_password))
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("upload failed: HTTP {status} — {body}");
        }
        info!("[upload] PUT {url} → {status}");
        Ok(())
    }

    /// Permanently delete a list file on Nextcloud (WebDAV DELETE) and wipe local
    /// caches/state for it. Mirrors Swift `UnifiedListProvider.deleteList` for the
    /// `.nextcloud` branch (`UnifiedListProvider.swift:961-974`): the server file is
    /// removed, then any cached document, ETag, pending upload, and on-disk copy
    /// are cleaned up. 404 from the server is treated as success (already gone).
    pub async fn delete_remote(&mut self, remote_path: &str) -> anyhow::Result<()> {
        let creds = self.creds()?;
        let url = creds.dav_url_for(remote_path);
        info!("[delete_remote] DELETE {url}");
        let resp = self.client
            .delete(&url)
            .basic_auth(&creds.username, Some(&creds.app_password))
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() && status.as_u16() != 404 {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("delete failed: HTTP {status} — {body}");
        }
        info!("[delete_remote] DELETE {url} → {status}");

        self.etags.0.remove(remote_path);
        self.etags.save();
        self.mem_cache.remove(remote_path);
        self.pending.0.remove(remote_path);
        self.pending.save();
        let _ = std::fs::remove_file(self.disk_cache_path(remote_path));
        Ok(())
    }

    /// PROPFIND a URL and return parsed entries.
    /// `include_root`: if false, drops the first entry (the collection itself, returned by Depth:1).
    async fn propfind_at(&self, url: &str, include_root: bool) -> anyhow::Result<Vec<RemoteEntry>> {
        let creds = self.creds()?;
        let body = r#"<?xml version="1.0"?>
<d:propfind xmlns:d="DAV:">
  <d:prop><d:getetag/><d:getcontenttype/><d:displayname/><d:resourcetype/></d:prop>
</d:propfind>"#;

        let resp = self
            .client
            .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), url)
            .basic_auth(&creds.username, Some(&creds.app_password))
            .header("Depth", "1")
            .header("Content-Type", "application/xml")
            .body(body)
            .send()
            .await?;

        let xml = resp.text().await?;
        let mut entries = parse_propfind_entries(&xml);
        if !include_root && !entries.is_empty() {
            entries.remove(0); // first entry is always the folder itself
        }
        Ok(entries)
    }
}

// ---------------------------------------------------------------------------
// XML helpers
// ---------------------------------------------------------------------------

fn parse_etag(xml: &str) -> Option<String> {
    let start = xml.find("<d:getetag>")? + "<d:getetag>".len();
    let end = xml[start..].find("</d:getetag>")? + start;
    Some(xml[start..end].trim_matches('"').to_string())
}

fn parse_propfind_entries(xml: &str) -> Vec<RemoteEntry> {
    let mut results = Vec::new();
    let mut remaining = xml;

    while let Some(response_start) = remaining.find("<d:response>") {
        let after_tag = &remaining[response_start..];
        let Some(response_end) = after_tag.find("</d:response>") else { break };
        let block = &after_tag[..response_end + "</d:response>".len()];
        remaining = &after_tag[response_end + "</d:response>".len()..];

        // Extract href.
        let Some(href_start) = block.find("<d:href>") else { continue };
        let href_start = href_start + "<d:href>".len();
        let Some(href_end) = block[href_start..].find("</d:href>") else { continue };
        let href = block[href_start..href_start + href_end].trim();
        tracing::info!("[propfind] raw href from server: {href}");

        // Filename is the last path component, URL-decoded.
        let name = href
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(href);
        let name = percent_decode(name);

        // Is it a directory?
        let is_directory = block.contains("<d:collection/>");

        // ETag.
        let etag = if let Some(et_start) = block.find("<d:getetag>") {
            let et_start = et_start + "<d:getetag>".len();
            if let Some(et_end) = block[et_start..].find("</d:getetag>") {
                block[et_start..et_start + et_end].trim_matches('"').to_string()
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        results.push(RemoteEntry { name, etag, is_directory });
    }

    results
}

/// Minimal percent-decoding for common URL-encoded characters in filenames.
fn percent_decode(s: &str) -> String {
    // Use a simple approach: just handle the most common cases.
    // For a full implementation we'd use the `percent-encoding` crate.
    s.replace("%20", " ")
        .replace("%21", "!")
        .replace("%23", "#")
        .replace("%24", "$")
        .replace("%25", "%")
        .replace("%26", "&")
        .replace("%27", "'")
        .replace("%28", "(")
        .replace("%29", ")")
        .replace("%2B", "+")
        .replace("%2C", ",")
        .replace("%3D", "=")
        .replace("%40", "@")
        .replace("%5B", "[")
        .replace("%5D", "]")
}
