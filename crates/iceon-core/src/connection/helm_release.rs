//! Release values read for the explicit upgrade editor. Values are fetched only
//! after a user action and are never logged or included in errors.
use super::install::HelmUpgradePreconditions;
use super::*;
use std::path::Path;

const VALUES_LIMIT: usize = 256 * 1024;
const VERSION_LIMIT: usize = 200;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelmReleaseValuesRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub read_id: u64,
    pub namespace: String,
    pub release_name: String,
}

impl HelmReleaseValuesRequest {
    fn detail(&self) -> HelmDetailRequest {
        HelmDetailRequest {
            cluster_id: self.cluster_id.clone(),
            session_id: self.session_id.clone(),
            read_id: self.read_id,
            view: HelmView::Releases,
            name: self.release_name.clone(),
            namespace: Some(self.namespace.clone()),
            version: None,
        }
    }
    fn session(&self) -> SessionRequest {
        SessionRequest {
            cluster_id: self.cluster_id.clone(),
            session_id: self.session_id.clone(),
        }
    }
}

/// A repository chart version that can be used as an upgrade target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelmChartVersion {
    pub chart: String,
    pub version: String,
    pub app_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelmReleaseMetadata {
    pub chart: String,
    pub version: String,
    pub app_version: String,
    pub revision: u32,
    pub status: String,
}

/// Intentionally not `Debug`: `values_yaml` may contain credentials.
#[derive(Clone)]
pub struct HelmReleaseValues {
    pub cluster_id: String,
    pub session_id: String,
    pub read_id: u64,
    pub namespace: String,
    pub release_name: String,
    pub metadata: HelmReleaseMetadata,
    /// User-supplied values of the current revision (`helm get values`).
    pub values_yaml: String,
    /// Release Secret identity; `None` when it is unreadable (e.g. a
    /// non-Secret storage driver), which disables upgrade preconditions.
    pub preconditions: Option<HelmUpgradePreconditions>,
    pub versions: Vec<HelmChartVersion>,
}

fn parse_metadata(request: &HelmReleaseValuesRequest, bytes: &[u8]) -> Result<HelmReleaseMetadata> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| ConnectionError::InvalidResponse)?;
    if field(&value, "name", true)? != request.release_name
        || field(&value, "namespace", true)? != request.namespace
    {
        return Err(ConnectionError::InvalidResponse);
    }
    let chart = field(&value, "chart", true)?;
    let version = field(&value, "version", true)?;
    if !safe_component(&chart, 128, b"_.-") || !safe_component(&version, 128, b".+_-") {
        return Err(ConnectionError::InvalidResponse);
    }
    let revision = value
        .get("revision")
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .filter(|v| *v > 0)
        .ok_or(ConnectionError::InvalidResponse)?;
    Ok(HelmReleaseMetadata {
        chart,
        version,
        app_version: field(&value, "appVersion", false)?,
        revision,
        status: field(&value, "status", false)?,
    })
}

fn normalize_values(bytes: &[u8]) -> Result<String> {
    if bytes.len() > VALUES_LIMIT {
        return Err(ConnectionError::LimitExceeded);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ConnectionError::InvalidResponse)?;
    if text.trim().is_empty() {
        return Ok("{}\n".into());
    }
    let value: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(text).map_err(|_| ConnectionError::InvalidResponse)?;
    match value {
        serde_yaml_ng::Value::Null => Ok("{}\n".into()),
        serde_yaml_ng::Value::Mapping(map) if map.is_empty() => Ok("{}\n".into()),
        serde_yaml_ng::Value::Mapping(_) => Ok(text.to_owned()),
        _ => Err(ConnectionError::InvalidResponse),
    }
}

fn parse_versions(chart_name: &str, bytes: &[u8]) -> Result<Vec<HelmChartVersion>> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| ConnectionError::InvalidResponse)?;
    let rows = value.as_array().ok_or(ConnectionError::InvalidResponse)?;
    let mut versions = Vec::new();
    for row in rows {
        let name = field(row, "name", true)?;
        let Some((repo, chart)) = name.split_once('/') else {
            continue;
        };
        let version = field(row, "version", true)?;
        if chart != chart_name
            || !safe_component(repo, 128, b"_.-")
            || !safe_component(&version, 128, b".+_-")
        {
            continue;
        }
        versions.push(HelmChartVersion {
            chart: name.clone(),
            version,
            app_version: field(row, "app_version", false)?,
        });
        if versions.len() >= VERSION_LIMIT {
            break;
        }
    }
    Ok(versions)
}

