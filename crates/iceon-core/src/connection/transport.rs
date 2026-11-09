use super::{ConnectionError, ResourceKind, ResourcePage, ResourceRequest, Result};
use http::Request;
use http_body_util::BodyExt;
use kube::{Client, client::Body};
use serde::{Deserialize, de::DeserializeOwned};
use std::{
    collections::HashSet,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio_util::sync::CancellationToken;

const BODY_LIMIT: usize = 2 * 1024 * 1024;
const ITEM_LIMIT: usize = 100;
const CRD_PAGE_LIMIT: usize = 10;
// CRD schemas can exceed the ordinary inventory budget. Only a single-object
// retry gets this allowance; schema content never enters the projected DTO.
const CRD_SINGLE_BODY_LIMIT: usize = 16 * 1024 * 1024;
const CRD_PROJECTION_LIMIT: usize = 64 * 1024;
const SCAN_PAGE_LIMIT: usize = 10;
const SCAN_ITEM_LIMIT: usize = 1_000;
pub(super) mod nodes;

#[derive(Clone)]
pub(super) struct CancelService<S>(pub S);
impl<S, B> tower::Service<Request<Body>> for CancelService<S>
where
    S: tower::Service<Request<Body>, Response = http::Response<B>>,
    S::Future: Send + 'static,
    S::Error: Into<tower::BoxError>,
    B: Send + 'static,
{
    type Response = http::Response<B>;
    type Error = tower::BoxError;
    type Future = std::pin::Pin<
        Box<
            dyn std::future::Future<Output = std::result::Result<Self::Response, Self::Error>>
                + Send,
        >,
    >;
    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::result::Result<(), Self::Error>> {
        self.0.poll_ready(cx).map_err(Into::into)
    }
    fn call(&mut self, request: Request<Body>) -> Self::Future {
        let cancel = request
            .extensions()
            .get::<CancellationToken>()
            .cloned()
            .unwrap_or_default();
        let future = self.0.call(request);
        Box::pin(async move {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "request cancelled").into()),
                result = tokio::time::timeout(super::DEADLINE, future) => result.map_err(|_| -> tower::BoxError { std::io::Error::new(std::io::ErrorKind::TimedOut, "request deadline").into() })?.map_err(Into::into),
            }
        })
    }
}

pub(super) async fn version(client: &Client, cancel: &CancellationToken) -> Result<String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Version {
        git_version: String,
    }

    let version: Version = get(client, "/version", 64 * 1024, cancel).await?;
    if version.git_version.is_empty()
        || version.git_version.len() > 256
        || version.git_version.chars().any(char::is_control)
    {
        return Err(ConnectionError::InvalidResponse);
    }
    Ok(version.git_version)
}

