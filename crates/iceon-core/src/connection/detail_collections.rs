use super::details::{DetailRequest, ResourceReference};
use super::transport::ResourceTransport;
use super::{ConnectionEngine, ConnectionError, ResourceKind, Result, SessionRequest};
use kube::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use tokio_util::sync::CancellationToken;

const ITEM_LIMIT: usize = 100;
const BODY_LIMIT: usize = 2 * 1024 * 1024;
const RESPONSE_LIMIT: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum CollectionKind {
    Pods,
    ReplicaSets,
    Jobs,
    EndpointSlices,
    Events,
    ResourceQuotas,
    LimitRanges,
}
impl CollectionKind {
    fn resource(self) -> ResourceKind {
        match self {
            Self::Pods => ResourceKind::Pods,
            Self::ReplicaSets => ResourceKind::ReplicaSets,
            Self::Jobs => ResourceKind::Jobs,
            Self::EndpointSlices => ResourceKind::EndpointSlices,
            Self::Events => ResourceKind::Events,
            Self::ResourceQuotas => ResourceKind::ResourceQuotas,
            Self::LimitRanges => ResourceKind::LimitRanges,
        }
    }
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CollectionRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub read_id: u64,
    pub reference: ResourceReference,
    pub collection: CollectionKind,
}
impl CollectionRequest {
    fn session(&self) -> SessionRequest {
        SessionRequest {
            cluster_id: self.cluster_id.clone(),
            session_id: self.session_id.clone(),
        }
    }
    fn detail(&self) -> DetailRequest {
        DetailRequest {
            cluster_id: self.cluster_id.clone(),
            session_id: self.session_id.clone(),
            read_id: self.read_id,
            reference: self.reference.clone(),
        }
    }
}
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CollectionState {
    Complete,
    Partial,
    Denied,
    Unavailable,
}
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CollectionItem {
    pub reference: ResourceReference,
    pub status: String,
    pub columns: BTreeMap<String, String>,
}
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResourceCollection {
    pub cluster_id: String,
    pub session_id: String,
    pub read_id: u64,
    pub reference: ResourceReference,
    pub collection: CollectionKind,
    pub state: CollectionState,
    pub items: Vec<CollectionItem>,
}
impl ResourceCollection {
    fn empty(r: &CollectionRequest, state: CollectionState) -> Self {
        Self {
            cluster_id: r.cluster_id.clone(),
            session_id: r.session_id.clone(),
            read_id: r.read_id,
            reference: r.reference.clone(),
            collection: r.collection,
            state,
            items: Vec::new(),
        }
    }
    fn bounded(mut self) -> Result<Self> {
        while serde_json::to_vec(&self)
            .map_err(|_| ConnectionError::InvalidResponse)?
            .len()
            > RESPONSE_LIMIT
        {
            if self.items.pop().is_none() {
                return Err(ConnectionError::LimitExceeded);
            }
            self.state = CollectionState::Partial;
        }
        Ok(self)
    }
}
#[derive(Default)]
pub(super) struct CollectionSlot {
    id: u64,
    request: Option<CollectionRequest>,
    cancel: Option<CancellationToken>,
}
impl CollectionSlot {
    fn start(
        &mut self,
        r: &CollectionRequest,
        root: &CancellationToken,
    ) -> Result<CancellationToken> {
        if r.read_id <= self.id {
            return Err(ConnectionError::StaleSession);
        }
        let cancel = root.child_token();
        if let Some(old) = self.cancel.replace(cancel.clone()) {
            old.cancel();
        }
        self.id = r.read_id;
        self.request = Some(r.clone());
        Ok(cancel)
    }
    fn cancel(&mut self, r: CollectionRequest) -> Result<()> {
        if self.id == r.read_id && self.request.as_ref().is_some_and(|old| old != &r) {
            return Err(ConnectionError::InvalidInput);
        }
        if r.read_id >= self.id {
            if let Some(cancel) = self.cancel.take() {
                cancel.cancel();
            }
            self.id = r.read_id;
            self.request = Some(r);
        }
        Ok(())
    }
}
fn validate(r: &CollectionRequest) -> Result<()> {
    super::validate_request(&r.session())?;
    super::validate_read_id(r.read_id)?;
    super::details::validate_reference(&r.reference)?;
    if r.reference.uid.is_none() {
        return Err(ConnectionError::InvalidInput);
    }
    if r.collection == CollectionKind::Events
        && r.reference.uid.as_deref().is_some_and(|uid| {
            uid.bytes()
                .any(|b| matches!(b, b',' | b'=' | b'!' | b'(' | b')' | b'\\'))
        })
    {
        return Err(ConnectionError::InvalidInput);
    }
    if matches!(
        r.collection,
        CollectionKind::ResourceQuotas | CollectionKind::LimitRanges
    ) && !super::transport::valid_namespace(&r.reference.name)
    {
        return Err(ConnectionError::InvalidInput);
    }
    use CollectionKind as C;
    use ResourceKind as R;
    let supported = match r.collection {
        C::Pods => matches!(
            r.reference.kind,
            R::Nodes
                | R::Deployments
                | R::ReplicaSets
                | R::DaemonSets
                | R::StatefulSets
                | R::ReplicationControllers
                | R::Jobs
        ),
        C::ReplicaSets => r.reference.kind == R::Deployments,
        C::Jobs => r.reference.kind == R::CronJobs,
        C::EndpointSlices => r.reference.kind == R::Services,
        C::Events => true,
        C::ResourceQuotas | C::LimitRanges => r.reference.kind == R::Namespaces,
    };
    if !supported {
        return Err(ConnectionError::InvalidInput);
    }
    Ok(())
}
impl ConnectionEngine {
    pub fn cancel_resource_collection(&self, r: CollectionRequest) -> Result<()> {
        validate(&r)?;
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        if let Some(session) = state
            .as_mut()
            .filter(|s| super::matches_session(s, &r.session()))
        {
            session
                .collections
                .entry(r.collection)
                .or_default()
                .cancel(r)?;
        }
        Ok(())
    }
    pub async fn resource_collection(&self, r: CollectionRequest) -> Result<ResourceCollection> {
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
            let cancel = session
                .collections
                .entry(r.collection)
                .or_default()
                .start(&r, &session.cancel)?;
            (client, cancel)
        };
        let _guard = CancelOnDrop(cancel.clone());
        let result = super::bounded(&cancel, collect(&client, &r, &cancel)).await;
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        let session = super::current_mut(&mut state, &identity)?;
        let slot = session
            .collections
            .get_mut(&r.collection)
            .ok_or(ConnectionError::StaleSession)?;
        if slot.id != r.read_id
            || slot.request.as_ref() != Some(&r)
            || (cancel.is_cancelled() && result != Err(ConnectionError::Timeout))
        {
            return Err(ConnectionError::StaleSession);
        }
        slot.cancel = None;
        result
    }
}
struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
async fn verify_parent(
    client: &Client,
    r: &CollectionRequest,
    cancel: &CancellationToken,
) -> Result<()> {
    let value = super::transport::LiveResourceTransport
        .detail(client, &r.reference, cancel)
        .await?;
    super::details::project(&r.detail(), value)?;
    Ok(())
}
async fn collect(
    client: &Client,
    r: &CollectionRequest,
    cancel: &CancellationToken,
) -> Result<ResourceCollection> {
    let result = async {
        verify_parent(client, r, cancel).await?;
        let mut response = ResourceCollection::empty(r, CollectionState::Complete);
        let mut replicas = Vec::new();
        if r.collection == CollectionKind::Pods && r.reference.kind == ResourceKind::Deployments {
            let page = list(client, r, ResourceKind::ReplicaSets, cancel).await?;
            if page.partial {
                response.state = CollectionState::Partial;
            }
            for value in page.items {
                if controlled_by(&value, &r.reference) {
                    replicas.push(project_item(ResourceKind::ReplicaSets, value)?.reference);
                }
            }
        }
        let kind = r.collection.resource();
        let page = list(client, r, kind, cancel).await?;
        if page.partial {
            response.state = CollectionState::Partial;
        }
        for value in page.items {
            if associated(r, &value, &replicas) {
                response.items.push(project_item(kind, value)?);
            }
        }
        // resourceVersion may change during this read, but parent UID must not.
        verify_parent(client, r, cancel).await?;
        response.items.sort_by(|a, b| {
            (&a.reference.namespace, &a.reference.name)
                .cmp(&(&b.reference.namespace, &b.reference.name))
        });
        response.bounded()
    }
    .await;
    match result {
        Err(ConnectionError::PermissionDenied) => {
            Ok(ResourceCollection::empty(r, CollectionState::Denied))
        }
        Err(ConnectionError::ResourceUnavailable) => {
            Ok(ResourceCollection::empty(r, CollectionState::Unavailable))
        }
        other => other,
    }
}
#[derive(Deserialize)]
struct List {
    #[serde(rename = "apiVersion")]
    api_version: Option<String>,
    kind: Option<String>,
    #[serde(default)]
    metadata: ListMetadata,
    items: Vec<Value>,
}
#[derive(Default, Deserialize)]
struct ListMetadata {
    #[serde(rename = "continue")]
    continuation: Option<String>,
}
struct Page {
    items: Vec<Value>,
    partial: bool,
}
fn scope(r: &CollectionRequest) -> Option<&str> {
    if matches!(
        r.collection,
        CollectionKind::ResourceQuotas | CollectionKind::LimitRanges
    ) {
        Some(&r.reference.name)
    } else {
        r.reference.namespace.as_deref()
    }
}
fn list_path(r: &CollectionRequest, kind: ResourceKind) -> String {
    let (api, plural) = super::transport::endpoint(kind);
    let scope = scope(r)
        .map(|ns| format!("/namespaces/{ns}"))
        .unwrap_or_default();
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.append_pair("limit", "100");
    if kind == ResourceKind::Pods && r.reference.kind == ResourceKind::Nodes {
        query.append_pair(
            "fieldSelector",
            &format!("spec.nodeName={}", r.reference.name),
        );
    }
    if kind == ResourceKind::Events {
        query.append_pair(
            "fieldSelector",
            &format!(
                "involvedObject.uid={}",
                r.reference.uid.as_deref().unwrap_or_default()
            ),
        );
    }
    if kind == ResourceKind::EndpointSlices {
        query.append_pair(
            "labelSelector",
            &format!("kubernetes.io/service-name={}", r.reference.name),
        );
    }
    format!("{api}{scope}/{plural}?{}", query.finish())
}
async fn list(
    client: &Client,
    r: &CollectionRequest,
    kind: ResourceKind,
    cancel: &CancellationToken,
) -> Result<Page> {
    let path = list_path(r, kind);
    let list: List = super::transport::get(client, &path, BODY_LIMIT, cancel).await?;
    let (api, singular) = super::details::object_type(kind);
    if list.api_version.as_deref() != Some(api)
        || list.kind.as_deref() != Some(&format!("{singular}List"))
    {
        return Err(ConnectionError::InvalidResponse);
    }
    if list.items.len() > ITEM_LIMIT {
        return Err(ConnectionError::LimitExceeded);
    }
    let mut seen = HashSet::new();
    for value in &list.items {
        // TypeMeta can be absent inside a list, never contradictory.
        if value
            .get("apiVersion")
            .is_some_and(|v| v.as_str() != Some(api))
            || value
                .get("kind")
                .is_some_and(|v| v.as_str() != Some(singular))
        {
            return Err(ConnectionError::InvalidResponse);
        }
        let item = super::resources::project(kind, value.clone())?;
        if scope(r).is_some_and(|ns| item.namespace.as_deref() != Some(ns))
            || !seen.insert((item.namespace, item.name))
        {
            return Err(ConnectionError::InvalidResponse);
        }
    }
    Ok(Page {
        items: list.items,
        partial: list.metadata.continuation.is_some_and(|s| !s.is_empty()),
    })
}
fn project_item(kind: ResourceKind, value: Value) -> Result<CollectionItem> {
    let item = super::resources::project(kind, value)?;
    let reference = ResourceReference {
        kind,
        namespace: item.namespace,
        name: item.name,
        uid: item.uid,
    };
    super::details::validate_reference(&reference).map_err(|_| ConnectionError::InvalidResponse)?;
    if reference.uid.is_none() {
        return Err(ConnectionError::InvalidResponse);
    }
    Ok(CollectionItem {
        reference,
        status: item.status,
        columns: item.columns,
    })
}
fn controlled_by(value: &Value, parent: &ResourceReference) -> bool {
    super::details::controlled_owner(value, parent.namespace.as_deref()).as_ref() == Some(parent)
}
fn associated(r: &CollectionRequest, value: &Value, replicas: &[ResourceReference]) -> bool {
    let parent = &r.reference;
    if scope(r)
        .is_some_and(|ns| value.pointer("/metadata/namespace").and_then(Value::as_str) != Some(ns))
    {
        return false;
    }
    match r.collection {
        CollectionKind::Pods if parent.kind == ResourceKind::Nodes => {
            value.pointer("/spec/nodeName").and_then(Value::as_str) == Some(&parent.name)
        }
        CollectionKind::Pods if parent.kind == ResourceKind::Deployments => {
            replicas.iter().any(|replica| controlled_by(value, replica))
        }
        CollectionKind::Pods | CollectionKind::ReplicaSets | CollectionKind::Jobs => {
            controlled_by(value, parent)
        }
        CollectionKind::EndpointSlices => {
            let (api, kind) = super::details::object_type(parent.kind);
            value
                .pointer("/metadata/ownerReferences")
                .and_then(Value::as_array)
                .is_some_and(|owners| {
                    owners.len() <= 32
                        && owners.iter().any(|owner| {
                            owner.get("apiVersion").and_then(Value::as_str) == Some(api)
                                && owner.get("kind").and_then(Value::as_str) == Some(kind)
                                && owner.get("name").and_then(Value::as_str) == Some(&parent.name)
                                && owner.get("uid").and_then(Value::as_str) == parent.uid.as_deref()
                                && owner
                                    .get("namespace")
                                    .is_none_or(|ns| ns.as_str() == parent.namespace.as_deref())
                        })
                })
        }
        CollectionKind::Events => {
            let (api, kind) = super::details::object_type(parent.kind);
            value.get("involvedObject").is_some_and(|object| {
                object.get("uid").and_then(Value::as_str) == parent.uid.as_deref()
                    && object.get("apiVersion").and_then(Value::as_str) == Some(api)
                    && object.get("kind").and_then(Value::as_str) == Some(kind)
                    && object.get("name").and_then(Value::as_str) == Some(&parent.name)
                    && if super::resources::namespaced(parent.kind) {
                        object.get("namespace").and_then(Value::as_str)
                            == parent.namespace.as_deref()
                    } else {
                        object
                            .get("namespace")
                            .is_none_or(|ns| ns.is_null() || ns.as_str() == Some(""))
                    }
            })
        }
        CollectionKind::ResourceQuotas | CollectionKind::LimitRanges => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::tests::{connected, fake, fixture};
    use serde_json::json;
    #[test]
    fn export_frontend_collection_contract() {
        let cases: Vec<_> = [
            (ResourceKind::Nodes, CollectionKind::Pods, json!({"spec":{"nodeName":"node-a","containers":[{"name":"main"}]},"status":{"phase":"Running","qosClass":"Burstable","containerStatuses":[{"name":"main","ready":true,"restartCount":2}]}})),
            (ResourceKind::Deployments, CollectionKind::ReplicaSets, json!({"spec":{"replicas":3},"status":{"replicas":3,"readyReplicas":2,"availableReplicas":2,"fullyLabeledReplicas":3}})),
            (ResourceKind::CronJobs, CollectionKind::Jobs, json!({"spec":{"completions":3,"parallelism":2,"suspend":false},"status":{"active":1,"succeeded":2,"failed":0}})),
            (ResourceKind::Services, CollectionKind::EndpointSlices, json!({"addressType":"IPv4","endpoints":[{"addresses":["10.0.0.1"],"conditions":{"ready":true}}],"ports":[{"port":8080,"protocol":"TCP"}]})),
            (ResourceKind::Deployments, CollectionKind::Events, json!({"type":"Normal","reason":"ScalingReplicaSet","message":"Scaled up replica set","count":2,"lastTimestamp":"2026-10-29T12:00:00Z","firstTimestamp":"2026-10-29T11:00:00Z","source":{"component":"deployment-controller"},"involvedObject":{"kind":"Deployment","name":"parent","namespace":"default","uid":"parent-uid","apiVersion":"apps/v1"}})),
            (ResourceKind::Namespaces, CollectionKind::ResourceQuotas, json!({"status":{"hard":{"requests.cpu":"4","requests.memory":"8Gi","pods":"20","limits.cpu":"8","limits.memory":"16Gi","requests.storage":"100Gi"},"used":{"requests.cpu":"1","requests.memory":"2Gi","pods":"3","limits.cpu":"2","limits.memory":"4Gi","requests.storage":"10Gi"}}})),
            (ResourceKind::Namespaces, CollectionKind::LimitRanges, json!({"spec":{"limits":[{"type":"Container","min":{"cpu":"100m","memory":"64Mi"},"max":{"cpu":"2","memory":"2Gi"},"default":{"cpu":"500m","memory":"256Mi"},"defaultRequest":{"cpu":"200m","memory":"128Mi"}}]}})),
        ].into_iter().map(|(parent_kind, collection, mut value)| {
            let r = request(parent_kind, collection);
            value["metadata"] = json!({"name":"child","namespace":scope(&r).unwrap_or("default"),"uid":"child-uid","resourceVersion":"10","annotations":{"example/credential":"PRIVATE"}});
            let mut response = ResourceCollection::empty(&r, CollectionState::Complete);
            response.items.push(project_item(collection.resource(), value).unwrap());
            let response = response.bounded().unwrap();
            let wire = serde_json::to_value(&response).unwrap();
            assert!(!wire.to_string().contains("PRIVATE"));
            json!({"request":r,"response":wire})
        }).collect();
        println!(
            "COLLECTION_CONTRACT_JSON:{}",
            serde_json::to_string(&cases).unwrap()
        );
    }
    fn request(kind: ResourceKind, collection: CollectionKind) -> CollectionRequest {
        CollectionRequest {
            cluster_id: "cluster".into(),
            session_id: "session".into(),
            read_id: 1,
            reference: ResourceReference {
                kind,
                namespace: super::super::resources::namespaced(kind).then(|| "default".into()),
                name: "parent".into(),
                uid: Some("parent-uid".into()),
            },
            collection,
        }
    }
    fn parent(r: &CollectionRequest) -> Value {
        let (api, kind) = super::super::details::object_type(r.reference.kind);
        let mut v = json!({"apiVersion":api,"kind":kind,"metadata":{"name":r.reference.name,"uid":r.reference.uid,"resourceVersion":"1"}});
        if let Some(ns) = &r.reference.namespace {
            v["metadata"]["namespace"] = json!(ns);
        }
        v
    }
    fn child(parent: &ResourceReference) -> Value {
        let (api, kind) = super::super::details::object_type(parent.kind);
        json!({"metadata":{"name":"child","namespace":"default","uid":"child-uid","resourceVersion":"1","ownerReferences":[{"apiVersion":api,"kind":kind,"name":parent.name,"uid":parent.uid,"controller":true}]}})
    }
    #[test]
    fn closed_contract_and_parent_allowlist() {
        let mut r = request(ResourceKind::Deployments, CollectionKind::Pods);
        assert!(validate(&r).is_ok());
        let mut wire = serde_json::to_value(&r).unwrap();
        wire["url"] = json!("https://untrusted");
        assert!(serde_json::from_value::<CollectionRequest>(wire).is_err());
        r.reference.uid = None;
        assert_eq!(validate(&r), Err(ConnectionError::InvalidInput));
        r = request(ResourceKind::Secrets, CollectionKind::Pods);
        assert_eq!(validate(&r), Err(ConnectionError::InvalidInput));
    }
    #[test]
    fn controller_associations_require_exact_identity() {
        for (kind, collection) in [
            (ResourceKind::Deployments, CollectionKind::ReplicaSets),
            (ResourceKind::CronJobs, CollectionKind::Jobs),
            (ResourceKind::ReplicaSets, CollectionKind::Pods),
            (ResourceKind::DaemonSets, CollectionKind::Pods),
            (ResourceKind::StatefulSets, CollectionKind::Pods),
            (ResourceKind::ReplicationControllers, CollectionKind::Pods),
            (ResourceKind::Jobs, CollectionKind::Pods),
        ] {
            let r = request(kind, collection);
            let mut value = child(&r.reference);
            assert!(associated(&r, &value, &[]));
            value["metadata"]["ownerReferences"][0]["uid"] = json!("replacement");
            assert!(!associated(&r, &value, &[]));
            value = child(&r.reference);
            value["metadata"]["ownerReferences"][0]["controller"] = json!(false);
            assert!(!associated(&r, &value, &[]));
            value = child(&r.reference);
            value["metadata"]["namespace"] = json!("other");
            assert!(!associated(&r, &value, &[]));
        }
    }
    #[test]
    fn deployment_pods_only_use_controlled_replica_set_identities() {
        let r = request(ResourceKind::Deployments, CollectionKind::Pods);
        let replica = project_item(ResourceKind::ReplicaSets, child(&r.reference))
            .unwrap()
            .reference;
        let value = child(&replica);
        assert!(!associated(&r, &value, &[]));
        assert!(associated(&r, &value, &[replica]));
        assert!(!associated(&r, &child(&r.reference), &[]));
    }
    #[test]
    fn service_labels_cannot_substitute_for_owner_uid() {
        let r = request(ResourceKind::Services, CollectionKind::EndpointSlices);
        let mut value = child(&r.reference);
        assert!(associated(&r, &value, &[]));
        value["metadata"]["ownerReferences"] = json!([]);
        value["metadata"]["labels"] = json!({"kubernetes.io/service-name":"parent"});
        assert!(!associated(&r, &value, &[]));
    }
    #[test]
    fn node_namespace_and_event_scopes_are_exact() {
        let r = request(ResourceKind::Nodes, CollectionKind::Pods);
        assert_eq!(
            list_path(&r, ResourceKind::Pods),
            "/api/v1/pods?limit=100&fieldSelector=spec.nodeName%3Dparent"
        );
        let mut value = json!({"metadata":{"namespace":"default"},"spec":{"nodeName":"parent"}});
        assert!(associated(&r, &value, &[]));
        value["spec"]["nodeName"] = json!("other");
        assert!(!associated(&r, &value, &[]));
        for collection in [CollectionKind::ResourceQuotas, CollectionKind::LimitRanges] {
            let r = request(ResourceKind::Namespaces, collection);
            assert!(list_path(&r, collection.resource()).contains("/namespaces/parent/"));
            assert!(!associated(
                &r,
                &json!({"metadata":{"namespace":"other"}}),
                &[]
            ));
        }
        let r = request(ResourceKind::Pods, CollectionKind::Events);
        let mut value = json!({"metadata":{"namespace":"default"},"involvedObject":{"apiVersion":"v1","kind":"Pod","namespace":"default","name":"parent","uid":"parent-uid"}});
        assert!(associated(&r, &value, &[]));
        value["involvedObject"]["uid"] = json!("old-uid");
        assert!(!associated(&r, &value, &[]));
    }
    #[test]
    fn slots_keep_cancellation_scoped_and_reject_late_work() {
        let mut r = request(ResourceKind::Deployments, CollectionKind::Pods);
        let root = CancellationToken::new();
        let mut slot = CollectionSlot::default();
        slot.cancel(r.clone()).unwrap();
        assert!(matches!(
            slot.start(&r, &root),
            Err(ConnectionError::StaleSession)
        ));
        r.read_id = 2;
        let old = slot.start(&r, &root).unwrap();
        let mut wrong = r.clone();
        wrong.reference.name = "wrong".into();
        assert_eq!(slot.cancel(wrong), Err(ConnectionError::InvalidInput));
        assert!(!old.is_cancelled());
        r.read_id = 3;
        let current = slot.start(&r, &root).unwrap();
        assert!(old.is_cancelled());
        let mut late = r.clone();
        late.read_id = 2;
        slot.cancel(late).unwrap();
        assert!(!current.is_cancelled());
        root.cancel();
        assert!(current.is_cancelled());
    }
    #[test]
    fn safe_projection_and_response_byte_budget() {
        let r = request(ResourceKind::Nodes, CollectionKind::Pods);
        let mut response = ResourceCollection::empty(&r, CollectionState::Complete);
        for i in 0..100 {
            let mut v = json!({"metadata":{"name":format!("pod-{i}"),"namespace":"default","uid":format!("uid-{i}"),"resourceVersion":"1","annotations":{"credential":"PRIVATE"}},"spec":{"containers":[{"name":"main","env":[{"name":"TOKEN","value":"PRIVATE"}]}]},"data":{"password":"PRIVATE"}});
            v["status"]["phase"] = json!("Running");
            let mut item = project_item(ResourceKind::Pods, v).unwrap();
            item.columns.insert("boundedTest".into(), "x".repeat(1024));
            response.items.push(item);
        }
        let response = response.bounded().unwrap();
        let wire = serde_json::to_vec(&response).unwrap();
        assert!(wire.len() <= RESPONSE_LIMIT);
        assert!(response.items.len() < 100);
        assert_eq!(response.state, CollectionState::Partial);
        assert!(!String::from_utf8(wire).unwrap().contains("PRIVATE"));
    }
    #[tokio::test]
    async fn fake_list_is_bounded_partial_and_parent_replacement_conflicts() {
        for replaced in [false, true] {
            let (_root, _catalog, engine, id, _) = fixture();
            let r = request(ResourceKind::Deployments, CollectionKind::ReplicaSets);
            let object = parent(&r);
            let child = child(&r.reference);
            let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let reads = count.clone();
            let opened = connected(&engine, id, fake(move |path| {
                let value = if path == "/version" { json!({"gitVersion":"v1.36.0"}) }
                else if path.ends_with("/deployments/parent") {
                    let mut value = object.clone();
                    if reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 0 {
                        value["metadata"]["resourceVersion"] = json!("2");
                        if replaced { value["metadata"]["uid"] = json!("replacement"); }
                    }
                    value
                } else {
                    assert_eq!(path, "/apis/apps/v1/namespaces/default/replicasets?limit=100");
                    json!({"apiVersion":"apps/v1","kind":"ReplicaSetList","metadata":{"continue":"PRIVATE-CONTINUATION"},"items":[child]})
                };
                (200, serde_json::to_vec(&value).unwrap())
            })).await;
            let mut r = r;
            r.cluster_id = opened.cluster_id;
            r.session_id = opened.session_id;
            let result = engine.resource_collection(r).await;
            if replaced {
                assert_eq!(result, Err(ConnectionError::Conflict));
            } else {
                let response = result.unwrap();
                assert_eq!(response.state, CollectionState::Partial);
                assert_eq!(response.items.len(), 1);
                assert!(
                    !serde_json::to_string(&response)
                        .unwrap()
                        .contains("PRIVATE")
                );
            }
            assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
        }
    }
    #[tokio::test]
    async fn rbac_and_missing_api_are_independent_collection_states() {
        for (status, expected) in [
            (403, CollectionState::Denied),
            (404, CollectionState::Unavailable),
        ] {
            let (_root, _catalog, engine, id, _) = fixture();
            let r = request(ResourceKind::Services, CollectionKind::EndpointSlices);
            let object = parent(&r);
            let opened = connected(
                &engine,
                id,
                fake(move |path| {
                    if path == "/version" {
                        (
                            200,
                            serde_json::to_vec(&json!({"gitVersion":"v1.36.0"})).unwrap(),
                        )
                    } else if path.ends_with("/services/parent") {
                        (200, serde_json::to_vec(&object).unwrap())
                    } else {
                        (status, b"PRIVATE server error".to_vec())
                    }
                }),
            )
            .await;
            let mut r = r;
            r.cluster_id = opened.cluster_id;
            r.session_id = opened.session_id;
            let response = engine.resource_collection(r).await.unwrap();
            assert_eq!(response.state, expected);
            assert!(response.items.is_empty());
            assert!(
                !serde_json::to_string(&response)
                    .unwrap()
                    .contains("PRIVATE")
            );
        }
    }
}