impl ConnectionEngine {
    /// Reads current release metadata, user-supplied values, upgrade
    /// preconditions and candidate repository versions for the upgrade editor.
    pub async fn helm_release_values_with_preferences(
        &self,
        request: HelmReleaseValuesRequest,
        preferences: HelmPreferences,
    ) -> Result<HelmReleaseValues> {
        validate_detail(&request.detail())?;
        let _permit = self
            .workers
            .try_acquire()
            .map_err(|_| ConnectionError::Busy)?;
        let (target, client, cancel) = {
            let state = self
                .session
                .lock()
                .map_err(|_| ConnectionError::StoreUnavailable)?;
            let session = current(&state, &request.session())?;
            let client = session
                .client
                .clone()
                .ok_or(ConnectionError::NotConnected)?;
            validate_credentials(&client)?;
            (session.target.clone(), client, session.cancel.child_token())
        };
        let _guard = CancelOnDrop(cancel.clone());
        let result = super::super::bounded(&cancel, async {
            let executable = executable_with_preferences(&preferences)?;
            let catalog = self.catalog.clone();
            let id = request.cluster_id.clone();
            let expected = target.clone();
            let permit = self
                .file_workers
                .clone()
                .try_acquire_owned()
                .map_err(|_| ConnectionError::Busy)?;
            let mut prepared = crate::runtime::spawn_blocking(move || {
                let _permit = permit;
                let latest = catalog
                    .connection_target(&id)
                    .map_err(super::super::map_catalog_error)?;
                if latest != expected {
                    return Err(ConnectionError::SourceChanged);
                }
                config::prepare(&expected)
            })
            .await
            .map_err(|_| ConnectionError::KubeconfigUnavailable)??;
            let base = target
                .source_path
                .parent()
                .ok_or(ConnectionError::KubeconfigUnavailable)?;
            auth::resolve_tracked(&mut prepared, base, &self.auth_tasks).await?;
            let yaml = serde_yaml_ng::to_string(&prepared)
                .map_err(|_| ConnectionError::KubeconfigInvalid)?;
            let config = || -> Result<tempfile::NamedTempFile> {
                let mut file = tempfile::NamedTempFile::new()
                    .map_err(|_| ConnectionError::KubeconfigUnavailable)?;
                file.write_all(yaml.as_bytes())
                    .map_err(|_| ConnectionError::KubeconfigUnavailable)?;
                file.flush()
                    .map_err(|_| ConnectionError::KubeconfigUnavailable)?;
                Ok(file)
            };
            self.check_pod_session(&request.session())?;
            let release = OsString::from(&request.release_name);
            let namespace = OsString::from(&request.namespace);
            let metadata = self
                .run_release_helm(
                    &executable,
                    vec![
                        "get".into(),
                        "metadata".into(),
                        release.clone(),
                        "--namespace".into(),
                        namespace.clone(),
                        "--output".into(),
                        "json".into(),
                    ],
                    config()?,
                    &cancel,
                    &preferences,
                )
                .await?;
            let metadata = parse_metadata(&request, &metadata)?;
            let values = self
                .run_release_helm(
                    &executable,
                    vec![
                        "get".into(),
                        "values".into(),
                        release,
                        "--namespace".into(),
                        namespace,
                        "--revision".into(),
                        metadata.revision.to_string().into(),
                        "--output".into(),
                        "yaml".into(),
                    ],
                    config()?,
                    &cancel,
                    &preferences,
                )
                .await?;
            let values_yaml = normalize_values(&values)?;
            // Repository lookup is best effort: values stay viewable without a repository.
            let versions = match self
                .run_release_helm(
                    &executable,
                    vec![
                        "search".into(),
                        "repo".into(),
                        metadata.chart.clone().into(),
                        "--versions".into(),
                        "--output".into(),
                        "json".into(),
                    ],
                    tempfile::NamedTempFile::new()
                        .map_err(|_| ConnectionError::KubeconfigUnavailable)?,
                    &cancel,
                    &preferences,
                )
                .await
            {
                Ok(bytes) => parse_versions(&metadata.chart, &bytes).unwrap_or_default(),
                Err(ConnectionError::StaleSession) => return Err(ConnectionError::StaleSession),
                Err(_) => Vec::new(),
            };
            let api: kube::Api<k8s_openapi::api::core::v1::Secret> =
                kube::Api::namespaced(client, &request.namespace);
            let secret = format!(
                "sh.helm.release.v1.{}.v{}",
                request.release_name, metadata.revision
            );
            let preconditions = api.get_metadata(&secret).await.ok().and_then(|object| {
                Some(HelmUpgradePreconditions {
                    revision: metadata.revision,
                    uid: object.metadata.uid.filter(|v| !v.is_empty())?,
                    resource_version: object.metadata.resource_version.filter(|v| !v.is_empty())?,
                })
            });
            Ok(HelmReleaseValues {
                cluster_id: request.cluster_id.clone(),
                session_id: request.session_id.clone(),
                read_id: request.read_id,
                namespace: request.namespace.clone(),
                release_name: request.release_name.clone(),
                metadata,
                values_yaml,
                preconditions,
                versions,
            })
        })
        .await;
        self.check_pod_session(&request.session())?;
        result
    }

