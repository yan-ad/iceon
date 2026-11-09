//! Reduced existing-object editing. Only reviewed scalar leaves become JSON Patch operations.
use super::details::ResourceReference;
use super::permissions::{PermissionAction, PermissionCheck};
use super::{ConnectionEngine, ConnectionError, ResourceKind, Result, SessionRequest};
use http::{Method, Request};
use http_body_util::BodyExt;
use kube::{Client, client::Body};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

const YAML_LIMIT: usize = 64 * 1024;
const BODY_LIMIT: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum EditMode {
    Read,
    DryRun,
    Apply,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub operation_id: u64,
    pub reference: ResourceReference,
    pub resource_version: Option<String>,
    pub yaml: Option<String>,
    pub mode: EditMode,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditCancelRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub operation_id: u64,
}
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EditResult {
    pub cluster_id: String,
    pub session_id: String,
    pub operation_id: u64,
    pub reference: ResourceReference,
    pub resource_version: String,
    pub yaml: Option<String>,
    pub mode: EditMode,
    pub accepted: bool,
}
#[derive(Default)]
pub(crate) struct EditSlot {
    high_water: u64,
    active_id: u64,
    cancel: Option<CancellationToken>,
}
impl EditSlot {
    pub(super) fn active(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|cancel| !cancel.is_cancelled())
    }
    fn start(&mut self, id: u64, cancel: CancellationToken) -> Result<()> {
        if id <= self.high_water {
            return Err(ConnectionError::StaleSession);
        }
        if self.cancel.as_ref().is_some_and(|c| !c.is_cancelled()) {
            return Err(ConnectionError::Busy);
        }
        self.high_water = id;
        self.active_id = id;
        self.cancel = Some(cancel);
        Ok(())
    }
    fn cancel(&mut self, id: u64) {
        self.high_water = self.high_water.max(id);
        if self.active_id == id
            && let Some(cancel) = self.cancel.take()
        {
            cancel.cancel();
        }
    }
}
#[cfg(test)]
mod transport_tests {
    use super::*;
    #[tokio::test]
    async fn permission_denial_never_reads_or_writes_the_resource() {
        let (_root, _, engine, id, _) = super::super::tests::fixture();
        let reviews = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let lease = super::super::tests::connected(
            &engine,
            id,
            super::super::tests::denied_permission_client(reviews.clone(), Some(false)),
        )
        .await;
        let (mut r, raw) = super::tests::fixture();
        r.cluster_id = lease.cluster_id;
        r.session_id = lease.session_id;
        r.yaml = Some(serde_yaml_ng::to_string(&projection(&raw, &r.reference).unwrap()).unwrap());
        assert_eq!(
            engine.resource_edit(r).await,
            Err(ConnectionError::PermissionDenied)
        );
        assert_eq!(reviews.lock().unwrap()[0]["resource"], "deployments");
    }

