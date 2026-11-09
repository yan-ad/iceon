use super::{ConnectionError, Result};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use jiff::Timestamp;
use kube::config::{
    AuthInfo, AuthProviderConfig, Cluster, ExecConfig, ExecInteractiveMode, Kubeconfig,
};
use process_wrap::tokio::{CommandWrap, KillOnDrop};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio_util::sync::CancellationToken;

#[cfg(any(target_os = "macos", test))]
#[path = "auth_path.rs"]
mod auth_path;

const STDOUT_LIMIT: usize = 1024 * 1024;
const STDERR_LIMIT: usize = 64 * 1024;
const CONFIG_LIMIT: usize = 1024 * 1024;
const TOKEN_LIMIT: usize = 64 * 1024;
const PEM_LIMIT: usize = 512 * 1024;
const HELPER_DEADLINE: Duration = Duration::from_secs(15);
const EXPIRY_MARGIN_SECONDS: i64 = 30;
const V1: &str = "client.authentication.k8s.io/v1";
const BETA: &str = "client.authentication.k8s.io/v1beta1";
const EXEC_INFO: &str = "KUBERNETES_EXEC_INFO";

pub(super) fn renewable(config: &Kubeconfig) -> bool {
    config.auth_infos.iter().any(|user| {
        user.auth_info.as_ref().is_some_and(|auth| {
            auth.auth_provider.is_none() && (auth.exec.is_some() || auth.token_file.is_some())
        })
    })
}

/// Resolve only the selected user; no kube automatic exec/provider runner survives this boundary.
#[cfg(test)]
pub(super) async fn resolve(config: &mut Kubeconfig, base: &Path) -> Result<Option<Timestamp>> {
    resolve_tracked(config, base, &tokio_util::task::TaskTracker::new()).await
}

pub(super) async fn resolve_tracked(
    config: &mut Kubeconfig,
    base: &Path,
    tasks: &tokio_util::task::TaskTracker,
) -> Result<Option<Timestamp>> {
    let context_name = config
        .current_context
        .as_ref()
        .ok_or(ConnectionError::KubeconfigInvalid)?;
    let mut contexts = config
        .contexts
        .iter()
        .filter(|entry| &entry.name == context_name);
    let context = contexts
        .next()
        .and_then(|entry| entry.context.as_ref())
        .ok_or(ConnectionError::KubeconfigInvalid)?;
    if contexts.next().is_some() {
        return Err(ConnectionError::KubeconfigInvalid);
    }
    let Some(user) = context.user.as_ref().filter(|user| !user.is_empty()) else {
        return Ok(None);
    };
    let mut users = config
        .auth_infos
        .iter()
        .enumerate()
        .filter(|(_, entry)| &entry.name == user);
    let (index, entry) = users.next().ok_or(ConnectionError::KubeconfigInvalid)?;
    if users.next().is_some() {
        return Err(ConnectionError::KubeconfigInvalid);
    }
    let auth = entry
        .auth_info
        .as_ref()
        .ok_or(ConnectionError::KubeconfigInvalid)?;
    let credentials = match (&auth.exec, &auth.auth_provider) {
        (Some(_), Some(_)) => return Err(ConnectionError::UnsupportedAuth),
        (Some(exec), None) => {
            let mut clusters = config
                .clusters
                .iter()
                .filter(|entry| entry.name == context.cluster);
            let cluster = clusters
                .next()
                .and_then(|entry| entry.cluster.as_ref())
                .ok_or(ConnectionError::KubeconfigInvalid)?;
            if clusters.next().is_some() {
                return Err(ConnectionError::KubeconfigInvalid);
            }
            let request = helper_request(exec, cluster, base)?;
            let bytes = run_tracked(request, HELPER_DEADLINE, tasks).await?;
            parse_response(&bytes, exec.api_version.as_deref().unwrap_or_default())?
        }
        (None, Some(provider)) => cached_provider(provider)?,
        (None, None) => {
            let Some(file) = auth.token_file.as_ref() else {
                return Ok(None);
            };
            let path = base.join(file);
            let bytes = tokio::task::spawn_blocking(move || {
                let path = std::fs::canonicalize(path)
                    .map_err(|_| crate::catalog::CatalogError::InputUnavailable)?;
                crate::catalog::bounded_read(&path, TOKEN_LIMIT)
            })
            .await
            .map_err(|_| ConnectionError::KubeconfigUnavailable)?
            .map_err(super::map_catalog_error)?;
            let token = std::str::from_utf8(&bytes)
                .map_err(|_| ConnectionError::KubeconfigInvalid)?
                .trim();
            validate_token(token)?;
            let token = token.to_owned();
            let auth = config.auth_infos[index]
                .auth_info
                .as_mut()
                .ok_or(ConnectionError::KubeconfigInvalid)?;
            auth.token = Some(token.into());
            auth.token_file = None;
            return Ok(None);
        }
    };
    let expiry = credentials.expiry;
    let auth = config.auth_infos[index]
        .auth_info
        .as_mut()
        .ok_or(ConnectionError::KubeconfigInvalid)?;
    apply(auth, credentials);
    Ok(expiry)
}

