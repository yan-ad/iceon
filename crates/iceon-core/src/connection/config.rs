use super::{ConnectionError, ConnectionTarget, DEADLINE, Result};
use crate::catalog::bounded_read;
use base64::{Engine, engine::general_purpose::STANDARD};
use kube::{
    Client, Config,
    config::{KubeConfigOptions, Kubeconfig},
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

const FILE_LIMIT: usize = 1024 * 1024;

#[cfg(test)]
pub(super) async fn build_client(
    target: ConnectionTarget,
    admission: Arc<tokio::sync::Semaphore>,
    auth_tasks: tokio_util::task::TaskTracker,
) -> Result<Client> {
    build_client_owned(
        target,
        admission,
        auth_tasks,
        tokio_util::sync::CancellationToken::new(),
        Arc::new(std::sync::atomic::AtomicU64::new(1)),
    )
    .await
}

pub(super) async fn build_client_owned(
    target: ConnectionTarget,
    admission: Arc<tokio::sync::Semaphore>,
    auth_tasks: tokio_util::task::TaskTracker,
    cancel: tokio_util::sync::CancellationToken,
    authentication_generation: super::permissions::AuthenticationGeneration,
) -> Result<Client> {
    let context = target.context_name.clone();
    let base = target
        .source_path
        .parent()
        .ok_or(ConnectionError::KubeconfigUnavailable)?
        .to_path_buf();
    let permit = admission
        .try_acquire_owned()
        .map_err(|_| ConnectionError::Busy)?;
    // Blocking preparation has a fixed input bound and never executes plugins or opens sockets.
    let mut config = crate::runtime::spawn_blocking(move || {
        let _permit = permit;
        prepare_inner(&target, true)
    })
    .await
    .map_err(|_| ConnectionError::KubeconfigInvalid)??;
    let source = config.clone();
    let renewable = super::auth::renewable(&source);
    let expiry = super::auth::resolve_tracked(&mut config, &base, &auth_tasks).await?;
    let client = materialize(config, context, expiry).await?;
    if renewable {
        Ok(super::renewal::wrap(
            client,
            source,
            base,
            auth_tasks,
            cancel,
            authentication_generation,
        ))
    } else {
        Ok(client)
    }
}

pub(super) async fn materialize(
    config: Kubeconfig,
    context: String,
    expiry: Option<jiff::Timestamp>,
) -> Result<Client> {
    let mut config = Config::from_custom_kubeconfig(
        config,
        &KubeConfigOptions {
            context: Some(context),
            ..Default::default()
        },
    )
    .await
    .map_err(|_| ConnectionError::KubeconfigInvalid)?;
    config.connect_timeout = Some(DEADLINE);
    // Snapshots have engine deadlines; log/exec streams must survive quiet periods.
    config.read_timeout = None;
    config.write_timeout = Some(DEADLINE);
    config.default_retry = false;
    // Proxy processes/tunnels are outside this slice, including inherited proxy URLs.
    config.proxy_url = None;
    Ok(kube::client::ClientBuilder::try_from(config)
        .map_err(|_| ConnectionError::TlsFailed)?
        .with_layer(&tower::layer::layer_fn(super::transport::CancelService))
        .with_valid_until(expiry)
        .build())
}

pub(super) fn prepare(target: &ConnectionTarget) -> Result<Kubeconfig> {
    prepare_inner(target, false)
}

fn prepare_inner(target: &ConnectionTarget, retain_token_file: bool) -> Result<Kubeconfig> {
    let canonical = fs::canonicalize(&target.provenance_path)
        .map_err(|_| ConnectionError::KubeconfigUnavailable)?;
    if canonical != target.source_path {
        return Err(ConnectionError::SourceChanged);
    }
    let bytes = bounded_read(&canonical, FILE_LIMIT).map_err(super::map_catalog_error)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| ConnectionError::KubeconfigInvalid)?;
    let mut config = Kubeconfig::from_yaml(text).map_err(|_| ConnectionError::KubeconfigInvalid)?;
    if config.kind.as_deref() != Some("Config") || config.api_version.as_deref() != Some("v1") {
        return Err(ConnectionError::KubeconfigInvalid);
    }
    let contexts = config
        .contexts
        .iter()
        .filter(|c| c.name == target.context_name)
        .collect::<Vec<_>>();
    if contexts.len() != 1 {
        return Err(ConnectionError::SourceChanged);
    }
    let context = contexts[0]
        .context
        .as_ref()
        .ok_or(ConnectionError::KubeconfigInvalid)?
        .clone();
    if config
        .clusters
        .iter()
        .filter(|c| c.name == context.cluster)
        .count()
        != 1
    {
        return Err(ConnectionError::KubeconfigInvalid);
    }
    config.contexts.retain(|c| c.name == target.context_name);
    config.clusters.retain(|c| c.name == context.cluster);
    config
        .auth_infos
        .retain(|u| Some(&u.name) == context.user.as_ref());
    if context.user.as_ref().is_some_and(|u| !u.is_empty()) && config.auth_infos.len() != 1 {
        return Err(ConnectionError::KubeconfigInvalid);
    }
    let base = canonical
        .parent()
        .ok_or(ConnectionError::KubeconfigUnavailable)?;
    let cluster = config.clusters[0]
        .cluster
        .as_mut()
        .ok_or(ConnectionError::KubeconfigInvalid)?;
    let server = url::Url::parse(
        cluster
            .server
            .as_deref()
            .ok_or(ConnectionError::KubeconfigInvalid)?,
    )
    .map_err(|_| ConnectionError::KubeconfigInvalid)?;
    if server.scheme() != "https"
        || server.host_str().is_none()
        || !server.username().is_empty()
        || server.password().is_some()
        || server.query().is_some()
        || server.fragment().is_some()
        || cluster.insecure_skip_tls_verify == Some(true)
    {
        return Err(ConnectionError::InsecureTransport);
    }
    if cluster.proxy_url.is_some() {
        return Err(ConnectionError::InsecureTransport);
    }
    inline_file(
        base,
        &mut cluster.certificate_authority,
        &mut cluster.certificate_authority_data,
    )?;
    for user in &mut config.auth_infos {
        let auth = user
            .auth_info
            .as_mut()
            .ok_or(ConnectionError::KubeconfigInvalid)?;
        inline_file(
            base,
            &mut auth.client_certificate,
            &mut auth.client_certificate_data,
        )?;
        if auth.client_key_data.is_none()
            && let Some(file) = auth.client_key.take()
        {
            auth.client_key_data = Some(STANDARD.encode(read_reference(base, &file)?).into());
        } else {
            auth.client_key = None;
        }
        if auth.token.is_none()
            && let Some(file) = auth.token_file.take()
        {
            let bytes = read_reference(base, &file)?;
            let token = std::str::from_utf8(&bytes)
                .map_err(|_| ConnectionError::KubeconfigInvalid)?
                .trim();
            if token.is_empty() || token.chars().any(char::is_control) {
                return Err(ConnectionError::KubeconfigInvalid);
            }
            auth.token = Some(token.to_owned().into());
            if retain_token_file {
                auth.token_file = Some(base.join(file).to_string_lossy().into_owned());
            }
        } else {
            auth.token_file = None;
        }
    }
    config.current_context = Some(target.context_name.clone());
    Ok(config)
}
fn inline_file(base: &Path, file: &mut Option<String>, data: &mut Option<String>) -> Result<()> {
    if data.is_none()
        && let Some(path) = file.take()
    {
        *data = Some(STANDARD.encode(read_reference(base, &path)?));
    } else {
        *file = None;
    }
    Ok(())
}
fn read_reference(base: &Path, file: &str) -> Result<Vec<u8>> {
    let path = PathBuf::from(file);
    let resolved = fs::canonicalize(if path.is_absolute() {
        path
    } else {
        base.join(path)
    })
    .map_err(|_| ConnectionError::KubeconfigUnavailable)?;
    bounded_read(&resolved, FILE_LIMIT).map_err(super::map_catalog_error)
}