pub(super) async fn resources(
    client: &Client,
    request: &ResourceRequest,
    cancel: &CancellationToken,
) -> Result<ResourcePage> {
    validate_resource_request(request)?;
    let mut continuation = request.continue_token.clone();
    let mut items = Vec::new();
    let mut scanned_pages = 0;
    let mut scanned_items = 0;
    let mut adaptive_limit = page_limit(request.kind);
    let mut projected_bytes = 0;
    let mut seen_tokens = HashSet::new();
    let mut seen_names = HashSet::new();
    if let Some(token) = &continuation {
        seen_tokens.insert(token.clone());
    }
    let mut metrics = None;
    let filtered = request.query.is_some() || request.status.is_some() || request.group.is_some();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if cancel.is_cancelled() {
            return Err(ConnectionError::StaleSession);
        }
        let limit = ITEM_LIMIT.saturating_sub(items.len()).min(adaptive_limit);
        let fetch = fetch_resource_page(client, request, continuation.as_deref(), limit, cancel);
        let (page, fetched_limit) = if filtered {
            match tokio::time::timeout_at(deadline, fetch).await {
                Ok(result) => result?,
                Err(_) if scanned_pages > 0 => break,
                Err(_) => return Err(ConnectionError::Timeout),
            }
        } else {
            fetch.await?
        };
        adaptive_limit = fetched_limit;
        if request.kind == ResourceKind::CustomResourceDefinitions {
            projected_bytes += serde_json::to_vec(&page.items)
                .map_err(|_| ConnectionError::InvalidResponse)?
                .len();
            if projected_bytes > CRD_PROJECTION_LIMIT {
                return Err(ConnectionError::LimitExceeded);
            }
        }
        scanned_pages += 1;
        scanned_items += page.items.len();
        items.extend(
            page.items
                .into_iter()
                .filter(|item| seen_names.insert((item.namespace.clone(), item.name.clone())))
                .filter(|item| matches_filter(request, item)),
        );
        continuation = page.continue_token;
        if let Some(token) = &continuation
            && !seen_tokens.insert(token.clone())
        {
            return Err(ConnectionError::InvalidResponse);
        }
        if continuation.is_none()
            || !filtered
            || items.len() >= ITEM_LIMIT
            || scanned_pages >= SCAN_PAGE_LIMIT
            || scanned_items >= SCAN_ITEM_LIMIT
        {
            break;
        }
    }
    if request.kind == ResourceKind::Nodes {
        metrics = Some(super::metrics::enrich(client, &mut items, cancel).await?);
    }
    let truncated = continuation.is_some();
    items.sort_by(|a, b| (&a.namespace, &a.name).cmp(&(&b.namespace, &b.name)));
    Ok(ResourcePage {
        cluster_id: request.cluster_id.clone(),
        session_id: request.session_id.clone(),
        kind: request.kind,
        read_id: request.read_id,
        namespace: request.namespace.clone(),
        truncated,
        continue_token: continuation,
        items,
        metrics,
        fetched_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ConnectionError::ConnectionFailed)?
            .as_millis() as u64,
    })
}

fn page_limit(kind: ResourceKind) -> usize {
    if kind == ResourceKind::CustomResourceDefinitions {
        CRD_PAGE_LIMIT
    } else {
        ITEM_LIMIT
    }
}

async fn fetch_resource_page(
    client: &Client,
    request: &ResourceRequest,
    continuation: Option<&str>,
    limit: usize,
    cancel: &CancellationToken,
) -> Result<(ResourcePage, usize)> {
    let fetch = async {
        let path = resource_page_path(request, continuation, limit);
        let body_limit = if request.kind == ResourceKind::CustomResourceDefinitions && limit == 1 {
            CRD_SINGLE_BODY_LIMIT
        } else {
            BODY_LIMIT
        };
        match fetch_page(client, request, &path, limit, body_limit, cancel).await {
            Err(ConnectionError::LimitExceeded)
                if request.kind == ResourceKind::CustomResourceDefinitions && limit > 1 =>
            {
                if cancel.is_cancelled() {
                    return Err(ConnectionError::StaleSession);
                }
                // Retry the same unconsumed token, not the next server page.
                let path = resource_page_path(request, continuation, 1);
                fetch_page(client, request, &path, 1, CRD_SINGLE_BODY_LIMIT, cancel)
                    .await
                    .map(|page| (page, 1))
            }
            result => result.map(|page| (page, limit)),
        }
    };
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(ConnectionError::StaleSession),
        result = tokio::time::timeout(super::DEADLINE, fetch) => result.map_err(|_| ConnectionError::Timeout)?,
    }
}

