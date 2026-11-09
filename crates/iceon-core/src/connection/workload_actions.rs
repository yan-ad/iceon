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
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

const BODY_LIMIT: usize = 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum WorkloadAction {
    Scale,
    Restart,
    Suspend,
    Resume,
    Trigger,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub operation_id: u64,
    pub reference: ResourceReference,
    pub resource_version: String,
    pub action: WorkloadAction,
    pub replicas: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Cancel {
    pub cluster_id: String,
    pub session_id: String,
    pub operation_id: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkloadActionResult {
    pub cluster_id: String,
    pub session_id: String,
    pub operation_id: u64,
    pub reference: ResourceReference,
    pub action: WorkloadAction,
    pub accepted: bool,
    pub created: Option<ResourceReference>,
}

#[derive(Default)]
pub(super) struct WorkloadActionSlot {
    high_water: u64,
    id: u64,
    cancel: Option<CancellationToken>,
}

impl WorkloadActionSlot {
    pub(super) fn active(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|cancel| !cancel.is_cancelled())
    }

    fn begin(&mut self, id: u64, cancel: CancellationToken) -> Result<()> {
        if id <= self.high_water {
            return Err(ConnectionError::StaleSession);
        }
        if self
            .cancel
            .as_ref()
            .is_some_and(|token| !token.is_cancelled())
        {
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

impl WorkflowRequest {
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

fn validate(request: &WorkflowRequest) -> Result<()> {
    super::validate_request(&request.session())?;
    super::validate_read_id(request.operation_id)?;
    validate_reference(&request.reference)?;
    if request.reference.uid.is_none() || !safe_version(&request.resource_version) {
        return Err(ConnectionError::InvalidInput);
    }
    use ResourceKind::*;
    let supported = match request.action {
        WorkloadAction::Scale => {
            matches!(
                request.reference.kind,
                Deployments | StatefulSets | ReplicaSets | ReplicationControllers
            ) && request.replicas.is_some_and(|n| n <= i32::MAX as u32)
        }
        WorkloadAction::Restart => {
            matches!(
                request.reference.kind,
                Deployments | DaemonSets | StatefulSets
            ) && request.replicas.is_none()
        }
        WorkloadAction::Suspend | WorkloadAction::Resume => {
            matches!(request.reference.kind, Jobs | CronJobs) && request.replicas.is_none()
        }
        WorkloadAction::Trigger => request.reference.kind == CronJobs && request.replicas.is_none(),
    };
    if supported {
        Ok(())
    } else {
        Err(ConnectionError::InvalidInput)
    }
}

fn checks(request: &WorkflowRequest) -> [PermissionCheck; 2] {
    let reference = &request.reference;
    let trigger = request.action == WorkloadAction::Trigger;
    [
        PermissionCheck {
            kind: reference.kind,
            action: PermissionAction::Get,
            namespace: reference.namespace.clone(),
            name: Some(reference.name.clone()),
        },
        PermissionCheck {
            kind: if trigger {
                ResourceKind::Jobs
            } else {
                reference.kind
            },
            action: if trigger {
                PermissionAction::Create
            } else {
                PermissionAction::Patch
            },
            namespace: reference.namespace.clone(),
            name: if trigger {
                None
            } else {
                Some(reference.name.clone())
            },
        },
    ]
}

struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

impl ConnectionEngine {
    pub fn cancel_workload_action(&self, request: Cancel) -> Result<()> {
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
            session.workload_action.cancel(request.operation_id);
        }
        Ok(())
    }

    /// Errors after a write is dispatched (including timeout, cancellation or a lost
    /// response) do not prove rejection. Refresh before offering another action;
    /// an operation ID is consumed even on failure and must never be replayed.
    pub async fn resource_workload_action(
        &self,
        request: WorkflowRequest,
    ) -> Result<WorkloadActionResult> {
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
                .workload_action
                .begin(request.operation_id, cancel.clone())?;
            (client, cancel)
        };
        let _guard = CancelOnDrop(cancel.clone());
        let dispatched = Arc::new(AtomicBool::new(false));
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
            let created = mutate(&client, &request, &cancel, &dispatched).await?;
            epoch.validate()?;
            Ok(created)
        };
        let result = tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(ConnectionError::StaleSession),
            result = tokio::time::timeout(DEADLINE, operation) => result.unwrap_or(Err(ConnectionError::Timeout)),
        };
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        let result = match super::current_mut(&mut state, &identity) {
            Ok(session) if session.workload_action.id == request.operation_id => {
                session.workload_action.cancel = None;
                if cancel.is_cancelled() {
                    Err(ConnectionError::StaleSession)
                } else {
                    result
                }
            }
            _ => Err(ConnectionError::StaleSession),
        };
        // ConnectionError has no indeterminate-write variant. Network explicitly
        // avoids reporting a cancelled submitted write as a safely rejected action.
        let created = result.map_err(|error| {
            if dispatched.load(Ordering::SeqCst) && error == ConnectionError::StaleSession {
                ConnectionError::Network
            } else {
                error
            }
        })?;
        Ok(WorkloadActionResult {
            cluster_id: request.cluster_id,
            session_id: request.session_id,
            operation_id: request.operation_id,
            reference: request.reference,
            action: request.action,
            accepted: true,
            created,
        })
    }
}

fn verify_identity(value: &Value, reference: &ResourceReference) -> Result<()> {
    let (api, kind) = super::create::creation_info(reference.kind);
    if value["apiVersion"].as_str() != Some(api)
        || value["kind"].as_str() != Some(kind)
        || value.pointer("/metadata/name").and_then(Value::as_str) != Some(&reference.name)
        || value.pointer("/metadata/namespace").and_then(Value::as_str)
            != reference.namespace.as_deref()
        || value.pointer("/metadata/uid").and_then(Value::as_str) != reference.uid.as_deref()
        || !value
            .pointer("/metadata/resourceVersion")
            .and_then(Value::as_str)
            .is_some_and(safe_version)
    {
        return Err(ConnectionError::InvalidResponse);
    }
    Ok(())
}

fn verify_baseline(value: &Value, request: &WorkflowRequest) -> Result<()> {
    if value.pointer("/metadata/uid").and_then(Value::as_str) != request.reference.uid.as_deref()
        || value
            .pointer("/metadata/resourceVersion")
            .and_then(Value::as_str)
            != Some(&request.resource_version)
    {
        return Err(ConnectionError::Conflict);
    }
    verify_identity(value, &request.reference)
}

fn patch_body(value: &Value, request: &WorkflowRequest) -> Result<Value> {
    if !value.get("spec").is_some_and(Value::is_object) {
        return Err(ConnectionError::InvalidResponse);
    }
    let mut ops = vec![
        json!({"op":"test","path":"/metadata/uid","value":request.reference.uid}),
        json!({"op":"test","path":"/metadata/resourceVersion","value":request.resource_version}),
    ];
    match request.action {
        WorkloadAction::Scale => ops.push(json!({"op":"add","path":"/spec/replicas","value":request.replicas})),
        WorkloadAction::Suspend | WorkloadAction::Resume => ops.push(json!({"op":"add","path":"/spec/suspend","value":request.action == WorkloadAction::Suspend})),
        WorkloadAction::Restart => {
            if !value.pointer("/spec/template").is_some_and(Value::is_object) {
                return Err(ConnectionError::InvalidResponse);
            }
            let metadata = value.pointer("/spec/template/metadata");
            if metadata.is_none_or(Value::is_null) {
                ops.push(json!({"op":"add","path":"/spec/template/metadata","value":{"annotations":{}}}));
            } else {
                let metadata = metadata.unwrap();
                if !metadata.is_object() { return Err(ConnectionError::InvalidResponse); }
                match metadata.get("annotations") {
                    None | Some(Value::Null) => ops.push(json!({"op":"add","path":"/spec/template/metadata/annotations","value":{}})),
                    Some(annotations) if annotations.is_object() => {},
                    _ => return Err(ConnectionError::InvalidResponse),
                }
            }
            ops.push(json!({"op":"add","path":"/spec/template/metadata/annotations/kubectl.kubernetes.io~1restartedAt","value":jiff::Timestamp::now().to_string()}));
        }
        WorkloadAction::Trigger => return Err(ConnectionError::InvalidInput),
    }
    Ok(Value::Array(ops))
}

fn job_prefix(name: &str) -> Result<String> {
    if !super::resources::valid_name(name) {
        return Err(ConnectionError::InvalidInput);
    }
    let base = name[..name.len().min(48)].trim_end_matches(['-', '.']);
    let prefix = format!("{base}-manual-");
    if base.is_empty() || prefix.len() > 56 {
        return Err(ConnectionError::InvalidInput);
    }
    Ok(prefix)
}

fn job_body(value: &Value, request: &WorkflowRequest) -> Result<Value> {
    let spec = value
        .pointer("/spec/jobTemplate/spec")
        .filter(|s| s.is_object())
        .ok_or(ConnectionError::InvalidResponse)?;
    let prefix = job_prefix(&request.reference.name)?;
    let mut metadata = json!({
        "generateName":prefix,
        "namespace":request.reference.namespace,
        "ownerReferences":[{"apiVersion":"batch/v1","kind":"CronJob","name":request.reference.name,"uid":request.reference.uid,"controller":true,"blockOwnerDeletion":false}],
    });
    for field in ["labels", "annotations"] {
        if let Some(data) = value.pointer(&format!("/spec/jobTemplate/metadata/{field}")) {
            if data.is_null() {
                continue;
            }
            let object = data.as_object().ok_or(ConnectionError::InvalidResponse)?;
            if object.values().any(|v| !v.is_string()) {
                return Err(ConnectionError::InvalidResponse);
            }
            metadata[field] = data.clone();
        }
    }
    // Only the API server receives the template. It is never included in results,
    // errors, events or logs because Pod specs can contain inline credentials.
    Ok(json!({"apiVersion":"batch/v1","kind":"Job","metadata":metadata,"spec":spec}))
}

async fn mutate(
    client: &Client,
    request: &WorkflowRequest,
    cancel: &CancellationToken,
    dispatched: &AtomicBool,
) -> Result<Option<ResourceReference>> {
    let path = named_path(&request.reference)?;
    let (get_status, current) =
        send(client, cancel, Method::GET, &path, None, "application/json").await?;
    if get_status != 200 {
        return Err(ConnectionError::InvalidResponse);
    }
    verify_baseline(&current, request)?;
    let trigger = request.action == WorkloadAction::Trigger;
    let (method, path, body, content_type) = if trigger {
        let (api, resource) = super::transport::endpoint(ResourceKind::Jobs);
        (
            Method::POST,
            format!(
                "{api}/namespaces/{}/{resource}",
                request
                    .reference
                    .namespace
                    .as_deref()
                    .ok_or(ConnectionError::InvalidInput)?
            ),
            job_body(&current, request)?,
            "application/json",
        )
    } else {
        (
            Method::PATCH,
            path,
            patch_body(&current, request)?,
            "application/json-patch+json",
        )
    };
    super::validate_credentials(client)?;
    if cancel.is_cancelled() {
        return Err(ConnectionError::StaleSession);
    }
    // Reserve uncertainty before polling send; neither PATCH nor POST is retried.
    dispatched.store(true, Ordering::SeqCst);
    let (status, response) = send(client, cancel, method, &path, Some(body), content_type).await?;
    if trigger {
        if status != 201 {
            return Err(ConnectionError::InvalidResponse);
        }
        let name = response
            .pointer("/metadata/name")
            .and_then(Value::as_str)
            .ok_or(ConnectionError::InvalidResponse)?;
        let uid = response
            .pointer("/metadata/uid")
            .and_then(Value::as_str)
            .filter(|s| safe_version(s))
            .ok_or(ConnectionError::InvalidResponse)?;
        if !name.starts_with(&job_prefix(&request.reference.name)?)
            || !super::resources::valid_name(name)
        {
            return Err(ConnectionError::InvalidResponse);
        }
        let created = ResourceReference {
            kind: ResourceKind::Jobs,
            namespace: request.reference.namespace.clone(),
            name: name.into(),
            uid: Some(uid.into()),
        };
        verify_identity(&response, &created)?;
        Ok(Some(created))
    } else {
        if status != 200 {
            return Err(ConnectionError::InvalidResponse);
        }
        verify_identity(&response, &request.reference)?;
        Ok(None)
    }
}

async fn send(
    client: &Client,
    cancel: &CancellationToken,
    method: Method,
    path: &str,
    body: Option<Value>,
    content_type: &str,
) -> Result<(u16, Value)> {
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
        .header("Content-Type", content_type)
        .body(Body::from(bytes))
        .map_err(|_| ConnectionError::InvalidInput)?;
    request.extensions_mut().insert(cancel.clone());
    let mut response = client
        .send(request)
        .await
        .map_err(super::pods::map_pod_error)?;
    let status = response.status().as_u16();
    match status {
        200..=299 => {}
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
    Ok((
        status,
        serde_json::from_slice(&bytes).map_err(|_| ConnectionError::InvalidResponse)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::Response;
    use std::sync::{Mutex, atomic::AtomicU64};

    fn request(kind: ResourceKind, action: WorkloadAction) -> WorkflowRequest {
        WorkflowRequest {
            cluster_id: "cluster".into(),
            session_id: "session".into(),
            operation_id: 1,
            reference: ResourceReference {
                kind,
                namespace: Some("default".into()),
                name: "sample".into(),
                uid: Some("uid-1".into()),
            },
            resource_version: "7".into(),
            action,
            replicas: (action == WorkloadAction::Scale).then_some(3),
        }
    }
    fn parent(request: &WorkflowRequest) -> Value {
        let (api, kind) = super::super::create::creation_info(request.reference.kind);
        json!({"apiVersion":api,"kind":kind,"metadata":{"name":"sample","namespace":"default","uid":"uid-1","resourceVersion":"7"},"spec":{"template":{"metadata":{"annotations":{"keep":"yes"}}},"jobTemplate":{"metadata":{"labels":{"keep":"yes"},"annotations":{"keep":"yes"}},"spec":{"template":{"spec":{"containers":[{"name":"job","image":"fixture","env":[{"name":"SECRET","value":"PRIVATE"}]}],"restartPolicy":"Never"}}}}}})
    }

    #[test]
    fn validates_action_matrix_and_exact_inputs() {
        use ResourceKind::*;
        for kind in [
            Deployments,
            StatefulSets,
            ReplicaSets,
            ReplicationControllers,
        ] {
            assert!(validate(&request(kind, WorkloadAction::Scale)).is_ok());
        }
        for kind in [Deployments, DaemonSets, StatefulSets] {
            assert!(validate(&request(kind, WorkloadAction::Restart)).is_ok());
        }
        for kind in [Jobs, CronJobs] {
            assert!(validate(&request(kind, WorkloadAction::Suspend)).is_ok());
            assert!(validate(&request(kind, WorkloadAction::Resume)).is_ok());
        }
        assert!(validate(&request(CronJobs, WorkloadAction::Trigger)).is_ok());
        for (kind, action) in [
            (Pods, WorkloadAction::Scale),
            (Jobs, WorkloadAction::Restart),
            (Deployments, WorkloadAction::Trigger),
            (DaemonSets, WorkloadAction::Suspend),
        ] {
            assert_eq!(
                validate(&request(kind, action)),
                Err(ConnectionError::InvalidInput)
            );
        }
        let mut r = request(Deployments, WorkloadAction::Scale);
        for count in [None, Some(u32::MAX)] {
            r.replicas = count;
            assert!(validate(&r).is_err());
        }
        r.replicas = Some(0);
        assert!(validate(&r).is_ok());
        r.reference.uid = None;
        assert!(validate(&r).is_err());
        r.reference.uid = Some("uid-1".into());
        for version in ["", "\n", "a b"] {
            r.resource_version = version.into();
            assert!(validate(&r).is_err());
        }
        let raw = json!({"clusterId":"cluster","sessionId":"session","operationId":1,"reference":{"kind":"deployments","namespace":"default","name":"sample","uid":"u"},"resourceVersion":"7","action":"scale","replicas":1});
        assert!(serde_json::from_value::<WorkflowRequest>(raw.clone()).is_ok());
        let mut extra = raw.clone();
        extra["unsafe"] = json!(true);
        assert!(serde_json::from_value::<WorkflowRequest>(extra).is_err());
        let mut extra = raw;
        extra["reference"]["unsafe"] = json!(true);
        assert!(serde_json::from_value::<WorkflowRequest>(extra).is_err());
        assert!(serde_json::from_value::<WorkloadAction>(json!({"scale":{}})).is_err());
        assert!(
            serde_json::from_value::<Cancel>(
                json!({"clusterId":"c","sessionId":"s","operationId":1,"extra":true})
            )
            .is_err()
        );
    }

    #[test]
    fn patches_preserve_siblings_and_test_both_baselines() {
        for (kind, action) in [
            (ResourceKind::Deployments, WorkloadAction::Scale),
            (ResourceKind::StatefulSets, WorkloadAction::Restart),
            (ResourceKind::Jobs, WorkloadAction::Suspend),
            (ResourceKind::CronJobs, WorkloadAction::Resume),
        ] {
            let r = request(kind, action);
            let body = patch_body(&parent(&r), &r).unwrap();
            assert_eq!(
                body[0],
                json!({"op":"test","path":"/metadata/uid","value":"uid-1"})
            );
            assert_eq!(
                body[1],
                json!({"op":"test","path":"/metadata/resourceVersion","value":"7"})
            );
            assert!(
                body.as_array()
                    .unwrap()
                    .iter()
                    .all(|op| op["path"] != "/spec")
            );
            match action {
                WorkloadAction::Scale => assert_eq!(body[2]["value"], 3),
                WorkloadAction::Suspend => assert_eq!(body[2]["value"], true),
                WorkloadAction::Resume => assert_eq!(body[2]["value"], false),
                WorkloadAction::Restart => {
                    assert_eq!(
                        body[2]["path"],
                        "/spec/template/metadata/annotations/kubectl.kubernetes.io~1restartedAt"
                    );
                    assert!(
                        body[2]["value"]
                            .as_str()
                            .unwrap()
                            .parse::<jiff::Timestamp>()
                            .is_ok()
                    );
                }
                _ => unreachable!(),
            }
            let mut changed = parent(&r);
            changed["metadata"]["uid"] = json!("replacement");
            assert_eq!(
                verify_baseline(&changed, &r),
                Err(ConnectionError::Conflict)
            );
            changed = parent(&r);
            changed["metadata"]["resourceVersion"] = json!("8");
            assert_eq!(
                verify_baseline(&changed, &r),
                Err(ConnectionError::Conflict)
            );
            changed = parent(&r);
            changed["spec"] = Value::Null;
            assert_eq!(
                patch_body(&changed, &r),
                Err(ConnectionError::InvalidResponse)
            );
        }
        let r = request(ResourceKind::Deployments, WorkloadAction::Restart);
        for missing in ["metadata", "annotations"] {
            let mut p = parent(&r);
            if missing == "metadata" {
                p["spec"]["template"]
                    .as_object_mut()
                    .unwrap()
                    .remove("metadata");
            } else {
                p["spec"]["template"]["metadata"]
                    .as_object_mut()
                    .unwrap()
                    .remove("annotations");
            }
            assert_eq!(patch_body(&p, &r).unwrap().as_array().unwrap().len(), 4);
        }
    }

    #[test]
    fn trigger_only_copies_template_and_permissions_match_wire_verbs() {
        let r = request(ResourceKind::CronJobs, WorkloadAction::Trigger);
        let p = parent(&r);
        let body = job_body(&p, &r).unwrap();
        assert_eq!(body["spec"], p["spec"]["jobTemplate"]["spec"]);
        assert_eq!(body["metadata"]["generateName"], "sample-manual-");
        assert_eq!(body["metadata"]["ownerReferences"][0]["uid"], "uid-1");
        assert_eq!(body["metadata"]["labels"]["keep"], "yes");
        assert!(body["metadata"].get("uid").is_none());
        for name in [
            "x".to_string(),
            "a".repeat(253),
            format!("{}-a", "a".repeat(47)),
        ] {
            let prefix = job_prefix(&name).unwrap();
            assert!(prefix.len() <= 56);
            assert!(super::super::resources::valid_name(&format!(
                "{prefix}abcde"
            )));
        }
        assert!(job_prefix("../escape").is_err());
        let c = checks(&r);
        assert_eq!(c[0].action, PermissionAction::Get);
        assert_eq!(c[0].kind, ResourceKind::CronJobs);
        assert_eq!(c[1].action, PermissionAction::Create);
        assert_eq!(c[1].kind, ResourceKind::Jobs);
        assert!(c[1].name.is_none());
        assert_eq!(
            checks(&request(ResourceKind::Deployments, WorkloadAction::Scale))[1].action,
            PermissionAction::Patch
        );
    }

    #[test]
    fn slot_consumes_ids_and_cancellation_is_exact() {
        let mut slot = WorkloadActionSlot::default();
        let first = CancellationToken::new();
        slot.begin(1, first.clone()).unwrap();
        assert_eq!(
            slot.begin(1, CancellationToken::new()),
            Err(ConnectionError::StaleSession)
        );
        assert_eq!(
            slot.begin(2, CancellationToken::new()),
            Err(ConnectionError::Busy)
        );
        slot.cancel(3);
        assert!(!first.is_cancelled());
        slot.cancel(1);
        assert!(first.is_cancelled());
        assert_eq!(
            slot.begin(3, CancellationToken::new()),
            Err(ConnectionError::StaleSession)
        );
        slot.begin(4, CancellationToken::new()).unwrap();
        slot.cancel(1);
        assert!(!slot.cancel.as_ref().unwrap().is_cancelled());
    }

    fn fake(
        parent: Value,
        status: u16,
        writes: Arc<Mutex<Vec<(String, String, Value)>>>,
    ) -> Client {
        Client::new(
            super::super::transport::CancelService(tower::service_fn(move |r: Request<Body>| {
                let parent = parent.clone();
                let writes = writes.clone();
                async move {
                    let is_get = r.method() == Method::GET;
                    let response = if is_get {
                        parent
                    } else {
                        let method = r.method().to_string();
                        let path = r.uri().path().to_owned();
                        let content_type = r.headers()["content-type"].to_str().unwrap().to_owned();
                        let body: Value = serde_json::from_slice(
                            &r.into_body().collect().await.unwrap().to_bytes(),
                        )
                        .unwrap();
                        if method == "PATCH" {
                            assert_eq!(content_type, "application/json-patch+json");
                        }
                        writes.lock().unwrap().push((method.clone(), path, body));
                        if method == "POST" {
                            json!({"apiVersion":"batch/v1","kind":"Job","metadata":{"name":"sample-manual-abcde","namespace":"default","uid":"new-uid","resourceVersion":"8"}})
                        } else {
                            parent
                        }
                    };
                    Ok::<_, std::io::Error>(
                        Response::builder()
                            .status(if is_get { 200 } else { status })
                            .body(Body::from(serde_json::to_vec(&response).unwrap()))
                            .unwrap(),
                    )
                }
            })),
            "default",
        )
    }
    #[tokio::test]
    async fn fake_mutations_are_single_shot_and_return_metadata_only() {
        for (kind, action, status) in [
            (ResourceKind::Deployments, WorkloadAction::Scale, 200),
            (ResourceKind::CronJobs, WorkloadAction::Trigger, 201),
        ] {
            let r = request(kind, action);
            let writes = Arc::new(Mutex::new(Vec::new()));
            let client = fake(parent(&r), status, writes.clone());
            let dispatched = AtomicBool::new(false);
            let result = mutate(&client, &r, &CancellationToken::new(), &dispatched)
                .await
                .unwrap();
            assert!(dispatched.load(Ordering::SeqCst));
            assert_eq!(writes.lock().unwrap().len(), 1);
            let output = WorkloadActionResult {
                cluster_id: r.cluster_id,
                session_id: r.session_id,
                operation_id: r.operation_id,
                reference: r.reference,
                action,
                accepted: true,
                created: result,
            };
            assert!(!serde_json::to_string(&output).unwrap().contains("PRIVATE"));
        }
        let r = request(ResourceKind::Deployments, WorkloadAction::Scale);
        for status in [403, 409, 500] {
            let writes = Arc::new(Mutex::new(Vec::new()));
            let client = fake(parent(&r), status, writes.clone());
            assert!(
                mutate(
                    &client,
                    &r,
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
    async fn stale_baseline_and_epoch_never_write() {
        let r = request(ResourceKind::Deployments, WorkloadAction::Scale);
        let writes = Arc::new(Mutex::new(Vec::new()));
        let mut p = parent(&r);
        p["metadata"]["resourceVersion"] = json!("8");
        assert_eq!(
            mutate(
                &fake(p, 200, writes.clone()),
                &r,
                &CancellationToken::new(),
                &AtomicBool::new(false)
            )
            .await,
            Err(ConnectionError::Conflict)
        );
        assert!(writes.lock().unwrap().is_empty());
        let generation = Arc::new(AtomicU64::new(1));
        let epoch = super::super::permissions::AuthorizationEpoch::capture(&generation);
        let client = authorized_client(fake(parent(&r), 200, writes.clone()), epoch);
        generation.fetch_add(1, Ordering::SeqCst);
        assert!(
            mutate(
                &client,
                &r,
                &CancellationToken::new(),
                &AtomicBool::new(false)
            )
            .await
            .is_err()
        );
        assert!(writes.lock().unwrap().is_empty());
    }
}
