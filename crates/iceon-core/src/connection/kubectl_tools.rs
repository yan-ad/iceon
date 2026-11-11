//! Lease-scoped kubectl preparation. Downloaded executables are never launched.

use super::{ConnectionEngine, ConnectionError, SessionRequest, bounded, transport};
use crate::{
    kubernetes_tools::{KubectlError, ResolvedKubectl, resolve_kubectl},
    preferences::{KubernetesPreferences, ProxyPreferences},
};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PrepareKubectlError {
    #[error("cluster connection is unavailable: {0:?}")]
    Connection(ConnectionError),
    #[error(transparent)]
    Tool(#[from] KubectlError),
}

impl From<ConnectionError> for PrepareKubectlError {
    fn from(error: ConnectionError) -> Self {
        Self::Connection(error)
    }
}

impl ConnectionEngine {
    /// Query `/version` using the current authenticated client, not another
    /// kubeconfig context or a local kubectl process.
    pub async fn server_version(&self, request: &SessionRequest) -> super::Result<String> {
        let (client, cancel) = self.pod_client(request)?;
        let version = bounded(&cancel, transport::version(&client, &cancel)).await?;
        self.check_pod_session(request)?;
        Ok(version)
    }

    /// Prepare the saved override or opt-in cluster-matching download. Dropping
    /// the workspace lease cancels both version discovery and HTTP downloads.
    /// No PATH/environment changes or executable invocation take place.
    pub async fn prepare_kubectl(
        &self,
        request: &SessionRequest,
        prefs: &KubernetesPreferences,
        proxy: &ProxyPreferences,
        data_dir: &Path,
    ) -> Result<ResolvedKubectl, PrepareKubectlError> {
        let (_, cancel) = self.pod_client(request)?;
        // Try all configured local paths before querying `/version`, which may
        // be denied even when the local binary is usable. Keep the resolver's
        // path validation and explicit-override precedence intact.
        let mut local_prefs = prefs.clone();
        local_prefs.kubectl_auto_download = false;
        let local = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(ConnectionError::StaleSession.into()),
            result = resolve_kubectl(&local_prefs, proxy, "", data_dir) => result,
        };
        let resolved = match local {
            Ok(resolved) => resolved,
            Err(KubectlError::NotConfigured)
                if prefs.kubectl_auto_download && prefs.kubectl_binary_path.is_empty() =>
            {
                let version = self.server_version(request).await?;
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return Err(ConnectionError::StaleSession.into()),
                    result = resolve_kubectl(prefs, proxy, &version, data_dir) => result?,
                }
            }
            Err(error) => return Err(error.into()),
        };
        self.check_pod_session(request)?;
        Ok(resolved)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        OpenRequest,
        tests::{connected, fake, fixture, identity, normal},
    };
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[tokio::test]
    async fn version_uses_live_session_and_rejects_replaced_lease() {
        let (_root, _catalog, engine, id, _) = fixture();
        let session = connected(&engine, id.clone(), fake(normal)).await;
        let request = identity(&session);
        assert_eq!(engine.server_version(&request).await.unwrap(), "v1.36.0");
        engine.open(OpenRequest { cluster_id: id }).await.unwrap();
        assert_eq!(
            engine.server_version(&request).await.unwrap_err(),
            ConnectionError::StaleSession
        );
    }

    #[tokio::test]
    async fn disabled_download_does_not_query_server_or_create_directory() {
        let (root, _catalog, engine, id, _) = fixture();
        let calls = Arc::new(AtomicUsize::new(0));
        let requests = calls.clone();
        let session = connected(
            &engine,
            id,
            fake(move |path| {
                requests.fetch_add(1, Ordering::SeqCst);
                normal(path)
            }),
        )
        .await;
        let before = calls.load(Ordering::SeqCst);
        let prefs = KubernetesPreferences {
            kubectl_auto_download: false,
            ..Default::default()
        };
        let destination = root.path().join("not-created");
        let result = engine
            .prepare_kubectl(
                &identity(&session),
                &prefs,
                &ProxyPreferences::default(),
                &destination,
            )
            .await;
        assert!(matches!(
            result,
            Err(PrepareKubectlError::Tool(KubectlError::NotConfigured))
        ));
        assert_eq!(calls.load(Ordering::SeqCst), before);
        assert!(!destination.exists());
    }

    #[tokio::test]
    async fn explicit_override_is_resolved_without_server_request() {
        let (root, _catalog, engine, id, _) = fixture();
        let session = connected(&engine, id, fake(normal)).await;
        let path = root.path().join("kubectl-override");
        std::fs::write(&path, "not executed").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        // Any further HTTP request fails; the explicit override must win.
        engine.session.lock().unwrap().as_mut().unwrap().client =
            Some(fake(|_| panic!("unexpected API request")));
        let prefs = KubernetesPreferences {
            kubectl_auto_download: true,
            kubectl_binary_path: path.to_string_lossy().into_owned(),
            ..Default::default()
        };
        let result = engine
            .prepare_kubectl(
                &identity(&session),
                &prefs,
                &ProxyPreferences::default(),
                root.path(),
            )
            .await
            .unwrap();
        assert_eq!(result.path, path);
        assert!(!result.downloaded);
    }

    #[tokio::test]
    async fn binaries_directory_resolves_locally_when_version_is_denied() {
        let (root, _catalog, engine, id, _) = fixture();
        let session = connected(&engine, id, fake(normal)).await;
        let calls = Arc::new(AtomicUsize::new(0));
        let requests = calls.clone();
        engine.session.lock().unwrap().as_mut().unwrap().client = Some(fake(move |_| {
            requests.fetch_add(1, Ordering::SeqCst);
            (
                403,
                br#"{"kind":"Status","status":"Failure","reason":"Forbidden","code":403}"#.to_vec(),
            )
        }));
        let directory = root.path().join("local-tools");
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join(if cfg!(windows) {
            "kubectl.exe"
        } else {
            "kubectl"
        });
        std::fs::write(&path, "not executed").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let prefs = KubernetesPreferences {
            kubectl_auto_download: true,
            binaries_directory: directory.to_string_lossy().into_owned(),
            ..Default::default()
        };
        let destination = root.path().join("not-created");
        let result = engine
            .prepare_kubectl(
                &identity(&session),
                &prefs,
                &ProxyPreferences::default(),
                &destination,
            )
            .await
            .unwrap();
        assert_eq!(result.path, path);
        assert!(!result.downloaded);
        assert_eq!(result.version, None);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(!destination.exists());
    }

    #[tokio::test]
    async fn matching_release_download_is_cancelled_when_lease_is_replaced() {
        use crate::preferences::DownloadMirror;
        use axum::{Router, routing::get};
        let (root, _catalog, engine, id, _) = fixture();
        let engine = Arc::new(engine);
        let session = connected(&engine, id.clone(), fake(normal)).await;
        let entered = Arc::new(tokio::sync::Notify::new());
        let requested = entered.clone();
        let app = Router::new().fallback(get(move |uri: axum::http::Uri| {
            let requested = requested.clone();
            async move {
                assert!(uri.path().starts_with("/release/v1.36.0/bin/"));
                requested.notify_one();
                std::future::pending::<&'static str>().await
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let worker = engine.clone();
        let destination = root.path().join("tools");
        let target = destination.clone();
        let task = tokio::spawn(async move {
            let prefs = KubernetesPreferences {
                kubectl_auto_download: true,
                kubectl_download_mirror: DownloadMirror::Custom,
                kubectl_mirror_url: url,
                ..Default::default()
            };
            worker
                .prepare_kubectl(
                    &identity(&session),
                    &prefs,
                    &ProxyPreferences::default(),
                    &target,
                )
                .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(2), entered.notified())
            .await
            .unwrap();
        engine.open(OpenRequest { cluster_id: id }).await.unwrap();
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            result,
            Err(PrepareKubectlError::Connection(
                ConnectionError::StaleSession
            ))
        ));
        assert!(!destination.exists());
        server.abort();
    }
}
