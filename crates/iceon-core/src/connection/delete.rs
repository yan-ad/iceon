use super::permissions::{PermissionAction, PermissionCheck};
use super::{
    ConnectionEngine, ConnectionError, ResourceKind, Result, SessionRequest,
    create::{creation_info, plural},
    validate_credentials, validate_read_id,
};
use http::{Method, Request};
use http_body_util::BodyExt;
use kube::{Client, client::Body};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

const BODY_LIMIT: usize = 256 * 1024;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub kind: ResourceKind,
    #[serde(default)]
    pub namespace: Option<String>,
    pub name: String,
    pub uid: String,
    pub resource_version: String,
    pub read_id: u64,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteCancelRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub kind: ResourceKind,
    pub read_id: u64,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeleteResult {
    pub cluster_id: String,
    pub session_id: String,
    pub kind: ResourceKind,
    pub namespace: Option<String>,
    pub name: String,
    pub uid: String,
    pub resource_version: String,
    pub deleted: bool,
    pub accepted: bool,
}

pub(crate) struct DeleteSlot {
    pub(crate) id: u64,
    high_water: u64,
    pub(crate) cancel: Option<CancellationToken>,
}

fn identity(r: &DeleteRequest) -> SessionRequest {
    SessionRequest {
        cluster_id: r.cluster_id.clone(),
        session_id: r.session_id.clone(),
    }
}
fn safe(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 256
        && !s.contains('\0')
        && s.chars().all(|c| !c.is_control() && !c.is_whitespace())
}
pub(super) fn validate(r: &DeleteRequest) -> Result<Option<String>> {
    validate_read_id(r.read_id)?;
    super::validate_request(&identity(r))?;
    if !super::resources::valid_name(&r.name) || !safe(&r.uid) || !safe(&r.resource_version) {
        return Err(ConnectionError::InvalidInput);
    }
    let namespaced = super::resources::namespaced(r.kind);
    if namespaced {
        let ns = r
            .namespace
            .as_deref()
            .ok_or(ConnectionError::InvalidInput)?;
        if !super::transport::valid_namespace(ns) {
            return Err(ConnectionError::InvalidInput);
        }
        Ok(Some(ns.to_owned()))
    } else if r.namespace.is_some() {
        Err(ConnectionError::InvalidInput)
    } else {
        Ok(None)
    }
}

impl ConnectionEngine {
    pub fn cancel_delete(&self, r: DeleteCancelRequest) -> Result<()> {
        validate_read_id(r.read_id)?;
        let id = SessionRequest {
            cluster_id: r.cluster_id,
            session_id: r.session_id,
        };
        super::validate_request(&id)?;
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        if let Some(session) = state.as_mut().filter(|s| super::matches_session(s, &id)) {
            let slot = session.deletes.entry(r.kind).or_insert(DeleteSlot {
                id: 0,
                high_water: 0,
                cancel: None,
            });
            slot.high_water = slot.high_water.max(r.read_id);
            if r.read_id == slot.id
                && let Some(token) = slot.cancel.take()
            {
                token.cancel();
            }
        }
        Ok(())
    }

    pub async fn delete(&self, r: DeleteRequest) -> Result<DeleteResult> {
        let namespace = validate(&r)?;
        let id = identity(&r);
        let authorization = self
            .require_permissions(
                &id,
                &[
                    PermissionCheck {
                        kind: r.kind,
                        action: PermissionAction::Get,
                        namespace: namespace.clone(),
                        name: Some(r.name.clone()),
                    },
                    PermissionCheck {
                        kind: r.kind,
                        action: PermissionAction::Delete,
                        namespace: namespace.clone(),
                        name: Some(r.name.clone()),
                    },
                ],
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
            let session = super::current_mut(&mut state, &id)?;
            let client = session
                .client
                .clone()
                .ok_or(ConnectionError::NotConnected)?;
            validate_credentials(&client)?;
            let cancel = session.cancel.child_token();
            let slot = session.deletes.entry(r.kind).or_insert(DeleteSlot {
                id: 0,
                high_water: 0,
                cancel: None,
            });
            if r.read_id <= slot.high_water {
                return Err(ConnectionError::StaleSession);
            }
            if slot.cancel.as_ref().is_some_and(|c| !c.is_cancelled()) {
                return Err(ConnectionError::Busy);
            }
            slot.id = r.read_id;
            slot.high_water = r.read_id;
            slot.cancel = Some(cancel.clone());
            (client, cancel)
        };
        let client = super::permissions::authorized_client(client, authorization);
        let _guard = CancelOnDrop(cancel.clone());
        let result = super::bounded(
            &cancel,
            delete_transport(&client, &r, namespace.as_deref(), &cancel),
        )
        .await;
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        let session = super::current_mut(&mut state, &id)?;
        if session
            .deletes
            .get(&r.kind)
            .is_none_or(|s| s.id != r.read_id)
        {
            return Err(ConnectionError::StaleSession);
        }
        session.deletes.get_mut(&r.kind).unwrap().cancel = None;
        if cancel.is_cancelled() && result != Err(ConnectionError::Timeout) {
            return Err(ConnectionError::StaleSession);
        }
        result.map(|(deleted, accepted)| DeleteResult {
            cluster_id: r.cluster_id,
            session_id: r.session_id,
            kind: r.kind,
            namespace,
            name: r.name,
            uid: r.uid,
            resource_version: r.resource_version,
            deleted,
            accepted,
        })
    }
}

