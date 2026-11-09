use super::permissions::{PermissionAction, PermissionCheck};
use super::{
    ConnectionEngine, ConnectionError, ResourceKind, Result, SessionRequest, validate_credentials,
    validate_read_id,
};
use http::{Method, Request};
use http_body_util::BodyExt;
use kube::{Client, client::Body};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

const YAML_LIMIT: usize = 256 * 1024;
const RESPONSE_LIMIT: usize = 1024 * 1024;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub kind: ResourceKind,
    #[serde(default)]
    pub namespace: Option<String>,
    pub yaml: String,
    pub dry_run: bool,
    pub read_id: u64,
}
impl CreateRequest {
    fn session(&self) -> SessionRequest {
        SessionRequest {
            cluster_id: self.cluster_id.clone(),
            session_id: self.session_id.clone(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateCancelRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub kind: ResourceKind,
    pub read_id: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateResult {
    pub cluster_id: String,
    pub session_id: String,
    pub kind: ResourceKind,
    pub namespace: Option<String>,
    pub name: String,
    pub created: bool,
}

/// The single source of truth for create API metadata. Keep this aligned with the inventory paths.
pub fn creation_info(kind: ResourceKind) -> (&'static str, &'static str) {
    use ResourceKind::*;
    match kind {
        Nodes => ("v1", "Node"),
        Namespaces => ("v1", "Namespace"),
        Pods => ("v1", "Pod"),
        Deployments => ("apps/v1", "Deployment"),
        DaemonSets => ("apps/v1", "DaemonSet"),
        StatefulSets => ("apps/v1", "StatefulSet"),
        ReplicaSets => ("apps/v1", "ReplicaSet"),
        ReplicationControllers => ("v1", "ReplicationController"),
        Jobs => ("batch/v1", "Job"),
        CronJobs => ("batch/v1", "CronJob"),
        ConfigMaps => ("v1", "ConfigMap"),
        Secrets => ("v1", "Secret"),
        ResourceQuotas => ("v1", "ResourceQuota"),
        LimitRanges => ("v1", "LimitRange"),
        HorizontalPodAutoscalers => ("autoscaling/v2", "HorizontalPodAutoscaler"),
        VerticalPodAutoscalers => ("autoscaling.k8s.io/v1", "VerticalPodAutoscaler"),
        PodDisruptionBudgets => ("policy/v1", "PodDisruptionBudget"),
        PriorityClasses => ("scheduling.k8s.io/v1", "PriorityClass"),
        RuntimeClasses => ("node.k8s.io/v1", "RuntimeClass"),
        Leases => ("coordination.k8s.io/v1", "Lease"),
        Services => ("v1", "Service"),
        Endpoints => ("v1", "Endpoints"),
        Ingresses => ("networking.k8s.io/v1", "Ingress"),
        IngressClasses => ("networking.k8s.io/v1", "IngressClass"),
        NetworkPolicies => ("networking.k8s.io/v1", "NetworkPolicy"),
        EndpointSlices => ("discovery.k8s.io/v1", "EndpointSlice"),
        MutatingWebhookConfigurations => (
            "admissionregistration.k8s.io/v1",
            "MutatingWebhookConfiguration",
        ),
        ValidatingWebhookConfigurations => (
            "admissionregistration.k8s.io/v1",
            "ValidatingWebhookConfiguration",
        ),
        ValidatingAdmissionPolicies => (
            "admissionregistration.k8s.io/v1",
            "ValidatingAdmissionPolicy",
        ),
        ValidatingAdmissionPolicyBindings => (
            "admissionregistration.k8s.io/v1",
            "ValidatingAdmissionPolicyBinding",
        ),
        PodSecurityPolicies => ("policy/v1beta1", "PodSecurityPolicy"),
        PersistentVolumes => ("v1", "PersistentVolume"),
        PersistentVolumeClaims => ("v1", "PersistentVolumeClaim"),
        StorageClasses => ("storage.k8s.io/v1", "StorageClass"),
        Events => ("v1", "Event"),
        ServiceAccounts => ("v1", "ServiceAccount"),
        Roles => ("rbac.authorization.k8s.io/v1", "Role"),
        RoleBindings => ("rbac.authorization.k8s.io/v1", "RoleBinding"),
        ClusterRoles => ("rbac.authorization.k8s.io/v1", "ClusterRole"),
        ClusterRoleBindings => ("rbac.authorization.k8s.io/v1", "ClusterRoleBinding"),
        CustomResourceDefinitions => ("apiextensions.k8s.io/v1", "CustomResourceDefinition"),
    }
}
pub(super) fn plural(kind: ResourceKind) -> &'static str {
    use ResourceKind::*;
    match kind {
        Nodes => "nodes",
        Namespaces => "namespaces",
        Pods => "pods",
        Deployments => "deployments",
        DaemonSets => "daemonsets",
        StatefulSets => "statefulsets",
        ReplicaSets => "replicasets",
        ReplicationControllers => "replicationcontrollers",
        Jobs => "jobs",
        CronJobs => "cronjobs",
        ConfigMaps => "configmaps",
        Secrets => "secrets",
        ResourceQuotas => "resourcequotas",
        LimitRanges => "limitranges",
        HorizontalPodAutoscalers => "horizontalpodautoscalers",
        VerticalPodAutoscalers => "verticalpodautoscalers",
        PodDisruptionBudgets => "poddisruptionbudgets",
        PriorityClasses => "priorityclasses",
        RuntimeClasses => "runtimeclasses",
        Leases => "leases",
        Services => "services",
        Endpoints => "endpoints",
        Ingresses => "ingresses",
        IngressClasses => "ingressclasses",
        NetworkPolicies => "networkpolicies",
        EndpointSlices => "endpointslices",
        MutatingWebhookConfigurations => "mutatingwebhookconfigurations",
        ValidatingWebhookConfigurations => "validatingwebhookconfigurations",
        ValidatingAdmissionPolicies => "validatingadmissionpolicies",
        ValidatingAdmissionPolicyBindings => "validatingadmissionpolicybindings",
        PodSecurityPolicies => "podsecuritypolicies",
        PersistentVolumes => "persistentvolumes",
        PersistentVolumeClaims => "persistentvolumeclaims",
        StorageClasses => "storageclasses",
        Events => "events",
        ServiceAccounts => "serviceaccounts",
        Roles => "roles",
        RoleBindings => "rolebindings",
        ClusterRoles => "clusterroles",
        ClusterRoleBindings => "clusterrolebindings",
        CustomResourceDefinitions => "customresourcedefinitions",
    }
}
fn namespaced(kind: ResourceKind) -> bool {
    super::resources::namespaced(kind)
}
fn dns(s: &str, max: usize) -> bool {
    !s.is_empty()
        && s.len() <= max
        && s.split('.').all(|p| {
            !p.is_empty()
                && p.len() <= 63
                && p.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                && p.as_bytes()[0].is_ascii_alphanumeric()
                && p.as_bytes()[p.len() - 1].is_ascii_alphanumeric()
        })
}

pub fn validate_yaml(request: &CreateRequest) -> Result<(Value, String, Option<String>)> {
    if request.yaml.len() > YAML_LIMIT || request.yaml.contains('\0') {
        return Err(ConnectionError::InvalidInput);
    }
    let mut documents = serde_yaml_ng::Deserializer::from_str(&request.yaml);
    let document = documents.next().ok_or(ConnectionError::InvalidInput)?;
    let yaml =
        serde_yaml_ng::Value::deserialize(document).map_err(|_| ConnectionError::InvalidInput)?;
    if documents.next().is_some() {
        return Err(ConnectionError::InvalidInput);
    }
    let mut value = serde_json::to_value(yaml).map_err(|_| ConnectionError::InvalidInput)?;
    let object = value.as_object().ok_or(ConnectionError::InvalidInput)?;
    let (api, kube_kind) = creation_info(request.kind);
    if object.get("apiVersion").and_then(Value::as_str) != Some(api)
        || object.get("kind").and_then(Value::as_str) != Some(kube_kind)
    {
        return Err(ConnectionError::InvalidInput);
    }
    if object.contains_key("status") {
        return Err(ConnectionError::InvalidInput);
    }
    let metadata = object
        .get("metadata")
        .and_then(Value::as_object)
        .ok_or(ConnectionError::InvalidInput)?;
    let name = metadata
        .get("name")
        .and_then(Value::as_str)
        .ok_or(ConnectionError::InvalidInput)?
        .to_owned();
    if !dns(&name, 253) {
        return Err(ConnectionError::InvalidInput);
    }
    for key in [
        "generateName",
        "uid",
        "resourceVersion",
        "generation",
        "creationTimestamp",
        "deletionTimestamp",
        "deletionGracePeriodSeconds",
        "managedFields",
        "selfLink",
    ] {
        if metadata.contains_key(key) {
            return Err(ConnectionError::InvalidInput);
        }
    }
    let namespace = if namespaced(request.kind) {
        let selected = request
            .namespace
            .clone()
            .or_else(|| {
                metadata
                    .get("namespace")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "default".into());
        if !super::transport::valid_namespace(&selected) {
            return Err(ConnectionError::InvalidInput);
        }
        Some(selected)
    } else {
        if request.namespace.is_some() || metadata.contains_key("namespace") {
            return Err(ConnectionError::InvalidInput);
        }
        None
    };
    if let Some(ns) = &namespace
        && metadata
            .get("namespace")
            .is_some_and(|v| v.as_str() != Some(ns))
    {
        return Err(ConnectionError::InvalidInput);
    }
    if let Some(ns) = &namespace {
        value["metadata"]["namespace"] = Value::String(ns.clone());
    }
    Ok((value, name, namespace))
}

impl ConnectionEngine {
    pub fn cancel_create(&self, request: CreateCancelRequest) -> Result<()> {
        validate_read_id(request.read_id)?;
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
            let slot = session
                .creates
                .entry(request.kind)
                .or_insert(super::CreateSlot {
                    id: 0,
                    cancel: None,
                });
            if request.read_id == slot.id {
                if let Some(cancel) = slot.cancel.take() {
                    cancel.cancel();
                }
            } else if request.read_id > slot.id && slot.cancel.is_none() {
                slot.id = request.read_id;
            }
        }
        Ok(())
    }
    pub async fn create(&self, request: CreateRequest) -> Result<CreateResult> {
        let (value, name, namespace) = validate_yaml(&request)?;
        validate_read_id(request.read_id)?;
        let identity = request.session();
        super::validate_request(&identity)?;
        let authorization = self
            .require_permissions(
                &identity,
                &[PermissionCheck {
                    kind: request.kind,
                    action: PermissionAction::Create,
                    namespace: namespace.clone(),
                    name: None,
                }],
            )
            .await?;
        let _permit = self
            .workers
            .try_acquire()
            .map_err(|_| ConnectionError::Busy)?;
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
            validate_credentials(&client)?;
            let cancel = session.cancel.child_token();
            let slot = session
                .creates
                .entry(request.kind)
                .or_insert(super::CreateSlot {
                    id: 0,
                    cancel: None,
                });
            if request.read_id <= slot.id {
                return Err(ConnectionError::StaleSession);
            }
            if slot.cancel.is_some() {
                return Err(ConnectionError::Busy);
            }
            slot.id = request.read_id;
            slot.cancel = Some(cancel.clone());
            (client, cancel)
        };
        let client = super::permissions::authorized_client(client, authorization);
        let result = super::bounded(
            &cancel,
            create_transport(&client, &request, namespace.as_deref(), value, &cancel),
        )
        .await;
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        let session = super::current_mut(&mut state, &identity)?;
        if session
            .creates
            .get(&request.kind)
            .is_none_or(|slot| slot.id != request.read_id)
        {
            return Err(ConnectionError::StaleSession);
        }
        session.creates.get_mut(&request.kind).unwrap().cancel = None;
        if cancel.is_cancelled() && result != Err(ConnectionError::Timeout) {
            return Err(ConnectionError::StaleSession);
        }
        result.map(|_| CreateResult {
            cluster_id: request.cluster_id,
            session_id: request.session_id,
            kind: request.kind,
            namespace,
            name,
            created: !request.dry_run,
        })
    }
}

async fn create_transport(
    client: &Client,
    request: &CreateRequest,
    namespace: Option<&str>,
    value: Value,
    cancel: &CancellationToken,
) -> Result<()> {
    let (api, _) = creation_info(request.kind);
    let prefix = if api == "v1" {
        "/api/v1".to_owned()
    } else {
        format!("/apis/{api}")
    };
    let scope = namespace
        .map(|ns| format!("/namespaces/{ns}"))
        .unwrap_or_default();
    let dry = if request.dry_run { "?dryRun=All" } else { "" };
    let path = format!("{prefix}{scope}/{}{}", plural(request.kind), dry);
    let body = serde_json::to_vec(&value).map_err(|_| ConnectionError::InvalidInput)?;
    if body.len() > RESPONSE_LIMIT {
        return Err(ConnectionError::LimitExceeded);
    }
    let mut wire = Request::builder()
        .method(Method::POST)
        .uri(path)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .map_err(|_| ConnectionError::InvalidInput)?;
    wire.extensions_mut().insert(cancel.clone());
    let mut response = client
        .send(wire)
        .await
        .map_err(|_| ConnectionError::Network)?;
    match response.status().as_u16() {
        200..=299 => {}
        401 => return Err(ConnectionError::AuthFailed),
        403 => return Err(ConnectionError::PermissionDenied),
        404 => return Err(ConnectionError::ResourceUnavailable),
        409 => return Err(ConnectionError::Conflict),
        400 | 422 => return Err(ConnectionError::InvalidInput),
        408 | 504 => return Err(ConnectionError::Timeout),
        _ => return Err(ConnectionError::ConnectionFailed),
    }
    if response
        .headers()
        .get("content-length")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<usize>().ok())
        .is_some_and(|n| n > RESPONSE_LIMIT)
    {
        return Err(ConnectionError::LimitExceeded);
    }
    let mut bytes = Vec::new();
    while let Some(frame) = response.body_mut().frame().await {
        let frame = frame.map_err(|_| ConnectionError::Network)?;
        if let Ok(data) = frame.into_data() {
            if data.len() > RESPONSE_LIMIT.saturating_sub(bytes.len()) {
                return Err(ConnectionError::LimitExceeded);
            }
            bytes.extend_from_slice(&data);
        }
    }
    let response: Value =
        serde_json::from_slice(&bytes).map_err(|_| ConnectionError::InvalidResponse)?;
    for pointer in [
        "/apiVersion",
        "/kind",
        "/metadata/name",
        "/metadata/namespace",
    ] {
        if response.pointer(pointer) != value.pointer(pointer) {
            return Err(ConnectionError::InvalidResponse);
        }
    }
    Ok(())
}
