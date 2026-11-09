use super::*;
use crate::catalog::ImportRequest;
use http::{Request, Response};
use kube::client::Body;
use serde_json::json;
use std::{
    fs,
    sync::atomic::{AtomicUsize, Ordering},
};

pub(super) const CONFIG: &str = "apiVersion: v1\nkind: Config\nclusters:\n- name: target\n  cluster:\n    server: https://fixture.invalid\ncontexts:\n- name: chosen\n  context:\n    cluster: target\n    user: fixture-user\nusers:\n- name: fixture-user\n  user:\n    token: PRIVATE_FIXTURE_TOKEN\n";
pub(super) fn fixture() -> (
    tempfile::TempDir,
    Arc<CatalogEngine>,
    ConnectionEngine,
    String,
    ConnectionTarget,
) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("config");
    fs::write(&path, CONFIG).unwrap();
    let catalog = Arc::new(CatalogEngine::new(root.path().join("data")));
    let snapshot = catalog
        .import_kubeconfig(ImportRequest {
            path: path.to_str().unwrap().into(),
        })
        .unwrap();
    let id = snapshot.clusters[0].id.clone();
    let target = catalog.connection_target(&id).unwrap();
    let engine = ConnectionEngine::new(catalog.clone());
    (root, catalog, engine, id, target)
}
pub(super) fn identity(snapshot: &ConnectionSnapshot) -> SessionRequest {
    SessionRequest {
        cluster_id: snapshot.cluster_id.clone(),
        session_id: snapshot.session_id.clone(),
    }
}
pub(super) fn resource(snapshot: &ConnectionSnapshot, kind: ResourceKind) -> ResourceRequest {
    ResourceRequest {
        cluster_id: snapshot.cluster_id.clone(),
        session_id: snapshot.session_id.clone(),
        kind,
        read_id: 1,
        namespace: None,
        continue_token: None,
        query: None,
        status: None,
        group: None,
    }
}
pub(super) fn fake(response: impl Fn(&str) -> (u16, Vec<u8>) + Send + Sync + 'static) -> Client {
    let response = Arc::new(response);
    Client::new(
        transport::CancelService(tower::service_fn(move |request: Request<Body>| {
            let (status, bytes) = response(&request.uri().to_string());
            async move {
                Ok::<_, std::io::Error>(
                    Response::builder()
                        .status(status)
                        .body(Body::from(bytes))
                        .unwrap(),
                )
            }
        })),
        "default",
    )
}
pub(super) fn normal(path: &str) -> (u16, Vec<u8>) {
    if path.starts_with("/apis/metrics.k8s.io/") {
        return (404, Vec::new());
    }
    let body = if path == "/version" {
        json!({"gitVersion":"v1.36.0"})
    } else if path.starts_with("/api/v1/nodes?") {
        assert!(path.contains("limit=100") && path.contains("timeoutSeconds=10"));
        json!({"metadata":{"continue":"opaque-not-for-renderer"},"items":[
            {"metadata":{"name":"node-b","annotations":{"SECRET":"PRIVATE"}},"status":{"conditions":[{"type":"Ready","status":"False"}]}},
            {"metadata":{"name":"node-a"},"status":{"conditions":[{"type":"Ready","status":"True"}]}}
        ]})
    } else if path == "/apis/metrics.k8s.io/v1beta1/nodes?limit=100&timeoutSeconds=10" {
        json!({"items":[]})
    } else {
        assert!(path.starts_with("/api/v1/namespaces?"));
        json!({"metadata":{},"items":[{"metadata":{"name":"default"},"status":{"phase":"Active"}}]})
    };
    (200, serde_json::to_vec(&body).unwrap())
}
pub(super) async fn connected(
    engine: &ConnectionEngine,
    id: String,
    client: Client,
) -> ConnectionSnapshot {
    let opened = engine.open(OpenRequest { cluster_id: id }).await.unwrap();
    engine
        .connect_with(identity(&opened), |_| async { Ok(client) })
        .await
        .unwrap()
}