async fn delete_transport(
    client: &Client,
    r: &DeleteRequest,
    namespace: Option<&str>,
    cancel: &CancellationToken,
) -> Result<(bool, bool)> {
    let (api, _) = creation_info(r.kind);
    let prefix = if api == "v1" {
        "/api/v1".to_string()
    } else {
        format!("/apis/{api}")
    };
    let scope = namespace
        .map(|n| format!("/namespaces/{n}"))
        .unwrap_or_default();
    let path = format!("{scope}/{}/{}", plural(r.kind), r.name);
    let (get_status, current) = send_raw(
        client,
        cancel,
        Method::GET,
        &format!("{prefix}{path}"),
        None,
    )
    .await?;
    if get_status != 200 {
        return Err(ConnectionError::InvalidResponse);
    }
    verify_object(&current, r, namespace)?;
    if current.pointer("/metadata/uid").and_then(Value::as_str) != Some(&r.uid)
        || current
            .pointer("/metadata/resourceVersion")
            .and_then(Value::as_str)
            != Some(&r.resource_version)
    {
        return Err(ConnectionError::Conflict);
    }
    let options = json!({"apiVersion":"v1", "kind":"DeleteOptions", "propagationPolicy":"Background", "preconditions":{"uid":r.uid,"resourceVersion":r.resource_version}});
    validate_credentials(client)?;
    if cancel.is_cancelled() {
        return Err(ConnectionError::StaleSession);
    }
    let response = send_raw(
        client,
        cancel,
        Method::DELETE,
        &format!("{prefix}{path}"),
        Some(options),
    )
    .await?;
    let (status, value) = response;
    if status != 200 && status != 202 && status != 204 {
        return Err(ConnectionError::InvalidResponse);
    }
    if status == 204 {
        if !value.is_null() {
            return Err(ConnectionError::InvalidResponse);
        }
        return Ok((true, false));
    }
    if value.get("kind").and_then(Value::as_str) == Some("Status") {
        let group = api.split_once('/').map_or("", |(group, _)| group);
        if value.get("apiVersion").and_then(Value::as_str) != Some("v1")
            || value.get("status").and_then(Value::as_str) != Some("Success")
            || value.pointer("/details/name").and_then(Value::as_str) != Some(&r.name)
            || value.pointer("/details/uid").and_then(Value::as_str) != Some(&r.uid)
            || ![Some(creation_info(r.kind).1), Some(plural(r.kind))]
                .contains(&value.pointer("/details/kind").and_then(Value::as_str))
            || value
                .pointer("/details/group")
                .is_some_and(|v| v.as_str() != Some(group))
            || value
                .pointer("/details/namespace")
                .is_some_and(|v| v.as_str() != namespace)
            || value
                .get("code")
                .is_some_and(|v| v.as_u64() != Some(status as u64))
        {
            return Err(ConnectionError::InvalidResponse);
        }
    } else {
        verify_object(&value, r, namespace)?;
        if value.pointer("/metadata/uid").and_then(Value::as_str) != Some(&r.uid) {
            return Err(ConnectionError::InvalidResponse);
        }
    }
    // Even HTTP 200 can return an object waiting for grace or finalizers.
    Ok((false, true))
}

struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
fn verify_object(value: &Value, r: &DeleteRequest, namespace: Option<&str>) -> Result<()> {
    let (api, kind) = creation_info(r.kind);
    if value.get("apiVersion").and_then(Value::as_str) != Some(api)
        || value.get("kind").and_then(Value::as_str) != Some(kind)
        || value.pointer("/metadata/name").and_then(Value::as_str) != Some(&r.name)
        || value.pointer("/metadata/namespace").and_then(Value::as_str) != namespace
        || !value
            .pointer("/metadata/uid")
            .and_then(Value::as_str)
            .is_some_and(safe)
        || !value
            .pointer("/metadata/resourceVersion")
            .and_then(Value::as_str)
            .is_some_and(safe)
    {
        return Err(ConnectionError::InvalidResponse);
    }
    Ok(())
}

async fn send_raw(
    client: &Client,
    cancel: &CancellationToken,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<(u16, Value)> {
    let bytes = body
        .map(|v| serde_json::to_vec(&v))
        .transpose()
        .map_err(|_| ConnectionError::InvalidInput)?
        .unwrap_or_default();
    if bytes.len() > BODY_LIMIT {
        return Err(ConnectionError::LimitExceeded);
    }
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .body(Body::from(bytes))
        .map_err(|_| ConnectionError::InvalidInput)?;
    req.extensions_mut().insert(cancel.clone());
    let mut response = client.send(req).await.map_err(super::pods::map_pod_error)?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(map_status(status));
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
        let data = frame
            .map_err(|_| ConnectionError::Network)?
            .into_data()
            .map_err(|_| ConnectionError::InvalidResponse)?;
        if data.len() > BODY_LIMIT.saturating_sub(bytes.len()) {
            return Err(ConnectionError::LimitExceeded);
        }
        bytes.extend_from_slice(&data);
    }
    if bytes.is_empty() {
        return Ok((status, Value::Null));
    }
    if status == 204 {
        return Err(ConnectionError::InvalidResponse);
    }
    Ok((
        status,
        serde_json::from_slice(&bytes).map_err(|_| ConnectionError::InvalidResponse)?,
    ))
}
fn map_status(s: u16) -> ConnectionError {
    match s {
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
