use super::{
    ConnectionEngine, ConnectionError, ResourceItem, ResourceKind, Result, SessionRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceReference {
    pub kind: ResourceKind,
    pub namespace: Option<String>,
    pub name: String,
    #[serde(default)]
    pub uid: Option<String>,
}
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DetailRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub read_id: u64,
    pub reference: ResourceReference,
}
impl DetailRequest {
    fn session(&self) -> SessionRequest {
        SessionRequest {
            cluster_id: self.cluster_id.clone(),
            session_id: self.session_id.clone(),
        }
    }
}
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResourceDetail {
    pub cluster_id: String,
    pub session_id: String,
    pub read_id: u64,
    pub reference: ResourceReference,
    pub item: ResourceItem,
    pub related: BTreeMap<String, ResourceReference>,
    pub sections: Vec<super::detail_facts::DetailSection>,
    pub metadata: Option<super::detail_metadata::DetailMetadata>,
    pub event_diagnostic: Option<super::event_diagnostic::EventDiagnostic>,
}
#[derive(Default)]
pub(super) struct DetailSlot {
    id: u64,
    request: Option<DetailRequest>,
    cancel: Option<CancellationToken>,
}
fn valid_uid(s: &str) -> bool {
    !s.is_empty() && s.len() <= 256 && s.bytes().all(|b| (0x21..=0x7e).contains(&b))
}
pub(super) fn valid_resource_name(kind: ResourceKind, name: &str) -> bool {
    if matches!(
        kind,
        ResourceKind::Roles
            | ResourceKind::RoleBindings
            | ResourceKind::ClusterRoles
            | ResourceKind::ClusterRoleBindings
    ) {
        // RBAC uses Kubernetes path-segment names, not DNS subdomains.
        !name.is_empty()
            && name.len() <= 253
            && !matches!(name, "." | "..")
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
    } else {
        super::resources::valid_name(name)
    }
}
pub(super) fn validate_reference(r: &ResourceReference) -> Result<()> {
    if !valid_resource_name(r.kind, &r.name)
        || r.uid.as_deref().is_some_and(|s| !valid_uid(s))
        || if super::resources::namespaced(r.kind) {
            !r.namespace
                .as_deref()
                .is_some_and(super::transport::valid_namespace)
        } else {
            r.namespace.is_some()
        }
    {
        return Err(ConnectionError::InvalidInput);
    }
    Ok(())
}
fn validate(r: &DetailRequest) -> Result<()> {
    super::validate_request(&r.session())?;
    super::validate_read_id(r.read_id)?;
    validate_reference(&r.reference)
}
pub(super) fn named_path(r: &ResourceReference) -> Result<String> {
    validate_reference(r)?;
    let (api, plural) = super::transport::endpoint(r.kind);
    let scope = r
        .namespace
        .as_ref()
        .map(|n| format!("/namespaces/{n}"))
        .unwrap_or_default();
    let name: String = url::form_urlencoded::byte_serialize(r.name.as_bytes()).collect();
    Ok(format!("{api}{scope}/{plural}/{name}"))
}
impl ConnectionEngine {
    pub fn cancel_resource_detail(&self, r: DetailRequest) -> Result<()> {
        validate(&r)?;
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        if let Some(session) = state
            .as_mut()
            .filter(|s| super::matches_session(s, &r.session()))
        {
            let slot = &mut session.detail;
            if slot.id == r.read_id && slot.request.as_ref().is_some_and(|old| old != &r) {
                return Err(ConnectionError::InvalidInput);
            }
            if r.read_id >= slot.id {
                if let Some(cancel) = slot.cancel.take() {
                    cancel.cancel();
                }
                slot.id = r.read_id;
                slot.request = Some(r);
            }
        }
        Ok(())
    }
    pub async fn resource_detail(&self, r: DetailRequest) -> Result<ResourceDetail> {
        self.resource_detail_with(r, &super::transport::LiveResourceTransport)
            .await
    }
    pub(super) async fn resource_detail_with<T: super::transport::ResourceTransport>(
        &self,
        r: DetailRequest,
        transport: &T,
    ) -> Result<ResourceDetail> {
        validate(&r)?;
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
            let session = super::current_mut(&mut state, &identity)?;
            let client = session
                .client
                .clone()
                .ok_or(ConnectionError::NotConnected)?;
            super::validate_credentials(&client)?;
            let slot = &mut session.detail;
            if r.read_id <= slot.id {
                return Err(ConnectionError::StaleSession);
            }
            let cancel = session.cancel.child_token();
            if let Some(old) = slot.cancel.replace(cancel.clone()) {
                old.cancel();
            }
            slot.id = r.read_id;
            slot.request = Some(r.clone());
            (client, cancel)
        };
        let _guard = CancelOnDrop(cancel.clone());
        let result = super::bounded(&cancel, async {
            let value = transport.detail(&client, &r.reference, &cancel).await?;
            project(&r, value)
        })
        .await;
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        let session = super::current_mut(&mut state, &identity)?;
        if session.detail.id != r.read_id
            || (cancel.is_cancelled() && result != Err(ConnectionError::Timeout))
        {
            return Err(ConnectionError::StaleSession);
        }
        session.detail.cancel = None;
        result
    }
}
struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