pub(super) fn allow_permission_reviews(client: Client) -> Client {
    Client::new(
        tower::service_fn(move |request: Request<Body>| {
            let client = client.clone();
            async move {
                if request.uri().path() == "/apis/authorization.k8s.io/v1/selfsubjectaccessreviews"
                {
                    use http_body_util::BodyExt;
                    let bytes = request.into_body().collect().await.unwrap().to_bytes();
                    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                    return Ok::<_, kube::Error>(Response::new(Body::from(
                        serde_json::to_vec(&json!({
                            "apiVersion":"authorization.k8s.io/v1", "kind":"SelfSubjectAccessReview",
                            "status":{"allowed":true}
                            ,"spec":value["spec"]
                        })).unwrap(),
                    )));
                }
                client.send(request).await
            }
        }),
        "default",
    )
}

pub(super) async fn connected_authorized(
    engine: &ConnectionEngine,
    id: String,
    client: Client,
) -> ConnectionSnapshot {
    connected(engine, id, allow_permission_reviews(client)).await
}

pub(super) fn denied_permission_client(
    reviews: Arc<Mutex<Vec<serde_json::Value>>>,
    allowed: Option<bool>,
) -> Client {
    Client::new(
        tower::service_fn(move |request: Request<Body>| {
            use http_body_util::BodyExt;
            let reviews = reviews.clone();
            async move {
                if request.uri().path() == "/version" {
                    return Ok::<_, std::io::Error>(Response::new(Body::from(
                        br#"{"gitVersion":"v1.36"}"#.to_vec(),
                    )));
                }
                assert_eq!(
                    request.uri().path(),
                    "/apis/authorization.k8s.io/v1/selfsubjectaccessreviews",
                    "denied operations must not reach the resource API"
                );
                assert_eq!(request.method(), http::Method::POST);
                let body = request.into_body().collect().await.unwrap().to_bytes();
                let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
                reviews
                    .lock()
                    .unwrap()
                    .push(value["spec"]["resourceAttributes"].clone());
                let status = allowed
                    .map(|allowed| json!({"allowed":allowed}))
                    .unwrap_or_else(|| json!({"evaluationError":"PRIVATE"}));
                Ok(Response::new(Body::from(serde_json::to_vec(&json!({
                "apiVersion":"authorization.k8s.io/v1", "kind":"SelfSubjectAccessReview", "spec":value["spec"], "status":status
            })).unwrap())))
            }
        }),
        "default",
    )
}

