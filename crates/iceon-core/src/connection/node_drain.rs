//! Conservative HTTP eviction workflow, not kubectl's force/delete drain defaults.
//! Behavioral reference: Freelens cd07118f843c4a69f937a319a37a7c12ef2c92d8,
//! packages/core/src/renderer/components/node-pod-menu/node-menu.tsx:69-104.
//! No upstream source is adapted. PVC backing storage is not certified as nonlocal.
use super::{
    ConnectionEngine, ConnectionError, ResourceKind, Result, SessionRequest,
    details::{ResourceReference, named_path, validate_reference},
    permissions::{AuthorizationEpoch, PermissionAction, PermissionCheck, authorized_client},
};
use http::{Method, Request};
use http_body_util::BodyExt;
use kube::{Client, client::Body};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

const INPUT_LIMIT: usize = 2 * 1024 * 1024;
const OUTPUT_LIMIT: usize = 64 * 1024;
const POD_LIMIT: usize = 100;
const PLAN_TTL: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DrainReviewRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub operation_id: u64,
    pub reference: ResourceReference,
    pub resource_version: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DrainExecuteRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub operation_id: u64,
    pub plan_id: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DrainCancel {
    pub cluster_id: String,
    pub session_id: String,
    pub operation_id: u64,
}
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DrainDisposition {
    Evict,
    Skip,
    Wait,
    Block,
}
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DrainPodReason {
    Managed,
    DaemonSet,
    Mirror,
    Terminating,
    Unmanaged,
    LocalData,
    UnsupportedController,
    ControllerUnavailable,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DrainPod {
    pub name: String,
    pub namespace: String,
    pub uid: String,
    pub resource_version: String,
    pub disposition: DrainDisposition,
    pub reason: DrainPodReason,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DrainReview {
    pub cluster_id: String,
    pub session_id: String,
    pub operation_id: u64,
    pub reference: ResourceReference,
    pub resource_version: String,
    pub plan_id: Option<String>,
    pub expires_at: Option<u64>,
    pub pods: Vec<DrainPod>,
    pub can_drain: bool,
    pub partial: bool,
}
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DrainState {
    Complete,
    Blocked,
    Partial,
    Cancelled,
    Timeout,
    Uncertain,
}
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DrainOutcomeState {
    Evicted,
    Gone,
    Pending,
    Blocked,
    Skipped,
    Uncertain,
    NotSubmitted,
}
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DrainOutcomeReason {
    DisruptionBudget,
    PermissionDenied,
    Conflict,
    Unavailable,
    Cancelled,
    Timeout,
    NewPods,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DrainOutcome {
    pub name: String,
    pub namespace: String,
    pub uid: String,
    pub state: DrainOutcomeState,
    pub reason: Option<DrainOutcomeReason>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DrainResult {
    pub cluster_id: String,
    pub session_id: String,
    pub operation_id: u64,
    pub reference: ResourceReference,
    pub cordoned: bool,
    pub state: DrainState,
    pub pods: Vec<DrainOutcome>,
}
struct DrainPlan {
    id: String,
    review_id: u64,
    request: DrainReviewRequest,
    pods: Vec<DrainPod>,
    checks: Vec<PermissionCheck>,
    epoch: AuthorizationEpoch,
    deadline: Instant,
}
#[derive(Default)]
pub(super) struct DrainSlot {
    high_water: u64,
    id: u64,
    cancel: Option<CancellationToken>,
    plan: Option<DrainPlan>,
}
impl DrainSlot {
    pub(super) fn active(&self) -> bool {
        self.cancel.is_some()
    }
    fn begin(&mut self, id: u64, cancel: CancellationToken) -> Result<()> {
        if id <= self.high_water {
            return Err(ConnectionError::StaleSession);
        }
        if self.active() {
            return Err(ConnectionError::Busy);
        }
        self.high_water = id;
        self.id = id;
        self.cancel = Some(cancel);
        Ok(())
    }
    fn cancel(&mut self, id: u64) {
        self.high_water = self.high_water.max(id);
        if self.plan.as_ref().is_some_and(|p| p.review_id == id) {
            self.plan = None;
        }
        if self.id == id
            && let Some(cancel) = &self.cancel
        {
            cancel.cancel();
        }
    }
}
struct OperationGuard<'a> {
    engine: &'a ConnectionEngine,
    identity: SessionRequest,
    id: u64,
    cancel: CancellationToken,
}
impl Drop for OperationGuard<'_> {
    fn drop(&mut self) {
        // Also releases the gate if the IPC future itself is dropped, without a task.
        if let Ok(mut state) = self.engine.session.lock()
            && let Some(session) = state
                .as_mut()
                .filter(|s| super::matches_session(s, &self.identity))
            && session.node_drain.id == self.id
        {
            if self.cancel.is_cancelled() {
                session.node_drain.plan = None;
            }
            session.node_drain.cancel = None;
        }
        self.cancel.cancel();
    }
}
fn identity(cluster: &str, session: &str, id: u64) -> Result<SessionRequest> {
    let request = SessionRequest {
        cluster_id: cluster.into(),
        session_id: session.into(),
    };
    super::validate_request(&request)?;
    super::validate_read_id(id)?;
    Ok(request)
}
fn safe_token(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && value.bytes().all(|b| (0x21..=0x7e).contains(&b))
}
fn validate(request: &DrainReviewRequest) -> Result<SessionRequest> {
    let identity = identity(
        &request.cluster_id,
        &request.session_id,
        request.operation_id,
    )?;
    validate_reference(&request.reference)?;
    if request.reference.kind != ResourceKind::Nodes
        || request.reference.namespace.is_some()
        || request.reference.uid.is_none()
        || !safe_token(&request.resource_version)
    {
        return Err(ConnectionError::InvalidInput);
    }
    Ok(identity)
}
fn bounded<T: Serialize>(output: &T) -> Result<()> {
    if serde_json::to_vec(output)
        .map_err(|_| ConnectionError::InvalidResponse)?
        .len()
        > OUTPUT_LIMIT
    {
        return Err(ConnectionError::LimitExceeded);
    }
    Ok(())
}
fn check(
    kind: ResourceKind,
    action: PermissionAction,
    namespace: Option<String>,
    name: Option<String>,
) -> PermissionCheck {
    PermissionCheck {
        kind,
        action,
        namespace,
        name,
    }
}
fn base_checks(request: &DrainReviewRequest) -> Vec<PermissionCheck> {
    vec![
        check(
            ResourceKind::Nodes,
            PermissionAction::Get,
            None,
            Some(request.reference.name.clone()),
        ),
        check(
            ResourceKind::Nodes,
            PermissionAction::Patch,
            None,
            Some(request.reference.name.clone()),
        ),
        check(ResourceKind::Pods, PermissionAction::List, None, None),
    ]
}
impl ConnectionEngine {
    fn drain_start(
        &self,
        identity: SessionRequest,
        id: u64,
    ) -> Result<(Client, OperationGuard<'_>)> {
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        let session = super::current_mut(&mut state, &identity)?;
        let client = session
            .client
            .clone()
            .ok_or(ConnectionError::NotConnected)?;
        super::validate_credentials(&client)?;
        let cancel = session.cancel.child_token();
        session.node_drain.begin(id, cancel.clone())?;
        Ok((
            client,
            OperationGuard {
                engine: self,
                identity,
                id,
                cancel,
            },
        ))
    }
    async fn drain_permissions(
        &self,
        identity: &SessionRequest,
        checks: &[PermissionCheck],
        epoch: &AuthorizationEpoch,
    ) -> Result<()> {
        epoch.validate()?;
        for chunk in checks.chunks(16) {
            if self.require_permissions(identity, chunk).await? != *epoch {
                return Err(ConnectionError::StaleSession);
            }
            epoch.validate()?;
        }
        Ok(())
    }
    pub fn node_drain_cancel(&self, request: DrainCancel) -> Result<()> {
        let identity = identity(
            &request.cluster_id,
            &request.session_id,
            request.operation_id,
        )?;
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        if let Some(session) = state
            .as_mut()
            .filter(|s| super::matches_session(s, &identity))
        {
            session.node_drain.cancel(request.operation_id);
        }
        Ok(())
    }
    pub async fn node_drain_review(&self, request: DrainReviewRequest) -> Result<DrainReview> {
        let identity = validate(&request)?;
        let (client, guard) = self.drain_start(identity.clone(), request.operation_id)?;
        {
            let mut state = self
                .session
                .lock()
                .map_err(|_| ConnectionError::StoreUnavailable)?;
            super::current_mut(&mut state, &identity)?.node_drain.plan = None;
        }
        let epoch = AuthorizationEpoch::capture(&self.authentication_generation);
        let operation = async {
            self.drain_permissions(&identity, &base_checks(&request), &epoch)
                .await?;
            let client = authorized_client(client, epoch.clone());
            let (raw, partial) = {
                let _worker = self
                    .workers
                    .try_acquire()
                    .map_err(|_| ConnectionError::Busy)?;
                node(&client, &guard.cancel, &request, true, false).await?;
                inventory(&client, &guard.cancel, &request.reference.name).await?
            };
            let mut checks = base_checks(&request);
            for value in &raw {
                let pod = pod_identity(value, &request.reference.name)?;
                if let Some(owner) = controller(value)? {
                    checks.push(check(
                        owner.reference.kind,
                        PermissionAction::Get,
                        owner.reference.namespace,
                        Some(owner.reference.name),
                    ));
                }
                checks.push(check(
                    ResourceKind::Pods,
                    PermissionAction::Get,
                    Some(pod.namespace.clone()),
                    Some(pod.name.clone()),
                ));
            }
            let mut unique_checks = Vec::with_capacity(checks.len());
            for check in checks {
                if !unique_checks.contains(&check) {
                    unique_checks.push(check);
                }
            }
            let mut checks = unique_checks;
            self.drain_permissions(&identity, &checks, &epoch).await?;
            let pods = {
                let _worker = self
                    .workers
                    .try_acquire()
                    .map_err(|_| ConnectionError::Busy)?;
                let pods =
                    classify_all(&client, &guard.cancel, &request.reference.name, &raw).await?;
                node(&client, &guard.cancel, &request, true, false).await?;
                pods
            };
            epoch.validate()?;
            let can_drain = !partial
                && pods
                    .iter()
                    .all(|p| p.disposition != DrainDisposition::Block);
            if can_drain {
                let eviction_checks: Vec<_> = pods
                    .iter()
                    .filter(|pod| pod.disposition == DrainDisposition::Evict)
                    .map(|pod| {
                        check(
                            ResourceKind::Pods,
                            PermissionAction::Evict,
                            Some(pod.namespace.clone()),
                            Some(pod.name.clone()),
                        )
                    })
                    .collect();
                self.drain_permissions(&identity, &eviction_checks, &epoch)
                    .await?;
                checks.extend(eviction_checks);
            }
            let mut response = DrainReview {
                cluster_id: request.cluster_id.clone(),
                session_id: request.session_id.clone(),
                operation_id: request.operation_id,
                reference: request.reference.clone(),
                resource_version: request.resource_version.clone(),
                plan_id: None,
                expires_at: None,
                pods: pods.clone(),
                can_drain,
                partial,
            };
            let plan = if can_drain {
                let mut bytes = [0u8; 16];
                getrandom::fill(&mut bytes).map_err(|_| ConnectionError::StoreUnavailable)?;
                let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
                response.plan_id = Some(id.clone());
                response.expires_at = Some(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_err(|_| ConnectionError::StoreUnavailable)?
                        .as_millis() as u64
                        + 120_000,
                );
                Some(DrainPlan {
                    id,
                    review_id: request.operation_id,
                    request: request.clone(),
                    pods,
                    checks,
                    epoch: epoch.clone(),
                    deadline: Instant::now() + PLAN_TTL,
                })
            } else {
                None
            };
            bounded(&response)?;
            let mut state = self
                .session
                .lock()
                .map_err(|_| ConnectionError::StoreUnavailable)?;
            let session = super::current_mut(&mut state, &identity)?;
            if guard.cancel.is_cancelled() || session.node_drain.id != request.operation_id {
                return Err(ConnectionError::StaleSession);
            }
            epoch.validate()?;
            session.node_drain.plan = plan;
            Ok(response)
        };
        tokio::select! { biased;
            _ = guard.cancel.cancelled() => Err(ConnectionError::StaleSession),
            result = tokio::time::timeout(Duration::from_secs(15), operation) => result.unwrap_or(Err(ConnectionError::Timeout)),
        }
    }
    pub async fn node_drain_execute(&self, request: DrainExecuteRequest) -> Result<DrainResult> {
        let identity = identity(
            &request.cluster_id,
            &request.session_id,
            request.operation_id,
        )?;
        let (client, guard) = self.drain_start(identity.clone(), request.operation_id)?;
        // Taking the server-owned plan precedes every confirmation check, including expiry.
        let plan = {
            let mut state = self
                .session
                .lock()
                .map_err(|_| ConnectionError::StoreUnavailable)?;
            super::current_mut(&mut state, &identity)?
                .node_drain
                .plan
                .take()
                .ok_or(ConnectionError::StaleSession)?
        };
        if plan.id != request.plan_id
            || Instant::now() >= plan.deadline
            || plan.request.cluster_id != request.cluster_id
            || plan.request.session_id != request.session_id
        {
            return Err(ConnectionError::StaleSession);
        }
        plan.epoch.validate()?;
        let mut result = DrainResult {
            cluster_id: request.cluster_id,
            session_id: request.session_id,
            operation_id: request.operation_id,
            reference: plan.request.reference.clone(),
            cordoned: false,
            state: DrainState::Blocked,
            pods: plan
                .pods
                .iter()
                .map(|p| DrainOutcome {
                    name: p.name.clone(),
                    namespace: p.namespace.clone(),
                    uid: p.uid.clone(),
                    state: if p.disposition == DrainDisposition::Skip {
                        DrainOutcomeState::Skipped
                    } else {
                        DrainOutcomeState::NotSubmitted
                    },
                    reason: None,
                })
                .collect(),
        };
        let mut inflight = false;
        let operation = async {
            self.drain_permissions(&identity, &plan.checks, &plan.epoch)
                .await?;
            let _worker = self
                .workers
                .try_acquire()
                .map_err(|_| ConnectionError::Busy)?;
            let client = authorized_client(client, plan.epoch.clone());
            execute(&client, &guard.cancel, &plan, &mut result, &mut inflight).await
        };
        let finish = tokio::select! { biased;
            _ = guard.cancel.cancelled() => Err(ConnectionError::StaleSession),
            value = tokio::time::timeout(PLAN_TTL, operation) => value.unwrap_or(Err(ConnectionError::Timeout)),
        };
        if let Err(error) = finish {
            let reason = outcome_reason(error);
            result.state = if inflight {
                DrainState::Uncertain
            } else if error == ConnectionError::StaleSession {
                DrainState::Cancelled
            } else if error == ConnectionError::Timeout {
                DrainState::Timeout
            } else if result.cordoned {
                DrainState::Partial
            } else {
                DrainState::Blocked
            };
            for pod in &mut result.pods {
                if matches!(
                    pod.state,
                    DrainOutcomeState::NotSubmitted
                        | DrainOutcomeState::Pending
                        | DrainOutcomeState::Uncertain
                ) {
                    pod.reason = Some(reason);
                }
            }
        }
        bounded(&result)?;
        Ok(result)
    }
}
fn string_at<'a>(value: &'a Value, path: &str) -> Result<&'a str> {
    value
        .pointer(path)
        .and_then(Value::as_str)
        .filter(|v| safe_token(v))
        .ok_or(ConnectionError::InvalidResponse)
}
fn node_identity(
    value: &Value,
    request: &DrainReviewRequest,
    exact_version: bool,
    cordoned: bool,
) -> Result<()> {
    if value["apiVersion"] != "v1"
        || value["kind"] != "Node"
        || string_at(value, "/metadata/name")? != request.reference.name
        || value
            .pointer("/metadata/namespace")
            .is_some_and(|v| !v.is_null())
        || string_at(value, "/metadata/uid")?
            != request
                .reference
                .uid
                .as_deref()
                .ok_or(ConnectionError::InvalidInput)?
        || (exact_version
            && string_at(value, "/metadata/resourceVersion")? != request.resource_version)
    {
        return Err(ConnectionError::Conflict);
    }
    string_at(value, "/metadata/resourceVersion")?;
    let spec = value
        .get("spec")
        .and_then(Value::as_object)
        .ok_or(ConnectionError::InvalidResponse)?;
    if spec.get("unschedulable").is_some_and(|v| !v.is_boolean()) {
        return Err(ConnectionError::InvalidResponse);
    }
    if cordoned && spec.get("unschedulable").and_then(Value::as_bool) != Some(true) {
        return Err(ConnectionError::Conflict);
    }
    Ok(())
}
async fn node(
    client: &Client,
    cancel: &CancellationToken,
    request: &DrainReviewRequest,
    exact: bool,
    cordoned: bool,
) -> Result<Value> {
    let value = send(
        client,
        cancel,
        Method::GET,
        &named_path(&request.reference)?,
        None,
    )
    .await?
    .ok_or(ConnectionError::NotFound)?;
    node_identity(&value, request, exact, cordoned)?;
    Ok(value)
}
async fn inventory(
    client: &Client,
    cancel: &CancellationToken,
    node: &str,
) -> Result<(Vec<Value>, bool)> {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("fieldSelector", &format!("spec.nodeName={node}"))
        .append_pair("limit", "100")
        .finish();
    let value = send(
        client,
        cancel,
        Method::GET,
        &format!("/api/v1/pods?{query}"),
        None,
    )
    .await?
    .ok_or(ConnectionError::InvalidResponse)?;
    if value["apiVersion"] != "v1" || value["kind"] != "PodList" {
        return Err(ConnectionError::InvalidResponse);
    }
    let items = value["items"]
        .as_array()
        .ok_or(ConnectionError::InvalidResponse)?;
    if items.len() > POD_LIMIT {
        return Err(ConnectionError::LimitExceeded);
    }
    let continuation = match value.pointer("/metadata/continue") {
        None => false,
        Some(Value::String(v)) => !v.is_empty(),
        _ => return Err(ConnectionError::InvalidResponse),
    };
    let mut items = items.clone();
    let mut names = BTreeSet::new();
    for value in &mut items {
        let object = value
            .as_object_mut()
            .ok_or(ConnectionError::InvalidResponse)?;
        if object
            .get("apiVersion")
            .is_some_and(|v| v.as_str() != Some("v1"))
            || object
                .get("kind")
                .is_some_and(|v| v.as_str() != Some("Pod"))
        {
            return Err(ConnectionError::InvalidResponse);
        }
        object.entry("apiVersion").or_insert_with(|| json!("v1"));
        object.entry("kind").or_insert_with(|| json!("Pod"));
        let pod = pod_identity(value, node)?;
        if !names.insert((pod.namespace, pod.name)) {
            return Err(ConnectionError::InvalidResponse);
        }
    }
    // Never follow a continuation: a truncated bound-node view cannot authorize writes.
    Ok((items, continuation))
}
fn pod_identity(value: &Value, node: &str) -> Result<DrainPod> {
    if value["apiVersion"] != "v1"
        || value["kind"] != "Pod"
        || string_at(value, "/spec/nodeName")? != node
    {
        return Err(ConnectionError::InvalidResponse);
    }
    let name = string_at(value, "/metadata/name")?.to_owned();
    let namespace = string_at(value, "/metadata/namespace")?.to_owned();
    if !super::resources::valid_name(&name) || !super::transport::valid_namespace(&namespace) {
        return Err(ConnectionError::InvalidResponse);
    }
    Ok(DrainPod {
        name,
        namespace,
        uid: string_at(value, "/metadata/uid")?.into(),
        resource_version: string_at(value, "/metadata/resourceVersion")?.into(),
        disposition: DrainDisposition::Block,
        reason: DrainPodReason::Unmanaged,
    })
}
struct Controller {
    reference: ResourceReference,
    api: String,
    kind: String,
}
fn controller(value: &Value) -> Result<Option<Controller>> {
    let Some(owners) = value.pointer("/metadata/ownerReferences") else {
        return Ok(None);
    };
    let owners = owners.as_array().ok_or(ConnectionError::InvalidResponse)?;
    if owners.len() > 32 {
        return Err(ConnectionError::InvalidResponse);
    }
    let mut selected = None;
    for owner in owners {
        if !owner.is_object() || owner.get("controller").is_some_and(|v| !v.is_boolean()) {
            return Err(ConnectionError::InvalidResponse);
        }
        if owner["controller"] != true {
            continue;
        }
        if selected.is_some() {
            return Err(ConnectionError::InvalidResponse);
        }
        let api = string_at(owner, "/apiVersion")?;
        let kind = string_at(owner, "/kind")?;
        let resource = match (api, kind) {
            ("apps/v1", "ReplicaSet") => ResourceKind::ReplicaSets,
            ("apps/v1", "StatefulSet") => ResourceKind::StatefulSets,
            ("apps/v1", "DaemonSet") => ResourceKind::DaemonSets,
            ("v1", "ReplicationController") => ResourceKind::ReplicationControllers,
            ("batch/v1", "Job") => ResourceKind::Jobs,
            _ => return Ok(None),
        };
        let reference = ResourceReference {
            kind: resource,
            name: string_at(owner, "/name")?.into(),
            namespace: Some(string_at(value, "/metadata/namespace")?.into()),
            uid: Some(string_at(owner, "/uid")?.into()),
        };
        validate_reference(&reference).map_err(|_| ConnectionError::InvalidResponse)?;
        selected = Some(Controller {
            reference,
            api: api.into(),
            kind: kind.into(),
        });
    }
    Ok(selected)
}
fn local_data(value: &Value) -> Result<bool> {
    let Some(volumes) = value.pointer("/spec/volumes") else {
        return Ok(false);
    };
    let volumes = volumes.as_array().ok_or(ConnectionError::InvalidResponse)?;
    for volume in volumes {
        let object = volume.as_object().ok_or(ConnectionError::InvalidResponse)?;
        if !object
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(super::resources::valid_name)
        {
            return Err(ConnectionError::InvalidResponse);
        }
        if object.contains_key("emptyDir") || object.contains_key("hostPath") {
            return Ok(true);
        }
        // Only known non-host sources qualify. PVC topology is still not a disk-safety guarantee.
        if object.len() != 2
            || !object.iter().any(|(k, v)| {
                k != "name"
                    && v.is_object()
                    && matches!(
                        k.as_str(),
                        "persistentVolumeClaim"
                            | "secret"
                            | "configMap"
                            | "projected"
                            | "downwardAPI"
                            | "csi"
                            | "ephemeral"
                            | "nfs"
                    )
            })
        {
            return Ok(true);
        }
    }
    Ok(false)
}
async fn classify(
    client: &Client,
    cancel: &CancellationToken,
    node: &str,
    value: &Value,
) -> Result<DrainPod> {
    let mut pod = pod_identity(value, node)?;
    let terminating = match value.pointer("/metadata/deletionTimestamp") {
        None | Some(Value::Null) => false,
        Some(Value::String(v)) if !v.is_empty() && v.parse::<jiff::Timestamp>().is_ok() => true,
        _ => return Err(ConnectionError::InvalidResponse),
    };
    let mirror = match value.pointer("/metadata/annotations") {
        None => false,
        Some(Value::Object(v)) if v.values().all(Value::is_string) => v
            .get("kubernetes.io/config.mirror")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty()),
        _ => return Err(ConnectionError::InvalidResponse),
    };
    if mirror {
        pod.disposition = DrainDisposition::Skip;
        pod.reason = DrainPodReason::Mirror;
        return Ok(pod);
    }
    let Some(owner) = controller(value)? else {
        pod.reason = if value
            .pointer("/metadata/ownerReferences")
            .and_then(Value::as_array)
            .is_some_and(|owners| owners.iter().any(|o| o["controller"] == true))
        {
            DrainPodReason::UnsupportedController
        } else {
            DrainPodReason::Unmanaged
        };
        return Ok(pod);
    };
    let live = send(
        client,
        cancel,
        Method::GET,
        &named_path(&owner.reference)?,
        None,
    )
    .await;
    let live = match live {
        Ok(Some(live)) => live,
        Ok(None) | Err(ConnectionError::NotFound | ConnectionError::ConnectionFailed) => {
            pod.reason = DrainPodReason::ControllerUnavailable;
            return Ok(pod);
        }
        Err(error) => return Err(error),
    };
    if live["apiVersion"] != owner.api
        || live["kind"] != owner.kind
        || string_at(&live, "/metadata/name")? != owner.reference.name
        || Some(string_at(&live, "/metadata/namespace")?) != owner.reference.namespace.as_deref()
        || Some(string_at(&live, "/metadata/uid")?) != owner.reference.uid.as_deref()
        || live
            .pointer("/metadata/deletionTimestamp")
            .is_some_and(|v| !v.is_null())
    {
        pod.reason = DrainPodReason::ControllerUnavailable;
        return Ok(pod);
    }
    if owner.reference.kind == ResourceKind::DaemonSets {
        pod.disposition = DrainDisposition::Skip;
        pod.reason = DrainPodReason::DaemonSet;
    } else if local_data(value)? {
        pod.reason = DrainPodReason::LocalData;
    } else if terminating {
        pod.disposition = DrainDisposition::Wait;
        pod.reason = DrainPodReason::Terminating;
    } else {
        pod.disposition = DrainDisposition::Evict;
        pod.reason = DrainPodReason::Managed;
    }
    Ok(pod)
}
async fn classify_all(
    client: &Client,
    cancel: &CancellationToken,
    node: &str,
    values: &[Value],
) -> Result<Vec<DrainPod>> {
    let mut pods = Vec::with_capacity(values.len());
    for value in values {
        pods.push(classify(client, cancel, node, value).await?);
    }
    pods.sort_by(|a, b| (&a.namespace, &a.name).cmp(&(&b.namespace, &b.name)));
    Ok(pods)
}
fn pod_path(pod: &DrainPod) -> String {
    format!("/api/v1/namespaces/{}/pods/{}", pod.namespace, pod.name)
}
fn outcome_reason(error: ConnectionError) -> DrainOutcomeReason {
    match error {
        ConnectionError::Busy => DrainOutcomeReason::DisruptionBudget,
        ConnectionError::PermissionDenied | ConnectionError::AuthFailed => {
            DrainOutcomeReason::PermissionDenied
        }
        ConnectionError::Conflict => DrainOutcomeReason::Conflict,
        ConnectionError::StaleSession => DrainOutcomeReason::Cancelled,
        ConnectionError::Timeout => DrainOutcomeReason::Timeout,
        _ => DrainOutcomeReason::Unavailable,
    }
}
async fn execute(
    client: &Client,
    cancel: &CancellationToken,
    plan: &DrainPlan,
    result: &mut DrainResult,
    inflight: &mut bool,
) -> Result<()> {
    let request = &plan.request;
    node(client, cancel, request, true, false).await?;
    let (values, partial) = inventory(client, cancel, &request.reference.name).await?;
    let current = classify_all(client, cancel, &request.reference.name, &values).await?;
    if partial
        || current != plan.pods
        || current
            .iter()
            .any(|p| p.disposition == DrainDisposition::Block)
    {
        for outcome in &mut result.pods {
            if outcome.state != DrainOutcomeState::Skipped {
                outcome.state = DrainOutcomeState::Blocked;
                outcome.reason = Some(DrainOutcomeReason::Conflict);
            }
        }
        result.state = DrainState::Blocked;
        return Ok(());
    }
    for pod in &plan.pods {
        let fresh = send(client, cancel, Method::GET, &pod_path(pod), None)
            .await?
            .ok_or(ConnectionError::Conflict)?;
        let fresh = pod_identity(&fresh, &request.reference.name)?;
        if fresh.uid != pod.uid || fresh.resource_version != pod.resource_version {
            return Err(ConnectionError::Conflict);
        }
    }
    // Final optimistic Node guard follows the complete Pod/controller preflight.
    node(client, cancel, request, true, false).await?;
    let patch = json!([
        {"op":"test","path":"/metadata/uid","value":request.reference.uid},
        {"op":"test","path":"/metadata/resourceVersion","value":request.resource_version},
        {"op":"add","path":"/spec/unschedulable","value":true}
    ]);
    plan.epoch.validate()?;
    if cancel.is_cancelled() {
        return Err(ConnectionError::StaleSession);
    }
    *inflight = true;
    let response = send(
        client,
        cancel,
        Method::PATCH,
        &named_path(&request.reference)?,
        Some(patch),
    )
    .await;
    if response.as_ref().is_err_and(|e| {
        matches!(
            e,
            ConnectionError::PermissionDenied
                | ConnectionError::Conflict
                | ConnectionError::InvalidInput
                | ConnectionError::AuthFailed
        )
    }) {
        *inflight = false;
    }
    let response = response?.ok_or(ConnectionError::InvalidResponse)?;
    node_identity(&response, request, false, true)?;
    *inflight = false;
    result.cordoned = true;
    for (index, pod) in plan.pods.iter().enumerate() {
        if pod.disposition == DrainDisposition::Skip {
            continue;
        }
        node(client, cancel, request, false, true).await?;
        let current = send(client, cancel, Method::GET, &pod_path(pod), None).await?;
        if let Some(current) = current {
            let fresh = pod_identity(&current, &request.reference.name)?;
            if fresh.uid != pod.uid {
                result.pods[index].state = DrainOutcomeState::Gone;
                continue;
            }
            if fresh.resource_version != pod.resource_version {
                return Err(ConnectionError::Conflict);
            }
            if pod.disposition == DrainDisposition::Evict {
                // No grace override, force flag, DELETE fallback or write retry.
                let body = json!({"apiVersion":"policy/v1","kind":"Eviction", "metadata":{"name":pod.name,"namespace":pod.namespace},
                    "deleteOptions":{"preconditions":{"uid":pod.uid,"resourceVersion":pod.resource_version}}});
                plan.epoch.validate()?;
                if cancel.is_cancelled() {
                    return Err(ConnectionError::StaleSession);
                }
                result.pods[index].state = DrainOutcomeState::Uncertain;
                *inflight = true;
                let sent = send(
                    client,
                    cancel,
                    Method::POST,
                    &format!("{}/eviction", pod_path(pod)),
                    Some(body),
                )
                .await;
                match sent {
                    Ok(_) => {
                        *inflight = false;
                        result.pods[index].state = DrainOutcomeState::Pending;
                    }
                    Err(error) => {
                        if matches!(
                            error,
                            ConnectionError::Busy
                                | ConnectionError::PermissionDenied
                                | ConnectionError::Conflict
                                | ConnectionError::InvalidInput
                                | ConnectionError::AuthFailed
                        ) {
                            *inflight = false;
                            result.pods[index].state = DrainOutcomeState::Blocked;
                        }
                        result.pods[index].reason = Some(outcome_reason(error));
                        return Err(error);
                    }
                }
            } else {
                result.pods[index].state = DrainOutcomeState::Pending;
            }
            loop {
                node(client, cancel, request, false, true).await?;
                match send(client, cancel, Method::GET, &pod_path(pod), None).await? {
                    None => {
                        result.pods[index].state = if pod.disposition == DrainDisposition::Evict {
                            DrainOutcomeState::Evicted
                        } else {
                            DrainOutcomeState::Gone
                        };
                        break;
                    }
                    Some(current) => {
                        let current = pod_identity(&current, &request.reference.name)?;
                        if current.uid != pod.uid {
                            result.pods[index].state = DrainOutcomeState::Gone;
                            break;
                        }
                    }
                }
                tokio::select! { biased;
                    _ = cancel.cancelled() => return Err(ConnectionError::StaleSession),
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {},
                }
            }
        } else {
            result.pods[index].state = DrainOutcomeState::Gone;
        }
    }
    node(client, cancel, request, false, true).await?;
    let (remaining, partial) = inventory(client, cancel, &request.reference.name).await?;
    let mut unexpected = false;
    for value in remaining {
        let current = pod_identity(&value, &request.reference.name)?;
        let skip = plan.pods.iter().find(|pod| {
            pod.namespace == current.namespace
                && pod.name == current.name
                && pod.uid == current.uid
                && pod.resource_version == current.resource_version
                && pod.disposition == DrainDisposition::Skip
        });
        if skip.is_none() {
            unexpected = true;
            if let Some(outcome) = result.pods.iter_mut().find(|p| {
                p.namespace == current.namespace && p.name == current.name && p.uid == current.uid
            }) {
                outcome.state = DrainOutcomeState::Blocked;
                outcome.reason = Some(DrainOutcomeReason::NewPods);
            } else if result.pods.len() < POD_LIMIT {
                result.pods.push(DrainOutcome {
                    name: current.name,
                    namespace: current.namespace,
                    uid: current.uid,
                    state: DrainOutcomeState::Blocked,
                    reason: Some(DrainOutcomeReason::NewPods),
                });
            }
        }
    }
    node(client, cancel, request, false, true).await?;
    plan.epoch.validate()?;
    result.state = if partial || unexpected {
        DrainState::Partial
    } else {
        DrainState::Complete
    };
    Ok(())
}
async fn send(
    client: &Client,
    cancel: &CancellationToken,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<Option<Value>> {
    let operation = async {
        super::validate_credentials(client)?;
        if cancel.is_cancelled() {
            return Err(ConnectionError::StaleSession);
        }
        let bytes = body
            .map(|v| serde_json::to_vec(&v))
            .transpose()
            .map_err(|_| ConnectionError::InvalidInput)?
            .unwrap_or_default();
        let post = method == Method::POST;
        let content_type = if method == Method::PATCH {
            "application/json-patch+json"
        } else {
            "application/json"
        };
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("Accept", "application/json")
            .header("Content-Type", content_type)
            .body(Body::from(bytes))
            .map_err(|_| ConnectionError::InvalidInput)?;
        request.extensions_mut().insert(cancel.clone());
        let mut response = client
            .send(request)
            .await
            .map_err(super::pods::map_pod_error)?;
        match response.status().as_u16() {
            200..=299 => {}
            404 => return Ok(None),
            400 | 422 => return Err(ConnectionError::InvalidInput),
            401 => return Err(ConnectionError::AuthFailed),
            403 => return Err(ConnectionError::PermissionDenied),
            409 => return Err(ConnectionError::Conflict),
            429 => return Err(ConnectionError::Busy),
            408 | 504 => return Err(ConnectionError::Timeout),
            _ => return Err(ConnectionError::ConnectionFailed),
        }
        if response
            .headers()
            .get("content-length")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.parse::<usize>().ok())
            .is_some_and(|n| n > INPUT_LIMIT)
        {
            return Err(ConnectionError::LimitExceeded);
        }
        let mut bytes = Vec::new();
        while let Some(frame) = response.body_mut().frame().await {
            let frame = frame.map_err(|_| ConnectionError::Network)?;
            if let Some(data) = frame.data_ref() {
                if data.len() > INPUT_LIMIT.saturating_sub(bytes.len()) {
                    return Err(ConnectionError::LimitExceeded);
                }
                bytes.extend_from_slice(data);
            }
        }
        if post {
            return Ok(Some(Value::Null));
        }
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| ConnectionError::InvalidResponse)
    };
    tokio::select! { biased;
        _ = cancel.cancelled() => Err(ConnectionError::StaleSession),
        result = tokio::time::timeout(Duration::from_secs(15), operation) => result.unwrap_or(Err(ConnectionError::Timeout)),
    }
}

#[cfg(test)]
mod tests;
