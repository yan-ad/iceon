use super::*;
use std::time::Duration;

const VALUES_LIMIT: usize = 256 * 1024;
const INSTALL_DEADLINE: Duration = Duration::from_secs(120);

/// Identity of the Helm release Secret shown to the user. This is a
/// conservative preflight, not an atomic compare-and-swap (Helm has no such CLI flag).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelmUpgradePreconditions {
    pub revision: u32,
    pub uid: String,
    pub resource_version: String,
}

fn reopen_config(file: &tempfile::NamedTempFile) -> Result<tempfile::NamedTempFile> {
    let mut copy =
        tempfile::NamedTempFile::new().map_err(|_| ConnectionError::KubeconfigUnavailable)?;
    std::io::copy(
        &mut file
            .reopen()
            .map_err(|_| ConnectionError::KubeconfigUnavailable)?,
        &mut copy,
    )
    .map_err(|_| ConnectionError::KubeconfigUnavailable)?;
    Ok(copy)
}

// Helm 3 has no native SSA/force-conflicts flags. Do not silently downgrade
// a requested ownership policy. Helm 4 install is bool; upgrade is string.
fn apply_preference_arguments(
    args: &mut Vec<OsString>,
    version: &[u8],
    preferences: &HelmPreferences,
    force_conflicts: bool,
) -> Result<()> {
    let version = std::str::from_utf8(version).map_err(|_| ConnectionError::HelmUnavailable)?;
    let version =
        semver::Version::parse(version.trim().strip_prefix('v').unwrap_or(version.trim()))
            .map_err(|_| ConnectionError::HelmUnavailable)?;
    if version.major == 3 {
        if preferences.server_side || force_conflicts {
            return Err(ConnectionError::HelmUnavailable);
        }
    } else if version.major == 4 {
        args.push(
            format!(
                "--server-side={}",
                preferences.server_side || force_conflicts
            )
            .into(),
        );
        if force_conflicts {
            args.push("--force-conflicts".into());
        }
    } else {
        return Err(ConnectionError::HelmUnavailable);
    }
    Ok(())
}

