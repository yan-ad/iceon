use super::{
    ConnectionEngine, ConnectionError, ResourceKind, Result, SessionRequest,
    details::{ResourceReference, named_path, validate_reference},
    permissions::{PermissionAction, PermissionCheck, authorized_client},
};
use http::{Method, Request};
use http_body_util::BodyExt;
use kube::{Client, client::Body};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

const BODY_LIMIT: usize = 1024 * 1024;
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum NodeAction {
    Cordon,
    Uncordon,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NodeActionRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub operation_id: u64,
    pub reference: ResourceReference,
    pub resource_version: String,
    pub action: NodeAction,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Cancel {
    pub cluster_id: String,
    pub session_id: String,
    pub operation_id: u64,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeActionResult {
    pub cluster_id: String,
    pub session_id: String,
    pub operation_id: u64,
    pub reference: ResourceReference,
    pub action: NodeAction,
    pub accepted: bool,
}
#[derive(Default)]
pub(super) struct NodeActionSlot {
    high_water: u64,
    id: u64,
    cancel: Option<CancellationToken>,
}
impl NodeActionSlot {
    // Cancellation does not release the update gate until the operation settles.
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
        if id == self.id
            && let Some(token) = &self.cancel
        {
            token.cancel();
        }
    }
}
impl NodeActionRequest {
    fn session(&self) -> SessionRequest {
        SessionRequest {
            cluster_id: self.cluster_id.clone(),
            session_id: self.session_id.clone(),
        }
    }
}
fn safe_version(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && value.bytes().all(|b| (0x21..=0x7e).contains(&b))
}
fn validate(request: &NodeActionRequest) -> Result<()> {
    super::validate_request(&request.session())?;
    super::validate_read_id(request.operation_id)?;
    validate_reference(&request.reference)?;
    if request.reference.kind != ResourceKind::Nodes
        || request.reference.namespace.is_some()
        || request.reference.uid.is_none()
        || !safe_version(&request.resource_version)
    {
        return Err(ConnectionError::InvalidInput);
    }
    Ok(())
}
fn checks(request: &NodeActionRequest) -> [PermissionCheck; 2] {
    [PermissionAction::Get, PermissionAction::Patch].map(|action| PermissionCheck {
        kind: ResourceKind::Nodes,
        action,
        namespace: None,
        name: Some(request.reference.name.clone()),
    })
}
struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
impl ConnectionEngine {
    pub fn cancel_node_action(&self, request: Cancel) -> Result<()> {
        super::validate_read_id(request.operation_id)?;
        let identity = SessionRequest {
            cluster_id: request.cluster_id,
            session_id: request.session_id,
        };
        super::validate_request(&identity)?;
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        if let Some(session) = state
            .as_mut()
            .filter(|s| super::matches_session(s, &identity))
        {
            session.node_action.cancel(request.operation_id);
        }
        Ok(())
    }
    pub async fn resource_node_action(
        &self,
        request: NodeActionRequest,
    ) -> Result<NodeActionResult> {
        validate(&request)?;
        let identity = request.session();
        let (client, cancel) = {
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
            session
                .node_action
                .begin(request.operation_id, cancel.clone())?;
            (client, cancel)
        };
        let _guard = CancelOnDrop(cancel.clone());
        let dispatched = AtomicBool::new(false);
        let operation = async {
            let epoch = self
                .require_permissions(&identity, &checks(&request))
                .await?;
            epoch.validate()?;
            let _permit = self
                .workers
                .try_acquire()
                .map_err(|_| ConnectionError::Busy)?;
            let client = authorized_client(client, epoch.clone());
            mutate(&client, &request, &cancel, &dispatched).await?;
            epoch.validate()
        };
        let result = tokio::select! { biased;
            _ = cancel.cancelled() => Err(ConnectionError::StaleSession),
            result = tokio::time::timeout(Duration::from_secs(15), operation) => result.unwrap_or(Err(ConnectionError::Timeout)),
        };
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        let result = match super::current_mut(&mut state, &identity) {
            Ok(session) if session.node_action.id == request.operation_id => {
                session.node_action.cancel = None;
                if cancel.is_cancelled() {
                    Err(ConnectionError::StaleSession)
                } else {
                    result
                }
            }
            _ => Err(ConnectionError::StaleSession),
        };
        result.map_err(|error| {
            if dispatched.load(Ordering::SeqCst) && error == ConnectionError::StaleSession {
                ConnectionError::Network
            } else {
                error
            }
        })?;
        Ok(NodeActionResult {
            cluster_id: request.cluster_id,
            session_id: request.session_id,
            operation_id: request.operation_id,
            reference: request.reference,
            action: request.action,
            accepted: true,
        })
    }
}
fn verify_identity(value: &Value, request: &NodeActionRequest) -> Result<()> {
    if value["apiVersion"].as_str() != Some("v1")
        || value["kind"].as_str() != Some("Node")
        || value.pointer("/metadata/name").and_then(Value::as_str) != Some(&request.reference.name)
        || value
            .pointer("/metadata/namespace")
            .is_some_and(|v| !v.is_null())
        || value.pointer("/metadata/uid").and_then(Value::as_str)
            != request.reference.uid.as_deref()
        || !value
            .pointer("/metadata/resourceVersion")
            .and_then(Value::as_str)
            .is_some_and(safe_version)
    {
        return Err(ConnectionError::InvalidResponse);
    }
    Ok(())
}
fn patch_body(value: &Value, request: &NodeActionRequest) -> Result<Value> {
    if value.pointer("/metadata/uid").and_then(Value::as_str) != request.reference.uid.as_deref()
        || value
            .pointer("/metadata/resourceVersion")
            .and_then(Value::as_str)
            != Some(&request.resource_version)
    {
        return Err(ConnectionError::Conflict);
    }
    verify_identity(value, request)?;
    let mut ops = vec![
        json!({"op":"test","path":"/metadata/uid","value":request.reference.uid}),
        json!({"op":"test","path":"/metadata/resourceVersion","value":request.resource_version}),
    ];
    match value.get("spec") {
        None => ops.push(json!({"op":"add","path":"/spec","value":{}})),
        Some(spec) if spec.is_object() => {}
        _ => return Err(ConnectionError::InvalidResponse),
    }
    ops.push(json!({"op":"add","path":"/spec/unschedulable","value":request.action == NodeAction::Cordon}));
    Ok(Value::Array(ops))
}
async fn mutate(
    client: &Client,
    request: &NodeActionRequest,
    cancel: &CancellationToken,
    dispatched: &AtomicBool,
) -> Result<()> {
    let path = named_path(&request.reference)?;
    let current = send(client, cancel, Method::GET, &path, None).await?;
    let patch = patch_body(&current, request)?;
    super::validate_credentials(client)?;
    if cancel.is_cancelled() {
        return Err(ConnectionError::StaleSession);
    }
    // Once submitted, a lost response cannot establish that the write was rejected.
    dispatched.store(true, Ordering::SeqCst);
    let response = send(client, cancel, Method::PATCH, &path, Some(patch)).await?;
    verify_identity(&response, request)?;
    if response
        .pointer("/spec/unschedulable")
        .and_then(Value::as_bool)
        != Some(request.action == NodeAction::Cordon)
    {
        return Err(ConnectionError::InvalidResponse);
    }
    Ok(())
}
async fn send(
    client: &Client,
    cancel: &CancellationToken,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value> {
    let bytes = body
        .map(|v| serde_json::to_vec(&v))
        .transpose()
        .map_err(|_| ConnectionError::InvalidInput)?
        .unwrap_or_default();
    if bytes.len() > BODY_LIMIT {
        return Err(ConnectionError::LimitExceeded);
    }
    if cancel.is_cancelled() {
        return Err(ConnectionError::StaleSession);
    }
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json-patch+json")
        .body(Body::from(bytes))
        .map_err(|_| ConnectionError::InvalidInput)?;
    request.extensions_mut().insert(cancel.clone());
    let mut response = client
        .send(request)
        .await
        .map_err(super::pods::map_pod_error)?;
    match response.status().as_u16() {
        200 => {}
        400 | 422 => return Err(ConnectionError::InvalidInput),
        401 => return Err(ConnectionError::AuthFailed),
        403 => return Err(ConnectionError::PermissionDenied),
        404 => return Err(ConnectionError::NotFound),
        409 => return Err(ConnectionError::Conflict),
        408 | 504 => return Err(ConnectionError::Timeout),
        413 => return Err(ConnectionError::LimitExceeded),
        429 => return Err(ConnectionError::Busy),
        _ => return Err(ConnectionError::ConnectionFailed),
    }
    if response
        .headers()
        .get("content-length")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.parse::<usize>().ok())
        .is_some_and(|n| n > BODY_LIMIT)
    {
        return Err(ConnectionError::LimitExceeded);
    }
    let mut bytes = Vec::new();
    while let Some(frame) = response.body_mut().frame().await {
        let frame = frame.map_err(|_| ConnectionError::Network)?;
        if let Some(data) = frame.data_ref() {
            if data.len() > BODY_LIMIT.saturating_sub(bytes.len()) {
                return Err(ConnectionError::LimitExceeded);
            }
            bytes.extend_from_slice(data);
        }
    }
    serde_json::from_slice(&bytes).map_err(|_| ConnectionError::InvalidResponse)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex, atomic::AtomicU64};
    type Writes = Arc<Mutex<Vec<(String, Value)>>>;

    fn fake(current: Value, status: u16, writes: Writes) -> Client {
        Client::new(
            super::super::transport::CancelService(tower::service_fn(move |r: Request<Body>| {
                let mut current = current.clone();
                let writes = writes.clone();
                async move {
                    let get = r.method() == Method::GET;
                    if !get {
                        assert_eq!(r.method(), Method::PATCH);
                        assert_eq!(r.headers()["content-type"], "application/json-patch+json");
                        let path = r.uri().path().to_owned();
                        let patch: Value = serde_json::from_slice(
                            &r.into_body().collect().await.unwrap().to_bytes(),
                        )
                        .unwrap();
                        current["spec"]["unschedulable"] =
                            patch.as_array().unwrap().last().unwrap()["value"].clone();
                        current["metadata"]["resourceVersion"] = json!("8");
                        writes.lock().unwrap().push((path, patch));
                    }
                    Ok::<_, std::io::Error>(
                        http::Response::builder()
                            .status(if get { 200 } else { status })
                            .body(Body::from(serde_json::to_vec(&current).unwrap()))
                            .unwrap(),
                    )
                }
            })),
            "default",
        )
    }

    #[tokio::test]
    async fn fake_transport_is_single_shot_and_never_returns_node_data() {
        for action in [NodeAction::Cordon, NodeAction::Uncordon] {
            let r = request(action);
            let writes = Arc::new(Mutex::new(Vec::new()));
            let mut current = node();
            current["metadata"]["annotations"] = json!({"private":"PRIVATE"});
            mutate(
                &fake(current, 200, writes.clone()),
                &r,
                &CancellationToken::new(),
                &AtomicBool::new(false),
            )
            .await
            .unwrap();
            let seen = writes.lock().unwrap();
            assert_eq!(seen.len(), 1);
            assert_eq!(seen[0].0, "/api/v1/nodes/worker");
            let result = NodeActionResult {
                cluster_id: r.cluster_id,
                session_id: r.session_id,
                operation_id: r.operation_id,
                reference: r.reference,
                action,
                accepted: true,
            };
            let output = serde_json::to_value(result).unwrap();
            assert_eq!(output.as_object().unwrap().len(), 6);
            assert!(!output.to_string().contains("PRIVATE"));
        }
        for status in [403, 409, 500] {
            let writes = Arc::new(Mutex::new(Vec::new()));
            assert!(
                mutate(
                    &fake(node(), status, writes.clone()),
                    &request(NodeAction::Cordon),
                    &CancellationToken::new(),
                    &AtomicBool::new(false)
                )
                .await
                .is_err()
            );
            assert_eq!(writes.lock().unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn cancellation_conflicts_and_epoch_changes_prevent_write() {
        let r = request(NodeAction::Cordon);
        let writes = Arc::new(Mutex::new(Vec::new()));
        let cancel = CancellationToken::new();
        cancel.cancel();
        let dispatched = AtomicBool::new(false);
        assert!(
            mutate(&fake(node(), 200, writes.clone()), &r, &cancel, &dispatched)
                .await
                .is_err()
        );
        assert!(!dispatched.load(Ordering::SeqCst));
        let mut current = node();
        current["metadata"]["resourceVersion"] = json!("8");
        assert_eq!(
            mutate(
                &fake(current, 200, writes.clone()),
                &r,
                &CancellationToken::new(),
                &dispatched
            )
            .await,
            Err(ConnectionError::Conflict)
        );
        let generation = Arc::new(AtomicU64::new(1));
        let epoch = super::super::permissions::AuthorizationEpoch::capture(&generation);
        let client = authorized_client(fake(node(), 200, writes.clone()), epoch);
        generation.fetch_add(1, Ordering::SeqCst);
        assert!(
            mutate(&client, &r, &CancellationToken::new(), &dispatched)
                .await
                .is_err()
        );
        assert!(writes.lock().unwrap().is_empty());
    }
    fn request(action: NodeAction) -> NodeActionRequest {
        NodeActionRequest {
            cluster_id: "one".into(),
            session_id: "session".into(),
            operation_id: 1,
            reference: ResourceReference {
                kind: ResourceKind::Nodes,
                namespace: None,
                name: "worker".into(),
                uid: Some("uid".into()),
            },
            resource_version: "7".into(),
            action,
        }
    }
    fn node() -> Value {
        json!({"apiVersion":"v1","kind":"Node","metadata":{"name":"worker","uid":"uid","resourceVersion":"7"},"spec":{"taints":[{"key":"keep"}]}})
    }
    #[test]
    fn guarded_patch_preserves_spec_and_initializes_only_absent_spec() {
        for action in [NodeAction::Cordon, NodeAction::Uncordon] {
            let r = request(action);
            let mut n = node();
            let patch = patch_body(&n, &r).unwrap();
            assert_eq!(patch.as_array().unwrap().len(), 3);
            assert_eq!(patch[0]["path"], "/metadata/uid");
            assert_eq!(patch[1]["value"], "7");
            assert_eq!(patch[2]["value"], action == NodeAction::Cordon);
            n.as_object_mut().unwrap().remove("spec");
            assert_eq!(
                patch_body(&n, &r).unwrap()[2],
                json!({"op":"add","path":"/spec","value":{}})
            );
            n["spec"] = Value::Null;
            assert!(patch_body(&n, &r).is_err());
            n = node();
            n["metadata"]["resourceVersion"] = json!("8");
            assert_eq!(patch_body(&n, &r), Err(ConnectionError::Conflict));
            n = node();
            n["metadata"]["uid"] = json!("other");
            assert_eq!(patch_body(&n, &r), Err(ConnectionError::Conflict));
        }
    }
    #[test]
    fn strict_contract_and_slot() {
        let mut r = request(NodeAction::Cordon);
        assert!(validate(&r).is_ok());
        r.reference.namespace = Some("default".into());
        assert!(validate(&r).is_err());
        r.reference.namespace = None;
        r.reference.uid = None;
        assert!(validate(&r).is_err());
        assert!(serde_json::from_value::<NodeAction>(json!("drain")).is_err());
        let mut slot = NodeActionSlot::default();
        slot.cancel(3);
        assert_eq!(
            slot.begin(3, CancellationToken::new()),
            Err(ConnectionError::StaleSession)
        );
        let token = CancellationToken::new();
        slot.begin(4, token.clone()).unwrap();
        slot.cancel(5);
        assert!(!token.is_cancelled());
        slot.cancel(4);
        assert!(token.is_cancelled());
        assert!(slot.active());
        assert_eq!(
            slot.begin(6, CancellationToken::new()),
            Err(ConnectionError::Busy)
        );
    }
}
