use super::permissions::{PermissionAction, PermissionCheck};
use super::{ConnectionEngine, ConnectionError, ResourceKind, Result, SessionRequest};
use http::{Method, Request};
use http_body_util::BodyExt;
use kube::{Client, client::Body};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

const BODY_LIMIT: usize = 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PodRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub namespace: String,
    pub name: String,
    pub uid: Option<String>,
}
impl PodRequest {
    fn session(&self) -> SessionRequest {
        SessionRequest {
            cluster_id: self.cluster_id.clone(),
            session_id: self.session_id.clone(),
        }
    }
    fn path(&self) -> String {
        format!("/api/v1/namespaces/{}/pods/{}", self.namespace, self.name)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodDetails {
    pub cluster_id: String,
    pub session_id: String,
    pub namespace: String,
    pub name: String,
    pub uid: String,
    pub resource_version: String,
    pub phase: String,
    pub node: Option<String>,
    pub pod_ips: Vec<String>,
    pub host_ips: Vec<String>,
    pub service_account: Option<String>,
    pub qos: Option<String>,
    pub created_at: Option<String>,
    pub deletion_timestamp: Option<String>,
    pub containers: Vec<PodContainer>,
    pub conditions: Vec<PodCondition>,
    pub yaml: String,
    pub events: Vec<PodEvent>,
    pub events_state: PodEventsState,
}
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PodEventsState {
    Complete,
    Denied,
    Unavailable,
    Partial,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodContainer {
    pub name: String,
    pub image: String,
    pub kind: &'static str,
    pub state: String,
    pub ready: bool,
    pub restarts: u32,
    pub ports: Vec<PodPort>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodPort {
    pub name: Option<String>,
    pub port: u16,
    pub protocol: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct PodCondition {
    #[serde(rename = "type")]
    pub kind: String,
    pub status: String,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodEvent {
    #[serde(rename = "type")]
    pub kind: String,
    pub reason: String,
    pub count: u32,
    pub last_seen: Option<String>,
}
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PodAction {
    Delete,
    ForceDelete,
    Finalize,
    Replace,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PodMutationRequest {
    #[serde(flatten)]
    pub pod: PodRequest,
    pub action: PodAction,
    pub resource_version: String,
    pub yaml: Option<String>,
}

pub(super) fn validate_pod_request(request: &PodRequest) -> Result<()> {
    fn dns(value: &str, limit: usize, dots: bool) -> bool {
        !value.is_empty()
            && value.len() <= limit
            && value.split('.').all(|part| {
                !part.is_empty()
                    && (dots || part.len() == value.len())
                    && part
                        .as_bytes()
                        .first()
                        .is_some_and(u8::is_ascii_alphanumeric)
                    && part
                        .as_bytes()
                        .last()
                        .is_some_and(u8::is_ascii_alphanumeric)
                    && part
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            })
    }
    super::validate_request(&request.session())?;
    if !dns(&request.namespace, 63, false)
        || !dns(&request.name, 253, true)
        || request.uid.as_ref().is_some_and(|uid| !safe_identity(uid))
    {
        return Err(ConnectionError::InvalidInput);
    }
    Ok(())
}
fn safe_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}
fn required(value: &Value, pointer: &str) -> Result<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or(ConnectionError::InvalidResponse)
}
fn text(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::to_owned)
}
fn array<'a>(value: &'a Value, pointer: &str) -> &'a [Value] {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}
fn verify_identity(value: &Value, request: &PodRequest) -> Result<()> {
    if required(value, "/apiVersion")? != "v1"
        || required(value, "/kind")? != "Pod"
        || required(value, "/metadata/name")? != request.name
        || required(value, "/metadata/namespace")? != request.namespace
    {
        return Err(ConnectionError::InvalidResponse);
    }
    let uid = required(value, "/metadata/uid")?;
    if !safe_identity(&uid) {
        return Err(ConnectionError::InvalidResponse);
    }
    if request
        .uid
        .as_ref()
        .is_some_and(|expected| expected != &uid)
    {
        return Err(ConnectionError::Conflict);
    }
    required(value, "/metadata/resourceVersion")?;
    Ok(())
}

impl ConnectionEngine {
    pub async fn pod_detail(&self, request: PodRequest) -> Result<PodDetails> {
        validate_pod_request(&request)?;
        let session = request.session();
        let _permit = self
            .workers
            .try_acquire()
            .map_err(|_| ConnectionError::Busy)?;
        let (client, cancel) = self.pod_client(&session)?;
        let operation = async {
            let pod = send_json(
                &client,
                &cancel,
                Method::GET,
                &request.path(),
                None,
                "application/json",
            )
            .await?;
            verify_identity(&pod, &request)?;
            let uid = required(&pod, "/metadata/uid")?;
            let path = format!(
                "/api/v1/namespaces/{}/events?fieldSelector=involvedObject.uid%3D{}&limit=100&timeoutSeconds=10",
                request.namespace, uid
            );
            let events = tokio::time::timeout(
                std::time::Duration::from_secs(3),
                send_json(
                    &client,
                    &cancel,
                    Method::GET,
                    &path,
                    None,
                    "application/json",
                ),
            )
            .await
            .unwrap_or(Err(ConnectionError::Timeout));
            let mut details = project(&pod, &json!({"items":[]}), request.clone())?;
            match events {
                Ok(events) => match project(&pod, &events, request) {
                    Ok(projected) => details = projected,
                    Err(ConnectionError::LimitExceeded) => {
                        details.events_state = PodEventsState::Partial
                    }
                    Err(_) => details.events_state = PodEventsState::Unavailable,
                },
                Err(ConnectionError::PermissionDenied) => {
                    details.events_state = PodEventsState::Denied
                }
                Err(ConnectionError::StaleSession) => return Err(ConnectionError::StaleSession),
                Err(_) => details.events_state = PodEventsState::Unavailable,
            }
            self.check_pod_session(&session)?;
            Ok(details)
        };
        super::bounded(&cancel, operation).await
    }

    pub async fn pod_mutate(&self, request: PodMutationRequest) -> Result<()> {
        validate_pod_request(&request.pod)?;
        if request.pod.uid.is_none()
            || !safe_identity(&request.resource_version)
            || (request.action == PodAction::Replace) != request.yaml.is_some()
        {
            return Err(ConnectionError::InvalidInput);
        }
        if request
            .yaml
            .as_ref()
            .is_some_and(|yaml| yaml.len() > BODY_LIMIT)
        {
            return Err(ConnectionError::LimitExceeded);
        }
        if let Some(yaml) = &request.yaml {
            let edited: Value =
                serde_yaml_ng::from_str(yaml).map_err(|_| ConnectionError::InvalidInput)?;
            if edited["apiVersion"] != "v1"
                || edited["kind"] != "Pod"
                || edited.pointer("/metadata/name").and_then(Value::as_str)
                    != Some(request.pod.name.as_str())
                || edited
                    .pointer("/metadata/namespace")
                    .and_then(Value::as_str)
                    != Some(request.pod.namespace.as_str())
                || edited.pointer("/metadata/uid").and_then(Value::as_str)
                    != request.pod.uid.as_deref()
                || edited
                    .pointer("/metadata/resourceVersion")
                    .and_then(Value::as_str)
                    != Some(request.resource_version.as_str())
            {
                return Err(ConnectionError::InvalidInput);
            }
        }
        let session = request.pod.session();
        let action = match request.action {
            PodAction::Delete | PodAction::ForceDelete => PermissionAction::Delete,
            PodAction::Finalize => PermissionAction::Patch,
            PodAction::Replace => PermissionAction::Update,
        };
        let authorization = self
            .require_permissions(
                &session,
                &[
                    PermissionCheck {
                        kind: ResourceKind::Pods,
                        action: PermissionAction::Get,
                        namespace: Some(request.pod.namespace.clone()),
                        name: Some(request.pod.name.clone()),
                    },
                    PermissionCheck {
                        kind: ResourceKind::Pods,
                        action,
                        namespace: Some(request.pod.namespace.clone()),
                        name: Some(request.pod.name.clone()),
                    },
                ],
            )
            .await?;
        let _permit = self
            .workers
            .try_acquire()
            .map_err(|_| ConnectionError::Busy)?;
        let (client, cancel) = self.pod_client(&session)?;
        let client = super::permissions::authorized_client(client, authorization);
        let operation = async {
            let path = request.pod.path();
            let current = send_json(
                &client,
                &cancel,
                Method::GET,
                &path,
                None,
                "application/json",
            )
            .await?;
            verify_identity(&current, &request.pod)?;
            if required(&current, "/metadata/resourceVersion")? != request.resource_version {
                return Err(ConnectionError::Conflict);
            }
            let preconditions =
                json!({"uid": request.pod.uid, "resourceVersion": request.resource_version});
            let (method, body, content_type) = match request.action {
                PodAction::Delete | PodAction::ForceDelete => {
                    let mut options = json!({"apiVersion":"v1", "kind":"DeleteOptions", "preconditions": preconditions});
                    if request.action == PodAction::ForceDelete {
                        options["gracePeriodSeconds"] = json!(0);
                    }
                    (Method::DELETE, options, "application/json")
                }
                PodAction::Finalize => (
                    Method::PATCH,
                    json!([
                        {"op":"test", "path":"/metadata/uid", "value":request.pod.uid},
                        {"op":"test", "path":"/metadata/resourceVersion", "value":request.resource_version},
                        {"op":"add", "path":"/metadata/finalizers", "value":[]}
                    ]),
                    "application/json-patch+json",
                ),
                PodAction::Replace => (
                    Method::PUT,
                    replacement(&current, &request)?,
                    "application/json",
                ),
            };
            self.check_pod_session(&session)?;
            send_json(&client, &cancel, method, &path, Some(body), content_type).await?;
            self.check_pod_session(&session)
        };
        super::bounded(&cancel, operation).await
    }
}

pub(super) async fn send_json(
    client: &Client,
    cancel: &CancellationToken,
    method: Method,
    path: &str,
    body: Option<Value>,
    content_type: &str,
) -> Result<Value> {
    let bytes = body
        .map(|value| serde_json::to_vec(&value))
        .transpose()
        .map_err(|_| ConnectionError::InvalidInput)?
        .unwrap_or_default();
    if bytes.len() > BODY_LIMIT {
        return Err(ConnectionError::LimitExceeded);
    }
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("Accept", "application/json")
        .header("Content-Type", content_type)
        .body(Body::from(bytes))
        .map_err(|_| ConnectionError::InvalidInput)?;
    request.extensions_mut().insert(cancel.clone());
    let mut response = client.send(request).await.map_err(map_pod_error)?;
    match response.status().as_u16() {
        200..=299 => {}
        status => return Err(map_status(status)),
    }
    if response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .is_some_and(|size| size > BODY_LIMIT)
    {
        return Err(ConnectionError::LimitExceeded);
    }
    let mut bytes = Vec::new();
    while let Some(frame) = response.body_mut().frame().await {
        if let Ok(data) = frame.map_err(map_pod_error)?.into_data() {
            if data.len() > BODY_LIMIT.saturating_sub(bytes.len()) {
                return Err(ConnectionError::LimitExceeded);
            }
            bytes.extend_from_slice(&data);
        }
    }
    if bytes.is_empty() && response.status() == http::StatusCode::NO_CONTENT {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes).map_err(|_| ConnectionError::InvalidResponse)
}
fn map_status(status: u16) -> ConnectionError {
    match status {
        400 | 422 => ConnectionError::InvalidInput,
        401 => ConnectionError::AuthFailed,
        403 => ConnectionError::PermissionDenied,
        404 => ConnectionError::NotFound,
        409 => ConnectionError::Conflict,
        408 | 504 => ConnectionError::Timeout,
        413 => ConnectionError::LimitExceeded,
        429 => ConnectionError::Busy,
        _ => ConnectionError::ConnectionFailed,
    }
}
pub(super) fn map_pod_error(error: kube::Error) -> ConnectionError {
    match error {
        kube::Error::Api(error) => map_status(error.code),
        kube::Error::Auth(_) => ConnectionError::AuthFailed,
        kube::Error::RustlsTls(_) | kube::Error::TlsRequired => ConnectionError::TlsFailed,
        kube::Error::Service(error) => {
            let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(error.as_ref());
            for _ in 0..16 {
                let Some(current) = cause else {
                    break;
                };
                if current.is::<rustls::Error>() {
                    return ConnectionError::TlsFailed;
                }
                if current
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::TimedOut)
                {
                    return ConnectionError::Timeout;
                }
                cause = current.source();
            }
            ConnectionError::Network
        }
        _ => ConnectionError::Network,
    }
}

// This is an editable, deliberately reduced Pod view, not an export. Omitted fields
// (including annotations, env, commands, volumes and credentials) stay on the server.
fn yaml_view(pod: &Value) -> Value {
    let mut view = json!({"apiVersion":"v1", "kind":"Pod", "metadata":{}, "spec":{}});
    for key in [
        "name",
        "namespace",
        "uid",
        "resourceVersion",
        "creationTimestamp",
        "deletionTimestamp",
        "finalizers",
    ] {
        if let Some(value) = pod["metadata"].get(key) {
            view["metadata"][key] = value.clone();
        }
    }
    for key in [
        "nodeName",
        "serviceAccountName",
        "restartPolicy",
        "dnsPolicy",
        "schedulerName",
        "hostNetwork",
    ] {
        if let Some(value) = pod["spec"].get(key) {
            view["spec"][key] = value.clone();
        }
    }
    for key in ["containers", "initContainers", "ephemeralContainers"] {
        if let Some(containers) = pod["spec"].get(key).and_then(Value::as_array) {
            view["spec"][key] = Value::Array(
                containers
                    .iter()
                    .map(|container| {
                        let mut safe = json!({});
                        for key in ["name", "image", "imagePullPolicy", "ports", "resources"] {
                            if let Some(value) = container.get(key) {
                                safe[key] = value.clone();
                            }
                        }
                        safe
                    })
                    .collect(),
            );
        }
    }
    view
}

fn replacement(current: &Value, request: &PodMutationRequest) -> Result<Value> {
    let yaml = request.yaml.as_ref().ok_or(ConnectionError::InvalidInput)?;
    let edited: Value = serde_yaml_ng::from_str(yaml).map_err(|_| ConnectionError::InvalidInput)?;
    let original = yaml_view(current);
    // A restricted view cannot add fields: this prevents edited YAML injecting
    // credentials or redaction markers into fields the renderer was never given.
    fn merge(live: &mut Value, safe: &Value, edited: &Value) -> Result<()> {
        match (safe, edited) {
            (Value::Object(safe), Value::Object(edited)) => {
                if safe.len() != edited.len() || edited.keys().any(|key| !safe.contains_key(key)) {
                    return Err(ConnectionError::InvalidInput);
                }
                for (key, value) in edited {
                    merge(&mut live[key], &safe[key], value)?;
                }
            }
            (Value::Array(safe), Value::Array(edited)) => {
                if safe.len() != edited.len() {
                    return Err(ConnectionError::InvalidInput);
                }
                for (index, value) in edited.iter().enumerate() {
                    merge(&mut live[index], &safe[index], value)?;
                }
            }
            (Value::String(_), Value::String(_))
            | (Value::Bool(_), Value::Bool(_))
            | (Value::Number(_), Value::Number(_))
            | (Value::Null, Value::Null) => *live = edited.clone(),
            _ => return Err(ConnectionError::InvalidInput),
        }
        Ok(())
    }
    for pointer in [
        "/apiVersion",
        "/kind",
        "/metadata/name",
        "/metadata/namespace",
        "/metadata/uid",
        "/metadata/resourceVersion",
        "/metadata/creationTimestamp",
        "/metadata/deletionTimestamp",
    ] {
        if edited.pointer(pointer) != original.pointer(pointer) {
            return Err(ConnectionError::InvalidInput);
        }
    }
    for key in ["containers", "initContainers", "ephemeralContainers"] {
        let pointer = format!("/spec/{key}");
        let expected = array(&original, &pointer);
        let actual = array(&edited, &pointer);
        if expected.len() != actual.len()
            || expected
                .iter()
                .zip(actual)
                .any(|(a, b)| a.get("name") != b.get("name"))
        {
            return Err(ConnectionError::InvalidInput);
        }
    }
    let mut merged = current.clone();
    merge(&mut merged, &original, &edited)?;
    Ok(merged)
}

fn project(pod: &Value, events: &Value, request: PodRequest) -> Result<PodDetails> {
    let mut containers = Vec::new();
    for (spec_key, status_key, kind) in [
        ("containers", "containerStatuses", "container"),
        ("initContainers", "initContainerStatuses", "init"),
        (
            "ephemeralContainers",
            "ephemeralContainerStatuses",
            "ephemeral",
        ),
    ] {
        for container in array(pod, &format!("/spec/{spec_key}")) {
            let name = required(container, "/name")?;
            let status = array(pod, &format!("/status/{status_key}"))
                .iter()
                .find(|s| s["name"].as_str() == Some(&name));
            let state = status
                .and_then(|s| s["state"].as_object())
                .and_then(|s| {
                    ["running", "waiting", "terminated"]
                        .into_iter()
                        .find(|key| s.contains_key(*key))
                })
                .unwrap_or("unknown");
            let ports = array(container, "/ports")
                .iter()
                .map(|port| {
                    let number = port["containerPort"]
                        .as_u64()
                        .and_then(|n| u16::try_from(n).ok())
                        .filter(|n| *n > 0)
                        .ok_or(ConnectionError::InvalidResponse)?;
                    Ok(PodPort {
                        name: text(port, "/name"),
                        port: number,
                        protocol: text(port, "/protocol").unwrap_or_else(|| "TCP".into()),
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            containers.push(PodContainer {
                name,
                image: required(container, "/image")?,
                kind,
                state: state.into(),
                ready: status.and_then(|s| s["ready"].as_bool()).unwrap_or(false),
                restarts: status
                    .and_then(|s| s["restartCount"].as_u64())
                    .and_then(|n| u32::try_from(n).ok())
                    .unwrap_or(0),
                ports,
            });
        }
    }
    let conditions = array(pod, "/status/conditions")
        .iter()
        .map(|c| {
            Ok(PodCondition {
                kind: required(c, "/type")?,
                status: required(c, "/status")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let uid = required(pod, "/metadata/uid")?;
    let event_items = events
        .get("items")
        .and_then(Value::as_array)
        .ok_or(ConnectionError::InvalidResponse)?;
    if event_items.len() > 100
        || events
            .pointer("/metadata/continue")
            .and_then(Value::as_str)
            .is_some_and(|token| !token.is_empty())
    {
        return Err(ConnectionError::LimitExceeded);
    }
    let events = event_items
        .iter()
        .filter(|e| e.pointer("/involvedObject/uid").and_then(Value::as_str) == Some(&uid))
        .map(|e| {
            Ok(PodEvent {
                kind: required(e, "/type")?,
                reason: required(e, "/reason")?,
                count: e["count"]
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .unwrap_or(1),
                last_seen: text(e, "/lastTimestamp").or_else(|| text(e, "/eventTime")),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let yaml = format!(
        "# Reduced editable Pod view. Omitted fields are preserved from the live Pod.\n# Annotations, env, commands, volumes and credentials cannot be edited here.\n{}",
        serde_yaml_ng::to_string(&yaml_view(pod)).map_err(|_| ConnectionError::InvalidResponse)?
    );
    if yaml.len() > BODY_LIMIT {
        return Err(ConnectionError::LimitExceeded);
    }
    fn ips(pod: &Value, plural: &str, singular: &str) -> Vec<String> {
        let mut ips: Vec<_> = array(pod, plural)
            .iter()
            .filter_map(|ip| text(ip, "/ip"))
            .collect();
        if ips.is_empty()
            && let Some(ip) = text(pod, singular)
        {
            ips.push(ip);
        }
        ips
    }
    Ok(PodDetails {
        cluster_id: request.cluster_id,
        session_id: request.session_id,
        namespace: request.namespace,
        name: request.name,
        uid,
        resource_version: required(pod, "/metadata/resourceVersion")?,
        phase: text(pod, "/status/phase").unwrap_or_else(|| "Unknown".into()),
        node: text(pod, "/spec/nodeName"),
        pod_ips: ips(pod, "/status/podIPs", "/status/podIP"),
        host_ips: ips(pod, "/status/hostIPs", "/status/hostIP"),
        service_account: text(pod, "/spec/serviceAccountName"),
        qos: text(pod, "/status/qosClass"),
        created_at: text(pod, "/metadata/creationTimestamp"),
        deletion_timestamp: text(pod, "/metadata/deletionTimestamp"),
        containers,
        conditions,
        yaml,
        events,
        events_state: PodEventsState::Complete,
    })
}

#[cfg(test)]
#[path = "pod_tests.rs"]
mod tests;
