use crate::catalog::{CatalogEngine, CatalogError, ConnectionTarget};
use kube::Client;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

mod auth;
mod config;
pub mod create;
#[cfg(test)]
mod create_tests;
pub mod custom_resources;
pub mod delete;
#[cfg(test)]
mod delete_tests;
pub mod detail_collections;
pub mod detail_facts;
pub mod detail_metadata;
pub mod details;
pub mod event_diagnostic;
pub mod health;
pub mod helm;
pub mod kubectl_tools;
#[cfg(test)]
mod lifecycle_tests;
mod metrics;
pub mod namespace_cache;
pub mod node_actions;
pub mod node_drain;
#[cfg(test)]
mod node_tests;
pub mod permissions;
#[cfg(test)]
mod platform_tests;
pub mod pod_runtime;
pub mod pod_sync;
pub mod pods;
pub mod rbac;
pub mod rbac_management;
mod renewal;
pub mod resource_edit;
#[cfg(test)]
mod resource_tests;
mod resources;
#[cfg(test)]
mod runtime_status_tests;
pub mod service_forward;
#[cfg(test)]
mod tests;
mod transport;
pub mod workload_actions;

const DEADLINE: Duration = Duration::from_secs(15);
type Result<T> = std::result::Result<T, ConnectionError>;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionError {
    InvalidInput,
    ProcessUnavailable,
    NotFound,
    KubeconfigUnavailable,
    KubeconfigInvalid,
    SourceChanged,
    UnsupportedAuth,
    AuthHelperMissing,
    AuthHelperFailed,
    AuthHelperInvalidResponse,
    AuthInteractiveRequired,
    AuthExpired,
    InsecureTransport,
    AuthFailed,
    TlsFailed,
    PermissionDenied,
    PermissionReviewUnavailable,
    ResourceUnavailable,
    Timeout,
    Network,
    ConnectionFailed,
    NotConnected,
    StaleSession,
    Busy,
    LimitExceeded,
    InvalidResponse,
    StoreUnavailable,
    HelmUnavailable,
    Conflict,
}
#[cfg(test)]
mod update_gate_tests {
    use super::*;

    #[tokio::test]
    async fn pending_mutation_or_busy_worker_defers_without_dropping_the_session() {
        let (_root, _catalog, engine, id, _) = tests::fixture();
        engine.open(OpenRequest { cluster_id: id }).await.unwrap();
        {
            let mut state = engine.session.lock().unwrap();
            state.as_mut().unwrap().creates.insert(
                ResourceKind::Pods,
                CreateSlot {
                    id: 1,
                    cancel: Some(CancellationToken::new()),
                },
            );
        }
        assert!(engine.has_active_runtime());
        assert!(!engine.prepare_update_shutdown());
        assert!(engine.session.lock().unwrap().is_some());
        engine
            .session
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .creates
            .clear();
        let worker = engine.workers.try_acquire().unwrap();
        assert!(!engine.prepare_update_shutdown());
        assert!(engine.session.lock().unwrap().is_some());
        drop(worker);
        assert!(engine.prepare_update_shutdown());
        assert!(engine.session.lock().unwrap().is_none());
    }
}
impl Session {
    fn catalog_status(&self) -> &'static str {
        if self.cancel.is_cancelled() {
            return "disconnected";
        }
        match (self.snapshot.status, self.client.is_some()) {
            ("connected", true) => "connected",
            ("connecting", _) => "connecting",
            ("error", _) => "error",
            _ => "disconnected",
        }
    }
}
struct CreateSlot {
    id: u64,
    cancel: Option<CancellationToken>,
}
fn validate_credentials(client: &Client) -> Result<()> {
    if client
        .valid_until()
        .is_some_and(|expiry| expiry <= jiff::Timestamp::now())
    {
        Err(ConnectionError::AuthExpired)
    } else {
        Ok(())
    }
}

impl ConnectionEngine {
    /// Project ephemeral reachability without granting another workspace lease.
    pub fn project_catalog_statuses(
        &self,
        snapshot: &mut crate::catalog::CatalogSnapshot,
    ) -> Result<()> {
        let states = self.catalog_connection_states()?;
        for cluster in &mut snapshot.clusters {
            cluster.status = states
                .iter()
                .find(|s| s.cluster_id == cluster.id)
                .map_or("disconnected", |s| s.status);
        }
        Ok(())
    }

