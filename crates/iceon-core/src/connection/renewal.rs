use super::{ConnectionError, Result, auth, config};
use http::{Method, Request, Response};
use kube::{Client, client::Body, config::Kubeconfig};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::Mutex;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

const REFRESH_MARGIN: i64 = 30;
const FAILURE_BACKOFF: Duration = Duration::from_secs(5);

struct State {
    client: Client,
    generation: u64,
    failure: Option<(tokio::time::Instant, ConnectionError)>,
}
pub(super) fn authorization_error(error: ConnectionError) -> tower::BoxError {
    box_error(error)
}
struct Lease {
    state: Mutex<State>,
    source: Kubeconfig,
    base: PathBuf,
    tasks: TaskTracker,
    cancel: CancellationToken,
    authentication_generation: super::permissions::AuthenticationGeneration,
}

pub(super) fn wrap(
    client: Client,
    source: Kubeconfig,
    base: PathBuf,
    tasks: TaskTracker,
    cancel: CancellationToken,
    authentication_generation: super::permissions::AuthenticationGeneration,
) -> Client {
    super::permissions::invalidate_authentication(&authentication_generation);
    let namespace = client.default_namespace().to_owned();
    let lease = Arc::new(Lease {
        state: Mutex::new(State {
            client,
            generation: 0,
            failure: None,
        }),
        source,
        base,
        tasks,
        cancel,
        authentication_generation,
    });
    // The outer client has no fixed deadline: the lease checks its current inner client.
    Client::new(
        tower::service_fn(move |request: Request<Body>| {
            let lease = lease.clone();
            async move { lease.send(request).await }
        }),
        namespace,
    )
}

impl Lease {
    async fn client(&self, rejected: Option<u64>) -> Result<(Client, u64)> {
        let mut state = self.state.lock().await;
        if self.cancel.is_cancelled() {
            return Err(ConnectionError::StaleSession);
        }
        let expiring = state.client.valid_until().is_some_and(|expiry| {
            expiry.as_second()
                <= jiff::Timestamp::now()
                    .as_second()
                    .saturating_add(REFRESH_MARGIN)
        });
        if !expiring && rejected != Some(state.generation) {
            return Ok((state.client.clone(), state.generation));
        }
        if let Some((at, error)) = state.failure
            && at.elapsed() < FAILURE_BACKOFF
        {
            return Err(error);
        }
        let mut source = self.source.clone();
        let refresh = async {
            let expiry = auth::resolve_tracked(&mut source, &self.base, &self.tasks).await?;
            let context = source
                .current_context
                .clone()
                .ok_or(ConnectionError::KubeconfigInvalid)?;
            config::materialize(source, context, expiry).await
        };
        let result = tokio::select! {
            biased;
            _ = self.cancel.cancelled() => Err(ConnectionError::StaleSession),
            result = tokio::time::timeout(super::DEADLINE, refresh) => result.unwrap_or(Err(ConnectionError::Timeout)),
        };
        match result {
            Ok(client) => {
                state.client = client;
                state.generation += 1;
                super::permissions::invalidate_authentication(&self.authentication_generation);
                state.failure = None;
                Ok((state.client.clone(), state.generation))
            }
            Err(error) => {
                state.failure = Some((tokio::time::Instant::now(), error));
                Err(error)
            }
        }
    }

    async fn send(
        &self,
        request: Request<Body>,
    ) -> std::result::Result<Response<Body>, tower::BoxError> {
        let request_cancel = request
            .extensions()
            .get::<CancellationToken>()
            .cloned()
            .unwrap_or_default();
        tokio::select! {
            biased;
            _ = self.cancel.cancelled() => Err(box_error(ConnectionError::StaleSession)),
            _ = request_cancel.cancelled() => Err(box_error(ConnectionError::StaleSession)),
            result = self.send_inner(request) => result,
        }
    }

    async fn send_inner(
        &self,
        request: Request<Body>,
    ) -> std::result::Result<Response<Body>, tower::BoxError> {
        // GET upgrades may establish exec/attach/forward sessions. Never replay them.
        let retry = replay(&request);
        let (client, generation) = self.client(None).await.map_err(box_error)?;
        if let Some(epoch) = request
            .extensions()
            .get::<super::permissions::AuthorizationEpoch>()
        {
            epoch.validate().map_err(box_error)?;
        }
        let response = client.send(request).await?;
        if response.status() != http::StatusCode::UNAUTHORIZED {
            return Ok(response);
        }
        let Some(retry) = retry else {
            return Ok(response);
        };
        drop(response);
        let (client, _) = self.client(Some(generation)).await.map_err(box_error)?;
        if let Some(epoch) = retry
            .extensions()
            .get::<super::permissions::AuthorizationEpoch>()
        {
            epoch.validate().map_err(box_error)?;
        }
        client.send(retry).await.map_err(Into::into)
    }
}

fn replay(request: &Request<Body>) -> Option<Request<Body>> {
    if !matches!(*request.method(), Method::GET | Method::HEAD)
        || request.headers().contains_key(http::header::UPGRADE)
        || request
            .uri()
            .path()
            .split('/')
            .any(|part| matches!(part, "exec" | "attach" | "portforward"))
    {
        return None;
    }
    let mut retry = Request::new(request.body().try_clone()?);
    *retry.method_mut() = request.method().clone();
    *retry.uri_mut() = request.uri().clone();
    *retry.version_mut() = request.version();
    *retry.headers_mut() = request.headers().clone();
    *retry.extensions_mut() = request.extensions().clone();
    Some(retry)
}

#[derive(Debug)]
pub(super) struct RenewalError(pub(super) ConnectionError);
impl std::fmt::Display for RenewalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "credential renewal: {:?}", self.0)
    }
}
impl std::error::Error for RenewalError {}
fn box_error(error: ConnectionError) -> tower::BoxError {
    Box::new(RenewalError(error))
}

#[cfg(test)]
#[path = "renewal_tests.rs"]
mod tests;