pub(super) fn object_type(kind: ResourceKind) -> (&'static str, &'static str) {
    // Creation and reads share the same served group/version and object kind.
    super::create::creation_info(kind)
}
pub(super) fn project(r: &DetailRequest, value: Value) -> Result<ResourceDetail> {
    let reference = &r.reference;
    let sensitive = matches!(
        reference.kind,
        ResourceKind::Secrets | ResourceKind::ConfigMaps
    );
    let (api, kind) = if sensitive {
        ("meta.k8s.io/v1", "PartialObjectMetadata")
    } else {
        object_type(reference.kind)
    };
    if value.get("apiVersion").and_then(Value::as_str) != Some(api)
        || value.get("kind").and_then(Value::as_str) != Some(kind)
        || value.pointer("/metadata/name").and_then(Value::as_str) != Some(&reference.name)
        || value.pointer("/metadata/namespace").and_then(Value::as_str)
            != reference.namespace.as_deref()
    {
        return Err(ConnectionError::InvalidResponse);
    }
    if value
        .pointer("/metadata/namespace")
        .is_some_and(|ns| !ns.is_null() && !ns.is_string())
    {
        return Err(ConnectionError::InvalidResponse);
    }
    let uid = value
        .pointer("/metadata/uid")
        .and_then(Value::as_str)
        .filter(|s| valid_uid(s))
        .ok_or(ConnectionError::InvalidResponse)?;
    if reference
        .uid
        .as_deref()
        .is_some_and(|expected| expected != uid)
    {
        return Err(ConnectionError::Conflict);
    }
    let sections = super::detail_facts::project(reference.kind, &value);
    let metadata = super::detail_metadata::project(reference.kind, &value);
    let event_diagnostic = super::event_diagnostic::project(reference.kind, &value);
    let related = related(reference.kind, &value, reference.namespace.as_deref());
    let mut item = super::resources::project(reference.kind, value)?;
    for column in ["controlledBy", "serviceAccount", "involvedObject"] {
        if let Some(target) = related.get(column) {
            let display = if column == "involvedObject" {
                format!("{}/{}", object_type(target.kind).1, target.name)
            } else {
                target.name.clone()
            };
            item.columns.insert(column.into(), display);
        }
    }
    let mut resolved = reference.clone();
    resolved.uid = item.uid.clone();
    Ok(ResourceDetail {
        cluster_id: r.cluster_id.clone(),
        session_id: r.session_id.clone(),
        read_id: r.read_id,
        reference: resolved,
        item,
        related,
        sections,
        metadata,
        event_diagnostic,
    })
}
fn related(
    kind: ResourceKind,
    v: &Value,
    namespace: Option<&str>,
) -> BTreeMap<String, ResourceReference> {
    use ResourceKind::*;
    let mut links = BTreeMap::new();
    let mut add = |column: &str,
                   target: ResourceKind,
                   ns: Option<&str>,
                   name: Option<&str>,
                   uid: Option<&str>| {
        if let Some(name) = name {
            let r = ResourceReference {
                kind: target,
                namespace: ns.map(str::to_owned),
                name: name.into(),
                uid: uid.map(str::to_owned),
            };
            if validate_reference(&r).is_ok() {
                links.insert(column.into(), r);
            }
        }
    };
    let text = |path: &str| v.pointer(path).and_then(Value::as_str);
    add("namespace", Namespaces, None, namespace, None);
    match kind {
        Pods => {
            add("node", Nodes, None, text("/spec/nodeName"), None);
            add(
                "serviceAccount",
                ServiceAccounts,
                namespace,
                text("/spec/serviceAccountName"),
                None,
            );
        }
        Ingresses => add(
            "class",
            IngressClasses,
            None,
            text("/spec/ingressClassName"),
            None,
        ),
        Events => {
            if let Some(target) = event_object(v, namespace) {
                add(
                    "involvedObject",
                    target.kind,
                    target.namespace.as_deref(),
                    Some(&target.name),
                    target.uid.as_deref(),
                );
            }
        }
        PersistentVolumeClaims => {
            add(
                "storageClass",
                StorageClasses,
                None,
                text("/spec/storageClassName"),
                None,
            );
            add(
                "volume",
                PersistentVolumes,
                None,
                text("/spec/volumeName"),
                None,
            );
        }
        PersistentVolumes => {
            add(
                "storageClass",
                StorageClasses,
                None,
                text("/spec/storageClassName"),
                None,
            );
            add(
                "claim",
                PersistentVolumeClaims,
                text("/spec/claimRef/namespace"),
                text("/spec/claimRef/name"),
                text("/spec/claimRef/uid"),
            );
        }
        RoleBindings | ClusterRoleBindings
            if text("/roleRef/apiGroup") == Some("rbac.authorization.k8s.io") =>
        {
            match text("/roleRef/kind") {
                Some("Role") if kind == RoleBindings => {
                    add("roleRef", Roles, namespace, text("/roleRef/name"), None)
                }
                Some("ClusterRole") => {
                    add("roleRef", ClusterRoles, None, text("/roleRef/name"), None)
                }
                _ => {}
            }
        }
        ValidatingAdmissionPolicyBindings => add(
            "policy",
            ValidatingAdmissionPolicies,
            None,
            text("/spec/policyName"),
            None,
        ),
        HorizontalPodAutoscalers | VerticalPodAutoscalers => {
            let path = if kind == HorizontalPodAutoscalers {
                "/spec/scaleTargetRef"
            } else {
                "/spec/targetRef"
            };
            if let Some(target) = v.pointer(path) {
                for candidate in [
                    Deployments,
                    DaemonSets,
                    StatefulSets,
                    ReplicaSets,
                    ReplicationControllers,
                ] {
                    let (api, name) = object_type(candidate);
                    if target.get("apiVersion").and_then(Value::as_str) == Some(api)
                        && target.get("kind").and_then(Value::as_str) == Some(name)
                    {
                        add(
                            "target",
                            candidate,
                            namespace,
                            target.get("name").and_then(Value::as_str),
                            None,
                        );
                    }
                }
            }
        }
        _ => {}
    }
    if matches!(
        kind,
        Pods | Deployments
            | DaemonSets
            | StatefulSets
            | ReplicaSets
            | ReplicationControllers
            | Jobs
            | CronJobs
    ) && let Some(owner) = controlled_owner(v, namespace)
    {
        add(
            "controlledBy",
            owner.kind,
            owner.namespace.as_deref(),
            Some(&owner.name),
            owner.uid.as_deref(),
        );
    }
    links
}

