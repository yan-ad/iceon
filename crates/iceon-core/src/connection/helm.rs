use super::{
    ConnectionEngine, ConnectionError, Result, SessionRequest, auth, config, current,
    validate_credentials, validate_read_id, validate_request,
};
use process_wrap::tokio::{CommandWrap, KillOnDrop};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap, ffi::OsString, io::Write, path::PathBuf, process::Stdio, sync::Mutex,
};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio_util::sync::CancellationToken;
use url::Url;

const OUTPUT_LIMIT: usize = 2 * 1024 * 1024;
const ITEM_LIMIT: usize = 500;

/// Explicit Helm process preferences.  This is intentionally independent from
/// the persistence schema so callers can pass a snapshot without making the
/// command runner reach into global state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HelmPreferences {
    pub binary_path: Option<PathBuf>,
    pub server_side: bool,
    pub proxy: Option<String>,
    pub allow_untrusted_cas: bool,
}

impl From<&crate::preferences::Preferences> for HelmPreferences {
    fn from(preferences: &crate::preferences::Preferences) -> Self {
        Self {
            binary_path: (!preferences.kubernetes.helm_binary_path.is_empty())
                .then(|| preferences.kubernetes.helm_binary_path.clone().into()),
            server_side: preferences.kubernetes.helm_server_side,
            proxy: (!preferences.proxy.https_proxy.is_empty())
                .then(|| preferences.proxy.https_proxy.clone()),
            allow_untrusted_cas: preferences.proxy.allow_untrusted_cas,
        }
    }
}

pub use crate::preferences::HelmRepository;

fn validate_repository(repo: &HelmRepository) -> Result<Url> {
    if repo.name.len() > 128 || !safe_component(&repo.name, 128, b"_-.") {
        return Err(ConnectionError::InvalidInput);
    }
    let url = Url::parse(&repo.url).map_err(|_| ConnectionError::InvalidInput)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || repo.url.len() > 2048
        || repo.url.bytes().any(|b| b.is_ascii_control())
    {
        return Err(ConnectionError::InvalidInput);
    }
    Ok(url)
}

fn validate_proxy(proxy: Option<&str>) -> Result<()> {
    if let Some(value) = proxy {
        let url = Url::parse(value).map_err(|_| ConnectionError::InvalidInput)?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || value.len() > 2048
            || value.chars().any(char::is_control)
            || url.fragment().is_some()
        {
            return Err(ConnectionError::InvalidInput);
        }
    }
    Ok(())
}

#[path = "helm_install.rs"]
pub mod install;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum HelmView {
    Charts,
    Releases,
}

