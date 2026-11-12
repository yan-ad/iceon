use super::{
    ConnectionEngine, ConnectionError, ConnectionTarget, ResourceKind, ResourceRequest, Result,
    SessionRequest, current, current_mut, transport, validate_credentials, validate_read_id,
    validate_request,
};
use http::Request;
use http_body_util::BodyExt;
use kube::{Client, client::Body};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    sync::{Arc, atomic::Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio_util::sync::CancellationToken;

const TTL: u64 = 24 * 60 * 60 * 1000;
const LIMIT: usize = 1000;
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 63
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SourceIdentity(String);
#[derive(Default)]
pub(crate) struct LeaseCache {
    identity: Option<SourceIdentity>,
    authentication_generation: u64,
    principal: Option<String>,
}
impl LeaseCache {
    pub(crate) fn bind(&mut self, identity: SourceIdentity, authentication_generation: u64) {
        self.identity = Some(identity);
        self.authentication_generation = authentication_generation;
        self.principal = None;
    }
}
pub(crate) async fn source_identity_async(target: ConnectionTarget) -> Result<SourceIdentity> {
    crate::runtime::spawn_blocking(move || {
        let prepared = super::config::prepare(&target)?;
        let source = crate::catalog::bounded_read(&target.source_path, 1024 * 1024)
            .map_err(super::map_catalog_error)?;
        let mut hash = Sha256::new();
        for part in [
            target.source_path.to_string_lossy().as_bytes(),
            target.context_name.as_bytes(),
            &source,
            &serde_json::to_vec(&prepared).map_err(|_| ConnectionError::KubeconfigInvalid)?,
        ] {
            hash.update((part.len() as u64).to_le_bytes());
            hash.update(part);
        }
        Ok(SourceIdentity(format!("{:x}", hash.finalize())))
    })
    .await
    .map_err(|_| ConnectionError::StoreUnavailable)?
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NamespaceSnapshot {
    pub cluster_id: String,
    pub session_id: String,
    pub names: Vec<String>,
    pub fetched_at: u64,
    pub expires_at: u64,
    pub partial: bool,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NamespaceRefreshRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub read_id: u64,
}
impl NamespaceRefreshRequest {
    fn scope(&self) -> SessionRequest {
        SessionRequest {
            cluster_id: self.cluster_id.clone(),
            session_id: self.session_id.clone(),
        }
    }
}
async fn principal(client: &Client, cancel: &CancellationToken) -> Result<String> {
    let work = async {
        let mut request = Request::post("/apis/authentication.k8s.io/v1/selfsubjectreviews")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(
                    &json!({"apiVersion":"authentication.k8s.io/v1","kind":"SelfSubjectReview"}),
                )
                .map_err(|_| ConnectionError::InvalidResponse)?,
            ))
            .map_err(|_| ConnectionError::InvalidInput)?;
        request.extensions_mut().insert(cancel.clone());
        let response = client
            .send(request)
            .await
            .map_err(|_| ConnectionError::Network)?;
        if !response.status().is_success() {
            return Err(ConnectionError::PermissionReviewUnavailable);
        }
        let mut body = response.into_body();
        let mut bytes = Vec::new();
        while let Some(frame) = body.frame().await {
            let frame = frame.map_err(|_| ConnectionError::InvalidResponse)?;
            if let Some(data) = frame.data_ref() {
                if bytes.len() + data.len() > 64 * 1024 {
                    return Err(ConnectionError::LimitExceeded);
                }
                bytes.extend_from_slice(data);
            }
        }
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| ConnectionError::InvalidResponse)?;
        if value["apiVersion"] != "authentication.k8s.io/v1"
            || value["kind"] != "SelfSubjectReview"
            || value["status"]["userInfo"]["username"]
                .as_str()
                .is_none_or(|name| name.is_empty())
        {
            return Err(ConnectionError::InvalidResponse);
        }
        Ok(format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&value["status"]["userInfo"])
                    .map_err(|_| ConnectionError::InvalidResponse)?
            )
        ))
    };
    tokio::select! { biased; _ = cancel.cancelled() => Err(ConnectionError::StaleSession), result = tokio::time::timeout(Duration::from_secs(3), work) => result.map_err(|_| ConnectionError::Timeout)? }
}
fn validate_snapshot(snapshot: &NamespaceSnapshot) -> bool {
    snapshot.names.len() <= LIMIT
        && snapshot.names.iter().all(|name| valid_name(name))
        && snapshot.names.iter().collect::<BTreeSet<_>>().len() == snapshot.names.len()
        && snapshot.fetched_at > 0
        && snapshot.fetched_at <= now()
        && snapshot.expires_at > now()
        && snapshot.expires_at > snapshot.fetched_at
        && snapshot.expires_at - snapshot.fetched_at <= TTL
}
impl ConnectionEngine {
    async fn namespace_identity(&self, scope: &SessionRequest) -> Result<Option<(String, u64)>> {
        validate_request(scope)?;
        let (client, source, target, epoch, cancel, cached) = {
            let state = self
                .session
                .lock()
                .map_err(|_| ConnectionError::StoreUnavailable)?;
            let session = current(&state, scope)?;
            let client = session
                .client
                .clone()
                .ok_or(ConnectionError::NotConnected)?;
            validate_credentials(&client)?;
            let epoch = self.authentication_generation.load(Ordering::SeqCst);
            if session.namespace_cache.authentication_generation != epoch {
                return Ok(None);
            }
            let Some(source) = session.namespace_cache.identity.clone() else {
                return Ok(None);
            };
            (
                client,
                source,
                session.target.clone(),
                epoch,
                session.cancel.child_token(),
                session.namespace_cache.principal.clone(),
            )
        };
        if source_identity_async(target).await? != source {
            return Ok(None);
        }
        let user = match cached {
            Some(user) => user,
            None => match principal(&client, &cancel).await {
                Ok(user) => user,
                Err(ConnectionError::StaleSession) => return Err(ConnectionError::StaleSession),
                Err(_) => return Ok(None),
            },
        };
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        let session = current_mut(&mut state, scope)?;
        if self.authentication_generation.load(Ordering::SeqCst) != epoch || cancel.is_cancelled() {
            return Err(ConnectionError::StaleSession);
        }
        session.namespace_cache.principal = Some(user.clone());
        Ok(Some((
            format!(
                "{:x}",
                Sha256::digest(format!("{}:{}:{}", scope.cluster_id, source.0, user))
            ),
            epoch,
        )))
    }
    pub async fn namespace_cache_get(
        &self,
        scope: SessionRequest,
    ) -> Result<Option<NamespaceSnapshot>> {
        let _worker = self
            .workers
            .try_acquire()
            .map_err(|_| ConnectionError::Busy)?;
        self.reject_known_denied_read(
            &scope,
            &super::permissions::PermissionCheck {
                kind: ResourceKind::Namespaces,
                action: super::permissions::PermissionAction::List,
                namespace: None,
                name: None,
            },
        )?;
        let Some((key, epoch)) = self.namespace_identity(&scope).await? else {
            return Ok(None);
        };
        let catalog = self.catalog.clone();
        let identity = scope.clone();
        let snapshot = crate::runtime::spawn_blocking(move || catalog.namespace_cache_storage(|connection| {
            let raw = connection.query_row("SELECT names,fetched_at,expires_at,partial FROM namespace_snapshots WHERE identity=?1", [&key], |row| Ok((row.get::<_,String>(0)?, row.get::<_,i64>(1)?, row.get::<_,i64>(2)?, row.get::<_,bool>(3)?))).optional().map_err(|_| crate::catalog::CatalogError::StoreCorrupt)?;
            let Some((names,fetched_at,expires_at,partial)) = raw else { return Ok(None); };
            let snapshot = NamespaceSnapshot { cluster_id: identity.cluster_id, session_id: identity.session_id, names: serde_json::from_str(&names).map_err(|_| crate::catalog::CatalogError::StoreCorrupt)?, fetched_at: u64::try_from(fetched_at).unwrap_or_default(), expires_at: u64::try_from(expires_at).unwrap_or_default(), partial };
            if !validate_snapshot(&snapshot) { connection.execute("DELETE FROM namespace_snapshots WHERE identity=?1", [&key]).map_err(|_| crate::catalog::CatalogError::StoreUnavailable)?; return Ok(None); }
            Ok(Some(snapshot))
        })).await.map_err(|_| ConnectionError::StoreUnavailable)?.map_err(super::map_catalog_error)?;
        let state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        current(&state, &scope)?;
        if self.authentication_generation.load(Ordering::SeqCst) != epoch {
            return Err(ConnectionError::StaleSession);
        }
        Ok(snapshot)
    }
    /// Independent metadata-only discovery: does not occupy inventory table read slots.
    pub async fn discover_namespaces(&self, scope: SessionRequest) -> Result<Vec<String>> {
        validate_request(&scope)?;
        self.reject_known_denied_read(
            &scope,
            &super::permissions::PermissionCheck {
                kind: ResourceKind::Namespaces,
                action: super::permissions::PermissionAction::List,
                namespace: None,
                name: None,
            },
        )?;
        let _worker = self
            .workers
            .try_acquire()
            .map_err(|_| ConnectionError::Busy)?;
        let (client, cancel, epoch) = {
            let state = self
                .session
                .lock()
                .map_err(|_| ConnectionError::StoreUnavailable)?;
            let session = current(&state, &scope)?;
            let client = session
                .client
                .clone()
                .ok_or(ConnectionError::NotConnected)?;
            validate_credentials(&client)?;
            (
                client,
                session.cancel.child_token(),
                self.authentication_generation.load(Ordering::SeqCst),
            )
        };
        let result = super::bounded(&cancel, async {
            let mut names = BTreeSet::new();
            let mut tokens = BTreeSet::new();
            let mut request = ResourceRequest {
                cluster_id: scope.cluster_id.clone(),
                session_id: scope.session_id.clone(),
                kind: ResourceKind::Namespaces,
                read_id: 1,
                namespace: None,
                continue_token: None,
                query: None,
                status: None,
                group: None,
            };
            for _ in 0..100 {
                let page = transport::resources(&client, &request, &cancel).await?;
                for item in page.items {
                    if !valid_name(&item.name) {
                        return Err(ConnectionError::InvalidResponse);
                    }
                    names.insert(item.name);
                }
                if names.len() > 10000 {
                    return Err(ConnectionError::LimitExceeded);
                }
                request.continue_token = page.continue_token;
                if request.continue_token.is_none() {
                    if page.truncated {
                        return Err(ConnectionError::LimitExceeded);
                    }
                    return Ok(names.into_iter().collect());
                }
                if !tokens.insert(request.continue_token.clone()) {
                    return Err(ConnectionError::InvalidResponse);
                }
            }
            Err(ConnectionError::LimitExceeded)
        })
        .await;
        cancel.cancel();
        let state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        current(&state, &scope)?;
        if self.authentication_generation.load(Ordering::SeqCst) != epoch {
            return Err(ConnectionError::StaleSession);
        }
        result
    }