const EVENT_TARGETS: &[ResourceKind] = &[
    ResourceKind::Nodes,
    ResourceKind::Namespaces,
    ResourceKind::Pods,
    ResourceKind::Deployments,
    ResourceKind::DaemonSets,
    ResourceKind::StatefulSets,
    ResourceKind::ReplicaSets,
    ResourceKind::ReplicationControllers,
    ResourceKind::Jobs,
    ResourceKind::CronJobs,
    ResourceKind::ConfigMaps,
    ResourceKind::Secrets,
    ResourceKind::ResourceQuotas,
    ResourceKind::LimitRanges,
    ResourceKind::HorizontalPodAutoscalers,
    ResourceKind::VerticalPodAutoscalers,
    ResourceKind::PodDisruptionBudgets,
    ResourceKind::PriorityClasses,
    ResourceKind::RuntimeClasses,
    ResourceKind::Leases,
    ResourceKind::Services,
    ResourceKind::Endpoints,
    ResourceKind::Ingresses,
    ResourceKind::IngressClasses,
    ResourceKind::NetworkPolicies,
    ResourceKind::EndpointSlices,
    ResourceKind::MutatingWebhookConfigurations,
    ResourceKind::ValidatingWebhookConfigurations,
    ResourceKind::ValidatingAdmissionPolicies,
    ResourceKind::ValidatingAdmissionPolicyBindings,
    ResourceKind::PodSecurityPolicies,
    ResourceKind::PersistentVolumes,
    ResourceKind::PersistentVolumeClaims,
    ResourceKind::StorageClasses,
    ResourceKind::Events,
    ResourceKind::ServiceAccounts,
    ResourceKind::Roles,
    ResourceKind::RoleBindings,
    ResourceKind::ClusterRoles,
    ResourceKind::ClusterRoleBindings,
    ResourceKind::CustomResourceDefinitions,
];