struct Credentials {
    token: Option<String>,
    certificate: Option<String>,
    key: Option<String>,
    expiry: Option<Timestamp>,
}

fn apply(auth: &mut AuthInfo, credentials: Credentials) {
    // Plugin output wins deterministically over every static credential, not over impersonation.
    auth.exec = None;
    auth.auth_provider = None;
    auth.username = None;
    auth.password = None;
    auth.token_file = None;
    auth.client_certificate = None;
    auth.client_key = None;
    auth.token = credentials.token.map(Into::into);
    auth.client_certificate_data = credentials.certificate.map(|pem| STANDARD.encode(pem));
    auth.client_key_data = credentials.key.map(|pem| STANDARD.encode(pem).into());
}

struct HelperRequest {
    command: PathBuf,
    args: Vec<String>,
    env: Vec<(String, String)>,
    info: String,
    #[cfg(any(target_os = "macos", test))]
    fallback_home: Option<PathBuf>,
}

fn helper_request(exec: &ExecConfig, cluster: &Cluster, base: &Path) -> Result<HelperRequest> {
    let version = exec
        .api_version
        .as_deref()
        .ok_or(ConnectionError::KubeconfigInvalid)?;
    if !matches!(version, V1 | BETA) {
        return Err(ConnectionError::KubeconfigInvalid);
    }
    match &exec.interactive_mode {
        Some(ExecInteractiveMode::Always) => return Err(ConnectionError::AuthInteractiveRequired),
        None if version == V1 => return Err(ConnectionError::KubeconfigInvalid),
        _ => {}
    }
    let command = exec
        .command
        .as_deref()
        .filter(|command| !command.is_empty())
        .ok_or(ConnectionError::KubeconfigInvalid)?;
    let mut size = command.len();
    if command.contains('\0') || size > CONFIG_LIMIT {
        return Err(ConnectionError::KubeconfigInvalid);
    }
    let path = PathBuf::from(command);
    // Relative paths stay relative to the canonical kubeconfig directory.
    let command = if path.is_relative() && (command.contains('/') || command.contains('\\')) {
        base.join(path)
    } else {
        path
    };
    #[cfg(windows)]
    let command = {
        // Rust's Windows process API may implicitly invoke cmd.exe for batch files.
        // Require an explicit native executable, including for PATH lookup.
        let mut command = command;
        if command.extension().is_none() {
            command.set_extension("exe");
        } else if !command
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
        {
            return Err(ConnectionError::UnsupportedAuth);
        }
        command
    };
    let args = exec.args.clone().unwrap_or_default();
    if args.len() > 4096 {
        return Err(ConnectionError::LimitExceeded);
    }
    for arg in &args {
        size = size.saturating_add(arg.len());
        if arg.contains('\0') {
            return Err(ConnectionError::KubeconfigInvalid);
        }
    }
    let mut env = Vec::new();
    for entry in exec.env.iter().flatten() {
        if env.len() >= 1024 {
            return Err(ConnectionError::LimitExceeded);
        }
        let name = entry
            .get("name")
            .ok_or(ConnectionError::KubeconfigInvalid)?;
        let value = entry
            .get("value")
            .ok_or(ConnectionError::KubeconfigInvalid)?;
        if entry.len() != 2
            || name.is_empty()
            || name.contains(['=', '\0'])
            || value.contains('\0')
            || name.eq_ignore_ascii_case(EXEC_INFO)
            || env
                .iter()
                .any(|(existing, _): &(String, String)| existing.eq_ignore_ascii_case(name))
        {
            return Err(ConnectionError::KubeconfigInvalid);
        }
        size = size.saturating_add(name.len()).saturating_add(value.len());
        env.push((name.clone(), value.clone()));
    }
    if size > CONFIG_LIMIT {
        return Err(ConnectionError::LimitExceeded);
    }
    let mut spec = json!({"interactive": false});
    if exec.provide_cluster_info {
        // Construct explicitly: kube's TryFrom<&Cluster> can read credential files.
        if cluster.certificate_authority.is_some() {
            return Err(ConnectionError::KubeconfigInvalid);
        }
        let mut info = json!({});
        for (name, value) in [
            ("server", cluster.server.as_ref()),
            ("tls-server-name", cluster.tls_server_name.as_ref()),
            (
                "certificate-authority-data",
                cluster.certificate_authority_data.as_ref(),
            ),
        ] {
            if let Some(value) = value {
                info[name] = json!(value);
            }
        }
        if let Some(value) = cluster.insecure_skip_tls_verify {
            info["insecure-skip-tls-verify"] = json!(value);
        }
        if let Some(value) = cluster.disable_compression {
            info["disable-compression"] = json!(value);
        }
        if let Some(extension) = cluster
            .extensions
            .iter()
            .flatten()
            .find(|extension| extension.name == "client.authentication.k8s.io/exec")
        {
            info["config"] = extension.extension.clone();
        }
        spec["cluster"] = info;
    }
    let info = serde_json::to_string(
        &json!({"apiVersion": version, "kind": "ExecCredential", "spec": spec}),
    )
    .map_err(|_| ConnectionError::KubeconfigInvalid)?;
    if size.saturating_add(info.len()) > CONFIG_LIMIT {
        return Err(ConnectionError::LimitExceeded);
    }
    #[cfg(any(target_os = "macos", test))]
    let fallback_home = if cfg!(target_os = "macos") && !env.iter().any(|(name, _)| name == "PATH")
    {
        std::env::var_os("HOME").map(PathBuf::from)
    } else {
        None
    };
    #[cfg(target_os = "macos")]
    auth_path::configure(
        &mut env,
        std::env::var_os("PATH").as_deref(),
        std::env::var_os("HOME").as_deref().map(Path::new),
    )?;
    Ok(HelperRequest {
        command,
        args,
        env,
        info,
        #[cfg(any(target_os = "macos", test))]
        fallback_home,
    })
}