async fn fetch_page(
    client: &Client,
    request: &ResourceRequest,
    path: &str,
    limit: usize,
    body_limit: usize,
    cancel: &CancellationToken,
) -> Result<ResourcePage> {
    let accept = if matches!(
        request.kind,
        ResourceKind::Secrets | ResourceKind::ConfigMaps
    ) {
        "application/json;as=PartialObjectMetadataList;g=meta.k8s.io;v=v1"
    } else {
        "application/json"
    };
    let list: List = get_with_accept(client, path, body_limit, cancel, accept).await?;
    if matches!(
        request.kind,
        ResourceKind::Secrets | ResourceKind::ConfigMaps
    ) && list.kind.as_deref() != Some("PartialObjectMetadataList")
    {
        return Err(ConnectionError::InvalidResponse);
    }
    if list.items.len() > limit {
        return Err(ConnectionError::LimitExceeded);
    }
    let continue_token = list.metadata.continue_token.filter(|s| !s.is_empty());
    if continue_token.as_deref().is_some_and(|s| !valid_token(s)) {
        return Err(ConnectionError::InvalidResponse);
    }
    let mut names = HashSet::new();
    let mut items = Vec::new();
    let mut projected_bytes = 0;
    for raw in list.items {
        let item = super::resources::project(request.kind, raw)?;
        if !names.insert((item.namespace.clone(), item.name.clone()))
            || (request.namespace.is_some() && item.namespace != request.namespace)
            || (!super::resources::namespaced(request.kind) && item.namespace.is_some())
        {
            return Err(ConnectionError::InvalidResponse);
        }
        if request.kind == ResourceKind::CustomResourceDefinitions {
            projected_bytes += serde_json::to_vec(&item)
                .map_err(|_| ConnectionError::InvalidResponse)?
                .len();
            if projected_bytes > CRD_PROJECTION_LIMIT {
                return Err(ConnectionError::LimitExceeded);
            }
        }
        items.push(item);
    }
    items.sort_by(|a, b| (&a.namespace, &a.name).cmp(&(&b.namespace, &b.name)));
    Ok(ResourcePage {
        cluster_id: request.cluster_id.clone(),
        session_id: request.session_id.clone(),
        kind: request.kind,
        read_id: request.read_id,
        namespace: request.namespace.clone(),
        truncated: continue_token.is_some(),
        continue_token,
        items,
        metrics: None,
        fetched_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ConnectionError::ConnectionFailed)?
            .as_millis() as u64,
    })
}
fn matches_filter(request: &ResourceRequest, item: &super::ResourceItem) -> bool {
    if request
        .status
        .as_deref()
        .is_some_and(|status| status != item.status)
    {
        return false;
    }
    if request
        .group
        .as_deref()
        .is_some_and(|group| item.columns.get("group").map(String::as_str) != Some(group))
    {
        return false;
    }
    request.query.as_deref().is_none_or(|query| {
        let query = query.to_lowercase();
        [
            item.name.as_str(),
            item.namespace.as_deref().unwrap_or(""),
            item.status.as_str(),
        ]
        .into_iter()
        .chain(item.columns.values().map(String::as_str))
        .chain(item.node.iter().flat_map(|node| {
            node.internal_ips
                .iter()
                .chain(node.roles.iter())
                .chain(node.version.iter())
                .map(String::as_str)
                .chain(
                    node.conditions
                        .iter()
                        .flat_map(|condition| [condition.kind.as_str(), condition.status.as_str()]),
                )
        }))
        .any(|value| value.to_lowercase().contains(&query))
    })
}
pub(super) fn valid_namespace(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && s.as_bytes()[0].is_ascii_alphanumeric()
        && s.as_bytes()[s.len() - 1].is_ascii_alphanumeric()
}
fn valid_token(s: &str) -> bool {
    s.len() <= 4096 && !s.chars().any(char::is_control)
}
pub(super) fn validate_resource_request(request: &ResourceRequest) -> Result<()> {
    if request
        .namespace
        .as_deref()
        .is_some_and(|s| !super::resources::namespaced(request.kind) || !valid_namespace(s))
        || request
            .continue_token
            .as_deref()
            .is_some_and(|s| !valid_token(s))
    {
        return Err(ConnectionError::InvalidInput);
    }
    for value in [&request.query, &request.status, &request.group]
        .into_iter()
        .filter_map(Option::as_deref)
    {
        if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
            return Err(ConnectionError::InvalidInput);
        }
    }
    if request.group.is_some() && request.kind != ResourceKind::CustomResourceDefinitions {
        return Err(ConnectionError::InvalidInput);
    }
    Ok(())
}
pub(super) fn resource_path(request: &ResourceRequest) -> String {
    resource_page_path(
        request,
        request.continue_token.as_deref(),
        page_limit(request.kind),
    )
}
fn resource_page_path(
    request: &ResourceRequest,
    continuation: Option<&str>,
    limit: usize,
) -> String {
    let (group, plural) = endpoint(request.kind);
    let scope = request
        .namespace
        .as_ref()
        .map(|s| format!("/namespaces/{s}"))
        .unwrap_or_default();
    let mut path = format!("{group}{scope}/{plural}?limit={limit}&timeoutSeconds=10");
    if let Some(token) = continuation {
        let query: String = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("continue", token)
            .finish();
        path.push('&');
        path.push_str(&query);
    }
    path
}

