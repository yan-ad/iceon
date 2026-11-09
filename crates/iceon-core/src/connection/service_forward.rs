use super::{
    ConnectionEngine, ConnectionError, ResourceKind, Result, SessionRequest,
    details::{ResourceReference, named_path, validate_reference},
    permissions::{PermissionAction, PermissionCheck, authorized_client},
    pod_runtime::{PodForward, PodForwardRequest, PodForwardStopRequest},
};
use http::Request;
use http_body_util::BodyExt;
use kube::{Client, client::Body};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

const BODY_LIMIT: usize = 1024 * 1024;
const POD_LIMIT: usize = 200;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceForwardRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub operation_id: u64,
    pub reference: ResourceReference,
    pub resource_version: String,
    pub service_port: u16,
    pub local_port: u16,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceForwardCancel {
    pub cluster_id: String,
    pub session_id: String,
    pub operation_id: u64,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceForwardResult {
    pub cluster_id: String,
    pub session_id: String,
    pub reference: ResourceReference,
    pub service_port: u16,
    pub pod: ResourceReference,
    pub forward: PodForward,
}
#[derive(Default)]
pub(super) struct ServiceForwardSlot {
    high_water: u64,
    id: u64,
    cancel: Option<CancellationToken>,
}
impl ServiceForwardSlot {
    pub(super) fn active(&self) -> bool {
        self.cancel.is_some()
    }
    fn begin(&mut self, id: u64, token: CancellationToken) -> Result<()> {
        if id <= self.high_water {
            return Err(ConnectionError::StaleSession);
        }
        if self.active() {
            return Err(ConnectionError::Busy);
        }
        self.high_water = id;
        self.id = id;
        self.cancel = Some(token);
        Ok(())
    }
    fn cancel(&mut self, id: u64) {
        self.high_water = self.high_water.max(id);
        if self.id == id
            && let Some(token) = &self.cancel
        {
            token.cancel();
        }
    }
}
impl ServiceForwardRequest {
    fn session(&self) -> SessionRequest {
        SessionRequest {
            cluster_id: self.cluster_id.clone(),
            session_id: self.session_id.clone(),
        }
    }
}
struct Guard<'a> {
    engine: &'a ConnectionEngine,
    request: &'a ServiceForwardRequest,
    cancel: CancellationToken,
    forward: Option<String>,
}
impl Drop for Guard<'_> {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(id) = self.forward.take() {
            let _ = self.engine.pod_forward_stop(PodForwardStopRequest {
                cluster_id: self.request.cluster_id.clone(),
                session_id: self.request.session_id.clone(),
                id,
            });
        }
        if let Ok(mut state) = self.engine.session.lock()
            && let Ok(session) = super::current_mut(&mut state, &self.request.session())
            && session.service_forward.id == self.request.operation_id
        {
            session.service_forward.cancel = None;
        }
    }
}
async fn bounded<T>(
    token: &CancellationToken,
    future: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    tokio::select! { biased;
        _ = token.cancelled() => Err(ConnectionError::StaleSession),
        result = tokio::time::timeout(super::DEADLINE, future) => result.unwrap_or(Err(ConnectionError::Timeout)),
    }
}
fn check(
    kind: ResourceKind,
    action: PermissionAction,
    namespace: &Option<String>,
    name: Option<String>,
) -> PermissionCheck {
    PermissionCheck {
        kind,
        action,
        namespace: namespace.clone(),
        name,
    }
}
impl ConnectionEngine {
    pub fn service_forward_cancel(&self, request: ServiceForwardCancel) -> Result<()> {
        super::validate_read_id(request.operation_id)?;
        let scope = SessionRequest {
            cluster_id: request.cluster_id,
            session_id: request.session_id,
        };
        super::validate_request(&scope)?;
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        if let Some(session) = state.as_mut().filter(|s| super::matches_session(s, &scope)) {
            session.service_forward.cancel(request.operation_id);
        }
        Ok(())
    }
    pub async fn service_forward_start(
        &self,
        request: ServiceForwardRequest,
    ) -> Result<ServiceForwardResult> {
        let scope = request.session();
        super::validate_request(&scope)?;
        super::validate_read_id(request.operation_id)?;
        validate_reference(&request.reference)?;
        if request.reference.kind != ResourceKind::Services
            || request.reference.uid.is_none()
            || request.service_port == 0
            || !identity(&request.resource_version)
        {
            return Err(ConnectionError::InvalidInput);
        }
        let (client, cancel) = {
            let mut state = self
                .session
                .lock()
                .map_err(|_| ConnectionError::StoreUnavailable)?;
            let session = super::current_mut(&mut state, &scope)?;
            let client = session
                .client
                .clone()
                .ok_or(ConnectionError::NotConnected)?;
            super::validate_credentials(&client)?;
            let cancel = session.cancel.child_token();
            session
                .service_forward
                .begin(request.operation_id, cancel.clone())?;
            (client, cancel)
        };
        let mut guard = Guard {
            engine: self,
            request: &request,
            cancel: cancel.clone(),
            forward: None,
        };
        let ns = &request.reference.namespace;
        let epoch = bounded(
            &cancel,
            self.require_permissions(
                &scope,
                &[
                    check(
                        ResourceKind::Services,
                        PermissionAction::Get,
                        ns,
                        Some(request.reference.name.clone()),
                    ),
                    check(ResourceKind::Pods, PermissionAction::List, ns, None),
                ],
            ),
        )
        .await?;
        let client = authorized_client(client, epoch.clone());
        let (pod, _) = bounded(&cancel, async {
            let _permit = self
                .workers
                .try_acquire()
                .map_err(|_| ConnectionError::Busy)?;
            resolve(&client, &request, &cancel).await
        })
        .await?;
        bounded(
            &cancel,
            self.require_permissions(
                &scope,
                &[
                    check(
                        ResourceKind::Pods,
                        PermissionAction::Get,
                        ns,
                        Some(pod.name.clone()),
                    ),
                    check(
                        ResourceKind::Pods,
                        PermissionAction::PortForward,
                        ns,
                        Some(pod.name.clone()),
                    ),
                ],
            ),
        )
        .await?
        .validate()?;
        let remote_port = bounded(&cancel, async {
            let current = get(&client, &named_path(&pod)?, &cancel).await?;
            verify_object(&current, &pod, "Pod")?;
            let service = verify_service(&client, &request, &cancel).await?;
            let labels = service["spec"]["selector"]
                .as_object()
                .ok_or(ConnectionError::Conflict)?;
            if current["status"]["phase"] != "Running"
                || !current["status"]["conditions"]
                    .as_array()
                    .is_some_and(|cs| {
                        cs.iter()
                            .any(|c| c["type"] == "Ready" && c["status"] == "True")
                    })
                || !labels
                    .iter()
                    .all(|(k, v)| current["metadata"]["labels"][k] == *v)
            {
                return Err(ConnectionError::Conflict);
            }
            remote_port(&current, &target(&service, request.service_port)?)
        })
        .await?;
        epoch.validate()?;
        if cancel.is_cancelled() {
            return Err(ConnectionError::StaleSession);
        }
        // Do not select cancellation against this call: a late listener must be reclaimed.
        let forward = self
            .pod_forward_start(PodForwardRequest {
                cluster_id: scope.cluster_id.clone(),
                session_id: scope.session_id.clone(),
                namespace: pod
                    .namespace
                    .clone()
                    .ok_or(ConnectionError::InvalidResponse)?,
                name: pod.name.clone(),
                uid: pod.uid.clone().ok_or(ConnectionError::InvalidResponse)?,
                remote_port,
                local_port: request.local_port,
            })
            .await?;
        guard.forward = Some(forward.id.clone());
        bounded(&cancel, verify_service(&client, &request, &cancel)).await?;
        epoch.validate()?;
        let result = ServiceForwardResult {
            cluster_id: scope.cluster_id,
            session_id: scope.session_id,
            reference: request.reference.clone(),
            service_port: request.service_port,
            pod,
            forward,
        };
        serde_json::to_vec(&result).map_err(|_| ConnectionError::InvalidResponse)?;
        // Cancel and delivery commit are serialized with the operation slot.
        {
            let mut state = self
                .session
                .lock()
                .map_err(|_| ConnectionError::StoreUnavailable)?;
            let session = super::current_mut(&mut state, &request.session())?;
            if cancel.is_cancelled() || session.service_forward.id != request.operation_id {
                return Err(ConnectionError::StaleSession);
            }
            session.service_forward.cancel = None;
            guard.forward = None;
        }
        Ok(result)
    }
}
fn identity(s: &str) -> bool {
    !s.is_empty() && s.len() <= 256 && s.bytes().all(|b| (0x21..=0x7e).contains(&b))
}
fn verify_object(value: &Value, reference: &ResourceReference, kind: &str) -> Result<()> {
    if value["apiVersion"] != "v1"
        || value["kind"] != kind
        || value["metadata"]["name"].as_str() != Some(&reference.name)
        || value["metadata"]["namespace"].as_str() != reference.namespace.as_deref()
        || value["metadata"]["uid"].as_str() != reference.uid.as_deref()
        || !value["metadata"]["deletionTimestamp"].is_null()
    {
        return Err(ConnectionError::Conflict);
    }
    Ok(())
}
async fn verify_service(
    client: &Client,
    request: &ServiceForwardRequest,
    token: &CancellationToken,
) -> Result<Value> {
    let value = get(client, &named_path(&request.reference)?, token).await?;
    verify_object(&value, &request.reference, "Service")?;
    if value["metadata"]["resourceVersion"].as_str() != Some(&request.resource_version) {
        return Err(ConnectionError::Conflict);
    }
    Ok(value)
}
fn label_part(s: &str, empty: bool) -> bool {
    (empty && s.is_empty())
        || (!s.is_empty()
            && s.len() <= 63
            && s.as_bytes()[0].is_ascii_alphanumeric()
            && s.as_bytes()[s.len() - 1].is_ascii_alphanumeric()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)))
}
fn selector(service: &Value) -> Result<String> {
    let labels = service
        .pointer("/spec/selector")
        .and_then(Value::as_object)
        .filter(|s| !s.is_empty() && s.len() <= 64)
        .ok_or(ConnectionError::ResourceUnavailable)?;
    let mut parts = Vec::new();
    for (key, value) in labels {
        let (prefix, name) = key
            .rsplit_once('/')
            .map_or((None, key.as_str()), |(p, n)| (Some(p), n));
        if !label_part(name, false)
            || prefix.is_some_and(|p| p.len() > 253 || !super::resources::valid_name(p))
            || !value.as_str().is_some_and(|s| label_part(s, true))
        {
            return Err(ConnectionError::InvalidResponse);
        }
        parts.push(format!("{key}={}", value.as_str().unwrap()));
    }
    parts.sort();
    Ok(parts.join(","))
}
fn target(service: &Value, port: u16) -> Result<Value> {
    let ports = service
        .pointer("/spec/ports")
        .and_then(Value::as_array)
        .ok_or(ConnectionError::InvalidResponse)?;
    let matches: Vec<_> = ports
        .iter()
        .filter(|p| {
            p["port"].as_u64() == Some(u64::from(port))
                && p["protocol"].as_str().unwrap_or("TCP") == "TCP"
        })
        .collect();
    if matches.len() != 1 {
        return Err(ConnectionError::InvalidInput);
    }
    let p = matches[0];
    Ok(if p["targetPort"].is_null() {
        p["port"].clone()
    } else {
        p["targetPort"].clone()
    })
}
fn remote_port(pod: &Value, target: &Value) -> Result<u16> {
    if let Some(n) = target.as_u64() {
        return u16::try_from(n)
            .ok()
            .filter(|n| *n != 0)
            .ok_or(ConnectionError::InvalidResponse);
    }
    let name = target
        .as_str()
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 15
                && s.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
        .ok_or(ConnectionError::InvalidResponse)?;
    let containers = pod
        .pointer("/spec/containers")
        .and_then(Value::as_array)
        .ok_or(ConnectionError::InvalidResponse)?;
    let mut numbers = std::collections::BTreeSet::new();
    for c in containers {
        if let Some(ports) = c["ports"].as_array() {
            for p in ports {
                if p["name"].as_str() == Some(name)
                    && p["protocol"].as_str().unwrap_or("TCP") == "TCP"
                {
                    let n = p["containerPort"]
                        .as_u64()
                        .and_then(|n| u16::try_from(n).ok())
                        .filter(|n| *n != 0)
                        .ok_or(ConnectionError::InvalidResponse)?;
                    numbers.insert(n);
                }
            }
        }
    }
    if numbers.len() != 1 {
        return Err(ConnectionError::ResourceUnavailable);
    }
    Ok(*numbers.first().unwrap())
}
async fn resolve(
    client: &Client,
    request: &ServiceForwardRequest,
    token: &CancellationToken,
) -> Result<(ResourceReference, u16)> {
    let service = verify_service(client, request, token).await?;
    let target = target(&service, request.service_port)?;
    let selector = selector(&service)?;
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("labelSelector", &selector)
        .append_pair("limit", &POD_LIMIT.to_string())
        .finish();
    let namespace = request
        .reference
        .namespace
        .as_deref()
        .ok_or(ConnectionError::InvalidInput)?;
    let list = get(
        client,
        &format!("/api/v1/namespaces/{namespace}/pods?{query}"),
        token,
    )
    .await?;
    if list["kind"] != "PodList"
        || list["apiVersion"] != "v1"
        || list["metadata"]["continue"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
    {
        return Err(ConnectionError::LimitExceeded);
    }
    let items = list["items"]
        .as_array()
        .filter(|a| a.len() <= POD_LIMIT)
        .ok_or(ConnectionError::InvalidResponse)?;
    let labels = service["spec"]["selector"].as_object().unwrap();
    let mut pods: Vec<_> = items
        .iter()
        .filter(|p| {
            p["status"]["phase"] == "Running"
                && p["metadata"]["deletionTimestamp"].is_null()
                && p["status"]["conditions"].as_array().is_some_and(|cs| {
                    cs.iter()
                        .any(|c| c["type"] == "Ready" && c["status"] == "True")
                })
                && labels.iter().all(|(k, v)| p["metadata"]["labels"][k] == *v)
        })
        .collect();
    pods.sort_by_key(|p| {
        (
            p["metadata"]["name"].as_str().unwrap_or(""),
            p["metadata"]["uid"].as_str().unwrap_or(""),
        )
    });
    let candidate = pods.first().ok_or(ConnectionError::ResourceUnavailable)?;
    let pod = ResourceReference {
        kind: ResourceKind::Pods,
        namespace: Some(namespace.into()),
        name: candidate["metadata"]["name"]
            .as_str()
            .ok_or(ConnectionError::InvalidResponse)?
            .into(),
        uid: Some(
            candidate["metadata"]["uid"]
                .as_str()
                .filter(|s| identity(s))
                .ok_or(ConnectionError::InvalidResponse)?
                .into(),
        ),
    };
    validate_reference(&pod)?;
    verify_object(candidate, &pod, "Pod")?;
    Ok((pod, remote_port(candidate, &target)?))
}
async fn get(client: &Client, path: &str, token: &CancellationToken) -> Result<Value> {
    let mut request = Request::get(path)
        .header("Accept", "application/json")
        .body(Body::empty())
        .map_err(|_| ConnectionError::InvalidInput)?;
    request.extensions_mut().insert(token.clone());
    let mut response = client
        .send(request)
        .await
        .map_err(super::pods::map_pod_error)?;
    match response.status().as_u16() {
        200 => {}
        401 => return Err(ConnectionError::AuthFailed),
        403 => return Err(ConnectionError::PermissionDenied),
        404 => return Err(ConnectionError::NotFound),
        409 => return Err(ConnectionError::Conflict),
        429 => return Err(ConnectionError::Busy),
        _ => return Err(ConnectionError::ConnectionFailed),
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
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    fn request() -> ServiceForwardRequest {
        ServiceForwardRequest {
            cluster_id: "cluster".into(),
            session_id: "session".into(),
            operation_id: 1,
            reference: ResourceReference {
                kind: ResourceKind::Services,
                namespace: Some("default".into()),
                name: "web".into(),
                uid: Some("service-uid".into()),
            },
            resource_version: "7".into(),
            service_port: 80,
            local_port: 0,
        }
    }
    fn service() -> Value {
        json!({"apiVersion":"v1","kind":"Service","metadata":{"name":"web","namespace":"default","uid":"service-uid","resourceVersion":"7"},"spec":{"selector":{"example.test/app":"web","track":""},"ports":[{"port":80,"protocol":"TCP","targetPort":"http"}]}})
    }
    fn pod(name: &str, uid: &str) -> Value {
        json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":name,"namespace":"default","uid":uid,"labels":{"example.test/app":"web","track":""}},"status":{"phase":"Running","conditions":[{"type":"Ready","status":"True"}]},"spec":{"containers":[{"name":"web","ports":[{"name":"http","containerPort":8080,"protocol":"TCP"}]}]}})
    }
    fn fake(service: Value, list: Value, paths: Arc<Mutex<Vec<String>>>) -> Client {
        Client::new(
            super::super::transport::CancelService(tower::service_fn(move |r: Request<Body>| {
                let response = if r.uri().path().ends_with("/services/web") {
                    service.clone()
                } else {
                    list.clone()
                };
                paths.lock().unwrap().push(r.uri().to_string());
                async move {
                    Ok::<_, std::io::Error>(
                        http::Response::builder()
                            .status(200)
                            .body(Body::from(serde_json::to_vec(&response).unwrap()))
                            .unwrap(),
                    )
                }
            })),
            "default",
        )
    }
    #[tokio::test]
    async fn exact_encoded_selector_deterministic_ready_pod_and_named_target() {
        let paths = Arc::new(Mutex::new(Vec::new()));
        let mut deleting = pod("aaa", "deleting");
        deleting["metadata"]["deletionTimestamp"] = json!("2026-01-01T00:00:00Z");
        let mut unready = pod("aab", "unready");
        unready["status"]["conditions"][0]["status"] = json!("False");
        let list = json!({"apiVersion":"v1","kind":"PodList","metadata":{},"items":[pod("z-last","z"),deleting,unready,pod("b-first","b")]});
        let (selected, port) = resolve(
            &fake(service(), list, paths.clone()),
            &request(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(selected.name, "b-first");
        assert_eq!(selected.uid.as_deref(), Some("b"));
        assert_eq!(port, 8080);
        let paths = paths.lock().unwrap();
        assert_eq!(paths[0], "/api/v1/namespaces/default/services/web");
        assert!(paths[1].contains("labelSelector=example.test%2Fapp%3Dweb%2Ctrack%3D"));
        assert!(paths[1].contains("limit=200"));
    }
    #[tokio::test]
    async fn changed_parent_and_partial_list_fail_closed() {
        let mut changed = service();
        changed["metadata"]["uid"] = json!("replacement");
        let paths = Arc::new(Mutex::new(Vec::new()));
        assert_eq!(
            resolve(
                &fake(changed, Value::Null, paths.clone()),
                &request(),
                &CancellationToken::new()
            )
            .await,
            Err(ConnectionError::Conflict)
        );
        assert_eq!(paths.lock().unwrap().len(), 1);
        let list = json!({"kind":"PodList","apiVersion":"v1","metadata":{"continue":"next"},"items":[pod("web","uid")]});
        assert_eq!(
            resolve(
                &fake(service(), list, paths),
                &request(),
                &CancellationToken::new()
            )
            .await,
            Err(ConnectionError::LimitExceeded)
        );
    }
    #[test]
    fn target_and_selector_validation_and_closed_wire_schema() {
        let mut s = service();
        s["spec"]["selector"] = json!({"app":"web,other=evil"});
        assert!(selector(&s).is_err());
        s["spec"]["selector"] = json!({});
        assert!(selector(&s).is_err());
        s = service();
        s["spec"]["ports"][0]["protocol"] = json!("UDP");
        assert!(target(&s, 80).is_err());
        s = service();
        let duplicate = s["spec"]["ports"][0].clone();
        s["spec"]["ports"].as_array_mut().unwrap().push(duplicate);
        assert!(target(&s, 80).is_err());
        let mut p = pod("web", "uid");
        assert_eq!(remote_port(&p, &json!(80)), Ok(80));
        assert!(remote_port(&p, &json!(0)).is_err());
        p["spec"]["containers"]
            .as_array_mut()
            .unwrap()
            .push(json!({"ports":[{"name":"http","containerPort":9090,"protocol":"TCP"}]}));
        assert!(remote_port(&p, &json!("http")).is_err());
        let wire = json!({"clusterId":"cluster","sessionId":"session","operationId":1,"reference":{"kind":"services","namespace":"default","name":"web","uid":"uid"},"resourceVersion":"7","servicePort":80,"localPort":0,"targetPort":8080});
        assert!(serde_json::from_value::<ServiceForwardRequest>(wire).is_err());
    }
    #[test]
    fn cancel_before_begin_rejects_replay_and_cancelled_inflight_stays_busy() {
        let mut slot = ServiceForwardSlot::default();
        slot.cancel(3);
        assert_eq!(
            slot.begin(3, CancellationToken::new()),
            Err(ConnectionError::StaleSession)
        );
        let token = CancellationToken::new();
        slot.begin(4, token.clone()).unwrap();
        slot.cancel(4);
        assert!(token.is_cancelled());
        assert!(slot.active());
        assert_eq!(
            slot.begin(5, CancellationToken::new()),
            Err(ConnectionError::Busy)
        );
    }
}