// Dropping the caller's future signals, rather than aborts, its independently owned reaper.
struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

#[cfg(test)]
async fn run(request: HelperRequest, deadline: Duration) -> Result<Vec<u8>> {
    run_tracked(request, deadline, &tokio_util::task::TaskTracker::new()).await
}

async fn run_tracked(
    request: HelperRequest,
    deadline: Duration,
    tasks: &tokio_util::task::TaskTracker,
) -> Result<Vec<u8>> {
    let cancel = CancellationToken::new();
    let guard = CancelOnDrop(cancel.clone());
    let worker = tasks.spawn(supervise(request, deadline, cancel));
    let result = worker
        .await
        .map_err(|_| ConnectionError::AuthHelperFailed)?;
    drop(guard);
    result
}

async fn supervise(
    request: HelperRequest,
    deadline: Duration,
    cancel: CancellationToken,
) -> Result<Vec<u8>> {
    #[cfg(any(target_os = "macos", test))]
    let mut request = request;
    let expires = tokio::time::Instant::now() + deadline;
    if cancel.is_cancelled() {
        return Err(ConnectionError::AuthHelperFailed);
    }
    #[cfg(any(target_os = "macos", test))]
    if request.fallback_home.is_some()
        && let Some(path) = auth_path::resolve(&request.command, &request.env)
    {
        request.command = path;
    }
    if cancel.is_cancelled() {
        return Err(ConnectionError::AuthHelperFailed);
    }
    if tokio::time::Instant::now() >= expires {
        return Err(ConnectionError::Timeout);
    }
    let mut child = match spawn_helper(&request) {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            #[cfg(any(target_os = "macos", test))]
            {
                let home = request.fallback_home.take();
                if home.is_none() || auth_path::bare(&request.command).is_none() {
                    return Err(ConnectionError::AuthHelperMissing);
                }
                let worker_cancel = cancel.clone();
                let worker = tokio::task::spawn_blocking(move || {
                    if let Some(home) = home {
                        auth_path::fallback(
                            &mut request.command,
                            &mut request.env,
                            &home,
                            Path::new("/etc"),
                            std::env::var_os("XDG_CONFIG_HOME")
                                .as_deref()
                                .map(Path::new),
                            || {
                                worker_cancel.is_cancelled()
                                    || tokio::time::Instant::now() >= expires
                            },
                        )?;
                    }
                    Ok::<_, ConnectionError>(request)
                });
                request = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return Err(ConnectionError::AuthHelperFailed),
                    result = tokio::time::timeout_at(expires, worker) => result.map_err(|_| ConnectionError::Timeout)?.map_err(|_| ConnectionError::AuthHelperFailed)??,
                };
                if cancel.is_cancelled() {
                    return Err(ConnectionError::AuthHelperFailed);
                }
                if tokio::time::Instant::now() >= expires {
                    return Err(ConnectionError::Timeout);
                }
                if !request.command.is_absolute() {
                    return Err(ConnectionError::AuthHelperMissing);
                }
                spawn_helper(&request).map_err(spawn_error)?
            }
            #[cfg(not(any(target_os = "macos", test)))]
            return Err(spawn_error(error));
        }
        Err(error) => return Err(spawn_error(error)),
    };
    let stdout = child
        .stdout()
        .take()
        .ok_or(ConnectionError::AuthHelperFailed)?;
    let stderr = child
        .stderr()
        .take()
        .ok_or(ConnectionError::AuthHelperFailed)?;
    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(ConnectionError::AuthHelperFailed),
        result = tokio::time::timeout_at(expires, async {
            let (status, output, _) = tokio::try_join!(
                async { child.wait().await.map_err(|_| ConnectionError::AuthHelperFailed) },
                read_capped(stdout, STDOUT_LIMIT, true),
                read_capped(stderr, STDERR_LIMIT, false),
            )?;
            if !status.success() { return Err(ConnectionError::AuthHelperFailed); }
            Ok(output)
        }) => result.unwrap_or(Err(ConnectionError::Timeout)),
    };
    let _ = child.start_kill();
    let _ = child.wait().await;
    result
}