pub(super) fn endpoint(kind: ResourceKind) -> (&'static str, &'static str) {
    use ResourceKind::*;
    match kind {
        Nodes => ("/api/v1", "nodes"),
        Namespaces => ("/api/v1", "namespaces"),
        Pods => ("/api/v1", "pods"),
        ConfigMaps => ("/api/v1", "configmaps"),
        Secrets => ("/api/v1", "secrets"),
        ResourceQuotas => ("/api/v1", "resourcequotas"),
        LimitRanges => ("/api/v1", "limitranges"),
        Services => ("/api/v1", "services"),
        Endpoints => ("/api/v1", "endpoints"),
        PersistentVolumes => ("/api/v1", "persistentvolumes"),
        PersistentVolumeClaims => ("/api/v1", "persistentvolumeclaims"),
        Events => ("/api/v1", "events"),
        ServiceAccounts => ("/api/v1", "serviceaccounts"),
        Deployments => ("/apis/apps/v1", "deployments"),
        DaemonSets => ("/apis/apps/v1", "daemonsets"),
        StatefulSets => ("/apis/apps/v1", "statefulsets"),
        ReplicaSets => ("/apis/apps/v1", "replicasets"),
        ReplicationControllers => ("/api/v1", "replicationcontrollers"),
        Jobs => ("/apis/batch/v1", "jobs"),
        CronJobs => ("/apis/batch/v1", "cronjobs"),
        HorizontalPodAutoscalers => ("/apis/autoscaling/v2", "horizontalpodautoscalers"),
        VerticalPodAutoscalers => ("/apis/autoscaling.k8s.io/v1", "verticalpodautoscalers"),
        PodDisruptionBudgets => ("/apis/policy/v1", "poddisruptionbudgets"),
        PriorityClasses => ("/apis/scheduling.k8s.io/v1", "priorityclasses"),
        RuntimeClasses => ("/apis/node.k8s.io/v1", "runtimeclasses"),
        Leases => ("/apis/coordination.k8s.io/v1", "leases"),
        Ingresses => ("/apis/networking.k8s.io/v1", "ingresses"),
        IngressClasses => ("/apis/networking.k8s.io/v1", "ingressclasses"),
        NetworkPolicies => ("/apis/networking.k8s.io/v1", "networkpolicies"),
        EndpointSlices => ("/apis/discovery.k8s.io/v1", "endpointslices"),
        MutatingWebhookConfigurations => (
            "/apis/admissionregistration.k8s.io/v1",
            "mutatingwebhookconfigurations",
        ),
        ValidatingWebhookConfigurations => (
            "/apis/admissionregistration.k8s.io/v1",
            "validatingwebhookconfigurations",
        ),
        ValidatingAdmissionPolicies => (
            "/apis/admissionregistration.k8s.io/v1",
            "validatingadmissionpolicies",
        ),
        ValidatingAdmissionPolicyBindings => (
            "/apis/admissionregistration.k8s.io/v1",
            "validatingadmissionpolicybindings",
        ),
        PodSecurityPolicies => ("/apis/policy/v1beta1", "podsecuritypolicies"),
        StorageClasses => ("/apis/storage.k8s.io/v1", "storageclasses"),
        Roles => ("/apis/rbac.authorization.k8s.io/v1", "roles"),
        RoleBindings => ("/apis/rbac.authorization.k8s.io/v1", "rolebindings"),
        ClusterRoles => ("/apis/rbac.authorization.k8s.io/v1", "clusterroles"),
        ClusterRoleBindings => ("/apis/rbac.authorization.k8s.io/v1", "clusterrolebindings"),
        CustomResourceDefinitions => ("/apis/apiextensions.k8s.io/v1", "customresourcedefinitions"),
    }
}

