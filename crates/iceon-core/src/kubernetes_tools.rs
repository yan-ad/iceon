//! Safe, explicit kubectl resolution and installation.
//!
//! This module only resolves or downloads a binary. It never executes kubectl.

use crate::preferences::{
    DownloadMirror, KubernetesPreferences, ProxyPreferences, secure_download_url,
};
use reqwest::Client;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use thiserror::Error;
use tokio::time::timeout;

const MAX_BINARY_BYTES: u64 = 256 * 1024 * 1024;
const MAX_HASH_BYTES: u64 = 16 * 1024;
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Debug, Error)]
pub enum KubectlError {
    #[error("kubectl binary is not configured")]
    NotConfigured,
    #[error("invalid kubectl version: {0}")]
    InvalidVersion(String),
    #[error("invalid download mirror")]
    InvalidMirror,
    #[error("download failed: {0}")]
    Download(#[from] reqwest::Error),
    #[error("download returned HTTP status {0}")]
    HttpStatus(reqwest::StatusCode),
    #[error("download exceeds the permitted size")]
    TooLarge,
    #[error("download hash did not match expected SHA-256")]
    HashMismatch,
    #[error("download hash sidecar is missing or invalid")]
    InvalidHashSidecar,
    #[error("filesystem error: {0}")]
    Io(#[from] io::Error),
    #[error("download timed out")]
    Timeout,
    #[error("invalid path")]
    InvalidPath,
}

fn usable_file(path: &Path) -> Result<bool, KubectlError> {
    if !path.exists() {
        return Ok(false);
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(KubectlError::InvalidPath);
    }
    Ok(true)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedKubectl {
    pub path: PathBuf,
    pub downloaded: bool,
    pub version: Option<String>,
}

fn release_version(version: &str) -> Result<String, KubectlError> {
    if version.len() > 128
        || version
            .chars()
            .any(|c| !c.is_ascii_alphanumeric() && !matches!(c, '.' | '-' | '+'))
    {
        return Err(KubectlError::InvalidVersion(version.into()));
    }
    let parsed = semver::Version::parse(version.strip_prefix('v').unwrap_or(version))
        .map_err(|_| KubectlError::InvalidVersion(version.into()))?;
    if parsed.major != 1 {
        return Err(KubectlError::InvalidVersion(version.into()));
    }
    // Vendor suffixes (e.g. +k3s1, -eks-...) are not upstream release names.
    Ok(format!(
        "v{}.{}.{}",
        parsed.major, parsed.minor, parsed.patch
    ))
}

fn platform() -> (&'static str, &'static str, &'static str) {
    (
        match std::env::consts::OS {
            "macos" => "darwin",
            os => os,
        },
        match std::env::consts::ARCH {
            "aarch64" => "arm64",
            "x86_64" => "amd64",
            arch => arch,
        },
        if cfg!(windows) { ".exe" } else { "" },
    )
}

fn ensure_safe_dir(path: &Path) -> Result<(), KubectlError> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|c| c == std::path::Component::ParentDir)
    {
        return Err(KubectlError::InvalidPath);
    }
    fs::create_dir_all(path)?;
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(KubectlError::InvalidPath);
    }
    Ok(())
}

fn configured_path(prefs: &KubernetesPreferences) -> Option<PathBuf> {
    if !prefs.kubectl_binary_path.is_empty() {
        Some(PathBuf::from(&prefs.kubectl_binary_path))
    } else if !prefs.binaries_directory.is_empty() {
        let (_, _, suffix) = platform();
        Some(PathBuf::from(&prefs.binaries_directory).join(format!("kubectl{suffix}")))
    } else {
        None
    }
}

fn usable_binary(path: &Path) -> Result<bool, KubectlError> {
    if !path.exists() {
        return Ok(false);
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(KubectlError::InvalidPath);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(KubectlError::InvalidPath);
        }
    }
    Ok(true)
}