// Helm 3.17 install flags: https://github.com/helm/helm/blob/v3.17.0/cmd/helm/install.go
// Helm 4.3.1: https://helm.sh/docs/helm/helm_install/
// Both support server dry-run/hide-secret and --wait; rollback flags differ, so neither is used.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelmInstallRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub read_id: u64,
    pub namespace: String,
    pub release_name: String,
    pub chart: String,
    pub version: Option<String>,
    pub values_yaml: String,
    pub dry_run: bool,
}
impl HelmInstallRequest {
    fn session(&self) -> SessionRequest {
        SessionRequest {
            cluster_id: self.cluster_id.clone(),
            session_id: self.session_id.clone(),
        }
    }
    fn key(&self) -> InstallKey {
        (
            self.cluster_id.clone(),
            self.session_id.clone(),
            self.read_id,
        )
    }
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelmInstallCancelRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub read_id: u64,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HelmInstallResult {
    pub cluster_id: String,
    pub session_id: String,
    pub namespace: String,
    pub release_name: String,
    pub chart: String,
    pub created: bool,
}
type InstallKey = (String, String, u64);
#[derive(Debug)]
struct InstallSlot {
    root: CancellationToken,
    cancel: Option<CancellationToken>,
}
#[derive(Debug, Default)]
pub(super) struct InstallRuntime(Mutex<HashMap<InstallKey, InstallSlot>>);
impl InstallRuntime {
    // Retain exact-ID tombstones until disconnect, preventing late IPC work from spawning.
    fn register(
        &self,
        key: InstallKey,
        root: &CancellationToken,
        cancel_only: bool,
    ) -> Result<CancellationToken> {
        let mut slots = self
            .0
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        slots.retain(|_, slot| !slot.root.is_cancelled());
        if root.is_cancelled() {
            return Err(ConnectionError::StaleSession);
        }
        if let Some(slot) = slots.get_mut(&key) {
            if cancel_only {
                if let Some(cancel) = slot.cancel.take() {
                    cancel.cancel();
                }
                return Ok(root.child_token());
            }
            return Err(ConnectionError::StaleSession);
        }
        if slots.len() >= 256 {
            return Err(ConnectionError::Busy);
        }
        if !cancel_only
            && slots.iter().any(|((cluster, session, _), slot)| {
                cluster == &key.0 && session == &key.1 && slot.cancel.is_some()
            })
        {
            return Err(ConnectionError::Busy);
        }
        let token = root.child_token();
        slots.insert(
            key,
            InstallSlot {
                root: root.clone(),
                cancel: (!cancel_only).then(|| token.clone()),
            },
        );
        Ok(token)
    }
    fn finish(&self, key: &InstallKey) {
        if let Ok(mut slots) = self.0.lock()
            && let Some(slot) = slots.get_mut(key)
            && let Some(cancel) = slot.cancel.take()
        {
            cancel.cancel();
        }
    }
}
impl Drop for InstallRuntime {
    fn drop(&mut self) {
        if let Ok(slots) = self.0.get_mut() {
            for slot in slots.values() {
                if let Some(cancel) = &slot.cancel {
                    cancel.cancel();
                }
            }
        }
    }
}
struct InstallGuard<'a>(&'a InstallRuntime, InstallKey);
impl Drop for InstallGuard<'_> {
    fn drop(&mut self) {
        self.0.finish(&self.1);
    }
}
fn dns_label(value: &str, limit: usize) -> bool {
    !value.is_empty()
        && value.len() <= limit
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        && !value.starts_with('-')
        && !value.ends_with('-')
}
fn validate(request: &HelmInstallRequest) -> Result<()> {
    validate_request(&request.session())?;
    validate_read_id(request.read_id)?;
    let parts: Vec<_> = request.chart.split('/').collect();
    let safe_part = |part: &str| {
        !part.is_empty()
            && part.len() <= 128
            && part.as_bytes()[0].is_ascii_alphanumeric()
            && part
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
            && !part.contains("..")
    };
    if !dns_label(&request.namespace, 63)
        || !dns_label(&request.release_name, 53)
        || parts.len() != 2
        || !parts.iter().all(|part| safe_part(part))
        || request.version.as_ref().is_some_and(|v| {
            v.is_empty()
                || v.len() > 128
                || v.starts_with('-')
                || !v.bytes().all(|b| b.is_ascii_graphic() || b == b' ')
        })
        || request.values_yaml.len() > VALUES_LIMIT
    {
        return Err(ConnectionError::InvalidInput);
    }
    let mut documents = serde_yaml_ng::Deserializer::from_str(&request.values_yaml);
    let value =
        serde_yaml_ng::Value::deserialize(documents.next().ok_or(ConnectionError::InvalidInput)?)
            .map_err(|_| ConnectionError::InvalidInput)?;
    if !matches!(value, serde_yaml_ng::Value::Mapping(_)) || documents.next().is_some() {
        return Err(ConnectionError::InvalidInput);
    }
    Ok(())
}
fn install_arguments(request: &HelmInstallRequest, values: &std::path::Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "install",
        &request.release_name,
        &request.chart,
        "--namespace",
        &request.namespace,
        "--values",
    ]
    .map(Into::into)
    .to_vec();
    args.push(values.into());
    args.extend(["--output", "json", "--timeout", "60s"].map(Into::into));
    if let Some(version) = &request.version {
        args.push(format!("--version={version}").into());
    }
    if request.dry_run {
        args.extend(["--dry-run=server", "--hide-secret"].map(Into::into));
    } else {
        args.push("--wait".into());
    }
    args
}
fn supported_version(output: &[u8]) -> bool {
    let Ok(version) = std::str::from_utf8(output) else {
        return false;
    };
    let mut parts = version.trim().strip_prefix('v').unwrap_or("").split('.');
    matches!(
        (
            parts.next().and_then(|p| p.parse::<u32>().ok()),
            parts.next().and_then(|p| p.parse::<u32>().ok())
        ),
        (Some(3), Some(17..)) | (Some(4), Some(_))
    )
}
impl ConnectionEngine {
    pub async fn helm_upgrade(
        &self,
        request: HelmInstallRequest,
        expected: HelmUpgradePreconditions,
    ) -> Result<HelmInstallResult> {
        self.helm_upgrade_with_preferences(request, expected, HelmPreferences::default(), false).await
    }
    pub async fn helm_upgrade_with_preferences(
        &self,
        request: HelmInstallRequest,
        expected: HelmUpgradePreconditions,
        preferences: HelmPreferences,
        force_conflicts: bool,
    ) -> Result<HelmInstallResult> {
        if expected.revision == 0
            || expected.uid.is_empty()
            || expected.resource_version.is_empty()
            || expected.uid.len() > 256
            || expected.resource_version.len() > 256
            || expected
                .uid
                .chars()
                .chain(expected.resource_version.chars())
                .any(|c| c.is_control() || c.is_whitespace())
        {
            return Err(ConnectionError::InvalidInput);
        }
        self.helm_mutate(request, preferences, force_conflicts, Some(expected))
            .await
    }
    async fn check_helm_upgrade(
        &self,
        request: &HelmInstallRequest,
        expected: &HelmUpgradePreconditions,
    ) -> Result<()> {
        let client = {
            let state = self
                .session
                .lock()
                .map_err(|_| ConnectionError::StoreUnavailable)?;
            current(&state, &request.session())?
                .client
                .clone()
                .ok_or(ConnectionError::NotConnected)?
        };
        let api: kube::Api<k8s_openapi::api::core::v1::Secret> =
            kube::Api::namespaced(client, &request.namespace);
        let name = format!(
            "sh.helm.release.v1.{}.v{}",
            request.release_name, expected.revision
        );
        let metadata = api
            .get_metadata(&name)
            .await
            .map_err(|_| ConnectionError::Conflict)?
            .metadata;
        if metadata.uid.as_deref() != Some(expected.uid.as_str())
            || metadata.resource_version.as_deref() != Some(expected.resource_version.as_str())
        {
            return Err(ConnectionError::Conflict);
        }
        Ok(())
    }
    pub async fn helm_install(&self, request: HelmInstallRequest) -> Result<HelmInstallResult> {
        self.helm_install_with_preferences(request, HelmPreferences::default(), false)
            .await
    }
    pub async fn helm_install_with_preferences(
        &self,
        request: HelmInstallRequest,
        preferences: HelmPreferences,
        force_conflicts: bool,
    ) -> Result<HelmInstallResult> {
        self.helm_mutate(request, preferences, force_conflicts, None)
            .await
    }
    async fn helm_mutate(
        &self,
        request: HelmInstallRequest,
        preferences: HelmPreferences,
        force_conflicts: bool,
        upgrade: Option<HelmUpgradePreconditions>,
    ) -> Result<HelmInstallResult> {
        validate(&request)?;
        let _permit = self
            .workers
            .try_acquire()
            .map_err(|_| ConnectionError::Busy)?;
        let (target, root) = {
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
            (session.target.clone(), session.cancel.clone())
        };
        let runtime = &self.helm_runtime.install;
        let cancel = runtime.register(request.key(), &root, false)?;
        let _guard = InstallGuard(runtime, request.key());
        let result = tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(ConnectionError::StaleSession),
            result = tokio::time::timeout(INSTALL_DEADLINE, async {
                let executable = executable_with_preferences(&preferences)?;
                let catalog = self.catalog.clone();
                let id = request.cluster_id.clone();
                let expected = target.clone();
                let permit = self.file_workers.clone().try_acquire_owned().map_err(|_| ConnectionError::Busy)?;
                let mut prepared = crate::runtime::spawn_blocking(move || {
                    let _permit = permit;
                    let latest = catalog.connection_target(&id).map_err(super::super::map_catalog_error)?;
                    if latest != expected { return Err(ConnectionError::SourceChanged); }
                    config::prepare(&expected)
                }).await.map_err(|_| ConnectionError::KubeconfigUnavailable)??;
                let base = target.source_path.parent().ok_or(ConnectionError::KubeconfigUnavailable)?;
                auth::resolve_tracked(&mut prepared, base, &self.auth_tasks).await?;
                let version = self.auth_tasks.spawn(supervise_preferences(executable.clone(),
                    ["version", "--template", "{{.Version}}"].map(Into::into).to_vec(),
                    tempfile::NamedTempFile::new().map_err(|_| ConnectionError::KubeconfigUnavailable)?, cancel.clone(), preferences.clone()))
                    .await.map_err(|_| ConnectionError::ConnectionFailed)??;
                if !supported_version(&version) { return Err(ConnectionError::HelmUnavailable); }
                let directory = tempfile::tempdir().map_err(|_| ConnectionError::KubeconfigUnavailable)?;
                let mut kubeconfig = tempfile::NamedTempFile::new_in(directory.path()).map_err(|_| ConnectionError::KubeconfigUnavailable)?;
                let yaml = serde_yaml_ng::to_string(&prepared).map_err(|_| ConnectionError::KubeconfigInvalid)?;
                kubeconfig.write_all(yaml.as_bytes()).map_err(|_| ConnectionError::KubeconfigUnavailable)?;
                kubeconfig.flush().map_err(|_| ConnectionError::KubeconfigUnavailable)?;
                let mut values = tempfile::NamedTempFile::new_in(directory.path()).map_err(|_| ConnectionError::KubeconfigUnavailable)?;
                values.write_all(request.values_yaml.as_bytes()).map_err(|_| ConnectionError::KubeconfigUnavailable)?;
                values.flush().map_err(|_| ConnectionError::KubeconfigUnavailable)?;
                self.check_pod_session(&request.session())?;
                let mut args = install_arguments(&request, values.path());
                if let Some(expected) = &upgrade {
                    // Check both the displayed latest revision and its Kubernetes identity.
                    self.check_helm_upgrade(&request, expected).await?;
                    let history = self.auth_tasks.spawn(supervise_preferences(executable.clone(),
                        vec!["history".into(), request.release_name.clone().into(), "--namespace".into(), request.namespace.clone().into(), "--max".into(), "1".into(), "--output".into(), "json".into()],
                        reopen_config(&kubeconfig)?, cancel.clone(), preferences.clone())).await.map_err(|_| ConnectionError::ConnectionFailed)??;
                    let history: Value = serde_json::from_slice(&history).map_err(|_| ConnectionError::InvalidResponse)?;
                    if history.as_array().filter(|rows| rows.len() == 1).and_then(|rows| rows[0].get("revision")).and_then(Value::as_u64) != Some(expected.revision as u64) {
                        return Err(ConnectionError::Conflict);
                    }
                    args[0] = "upgrade".into();
                }
                apply_preference_arguments(&mut args, &version, &preferences, force_conflicts)?;
                if preferences.allow_untrusted_cas { args.push("--insecure-skip-tls-verify".into()); }
                self.check_pod_session(&request.session())?;
                // The tracked supervisor, not the IPC future, owns all files until the child is reaped.
                let process_cancel = cancel.clone();
                self.auth_tasks.spawn(async move {
                    let _values = values;
                    supervise_configured(executable, args, kubeconfig, process_cancel, INSTALL_DEADLINE, Some(directory), preferences).await
                }).await.map_err(|_| ConnectionError::ConnectionFailed)??;
                Ok(HelmInstallResult { cluster_id: request.cluster_id.clone(), session_id: request.session_id.clone(),
                    namespace: request.namespace.clone(), release_name: request.release_name.clone(), chart: request.chart.clone(), created: !request.dry_run && upgrade.is_none() })
            }) => result.unwrap_or(Err(ConnectionError::Timeout)),
        };
        self.check_pod_session(&request.session())?;
        result
    }
    pub fn helm_install_cancel(&self, request: HelmInstallCancelRequest) -> Result<()> {
        let scope = SessionRequest {
            cluster_id: request.cluster_id.clone(),
            session_id: request.session_id.clone(),
        };
        validate_request(&scope)?;
        validate_read_id(request.read_id)?;
        let state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        if let Ok(session) = current(&state, &scope) {
            self.helm_runtime.install.register(
                (request.cluster_id, request.session_id, request.read_id),
                &session.cancel,
                true,
            )?;
        }
        Ok(())
    }
}
#[cfg(test)]
#[path = "helm_install_tests.rs"]
mod tests;
