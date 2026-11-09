use super::{ConnectionEngine, ConnectionError, Result, SessionRequest};
use http::Request;
use http_body_util::BodyExt;
use kube::{Client, client::Body};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio_util::sync::CancellationToken;

const BODY_LIMIT: usize = 2 * 1024 * 1024;
const TOTAL_LIMIT: usize = 10 * 1024 * 1024;
const SUBJECT_LIMIT: usize = 1000;
const BINDING_LIMIT: usize = 64;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RbacUsersRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub read_id: u64,
    pub namespace: Option<String>,
}
impl RbacUsersRequest {
    fn session(&self) -> SessionRequest {
        SessionRequest {
            cluster_id: self.cluster_id.clone(),
            session_id: self.session_id.clone(),
        }
    }
    fn validate(&self) -> Result<()> {
        super::validate_request(&self.session())?;
        super::validate_read_id(self.read_id)?;
        if self
            .namespace
            .as_deref()
            .is_some_and(|ns| !super::transport::valid_namespace(ns))
        {
            return Err(ConnectionError::InvalidInput);
        }
        Ok(())
    }
}
pub(super) struct UsersSlot {
    request: RbacUsersRequest,
    cancel: Option<CancellationToken>,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RbacUsersResponse {
    pub cluster_id: String,
    pub session_id: String,
    pub read_id: u64,
    pub namespace: Option<String>,
    pub items: Vec<SubjectRecord>,
    pub coverage: Vec<Coverage>,
    pub truncated: bool,
    pub fetched_at: u64,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubjectRecord {
    pub id: String,
    pub kind: &'static str,
    pub name: String,
    pub namespace: Option<String>,
    pub bindings: Vec<SubjectBinding>,
    pub bindings_truncated: bool,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubjectBinding {
    pub kind: &'static str,
    pub name: String,
    pub namespace: Option<String>,
    pub uid: String,
    pub resource_version: String,
    pub role_ref: SubjectRoleRef,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct SubjectRoleRef {
    pub kind: &'static str,
    pub name: String,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Coverage {
    pub resource: &'static str,
    pub state: &'static str,
}
impl ConnectionEngine {
    pub fn cancel_rbac_users(&self, request: RbacUsersRequest) -> Result<()> {
        request.validate()?;
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        if let Some(session) = state
            .as_mut()
            .filter(|s| super::matches_session(s, &request.session()))
        {
            if let Some(slot) = &session.rbac_users {
                if request.read_id == slot.request.read_id && request != slot.request {
                    return Err(ConnectionError::InvalidInput);
                }
                if request.read_id < slot.request.read_id {
                    return Ok(());
                }
            }
            if let Some(slot) = session.rbac_users.take()
                && let Some(cancel) = slot.cancel
            {
                cancel.cancel();
            }
            session.rbac_users = Some(UsersSlot {
                request,
                cancel: None,
            });
        }
        Ok(())
    }
    pub async fn rbac_users(&self, request: RbacUsersRequest) -> Result<RbacUsersResponse> {
        request.validate()?;
        let _permit = self
            .workers
            .try_acquire()
            .map_err(|_| ConnectionError::Busy)?;
        let (client, cancel) = {
            let mut state = self
                .session
                .lock()
                .map_err(|_| ConnectionError::StoreUnavailable)?;
            let session = super::current_mut(&mut state, &request.session())?;
            let client = session
                .client
                .clone()
                .ok_or(ConnectionError::NotConnected)?;
            super::validate_credentials(&client)?;
            if session
                .rbac_users
                .as_ref()
                .is_some_and(|s| request.read_id <= s.request.read_id)
            {
                return Err(ConnectionError::StaleSession);
            }
            let cancel = session.cancel.child_token();
            if let Some(slot) = session.rbac_users.replace(UsersSlot {
                request: request.clone(),
                cancel: Some(cancel.clone()),
            }) && let Some(old) = slot.cancel
            {
                old.cancel();
            }
            (client, cancel)
        };
        let result = super::bounded(&cancel, fetch(&client, &request, &cancel)).await;
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        let session = super::current_mut(&mut state, &request.session())?;
        super::validate_credentials(&client)?;
        let slot = session
            .rbac_users
            .as_mut()
            .ok_or(ConnectionError::StaleSession)?;
        if slot.request != request
            || (cancel.is_cancelled() && result != Err(ConnectionError::Timeout))
        {
            return Err(ConnectionError::StaleSession);
        }
        slot.cancel = None;
        result
    }
}
fn safe_text(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && value.trim() == value
        && !value.chars().any(|c| c.is_control() || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
}
fn field(value: &Value, path: &str, max: usize) -> Option<String> {
    value
        .pointer(path)
        .and_then(Value::as_str)
        .filter(|s| safe_text(s, max))
        .map(str::to_owned)
}
fn object_name(value: &Value, path: &str) -> Option<String> {
    field(value, path, 253).filter(|s| {
        !s.chars()
            .any(|c| c.is_whitespace() || matches!(c, '/' | '%' | '?' | '#'))
            && s != "."
            && s != ".."
    })
}
fn namespace(value: &Value, path: &str) -> Option<String> {
    field(value, path, 63).filter(|s| super::transport::valid_namespace(s))
}
fn subject(value: &Value, default_namespace: Option<&str>) -> Option<SubjectRecord> {
    let kind = match value.get("kind")?.as_str()? {
        "User" => "User",
        "Group" => "Group",
        "ServiceAccount" => "ServiceAccount",
        _ => return None,
    };
    let group = value.get("apiGroup").and_then(Value::as_str).unwrap_or("");
    if (kind == "ServiceAccount" && !group.is_empty())
        || (kind != "ServiceAccount" && group != "rbac.authorization.k8s.io")
    {
        return None;
    }
    let name = field(value, "/name", 1024)?;
    let ns = if kind == "ServiceAccount" {
        let ns = namespace(value, "/namespace").or_else(|| {
            if value
                .get("namespace")
                .is_none_or(|v| v.as_str() == Some(""))
            {
                default_namespace.map(str::to_owned)
            } else {
                None
            }
        })?;
        if name.len() > 253 || !name.split('.').all(super::transport::valid_namespace) {
            return None;
        }
        Some(ns)
    } else {
        None
    };
    // JSON tuple encoding avoids delimiter collisions in identities such as system:groups.
    let id = serde_json::to_string(&(kind, &ns, &name)).ok()?;
    Some(SubjectRecord {
        id,
        kind,
        name,
        namespace: ns,
        bindings: Vec::new(),
        bindings_truncated: false,
    })
}
fn binding(value: &Value, cluster: bool, scope: Option<&str>) -> Option<SubjectBinding> {
    let name = object_name(value, "/metadata/name")?;
    if cluster
        && value
            .pointer("/metadata/namespace")
            .is_some_and(|ns| !ns.is_null() && ns.as_str() != Some(""))
    {
        return None;
    }
    let ns = if cluster {
        None
    } else {
        Some(namespace(value, "/metadata/namespace")?)
    };
    if !cluster && scope.is_some_and(|s| ns.as_deref() != Some(s)) {
        return None;
    }
    if value.pointer("/roleRef/apiGroup")?.as_str()? != "rbac.authorization.k8s.io" {
        return None;
    }
    let kind = match value.pointer("/roleRef/kind")?.as_str()? {
        "ClusterRole" => "ClusterRole",
        "Role" if !cluster => "Role",
        _ => return None,
    };
    Some(SubjectBinding {
        kind: if cluster {
            "ClusterRoleBinding"
        } else {
            "RoleBinding"
        },
        name,
        namespace: ns,
        uid: field(value, "/metadata/uid", 256)?,
        resource_version: field(value, "/metadata/resourceVersion", 256)?,
        role_ref: SubjectRoleRef {
            kind,
            name: object_name(value, "/roleRef/name")?,
        },
    })
}
fn add_subject(
    items: &mut BTreeMap<String, SubjectRecord>,
    record: SubjectRecord,
    link: Option<&SubjectBinding>,
) -> bool {
    if !items.contains_key(&record.id) && items.len() >= SUBJECT_LIMIT {
        return false;
    }
    let item = items.entry(record.id.clone()).or_insert(record);
    if let Some(link) = link
        && !item.bindings.contains(link)
    {
        if item.bindings.len() >= BINDING_LIMIT {
            item.bindings_truncated = true;
            return false;
        }
        item.bindings.push(link.clone());
    }
    true
}
async fn page(
    client: &Client,
    path: &str,
    cancel: &CancellationToken,
    total: &mut usize,
) -> Result<Value> {
    let mut request = Request::get(path)
        .header("Accept", "application/json")
        .body(Body::empty())
        .map_err(|_| ConnectionError::InvalidInput)?;
    request.extensions_mut().insert(cancel.clone());
    let mut response = client
        .send(request)
        .await
        .map_err(|_| ConnectionError::Network)?;
    match response.status().as_u16() {
        200 => {}
        401 => return Err(ConnectionError::AuthFailed),
        403 => return Err(ConnectionError::PermissionDenied),
        404 => return Err(ConnectionError::ResourceUnavailable),
        _ => return Err(ConnectionError::ConnectionFailed),
    }
    let limit = BODY_LIMIT.min(TOTAL_LIMIT.saturating_sub(*total));
    if response
        .headers()
        .get("content-length")
        .and_then(|s| s.to_str().ok())
        .and_then(|s| s.parse::<usize>().ok())
        .is_some_and(|n| n > limit)
    {
        return Err(ConnectionError::LimitExceeded);
    }
    let mut bytes = Vec::new();
    while let Some(frame) = response.body_mut().frame().await {
        if let Ok(data) = frame.map_err(|_| ConnectionError::Network)?.into_data() {
            if data.len() > limit.saturating_sub(bytes.len()) {
                return Err(ConnectionError::LimitExceeded);
            }
            *total += data.len();
            bytes.extend_from_slice(&data);
        }
    }
    serde_json::from_slice(&bytes).map_err(|_| ConnectionError::InvalidResponse)
}
async fn fetch(
    client: &Client,
    request: &RbacUsersRequest,
    cancel: &CancellationToken,
) -> Result<RbacUsersResponse> {
    let ns_path = request
        .namespace
        .as_ref()
        .map(|ns| format!("/namespaces/{ns}"))
        .unwrap_or_default();
    let sources = [
        (
            "roleBindings",
            format!("/apis/rbac.authorization.k8s.io/v1{ns_path}/rolebindings"),
            "RoleBindingList",
        ),
        (
            "clusterRoleBindings",
            "/apis/rbac.authorization.k8s.io/v1/clusterrolebindings".into(),
            "ClusterRoleBindingList",
        ),
        (
            "serviceAccounts",
            format!("/api/v1{ns_path}/serviceaccounts"),
            "ServiceAccountList",
        ),
    ];
    let mut items = BTreeMap::new();
    let mut coverage = Vec::new();
    let mut truncated = false;
    let mut total = 0;
    for (resource, base, list_kind) in sources {
        let mut state = "complete";
        let mut continuation: Option<String> = None;
        let mut tokens = HashSet::new();
        let mut seen = HashSet::new();
        let mut uids = HashSet::new();
        for index in 0..5 {
            if total >= TOTAL_LIMIT {
                state = "partial";
                truncated = true;
                break;
            }
            let mut path = format!("{base}?limit=100&timeoutSeconds=10");
            if let Some(token) = &continuation {
                path.push_str("&continue=");
                path.extend(url::form_urlencoded::byte_serialize(token.as_bytes()));
            }
            let value = match page(client, &path, cancel, &mut total).await {
                Ok(value) => value,
                Err(ConnectionError::AuthFailed) => return Err(ConnectionError::AuthFailed),
                Err(ConnectionError::PermissionDenied) => {
                    state = "denied";
                    break;
                }
                Err(ConnectionError::LimitExceeded) => {
                    state = "partial";
                    truncated = true;
                    break;
                }
                Err(_) => {
                    state = if index == 0 { "unavailable" } else { "partial" };
                    break;
                }
            };
            let Some(rows) = value
                .get("items")
                .and_then(Value::as_array)
                .filter(|rows| rows.len() <= 100)
            else {
                state = "partial";
                break;
            };
            if value.get("kind").and_then(Value::as_str) != Some(list_kind) {
                state = "partial";
                break;
            }
            for row in rows {
                let uid = field(row, "/metadata/uid", 256);
                let identity = (
                    field(row, "/metadata/namespace", 63),
                    object_name(row, "/metadata/name"),
                );
                if uid.is_none()
                    || identity.1.is_none()
                    || !uids.insert(uid)
                    || !seen.insert(identity)
                {
                    state = "partial";
                    continue;
                }
                if resource == "serviceAccounts" {
                    let Some(ns) = namespace(row, "/metadata/namespace") else {
                        state = "partial";
                        continue;
                    };
                    if request.namespace.as_ref().is_some_and(|scope| scope != &ns) {
                        state = "partial";
                        continue;
                    }
                    let Some(name) = object_name(row, "/metadata/name") else {
                        state = "partial";
                        continue;
                    };
                    let value =
                        serde_json::json!({"kind":"ServiceAccount","name":name,"namespace":ns});
                    if let Some(record) = subject(&value, None) {
                        if !add_subject(&mut items, record, None) {
                            state = "partial";
                            truncated = true;
                        }
                    } else {
                        state = "partial";
                    }
                } else {
                    let Some(link) = binding(
                        row,
                        resource == "clusterRoleBindings",
                        request.namespace.as_deref(),
                    ) else {
                        state = "partial";
                        continue;
                    };
                    let subjects = match row.get("subjects") {
                        None | Some(Value::Null) => continue,
                        Some(Value::Array(subjects)) => subjects,
                        _ => {
                            state = "partial";
                            continue;
                        }
                    };
                    let mut subject_ids = HashSet::new();
                    for raw in subjects {
                        let Some(record) = subject(raw, link.namespace.as_deref()) else {
                            state = "partial";
                            continue;
                        };
                        if !subject_ids.insert(record.id.clone()) {
                            state = "partial";
                            continue;
                        }
                        if !add_subject(&mut items, record, Some(&link)) {
                            state = "partial";
                            truncated = true;
                        }
                    }
                }
            }
            continuation = match value.pointer("/metadata/continue") {
                None | Some(Value::Null) => None,
                Some(Value::String(token)) if token.is_empty() => None,
                Some(Value::String(token)) if safe_text(token, 4096) => Some(token.clone()),
                _ => {
                    state = "partial";
                    break;
                }
            };
            let Some(token) = &continuation else {
                break;
            };
            if !tokens.insert(token.clone()) || index == 4 {
                state = "partial";
                truncated = true;
                break;
            }
        }
        coverage.push(Coverage { resource, state });
    }
    Ok(RbacUsersResponse {
        cluster_id: request.cluster_id.clone(),
        session_id: request.session_id.clone(),
        read_id: request.read_id,
        namespace: request.namespace.clone(),
        items: items.into_values().collect(),
        coverage,
        truncated,
        fetched_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ConnectionError::ConnectionFailed)?
            .as_millis() as u64,
    })
}
#[cfg(test)]
mod tests;