impl Drop for HelmRuntime {
    fn drop(&mut self) {
        for storage in [&mut self.slots, &mut self.details] {
            if let Ok(slots) = storage.get_mut() {
                for slot in slots.values() {
                    if let Some(cancel) = &slot.cancel {
                        cancel.cancel();
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelmCancelRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub view: HelmView,
    pub read_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct HelmScope {
    cluster_id: String,
    session_id: String,
    view: HelmView,
}

#[derive(Debug)]
struct HelmSlot {
    read_id: u64,
    cancel: Option<CancellationToken>,
    root: CancellationToken,
}

#[derive(Debug, Default)]
pub struct HelmRuntime {
    install: install::InstallRuntime,
    slots: Mutex<HashMap<HelmScope, HelmSlot>>,
    details: Mutex<HashMap<HelmScope, HelmSlot>>,
}

impl HelmRuntime {
    const CAP: usize = 4;

    #[cfg(test)]
    fn token(&self, request: &HelmRequest, root: &CancellationToken) -> Result<CancellationToken> {
        Self::token_in(&self.slots, request, root)
    }

    fn token_in(
        storage: &Mutex<HashMap<HelmScope, HelmSlot>>,
        request: &HelmRequest,
        root: &CancellationToken,
    ) -> Result<CancellationToken> {
        validate_read_id(request.read_id)?;
        let scope = HelmScope {
            cluster_id: request.cluster_id.clone(),
            session_id: request.session_id.clone(),
            view: request.view,
        };
        let mut slots = storage
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        slots.retain(|_, slot| !slot.root.is_cancelled());
        if root.is_cancelled()
            || slots
                .get(&scope)
                .is_some_and(|slot| request.read_id <= slot.read_id)
        {
            return Err(ConnectionError::StaleSession);
        }
        if !slots.contains_key(&scope) && slots.len() >= Self::CAP {
            return Err(ConnectionError::Busy);
        }
        let token = root.child_token();
        if let Some(old) = slots.insert(
            scope,
            HelmSlot {
                read_id: request.read_id,
                cancel: Some(token.clone()),
                root: root.clone(),
            },
        ) && let Some(cancel) = old.cancel
        {
            cancel.cancel();
        }
        Ok(token)
    }

    #[cfg(test)]
    fn cancel(&self, request: &HelmCancelRequest, root: &CancellationToken) -> Result<()> {
        Self::cancel_in(&self.slots, request, root)
    }

    fn cancel_in(
        storage: &Mutex<HashMap<HelmScope, HelmSlot>>,
        request: &HelmCancelRequest,
        root: &CancellationToken,
    ) -> Result<()> {
        validate_request(&SessionRequest {
            cluster_id: request.cluster_id.clone(),
            session_id: request.session_id.clone(),
        })?;
        validate_read_id(request.read_id)?;
        let scope = HelmScope {
            cluster_id: request.cluster_id.clone(),
            session_id: request.session_id.clone(),
            view: request.view,
        };
        let mut slots = storage
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        slots.retain(|_, slot| !slot.root.is_cancelled());
        if root.is_cancelled() {
            return Ok(());
        }
        if let Some(slot) = slots.get_mut(&scope) {
            if request.read_id >= slot.read_id {
                if let Some(cancel) = slot.cancel.take() {
                    cancel.cancel();
                }
                slot.read_id = request.read_id;
            }
        } else {
            if slots.len() >= Self::CAP {
                return Err(ConnectionError::Busy);
            }
            slots.insert(
                scope,
                HelmSlot {
                    read_id: request.read_id,
                    cancel: None,
                    root: root.clone(),
                },
            );
        }
        Ok(())
    }

    #[cfg(test)]
    fn finish(&self, request: &HelmRequest) {
        Self::finish_in(&self.slots, request)
    }

    fn finish_in(storage: &Mutex<HashMap<HelmScope, HelmSlot>>, request: &HelmRequest) {
        let scope = HelmScope {
            cluster_id: request.cluster_id.clone(),
            session_id: request.session_id.clone(),
            view: request.view,
        };
        if let Ok(mut slots) = storage.lock()
            && let Some(slot) = slots.get_mut(&scope)
            && slot.read_id == request.read_id
            && let Some(cancel) = slot.cancel.take()
        {
            cancel.cancel();
        }
    }
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelmRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub view: HelmView,
    pub namespace: Option<String>,
    pub read_id: u64,
}
impl HelmRequest {
    fn session(&self) -> SessionRequest {
        SessionRequest {
            cluster_id: self.cluster_id.clone(),
            session_id: self.session_id.clone(),
        }
    }
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HelmPage {
    pub cluster_id: String,
    pub session_id: String,
    pub view: HelmView,
    pub items: Vec<HelmItem>,
    pub truncated: bool,
    pub read_id: u64,
    pub namespace: Option<String>,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HelmItem {
    pub id: String,
    pub name: String,
    pub namespace: Option<String>,
    pub version: String,
    pub app_version: String,
    pub description: Option<String>,
    pub status: Option<String>,
    pub revision: Option<u32>,
    pub updated: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelmDetailRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub read_id: u64,
    pub view: HelmView,
    pub name: String,
    pub namespace: Option<String>,
    pub version: Option<String>,
}
impl HelmDetailRequest {
    fn inventory(&self) -> HelmRequest {
        HelmRequest {
            cluster_id: self.cluster_id.clone(),
            session_id: self.session_id.clone(),
            read_id: self.read_id,
            view: self.view,
            namespace: self.namespace.clone(),
        }
    }
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HelmDetail {
    pub cluster_id: String,
    pub session_id: String,
    pub read_id: u64,
    pub view: HelmView,
    pub name: String,
    pub namespace: Option<String>,
    pub version: Option<String>,
    pub facts: Vec<HelmFact>,
    pub history: Vec<HelmHistory>,
    pub truncated: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct HelmFact {
    pub label: String,
    pub value: String,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HelmHistory {
    pub revision: u32,
    pub updated: String,
    pub status: String,
    pub chart: String,
    pub app_version: String,
}

fn safe_component(value: &str, max: usize, punctuation: &[u8]) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || punctuation.contains(&c))
}
fn validate_detail(request: &HelmDetailRequest) -> Result<()> {
    validate_request(&request.inventory().session())?;
    validate_read_id(request.read_id)?;
    validate_namespace(&request.inventory())?;
    let valid = match request.view {
        HelmView::Charts => {
            let parts: Vec<_> = request.name.split('/').collect();
            parts.len() == 2
                && request.name.len() <= 253
                && safe_component(parts[0], 128, b"_-")
                && safe_component(parts[1], 128, b"_.-")
                && request
                    .version
                    .as_ref()
                    .is_some_and(|v| safe_component(v, 128, b".+_-"))
        }
        HelmView::Releases => {
            request.namespace.is_some()
                && request.version.is_none()
                && request.name.len() <= 53
                && !request.name.ends_with('-')
                && safe_component(&request.name, 53, b"-")
                && request.name.bytes().all(|c| !c.is_ascii_uppercase())
        }
    };
    if valid {
        Ok(())
    } else {
        Err(ConnectionError::InvalidInput)
    }
}
fn validate_cached_chart(request: &HelmDetailRequest, bytes: &[u8]) -> Result<()> {
    let page = project(&request.inventory(), bytes)?;
    if page
        .items
        .iter()
        .any(|item| item.name == request.name && Some(&item.version) == request.version.as_ref())
    {
        Ok(())
    } else {
        Err(ConnectionError::Conflict)
    }
}
fn detail_arguments(request: &HelmDetailRequest) -> Vec<OsString> {
    match request.view {
        HelmView::Charts => vec![
            "show".into(),
            "chart".into(),
            request.name.clone().into(),
            "--version".into(),
            request.version.clone().unwrap_or_default().into(),
        ],
        HelmView::Releases => vec![
            "history".into(),
            request.name.clone().into(),
            "--namespace".into(),
            request.namespace.clone().unwrap_or_default().into(),
            "--max".into(),
            "100".into(),
            "--output".into(),
            "json".into(),
        ],
    }
}
fn project_detail(request: &HelmDetailRequest, bytes: &[u8]) -> Result<HelmDetail> {
    if bytes.len() > OUTPUT_LIMIT {
        return Err(ConnectionError::LimitExceeded);
    }
    let mut result = HelmDetail {
        cluster_id: request.cluster_id.clone(),
        session_id: request.session_id.clone(),
        read_id: request.read_id,
        view: request.view,
        name: request.name.clone(),
        namespace: request.namespace.clone(),
        version: request.version.clone(),
        facts: vec![],
        history: vec![],
        truncated: false,
    };
    let mut budget: usize = 64 * 1024;
    match request.view {
        HelmView::Charts => {
            let value: Value =
                serde_yaml_ng::from_slice(bytes).map_err(|_| ConnectionError::InvalidResponse)?;
            let expected_name = request
                .name
                .split('/')
                .nth(1)
                .ok_or(ConnectionError::InvalidInput)?;
            if value.get("name").and_then(Value::as_str) != Some(expected_name)
                || value.get("version").and_then(Value::as_str) != request.version.as_deref()
            {
                return Err(ConnectionError::InvalidResponse);
            }
            for (key, label) in [
                ("name", "Name"),
                ("version", "Version"),
                ("appVersion", "App version"),
                ("description", "Description"),
                ("type", "Type"),
                ("apiVersion", "Chart API"),
                ("home", "Home"),
                ("deprecated", "Deprecated"),
            ] {
                if let Some(value) = value.get(key) {
                    let text = match value {
                        Value::String(s) => s.clone(),
                        Value::Bool(v) => v.to_string(),
                        _ => return Err(ConnectionError::InvalidResponse),
                    };
                    push_fact(&mut result, label, &text, &mut budget);
                }
            }
            for (key, label) in [("sources", "Source"), ("keywords", "Keyword")] {
                if let Some(values) = value.get(key) {
                    let values = values.as_array().ok_or(ConnectionError::InvalidResponse)?;
                    result.truncated |= values.len() > 50;
                    for value in values.iter().take(50) {
                        push_fact(
                            &mut result,
                            label,
                            value.as_str().ok_or(ConnectionError::InvalidResponse)?,
                            &mut budget,
                        );
                    }
                }
            }
            if let Some(values) = value.get("maintainers") {
                let values = values.as_array().ok_or(ConnectionError::InvalidResponse)?;
                result.truncated |= values.len() > 50;
                for value in values.iter().take(50) {
                    if let Some(name) = value.get("name").and_then(Value::as_str) {
                        push_fact(&mut result, "Maintainer", name, &mut budget);
                    }
                }
            }
        }
        HelmView::Releases => {
            let value: Value =
                serde_json::from_slice(bytes).map_err(|_| ConnectionError::InvalidResponse)?;
            let rows = value.as_array().ok_or(ConnectionError::InvalidResponse)?;
            result.truncated = rows.len() >= 100;
            let mut revisions = std::collections::HashSet::new();
            for row in rows.iter().take(100) {
                let revision = row
                    .get("revision")
                    .and_then(Value::as_u64)
                    .and_then(|v| u32::try_from(v).ok())
                    .filter(|v| *v > 0)
                    .ok_or(ConnectionError::InvalidResponse)?;
                if !revisions.insert(revision) {
                    return Err(ConnectionError::InvalidResponse);
                }
                // Description, notes, values, manifests and arbitrary fields never cross IPC.
                let item = HelmHistory {
                    revision,
                    updated: field(row, "updated", false)?,
                    status: field(row, "status", true)?,
                    chart: field(row, "chart", true)?,
                    app_version: field(row, "app_version", false)?,
                };
                let size = item.updated.len()
                    + item.status.len()
                    + item.chart.len()
                    + item.app_version.len();
                if size > budget {
                    result.truncated = true;
                    break;
                }
                budget -= size;
                result.history.push(item);
            }
            result
                .history
                .sort_by_key(|item| std::cmp::Reverse(item.revision));
        }
    }
    Ok(result)
}
fn push_fact(result: &mut HelmDetail, label: &str, text: &str, budget: &mut usize) {
    let clean: String = text
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect();
    let mut end = clean.len().min(4096).min(*budget);
    while !clean.is_char_boundary(end) {
        end -= 1;
    }
    result.truncated |= end < clean.len();
    if end > 0 {
        *budget -= end;
        result.facts.push(HelmFact {
            label: label.into(),
            value: clean[..end].into(),
        });
    }
}

impl ConnectionEngine {
    /// Executes against Helm's real repository configuration/cache. A successful
    /// add means Helm fetched and validated the remote index, not just saved UI metadata.
    pub async fn helm_repository_add(
        &self,
        repository: HelmRepository,
        preferences: HelmPreferences,
        cancel: CancellationToken,
    ) -> Result<()> {
        validate_repository(&repository)?;
        let mut args = vec![
            "repo".into(),
            "add".into(),
            repository.name.into(),
            repository.url.into(),
        ];
        if preferences.allow_untrusted_cas {
            args.push("--insecure-skip-tls-verify".into());
        }
        self.helm_repository_command(args, preferences, cancel)
            .await?;
        Ok(())
    }

    pub async fn helm_repository_remove(
        &self,
        name: String,
        preferences: HelmPreferences,
        cancel: CancellationToken,
    ) -> Result<()> {
        if !safe_component(&name, 128, b"_-.") {
            return Err(ConnectionError::InvalidInput);
        }
        self.helm_repository_command(
            vec!["repo".into(), "remove".into(), name.into()],
            preferences,
            cancel,
        )
        .await?;
        Ok(())
    }

    pub async fn helm_repository_list(
        &self,
        preferences: HelmPreferences,
        cancel: CancellationToken,
    ) -> Result<Vec<HelmRepository>> {
        let output = self
            .helm_repository_command(
                vec![
                    "repo".into(),
                    "list".into(),
                    "--output".into(),
                    "json".into(),
                ],
                preferences,
                cancel,
            )
            .await?;
        let repositories: Vec<HelmRepository> =
            serde_json::from_slice(&output).map_err(|_| ConnectionError::InvalidResponse)?;
        if repositories.len() > ITEM_LIMIT {
            return Err(ConnectionError::LimitExceeded);
        }
        let mut names = std::collections::HashSet::new();
        for repository in &repositories {
            validate_repository(repository).map_err(|_| ConnectionError::InvalidResponse)?;
            if !names.insert(&repository.name) {
                return Err(ConnectionError::InvalidResponse);
            }
        }
        Ok(repositories)
    }

    async fn helm_repository_command(
        &self,
        args: Vec<OsString>,
        preferences: HelmPreferences,
        cancel: CancellationToken,
    ) -> Result<Vec<u8>> {
        let _permit = self
            .workers
            .try_acquire()
            .map_err(|_| ConnectionError::Busy)?;
        let cancel = cancel.child_token();
        let _guard = CancelOnDrop(cancel.clone());
        let executable = executable_with_preferences(&preferences)?;
        let config =
            tempfile::NamedTempFile::new().map_err(|_| ConnectionError::KubeconfigUnavailable)?;
        self.auth_tasks
            .spawn(supervise_preferences(
                executable,
                args,
                config,
                cancel,
                preferences,
            ))
            .await
            .map_err(|_| ConnectionError::ConnectionFailed)?
    }

    pub async fn helm_table(&self, request: HelmRequest) -> Result<HelmPage> {
        self.helm_table_with_preferences(request, HelmPreferences::default())
            .await
    }

    pub async fn helm_table_with_preferences(
        &self,
        request: HelmRequest,
        preferences: HelmPreferences,
    ) -> Result<HelmPage> {
        let output = self.helm_output(&request, None, preferences).await?;
        project(&request, &output)
    }

    pub async fn helm_detail(&self, request: HelmDetailRequest) -> Result<HelmDetail> {
        self.helm_detail_with_preferences(request, HelmPreferences::default())
            .await
    }

    pub async fn helm_detail_with_preferences(
        &self,
        request: HelmDetailRequest,
        preferences: HelmPreferences,
    ) -> Result<HelmDetail> {
        validate_detail(&request)?;
        let output = self
            .helm_output(&request.inventory(), Some(&request), preferences)
            .await?;
        project_detail(&request, &output)
    }

    async fn helm_output(
        &self,
        request: &HelmRequest,
        detail: Option<&HelmDetailRequest>,
        preferences: HelmPreferences,
    ) -> Result<Vec<u8>> {
        validate_read_id(request.read_id)?;
        validate_request(&request.session())?;
        validate_namespace(request)?;
        let _permit = self
            .workers
            .try_acquire()
            .map_err(|_| ConnectionError::Busy)?;
        let (target, session_cancel) = {
            let state = self
                .session
                .lock()
                .map_err(|_| ConnectionError::StoreUnavailable)?;
            let session = current(&state, &request.session())?;
            validate_credentials(
                session
                    .client
                    .as_ref()
                    .ok_or(ConnectionError::NotConnected)?,
            )?;
            (session.target.clone(), session.cancel.child_token())
        };
        let runtime = &self.helm_runtime;
        let storage = if detail.is_some() {
            &runtime.details
        } else {
            &runtime.slots
        };
        let cancel = HelmRuntime::token_in(storage, request, &session_cancel)?;
        let _read_guard = CancelOnDrop(cancel.clone());
        let result = super::bounded(&cancel, async {
            let executable = executable_with_preferences(&preferences)?;
            let catalog = self.catalog.clone();
            let id = request.cluster_id.clone();
            let expected = target.clone();
            let file_permit = self
                .file_workers
                .clone()
                .try_acquire_owned()
                .map_err(|_| ConnectionError::Busy)?;
            let view = request.view;
            let (prepared, file) = crate::runtime::spawn_blocking(move || {
                let _permit = file_permit;
                let latest = catalog
                    .connection_target(&id)
                    .map_err(super::map_catalog_error)?;
                if latest != expected {
                    return Err(ConnectionError::SourceChanged);
                }
                let prepared = if view == HelmView::Releases {
                    Some(config::prepare(&expected)?)
                } else {
                    None
                };
                let file = tempfile::NamedTempFile::new()
                    .map_err(|_| ConnectionError::KubeconfigUnavailable)?;
                Ok((prepared, file))
            })
            .await
            .map_err(|_| ConnectionError::KubeconfigUnavailable)??;
            let mut file = file;
            if let Some(mut prepared) = prepared {
                let base = target
                    .source_path
                    .parent()
                    .ok_or(ConnectionError::KubeconfigUnavailable)?;
                auth::resolve_tracked(&mut prepared, base, &self.auth_tasks).await?;
                let yaml = serde_yaml_ng::to_string(&prepared)
                    .map_err(|_| ConnectionError::KubeconfigInvalid)?;
                file.write_all(yaml.as_bytes())
                    .map_err(|_| ConnectionError::KubeconfigUnavailable)?;
                file.flush()
                    .map_err(|_| ConnectionError::KubeconfigUnavailable)?;
            }
            self.check_pod_session(&request.session())?;
            if let Some(detail) = detail
                && detail.view == HelmView::Charts
            {
                // Resolve only an exact configured-cache identity, never a renderer URL or path.
                let cached = self
                    .auth_tasks
                    .spawn(supervise_preferences(
                        executable.clone(),
                        arguments(HelmView::Charts, None),
                        tempfile::NamedTempFile::new()
                            .map_err(|_| ConnectionError::KubeconfigUnavailable)?,
                        cancel.clone(),
                        preferences.clone(),
                    ))
                    .await
                    .map_err(|_| ConnectionError::ConnectionFailed)??;
                validate_cached_chart(detail, &cached)?;
            }
            let mut args = detail
                .map(detail_arguments)
                .unwrap_or_else(|| arguments(request.view, request.namespace.as_deref()));
            if preferences.allow_untrusted_cas && detail.is_some_and(|d| d.view == HelmView::Charts)
            {
                args.push("--insecure-skip-tls-verify".into());
            }
            let output = self
                .auth_tasks
                .spawn(supervise_preferences(
                    executable,
                    args,
                    file,
                    cancel.clone(),
                    preferences.clone(),
                ))
                .await
                .map_err(|_| ConnectionError::ConnectionFailed)??;
            Ok(output)
        })
        .await;
        let canceled = cancel.is_cancelled();
        HelmRuntime::finish_in(storage, request);
        self.check_pod_session(&request.session())?;
        if canceled {
            Err(ConnectionError::StaleSession)
        } else {
            result
        }
    }

    pub fn helm_cancel(&self, request: HelmCancelRequest) -> Result<()> {
        self.helm_cancel_in(request, false)
    }

    pub fn helm_detail_cancel(&self, request: HelmCancelRequest) -> Result<()> {
        self.helm_cancel_in(request, true)
    }

    fn helm_cancel_in(&self, request: HelmCancelRequest, detail: bool) -> Result<()> {
        validate_request(&SessionRequest {
            cluster_id: request.cluster_id.clone(),
            session_id: request.session_id.clone(),
        })?;
        validate_read_id(request.read_id)?;
        let state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        let scope = SessionRequest {
            cluster_id: request.cluster_id.clone(),
            session_id: request.session_id.clone(),
        };
        // A closed lease is already canceled; never register it against a new lease.
        if let Ok(session) = current(&state, &scope) {
            let storage = if detail {
                &self.helm_runtime.details
            } else {
                &self.helm_runtime.slots
            };
            HelmRuntime::cancel_in(storage, &request, &session.cancel)?;
        }
        Ok(())
    }
}

fn validate_namespace(request: &HelmRequest) -> Result<()> {
    if let Some(namespace) = &request.namespace
        && (request.view != HelmView::Releases
            || namespace.is_empty()
            || namespace.len() > 63
            || !namespace
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
            || namespace.starts_with('-')
            || namespace.ends_with('-'))
    {
        return Err(ConnectionError::InvalidInput);
    }
    Ok(())
}
fn arguments(view: HelmView, namespace: Option<&str>) -> Vec<OsString> {
    match view {
        HelmView::Charts => ["search", "repo", "-o", "json"]
            .map(OsString::from)
            .to_vec(),
        HelmView::Releases => {
            let mut args = vec![OsString::from("list"), OsString::from("--all")];
            match namespace {
                Some(namespace) => {
                    args.push("--namespace".into());
                    args.push(namespace.into());
                }
                None => args.push("--all-namespaces".into()),
            }
            args.extend(["--output".into(), "json".into()]);
            args
        }
    }
}
fn executable_with_preferences(preferences: &HelmPreferences) -> Result<PathBuf> {
    validate_proxy(preferences.proxy.as_deref())?;
    if let Some(path) = &preferences.binary_path {
        if !path.is_absolute() {
            return Err(ConnectionError::InvalidInput);
        }
        let path = std::fs::canonicalize(path).map_err(|_| ConnectionError::HelmUnavailable)?;
        let metadata = path
            .metadata()
            .map_err(|_| ConnectionError::HelmUnavailable)?;
        if !metadata.is_file() {
            return Err(ConnectionError::HelmUnavailable);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 == 0 {
                return Err(ConnectionError::HelmUnavailable);
            }
        }
        return Ok(path);
    }
    let path = std::env::var_os("PATH").ok_or(ConnectionError::HelmUnavailable)?;
    lookup_executable(&path)
}
fn lookup_executable(path: &std::ffi::OsStr) -> Result<PathBuf> {
    for directory in std::env::split_paths(path) {
        if !directory.is_absolute() {
            continue;
        }
        let name = if cfg!(windows) { "helm.exe" } else { "helm" };
        let Ok(candidate) = std::fs::canonicalize(directory.join(name)) else {
            continue;
        };
        let Ok(metadata) = candidate.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 == 0 {
                continue;
            }
        }
        return Ok(candidate);
    }
    Err(ConnectionError::HelmUnavailable)
}
struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

async fn supervise_preferences(
    executable: PathBuf,
    args: Vec<OsString>,
    file: tempfile::NamedTempFile,
    cancel: CancellationToken,
    preferences: HelmPreferences,
) -> Result<Vec<u8>> {
    supervise_configured(
        executable,
        args,
        file,
        cancel,
        super::DEADLINE,
        None,
        preferences,
    )
    .await
}

#[cfg(test)]
async fn supervise(
    executable: PathBuf,
    args: Vec<OsString>,
    file: tempfile::NamedTempFile,
    cancel: CancellationToken,
) -> Result<Vec<u8>> {
    supervise_with_deadline(executable, args, file, cancel, super::DEADLINE).await
}

#[cfg(test)]
async fn supervise_with_deadline(
    executable: PathBuf,
    args: Vec<OsString>,
    file: tempfile::NamedTempFile,
    cancel: CancellationToken,
    deadline: std::time::Duration,
) -> Result<Vec<u8>> {
    supervise_owned(executable, args, file, cancel, deadline, None).await
}

#[cfg(test)]
async fn supervise_owned(
    executable: PathBuf,
    args: Vec<OsString>,
    file: tempfile::NamedTempFile,
    cancel: CancellationToken,
    deadline: std::time::Duration,
    directory: Option<tempfile::TempDir>,
) -> Result<Vec<u8>> {
    supervise_configured(
        executable,
        args,
        file,
        cancel,
        deadline,
        directory,
        HelmPreferences::default(),
    )
    .await
}

async fn supervise_configured(
    executable: PathBuf,
    args: Vec<OsString>,
    file: tempfile::NamedTempFile,
    cancel: CancellationToken,
    deadline: std::time::Duration,
    directory: Option<tempfile::TempDir>,
    preferences: HelmPreferences,
) -> Result<Vec<u8>> {
    if cancel.is_cancelled() {
        return Err(ConnectionError::StaleSession);
    }
    validate_proxy(preferences.proxy.as_deref())?;
    // Proxy preferences are for HTTP downloads, never Kubernetes transport.
    let config = std::fs::read(file.path()).map_err(|_| ConnectionError::KubeconfigUnavailable)?;
    let config: Value = serde_yaml_ng::from_slice(&config).unwrap_or(Value::Null);
    let bypass = config
        .get("clusters")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.pointer("/cluster/server").and_then(Value::as_str))
        .filter_map(|server| Url::parse(server).ok())
        .filter_map(|url| url.host_str().map(str::to_owned))
        .collect::<Vec<_>>()
        .join(",");
    let retain = directory.is_none();
    let mut command = CommandWrap::with_new(executable, |command| {
        if let Some(directory) = &directory {
            command.current_dir(directory.path());
        }
        command
            .args(args)
            .env_remove("HELM_DEBUG")
            .env("KUBECONFIG", file.path())
            .env("HELM_DRIVER", "secret")
            .env_remove("HELM_KUBECONTEXT")
            .env_remove("HELM_KUBEAPISERVER")
            .env_remove("HELM_KUBETOKEN")
            .env_remove("HELM_KUBECAFILE")
            .env_remove("HELM_KUBEINSECURE_SKIP_TLS_VERIFY")
            .env_remove("HELM_KUBETLS_SERVER_NAME")
            .env_remove("HELM_KUBEASUSER")
            .env_remove("HELM_KUBEASGROUPS")
            .env_remove("HTTP_PROXY")
            .env_remove("HTTPS_PROXY")
            .env_remove("ALL_PROXY")
            .env_remove("http_proxy")
            .env_remove("https_proxy")
            .env_remove("all_proxy")
            .env_remove("NO_PROXY")
            .env_remove("no_proxy")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(proxy) = &preferences.proxy {
            command
                .env("HTTP_PROXY", proxy)
                .env("HTTPS_PROXY", proxy)
                .env("http_proxy", proxy)
                .env("https_proxy", proxy)
                .env("NO_PROXY", &bypass)
                .env("no_proxy", &bypass);
        }
    });
    command.wrap(KillOnDrop);
    #[cfg(unix)]
    command.wrap(process_wrap::tokio::ProcessGroup::leader());
    #[cfg(windows)]
    command.wrap(process_wrap::tokio::JobObject);
    let mut child = command
        .spawn()
        .map_err(|_| ConnectionError::HelmUnavailable)?;
    let stdout = child
        .stdout()
        .take()
        .ok_or(ConnectionError::ConnectionFailed)?;
    let stderr = child
        .stderr()
        .take()
        .ok_or(ConnectionError::ConnectionFailed)?;
    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(ConnectionError::StaleSession),
        result = tokio::time::timeout(deadline, async {
            let (status, output, _) = tokio::try_join!(
                async { child.wait().await.map_err(|_| ConnectionError::ConnectionFailed) },
                read_output(stdout, retain), read_output(stderr, false))?;
            if !status.success() { return Err(ConnectionError::Conflict); }
            Ok(output)
        }) => result.unwrap_or(Err(ConnectionError::Timeout)),
    };
    let _ = child.start_kill();
    let _ = child.wait().await;
    drop(file);
    drop(directory);
    result
}
async fn read_output(mut reader: impl AsyncRead + Unpin, retain: bool) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut total = 0;
    let mut buffer = [0; 8192];
    loop {
        let count = reader
            .read(&mut buffer)
            .await
            .map_err(|_| ConnectionError::ConnectionFailed)?;
        if count == 0 {
            return Ok(output);
        }
        total += count;
        if total > OUTPUT_LIMIT {
            return Err(ConnectionError::LimitExceeded);
        }
        if retain {
            output.extend_from_slice(&buffer[..count]);
        }
    }
}
fn field(value: &Value, key: &str, required: bool) -> Result<String> {
    let text = value
        .get(key)
        .and_then(Value::as_str)
        .ok_or(ConnectionError::InvalidResponse)?;
    if text.len() > 4096 || text.chars().any(char::is_control) || (required && text.is_empty()) {
        return Err(ConnectionError::InvalidResponse);
    }
    Ok(text.to_owned())
}
fn project(request: &HelmRequest, bytes: &[u8]) -> Result<HelmPage> {
    if bytes.len() > OUTPUT_LIMIT {
        return Err(ConnectionError::LimitExceeded);
    }
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| ConnectionError::InvalidResponse)?;
    let rows = value.as_array().ok_or(ConnectionError::InvalidResponse)?;
    let mut items = Vec::new();
    let mut ids = std::collections::HashSet::new();
    for row in rows.iter().take(ITEM_LIMIT) {
        let name = field(row, "name", true)?;
        let (namespace, version, status, revision, updated, description) = match request.view {
            HelmView::Charts => (
                None,
                field(row, "version", true)?,
                None,
                None,
                None,
                Some(field(row, "description", false)?),
            ),
            HelmView::Releases => {
                let namespace = field(row, "namespace", true)?;
                let check = HelmRequest {
                    namespace: Some(namespace.clone()),
                    ..request.clone()
                };
                validate_namespace(&check).map_err(|_| ConnectionError::InvalidResponse)?;
                if request
                    .namespace
                    .as_ref()
                    .is_some_and(|selected| selected != &namespace)
                {
                    return Err(ConnectionError::InvalidResponse);
                }
                let revision = field(row, "revision", true)?
                    .parse::<u32>()
                    .map_err(|_| ConnectionError::InvalidResponse)?;
                (
                    Some(namespace),
                    field(row, "chart", true)?,
                    Some(field(row, "status", true)?),
                    Some(revision),
                    Some(field(row, "updated", false)?),
                    None,
                )
            }
        };
        let id = serde_json::to_string(&(&namespace, &name))
            .map_err(|_| ConnectionError::InvalidResponse)?;
        if !ids.insert(id.clone()) {
            return Err(ConnectionError::InvalidResponse);
        }
        items.push(HelmItem {
            id,
            name,
            namespace,
            version,
            app_version: field(row, "app_version", false)?,
            description,
            status,
            revision,
            updated,
        });
    }
    // Helm list defaults to 256 entries and does not expose a total in JSON.
    let truncated =
        rows.len() > ITEM_LIMIT || (request.view == HelmView::Releases && rows.len() >= 256);
    Ok(HelmPage {
        cluster_id: request.cluster_id.clone(),
        session_id: request.session_id.clone(),
        view: request.view,
        items,
        truncated,
        read_id: request.read_id,
        namespace: request.namespace.clone(),
    })
}

#[cfg(test)]
#[path = "helm_tests.rs"]
mod tests;

#[cfg(test)]
mod detail_tests {
    use super::*;
    fn request(view: HelmView) -> HelmDetailRequest {
        HelmDetailRequest {
            cluster_id: "one".into(),
            session_id: "lease-1".into(),
            read_id: 1,
            view,
            name: if view == HelmView::Charts {
                "stable/demo"
            } else {
                "demo"
            }
            .into(),
            namespace: (view == HelmView::Releases).then(|| "team-a".into()),
            version: (view == HelmView::Charts).then(|| "1.0.0".into()),
        }
    }
    #[test]
    fn details_reject_urls_paths_flags_and_unscoped_releases() {
        for name in [
            "https://host/chart",
            "../demo",
            "--help",
            "repo/a/b",
            "/tmp/demo",
        ] {
            let mut r = request(HelmView::Charts);
            r.name = name.into();
            assert_eq!(validate_detail(&r), Err(ConnectionError::InvalidInput));
        }
        let mut r = request(HelmView::Charts);
        r.version = Some("--help".into());
        assert_eq!(validate_detail(&r), Err(ConnectionError::InvalidInput));
        let mut r = request(HelmView::Releases);
        r.namespace = None;
        assert_eq!(validate_detail(&r), Err(ConnectionError::InvalidInput));
    }
    #[test]
    fn chart_projection_is_allowlisted_and_identity_bound() {
        let r = request(HelmView::Charts);
        let result = project_detail(&r, b"name: demo\nversion: 1.0.0\ndescription: Useful chart\nannotations:\n  token: secret-fixture\n").unwrap();
        let text = serde_json::to_string(&result).unwrap();
        assert!(text.contains("Useful chart"));
        assert!(!text.contains("secret-fixture"));
        assert!(project_detail(&r, b"name: other\nversion: 1.0.0\n").is_err());
        assert!(validate_cached_chart(&r, b"[]").is_err());
    }
    #[test]
    fn release_projection_withholds_descriptions_notes_and_values() {
        let result = project_detail(&request(HelmView::Releases), br#"[{"revision":2,"updated":"today","status":"deployed","chart":"demo-1.0.0","app_version":"1","description":"secret-fixture","notes":"secret-fixture","values":{"token":"secret-fixture"}}]"#).unwrap();
        assert_eq!(result.history[0].revision, 2);
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("secret-fixture")
        );
    }
    #[test]
    fn details_and_inventory_have_independent_cancellation() {
        let runtime = HelmRuntime::default();
        let root = CancellationToken::new();
        let r = request(HelmView::Charts).inventory();
        let inventory = runtime.token(&r, &root).unwrap();
        let detail = HelmRuntime::token_in(&runtime.details, &r, &root).unwrap();
        HelmRuntime::cancel_in(
            &runtime.details,
            &HelmCancelRequest {
                cluster_id: r.cluster_id.clone(),
                session_id: r.session_id.clone(),
                view: r.view,
                read_id: r.read_id,
            },
            &root,
        )
        .unwrap();
        assert!(detail.is_cancelled());
        assert!(!inventory.is_cancelled());
    }
    #[test]
    fn chart_metadata_budget_marks_partial() {
        let payload = format!(
            "name: demo\nversion: 1.0.0\ndescription: {}\n",
            "é".repeat(5000)
        );
        let result = project_detail(&request(HelmView::Charts), payload.as_bytes()).unwrap();
        assert!(result.truncated);
        assert!(result.facts.iter().all(|fact| fact.value.len() <= 4096));
    }
}