// Only this allowlisted request path reaches the client. API error bodies are never surfaced.
pub(super) async fn get<T: DeserializeOwned>(
    client: &Client,
    path: &str,
    limit: usize,
    cancel: &CancellationToken,
) -> Result<T> {
    get_with_accept(client, path, limit, cancel, "application/json").await
}
pub(super) async fn get_with_accept<T: DeserializeOwned>(
    client: &Client,
    path: &str,
    limit: usize,
    cancel: &CancellationToken,
    accept: &str,
) -> Result<T> {
    let mut request = Request::get(path)
        .header("Accept", accept)
        .body(Body::empty())
        .map_err(|_| ConnectionError::InvalidInput)?;
    request.extensions_mut().insert(cancel.clone());
    let mut response = client.send(request).await.map_err(map_kube_error)?;
    match response.status().as_u16() {
        200 => {}
        401 => return Err(ConnectionError::AuthFailed),
        403 => return Err(ConnectionError::PermissionDenied),
        404 => return Err(ConnectionError::ResourceUnavailable),
        408 | 504 => return Err(ConnectionError::Timeout),
        _ => return Err(ConnectionError::ConnectionFailed),
    }
    if response
        .headers()
        .get("content-length")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<usize>().ok())
        .is_some_and(|length| length > limit)
    {
        return Err(ConnectionError::LimitExceeded);
    }
    let mut bytes = Vec::new();
    while let Some(frame) = response.body_mut().frame().await {
        let frame = frame.map_err(map_kube_error)?;
        if let Ok(data) = frame.into_data() {
            if data.len() > limit.saturating_sub(bytes.len()) {
                return Err(ConnectionError::LimitExceeded);
            }
            bytes.extend_from_slice(&data);
        }
    }
    serde_json::from_slice(&bytes).map_err(|_| ConnectionError::InvalidResponse)
}
fn map_kube_error(error: kube::Error) -> ConnectionError {
    match error {
        kube::Error::Auth(_) => ConnectionError::AuthFailed,
        kube::Error::RustlsTls(_) | kube::Error::TlsRequired => ConnectionError::TlsFailed,
        kube::Error::Service(error) => {
            let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(error.as_ref());
            for _ in 0..16 {
                let Some(current) = cause else {
                    break;
                };
                if let Some(error) = current.downcast_ref::<super::renewal::RenewalError>() {
                    return error.0;
                }
                if current.is::<rustls::Error>() {
                    return ConnectionError::TlsFailed;
                }
                if let Some(io) = current.downcast_ref::<std::io::Error>()
                    && io.kind() == std::io::ErrorKind::TimedOut
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
#[derive(Deserialize)]
struct List {
    kind: Option<String>,
    #[serde(default)]
    metadata: ListMetadata,
    items: Vec<serde_json::Value>,
}
#[derive(Default, Deserialize)]
struct ListMetadata {
    #[serde(default, rename = "continue")]
    continue_token: Option<String>,
}

pub(super) trait ResourceTransport: Sync {
    fn detail(
        &self,
        _client: &Client,
        _reference: &super::details::ResourceReference,
        _cancel: &CancellationToken,
    ) -> impl std::future::Future<Output = Result<serde_json::Value>> + Send {
        async { Err(ConnectionError::ResourceUnavailable) }
    }
}
pub(super) struct LiveResourceTransport;
impl ResourceTransport for LiveResourceTransport {
    async fn detail(
        &self,
        client: &Client,
        reference: &super::details::ResourceReference,
        cancel: &CancellationToken,
    ) -> Result<serde_json::Value> {
        let path = super::details::named_path(reference)?;
        let accept = if matches!(
            reference.kind,
            ResourceKind::Secrets | ResourceKind::ConfigMaps
        ) {
            "application/json;as=PartialObjectMetadata;g=meta.k8s.io;v=v1"
        } else {
            "application/json"
        };
        get_with_accept(client, &path, BODY_LIMIT, cancel, accept).await
    }
}
