use super::{
    ConnectionEngine, ConnectionError, ResourceItem, ResourceKind, ResourcePage, ResourceRequest,
    Result, SessionRequest,
};
use http::Request;
use http_body_util::BodyExt;
use kube::{Client, client::Body};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

const EVENT_LIMIT: usize = 256 * 1024;
const LIST_LIMIT: usize = 2 * 1024 * 1024;
const INVENTORY_LIMIT: usize = 100;
const SYNC_LIMIT: usize = 4;
const RELIST_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PodSyncRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub sync_id: String,
}
impl PodSyncRequest {
    fn session(&self) -> SessionRequest {
        SessionRequest {
            cluster_id: self.cluster_id.clone(),
            session_id: self.session_id.clone(),
        }
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodSyncSnapshot {
    pub cluster_id: String,
    pub session_id: String,
    pub sync_id: String,
    pub sequence: u64,
    pub page: Option<ResourcePage>,
    pub status: &'static str,
    pub error: Option<ConnectionError>,
}
struct SyncState {
    request: ResourceRequest,
    id: String,
    cancel: CancellationToken,
    sequence: u64,
    page: Option<ResourcePage>,
    status: &'static str,
    error: Option<ConnectionError>,
}
impl SyncState {
    fn snapshot(&self) -> PodSyncSnapshot {
        PodSyncSnapshot {
            cluster_id: self.request.cluster_id.clone(),
            session_id: self.request.session_id.clone(),
            sync_id: self.id.clone(),
            sequence: self.sequence,
            page: self.page.clone(),
            status: self.status,
            error: self.error,
        }
    }
    fn transition(&mut self, status: &'static str, error: Option<ConnectionError>) {
        self.sequence = self.sequence.saturating_add(1);
        self.status = status;
        self.error = error;
    }
}
#[derive(Default)]
pub(super) struct PodSyncRuntime {
    registry: Mutex<HashMap<String, Arc<Mutex<SyncState>>>>,
    tasks: TaskTracker,
}
impl PodSyncRuntime {
    pub(super) fn cancel_all(&self) {
        if let Ok(registry) = self.registry.lock() {
            for slot in registry.values() {
                if let Ok(mut state) = slot.lock() {
                    state.cancel.cancel();
                    state.page = None;
                    state.transition("stopped", None);
                }
            }
        }
    }
    pub(super) async fn drain(&self) {
        self.tasks.close();
        self.cancel_all();
        self.tasks.wait().await;
        if let Ok(mut registry) = self.registry.lock() {
            registry.clear();
        }
    }
    fn lookup(&self, request: &PodSyncRequest) -> Result<Arc<Mutex<SyncState>>> {
        super::validate_id(&request.sync_id)?;
        let registry = self
            .registry
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        let slot = registry
            .get(&request.sync_id)
            .ok_or(ConnectionError::NotFound)?
            .clone();
        {
            let state = slot.lock().map_err(|_| ConnectionError::StoreUnavailable)?;
            if state.request.cluster_id != request.cluster_id
                || state.request.session_id != request.session_id
            {
                return Err(ConnectionError::StaleSession);
            }
        }
        Ok(slot)
    }
    fn start(
        &self,
        client: Client,
        request: ResourceRequest,
        cancel: CancellationToken,
    ) -> Result<PodSyncSnapshot> {
        validate(&request)?;
        let mut registry = self
            .registry
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        if self.tasks.is_closed() || cancel.is_cancelled() {
            return Err(ConnectionError::StaleSession);
        }
        registry.retain(|_, slot| slot.lock().is_ok_and(|state| !state.cancel.is_cancelled()));
        // Include retiring tasks in admission so close/open churn cannot accumulate workers.
        if registry.len() >= SYNC_LIMIT || self.tasks.len() >= SYNC_LIMIT {
            return Err(ConnectionError::Busy);
        }
        let mut random = [0; 16];
        getrandom::fill(&mut random).map_err(|_| ConnectionError::ConnectionFailed)?;
        let id: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let slot = Arc::new(Mutex::new(SyncState {
            request,
            id: id.clone(),
            cancel,
            sequence: 0,
            page: None,
            status: "opening",
            error: None,
        }));
        let snapshot = slot
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?
            .snapshot();
        registry.insert(id, slot.clone());
        // GPUI invokes synchronous actions on a non-Tokio thread. Use the
        // application runtime explicitly rather than relying on a thread-local
        // runtime context (which would panic on the Enable live sync action).
        self.tasks
            .spawn_on(run(client, slot), &crate::runtime::handle());
        Ok(snapshot)
    }
}
impl Drop for PodSyncRuntime {
    fn drop(&mut self) {
        self.cancel_all();
    }
}
impl ConnectionEngine {
    pub async fn pod_sync_open(&self, request: ResourceRequest) -> Result<PodSyncSnapshot> {
        validate(&request)?;
        let scope = request.session();
        use super::permissions::{PermissionAction, PermissionCheck};
        let authorization = self
            .require_permissions(
                &scope,
                &[
                    PermissionCheck {
                        kind: ResourceKind::Pods,
                        action: PermissionAction::List,
                        namespace: request.namespace.clone(),
                        name: None,
                    },
                    PermissionCheck {
                        kind: ResourceKind::Pods,
                        action: PermissionAction::Watch,
                        namespace: request.namespace.clone(),
                        name: None,
                    },
                ],
            )
            .await?;
        let (client, cancel) = self.pod_client(&scope)?;
        let client = super::permissions::authorized_client(client, authorization);
        let snapshot = self.pod_sync.start(client, request, cancel)?;
        self.check_pod_session(&scope)?;
        Ok(snapshot)
    }
    pub fn pod_sync_read(&self, request: PodSyncRequest) -> Result<PodSyncSnapshot> {
        let scope = request.session();
        self.check_pod_session(&scope)?;
        let slot = self.pod_sync.lookup(&request)?;
        let snapshot = {
            let state = slot.lock().map_err(|_| ConnectionError::StoreUnavailable)?;
            if state.cancel.is_cancelled() && state.status != "stopped" {
                return Err(ConnectionError::StaleSession);
            }
            state.snapshot()
        };
        self.check_pod_session(&scope)?;
        Ok(snapshot)
    }
    pub fn pod_sync_close(&self, request: PodSyncRequest) -> Result<()> {
        self.check_pod_lease(&request.session())?;
        let slot = match self.pod_sync.lookup(&request) {
            Err(ConnectionError::NotFound) => return Ok(()),
            other => other?,
        };
        let mut state = slot.lock().map_err(|_| ConnectionError::StoreUnavailable)?;
        state.cancel.cancel();
        state.page = None;
        state.transition("stopped", None);
        Ok(())
    }
}
fn validate(request: &ResourceRequest) -> Result<()> {
    super::validate_read_id(request.read_id)?;
    super::transport::validate_resource_request(request)?;
    if request.kind != ResourceKind::Pods
        || request.continue_token.is_some()
        || request.query.is_some()
        || request.status.is_some()
        || request.group.is_some()
    {
        return Err(ConnectionError::InvalidInput);
    }
    Ok(())
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn version(value: &Value) -> Result<String> {
    let text = value
        .pointer("/metadata/resourceVersion")
        .and_then(Value::as_str)
        .ok_or(ConnectionError::InvalidResponse)?;
    if text.is_empty()
        || text.len() > 256
        || text.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(ConnectionError::InvalidResponse);
    }
    Ok(text.to_owned())
}
type Key = (String, String);
struct Inventory {
    rows: BTreeMap<Key, ResourceItem>,
    version: String,
    truncated: bool,
}
fn projected(value: Value, namespace: Option<&str>) -> Result<ResourceItem> {
    if value
        .get("kind")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind != "Pod")
    {
        return Err(ConnectionError::InvalidResponse);
    }
    let item = super::resources::project(ResourceKind::Pods, value)?;
    if namespace.is_some_and(|ns| item.namespace.as_deref() != Some(ns)) {
        return Err(ConnectionError::InvalidResponse);
    }
    Ok(item)
}
fn key(item: &ResourceItem) -> Key {
    (
        item.namespace.clone().unwrap_or_default(),
        item.name.clone(),
    )
}
fn parse_list(value: Value, namespace: Option<&str>) -> Result<Inventory> {
    if value.get("kind").and_then(Value::as_str) != Some("PodList") {
        return Err(ConnectionError::InvalidResponse);
    }
    let version = version(&value)?;
    let continuation = value.pointer("/metadata/continue");
    if continuation.is_some_and(|v| !v.is_string() && !v.is_null()) {
        return Err(ConnectionError::InvalidResponse);
    }
    let truncated = continuation
        .and_then(Value::as_str)
        .is_some_and(|v| !v.is_empty());
    let items = value
        .get("items")
        .and_then(Value::as_array)
        .ok_or(ConnectionError::InvalidResponse)?;
    if items.len() > INVENTORY_LIMIT {
        return Err(ConnectionError::LimitExceeded);
    }
    let mut rows = BTreeMap::new();
    for value in items {
        let item = projected(value.clone(), namespace)?;
        if rows.insert(key(&item), item).is_some() {
            return Err(ConnectionError::InvalidResponse);
        }
    }
    Ok(Inventory {
        rows,
        version,
        truncated,
    })
}
#[derive(Deserialize)]
#[serde(tag = "type", content = "object")]
enum WatchEvent {
    #[serde(rename = "ADDED")]
    Added(Value),
    #[serde(rename = "MODIFIED")]
    Modified(Value),
    #[serde(rename = "DELETED")]
    Deleted(Value),
    #[serde(rename = "BOOKMARK")]
    Bookmark(Value),
    #[serde(rename = "ERROR")]
    Error(Value),
}
#[derive(Debug, PartialEq, Eq)]
enum WatchStep {
    Continue,
    Relist,
}
impl Inventory {
    fn apply(&mut self, event: WatchEvent, namespace: Option<&str>) -> Result<WatchStep> {
        let (value, deleted) = match event {
            WatchEvent::Error(value) => {
                return match value.get("code").and_then(Value::as_u64) {
                    Some(410) => Ok(WatchStep::Relist),
                    Some(code) => Err(status_error(u16::try_from(code).unwrap_or(500))),
                    None => Err(ConnectionError::InvalidResponse),
                };
            }
            WatchEvent::Bookmark(value) => {
                self.version = version(&value)?;
                return Ok(WatchStep::Continue);
            }
            WatchEvent::Added(value) | WatchEvent::Modified(value) => (value, false),
            WatchEvent::Deleted(value) => (value, true),
        };
        let version = version(&value)?;
        let item = projected(value, namespace)?;
        let key = key(&item);
        if deleted {
            self.rows.remove(&key);
            self.version = version;
            return Ok(if self.truncated {
                WatchStep::Relist
            } else {
                WatchStep::Continue
            });
        }
        // A partial page is not a complete inventory: additions require a fresh bounded LIST.
        if !self.rows.contains_key(&key) && (self.truncated || self.rows.len() == INVENTORY_LIMIT) {
            self.truncated = true;
            return Ok(WatchStep::Relist);
        }
        self.rows.insert(key, item);
        self.version = version;
        Ok(WatchStep::Continue)
    }
    fn page(&self, request: &ResourceRequest) -> ResourcePage {
        ResourcePage {
            cluster_id: request.cluster_id.clone(),
            session_id: request.session_id.clone(),
            kind: ResourceKind::Pods,
            read_id: request.read_id,
            items: self.rows.values().cloned().collect(),
            truncated: self.truncated,
            fetched_at: now(),
            namespace: request.namespace.clone(),
            continue_token: None,
            metrics: None,
        }
    }
}
#[derive(Default)]
struct Lines {
    pending: Vec<u8>,
}
impl Lines {
    fn push(
        &mut self,
        bytes: &[u8],
        mut event: impl FnMut(WatchEvent) -> Result<WatchStep>,
    ) -> Result<WatchStep> {
        // Process frame segments in place; never accumulate an unbounded frame in the line buffer.
        for segment in bytes.split_inclusive(|b| *b == b'\n') {
            let complete = segment.last() == Some(&b'\n');
            let segment = if complete {
                &segment[..segment.len() - 1]
            } else {
                segment
            };
            if segment.len() > EVENT_LIMIT.saturating_sub(self.pending.len()) {
                return Err(ConnectionError::LimitExceeded);
            }
            self.pending.extend_from_slice(segment);
            if complete {
                if !self.pending.iter().all(u8::is_ascii_whitespace) {
                    let value = serde_json::from_slice(&self.pending)
                        .map_err(|_| ConnectionError::InvalidResponse)?;
                    if event(value)? == WatchStep::Relist {
                        self.pending.clear();
                        return Ok(WatchStep::Relist);
                    }
                }
                self.pending.clear();
            }
        }
        Ok(WatchStep::Continue)
    }
    fn finish(&self) -> Result<()> {
        if self.pending.iter().all(u8::is_ascii_whitespace) {
            Ok(())
        } else {
            Err(ConnectionError::InvalidResponse)
        }
    }
}
fn status_error(code: u16) -> ConnectionError {
    match code {
        401 => ConnectionError::AuthFailed,
        403 => ConnectionError::PermissionDenied,
        404 => ConnectionError::ResourceUnavailable,
        408 | 504 => ConnectionError::Timeout,
        429 => ConnectionError::Busy,
        _ => ConnectionError::ConnectionFailed,
    }
}
fn permanent(error: ConnectionError) -> bool {
    matches!(
        error,
        ConnectionError::AuthFailed
            | ConnectionError::AuthExpired
            | ConnectionError::PermissionDenied
            | ConnectionError::ResourceUnavailable
            | ConnectionError::InvalidResponse
            | ConnectionError::LimitExceeded
            | ConnectionError::TlsFailed
    )
}
async fn response(
    client: &Client,
    path: &str,
    cancel: &CancellationToken,
) -> Result<http::Response<Body>> {
    super::validate_credentials(client)?;
    let mut request = Request::get(path)
        .header("Accept", "application/json")
        .body(Body::empty())
        .map_err(|_| ConnectionError::InvalidInput)?;
    request.extensions_mut().insert(cancel.clone());
    client
        .send(request)
        .await
        .map_err(|_| ConnectionError::Network)
}
async fn list(
    client: &Client,
    request: &ResourceRequest,
    cancel: &CancellationToken,
) -> Result<Inventory> {
    let mut response = response(client, &super::transport::resource_path(request), cancel).await?;
    if response.status().as_u16() != 200 {
        return Err(status_error(response.status().as_u16()));
    }
    if response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .is_some_and(|v| v > LIST_LIMIT)
    {
        return Err(ConnectionError::LimitExceeded);
    }
    let mut bytes = Vec::new();
    while let Some(frame) = response.body_mut().frame().await {
        if let Ok(data) = frame.map_err(|_| ConnectionError::Network)?.into_data() {
            if data.len() > LIST_LIMIT.saturating_sub(bytes.len()) {
                return Err(ConnectionError::LimitExceeded);
            }
            bytes.extend_from_slice(&data);
        }
    }
    parse_list(
        serde_json::from_slice(&bytes).map_err(|_| ConnectionError::InvalidResponse)?,
        request.namespace.as_deref(),
    )
}
fn publish(slot: &Mutex<SyncState>, inventory: &Inventory) -> Result<()> {
    let mut state = slot.lock().map_err(|_| ConnectionError::StoreUnavailable)?;
    if state.cancel.is_cancelled() {
        return Err(ConnectionError::StaleSession);
    }
    state.page = Some(inventory.page(&state.request));
    state.transition("syncing", None);
    Ok(())
}
async fn watch(
    client: &Client,
    request: &ResourceRequest,
    cancel: &CancellationToken,
    inventory: &mut Inventory,
    slot: &Mutex<SyncState>,
) -> Result<WatchStep> {
    let base = super::transport::resource_path(request);
    let base = base
        .split('?')
        .next()
        .ok_or(ConnectionError::InvalidInput)?;
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("watch", "true")
        .append_pair("allowWatchBookmarks", "true")
        .append_pair("timeoutSeconds", "10")
        .append_pair("resourceVersion", &inventory.version)
        .finish();
    let mut response = response(client, &format!("{base}?{query}"), cancel).await?;
    match response.status().as_u16() {
        200 => {}
        410 => return Ok(WatchStep::Relist),
        code => return Err(status_error(code)),
    }
    publish(slot, inventory)?;
    let mut lines = Lines::default();
    while let Some(frame) = response.body_mut().frame().await {
        if let Ok(data) = frame.map_err(|_| ConnectionError::Network)?.into_data() {
            let step = lines.push(&data, |event| {
                let step = inventory.apply(event, request.namespace.as_deref())?;
                publish(slot, inventory)?;
                Ok(step)
            })?;
            if step == WatchStep::Relist {
                return Ok(step);
            }
        }
    }
    lines.finish()?;
    Ok(WatchStep::Continue)
}
async fn bounded<T>(
    cancel: &CancellationToken,
    future: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(ConnectionError::StaleSession),
        result = tokio::time::timeout(super::DEADLINE, future) => result.unwrap_or(Err(ConnectionError::Timeout)),
    }
}
async fn run(client: Client, slot: Arc<Mutex<SyncState>>) {
    let (request, cancel) = match slot.lock() {
        Ok(state) => (state.request.clone(), state.cancel.clone()),
        Err(_) => return,
    };
    let mut inventory = None;
    let mut listed_at = tokio::time::Instant::now();
    let mut delay = 1;
    loop {
        let result = async {
            if inventory.is_none() || listed_at.elapsed() >= RELIST_INTERVAL {
                inventory = Some(bounded(&cancel, list(&client, &request, &cancel)).await?);
                listed_at = tokio::time::Instant::now();
                publish(
                    &slot,
                    inventory.as_ref().ok_or(ConnectionError::InvalidResponse)?,
                )?;
            }
            let current = inventory.as_mut().ok_or(ConnectionError::InvalidResponse)?;
            bounded(&cancel, watch(&client, &request, &cancel, current, &slot)).await
        }
        .await;
        let wait = match result {
            Ok(step) => {
                if step == WatchStep::Relist {
                    inventory = None;
                }
                delay = 1;
                1
            }
            Err(ConnectionError::StaleSession) if cancel.is_cancelled() => break,
            Err(error) => {
                if let Ok(mut state) = slot.lock() {
                    state.transition(
                        if permanent(error) {
                            "error"
                        } else {
                            "retrying"
                        },
                        Some(error),
                    );
                }
                if permanent(error) {
                    break;
                }
                let wait = delay;
                delay = (delay * 2).min(15);
                wait
            }
        };
        tokio::select! { biased; _ = cancel.cancelled() => break, _ = tokio::time::sleep(Duration::from_secs(wait)) => {} }
    }
    if cancel.is_cancelled()
        && let Ok(mut state) = slot.lock()
    {
        state.page = None;
        state.transition("stopped", None);
    }
}

#[cfg(test)]
#[path = "pod_sync_tests.rs"]
mod tests;