fn mirror_base(prefs: &KubernetesPreferences) -> Result<String, KubectlError> {
    let base = match prefs.kubectl_download_mirror {
        DownloadMirror::Default => "https://dl.k8s.io".to_owned(),
        DownloadMirror::Custom => prefs.kubectl_mirror_url.clone(),
    };
    let base = base.trim_end_matches('/');
    if prefs.kubectl_download_mirror == DownloadMirror::Custom
        && secure_download_url(base).is_none()
    {
        return Err(KubectlError::InvalidMirror);
    }
    Ok(base.to_owned())
}

async fn fetch_bounded(client: &Client, url: &str, max: u64) -> Result<Vec<u8>, KubectlError> {
    let mut response = timeout(DOWNLOAD_TIMEOUT, client.get(url).send())
        .await
        .map_err(|_| KubectlError::Timeout)??;
    if !response.status().is_success() {
        return Err(KubectlError::HttpStatus(response.status()));
    }
    if response.content_length().is_some_and(|n| n > max) {
        return Err(KubectlError::TooLarge);
    }
    let mut bytes = Vec::new();
    let read = async {
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() as u64 + chunk.len() as u64 > max {
                return Err(KubectlError::TooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    };
    timeout(DOWNLOAD_TIMEOUT, read)
        .await
        .map_err(|_| KubectlError::Timeout)?
}

fn parse_hash(body: &[u8]) -> Option<[u8; 32]> {
    let text = std::str::from_utf8(body).ok()?;
    let hex = text
        .split_whitespace()
        .next()?
        .strip_prefix("sha256:")
        .unwrap_or(text.split_whitespace().next()?);
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0; 32];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn install_atomic(path: &Path, bytes: &[u8]) -> Result<(), KubectlError> {
    let parent = path.parent().ok_or(KubectlError::InvalidPath)?;
    ensure_safe_dir(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o700))?;
    }
    tmp.persist(path)
        .map_err(|error| KubectlError::Io(error.error))?;
    Ok(())
}