fn spawn_helper(
    request: &HelperRequest,
) -> std::io::Result<Box<dyn process_wrap::tokio::ChildWrapper>> {
    let mut command = CommandWrap::with_new(&request.command, |command| {
        command
            .args(&request.args)
            .envs(request.env.iter().map(|(name, value)| (name, value)))
            .env(EXEC_INFO, &request.info)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
    });
    command.wrap(KillOnDrop);
    #[cfg(unix)]
    command.wrap(process_wrap::tokio::ProcessGroup::leader());
    #[cfg(windows)]
    command.wrap(process_wrap::tokio::JobObject);
    command.spawn()
}

fn spawn_error(error: std::io::Error) -> ConnectionError {
    if error.kind() == std::io::ErrorKind::NotFound {
        ConnectionError::AuthHelperMissing
    } else {
        ConnectionError::AuthHelperFailed
    }
}

async fn read_capped(
    mut reader: impl AsyncRead + Unpin,
    limit: usize,
    retain: bool,
) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut total = 0usize;
    let mut buffer = [0u8; 8192];
    loop {
        let count = reader
            .read(&mut buffer)
            .await
            .map_err(|_| ConnectionError::AuthHelperFailed)?;
        if count == 0 {
            return Ok(output);
        }
        total += count;
        if total > limit {
            return Err(ConnectionError::LimitExceeded);
        }
        if retain {
            output.extend_from_slice(&buffer[..count]);
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecResponse {
    api_version: String,
    kind: String,
    status: ExecStatus,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecStatus {
    token: Option<String>,
    client_certificate_data: Option<String>,
    client_key_data: Option<String>,
    expiration_timestamp: Option<String>,
}

fn parse_response(bytes: &[u8], version: &str) -> Result<Credentials> {
    if bytes.len() > STDOUT_LIMIT {
        return Err(ConnectionError::LimitExceeded);
    }
    let response: ExecResponse =
        serde_json::from_slice(bytes).map_err(|_| ConnectionError::AuthHelperInvalidResponse)?;
    if response.kind != "ExecCredential"
        || response.api_version != version
        || !matches!(version, V1 | BETA)
    {
        return Err(ConnectionError::AuthHelperInvalidResponse);
    }
    let status = response.status;
    if let Some(token) = &status.token {
        validate_token(token)?;
    }
    match (&status.client_certificate_data, &status.client_key_data) {
        (Some(certificate), Some(key)) => validate_pem(certificate, key)?,
        (None, None) => {}
        _ => return Err(ConnectionError::AuthHelperInvalidResponse),
    }
    if status.token.is_none() && status.client_certificate_data.is_none() {
        return Err(ConnectionError::AuthHelperInvalidResponse);
    }
    let expiry = status
        .expiration_timestamp
        .as_deref()
        .map(parse_expiry)
        .transpose()?;
    Ok(Credentials {
        token: status.token,
        certificate: status.client_certificate_data,
        key: status.client_key_data,
        expiry,
    })
}

fn validate_token(token: &str) -> Result<()> {
    if token.len() > TOKEN_LIMIT {
        return Err(ConnectionError::LimitExceeded);
    }
    if token.is_empty()
        || !token.bytes().all(|byte| byte.is_ascii_graphic())
        || http::HeaderValue::from_str(&format!("Bearer {token}")).is_err()
    {
        return Err(ConnectionError::AuthHelperInvalidResponse);
    }
    Ok(())
}

fn validate_pem(certificate: &str, key: &str) -> Result<()> {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
    if certificate.len() > PEM_LIMIT || key.len() > PEM_LIMIT {
        return Err(ConnectionError::LimitExceeded);
    }
    if certificate.contains('\0') || key.contains('\0') {
        return Err(ConnectionError::AuthHelperInvalidResponse);
    }
    let certs = CertificateDer::pem_slice_iter(certificate.as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| ConnectionError::AuthHelperInvalidResponse)?;
    if certs.is_empty() {
        return Err(ConnectionError::AuthHelperInvalidResponse);
    }
    let key = PrivateKeyDer::from_pem_slice(key.as_bytes())
        .map_err(|_| ConnectionError::AuthHelperInvalidResponse)?;
    rustls::sign::CertifiedKey::from_der(certs, key, &rustls::crypto::ring::default_provider())
        .map_err(|_| ConnectionError::AuthHelperInvalidResponse)?;
    Ok(())
}

fn parse_expiry(text: &str) -> Result<Timestamp> {
    // Timestamp's parser also accepts temporal annotations. The plugin contract only accepts RFC3339.
    let bytes = text.as_bytes();
    let valid = bytes.len() >= 20
        && bytes.get(4) == Some(&b'-')
        && bytes.get(7) == Some(&b'-')
        && matches!(bytes.get(10), Some(b'T' | b't'))
        && bytes.get(13) == Some(&b':')
        && bytes.get(16) == Some(&b':')
        && bytes[..19]
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7 | 10 | 13 | 16) || byte.is_ascii_digit());
    if !valid {
        return Err(ConnectionError::AuthHelperInvalidResponse);
    }
    let mut index = 19;
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == start {
            return Err(ConnectionError::AuthHelperInvalidResponse);
        }
    }
    let zone = &bytes[index..];
    if !(matches!(zone, [b'Z' | b'z'])
        || (zone.len() == 6
            && matches!(zone[0], b'+' | b'-')
            && zone[3] == b':'
            && [1, 2, 4, 5]
                .iter()
                .all(|index| zone[*index].is_ascii_digit())))
    {
        return Err(ConnectionError::AuthHelperInvalidResponse);
    }
    let expiry = text
        .parse::<Timestamp>()
        .map_err(|_| ConnectionError::AuthHelperInvalidResponse)?;
    validate_expiry(expiry)?;
    Ok(expiry)
}