    async fn run_release_helm(
        &self,
        executable: &Path,
        args: Vec<OsString>,
        file: tempfile::NamedTempFile,
        cancel: &CancellationToken,
        preferences: &HelmPreferences,
    ) -> Result<Vec<u8>> {
        self.auth_tasks
            .spawn(supervise_preferences(
                executable.to_path_buf(),
                args,
                file,
                cancel.clone(),
                preferences.clone(),
            ))
            .await
            .map_err(|_| ConnectionError::ConnectionFailed)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> HelmReleaseValuesRequest {
        HelmReleaseValuesRequest {
            cluster_id: "cluster".into(),
            session_id: "session".into(),
            read_id: 1,
            namespace: "dev".into(),
            release_name: "demo".into(),
        }
    }

    #[test]
    fn request_reuses_release_identity_validation() {
        assert!(validate_detail(&request().detail()).is_ok());
        for name in ["--all", "Demo", "a/b", ""] {
            let mut r = request();
            r.release_name = name.into();
            assert!(validate_detail(&r.detail()).is_err(), "{name}");
        }
        let mut r = request();
        r.namespace = "--namespace".into();
        assert!(validate_detail(&r.detail()).is_err());
    }

    #[test]
    fn metadata_is_identity_bound_for_helm_3_and_4() {
        let helm3 = br#"{"name":"demo","chart":"nginx","version":"1.2.3","appVersion":"1.25","namespace":"dev","revision":3,"status":"deployed","deployedAt":"2026-01-01T00:00:00Z"}"#;
        let helm4 = br#"{"name":"demo","chart":"nginx","version":"1.2.3","appVersion":"1.25","labels":{"a":"b"},"namespace":"dev","revision":3,"status":"deployed","deployedAt":"2026-01-01T00:00:00Z","applyMethod":"ssa"}"#;
        for bytes in [helm3.as_slice(), helm4] {
            let metadata = parse_metadata(&request(), bytes).unwrap();
            assert_eq!(metadata.chart, "nginx");
            assert_eq!(metadata.revision, 3);
        }
        for bytes in [
            br#"{"name":"other","chart":"nginx","version":"1","appVersion":"","namespace":"dev","revision":1,"status":""}"#.as_slice(),
            br#"{"name":"demo","chart":"nginx","version":"1","appVersion":"","namespace":"prod","revision":1,"status":""}"#,
            br#"{"name":"demo","chart":"--flag","version":"1","appVersion":"","namespace":"dev","revision":1,"status":""}"#,
            br#"{"name":"demo","chart":"nginx","version":"1","appVersion":"","namespace":"dev","revision":0,"status":""}"#,
            b"garbage",
        ] {
            assert!(parse_metadata(&request(), bytes).is_err());
        }
    }

    #[test]
    fn values_are_mapping_documents_and_bounded() {
        assert_eq!(normalize_values(b"").unwrap(), "{}\n");
        assert_eq!(normalize_values(b"null\n").unwrap(), "{}\n");
        assert_eq!(normalize_values(b"{}\n").unwrap(), "{}\n");
        assert_eq!(
            normalize_values(b"replicaCount: 2\n# keep\n").unwrap(),
            "replicaCount: 2\n# keep\n"
        );
        assert!(normalize_values(b"- a\n").is_err());
        assert!(normalize_values(&[0xff]).is_err());
        assert_eq!(
            normalize_values(&vec![b'a'; VALUES_LIMIT + 1]),
            Err(ConnectionError::LimitExceeded)
        );
    }

    #[test]
    fn versions_keep_exact_repository_chart_matches_only() {
        let bytes = br#"[
            {"name":"bitnami/nginx","version":"2.0.0","app_version":"1.27","description":"x"},
            {"name":"bitnami/nginx","version":"1.9.0","app_version":"1.26","description":"x"},
            {"name":"bitnami/nginx-ingress","version":"9.0.0","app_version":"1","description":"x"},
            {"name":"other/nginx","version":"--flag","app_version":"","description":"x"},
            {"name":"mirror/nginx","version":"1.0.0","app_version":"","description":"x"}
        ]"#;
        let versions = parse_versions("nginx", bytes).unwrap();
        assert_eq!(
            versions
                .iter()
                .map(|v| format!("{}@{}", v.chart, v.version))
                .collect::<Vec<_>>(),
            [
                "bitnami/nginx@2.0.0",
                "bitnami/nginx@1.9.0",
                "mirror/nginx@1.0.0"
            ]
        );
        assert!(parse_versions("nginx", b"{}").is_err());
    }
}
