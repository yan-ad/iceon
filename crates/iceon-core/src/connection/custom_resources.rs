use super::{
    ConnectionEngine, ConnectionError, ResourceKind, ResourcePage, ResourceRequest, Result,
    SessionRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

#[cfg(test)]
mod tests;

const BODY_LIMIT: usize = 2 * 1024 * 1024;
const METADATA_ACCEPT: &str = "application/json;as=PartialObjectMetadata;g=meta.k8s.io;v=v1";

pub(super) struct CustomSlot {
    id: u64,
    cancel: Option<CancellationToken>,
    request: ResourceRequest,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CustomDefinitionRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub read_id: u64,
    pub continue_token: Option<String>,
}

impl CustomDefinitionRequest {
    fn slot_request(&self) -> Result<ResourceRequest> {
        let request = ResourceRequest {
            cluster_id: self.cluster_id.clone(),
            session_id: self.session_id.clone(),
            read_id: self.read_id,
            kind: ResourceKind::CustomResourceDefinitions,
            namespace: None,
            continue_token: self.continue_token.clone(),
            query: None,
            status: None,
            group: None,
        };
        super::validate_request(&request.session())?;
        super::validate_read_id(request.read_id)?;
        super::transport::validate_resource_request(&request)?;
        Ok(request)
    }
}

fn cancel_slot(slot: &mut Option<CustomSlot>, request: ResourceRequest) -> Result<()> {
    if let Some(current) = slot {
        if current.id == request.read_id && current.request != request {
            return Err(ConnectionError::InvalidInput);
        }
        if current.id > request.read_id {
            return Ok(());
        }
    }
    if let Some(old) = slot.replace(CustomSlot {
        id: request.read_id,
        cancel: None,
        request,
    }) && let Some(cancel) = old.cancel
    {
        cancel.cancel();
    }
    Ok(())
}

fn start_slot(
    slot: &mut Option<CustomSlot>,
    request: ResourceRequest,
    parent: &CancellationToken,
) -> Result<CancellationToken> {
    if slot.as_ref().is_some_and(|s| s.id >= request.read_id) {
        return Err(ConnectionError::StaleSession);
    }
    let cancel = parent.child_token();
    if let Some(old) = slot.replace(CustomSlot {
        id: request.read_id,
        cancel: Some(cancel.clone()),
        request,
    }) && let Some(old_cancel) = old.cancel
    {
        old_cancel.cancel();
    }
    Ok(cancel)
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CustomResourceTarget {
    pub definition_name: String,
    pub definition_uid: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CustomResourceRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub read_id: u64,
    pub target: CustomResourceTarget,
    pub namespace: Option<String>,
    pub continue_token: Option<String>,
}
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CustomResourcePage {
    pub cluster_id: String,
    pub session_id: String,
    pub read_id: u64,
    pub target: CustomResourceTarget,
    pub group: String,
    pub version: String,
    pub plural: String,
    pub kind: String,
    pub namespaced: bool,
    pub namespace: Option<String>,
    pub items: Vec<CustomResourceItem>,
    pub continue_token: Option<String>,
    pub truncated: bool,
}
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CustomResourceItem {
    pub name: String,
    pub uid: String,
    pub namespace: Option<String>,
    pub created_at: Option<String>,
    pub label_count: usize,
}
struct Definition {
    group: String,
    version: String,
    plural: String,
    kind: String,
    namespaced: bool,
}
fn dns(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 253
        && value.split('.').all(super::transport::valid_namespace)
}
fn uid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}
fn version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && value.starts_with('v')
}
fn token(value: &str) -> bool {
    !value.is_empty() && value.len() <= 4096 && value.bytes().all(|b| (0x21..=0x7e).contains(&b))
}
impl CustomResourceRequest {
    fn session(&self) -> SessionRequest {
        SessionRequest {
            cluster_id: self.cluster_id.clone(),
            session_id: self.session_id.clone(),
        }
    }
    fn slot_request(&self) -> Result<ResourceRequest> {
        super::validate_request(&self.session())?;
        super::validate_read_id(self.read_id)?;
        if !dns(&self.target.definition_name)
            || !uid(&self.target.definition_uid)
            || self
                .namespace
                .as_deref()
                .is_some_and(|n| !super::transport::valid_namespace(n))
            || self.continue_token.as_deref().is_some_and(|t| !token(t))
        {
            return Err(ConnectionError::InvalidInput);
        }
        Ok(ResourceRequest {
            cluster_id: self.cluster_id.clone(),
            session_id: self.session_id.clone(),
            read_id: self.read_id,
            kind: ResourceKind::CustomResourceDefinitions,
            namespace: self.namespace.clone(),
            continue_token: self.continue_token.clone(),
            query: Some(
                serde_json::to_string(&self.target).map_err(|_| ConnectionError::InvalidInput)?,
            ),
            status: None,
            group: None,
        })
    }
}
impl ConnectionEngine {
    pub fn cancel_custom_resource_definitions(
        &self,
        request: CustomDefinitionRequest,
    ) -> Result<()> {
        let request = request.slot_request()?;
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        if let Some(session) = state
            .as_mut()
            .filter(|s| super::matches_session(s, &request.session()))
        {
            cancel_slot(&mut session.custom_discovery, request)?;
        }
        Ok(())
    }