    #[tokio::test]
    async fn fresh_version_conflict_prevents_patch() {
        let (mut r, raw) = super::tests::fixture();
        r.resource_version = Some("old".into());
        let client = Client::new(
            tower::service_fn(move |request: Request<Body>| {
                let response = raw.clone();
                async move {
                    assert_eq!(request.method(), Method::GET);
                    Ok::<_, std::io::Error>(http::Response::new(Body::from(
                        serde_json::to_vec(&response).unwrap(),
                    )))
                }
            }),
            "default",
        );
        assert_eq!(
            run(&client, &CancellationToken::new(), &r, Some(&json!({}))).await,
            Err(ConnectionError::Conflict)
        );
    }
    #[tokio::test]
    async fn fake_api_reads_then_patches_only_leaves_with_server_dry_run() {
        let (r, raw) = super::tests::fixture();
        let mut edited = projection(&raw, &r.reference).unwrap();
        edited["spec"]["template"]["spec"]["containers"][0]["image"] = json!("demo:v2");
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = seen.clone();
        let response = raw.clone();
        let client = Client::new(
            tower::service_fn(move |request: Request<Body>| {
                let seen = captured.clone();
                let response = response.clone();
                async move {
                    let method = request.method().clone();
                    let uri = request.uri().to_string();
                    let content_type = request
                        .headers()
                        .get("content-type")
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_owned();
                    let bytes = request.into_body().collect().await.unwrap().to_bytes();
                    seen.lock()
                        .unwrap()
                        .push((method, uri, content_type, bytes.to_vec()));
                    Ok::<_, std::io::Error>(http::Response::new(Body::from(
                        serde_json::to_vec(&response).unwrap(),
                    )))
                }
            }),
            "default",
        );
        let result = run(&client, &CancellationToken::new(), &r, Some(&edited))
            .await
            .unwrap();
        assert!(result.accepted);
        assert!(result.yaml.is_none());
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].0, Method::GET);
        assert_eq!(seen[1].0, Method::PATCH);
        assert!(seen[1].1.ends_with("?dryRun=All&fieldValidation=Strict"));
        assert_eq!(seen[1].2, "application/json-patch+json");
        let sent: Value = serde_json::from_slice(&seen[1].3).unwrap();
        assert_eq!(sent[0]["path"], "/metadata/uid");
        assert_eq!(sent[1]["path"], "/metadata/resourceVersion");
        assert_eq!(sent[4]["path"], "/spec/template/spec/containers/0/image");
        assert!(!String::from_utf8_lossy(&seen[1].3).contains("SECRET"));
    }
}
impl EditRequest {
    fn session(&self) -> SessionRequest {
        SessionRequest {
            cluster_id: self.cluster_id.clone(),
            session_id: self.session_id.clone(),
        }
    }
}
#[derive(Clone, Copy)]
enum Scalar {
    Count,
    Bool,
    Text,
    IntOrPercent,
}
fn fields(kind: ResourceKind) -> &'static [(&'static str, Scalar)] {
    use ResourceKind::*;
    use Scalar::*;
    match kind {
        Deployments => &[
            ("/spec/replicas", Count),
            ("/spec/paused", Bool),
            ("/spec/minReadySeconds", Count),
            ("/spec/revisionHistoryLimit", Count),
            ("/spec/progressDeadlineSeconds", Count),
            ("/spec/strategy/type", Text),
            ("/spec/strategy/rollingUpdate/maxSurge", IntOrPercent),
            ("/spec/strategy/rollingUpdate/maxUnavailable", IntOrPercent),
        ],
        StatefulSets => &[
            ("/spec/replicas", Count),
            ("/spec/minReadySeconds", Count),
            ("/spec/revisionHistoryLimit", Count),
            ("/spec/updateStrategy/type", Text),
            ("/spec/updateStrategy/rollingUpdate/partition", Count),
        ],
        DaemonSets => &[
            ("/spec/minReadySeconds", Count),
            ("/spec/revisionHistoryLimit", Count),
            ("/spec/updateStrategy/type", Text),
            ("/spec/updateStrategy/rollingUpdate/maxSurge", IntOrPercent),
            (
                "/spec/updateStrategy/rollingUpdate/maxUnavailable",
                IntOrPercent,
            ),
        ],
        ReplicaSets | ReplicationControllers => {
            &[("/spec/replicas", Count), ("/spec/minReadySeconds", Count)]
        }
        Jobs => &[
            ("/spec/parallelism", Count),
            ("/spec/completions", Count),
            ("/spec/activeDeadlineSeconds", Count),
            ("/spec/backoffLimit", Count),
            ("/spec/suspend", Bool),
            ("/spec/ttlSecondsAfterFinished", Count),
        ],
        CronJobs => &[
            ("/spec/schedule", Text),
            ("/spec/suspend", Bool),
            ("/spec/concurrencyPolicy", Text),
            ("/spec/startingDeadlineSeconds", Count),
            ("/spec/successfulJobsHistoryLimit", Count),
            ("/spec/failedJobsHistoryLimit", Count),
        ],
        HorizontalPodAutoscalers => &[("/spec/minReplicas", Count), ("/spec/maxReplicas", Count)],
        PodDisruptionBudgets => &[
            ("/spec/minAvailable", IntOrPercent),
            ("/spec/maxUnavailable", IntOrPercent),
            ("/spec/unhealthyPodEvictionPolicy", Text),
        ],
        Nodes => &[("/spec/unschedulable", Bool)],
        PersistentVolumes => &[("/spec/persistentVolumeReclaimPolicy", Text)],
        PersistentVolumeClaims => &[("/spec/volumeMode", Text), ("/spec/storageClassName", Text)],
        Services => &[
            ("/spec/type", Text),
            ("/spec/externalTrafficPolicy", Text),
            ("/spec/internalTrafficPolicy", Text),
            ("/spec/sessionAffinity", Text),
            ("/spec/allocateLoadBalancerNodePorts", Bool),
        ],
        StorageClasses => &[
            ("/reclaimPolicy", Text),
            ("/volumeBindingMode", Text),
            ("/allowVolumeExpansion", Bool),
        ],
        _ => &[],
    }
}
fn containers(kind: ResourceKind) -> bool {
    matches!(
        kind,
        ResourceKind::Deployments
            | ResourceKind::DaemonSets
            | ResourceKind::StatefulSets
            | ResourceKind::ReplicaSets
            | ResourceKind::ReplicationControllers
    )
}
fn safe_text(s: &str) -> bool {
    !s.is_empty() && s.len() <= 512 && s.chars().all(|c| !c.is_control())
}
fn scalar_valid(value: &Value, schema: Scalar) -> bool {
    match schema {
        Scalar::Count => value.as_u64().is_some_and(|n| n <= i32::MAX as u64),
        Scalar::Bool => value.is_boolean(),
        Scalar::Text => value.as_str().is_some_and(safe_text),
        Scalar::IntOrPercent => {
            scalar_valid(value, Scalar::Count)
                || value.as_str().is_some_and(|s| {
                    s.strip_suffix('%').is_some_and(|p| {
                        !p.is_empty()
                            && p.len() <= 3
                            && p.bytes().all(|b| b.is_ascii_digit())
                            && p.parse::<u16>().is_ok_and(|n| n <= 100)
                    })
                })
        }
    }
}
fn insert(root: &mut Value, path: &str, value: Value) {
    let mut node = root;
    let keys: Vec<_> = path.trim_start_matches('/').split('/').collect();
    for key in &keys[..keys.len() - 1] {
        node = node
            .as_object_mut()
            .unwrap()
            .entry(*key)
            .or_insert_with(|| json!({}));
    }
    node.as_object_mut()
        .unwrap()
        .insert(keys[keys.len() - 1].to_owned(), value);
}
fn verify(value: &Value, reference: &ResourceReference) -> Result<String> {
    let (api, kind) = super::create::creation_info(reference.kind);
    if value.get("apiVersion").and_then(Value::as_str) != Some(api)
        || value.get("kind").and_then(Value::as_str) != Some(kind)
        || value.pointer("/metadata/name").and_then(Value::as_str) != Some(reference.name.as_str())
        || value.pointer("/metadata/namespace").and_then(Value::as_str)
            != reference.namespace.as_deref()
        || value.pointer("/metadata/uid").and_then(Value::as_str) != reference.uid.as_deref()
    {
        return Err(ConnectionError::InvalidResponse);
    }
    value
        .pointer("/metadata/resourceVersion")
        .and_then(Value::as_str)
        .filter(|s| safe_text(s))
        .map(str::to_owned)
        .ok_or(ConnectionError::InvalidResponse)
}
fn projection(value: &Value, reference: &ResourceReference) -> Result<Value> {
    verify(value, reference)?;
    let mut out = json!({"apiVersion": value["apiVersion"], "kind": value["kind"], "metadata": {"name": value["metadata"]["name"], "uid": value["metadata"]["uid"], "resourceVersion": value["metadata"]["resourceVersion"]}});
    if let Some(ns) = &reference.namespace {
        out["metadata"]["namespace"] = json!(ns);
    }
    for &(path, schema) in fields(reference.kind) {
        if let Some(v) = value.pointer(path).filter(|v| !v.is_null()) {
            if !scalar_valid(v, schema) {
                return Err(ConnectionError::InvalidResponse);
            }
            insert(&mut out, path, v.clone());
        }
    }
    if containers(reference.kind)
        && let Some(list) = value.pointer("/spec/template/spec/containers")
    {
        let list = list
            .as_array()
            .filter(|a| !a.is_empty() && a.len() <= 128)
            .ok_or(ConnectionError::InvalidResponse)?;
        let mut reduced = Vec::new();
        for c in list {
            let name = c
                .get("name")
                .and_then(Value::as_str)
                .filter(|s| super::transport::valid_namespace(s))
                .ok_or(ConnectionError::InvalidResponse)?;
            let image = c
                .get("image")
                .and_then(Value::as_str)
                .filter(|s| safe_text(s))
                .ok_or(ConnectionError::InvalidResponse)?;
            reduced.push(json!({"name": name, "image": image}));
        }
        insert(&mut out, "/spec/template/spec/containers", json!(reduced));
    }
    Ok(out)
}
// Recursive closed shape validation rejects every key not in the sanitized baseline.
fn closed_shape(base: &Value, edited: &Value, depth: usize) -> bool {
    if depth > 12 {
        return false;
    }
    match (base, edited) {
        (Value::Object(b), Value::Object(e)) => e
            .iter()
            .all(|(k, v)| b.get(k).is_some_and(|old| closed_shape(old, v, depth + 1))),
        (Value::Array(b), Value::Array(e)) => {
            b.len() == e.len() && b.iter().zip(e).all(|(b, e)| closed_shape(b, e, depth + 1))
        }
        (
            Value::String(_) | Value::Number(_) | Value::Bool(_),
            Value::String(_) | Value::Number(_) | Value::Bool(_),
        ) => true,
        _ => false,
    }
}
fn patch(current: &Value, edited: &Value, r: &EditRequest) -> Result<Value> {
    let baseline = projection(current, &r.reference)?;
    if !closed_shape(&baseline, edited, 0)
        || edited.get("metadata") != baseline.get("metadata")
        || edited.get("apiVersion") != baseline.get("apiVersion")
        || edited.get("kind") != baseline.get("kind")
    {
        return Err(ConnectionError::InvalidInput);
    }
    let mut ops = vec![
        json!({"op":"test","path":"/metadata/uid","value":r.reference.uid}),
        json!({"op":"test","path":"/metadata/resourceVersion","value":r.resource_version}),
    ];
    for &(path, schema) in fields(r.reference.kind) {
        let old = baseline.pointer(path);
        let new = edited.pointer(path);
        if let Some(v) = new
            && !scalar_valid(v, schema)
        {
            return Err(ConnectionError::InvalidInput);
        }
        if old != new {
            match new {
                Some(v) => ops.push(json!({"op":"replace","path":path,"value":v})),
                None => ops.push(json!({"op":"remove","path":path})),
            }
        }
    }
    let path = "/spec/template/spec/containers";
    if let Some(list) = baseline.pointer(path).and_then(Value::as_array) {
        let edited = edited
            .pointer(path)
            .and_then(Value::as_array)
            .filter(|e| e.len() == list.len())
            .ok_or(ConnectionError::InvalidInput)?;
        for (i, (old, new)) in list.iter().zip(edited).enumerate() {
            if old.get("name") != new.get("name")
                || !new
                    .get("image")
                    .is_some_and(|v| scalar_valid(v, Scalar::Text))
            {
                return Err(ConnectionError::InvalidInput);
            }
            if old["image"] != new["image"] {
                ops.push(
                    json!({"op":"test","path":format!("{path}/{i}/name"),"value":old["name"]}),
                );
                ops.push(
                    json!({"op":"test","path":format!("{path}/{i}/image"),"value":old["image"]}),
                );
                ops.push(
                    json!({"op":"replace","path":format!("{path}/{i}/image"),"value":new["image"]}),
                );
            }
        }
    }
    if ops.len() == 2 {
        return Err(ConnectionError::InvalidInput);
    }
    Ok(json!(ops))
}
struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
impl ConnectionEngine {
    pub fn cancel_resource_edit(&self, r: EditCancelRequest) -> Result<()> {
        super::validate_read_id(r.operation_id)?;
        let id = SessionRequest {
            cluster_id: r.cluster_id,
            session_id: r.session_id,
        };
        super::validate_request(&id)?;
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        if let Some(s) = state.as_mut().filter(|s| super::matches_session(s, &id)) {
            s.resource_edit.cancel(r.operation_id);
        }
        Ok(())
    }
    pub async fn resource_edit(&self, r: EditRequest) -> Result<EditResult> {
        super::validate_request(&r.session())?;
        super::validate_read_id(r.operation_id)?;
        super::details::validate_reference(&r.reference)?;
        if fields(r.reference.kind).is_empty() || !r.reference.uid.as_deref().is_some_and(safe_text)
        {
            return Err(ConnectionError::InvalidInput);
        }
        let edited = match r.mode {
            EditMode::Read => {
                if r.yaml.is_some() || r.resource_version.is_some() {
                    return Err(ConnectionError::InvalidInput);
                }
                None
            }
            _ => {
                if !r.resource_version.as_deref().is_some_and(safe_text) {
                    return Err(ConnectionError::InvalidInput);
                }
                let yaml = r
                    .yaml
                    .as_deref()
                    .filter(|s| !s.is_empty() && s.len() <= YAML_LIMIT && !s.contains('\0'))
                    .ok_or(ConnectionError::InvalidInput)?;
                Some(
                    serde_yaml_ng::from_str::<Value>(yaml)
                        .map_err(|_| ConnectionError::InvalidInput)?,
                )
            }
        };
        let _permit = self
            .workers
            .try_acquire()
            .map_err(|_| ConnectionError::Busy)?;
        let identity = r.session();
        let (client, cancel) = {
            let mut state = self
                .session
                .lock()
                .map_err(|_| ConnectionError::StoreUnavailable)?;
            let s = super::current_mut(&mut state, &identity)?;
            let client = s.client.clone().ok_or(ConnectionError::NotConnected)?;
            super::validate_credentials(&client)?;
            let cancel = s.cancel.child_token();
            s.resource_edit.start(r.operation_id, cancel.clone())?;
            (client, cancel)
        };
        let _guard = CancelOnDrop(cancel.clone());
        let result = super::bounded(&cancel, async {
            let mut checks = vec![PermissionCheck {
                kind: r.reference.kind,
                action: PermissionAction::Get,
                namespace: r.reference.namespace.clone(),
                name: Some(r.reference.name.clone()),
            }];
            if r.mode != EditMode::Read {
                checks.push(PermissionCheck {
                    action: PermissionAction::Patch,
                    ..checks[0].clone()
                });
            }
            let epoch = self.require_permissions(&identity, &checks).await?;
            let client = super::permissions::authorized_client(client, epoch.clone());
            let result = run(&client, &cancel, &r, edited.as_ref()).await?;
            epoch.validate()?;
            Ok(result)
        })
        .await;
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        let s = super::current_mut(&mut state, &identity)?;
        if s.resource_edit.active_id != r.operation_id
            || cancel.is_cancelled() && result != Err(ConnectionError::Timeout)
        {
            return Err(ConnectionError::StaleSession);
        }
        s.resource_edit.cancel = None;
        result
    }
}
async fn run(
    client: &Client,
    cancel: &CancellationToken,
    r: &EditRequest,
    edited: Option<&Value>,
) -> Result<EditResult> {
    let path = super::details::named_path(&r.reference)?;
    let current = send(client, cancel, Method::GET, &path, None).await?;
    let mut rv = verify(&current, &r.reference)?;
    let yaml = if r.mode == EditMode::Read {
        let yaml = serde_yaml_ng::to_string(&projection(&current, &r.reference)?)
            .map_err(|_| ConnectionError::InvalidResponse)?;
        if yaml.len() > YAML_LIMIT {
            return Err(ConnectionError::LimitExceeded);
        }
        Some(yaml)
    } else {
        if r.resource_version.as_deref() != Some(rv.as_str()) {
            return Err(ConnectionError::Conflict);
        }
        let body = patch(&current, edited.ok_or(ConnectionError::InvalidInput)?, r)?;
        let path = if r.mode == EditMode::DryRun {
            format!("{path}?dryRun=All&fieldValidation=Strict")
        } else {
            format!("{path}?fieldValidation=Strict")
        };
        let response = send(client, cancel, Method::PATCH, &path, Some(body)).await?;
        rv = verify(&response, &r.reference)?;
        None
    };
    Ok(EditResult {
        cluster_id: r.cluster_id.clone(),
        session_id: r.session_id.clone(),
        operation_id: r.operation_id,
        reference: r.reference.clone(),
        resource_version: rv,
        yaml,
        mode: r.mode,
        accepted: r.mode != EditMode::Read,
    })
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
    if bytes.len() > YAML_LIMIT {
        return Err(ConnectionError::LimitExceeded);
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
        200 => (),
        400 | 422 => return Err(ConnectionError::InvalidInput),
        401 => return Err(ConnectionError::AuthFailed),
        403 => return Err(ConnectionError::PermissionDenied),
        404 => return Err(ConnectionError::NotFound),
        409 => return Err(ConnectionError::Conflict),
        408 | 504 => return Err(ConnectionError::Timeout),
        413 => return Err(ConnectionError::LimitExceeded),
        429 => return Err(ConnectionError::Busy),
        _ => return Err(ConnectionError::InvalidResponse),
    }
    if response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .is_some_and(|n| n > BODY_LIMIT)
    {
        return Err(ConnectionError::LimitExceeded);
    }
    let mut bytes = Vec::new();
    while let Some(frame) = response.body_mut().frame().await {
        let data = frame
            .map_err(|_| ConnectionError::Network)?
            .into_data()
            .map_err(|_| ConnectionError::InvalidResponse)?;
        if data.len() > BODY_LIMIT.saturating_sub(bytes.len()) {
            return Err(ConnectionError::LimitExceeded);
        }
        bytes.extend_from_slice(&data);
    }
    serde_json::from_slice(&bytes).map_err(|_| ConnectionError::InvalidResponse)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn integer_percentage_changes_use_reviewed_scalar_schema() {
        let (r, mut raw) = fixture();
        raw["spec"]["strategy"] =
            json!({"type":"RollingUpdate","rollingUpdate":{"maxSurge":1,"maxUnavailable":"25%"}});
        let mut edited = projection(&raw, &r.reference).unwrap();
        edited["spec"]["strategy"]["rollingUpdate"]["maxSurge"] = json!("30%");
        edited["spec"]["strategy"]["rollingUpdate"]["maxUnavailable"] = json!(0);
        let ops = patch(&raw, &edited, &r).unwrap();
        let ops = ops.as_array().unwrap();
        assert!(ops.iter().any(
            |op| op["path"] == "/spec/strategy/rollingUpdate/maxSurge" && op["value"] == "30%"
        ));
        assert!(ops.iter().any(
            |op| op["path"] == "/spec/strategy/rollingUpdate/maxUnavailable" && op["value"] == 0
        ));
        edited["spec"]["replicas"] = json!("30%");
        assert_eq!(patch(&raw, &edited, &r), Err(ConnectionError::InvalidInput));
    }
    pub(super) fn fixture() -> (EditRequest, Value) {
        (
            EditRequest {
                cluster_id: "fixture".into(),
                session_id: "session".into(),
                operation_id: 1,
                reference: ResourceReference {
                    kind: ResourceKind::Deployments,
                    namespace: Some("default".into()),
                    name: "demo".into(),
                    uid: Some("uid".into()),
                },
                resource_version: Some("7".into()),
                yaml: None,
                mode: EditMode::DryRun,
            },
            json!({"apiVersion":"apps/v1","kind":"Deployment","metadata":{"name":"demo","namespace":"default","uid":"uid","resourceVersion":"7","annotations":{"token":"SECRET"}},"spec":{"replicas":2,"template":{"spec":{"containers":[{"name":"main","image":"demo:v1","env":[{"name":"TOKEN","value":"SECRET"}],"volumeMounts":[]}],"imagePullSecrets":[{"name":"SECRET"}]}}},"status":{"secret":"SECRET"}}),
        )
    }
    #[test]
    fn projection_withholds_unreviewed_fields() {
        let (r, raw) = fixture();
        let reduced = projection(&raw, &r.reference).unwrap();
        assert!(!reduced.to_string().contains("SECRET"));
        assert!(
            reduced
                .pointer("/spec/template/spec/containers/0/env")
                .is_none()
        );
    }
    #[test]
    fn leaf_patch_never_replaces_container_or_spec() {
        let (r, raw) = fixture();
        let mut edited = projection(&raw, &r.reference).unwrap();
        edited["spec"]["replicas"] = json!(3);
        edited["spec"]["template"]["spec"]["containers"][0]["image"] = json!("demo:v2");
        let ops = patch(&raw, &edited, &r).unwrap();
        let ops = ops.as_array().unwrap();
        assert_eq!(ops.len(), 6);
        assert_eq!(ops[4]["path"], "/spec/template/spec/containers/0/image");
        assert_eq!(ops[5]["op"], "replace");
        assert_eq!(ops[5]["path"], "/spec/template/spec/containers/0/image");
        assert!(
            ops.iter()
                .all(|op| op["path"] != "/spec/template/spec/containers" && op["path"] != "/spec")
        );
    }
    #[test]
    fn rejects_unknown_keys_identity_changes_arrays_and_types() {
        let (r, raw) = fixture();
        let base = projection(&raw, &r.reference).unwrap();
        for path in [
            "/metadata/annotations",
            "/spec/token",
            "/spec/template/spec/containers/0/env",
        ] {
            let mut edited = base.clone();
            // Construct an extra-key payload without ever forwarding it.
            if path == "/metadata/annotations" {
                edited["metadata"]["annotations"] = json!({"x":"y"});
            } else if path == "/spec/token" {
                edited["spec"]["token"] = json!("x");
            } else {
                edited["spec"]["template"]["spec"]["containers"][0]["env"] = json!([]);
            }
            assert_eq!(patch(&raw, &edited, &r), Err(ConnectionError::InvalidInput));
        }
        let mut edited = base.clone();
        edited["metadata"]["uid"] = json!("other");
        assert!(patch(&raw, &edited, &r).is_err());
        let mut edited = base.clone();
        edited["spec"]["replicas"] = json!(-1);
        assert!(patch(&raw, &edited, &r).is_err());
        let mut edited = base.clone();
        edited["spec"]["template"]["spec"]["containers"] = json!([]);
        assert!(patch(&raw, &edited, &r).is_err());
        let mut edited = base;
        edited["spec"]["template"]["spec"]["containers"][0]["name"] = json!("other");
        assert!(patch(&raw, &edited, &r).is_err());
    }
    #[test]
    fn omission_removes_only_reviewed_leaf() {
        let (r, raw) = fixture();
        let mut edited = projection(&raw, &r.reference).unwrap();
        edited["spec"].as_object_mut().unwrap().remove("replicas");
        assert_eq!(
            patch(&raw, &edited, &r).unwrap()[2],
            json!({"op":"remove","path":"/spec/replicas"})
        );
    }
    #[test]
    fn cancellation_tombstones_and_does_not_cancel_other_operation() {
        let mut slot = EditSlot::default();
        slot.cancel(2);
        assert_eq!(
            slot.start(2, CancellationToken::new()),
            Err(ConnectionError::StaleSession)
        );
        let token = CancellationToken::new();
        slot.start(3, token.clone()).unwrap();
        slot.cancel(2);
        assert!(!token.is_cancelled());
        slot.cancel(3);
        assert!(token.is_cancelled());
    }
    #[test]
    fn sensitive_and_unreviewed_kinds_are_denied() {
        for kind in [
            ResourceKind::Pods,
            ResourceKind::Secrets,
            ResourceKind::ConfigMaps,
            ResourceKind::MutatingWebhookConfigurations,
            ResourceKind::CustomResourceDefinitions,
        ] {
            assert!(fields(kind).is_empty());
        }
    }
}
