use super::{
    ConnectionEngine, ConnectionError, ResourceKind, Result, SessionRequest, current_mut,
    resources, transport, validate_read_id, validate_request,
};
use futures::{StreamExt, TryStreamExt, stream};
use http::Request;
use http_body_util::BodyExt;
use kube::{Client, client::Body};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio_util::sync::CancellationToken;

pub(super) type AuthenticationGeneration = Arc<AtomicU64>;
static REVIEW_WORKERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
const REVIEW_PATH: &str = "/apis/authorization.k8s.io/v1/selfsubjectaccessreviews";

pub(super) fn invalidate_authentication(generation: &AuthenticationGeneration) {
    generation.fetch_add(1, Ordering::SeqCst);
}

fn valid_resource_name(kind: ResourceKind, name: &str) -> bool {
    if matches!(
        kind,
        ResourceKind::Roles
            | ResourceKind::ClusterRoles
            | ResourceKind::RoleBindings
            | ResourceKind::ClusterRoleBindings
    ) {
        !name.is_empty()
            && name.len() <= 253
            && !matches!(name, "." | "..")
            && name.chars().all(|c| {
                !c.is_control() && !c.is_whitespace() && !matches!(c, '/' | '%' | '?' | '#')
            })
    } else {
        resources::valid_name(name)
    }
}
#[derive(Debug, Clone)]
pub(super) struct AuthorizationEpoch {
    generation: AuthenticationGeneration,
    captured: u64,
}

impl PartialEq for AuthorizationEpoch {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.generation, &other.generation) && self.captured == other.captured
    }
}
impl Eq for AuthorizationEpoch {}

impl AuthorizationEpoch {
    pub(super) fn capture(generation: &AuthenticationGeneration) -> Self {
        Self {
            generation: generation.clone(),
            captured: generation.load(Ordering::SeqCst),
        }
    }

    pub(super) fn validate(&self) -> Result<()> {
        if self.captured == self.generation.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(ConnectionError::StaleSession)
        }
    }
}