    pub async fn namespace_cache_refresh(
        self: &Arc<Self>,
        request: NamespaceRefreshRequest,
    ) -> Result<NamespaceSnapshot> {
        let scope = request.scope();
        validate_request(&scope)?;
        validate_read_id(request.read_id)?;
        self.reject_known_denied_read(
            &scope,
            &super::permissions::PermissionCheck {
                kind: ResourceKind::Namespaces,
                action: super::permissions::PermissionAction::List,
                namespace: None,
                name: None,
            },
        )?;
        let key = self.namespace_identity(&scope).await?;
        let _worker = self
            .workers
            .try_acquire()
            .map_err(|_| ConnectionError::Busy)?;
        let resource = ResourceRequest {
            cluster_id: scope.cluster_id.clone(),
            session_id: scope.session_id.clone(),
            kind: ResourceKind::Namespaces,
            read_id: request.read_id,
            namespace: None,
            continue_token: None,
            query: None,
            status: None,
            group: None,
        };
        let (client, cancel, epoch) = {
            let mut state = self
                .session
                .lock()
                .map_err(|_| ConnectionError::StoreUnavailable)?;
            let session = current_mut(&mut state, &scope)?;
            let client = session
                .client
                .clone()
                .ok_or(ConnectionError::NotConnected)?;
            validate_credentials(&client)?;
            let slot = session
                .reads
                .entry(ResourceKind::Namespaces)
                .or_insert(super::ReadSlot {
                    id: 0,
                    cancel: None,
                    request: resource.clone(),
                });
            if request.read_id <= slot.id {
                return Err(ConnectionError::StaleSession);
            }
            let cancel = session.cancel.child_token();
            if let Some(previous) = slot.cancel.replace(cancel.clone()) {
                previous.cancel();
            }
            slot.id = request.read_id;
            slot.request = resource.clone();
            (
                client,
                cancel,
                self.authentication_generation.load(Ordering::SeqCst),
            )
        };
        let result = super::bounded(&cancel, async {
            let mut names = BTreeSet::new();
            let mut tokens = BTreeSet::new();
            let mut resource = resource;
            for _ in 0..10 {
                let page = transport::resources(&client, &resource, &cancel).await?;
                for item in page.items {
                    if !valid_name(&item.name) || !names.insert(item.name) {
                        return Err(ConnectionError::InvalidResponse);
                    }
                }
                if names.len() > LIMIT {
                    return Err(ConnectionError::LimitExceeded);
                }
                resource.continue_token = page.continue_token;
                if resource.continue_token.is_none() {
                    return Ok((names.into_iter().collect(), page.truncated));
                }
                if !tokens.insert(resource.continue_token.clone()) {
                    return Err(ConnectionError::InvalidResponse);
                }
            }
            Ok((names.into_iter().collect(), true))
        })
        .await;
        let snapshot = {
            let mut state = self
                .session
                .lock()
                .map_err(|_| ConnectionError::StoreUnavailable)?;
            let session = current_mut(&mut state, &scope)?;
            let slot = session
                .reads
                .get_mut(&ResourceKind::Namespaces)
                .ok_or(ConnectionError::StaleSession)?;
            if slot.id != request.read_id
                || self.authentication_generation.load(Ordering::SeqCst) != epoch
                || cancel.is_cancelled()
            {
                return Err(ConnectionError::StaleSession);
            }
            slot.cancel = None;
            let (names, partial) = result?;
            let fetched_at = now();
            NamespaceSnapshot {
                cluster_id: scope.cluster_id.clone(),
                session_id: scope.session_id.clone(),
                names,
                partial,
                fetched_at,
                expires_at: fetched_at + TTL,
            }
        };
        if let Some((key, captured)) = key.filter(|(_, captured)| *captured == epoch) {
            let engine = self.clone();
            let saved = snapshot.clone();
            let guard_scope = scope.clone();
            let _ = crate::runtime::spawn_blocking(move || -> Result<()> {
                let state = engine.session.lock().map_err(|_| ConnectionError::StoreUnavailable)?;
                let session = current(&state, &guard_scope)?;
                if engine.authentication_generation.load(Ordering::SeqCst) != captured || cancel.is_cancelled()
                    || session.reads.get(&ResourceKind::Namespaces).is_none_or(|slot| slot.id != request.read_id) { return Err(ConnectionError::StaleSession); }
                engine.catalog.namespace_cache_storage(|connection| {
                    let transaction = connection.transaction().map_err(|_| crate::catalog::CatalogError::StoreUnavailable)?;
                    transaction.execute("DELETE FROM namespace_snapshots WHERE expires_at<=?1 OR fetched_at>?1", [now() as i64]).map_err(|_| crate::catalog::CatalogError::StoreUnavailable)?;
                    transaction.execute("INSERT INTO namespace_snapshots(identity,names,fetched_at,expires_at,partial) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(identity) DO UPDATE SET names=excluded.names,fetched_at=excluded.fetched_at,expires_at=excluded.expires_at,partial=excluded.partial", params![key, serde_json::to_string(&saved.names).map_err(|_| crate::catalog::CatalogError::StoreCorrupt)?, saved.fetched_at as i64, saved.expires_at as i64, saved.partial]).map_err(|_| crate::catalog::CatalogError::StoreUnavailable)?;
                    transaction.execute("DELETE FROM namespace_snapshots WHERE identity IN (SELECT identity FROM namespace_snapshots ORDER BY fetched_at DESC,identity DESC LIMIT -1 OFFSET 64)", []).map_err(|_| crate::catalog::CatalogError::StoreUnavailable)?;
                    transaction.commit().map_err(|_| crate::catalog::CatalogError::StoreUnavailable)
                }).map_err(super::map_catalog_error)
            }).await;
        }
        let state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        current(&state, &scope)?;
        if self.authentication_generation.load(Ordering::SeqCst) != epoch {
            return Err(ConnectionError::StaleSession);
        }
        Ok(snapshot)
    }
}
#[cfg(test)]
mod tests;