    pub async fn custom_resource_definitions(
        &self,
        request: CustomDefinitionRequest,
    ) -> Result<ResourcePage> {
        let request = request.slot_request()?;
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
            let cancel = start_slot(
                &mut session.custom_discovery,
                request.clone(),
                &session.cancel,
            )?;
            (client, cancel)
        };
        let _guard = CancelOnDrop(cancel.clone());
        let result = super::bounded(
            &cancel,
            super::transport::resources(&client, &request, &cancel),
        )
        .await;
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        let session = super::current_mut(&mut state, &request.session())?;
        let slot = session
            .custom_discovery
            .as_mut()
            .ok_or(ConnectionError::StaleSession)?;
        if slot.id != request.read_id
            || (cancel.is_cancelled() && result != Err(ConnectionError::Timeout))
        {
            return Err(ConnectionError::StaleSession);
        }
        slot.cancel = None;
        result
    }

    pub fn cancel_custom_resources(&self, request: CustomResourceRequest) -> Result<()> {
        let slot_request = request.slot_request()?;
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        if let Some(session) = state
            .as_mut()
            .filter(|s| super::matches_session(s, &request.session()))
        {
            cancel_slot(&mut session.custom_instances, slot_request)?;
        }
        Ok(())
    }
    pub async fn custom_resources(
        &self,
        request: CustomResourceRequest,
    ) -> Result<CustomResourcePage> {
        let slot_request = request.slot_request()?;
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
            let cancel = start_slot(&mut session.custom_instances, slot_request, &session.cancel)?;
            (client, cancel)
        };
        let _guard = CancelOnDrop(cancel.clone());
        let result = super::bounded(&cancel, async {
            let path = format!(
                "/apis/apiextensions.k8s.io/v1/customresourcedefinitions/{}",
                request.target.definition_name
            );
            let value = super::transport::get(&client, &path, 16 * 1024 * 1024, &cancel).await?;
            let definition = definition(&request.target, value)?;
            let path = instance_path(&definition, &request)?;
            let value = super::transport::get_with_accept(
                &client,
                &path,
                BODY_LIMIT,
                &cancel,
                METADATA_ACCEPT,
            )
            .await?;
            project(&request, definition, value)
        })
        .await;
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        let session = super::current_mut(&mut state, &request.session())?;
        let slot = session
            .custom_instances
            .as_mut()
            .ok_or(ConnectionError::StaleSession)?;
        if slot.id != request.read_id
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
fn string<'a>(v: &'a Value, path: &str) -> Result<&'a str> {
    v.pointer(path)
        .and_then(Value::as_str)
        .ok_or(ConnectionError::InvalidResponse)
}
fn definition(target: &CustomResourceTarget, value: Value) -> Result<Definition> {
    if string(&value, "/apiVersion")? != "apiextensions.k8s.io/v1"
        || string(&value, "/kind")? != "CustomResourceDefinition"
        || string(&value, "/metadata/name")? != target.definition_name
    {
        return Err(ConnectionError::InvalidResponse);
    }
    if string(&value, "/metadata/uid")? != target.definition_uid {
        return Err(ConnectionError::SourceChanged);
    }
    let group = string(&value, "/spec/group")?;
    let plural = string(&value, "/spec/names/plural")?;
    let kind = string(&value, "/spec/names/kind")?;
    if !dns(group)
        || !group.contains('.')
        || !super::transport::valid_namespace(plural)
        || target.definition_name != format!("{plural}.{group}")
        || kind.is_empty()
        || kind.len() > 63
        || !kind.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return Err(ConnectionError::InvalidResponse);
    }
    let namespaced = match string(&value, "/spec/scope")? {
        "Namespaced" => true,
        "Cluster" => false,
        _ => return Err(ConnectionError::InvalidResponse),
    };
    if !value
        .pointer("/status/conditions")
        .and_then(Value::as_array)
        .is_some_and(|conditions| {
            conditions
                .iter()
                .any(|c| c["type"] == "Established" && c["status"] == "True")
        })
    {
        return Err(ConnectionError::ResourceUnavailable);
    }
    let versions = value
        .pointer("/spec/versions")
        .and_then(Value::as_array)
        .ok_or(ConnectionError::InvalidResponse)?;
    if versions.is_empty() || versions.len() > 100 {
        return Err(ConnectionError::InvalidResponse);
    }
    let served = |v: &&Value| v.get("served").and_then(Value::as_bool) == Some(true);
    let selected = versions
        .iter()
        .filter(served)
        .find(|v| v["storage"] == true)
        .or_else(|| versions.iter().find(served))
        .ok_or(ConnectionError::ResourceUnavailable)?;
    let selected_version = string(selected, "/name")?;
    if !version(selected_version) {
        return Err(ConnectionError::InvalidResponse);
    }
    Ok(Definition {
        group: group.into(),
        version: selected_version.into(),
        plural: plural.into(),
        kind: kind.into(),
        namespaced,
    })
}
fn instance_path(definition: &Definition, request: &CustomResourceRequest) -> Result<String> {
    if !definition.namespaced && request.namespace.is_some() {
        return Err(ConnectionError::InvalidInput);
    }
    let scope = request
        .namespace
        .as_ref()
        .map(|n| format!("/namespaces/{n}"))
        .unwrap_or_default();
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.append_pair("limit", "100");
    if let Some(token) = &request.continue_token {
        query.append_pair("continue", token);
    }
    Ok(format!(
        "/apis/{}/{}{scope}/{}?{}",
        definition.group,
        definition.version,
        definition.plural,
        query.finish()
    ))
}
fn project(
    request: &CustomResourceRequest,
    definition: Definition,
    value: Value,
) -> Result<CustomResourcePage> {
    if string(&value, "/apiVersion")? != "meta.k8s.io/v1"
        || string(&value, "/kind")? != "PartialObjectMetadataList"
    {
        return Err(ConnectionError::InvalidResponse);
    }
    let rows = value
        .get("items")
        .and_then(Value::as_array)
        .ok_or(ConnectionError::InvalidResponse)?;
    if rows.len() > 100 {
        return Err(ConnectionError::LimitExceeded);
    }
    let mut items = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for row in rows {
        let name = string(row, "/metadata/name")?;
        let identity = string(row, "/metadata/uid")?;
        let namespace = row.pointer("/metadata/namespace").and_then(Value::as_str);
        if !dns(name)
            || !uid(identity)
            || if definition.namespaced {
                !namespace.is_some_and(super::transport::valid_namespace)
                    || request
                        .namespace
                        .as_deref()
                        .is_some_and(|n| namespace != Some(n))
            } else {
                namespace.is_some()
            }
        {
            return Err(ConnectionError::InvalidResponse);
        }
        if !seen.insert((namespace, name)) {
            return Err(ConnectionError::InvalidResponse);
        }
        let created_at = row
            .pointer("/metadata/creationTimestamp")
            .and_then(Value::as_str)
            .filter(|s| s.len() <= 64 && !s.chars().any(char::is_control))
            .map(str::to_owned);
        items.push(CustomResourceItem {
            name: name.into(),
            uid: identity.into(),
            namespace: namespace.map(str::to_owned),
            created_at,
            label_count: row
                .pointer("/metadata/labels")
                .and_then(Value::as_object)
                .map_or(0, |labels| labels.len()),
        });
    }
    let continuation = value
        .pointer("/metadata/continue")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    if continuation.as_deref().is_some_and(|s| !token(s))
        || continuation.is_some() && continuation == request.continue_token
    {
        return Err(ConnectionError::InvalidResponse);
    }
    Ok(CustomResourcePage {
        cluster_id: request.cluster_id.clone(),
        session_id: request.session_id.clone(),
        read_id: request.read_id,
        target: request.target.clone(),
        group: definition.group,
        version: definition.version,
        plural: definition.plural,
        kind: definition.kind,
        namespaced: definition.namespaced,
        namespace: request.namespace.clone(),
        items,
        truncated: continuation.is_some(),
        continue_token: continuation,
    })
}