pub(super) fn authorized_client(client: Client, epoch: AuthorizationEpoch) -> Client {
    let namespace = client.default_namespace().to_owned();
    Client::new(
        tower::service_fn(move |mut request: Request<Body>| {
            let client = client.clone();
            let epoch = epoch.clone();
            async move {
                epoch
                    .validate()
                    .map_err(super::renewal::authorization_error)?;
                request.extensions_mut().insert(epoch);
                client
                    .send(request)
                    .await
                    .map_err(|error| -> tower::BoxError { error.into() })
            }
        }),
        namespace,
    )
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum PermissionAction {
    List,
    Get,
    Create,
    Update,
    Patch,
    Delete,
    Watch,
    Logs,
    Exec,
    Attach,
    PortForward,
    Evict,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PermissionCheck {
    pub kind: ResourceKind,
    pub action: PermissionAction,
    pub namespace: Option<String>,
    pub name: Option<String>,
}
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub read_id: u64,
    pub checks: Vec<PermissionCheck>,
}
impl ReviewRequest {
    fn session(&self) -> SessionRequest {
        SessionRequest {
            cluster_id: self.cluster_id.clone(),
            session_id: self.session_id.clone(),
        }
    }
    fn validate(&self) -> Result<()> {
        validate_request(&self.session())?;
        validate_read_id(self.read_id)?;
        validate_checks(&self.checks)
    }
}
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PermissionDecision {
    Allowed,
    Denied,
    Unknown,
}
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PermissionReason {
    #[serde(rename = "reviewDenied")]
    Denied,
    #[serde(rename = "reviewUnavailable")]
    Unavailable,
    #[serde(rename = "reviewFailed")]
    Failed,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionItem {
    pub check: PermissionCheck,
    pub decision: PermissionDecision,
    pub reason: Option<PermissionReason>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewResponse {
    pub cluster_id: String,
    pub session_id: String,
    pub read_id: u64,
    pub authorization_epoch: String,
    pub revision: u64,
    pub checked_at: u64,
    pub expires_at: u64,
    pub items: Vec<PermissionItem>,
}
struct Cached {
    item: PermissionItem,
    expires: u64,
    deadline: tokio::time::Instant,
    used: u64,
}
#[derive(Default)]
pub(super) struct PermissionSession {
    epoch: u64,
    revision: u64,
    clock: u64,
    cache: HashMap<PermissionCheck, Cached>,
    slot: Option<(ReviewRequest, Option<CancellationToken>)>,
}
impl PermissionSession {
    fn synchronize(&mut self, generation: &AuthenticationGeneration) {
        let epoch = generation.load(Ordering::SeqCst);
        if self.epoch != epoch {
            self.epoch = epoch;
            self.cache.clear();
        }
    }
}
fn validate_checks(checks: &[PermissionCheck]) -> Result<()> {
    if checks.is_empty() || checks.len() > 32 {
        return Err(ConnectionError::InvalidInput);
    }
    for check in checks {
        requirements(check)?;
    }
    Ok(())
}

/// Exact ResourceAttributes for the HTTP operation, including prerequisite Pod GETs.
pub(crate) fn requirements(check: &PermissionCheck) -> Result<Vec<Value>> {
    use PermissionAction::*;
    let scoped = resources::namespaced(check.kind);
    if check
        .namespace
        .as_deref()
        .is_some_and(|ns| !scoped || !transport::valid_namespace(ns))
        || check
            .name
            .as_deref()
            .is_some_and(|name| !valid_resource_name(check.kind, name))
    {
        return Err(ConnectionError::InvalidInput);
    }
    let subresource = match check.action {
        Logs => Some("log"),
        Exec => Some("exec"),
        Attach => Some("attach"),
        PortForward => Some("portforward"),
        Evict => Some("eviction"),
        _ => None,
    };
    if subresource.is_some() && (check.kind != ResourceKind::Pods || check.name.is_none())
        || matches!(check.action, List | Watch) && check.name.is_some()
        || scoped && check.namespace.is_none() && !matches!(check.action, List | Watch)
    {
        return Err(ConnectionError::InvalidInput);
    }
    let (path, resource) = transport::endpoint(check.kind);
    let (group, version) = path
        .strip_prefix("/apis/")
        .and_then(|s| s.split_once('/'))
        .unwrap_or(("", "v1"));
    let verb = match check.action {
        List => "list",
        Get | Logs | Exec | Attach | PortForward | Evict => "get",
        Create => "create",
        Update => "update",
        Patch => "patch",
        Delete => "delete",
        Watch => "watch",
    };
    let mut attributes = json!({"group":group,"version":version,"resource":resource,"verb":verb});
    if let Some(namespace) = &check.namespace {
        attributes["namespace"] = json!(namespace);
    }
    if let Some(name) = &check.name {
        attributes["name"] = json!(name);
    }
    if let Some(subresource) = subresource {
        let prerequisite = attributes.clone();
        attributes["subresource"] = json!(subresource);
        if check.action == Evict {
            attributes["verb"] = json!("create");
        }
        Ok(vec![prerequisite, attributes])
    } else {
        Ok(vec![attributes])
    }
}

impl ConnectionEngine {
    pub(crate) fn reject_known_denied_read(
        &self,
        request: &SessionRequest,
        check: &PermissionCheck,
    ) -> Result<()> {
        requirements(check)?;
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        let permissions = &mut current_mut(&mut state, request)?.permissions;
        permissions.synchronize(&self.authentication_generation);
        if permissions.cache.get(check).is_some_and(|entry| {
            entry.item.decision == PermissionDecision::Denied
                && entry.expires > now()
                && entry.deadline > tokio::time::Instant::now()
        }) {
            Err(ConnectionError::PermissionDenied)
        } else {
            Ok(())
        }
    }

    pub fn cancel_permissions(&self, request: ReviewRequest) -> Result<()> {
        request.validate()?;
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        if let Some(session) = state
            .as_mut()
            .filter(|s| super::matches_session(s, &request.session()))
        {
            let slot = &mut session.permissions.slot;
            if let Some((old, cancel)) = slot {
                if old.read_id == request.read_id && old != &request {
                    return Err(ConnectionError::InvalidInput);
                }
                if old.read_id > request.read_id {
                    return Ok(());
                }
                if let Some(cancel) = cancel.take() {
                    cancel.cancel();
                }
            }
            *slot = Some((request, None));
        }
        Ok(())
    }
    pub(crate) fn invalidate_permissions(&self, request: &SessionRequest) -> Result<()> {
        validate_request(request)?;
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        let session = current_mut(&mut state, request)?;
        invalidate_authentication(&self.authentication_generation);
        session
            .permissions
            .synchronize(&self.authentication_generation);
        if let Some((_, Some(cancel))) = &session.permissions.slot {
            cancel.cancel();
        }
        Ok(())
    }
    pub fn refresh_permissions(&self, request: SessionRequest) -> Result<()> {
        self.invalidate_permissions(&request)
    }
    pub async fn review_permissions(&self, request: ReviewRequest) -> Result<ReviewResponse> {
        request.validate()?;
        let identity = request.session();
        let (client, cancel, epoch) = {
            let mut state = self
                .session
                .lock()
                .map_err(|_| ConnectionError::StoreUnavailable)?;
            let session = current_mut(&mut state, &identity)?;
            let client = session
                .client
                .clone()
                .ok_or(ConnectionError::NotConnected)?;
            super::validate_credentials(&client)?;
            let permissions = &mut session.permissions;
            permissions.synchronize(&self.authentication_generation);
            if let Some((old, old_cancel)) = &mut permissions.slot {
                if old.read_id == request.read_id && old != &request {
                    return Err(ConnectionError::InvalidInput);
                }
                if old.read_id >= request.read_id {
                    return Err(ConnectionError::StaleSession);
                }
                if let Some(cancel) = old_cancel.take() {
                    cancel.cancel();
                }
            }
            let cancel = session.cancel.child_token();
            permissions.slot = Some((request.clone(), Some(cancel.clone())));
            (client, cancel, permissions.epoch)
        };
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let _worker = tokio::select! { biased; _ = cancel.cancelled() => return Err(ConnectionError::StaleSession), permit = tokio::time::timeout_at(deadline, self.workers.acquire()) => permit.map_err(|_| ConnectionError::Timeout)?.map_err(|_| ConnectionError::Busy)? };
        let mut expires_at = now() + 30_000;
        let mut expires_deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let mut reviewed = stream::iter(request.checks.clone().into_iter().enumerate())
            .map(|(index, check)| {
                let client = &client;
                let identity = &identity;
                let cancel = &cancel;
                async move {
                    let check = &check;
            let cached = {
                let mut state = self
                    .session
                    .lock()
                    .map_err(|_| ConnectionError::StoreUnavailable)?;
                let permissions = &mut current_mut(&mut state, identity)?.permissions;
                permissions.synchronize(&self.authentication_generation);
                if permissions.epoch != epoch {
                    return Err(ConnectionError::StaleSession);
                }
                permissions.clock += 1;
                let clock = permissions.clock;
                permissions
                    .cache
                    .get_mut(check)
                    .filter(|entry| {
                        entry.expires > now() && entry.deadline > tokio::time::Instant::now()
                    })
                    .map(|entry| {
                        entry.used = clock;
                        (entry.item.clone(), entry.expires, entry.deadline)
                    })
            };
            let (item, expiry, expiry_deadline) = if let Some(cached) = cached {
                cached
            } else {
                let item = tokio::select! { biased; _ = cancel.cancelled() => return Err(ConnectionError::StaleSession), result = tokio::time::timeout_at(deadline, review_check(client, check, cancel)) => result.unwrap_or_else(|_| unknown(check, PermissionReason::Unavailable)) };
                let ttl = match item.decision {
                    PermissionDecision::Allowed => 30_000,
                    PermissionDecision::Denied => 10_000,
                    PermissionDecision::Unknown => 5_000,
                };
                let expiry = now() + ttl;
                let expiry_deadline = tokio::time::Instant::now() + Duration::from_millis(ttl);
                let mut state = self
                    .session
                    .lock()
                    .map_err(|_| ConnectionError::StoreUnavailable)?;
                let permissions = &mut current_mut(&mut state, identity)?.permissions;
                if self.authentication_generation.load(Ordering::SeqCst) != epoch
                    || cancel.is_cancelled()
                {
                    return Err(ConnectionError::StaleSession);
                }
                if permissions.cache.len() >= 1024
                    && let Some(key) = permissions
                        .cache
                        .iter()
                        .min_by_key(|(_, entry)| entry.used)
                        .map(|(key, _)| key.clone())
                {
                    permissions.cache.remove(&key);
                }
                permissions.clock += 1;
                permissions.cache.insert(
                    check.clone(),
                    Cached {
                        item: item.clone(),
                        expires: expiry,
                        deadline: expiry_deadline,
                        used: permissions.clock,
                    },
                );
                (item, expiry, expiry_deadline)
            };
                    Ok::<_, ConnectionError>((index, item, expiry, expiry_deadline))
                }
            })
            .buffer_unordered(2)
            .try_collect::<Vec<_>>()
            .await?;
        reviewed.sort_unstable_by_key(|(index, ..)| *index);
        let mut items = Vec::with_capacity(reviewed.len());
        for (_, item, expiry, expiry_deadline) in reviewed {
            expires_at = expires_at.min(expiry);
            expires_deadline = expires_deadline.min(expiry_deadline);
            items.push(item);
        }
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        let permissions = &mut current_mut(&mut state, &identity)?.permissions;
        if self.authentication_generation.load(Ordering::SeqCst) != epoch
            || cancel.is_cancelled()
            || expires_at <= now()
            || expires_deadline <= tokio::time::Instant::now()
            || permissions
                .slot
                .as_ref()
                .is_none_or(|(active, _)| active != &request)
        {
            return Err(ConnectionError::StaleSession);
        }
        if let Some((_, token)) = &mut permissions.slot {
            *token = None;
        }
        permissions.revision += 1;
        Ok(ReviewResponse {
            cluster_id: request.cluster_id,
            session_id: request.session_id.clone(),
            read_id: request.read_id,
            authorization_epoch: format!("{}:{epoch}", request.session_id),
            revision: permissions.revision,
            checked_at: now(),
            expires_at,
            items,
        })
    }
    pub(super) async fn require_permissions(
        &self,
        request: &SessionRequest,
        checks: &[PermissionCheck],
    ) -> Result<AuthorizationEpoch> {
        validate_checks(checks)?;
        let (client, cancel) = self.pod_client(request)?;
        let epoch = AuthorizationEpoch::capture(&self.authentication_generation);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let operation = async {
            let _worker = self
                .workers
                .try_acquire()
                .map_err(|_| ConnectionError::Busy)?;
            for check in checks {
                match review_check(&client, check, &cancel).await.decision {
                    PermissionDecision::Allowed => {}
                    PermissionDecision::Denied => return Err(ConnectionError::PermissionDenied),
                    PermissionDecision::Unknown => {
                        return Err(ConnectionError::PermissionReviewUnavailable);
                    }
                }
            }
            Ok(())
        };
        let result = tokio::select! { biased; _ = cancel.cancelled() => Err(ConnectionError::StaleSession), result = tokio::time::timeout_at(deadline, operation) => result.unwrap_or(Err(ConnectionError::PermissionReviewUnavailable)) };
        self.check_pod_session(request)?;
        if epoch.validate().is_err() || cancel.is_cancelled() {
            return Err(ConnectionError::StaleSession);
        }
        result.map(|()| epoch)
    }
}
fn unknown(check: &PermissionCheck, reason: PermissionReason) -> PermissionItem {
    PermissionItem {
        check: check.clone(),
        decision: PermissionDecision::Unknown,
        reason: Some(reason),
    }
}
async fn review_check(
    client: &Client,
    check: &PermissionCheck,
    cancel: &CancellationToken,
) -> PermissionItem {
    let Ok(requirements) = requirements(check) else {
        return unknown(check, PermissionReason::Failed);
    };
    for attributes in requirements {
        match review_attribute(client, attributes, cancel).await {
            Ok(PermissionDecision::Allowed) => {}
            Ok(PermissionDecision::Denied) => {
                return PermissionItem {
                    check: check.clone(),
                    decision: PermissionDecision::Denied,
                    reason: Some(PermissionReason::Denied),
                };
            }
            Ok(PermissionDecision::Unknown) => {
                return unknown(check, PermissionReason::Failed);
            }
            Err(reason) => return unknown(check, reason),
        }
    }
    PermissionItem {
        check: check.clone(),
        decision: PermissionDecision::Allowed,
        reason: None,
    }
}
async fn review_attribute(
    client: &Client,
    attributes: Value,
    cancel: &CancellationToken,
) -> std::result::Result<PermissionDecision, PermissionReason> {
    let operation = async {
        let _permit = REVIEW_WORKERS
            .acquire()
            .await
            .map_err(|_| PermissionReason::Unavailable)?;
        let spec = json!({"resourceAttributes": attributes});
        let body = json!({"apiVersion":"authorization.k8s.io/v1","kind":"SelfSubjectAccessReview","spec":spec});
        let mut request = Request::post(REVIEW_PATH)
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&body).map_err(|_| PermissionReason::Failed)?,
            ))
            .map_err(|_| PermissionReason::Failed)?;
        request.extensions_mut().insert(cancel.clone());
        let response = client
            .send(request)
            .await
            .map_err(|_| PermissionReason::Unavailable)?;
        if !matches!(
            response.status(),
            http::StatusCode::OK | http::StatusCode::CREATED
        ) {
            return Err(PermissionReason::Unavailable);
        }
        let mut body = response.into_body();
        let mut bytes = Vec::new();
        while let Some(frame) = body.frame().await {
            let frame = frame.map_err(|_| PermissionReason::Failed)?;
            if let Some(data) = frame.data_ref() {
                if bytes.len() + data.len() > 64 * 1024 {
                    return Err(PermissionReason::Failed);
                }
                bytes.extend_from_slice(data);
            }
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| PermissionReason::Failed)?;
        if value["apiVersion"] != "authorization.k8s.io/v1"
            || value["kind"] != "SelfSubjectAccessReview"
            || !matching_spec(&value["spec"], &spec)
        {
            return Err(PermissionReason::Failed);
        }
        evaluate_status(&value["status"])
    };
    tokio::select! { biased; _ = cancel.cancelled() => Err(PermissionReason::Unavailable), result = tokio::time::timeout(Duration::from_secs(5), operation) => result.unwrap_or(Err(PermissionReason::Unavailable)) }
}
fn matching_spec(actual: &Value, expected: &Value) -> bool {
    let mut actual = actual.clone();
    if actual["resourceAttributes"].is_object()
        && actual["resourceAttributes"].get("group").is_none()
    {
        actual["resourceAttributes"]["group"] = json!("");
    }
    actual == *expected
}
fn evaluate_status(status: &Value) -> std::result::Result<PermissionDecision, PermissionReason> {
    let allowed = status["allowed"]
        .as_bool()
        .ok_or(PermissionReason::Failed)?;
    let denied = match status.get("denied") {
        None => false,
        Some(value) => value.as_bool().ok_or(PermissionReason::Failed)?,
    };
    if status
        .get("evaluationError")
        .is_some_and(|error| error.as_str().is_none_or(|value| !value.is_empty()))
        || allowed && denied
    {
        return Err(PermissionReason::Failed);
    }
    Ok(if allowed {
        PermissionDecision::Allowed
    } else {
        PermissionDecision::Denied
    })
}

#[cfg(test)]
mod limits_tests;
#[cfg(test)]
mod tests;