fn validate_expiry(expiry: Timestamp) -> Result<()> {
    if expiry.as_second()
        <= Timestamp::now()
            .as_second()
            .saturating_add(EXPIRY_MARGIN_SECONDS)
    {
        Err(ConnectionError::AuthExpired)
    } else {
        Ok(())
    }
}

fn cached_provider(provider: &AuthProviderConfig) -> Result<Credentials> {
    // Cached credentials only: never execute legacy cmd-path, refresh-token, or provider refresh flows.
    let get = |name: &str| {
        provider
            .config
            .get(name)
            .map(String::as_str)
            .ok_or(ConnectionError::AuthFailed)
    };
    let (token, expiry) = match provider.name.as_str() {
        "oidc" => {
            let token = get("id-token")?;
            validate_token(token)?;
            let mut parts = token.split('.');
            if parts.next().is_none_or(str::is_empty) {
                return Err(ConnectionError::AuthHelperInvalidResponse);
            }
            let payload = parts
                .next()
                .ok_or(ConnectionError::AuthHelperInvalidResponse)?;
            if parts.next().is_none_or(str::is_empty) || parts.next().is_some() {
                return Err(ConnectionError::AuthHelperInvalidResponse);
            }
            let payload = URL_SAFE_NO_PAD
                .decode(payload)
                .map_err(|_| ConnectionError::AuthHelperInvalidResponse)?;
            let value: Value = serde_json::from_slice(&payload)
                .map_err(|_| ConnectionError::AuthHelperInvalidResponse)?;
            let seconds = value
                .get("exp")
                .and_then(Value::as_i64)
                .ok_or(ConnectionError::AuthHelperInvalidResponse)?;
            let expiry = Timestamp::from_second(seconds)
                .map_err(|_| ConnectionError::AuthHelperInvalidResponse)?;
            validate_expiry(expiry)?;
            (token, expiry)
        }
        "gcp" => (get("access-token")?, parse_expiry(get("expiry")?)?),
        "azure" => {
            let seconds = get("expires-on")?
                .parse::<i64>()
                .map_err(|_| ConnectionError::AuthHelperInvalidResponse)?;
            let expiry = Timestamp::from_second(seconds)
                .map_err(|_| ConnectionError::AuthHelperInvalidResponse)?;
            validate_expiry(expiry)?;
            (get("access-token")?, expiry)
        }
        _ => return Err(ConnectionError::UnsupportedAuth),
    };
    validate_token(token)?;
    Ok(Credentials {
        token: Some(token.to_owned()),
        certificate: None,
        key: None,
        expiry: Some(expiry),
    })
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;