fn event_object(v: &Value, namespace: Option<&str>) -> Option<ResourceReference> {
    let parse = |object: &Value| {
        let kind = *EVENT_TARGETS.iter().find(|kind| {
            let (api, singular) = object_type(**kind);
            object.get("apiVersion").and_then(Value::as_str) == Some(api)
                && object.get("kind").and_then(Value::as_str) == Some(singular)
        })?;
        let target_namespace = match object.get("namespace") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) if s.is_empty() => None,
            Some(Value::String(s)) => Some(s.as_str()),
            _ => return None,
        };
        if super::resources::namespaced(kind) && target_namespace != namespace {
            return None;
        }
        let reference = ResourceReference {
            kind,
            namespace: target_namespace.map(str::to_owned),
            name: object.get("name")?.as_str()?.into(),
            uid: Some(object.get("uid")?.as_str()?.into()),
        };
        if format!("{}/{}", object_type(kind).1, reference.name).len() > 256 {
            return None;
        }
        validate_reference(&reference).ok().map(|_| reference)
    };
    match (v.get("involvedObject"), v.get("regarding")) {
        (Some(object), None) | (None, Some(object)) => parse(object),
        (Some(a), Some(b)) => {
            let target = parse(a)?;
            (target == parse(b)?).then_some(target)
        }
        _ => None,
    }
}
pub(super) fn controlled_owner(v: &Value, namespace: Option<&str>) -> Option<ResourceReference> {
    use ResourceKind::*;
    let owners = v.pointer("/metadata/ownerReferences")?.as_array()?;
    if owners.len() > 32 {
        return None;
    }
    let mut controllers = owners
        .iter()
        .filter(|o| o.get("controller").and_then(Value::as_bool) == Some(true));
    let owner = controllers.next()?;
    if controllers.next().is_some() {
        return None;
    }
    for kind in [
        Deployments,
        CronJobs,
        ReplicaSets,
        ReplicationControllers,
        DaemonSets,
        StatefulSets,
        Jobs,
    ] {
        let (api, singular) = object_type(kind);
        if owner.get("apiVersion").and_then(Value::as_str) == Some(api)
            && owner.get("kind").and_then(Value::as_str) == Some(singular)
        {
            if owner
                .get("namespace")
                .is_some_and(|ns| ns.as_str() != namespace)
            {
                return None;
            }
            let r = ResourceReference {
                kind,
                namespace: namespace.map(str::to_owned),
                name: owner.get("name")?.as_str()?.into(),
                uid: Some(owner.get("uid")?.as_str()?.into()),
            };
            return validate_reference(&r).ok().map(|_| r);
        }
    }
    None
}
#[cfg(test)]
mod tests;
