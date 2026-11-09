use super::*;
use http::StatusCode;
use kube::config::{AuthInfo, Cluster, Context, NamedAuthInfo, NamedCluster, NamedContext};
use std::sync::atomic::{AtomicUsize, Ordering};

fn source(file: &str) -> Kubeconfig {
    Kubeconfig {
        current_context: Some("test".into()),
        clusters: vec![NamedCluster {
            name: "test".into(),
            cluster: Some(Cluster {
                server: Some("https://fixture.invalid".into()),
                ..Default::default()
            }),
            ..Default::default()
        }],
        contexts: vec![NamedContext {
            name: "test".into(),
            context: Some(Context {
                cluster: "test".into(),
                user: Some("test".into()),
                ..Default::default()
            }),
            ..Default::default()
        }],
        auth_infos: vec![NamedAuthInfo {
            name: "test".into(),
            auth_info: Some(AuthInfo {
                token_file: Some(file.into()),
                ..Default::default()
            }),
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[tokio::test]
async fn concurrent_expired_reads_install_one_fresh_generation_without_network() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("token");
    std::fs::write(&path, "fixture-token").unwrap();
    let lease = lease(source(path.to_str().unwrap()), Some(jiff::Timestamp::now()));
    let (first, second) = tokio::join!(lease.client(None), lease.client(None));
    assert_eq!(first.unwrap().1, 1);
    assert_eq!(second.unwrap().1, 1);
    assert_eq!(lease.state.lock().await.generation, 1);
}

#[tokio::test]
async fn renewal_and_wrapper_construction_invalidate_only_the_owning_engine() {
    use super::super::permissions::{AuthorizationEpoch, authorized_client};
    let (_first_root, _, first, _, _) = super::super::tests::fixture();
    let (_second_root, _, second, _, _) = super::super::tests::fixture();
    let first_epoch = AuthorizationEpoch::capture(&first.authentication_generation);
    let second_epoch = AuthorizationEpoch::capture(&second.authentication_generation);
    let _wrapped = wrap(
        client(StatusCode::OK, Arc::new(AtomicUsize::new(0))),
        Kubeconfig::default(),
        PathBuf::new(),
        TaskTracker::new(),
        CancellationToken::new(),
        second.authentication_generation.clone(),
    );
    assert_eq!(first_epoch.validate(), Ok(()));
    assert_eq!(second_epoch.validate(), Err(ConnectionError::StaleSession));
    let second_epoch = AuthorizationEpoch::capture(&second.authentication_generation);
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("token");
    std::fs::write(&path, "fixture-token").unwrap();
    let mut renewing = lease(source(path.to_str().unwrap()), Some(jiff::Timestamp::now()));
    renewing.authentication_generation = first.authentication_generation.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let guarded = authorized_client(client(StatusCode::OK, calls.clone()), first_epoch.clone());
    renewing.client(None).await.unwrap();
    assert_eq!(first_epoch.validate(), Err(ConnectionError::StaleSession));
    assert_eq!(second_epoch.validate(), Ok(()));
    assert!(guarded.send(Request::new(Body::empty())).await.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn unauthorized_read_refreshes_once_and_does_not_replay_if_refresh_fails() {
    let calls = Arc::new(AtomicUsize::new(0));
    let lease = lease(source("/nonexistent-synthetic-token-file"), None);
    lease.state.lock().await.client = client(StatusCode::UNAUTHORIZED, calls.clone());
    assert!(lease.send(Request::new(Body::empty())).await.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(lease.state.lock().await.failure.is_some());
}
fn client(status: StatusCode, calls: Arc<AtomicUsize>) -> Client {
    Client::new(
        tower::service_fn(move |_request: Request<Body>| {
            calls.fetch_add(1, Ordering::SeqCst);
            async move {
                Ok::<_, std::io::Error>(
                    Response::builder()
                        .status(status)
                        .body(Body::empty())
                        .unwrap(),
                )
            }
        }),
        "default",
    )
}
fn lease(source: Kubeconfig, expiry: Option<jiff::Timestamp>) -> Lease {
    Lease {
        state: Mutex::new(State {
            client: client(StatusCode::OK, Arc::new(AtomicUsize::new(0))).with_valid_until(expiry),
            generation: 0,
            failure: None,
        }),
        source,
        base: PathBuf::new(),
        tasks: TaskTracker::new(),
        cancel: CancellationToken::new(),
        authentication_generation: Arc::new(std::sync::atomic::AtomicU64::new(1)),
    }
}
#[test]
fn replay_excludes_mutations_and_session_upgrades() {
    for (method, path, allowed) in [
        (Method::GET, "/api/v1/pods", true),
        (Method::HEAD, "/version", true),
        (Method::POST, "/api/v1/pods", false),
        (Method::DELETE, "/api/v1/pods/test", false),
        (Method::PATCH, "/api/v1/pods/test", false),
        (Method::GET, "/api/v1/pods/test/exec", false),
        (Method::GET, "/api/v1/pods/test/attach", false),
        (Method::GET, "/api/v1/pods/test/portforward", false),
    ] {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .body(Body::empty())
            .unwrap();
        assert_eq!(replay(&request).is_some(), allowed);
    }
    let upgrade = Request::builder()
        .header("upgrade", "websocket")
        .body(Body::empty())
        .unwrap();
    assert!(replay(&upgrade).is_none());
}
#[tokio::test]
async fn concurrent_failures_share_refresh_backoff_without_credentials_in_error() {
    let lease = Arc::new(lease(
        source("/nonexistent-synthetic-token-file"),
        Some(jiff::Timestamp::now()),
    ));
    let (a, b) = tokio::join!(lease.client(None), lease.client(None));
    assert!(a.is_err());
    assert_eq!(a.err(), b.err());
    assert_eq!(lease.state.lock().await.generation, 0);
    assert!(lease.state.lock().await.failure.is_some());
    assert!(
        !box_error(ConnectionError::AuthHelperFailed)
            .to_string()
            .contains("token")
    );
}
#[tokio::test]
async fn rejected_old_generation_reuses_successful_refresh() {
    let lease = lease(Kubeconfig::default(), None);
    lease.state.lock().await.generation = 1;
    assert_eq!(lease.client(Some(0)).await.unwrap().1, 1);
}
#[tokio::test]
async fn disconnect_rejects_even_unexpired_client() {
    let lease = lease(Kubeconfig::default(), None);
    lease.cancel.cancel();
    assert_eq!(
        lease.client(None).await.err(),
        Some(ConnectionError::StaleSession)
    );
}
#[tokio::test]
async fn unauthorized_mutation_is_sent_once_without_refresh() {
    let calls = Arc::new(AtomicUsize::new(0));
    let lease = lease(source("/nonexistent-synthetic-token-file"), None);
    lease.state.lock().await.client = client(StatusCode::UNAUTHORIZED, calls.clone());
    let request = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/pods")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        lease.send(request).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(lease.state.lock().await.failure.is_none());
}
#[tokio::test]
async fn waiting_refresh_is_cancelled_on_disconnect() {
    let lease = lease(Kubeconfig::default(), None);
    let _guard = lease.state.lock().await;
    let cancel = lease.cancel.clone();
    let request = Request::new(Body::empty());
    let (result, _) = tokio::join!(lease.send(request), async move {
        tokio::task::yield_now().await;
        cancel.cancel();
    });
    assert!(result.is_err());
}