#[tokio::test]
async fn session_open_is_metadata_only_and_camel_case_wire_contract_is_closed() {
    let (_root, _catalog, engine, id, target) = fixture();
    fs::remove_file(target.source_path).unwrap();
    let opened = engine
        .open(OpenRequest {
            cluster_id: id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(opened.status, "disconnected");
    assert_eq!(opened.error, None);
    assert_eq!(
        engine
            .resources(resource(&opened, ResourceKind::Nodes))
            .await,
        Err(ConnectionError::NotConnected)
    );
    let wire = serde_json::to_value(&opened).unwrap();
    assert!(wire.get("clusterId").is_some());
    assert!(wire.get("apiVersion").unwrap().is_null());
    assert!(!wire.to_string().contains("PRIVATE"));
    assert!(
        serde_json::from_value::<SessionRequest>(json!({"clusterId":id,"sessionId":"lease"}))
            .is_ok()
    );
    for value in [
        json!({"cluster_id":"id","session_id":"lease"}),
        json!({"clusterId":"id","sessionId":"lease","url":"https://other.invalid"}),
    ] {
        assert!(serde_json::from_value::<SessionRequest>(value).is_err());
    }
    assert!(
        serde_json::from_value::<ResourceRequest>(
            json!({"clusterId":"id","sessionId":"lease","kind":"secrets"})
        )
        .is_err()
    );
    assert_eq!(
        engine
            .open(OpenRequest {
                cluster_id: "".into()
            })
            .await,
        Err(ConnectionError::InvalidInput)
    );
}
#[tokio::test]
async fn success_scoped_resources_truncation_empty_and_redaction() {
    let (_root, _catalog, engine, id, _) = fixture();
    let snapshot = connected(&engine, id, fake(normal)).await;
    assert_eq!(snapshot.status, "connected");
    assert_eq!(snapshot.api_version.as_deref(), Some("v1.36.0"));
    let nodes = engine
        .resources(resource(&snapshot, ResourceKind::Nodes))
        .await
        .unwrap();
    assert!(nodes.truncated);
    assert_eq!(
        nodes
            .items
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        ["node-a", "node-b"]
    );
    assert_eq!(nodes.items[0].status, "Ready");
    let namespaces = engine
        .resources(resource(&snapshot, ResourceKind::Namespaces))
        .await
        .unwrap();
    assert_eq!(namespaces.items[0].name, "default");
    for value in [
        serde_json::to_string(&snapshot).unwrap(),
        serde_json::to_string(&nodes).unwrap(),
    ] {
        for forbidden in ["PRIVATE", "fixture.invalid", "fixture-user"] {
            assert!(!value.contains(forbidden));
        }
    }
    engine.disconnect(identity(&snapshot)).unwrap();
    assert_eq!(
        engine
            .resources(resource(&snapshot, ResourceKind::Nodes))
            .await,
        Err(ConnectionError::StaleSession)
    );
}
#[tokio::test]
async fn permission_denied_on_nodes_does_not_block_namespaces_or_disconnect() {
    let (_root, _catalog, engine, id, _) = fixture();
    let snapshot = connected(
        &engine,
        id,
        fake(|path| {
            if path.contains("nodes?") {
                (403, b"PRIVATE_TOKEN API error".to_vec())
            } else {
                normal(path)
            }
        }),
    )
    .await;
    assert_eq!(
        engine
            .resources(resource(&snapshot, ResourceKind::Nodes))
            .await,
        Err(ConnectionError::PermissionDenied)
    );
    assert_eq!(
        engine.status(identity(&snapshot)).unwrap().status,
        "connected"
    );
    assert!(
        engine
            .resources(resource(&snapshot, ResourceKind::Namespaces))
            .await
            .is_ok()
    );
}
#[tokio::test]
async fn malformed_and_oversized_bodies_fail_closed_before_publication() {
    for body in [
        b"PRIVATE not json".to_vec(),
        vec![b'x'; 2 * 1024 * 1024 + 1],
    ] {
        let (_root, _catalog, engine, id, _) = fixture();
        let expected = if body.len() > 2 * 1024 * 1024 {
            ConnectionError::LimitExceeded
        } else {
            ConnectionError::InvalidResponse
        };
        let snapshot = connected(
            &engine,
            id,
            fake(move |path| {
                if path == "/version" {
                    normal(path)
                } else {
                    (200, body.clone())
                }
            }),
        )
        .await;
        assert_eq!(
            engine
                .resources(resource(&snapshot, ResourceKind::Nodes))
                .await,
            Err(expected)
        );
    }
    for items in [
        json!([{"metadata":{"name":"duplicate"}},{"metadata":{"name":"duplicate"}}]),
        json!([{"metadata":{"name":"INVALID\nPRIVATE"}}]),
        json!(vec![json!({"metadata":{"name":"node"}}); 101]),
    ] {
        let (_root, _catalog, engine, id, _) = fixture();
        let expected = if items.as_array().unwrap().len() > 100 {
            ConnectionError::LimitExceeded
        } else {
            ConnectionError::InvalidResponse
        };
        let snapshot = connected(
            &engine,
            id,
            fake(move |path| {
                if path == "/version" {
                    normal(path)
                } else {
                    (200, json!({"items":items}).to_string().into_bytes())
                }
            }),
        )
        .await;
        assert_eq!(
            engine
                .resources(resource(&snapshot, ResourceKind::Nodes))
                .await,
            Err(expected)
        );
    }
}
#[tokio::test]
async fn transport_failure_does_not_publish_connected_and_retry_works() {
    for (status, expected) in [
        (401, ConnectionError::AuthFailed),
        (403, ConnectionError::PermissionDenied),
        (504, ConnectionError::Timeout),
    ] {
        let (_root, _catalog, engine, id, _) = fixture();
        let opened = engine.open(OpenRequest { cluster_id: id }).await.unwrap();
        let failed = engine
            .connect_with(identity(&opened), |_| async move {
                Ok(fake(move |_| (status, b"PRIVATE secret error".to_vec())))
            })
            .await
            .unwrap();
        assert_eq!(failed.status, "error");
        assert_eq!(failed.error, Some(expected));
        assert!(failed.api_version.is_none());
        let retry = engine
            .connect_with(identity(&opened), |_| async { Ok(fake(normal)) })
            .await
            .unwrap();
        assert_eq!(retry.status, "connected");
    }
}
#[tokio::test]
async fn disconnect_cancels_pending_connect_and_old_completion_cannot_publish() {
    let (_root, _catalog, engine, id, _) = fixture();
    let opened = engine
        .open(OpenRequest {
            cluster_id: id.clone(),
        })
        .await
        .unwrap();
    let started = Arc::new(tokio::sync::Notify::new());
    let marker = started.clone();
    let connect = engine.connect_with(identity(&opened), |_| async move {
        marker.notify_one();
        std::future::pending::<Result<Client>>().await
    });
    let disconnect = async {
        started.notified().await;
        assert_eq!(
            engine.status(identity(&opened)).unwrap().status,
            "connecting"
        );
        engine.disconnect(identity(&opened)).unwrap();
        let replacement = engine.open(OpenRequest { cluster_id: id }).await.unwrap();
        engine.disconnect(identity(&opened)).unwrap();
        assert_eq!(
            engine.status(identity(&replacement)).unwrap().status,
            "disconnected"
        );
    };
    let (result, ()) = tokio::join!(connect, disconnect);
    assert_eq!(result, Err(ConnectionError::StaleSession));
}
#[tokio::test]
async fn duplicate_connect_is_bounded_and_shutdown_cancels_pending_work() {
    let (_root, _catalog, engine, id, _) = fixture();
    let opened = engine.open(OpenRequest { cluster_id: id }).await.unwrap();
    let pending = engine.connect_with(identity(&opened), |_| {
        std::future::pending::<Result<Client>>()
    });
    let duplicate = async {
        tokio::task::yield_now().await;
        assert_eq!(
            engine
                .connect_with(identity(&opened), |_| async { Ok(fake(normal)) })
                .await,
            Err(ConnectionError::Busy)
        );
        engine.shutdown();
    };
    let (result, ()) = tokio::join!(pending, duplicate);
    assert_eq!(result, Err(ConnectionError::StaleSession));
}
#[tokio::test(start_paused = true)]
async fn timeout_covers_preparation_and_does_not_leave_connecting_state() {
    let (_root, _catalog, engine, id, _) = fixture();
    let opened = engine.open(OpenRequest { cluster_id: id }).await.unwrap();
    let result = engine
        .connect_with(identity(&opened), |_| {
            std::future::pending::<Result<Client>>()
        })
        .await
        .unwrap();
    assert_eq!(result.error, Some(ConnectionError::Timeout));
    assert_eq!(engine.status(identity(&opened)).unwrap().status, "error");
}
#[tokio::test]
async fn resource_disposal_cancels_underlying_client_future() {
    let (_root, _catalog, engine, id, _) = fixture();
    let started = Arc::new(tokio::sync::Notify::new());
    let dropped = Arc::new(AtomicUsize::new(0));
    struct Guard(Arc<AtomicUsize>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let signal = started.clone();
    let counter = dropped.clone();
    let client = Client::new(
        transport::CancelService(tower::service_fn(move |request: Request<Body>| {
            let signal = signal.clone();
            let counter = counter.clone();
            async move {
                if request.uri().path() == "/version" {
                    return Ok::<_, std::io::Error>(Response::new(Body::from(
                        normal("/version").1,
                    )));
                }
                let _guard = Guard(counter);
                signal.notify_one();
                std::future::pending().await
            }
        })),
        "default",
    );
    let snapshot = connected(&engine, id, client).await;
    let work = engine.resources(resource(&snapshot, ResourceKind::Nodes));
    let cancel = async {
        started.notified().await;
        engine
            .cancel_resources(resource(&snapshot, ResourceKind::Nodes))
            .unwrap();
    };
    let (result, ()) = tokio::join!(work, cancel);
    assert_eq!(result, Err(ConnectionError::StaleSession));
    tokio::task::yield_now().await;
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    engine.shutdown();
}
#[test]
fn configuration_selects_exact_context_and_reads_only_bounded_references() {
    let (_root, _catalog, _engine, _id, target) = fixture();
    let config = config::prepare(&target).unwrap();
    assert_eq!(config.contexts.len(), 1);
    assert_eq!(config.current_context.as_deref(), Some("chosen"));
    for (content, expected) in [
        (
            CONFIG.replace("https://fixture.invalid", "http://fixture.invalid"),
            ConnectionError::InsecureTransport,
        ),
        (
            CONFIG.replace(
                "server: https://fixture.invalid",
                "server: https://fixture.invalid\n    insecure-skip-tls-verify: true",
            ),
            ConnectionError::InsecureTransport,
        ),
        (
            CONFIG.replace("name: chosen", "name: changed"),
            ConnectionError::SourceChanged,
        ),
        (
            "PRIVATE_TOKEN: [".into(),
            ConnectionError::KubeconfigInvalid,
        ),
    ] {
        fs::write(&target.source_path, content).unwrap();
        assert_eq!(config::prepare(&target).err(), Some(expected));
    }
    let token = target.source_path.with_file_name("token");
    fs::write(&token, "FILE_FIXTURE_TOKEN\n").unwrap();
    fs::write(
        &target.source_path,
        CONFIG.replace("token: PRIVATE_FIXTURE_TOKEN", "tokenFile: token"),
    )
    .unwrap();
    let parsed = config::prepare(&target).unwrap();
    assert!(
        parsed.auth_infos[0]
            .auth_info
            .as_ref()
            .unwrap()
            .token_file
            .is_none()
    );
    assert!(
        parsed.auth_infos[0]
            .auth_info
            .as_ref()
            .unwrap()
            .token
            .is_some()
    );
    fs::write(token, vec![b'x'; 1024 * 1024 + 1]).unwrap();
    assert_eq!(
        config::prepare(&target).err(),
        Some(ConnectionError::LimitExceeded)
    );
}
#[cfg(unix)]
#[test]
fn symlink_target_change_never_connects_to_retargeted_source() {
    use std::os::unix::fs::symlink;
    let (_root, _catalog, _engine, _id, mut target) = fixture();
    let alias = target.source_path.with_file_name("alias");
    symlink(&target.source_path, &alias).unwrap();
    target.provenance_path = alias.clone();
    assert!(config::prepare(&target).is_ok());
    let other = target.source_path.with_file_name("other");
    fs::write(&other, CONFIG).unwrap();
    fs::remove_file(&alias).unwrap();
    symlink(other, alias).unwrap();
    assert_eq!(
        config::prepare(&target).err(),
        Some(ConnectionError::SourceChanged)
    );
}
