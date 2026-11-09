use super::{ConnectionEngine, ConnectionError, Result, SessionRequest};
use super::{
    ResourceKind,
    permissions::{PermissionAction, PermissionCheck},
};
use futures::{AsyncReadExt as FuturesReadExt, SinkExt};
use k8s_openapi::api::core::v1::Pod;
use kube::{
    Api,
    api::{AttachParams, AttachedProcess, LogParams, TerminalSize},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Semaphore, mpsc},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

const RING_LIMIT: usize = 256 * 1024;
const DRAIN_LIMIT: usize = 64 * 1024;
const INPUT_LIMIT: usize = 16 * 1024;
const STREAM_LIMIT: usize = 12;
const FORWARD_LIMIT: usize = 8;
const CONNECTION_LIMIT: usize = 8;
const CONNECTION_LIFETIME: Duration = Duration::from_secs(60 * 60);

async fn operation<T>(
    cancel: &CancellationToken,
    future: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(ConnectionError::StaleSession),
        result = tokio::time::timeout(super::DEADLINE, future) => result.unwrap_or(Err(ConnectionError::Timeout)),
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PodStreamMode {
    Logs,
    Shell,
    Attach,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PodStartRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub namespace: String,
    pub name: String,
    pub uid: String,
    pub container: String,
    pub mode: PodStreamMode,
    #[serde(default)]
    pub previous: bool,
    #[serde(default = "default_timestamps")]
    pub timestamps: bool,
    #[serde(default = "default_tail")]
    pub tail_lines: u32,
    pub shell: Option<String>,
}
fn default_timestamps() -> bool {
    true
}
fn default_tail() -> u32 {
    500
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PodStreamRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub stream_id: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PodInputRequest {
    #[serde(flatten)]
    pub stream: PodStreamRequest,
    pub data: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PodResizeRequest {
    #[serde(flatten)]
    pub stream: PodStreamRequest,
    pub cols: u16,
    pub rows: u16,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodStreamSnapshot {
    pub cluster_id: String,
    pub session_id: String,
    pub stream_id: String,
    pub sequence: u64,
    pub output: String,
    pub status: &'static str,
    pub error: Option<ConnectionError>,
    pub truncated: bool,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PodForwardRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub namespace: String,
    pub name: String,
    pub uid: String,
    pub remote_port: u16,
    #[serde(default)]
    pub local_port: u16,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodForward {
    pub id: String,
    pub cluster_id: String,
    pub session_id: String,
    pub namespace: String,
    pub name: String,
    pub remote_port: u16,
    pub local_port: u16,
    pub status: &'static str,
    pub error: Option<ConnectionError>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PodForwardStopRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub id: String,
}

#[derive(Default)]
struct Ring {
    text: String,
    truncated: bool,
    sequence: u64,
}
impl Ring {
    fn push(&mut self, value: &str) {
        if value.is_empty() {
            return;
        }
        self.sequence = self.sequence.saturating_add(1);
        self.text.push_str(value);
        if self.text.len() > RING_LIMIT {
            let mut drop = self.text.len() - RING_LIMIT;
            while !self.text.is_char_boundary(drop) {
                drop += 1;
            }
            self.text.drain(..drop);
            self.truncated = true;
        }
    }
    fn drain(&mut self) -> (String, bool) {
        let mut end = self.text.len().min(DRAIN_LIMIT);
        while !self.text.is_char_boundary(end) {
            end -= 1;
        }
        (
            self.text.drain(..end).collect(),
            std::mem::take(&mut self.truncated),
        )
    }
    fn clear(&mut self) {
        self.text.clear();
        self.truncated = false;
    }
}
// Keep incomplete codepoints across network reads; invalid bytes are replaced only once complete.
#[derive(Default)]
struct Decoder {
    pending: Vec<u8>,
}
impl Decoder {
    fn decode(&mut self, bytes: &[u8], eof: bool) -> String {
        self.pending.extend_from_slice(bytes);
        let mut output = String::new();
        let mut offset = 0;
        while offset < self.pending.len() {
            match std::str::from_utf8(&self.pending[offset..]) {
                Ok(text) => {
                    output.push_str(text);
                    offset = self.pending.len();
                }
                Err(error) => {
                    let valid = offset + error.valid_up_to();
                    output.push_str(
                        std::str::from_utf8(&self.pending[offset..valid])
                            .expect("valid UTF-8 prefix"),
                    );
                    match error.error_len() {
                        Some(len) => {
                            output.push('\u{fffd}');
                            offset = valid + len;
                        }
                        None if eof => {
                            output.push('\u{fffd}');
                            offset = self.pending.len();
                        }
                        None => {
                            offset = valid;
                            break;
                        }
                    }
                }
            }
        }
        self.pending.drain(..offset);
        output
    }
}
struct StreamState {
    scope: SessionRequest,
    id: String,
    cancel: CancellationToken,
    ring: Ring,
    status: &'static str,
    error: Option<ConnectionError>,
    input: Option<mpsc::Sender<String>>,
    resize: Option<mpsc::Sender<TerminalSize>>,
    finished: bool,
}
impl StreamState {
    fn snapshot(&mut self) -> PodStreamSnapshot {
        let (output, truncated) = self.ring.drain();
        PodStreamSnapshot {
            cluster_id: self.scope.cluster_id.clone(),
            session_id: self.scope.session_id.clone(),
            stream_id: self.id.clone(),
            sequence: self.ring.sequence,
            output,
            status: self.status,
            error: self.error,
            truncated,
        }
    }
    fn finish(&mut self, result: Result<()>) {
        self.finished = true;
        self.input = None;
        self.resize = None;
        if self.cancel.is_cancelled() {
            self.ring.clear();
            self.status = "exited";
            self.error = None;
        } else {
            self.status = if result.is_ok() { "exited" } else { "error" };
            self.error = result.err();
        }
        self.ring.sequence = self.ring.sequence.saturating_add(1);
    }
}
struct ForwardState {
    view: PodForward,
    cancel: CancellationToken,
    finished: bool,
}
#[derive(Default)]
struct Registry {
    streams: HashMap<String, Arc<Mutex<StreamState>>>,
    forwards: HashMap<String, Arc<Mutex<ForwardState>>>,
}
pub(super) struct PodRuntime {
    registry: Mutex<Registry>,
    tasks: TaskTracker,
    stream_slots: Arc<Semaphore>,
    forward_slots: Arc<Semaphore>,
}
impl Default for PodRuntime {
    fn default() -> Self {
        Self {
            registry: Mutex::new(Registry::default()),
            tasks: TaskTracker::new(),
            stream_slots: Arc::new(Semaphore::new(STREAM_LIMIT)),
            forward_slots: Arc::new(Semaphore::new(FORWARD_LIMIT)),
        }
    }
}
impl PodRuntime {
    pub(super) fn has_active_work(&self) -> bool {
        self.stream_slots.available_permits() != STREAM_LIMIT
            || self.forward_slots.available_permits() != FORWARD_LIMIT
    }
    pub(super) async fn drain(&self) {
        self.cancel_all();
        self.tasks.close();
        self.tasks.wait().await;
        if let Ok(mut registry) = self.registry.lock() {
            registry.streams.clear();
            registry.forwards.clear();
        }
    }
    pub(super) fn cancel_all(&self) {
        if let Ok(registry) = self.registry.lock() {
            for slot in registry.streams.values() {
                if let Ok(mut slot) = slot.lock() {
                    slot.cancel.cancel();
                    slot.ring.clear();
                    slot.input = None;
                    slot.resize = None;
                }
            }
            for slot in registry.forwards.values() {
                if let Ok(slot) = slot.lock() {
                    slot.cancel.cancel();
                }
            }
        }
    }
    fn stream(&self, request: &PodStreamRequest) -> Result<Arc<Mutex<StreamState>>> {
        super::validate_id(&request.stream_id)?;
        let registry = self
            .registry
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        let slot = registry
            .streams
            .get(&request.stream_id)
            .ok_or(ConnectionError::NotFound)?
            .clone();
        {
            let state = slot.lock().map_err(|_| ConnectionError::StoreUnavailable)?;
            if !same_scope(&state.scope, &stream_scope(request)) || state.cancel.is_cancelled() {
                return Err(ConnectionError::StaleSession);
            }
        }
        Ok(slot)
    }
}
impl Drop for PodRuntime {
    fn drop(&mut self) {
        self.cancel_all();
    }
}
fn same_scope(left: &SessionRequest, right: &SessionRequest) -> bool {
    left.cluster_id == right.cluster_id && left.session_id == right.session_id
}
fn stream_scope(request: &PodStreamRequest) -> SessionRequest {
    SessionRequest {
        cluster_id: request.cluster_id.clone(),
        session_id: request.session_id.clone(),
    }
}
fn random_id() -> Result<String> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|_| ConnectionError::ConnectionFailed)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn dns_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value.as_bytes()[value.len() - 1].is_ascii_alphanumeric()
}
fn validate_target(namespace: &str, name: &str, uid: &str) -> Result<()> {
    if !dns_label(namespace)
        || name.len() > 253
        || !name.split('.').all(dns_label)
        || uid.is_empty()
        || uid.len() > 128
        || uid.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(ConnectionError::InvalidInput);
    }
    Ok(())
}
fn shell_command(shell: Option<&str>) -> Result<&'static str> {
    match shell.unwrap_or("/bin/sh") {
        "sh" | "/bin/sh" => Ok("/bin/sh"),
        "bash" | "/bin/bash" => Ok("/bin/bash"),
        "ash" | "/bin/ash" => Ok("/bin/ash"),
        "powershell" => Ok("powershell"),
        "cmd" => Ok("cmd"),
        _ => Err(ConnectionError::InvalidInput),
    }
}
fn map_error(error: kube::Error) -> ConnectionError {
    match error {
        kube::Error::Api(response) => match response.code {
            401 => ConnectionError::AuthFailed,
            403 => ConnectionError::PermissionDenied,
            404 => ConnectionError::NotFound,
            409 => ConnectionError::SourceChanged,
            429 => ConnectionError::Busy,
            _ => ConnectionError::ResourceUnavailable,
        },
        _ => ConnectionError::Network,
    }
}
async fn verify_pod(
    api: &Api<Pod>,
    name: &str,
    uid: &str,
    container: Option<&str>,
    cancel: &CancellationToken,
) -> Result<()> {
    let pod = fetch_pod(api, name, cancel).await?;
    if pod.metadata.uid.as_deref() != Some(uid) {
        return Err(ConnectionError::SourceChanged);
    }
    if let Some(container) = container {
        let spec = pod.spec.ok_or(ConnectionError::InvalidResponse)?;
        let exists = spec.containers.iter().any(|c| c.name == container)
            || spec
                .init_containers
                .as_ref()
                .is_some_and(|cs| cs.iter().any(|c| c.name == container))
            || spec
                .ephemeral_containers
                .as_ref()
                .is_some_and(|cs| cs.iter().any(|c| c.name == container));
        if !exists {
            return Err(ConnectionError::NotFound);
        }
    }
    Ok(())
}

async fn fetch_pod(api: &Api<Pod>, name: &str, cancel: &CancellationToken) -> Result<Pod> {
    let value = super::pods::send_json(
        &api.clone().into_client(),
        cancel,
        http::Method::GET,
        &format!("{}/{name}", api.resource_url()),
        None,
        "application/json",
    )
    .await?;
    serde_json::from_value(value).map_err(|_| ConnectionError::InvalidResponse)
}

async fn verify_process(
    api: &Api<Pod>,
    request: &PodStartRequest,
    cancel: &CancellationToken,
) -> Result<()> {
    let pod = fetch_pod(api, &request.name, cancel).await?;
    if pod.metadata.uid.as_deref() != Some(&request.uid) {
        return Err(ConnectionError::SourceChanged);
    }
    let spec = pod.spec.ok_or(ConnectionError::InvalidResponse)?;
    if !spec.containers.iter().any(|c| c.name == request.container)
        && !spec
            .init_containers
            .as_ref()
            .is_some_and(|cs| cs.iter().any(|c| c.name == request.container))
        && !spec
            .ephemeral_containers
            .as_ref()
            .is_some_and(|cs| cs.iter().any(|c| c.name == request.container))
    {
        return Err(ConnectionError::NotFound);
    }
    if request.mode != PodStreamMode::Logs {
        let running = pod.status.is_some_and(|status| {
            status
                .container_statuses
                .into_iter()
                .flatten()
                .chain(status.init_container_statuses.into_iter().flatten())
                .chain(status.ephemeral_container_statuses.into_iter().flatten())
                .any(|c| {
                    c.name == request.container && c.state.is_some_and(|s| s.running.is_some())
                })
        });
        if !running {
            return Err(ConnectionError::ProcessUnavailable);
        }
    }
    Ok(())
}
impl ConnectionEngine {
    pub async fn pod_stream_open(&self, request: PodStartRequest) -> Result<PodStreamSnapshot> {
        validate_target(&request.namespace, &request.name, &request.uid)?;
        if !dns_label(&request.container)
            || request.tail_lines > 10_000
            || (request.mode != PodStreamMode::Shell && request.shell.is_some())
        {
            return Err(ConnectionError::InvalidInput);
        }
        shell_command(request.shell.as_deref())?;
        if self.pod_runtime.stream_slots.available_permits() == 0 {
            return Err(ConnectionError::LimitExceeded);
        }
        let scope = SessionRequest {
            cluster_id: request.cluster_id.clone(),
            session_id: request.session_id.clone(),
        };
        let action = match request.mode {
            PodStreamMode::Logs => PermissionAction::Logs,
            PodStreamMode::Shell => PermissionAction::Exec,
            PodStreamMode::Attach => PermissionAction::Attach,
        };
        let authorization = self
            .require_permissions(
                &scope,
                &[PermissionCheck {
                    kind: ResourceKind::Pods,
                    action,
                    namespace: Some(request.namespace.clone()),
                    name: Some(request.name.clone()),
                }],
            )
            .await?;
        let slot_permit = self
            .pod_runtime
            .stream_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| ConnectionError::LimitExceeded)?;
        let _permit = self
            .workers
            .try_acquire()
            .map_err(|_| ConnectionError::Busy)?;
        let (client, cancel) = self.pod_client(&scope)?;
        let client = super::permissions::authorized_client(client, authorization);
        let api = Api::<Pod>::namespaced(client, &request.namespace);
        operation(&cancel, verify_process(&api, &request, &cancel)).await?;
        self.check_pod_session(&scope)?;
        let id = random_id()?;
        let (input_tx, input_rx) = mpsc::channel(8);
        let (resize_tx, resize_rx) = mpsc::channel(8);
        let interactive = request.mode != PodStreamMode::Logs;
        let state = Arc::new(Mutex::new(StreamState {
            scope,
            id: id.clone(),
            cancel: cancel.clone(),
            ring: Ring::default(),
            status: "opening",
            error: None,
            input: interactive.then_some(input_tx),
            resize: interactive.then_some(resize_tx),
            finished: false,
        }));
        let snapshot;
        {
            let mut registry = self
                .pod_runtime
                .registry
                .lock()
                .map_err(|_| ConnectionError::StoreUnavailable)?;
            registry
                .streams
                .retain(|_, s| s.lock().is_ok_and(|s| !s.cancel.is_cancelled()));
            if registry.streams.len() >= STREAM_LIMIT {
                registry.streams.retain(|_, s| {
                    s.lock()
                        .is_ok_and(|s| !s.finished || !s.ring.text.is_empty())
                });
            }
            if registry.streams.len() >= STREAM_LIMIT {
                return Err(ConnectionError::LimitExceeded);
            }
            snapshot = state
                .lock()
                .map_err(|_| ConnectionError::StoreUnavailable)?
                .snapshot();
            registry.streams.insert(id, state.clone());
        }
        self.pod_runtime.tasks.spawn(async move {
            let _slot_permit = slot_permit;
            let result = if request.mode == PodStreamMode::Logs {
                run_logs(&api, &request, &state, &cancel).await
            } else {
                run_terminal(&api, &request, &state, &cancel, input_rx, resize_rx).await
            };
            if let Ok(mut state) = state.lock() {
                state.finish(result);
            }
        });
        Ok(snapshot)
    }
    pub fn pod_stream_read(&self, request: PodStreamRequest) -> Result<PodStreamSnapshot> {
        self.check_pod_session(&stream_scope(&request))?;
        let slot = self.pod_runtime.stream(&request)?;
        let mut state = slot.lock().map_err(|_| ConnectionError::StoreUnavailable)?;
        if state.cancel.is_cancelled() {
            state.ring.clear();
            return Err(ConnectionError::StaleSession);
        }
        Ok(state.snapshot())
    }
    pub fn pod_stream_close(&self, request: PodStreamRequest) -> Result<()> {
        super::validate_id(&request.stream_id)?;
        self.check_pod_lease(&stream_scope(&request))?;
        let mut registry = self
            .pod_runtime
            .registry
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        if let Some(slot) = registry.streams.get(&request.stream_id) {
            let mut state = slot.lock().map_err(|_| ConnectionError::StoreUnavailable)?;
            if !same_scope(&state.scope, &stream_scope(&request)) {
                return Err(ConnectionError::StaleSession);
            }
            state.cancel.cancel();
            state.ring.clear();
            state.input = None;
            state.resize = None;
        }
        registry.streams.remove(&request.stream_id);
        Ok(())
    }
    pub fn pod_stream_write(&self, request: PodInputRequest) -> Result<()> {
        if request.data.is_empty() || request.data.len() > INPUT_LIMIT {
            return Err(ConnectionError::InvalidInput);
        }
        self.check_pod_session(&stream_scope(&request.stream))?;
        let slot = self.pod_runtime.stream(&request.stream)?;
        let state = slot.lock().map_err(|_| ConnectionError::StoreUnavailable)?;
        if state.cancel.is_cancelled() {
            return Err(ConnectionError::StaleSession);
        }
        state
            .input
            .as_ref()
            .ok_or(ConnectionError::ResourceUnavailable)?
            .try_send(request.data)
            .map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => ConnectionError::Busy,
                _ => ConnectionError::ResourceUnavailable,
            })
    }
    pub fn pod_stream_resize(&self, request: PodResizeRequest) -> Result<()> {
        if request.cols == 0 || request.rows == 0 || request.cols > 4096 || request.rows > 4096 {
            return Err(ConnectionError::InvalidInput);
        }
        self.check_pod_session(&stream_scope(&request.stream))?;
        let slot = self.pod_runtime.stream(&request.stream)?;
        let state = slot.lock().map_err(|_| ConnectionError::StoreUnavailable)?;
        if state.cancel.is_cancelled() {
            return Err(ConnectionError::StaleSession);
        }
        state
            .resize
            .as_ref()
            .ok_or(ConnectionError::ResourceUnavailable)?
            .try_send(TerminalSize {
                width: request.cols,
                height: request.rows,
            })
            .map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => ConnectionError::Busy,
                _ => ConnectionError::ResourceUnavailable,
            })
    }
    pub async fn pod_forward_start(&self, request: PodForwardRequest) -> Result<PodForward> {
        validate_target(&request.namespace, &request.name, &request.uid)?;
        validate_port(request.remote_port)?;
        if self.pod_runtime.forward_slots.available_permits() == 0 {
            return Err(ConnectionError::LimitExceeded);
        }
        let scope = SessionRequest {
            cluster_id: request.cluster_id.clone(),
            session_id: request.session_id.clone(),
        };
        let authorization = self
            .require_permissions(
                &scope,
                &[PermissionCheck {
                    kind: ResourceKind::Pods,
                    action: PermissionAction::PortForward,
                    namespace: Some(request.namespace.clone()),
                    name: Some(request.name.clone()),
                }],
            )
            .await?;
        let slot_permit = self
            .pod_runtime
            .forward_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| ConnectionError::LimitExceeded)?;
        let _permit = self
            .workers
            .try_acquire()
            .map_err(|_| ConnectionError::Busy)?;
        let (client, cancel) = self.pod_client(&scope)?;
        let client = super::permissions::authorized_client(client, authorization);
        let api = Api::<Pod>::namespaced(client, &request.namespace);
        operation(
            &cancel,
            verify_pod(&api, &request.name, &request.uid, None, &cancel),
        )
        .await?;
        let listener = operation(&cancel, async {
            TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, request.local_port))
                .await
                .map_err(|_| ConnectionError::ResourceUnavailable)
        })
        .await?;
        self.check_pod_session(&scope)?;
        let view = PodForward {
            id: random_id()?,
            cluster_id: request.cluster_id.clone(),
            session_id: request.session_id.clone(),
            namespace: request.namespace.clone(),
            name: request.name.clone(),
            remote_port: request.remote_port,
            local_port: listener
                .local_addr()
                .map_err(|_| ConnectionError::Network)?
                .port(),
            status: "running",
            error: None,
        };
        let state = Arc::new(Mutex::new(ForwardState {
            view: view.clone(),
            cancel: cancel.clone(),
            finished: false,
        }));
        {
            let mut registry = self
                .pod_runtime
                .registry
                .lock()
                .map_err(|_| ConnectionError::StoreUnavailable)?;
            registry
                .forwards
                .retain(|_, f| f.lock().is_ok_and(|f| !f.cancel.is_cancelled()));
            if registry.forwards.len() >= FORWARD_LIMIT {
                registry
                    .forwards
                    .retain(|_, f| f.lock().is_ok_and(|f| !f.finished));
            }
            if registry.forwards.len() >= FORWARD_LIMIT {
                return Err(ConnectionError::LimitExceeded);
            }
            registry.forwards.insert(view.id.clone(), state.clone());
        }
        self.pod_runtime.tasks.spawn(async move {
            let _slot_permit = slot_permit;
            run_listener(listener, api, request, cancel, state).await;
        });
        Ok(view)
    }
    pub fn pod_forward_list(&self, request: SessionRequest) -> Result<Vec<PodForward>> {
        self.check_pod_session(&request)?;
        let registry = self
            .pod_runtime
            .registry
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        let mut views = Vec::new();
        for slot in registry.forwards.values() {
            let state = slot.lock().map_err(|_| ConnectionError::StoreUnavailable)?;
            if state.view.cluster_id == request.cluster_id
                && state.view.session_id == request.session_id
                && !state.cancel.is_cancelled()
            {
                views.push(state.view.clone());
            }
        }
        views.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(views)
    }
    pub fn pod_forward_stop(&self, request: PodForwardStopRequest) -> Result<()> {
        self.check_pod_lease(&SessionRequest {
            cluster_id: request.cluster_id.clone(),
            session_id: request.session_id.clone(),
        })?;
        super::validate_id(&request.id)?;
        let mut registry = self
            .pod_runtime
            .registry
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        if let Some(slot) = registry.forwards.get(&request.id) {
            let state = slot.lock().map_err(|_| ConnectionError::StoreUnavailable)?;
            if state.view.cluster_id != request.cluster_id
                || state.view.session_id != request.session_id
            {
                return Err(ConnectionError::StaleSession);
            }
            state.cancel.cancel();
        }
        registry.forwards.remove(&request.id);
        Ok(())
    }
}
fn running(state: &Arc<Mutex<StreamState>>) -> Result<()> {
    let mut state = state
        .lock()
        .map_err(|_| ConnectionError::StoreUnavailable)?;
    if state.cancel.is_cancelled() {
        return Err(ConnectionError::StaleSession);
    }
    state.status = "running";
    state.ring.sequence = state.ring.sequence.saturating_add(1);
    Ok(())
}
fn output(
    state: &Arc<Mutex<StreamState>>,
    decoder: &mut Decoder,
    bytes: &[u8],
    eof: bool,
) -> Result<()> {
    let text = decoder.decode(bytes, eof);
    let mut state = state
        .lock()
        .map_err(|_| ConnectionError::StoreUnavailable)?;
    if state.cancel.is_cancelled() {
        state.ring.clear();
        return Err(ConnectionError::StaleSession);
    }
    state.ring.push(&text);
    Ok(())
}
async fn run_logs(
    api: &Api<Pod>,
    request: &PodStartRequest,
    state: &Arc<Mutex<StreamState>>,
    cancel: &CancellationToken,
) -> Result<()> {
    let params = LogParams {
        container: Some(request.container.clone()),
        follow: !request.previous,
        previous: request.previous,
        timestamps: request.timestamps,
        tail_lines: Some(i64::from(request.tail_lines)),
        ..Default::default()
    };
    let mut stream = operation(cancel, async {
        api.log_stream(&request.name, &params)
            .await
            .map_err(map_error)
    })
    .await?;
    operation(cancel, verify_process(api, request, cancel)).await?;
    running(state)?;
    let mut decoder = Decoder::default();
    let mut buffer = [0; 8192];
    loop {
        let count = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(ConnectionError::StaleSession),
            count = FuturesReadExt::read(&mut stream, &mut buffer) => count.map_err(|error| {
                if error.kind() == std::io::ErrorKind::TimedOut {
                    ConnectionError::Timeout
                } else if error.kind() == std::io::ErrorKind::Interrupted && cancel.is_cancelled() {
                    ConnectionError::StaleSession
                } else {
                    ConnectionError::Network
                }
            })?,
        };
        output(state, &mut decoder, &buffer[..count], count == 0)?;
        if count == 0 {
            return Ok(());
        }
    }
}
async fn run_terminal(
    api: &Api<Pod>,
    request: &PodStartRequest,
    state: &Arc<Mutex<StreamState>>,
    cancel: &CancellationToken,
    mut inputs: mpsc::Receiver<String>,
    mut resizes: mpsc::Receiver<TerminalSize>,
) -> Result<()> {
    let params = AttachParams {
        container: Some(request.container.clone()),
        ..AttachParams::interactive_tty()
    };
    let mut process: AttachedProcess = operation(cancel, async {
        if request.mode == PodStreamMode::Shell {
            api.exec(
                &request.name,
                [shell_command(request.shell.as_deref())?],
                &params,
            )
            .await
            .map_err(map_error)
        } else {
            api.attach(&request.name, &params).await.map_err(map_error)
        }
    })
    .await?;
    let result = async {
        operation(cancel, verify_pod(api, &request.name, &request.uid, None, cancel)).await?;
        operation(cancel, verify_process(api, request, cancel)).await?;
        let mut stdout = process.stdout().ok_or(ConnectionError::InvalidResponse)?;
        let mut stdin = process.stdin().ok_or(ConnectionError::InvalidResponse)?;
        let mut sizes = process.terminal_size().ok_or(ConnectionError::InvalidResponse)?;
        let status = process.take_status().ok_or(ConnectionError::InvalidResponse)?;
        tokio::pin!(status);
        running(state)?;
        let mut decoder = Decoder::default(); let mut buffer = [0; 8192];
        let mut ended = false;
        let mut input_open = true;
        let mut resize_open = true;
        let mut exit_error = None;
        loop {
            tokio::select! { biased;
                _ = cancel.cancelled() => return Err(ConnectionError::StaleSession),
                result = async {
                    tokio::select! {
                        count = stdout.read(&mut buffer) => {
                            let count = count.map_err(|_| ConnectionError::Network)?;
                            output(state, &mut decoder, &buffer[..count], count == 0)?;
                            Ok::<bool, ConnectionError>(count == 0)
                        },
                        value = &mut status, if !ended => {
                            ended = true;
                            if value.is_none_or(|s| s.status.as_deref() != Some("Success")) { exit_error = Some(ConnectionError::ResourceUnavailable); }
                            Ok(false)
                        },
                 value = inputs.recv(), if input_open => {
                    let Some(value) = value else { input_open = false; return Ok(false); };
                    operation(cancel, verify_process(api, request, cancel)).await?;
                    operation(cancel, async { stdin.write_all(value.as_bytes()).await.map_err(|_| ConnectionError::Network) }).await?;
                    Ok(false)
                },
                value = resizes.recv(), if resize_open => {
                    let Some(value) = value else { resize_open = false; return Ok(false); };
                    operation(cancel, async { sizes.send(value).await.map_err(|_| ConnectionError::Network) }).await?;
                    Ok(false)
                }
                    }
                } => { if result? { break; } }
            }
        }
        if !ended {
            let value = operation(cancel, async { Ok(status.await) }).await?;
            if value.is_none_or(|s| s.status.as_deref() != Some("Success")) { exit_error = Some(ConnectionError::ResourceUnavailable); }
        }
        exit_error.map_or(Ok(()), Err)
    }.await;
    inputs.close();
    resizes.close();
    while inputs.try_recv().is_ok() {}
    while resizes.try_recv().is_ok() {}
    // kube owns an internal websocket task; always abort on disposal and explicitly reap it.
    // At this point stdout and the remote exit status have both been consumed, or disposal won.
    // Abort before join: timing out join itself would detach kube's extracted JoinHandle.
    process.abort();
    let _ = process.join().await;
    result
}
fn validate_port(remote: u16) -> Result<()> {
    if remote == 0 {
        Err(ConnectionError::InvalidInput)
    } else {
        Ok(())
    }
}
async fn run_listener(
    listener: TcpListener,
    api: Api<Pod>,
    request: PodForwardRequest,
    cancel: CancellationToken,
    state: Arc<Mutex<ForwardState>>,
) {
    let connections = TaskTracker::new();
    let permits = Arc::new(tokio::sync::Semaphore::new(CONNECTION_LIMIT));
    let mut failure = None;
    loop {
        tokio::select! { biased;
            _ = cancel.cancelled() => break,
            accepted = listener.accept() => {
                let (socket, _) = match accepted { Ok(value) => value, Err(_) => { failure = Some(ConnectionError::Network); break; } };
                let Ok(permit) = permits.clone().try_acquire_owned() else { drop(socket); continue; };
                let api = api.clone(); let request = request.clone(); let cancel = cancel.child_token(); let state = state.clone();
                connections.spawn(async move {
                    let _permit = permit;
                    let result = forward_connection(socket, &api, &request, &cancel).await;
                    if !cancel.is_cancelled() && let Err(error) = result && let Ok(mut state) = state.lock() { state.view.status = "error"; state.view.error = Some(error); }
                });
            }
        }
    }
    drop(listener);
    cancel.cancel();
    connections.close();
    connections.wait().await;
    if let Ok(mut state) = state.lock() {
        state.finished = true;
        state.view.status = if failure.is_some() { "error" } else { "exited" };
        state.view.error = failure;
    }
}
async fn forward_connection(
    mut socket: TcpStream,
    api: &Api<Pod>,
    request: &PodForwardRequest,
    cancel: &CancellationToken,
) -> Result<()> {
    operation(
        cancel,
        verify_pod(api, &request.name, &request.uid, None, cancel),
    )
    .await?;
    let mut forward = operation(cancel, async {
        api.portforward(&request.name, &[request.remote_port])
            .await
            .map_err(map_error)
    })
    .await?;
    let result = async {
        let mut remote = forward.take_stream(request.remote_port).ok_or(ConnectionError::InvalidResponse)?;
        let error = forward.take_error(request.remote_port).ok_or(ConnectionError::InvalidResponse)?;
        tokio::pin!(error);
        tokio::select! { biased;
            _ = cancel.cancelled() => Err(ConnectionError::StaleSession),
            error = &mut error => { let _ = error; Err(ConnectionError::ResourceUnavailable) },
            result = tokio::time::timeout(CONNECTION_LIFETIME, tokio::io::copy_bidirectional(&mut socket, &mut remote)) => match result { Ok(Ok(_)) => Ok(()), Ok(Err(_)) => Err(ConnectionError::Network), Err(_) => Err(ConnectionError::Timeout) },
        }
    }.await;
    // Portforwarder has no Drop abort: dropping it would detach its websocket task.
    forward.abort();
    let _ = forward.join().await;
    result
}

#[cfg(test)]
#[path = "pod_runtime_tests.rs"]
mod tests;

#[cfg(test)]
mod update_gate_tests {
    use super::*;

    #[test]
    fn opening_and_retiring_streams_and_forwards_block_installation() {
        let runtime = PodRuntime::default();
        assert!(!runtime.has_active_work());
        let stream = runtime.stream_slots.try_acquire().unwrap();
        assert!(runtime.has_active_work());
        drop(stream);
        let forward = runtime.forward_slots.try_acquire().unwrap();
        assert!(runtime.has_active_work());
        drop(forward);
        assert!(!runtime.has_active_work());
    }
}