    pub fn catalog_connection_states(&self) -> Result<Vec<crate::catalog::ClusterConnectionState>> {
        let state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        let mut states: Vec<_> = self
            .cluster_health_snapshot()?
            .into_iter()
            .map(|health| crate::catalog::ClusterConnectionState {
                cluster_id: health.cluster_id,
                status: health.status,
            })
            .collect();
        if let Some(session) = state.as_ref() {
            // Connecting/error are lease-specific. A connected lease must not hide a newer
            // failed health check. Merely opening a workspace does not disconnect a cluster.
            let status = session.catalog_status();
            if matches!(status, "connecting" | "error") {
                states.retain(|s| s.cluster_id != session.snapshot.cluster_id);
                states.push(crate::catalog::ClusterConnectionState {
                    cluster_id: session.snapshot.cluster_id.clone(),
                    status,
                });
            }
        }
        states.sort_by(|a, b| a.cluster_id.cmp(&b.cluster_id));
        Ok(states)
    }
}
fn validate_read_id(id: u64) -> Result<()> {
    if id == 0 || id > 9_007_199_254_740_991 {
        Err(ConnectionError::InvalidInput)
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenRequest {
    pub cluster_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionRequest {
    pub cluster_id: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum ResourceKind {
    Nodes,
    Namespaces,
    Pods,
    Deployments,
    DaemonSets,
    StatefulSets,
    ReplicaSets,
    ReplicationControllers,
    Jobs,
    CronJobs,
    ConfigMaps,
    Secrets,
    ResourceQuotas,
    LimitRanges,
    HorizontalPodAutoscalers,
    VerticalPodAutoscalers,
    PodDisruptionBudgets,
    PriorityClasses,
    RuntimeClasses,
    Leases,
    Services,
    Endpoints,
    Ingresses,
    IngressClasses,
    NetworkPolicies,
    EndpointSlices,
    MutatingWebhookConfigurations,
    ValidatingWebhookConfigurations,
    ValidatingAdmissionPolicies,
    ValidatingAdmissionPolicyBindings,
    PodSecurityPolicies,
    PersistentVolumes,
    PersistentVolumeClaims,
    StorageClasses,
    Events,
    ServiceAccounts,
    Roles,
    RoleBindings,
    ClusterRoles,
    ClusterRoleBindings,
    CustomResourceDefinitions,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub kind: ResourceKind,
    pub read_id: u64,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub continue_token: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
}
impl ResourceRequest {
    fn session(&self) -> SessionRequest {
        SessionRequest {
            cluster_id: self.cluster_id.clone(),
            session_id: self.session_id.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionSnapshot {
    pub cluster_id: String,
    pub session_id: String,
    pub context_name: String,
    pub status: &'static str,
    pub api_version: Option<String>,
    pub error: Option<ConnectionError>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResourcePage {
    pub cluster_id: String,
    pub session_id: String,
    pub kind: ResourceKind,
    pub read_id: u64,
    pub items: Vec<ResourceItem>,
    pub truncated: bool,
    pub fetched_at: u64,
    pub namespace: Option<String>,
    pub continue_token: Option<String>,
    pub metrics: Option<MetricState>,
}
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResourceItem {
    pub name: String,
    pub uid: Option<String>,
    pub resource_version: Option<String>,
    pub status: String,
    pub namespace: Option<String>,
    pub created_at: Option<String>,
    pub columns: std::collections::BTreeMap<String, String>,
    pub node: Option<NodeInfo>,
}
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MetricState {
    Available,
    Unavailable,
    Denied,
    Stale,
    Partial,
    Error,
}
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NodeInfo {
    pub roles: Vec<String>,
    pub taints: u32,
    pub version: Option<String>,
    pub internal_ips: Vec<String>,
    pub schedulable: bool,
    pub conditions: Vec<NodeCondition>,
    pub disk_allocatable: Option<String>,
    pub cpu: Usage,
    pub memory: Usage,
    pub metrics_timestamp: Option<String>,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NodeCondition {
    #[serde(rename = "type")]
    pub kind: String,
    pub status: String,
}
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Usage {
    pub used: Option<f64>,
    pub allocatable: Option<f64>,
    pub capacity: Option<f64>,
}

struct Session {
    namespace_cache: namespace_cache::LeaseCache,
    target: ConnectionTarget,
    snapshot: ConnectionSnapshot,
    client: Option<Client>,
    cancel: CancellationToken,
    reads: HashMap<ResourceKind, ReadSlot>,
    custom_discovery: Option<custom_resources::CustomSlot>,
    custom_instances: Option<custom_resources::CustomSlot>,
    creates: HashMap<ResourceKind, CreateSlot>,
    deletes: HashMap<ResourceKind, delete::DeleteSlot>,
    detail: details::DetailSlot,
    collections: HashMap<detail_collections::CollectionKind, detail_collections::CollectionSlot>,
    workload_action: workload_actions::WorkloadActionSlot,
    resource_edit: resource_edit::EditSlot,
    service_forward: service_forward::ServiceForwardSlot,
    node_action: node_actions::NodeActionSlot,
    node_drain: node_drain::DrainSlot,
    permissions: permissions::PermissionSession,
    rbac_users: Option<rbac::UsersSlot>,
    rbac_management: rbac_management::ManagementSlot,
}
struct ReadSlot {
    id: u64,
    cancel: Option<CancellationToken>,
    request: ResourceRequest,
}
impl Drop for Session {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

// A single main-window workspace owns the lease. Replacing or dropping it cancels all work.
pub struct ConnectionEngine {
    health: health::HealthRuntime,
    update_closing: AtomicBool,
    catalog: Arc<CatalogEngine>,
    session: Mutex<Option<Session>>,
    workers: tokio::sync::Semaphore,
    file_workers: Arc<tokio::sync::Semaphore>,
    auth_tasks: tokio_util::task::TaskTracker,
    open_generation: AtomicU64,
    authentication_generation: permissions::AuthenticationGeneration,
    pod_runtime: pod_runtime::PodRuntime,
    pod_sync: pod_sync::PodSyncRuntime,
    helm_runtime: helm::HelmRuntime,
}
impl ConnectionEngine {
    pub fn new(catalog: Arc<CatalogEngine>) -> Self {
        Self {
            health: health::HealthRuntime::default(),
            update_closing: AtomicBool::new(false),
            catalog,
            session: Mutex::new(None),
            workers: tokio::sync::Semaphore::new(4),
            file_workers: Arc::new(tokio::sync::Semaphore::new(2)),
            auth_tasks: tokio_util::task::TaskTracker::new(),
            open_generation: AtomicU64::new(0),
            authentication_generation: Arc::new(AtomicU64::new(1)),
            pod_runtime: pod_runtime::PodRuntime::default(),
            pod_sync: pod_sync::PodSyncRuntime::default(),
            helm_runtime: helm::HelmRuntime::default(),
        }
    }
    pub async fn open(&self, request: OpenRequest) -> Result<ConnectionSnapshot> {
        validate_id(&request.cluster_id)?;
        let generation = {
            let mut state = self
                .session
                .lock()
                .map_err(|_| ConnectionError::StoreUnavailable)?;
            if self.update_closing.load(Ordering::SeqCst) {
                return Err(ConnectionError::Busy);
            }
            let generation = self.open_generation.fetch_add(1, Ordering::SeqCst) + 1;
            permissions::invalidate_authentication(&self.authentication_generation);
            state.take();
            generation
        };
        let catalog = self.catalog.clone();
        let id = request.cluster_id.clone();
        let target = crate::runtime::spawn_blocking(move || catalog.connection_target(&id))
            .await
            .map_err(|_| ConnectionError::StoreUnavailable)?
            .map_err(map_catalog_error)?;
        let mut random = [0; 16];
        getrandom::fill(&mut random).map_err(|_| ConnectionError::ConnectionFailed)?;
        let snapshot = ConnectionSnapshot {
            cluster_id: request.cluster_id,
            session_id: random.iter().map(|byte| format!("{byte:02x}")).collect(),
            context_name: target.context_name.clone(),
            status: "disconnected",
            api_version: None,
            error: None,
        };
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        if self.update_closing.load(Ordering::SeqCst) {
            return Err(ConnectionError::Busy);
        }
        if generation != self.open_generation.load(Ordering::SeqCst) {
            return Err(ConnectionError::StaleSession);
        }
        *state = Some(Session {
            namespace_cache: namespace_cache::LeaseCache::default(),
            target,
            snapshot: snapshot.clone(),
            client: None,
            cancel: CancellationToken::new(),
            reads: HashMap::new(),
            custom_discovery: None,
            custom_instances: None,
            creates: HashMap::new(),
            deletes: HashMap::new(),
            detail: details::DetailSlot::default(),
            collections: HashMap::new(),
            workload_action: workload_actions::WorkloadActionSlot::default(),
            resource_edit: resource_edit::EditSlot::default(),
            service_forward: service_forward::ServiceForwardSlot::default(),
            node_action: node_actions::NodeActionSlot::default(),
            node_drain: node_drain::DrainSlot::default(),
            permissions: permissions::PermissionSession::default(),
            rbac_users: None,
            rbac_management: rbac_management::ManagementSlot::default(),
        });
        Ok(snapshot)
    }
    pub fn disconnect(&self, request: SessionRequest) -> Result<()> {
        validate_request(&request)?;
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        // Old disposal cannot release a replacement lease, including one for the same cluster.
        if state.as_ref().is_some_and(|s| matches_session(s, &request)) {
            permissions::invalidate_authentication(&self.authentication_generation);
            state.take();
            self.clear_cluster_health(&request.cluster_id)?;
        }
        Ok(())
    }
    pub fn has_active_runtime(&self) -> bool {
        self.pod_runtime.has_active_work()
            || self.session.lock().map_or(true, |state| {
                state.as_ref().is_some_and(|session| {
                    session.creates.values().any(|slot| slot.cancel.is_some())
                        || session.deletes.values().any(|slot| slot.cancel.is_some())
                        || session.workload_action.active()
                        || session.resource_edit.active()
                        || session.service_forward.active()
                        || session.node_action.active()
                        || session.node_drain.active()
                })
            })
    }
    pub fn prepare_update_shutdown(&self) -> bool {
        // Reserving worker permits closes the gap between checking pending mutations and
        // dropping their session. A short read may defer this tick, not cancel user work.
        let Ok(_workers) = self.workers.try_acquire_many(4) else {
            return false;
        };
        if self.has_active_runtime() {
            return false;
        }
        let Ok(mut state) = self.session.lock() else {
            return false;
        };
        if state
            .as_ref()
            .is_some_and(|session| session.rbac_management.is_executing())
        {
            return false;
        }
        self.health.clear();
        self.update_closing.store(true, Ordering::SeqCst);
        permissions::invalidate_authentication(&self.authentication_generation);
        self.open_generation.fetch_add(1, Ordering::SeqCst);
        self.pod_sync.cancel_all();
        state.take();
        true
    }
    pub fn finish_update_failure(&self) {
        self.update_closing.store(false, Ordering::SeqCst);
    }
    pub fn shutdown(&self) {
        permissions::invalidate_authentication(&self.authentication_generation);
        self.pod_sync.cancel_all();
        self.open_generation.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut state) = self.session.lock() {
            state.take();
            self.health.clear();
        }
    }
    pub async fn drain(&self) {
        // Cancellation must reach connect futures before their helper trackers can be empty.
        let _workers = self.workers.acquire_many(4).await;
        self.auth_tasks.close();
        self.auth_tasks.wait().await;
        self.pod_runtime.drain().await;
        self.pod_sync.drain().await;
    }
    fn pod_client(&self, request: &SessionRequest) -> Result<(Client, CancellationToken)> {
        validate_request(request)?;
        let state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        let session = current(&state, request)?;
        let client = session
            .client
            .clone()
            .ok_or(ConnectionError::NotConnected)?;
        validate_credentials(&client)?;
        Ok((client, session.cancel.child_token()))
    }
    fn check_pod_session(&self, request: &SessionRequest) -> Result<()> {
        self.pod_client(request).map(|_| ())
    }
    fn check_pod_lease(&self, request: &SessionRequest) -> Result<()> {
        validate_request(request)?;
        let state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        current(&state, request).map(|_| ())
    }
    pub fn status(&self, request: SessionRequest) -> Result<ConnectionSnapshot> {
        validate_request(&request)?;
        let state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        Ok(current(&state, &request)?.snapshot.clone())
    }
    pub async fn connect(&self, request: SessionRequest) -> Result<ConnectionSnapshot> {
        let cancel = {
            let state = self
                .session
                .lock()
                .map_err(|_| ConnectionError::StoreUnavailable)?;
            current(&state, &request)?.cancel.child_token()
        };
        let cache_identity = Arc::new(Mutex::new(None));
        let captured_identity = cache_identity.clone();
        let response = self
            .connect_with(request.clone(), |target| async move {
                let before = namespace_cache::source_identity_async(target.clone()).await?;
                let client = config::build_client_owned(
                    target.clone(),
                    self.file_workers.clone(),
                    self.auth_tasks.clone(),
                    cancel,
                    self.authentication_generation.clone(),
                )
                .await?;
                let after = namespace_cache::source_identity_async(target).await?;
                if before != after {
                    return Err(ConnectionError::SourceChanged);
                }
                *captured_identity
                    .lock()
                    .map_err(|_| ConnectionError::StoreUnavailable)? = Some(after);
                Ok(client)
            })
            .await?;
        if let Some(identity) = cache_identity
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?
            .take()
        {
            let mut state = self
                .session
                .lock()
                .map_err(|_| ConnectionError::StoreUnavailable)?;
            let session = current_mut(&mut state, &request)?;
            if response.status == "connected" {
                session.namespace_cache.bind(
                    identity,
                    self.authentication_generation.load(Ordering::SeqCst),
                );
            }
        }
        Ok(response)
    }
    async fn connect_with<F, Fut>(
        &self,
        request: SessionRequest,
        build: F,
    ) -> Result<ConnectionSnapshot>
    where
        F: FnOnce(ConnectionTarget) -> Fut,
        Fut: Future<Output = Result<Client>>,
    {
        validate_request(&request)?;
        let _permit = self
            .workers
            .try_acquire()
            .map_err(|_| ConnectionError::Busy)?;
        let (target, cancel) = {
            let mut state = self
                .session
                .lock()
                .map_err(|_| ConnectionError::StoreUnavailable)?;
            let session = current_mut(&mut state, &request)?;
            match session.snapshot.status {
                "connecting" => return Err(ConnectionError::Busy),
                "connected" => return Ok(session.snapshot.clone()),
                _ => {}
            }
            self.clear_cluster_health(&request.cluster_id)?;
            permissions::invalidate_authentication(&self.authentication_generation);
            session.snapshot.status = "connecting";
            session.snapshot.error = None;
            (session.target.clone(), session.cancel.child_token())
        };
        let result = bounded(&cancel, async {
            let source = health::source_identity(target.clone()).await?;
            let client = build(target.clone()).await?;
            // Saved metadata may have been removed/reimported during configuration preparation.
            let catalog = self.catalog.clone();
            let id = request.cluster_id.clone();
            let latest = crate::runtime::spawn_blocking(move || catalog.connection_target(&id))
                .await
                .map_err(|_| ConnectionError::StoreUnavailable)?
                .map_err(map_catalog_error)?;
            if latest != target {
                return Err(ConnectionError::SourceChanged);
            }
            let version = transport::version(&client, &cancel).await?;
            validate_credentials(&client)?;
            if health::source_identity(target.clone()).await? != source {
                return Err(ConnectionError::SourceChanged);
            }
            let catalog = self.catalog.clone();
            let id = request.cluster_id.clone();
            let latest = crate::runtime::spawn_blocking(move || catalog.connection_target(&id))
                .await
                .map_err(|_| ConnectionError::StoreUnavailable)?
                .map_err(map_catalog_error)?;
            if latest != target {
                return Err(ConnectionError::SourceChanged);
            }
            Ok((client, version, source))
        })
        .await;
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        let session = current_mut(&mut state, &request)?;
        match result {
            Ok((client, version, source)) => {
                self.health.register(
                    request.cluster_id.clone(),
                    target.clone(),
                    source,
                    client.clone(),
                    version.clone(),
                )?;
                session.client = Some(client);
                session.snapshot.status = "connected";
                session.snapshot.api_version = Some(version);
                session.snapshot.error = None;
            }
            Err(error) => {
                session.client = None;
                session.snapshot.status = "error";
                session.snapshot.api_version = None;
                session.snapshot.error = Some(error);
            }
        }
        Ok(session.snapshot.clone())
    }
    pub fn cancel_resources(&self, request: ResourceRequest) -> Result<()> {
        transport::validate_resource_request(&request)?;
        validate_request(&request.session())?;
        validate_read_id(request.read_id)?;
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        if let Some(session) = state
            .as_mut()
            .filter(|s| matches_session(s, &request.session()))
        {
            let slot = session.reads.entry(request.kind).or_insert(ReadSlot {
                id: 0,
                cancel: None,
                request: request.clone(),
            });
            if request.read_id == slot.id && request != slot.request {
                return Err(ConnectionError::InvalidInput);
            }
            if request.read_id >= slot.id {
                if let Some(cancel) = slot.cancel.take() {
                    cancel.cancel();
                }
                slot.id = request.read_id;
                slot.request = request;
            }
        }
        Ok(())
    }
    pub async fn resources(&self, request: ResourceRequest) -> Result<ResourcePage> {
        transport::validate_resource_request(&request)?;
        let identity = request.session();
        validate_request(&identity)?;
        validate_read_id(request.read_id)?;
        self.reject_known_denied_read(
            &identity,
            &permissions::PermissionCheck {
                kind: request.kind,
                action: permissions::PermissionAction::List,
                namespace: request.namespace.clone(),
                name: None,
            },
        )?;
        let _permit = self
            .workers
            .try_acquire()
            .map_err(|_| ConnectionError::Busy)?;
        let (client, cancel) = {
            let mut state = self
                .session
                .lock()
                .map_err(|_| ConnectionError::StoreUnavailable)?;
            let session = current_mut(&mut state, &identity)?;
            let client = session
                .client
                .clone()
                .ok_or(ConnectionError::NotConnected)?;
            validate_credentials(&client)?;
            let cancel = session.cancel.child_token();
            let slot = session.reads.entry(request.kind).or_insert(ReadSlot {
                id: 0,
                cancel: None,
                request: request.clone(),
            });
            if request.read_id <= slot.id {
                return Err(ConnectionError::StaleSession);
            }
            if let Some(old) = slot.cancel.replace(cancel.clone()) {
                old.cancel();
            }
            slot.id = request.read_id;
            slot.request = request.clone();
            (client, cancel)
        };
        let result = bounded(&cancel, transport::resources(&client, &request, &cancel)).await;
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        let session = current_mut(&mut state, &identity)?;
        let slot = session
            .reads
            .get_mut(&request.kind)
            .ok_or(ConnectionError::StaleSession)?;
        if slot.id != request.read_id {
            return Err(ConnectionError::StaleSession);
        }
        if cancel.is_cancelled() && result != Err(ConnectionError::Timeout) {
            return Err(ConnectionError::StaleSession);
        }
        slot.cancel = None;
        result
    }
}
async fn bounded<T>(
    cancel: &CancellationToken,
    operation: impl Future<Output = Result<T>>,
) -> Result<T> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(ConnectionError::StaleSession),
        result = tokio::time::timeout(DEADLINE, operation) => match result {
            Ok(result) => result,
            Err(_) => { cancel.cancel(); Err(ConnectionError::Timeout) }
        },
    }
}
fn matches_session(session: &Session, request: &SessionRequest) -> bool {
    session.snapshot.cluster_id == request.cluster_id
        && session.snapshot.session_id == request.session_id
}
fn current<'a>(state: &'a Option<Session>, request: &SessionRequest) -> Result<&'a Session> {
    state
        .as_ref()
        .filter(|s| matches_session(s, request))
        .ok_or(ConnectionError::StaleSession)
}
fn current_mut<'a>(
    state: &'a mut Option<Session>,
    request: &SessionRequest,
) -> Result<&'a mut Session> {
    state
        .as_mut()
        .filter(|s| matches_session(s, request))
        .ok_or(ConnectionError::StaleSession)
}
fn validate_id(id: &str) -> Result<()> {
    if id.is_empty() || id.len() > 256 || id.trim() != id || id.chars().any(char::is_control) {
        Err(ConnectionError::InvalidInput)
    } else {
        Ok(())
    }
}
fn validate_request(request: &SessionRequest) -> Result<()> {
    validate_id(&request.cluster_id)?;
    validate_id(&request.session_id)
}
fn map_catalog_error(error: CatalogError) -> ConnectionError {
    match error {
        CatalogError::InvalidInput => ConnectionError::InvalidInput,
        CatalogError::NotFound => ConnectionError::NotFound,
        CatalogError::StoreUnavailable | CatalogError::StoreCorrupt => {
            ConnectionError::StoreUnavailable
        }
        CatalogError::LimitExceeded => ConnectionError::LimitExceeded,
        _ => ConnectionError::KubeconfigUnavailable,
    }
}