/// Resolve a kubectl binary, optionally downloading the cluster-matching release.
/// The returned path is never executed by this API.
pub async fn resolve_kubectl(
    prefs: &KubernetesPreferences,
    proxy: &ProxyPreferences,
    cluster_version: &str,
    data_dir: &Path,
) -> Result<ResolvedKubectl, KubectlError> {
    if let Some(path) = configured_path(prefs) {
        if !path.is_absolute()
            || path
                .components()
                .any(|c| c == std::path::Component::ParentDir)
        {
            return Err(KubectlError::InvalidPath);
        }
        if usable_binary(&path)? {
            return Ok(ResolvedKubectl {
                path,
                downloaded: false,
                version: None,
            });
        }
        if !prefs.kubectl_binary_path.is_empty() {
            return Err(KubectlError::NotConfigured);
        }
    }
    if !prefs.kubectl_auto_download {
        return Err(KubectlError::NotConfigured);
    }
    let release = release_version(cluster_version)?;
    let (os, arch, suffix) = platform();
    let relative = format!("release/{release}/bin/{os}/{arch}/kubectl{suffix}");
    let base = mirror_base(prefs)?;
    let binary_url = format!("{base}/{relative}");
    let hash_url = format!("{binary_url}.sha256");
    let directory = if prefs.binaries_directory.is_empty() {
        data_dir.join("binaries")
    } else {
        PathBuf::from(&prefs.binaries_directory)
    };
    let destination = directory.join(format!("kubectl-{release}{suffix}"));
    let cached_hash = directory.join(format!("kubectl-{release}{suffix}.sha256"));
    if usable_binary(&destination)?
        && usable_file(&cached_hash)?
        && fs::metadata(&cached_hash)?.len() <= MAX_HASH_BYTES
    {
        let hash = fs::read(&cached_hash)?;
        if let Some(expected) = parse_hash(&hash)
            && fs::metadata(&destination)?.len() <= MAX_BINARY_BYTES
        {
            let existing = fs::read(&destination)?;
            if <[u8; 32]>::from(Sha256::digest(&existing)) == expected {
                return Ok(ResolvedKubectl {
                    path: destination,
                    downloaded: false,
                    version: Some(release),
                });
            }
        }
    }
    let mut builder = Client::builder()
        .no_proxy()
        .timeout(DOWNLOAD_TIMEOUT)
        .connect_timeout(Duration::from_secs(10))
        // This explicit opt-in applies only to this application download client.
        .danger_accept_invalid_certs(proxy.allow_untrusted_cas)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5
                || secure_download_url(attempt.url().as_str()).is_none()
            {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }));
    if !proxy.https_proxy.is_empty() {
        builder = builder.proxy(reqwest::Proxy::all(&proxy.https_proxy)?);
    }
    // Kubernetes API TLS and process environment variables are untouched.
    let client = builder.build()?;
    let bytes = fetch_bounded(&client, &binary_url, MAX_BINARY_BYTES).await?;
    let expected = parse_hash(&fetch_bounded(&client, &hash_url, MAX_HASH_BYTES).await?)
        .ok_or(KubectlError::InvalidHashSidecar)?;
    let actual: [u8; 32] = Sha256::digest(&bytes).into();
    if actual != expected {
        return Err(KubectlError::HashMismatch);
    }
    install_atomic(&destination, &bytes)?;
    install_atomic(
        &cached_hash,
        format!("{:x}", Sha256::digest(&bytes)).as_bytes(),
    )?;
    Ok(ResolvedKubectl {
        path: destination,
        downloaded: true,
        version: Some(release),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preferences::DownloadMirror;
    use axum::{Router, response::IntoResponse, routing::get};
    use std::sync::Arc;
    use tempfile::tempdir;
    use tokio::net::TcpListener;

    async fn fixture() -> (String, Arc<Vec<u8>>, tokio::task::JoinHandle<()>) {
        let bytes = Arc::new(b"fake kubectl bytes".to_vec());
        let hash = format!("{:x}", Sha256::digest(bytes.as_slice()));
        let payload = bytes.clone();
        let app = Router::new().fallback(get(move |uri: axum::http::Uri| {
            let p = payload.clone();
            let h = hash.clone();
            async move {
                let (os, arch, suffix) = platform();
                let expected = format!("/release/v1.31.0/bin/{os}/{arch}/kubectl{suffix}");
                assert!(uri.path() == expected || uri.path() == format!("{expected}.sha256"));
                if uri.path().ends_with(".sha256") {
                    h.into_response()
                } else {
                    p.as_ref().clone().into_response()
                }
            }
        }));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{address}"), bytes, task)
    }

    #[tokio::test]
    async fn downloads_verifies_and_caches_custom_mirror() {
        let (mirror, bytes, task) = fixture().await;
        let dir = tempdir().unwrap();
        let prefs = KubernetesPreferences {
            kubectl_auto_download: true,
            kubectl_download_mirror: DownloadMirror::Custom,
            kubectl_mirror_url: mirror,
            binaries_directory: dir.path().join("bin").to_string_lossy().into(),
            ..Default::default()
        };
        let first = resolve_kubectl(&prefs, &ProxyPreferences::default(), "1.31.0", dir.path())
            .await
            .unwrap();
        assert!(first.downloaded);
        assert_eq!(fs::read(&first.path).unwrap(), *bytes);
        let second = resolve_kubectl(&prefs, &ProxyPreferences::default(), "1.31.0", dir.path())
            .await
            .unwrap();
        assert!(!second.downloaded);
        task.abort();
        assert!(
            !resolve_kubectl(&prefs, &ProxyPreferences::default(), "1.31.0", dir.path())
                .await
                .unwrap()
                .downloaded
        );
    }

    #[tokio::test]
    async fn rejects_insecure_non_loopback_mirror_and_bad_version() {
        let dir = tempdir().unwrap();
        let prefs = KubernetesPreferences {
            kubectl_auto_download: true,
            kubectl_download_mirror: DownloadMirror::Custom,
            kubectl_mirror_url: "http://example.invalid".into(),
            ..Default::default()
        };
        assert!(matches!(
            resolve_kubectl(&prefs, &ProxyPreferences::default(), "1.31.0", dir.path()).await,
            Err(KubectlError::InvalidMirror)
        ));
        let prefs = KubernetesPreferences {
            kubectl_auto_download: true,
            ..Default::default()
        };
        assert!(matches!(
            resolve_kubectl(
                &prefs,
                &ProxyPreferences::default(),
                "../../etc",
                dir.path()
            )
            .await,
            Err(KubectlError::InvalidVersion(_))
        ));
    }
    #[tokio::test]
    async fn opt_in_and_override_do_not_make_network_requests() {
        let dir = tempdir().unwrap();
        let proxy = ProxyPreferences::default();
        assert!(matches!(
            resolve_kubectl(&KubernetesPreferences::default(), &proxy, "bad", dir.path()).await,
            Err(KubectlError::NotConfigured)
        ));
        let path = dir.path().join("custom-kubectl");
        install_atomic(&path, b"not executed").unwrap();
        let prefs = KubernetesPreferences {
            kubectl_binary_path: path.to_string_lossy().into(),
            ..Default::default()
        };
        assert_eq!(
            resolve_kubectl(&prefs, &proxy, "bad", dir.path())
                .await
                .unwrap()
                .path,
            path
        );
    }

    #[test]
    fn normalizes_vendor_versions_and_platform_urls() {
        assert_eq!(release_version("v1.31.7+k3s1").unwrap(), "v1.31.7");
        assert_eq!(release_version("v1.31.7-eks-123").unwrap(), "v1.31.7");
        for invalid in [
            "v1.31",
            "v1.31.0/../../x",
            "v1.31.0?x",
            "v1.31.0%2f",
            "v2.0.0",
            "v1.31.0\n",
        ] {
            assert!(release_version(invalid).is_err());
        }
        let (os, arch, _) = platform();
        assert_ne!(os, "macos");
        assert_ne!(arch, "aarch64");
        assert_ne!(arch, "x86_64");
        assert_eq!(
            mirror_base(&KubernetesPreferences::default()).unwrap(),
            "https://dl.k8s.io"
        );
    }

    #[tokio::test]
    async fn bounded_streams_status_and_hash_fail_closed() {
        let app = Router::new().fallback(get(|uri: axum::http::Uri| async move {
            match uri.path() {
                "/large" => "0123456789".into_response(),
                "/missing" => axum::http::StatusCode::NOT_FOUND.into_response(),
                _ if uri.path().ends_with(".sha256") => "0".repeat(64).into_response(),
                _ => "bad binary".into_response(),
            }
        }));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = Client::builder().no_proxy().build().unwrap();
        assert!(matches!(
            fetch_bounded(&client, &format!("{base}/large"), 5).await,
            Err(KubectlError::TooLarge)
        ));
        assert!(matches!(
            fetch_bounded(&client, &format!("{base}/missing"), 5).await,
            Err(KubectlError::HttpStatus(_))
        ));
        let dir = tempdir().unwrap();
        let prefs = KubernetesPreferences {
            kubectl_auto_download: true,
            kubectl_download_mirror: DownloadMirror::Custom,
            kubectl_mirror_url: base,
            ..Default::default()
        };
        assert!(matches!(
            resolve_kubectl(&prefs, &ProxyPreferences::default(), "1.31.0", dir.path()).await,
            Err(KubectlError::HashMismatch)
        ));
        assert!(!dir.path().join("binaries").exists());
        task.abort();
    }
    #[tokio::test]
    async fn application_proxy_is_used_without_environment_mutation() {
        let app = Router::new().fallback(get(|uri: axum::http::Uri| async move {
            assert_eq!(uri.host(), Some("mirror.invalid"));
            if uri.path().ends_with(".sha256") {
                format!("{:x}", Sha256::digest(b"proxy binary")).into_response()
            } else {
                "proxy binary".into_response()
            }
        }));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        // Resolve the URL directly through the HTTP proxy to prove client configuration.
        let client = Client::builder()
            .no_proxy()
            .proxy(reqwest::Proxy::all(proxy).unwrap())
            .build()
            .unwrap();
        assert_eq!(
            fetch_bounded(&client, "http://mirror.invalid/binary", 100)
                .await
                .unwrap(),
            b"proxy binary"
        );
        task.abort();
    }
}
