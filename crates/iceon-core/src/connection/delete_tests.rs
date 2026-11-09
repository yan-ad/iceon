use super::{
    delete::*,
    tests::{connected_authorized as connected, fixture},
    *,
};
use http::{Request, Response};
use http_body_util::BodyExt;
use kube::client::Body;
use serde_json::{Value, json};

fn request(kind: ResourceKind) -> DeleteRequest {
    DeleteRequest {
        cluster_id: "cluster".into(),
        session_id: "session".into(),
        kind,
        namespace: if super::resources::namespaced(kind) {
            Some("default".into())
        } else {
            None
        },
        name: "sample".into(),
        uid: "uid-1".into(),
        resource_version: "7".into(),
        read_id: 1,
    }
}

#[tokio::test]
async fn delete_denial_stops_before_precondition_get_and_delete() {
    let (_root, _, engine, id, _) = fixture();
    let reviews = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let lease = tests::connected(
        &engine,
        id,
        tests::denied_permission_client(reviews.clone(), Some(false)),
    )
    .await;
    let mut r = request(ResourceKind::Pods);
    r.cluster_id = lease.cluster_id;
    r.session_id = lease.session_id;
    assert_eq!(
        engine.delete(r).await,
        Err(ConnectionError::PermissionDenied)
    );
    let reviews = reviews.lock().unwrap();
    assert!(!reviews.is_empty());
    assert_eq!(reviews[0]["verb"], "get");
    assert_eq!(reviews[0]["resource"], "pods");
    assert_eq!(reviews[0]["namespace"], "default");
    assert_eq!(reviews[0]["name"], "sample");
}
#[test]
fn validates_namespaced_and_cluster_scopes() {
    assert!(validate(&request(ResourceKind::Pods)).is_ok());
    let mut scoped_cluster = request(ResourceKind::Nodes);
    scoped_cluster.namespace = Some("default".into());
    assert!(validate(&scoped_cluster).is_err());
    let mut r = request(ResourceKind::Nodes);
    r.namespace = None;
    assert!(validate(&r).is_ok());
    for bad in [
        "../x",
        "a/b",
        "UPPER",
        " ",
        ".",
        "..",
        "-a",
        "a-",
        ".a",
        "a.",
        "a..b",
        "a_b",
        "system:admin",
        "a%2fb",
        "a%00",
        "a?b",
        "a#b",
        "a\\b",
        "a\n",
        "é",
    ] {
        let mut r = request(ResourceKind::Pods);
        r.name = bad.into();
        assert!(validate(&r).is_err());
    }
    for name in ["a", "a-b.c9", &"a".repeat(253)] {
        let mut r = request(ResourceKind::Pods);
        r.name = name.into();
        assert!(validate(&r).is_ok());
    }
    let mut r = request(ResourceKind::Pods);
    r.name = "a".repeat(254);
    assert!(validate(&r).is_err());
    r = request(ResourceKind::Pods);
    r.namespace = None;
    assert!(validate(&r).is_err());
    for bad in ["", "../x", "a%2fb", "a_b", "UPPER"] {
        r.namespace = Some(bad.into());
        assert!(validate(&r).is_err());
    }
    for bad in ["", " ", "a\n", "a\0"] {
        r = request(ResourceKind::Pods);
        r.uid = bad.into();
        assert!(validate(&r).is_err());
        r = request(ResourceKind::Pods);
        r.resource_version = bad.into();
        assert!(validate(&r).is_err());
    }
}
#[tokio::test]
async fn preflights_and_deletes_with_preconditions() {
    let (_root, _catalog, engine, id, _) = fixture();
    let client = Client::new(
        super::transport::CancelService(tower::service_fn(|r: Request<Body>| async move {
            let p = r.uri().path();
            if p == "/version" {
                return Ok::<_, std::io::Error>(Response::new(Body::from(
                    br#"{"gitVersion":"v1.36"}"#.to_vec(),
                )));
            }
            if r.method() == "GET" {
                return Ok(Response::new(Body::from(serde_json::to_vec(&json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":"sample","namespace":"default","uid":"uid-1","resourceVersion":"7"}})).unwrap())));
            }
            assert_eq!(r.method(), "DELETE");
            assert!(p.contains("/api/v1/namespaces/default/pods/sample"));
            let v: Value =
                serde_json::from_slice(&r.into_body().collect().await.unwrap().to_bytes()).unwrap();
            assert_eq!(v["preconditions"]["uid"], "uid-1");
            assert_eq!(v["preconditions"]["resourceVersion"], "7");
            Ok(Response::builder().status(202).body(Body::from(serde_json::to_vec(&json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":"sample","namespace":"default","uid":"uid-1","resourceVersion":"8"}})).unwrap())).unwrap())
        })),
        "default",
    );
    let lease = connected(&engine, id, client).await;
    let mut r = request(ResourceKind::Pods);
    r.cluster_id = lease.cluster_id;
    r.session_id = lease.session_id;
    let out = engine.delete(r).await.unwrap();
    assert!(out.accepted && !out.deleted);
}
#[tokio::test]
async fn stale_identity_and_server_errors_are_safe() {
    let (_root, _catalog, engine, id, _) = fixture();
    let lease = connected(
        &engine,
        id,
        tests::fake(|p| {
            if p == "/version" {
                tests::normal(p)
            } else {
                (404, b"PRIVATE_SECRET".to_vec())
            }
        }),
    )
    .await;
    let mut r = request(ResourceKind::Pods);
    r.cluster_id = lease.cluster_id;
    r.session_id = lease.session_id;
    assert_eq!(engine.delete(r).await, Err(ConnectionError::NotFound));
}
#[test]
fn cancellation_tombstones_high_water() {
    let (_root, _catalog, engine, id, _) = fixture();
    let open = futures::executor::block_on(engine.open(OpenRequest { cluster_id: id })).unwrap();
    let r = DeleteCancelRequest {
        cluster_id: open.cluster_id,
        session_id: open.session_id,
        kind: ResourceKind::Pods,
        read_id: 9,
    };
    engine.cancel_delete(r).unwrap();
}

#[tokio::test]
async fn all_kinds_use_single_object_paths_and_safe_options() {
    for variant in include_str!("../connection.rs")
        .split("pub enum ResourceKind {")
        .nth(1)
        .unwrap()
        .split('}')
        .next()
        .unwrap()
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let variant = variant.trim_end_matches(',');
        let kind: ResourceKind = serde_json::from_value(json!(format!(
            "{}{}",
            variant[..1].to_lowercase(),
            &variant[1..]
        )))
        .unwrap();
        let (_root, _catalog, engine, id, _) = fixture();
        let (api, kube_kind) = create::creation_info(kind);
        let mut object = json!({"apiVersion":api,"kind":kube_kind,"metadata":{"name":"sample","uid":"uid-1","resourceVersion":"7"},"data":{"password":"PRIVATE_SECRET"}});
        let scope = if resources::namespaced(kind) {
            object["metadata"]["namespace"] = json!("default");
            "/namespaces/default"
        } else {
            ""
        };
        let prefix = if api == "v1" {
            "/api/v1".to_string()
        } else {
            format!("/apis/{api}")
        };
        let path = format!("{prefix}{scope}/{}/sample", create::plural(kind));
        let client = Client::new(
            transport::CancelService(tower::service_fn(move |r: Request<Body>| {
                let object = object.clone();
                let path = path.clone();
                async move {
                    if r.uri().path() == "/version" {
                        return Ok::<_, std::io::Error>(Response::new(Body::from(
                            br#"{"gitVersion":"v1.36"}"#.to_vec(),
                        )));
                    }
                    assert_eq!(r.uri().to_string(), path);
                    if r.method() == "DELETE" {
                        let v: Value = serde_json::from_slice(
                            &r.into_body().collect().await.unwrap().to_bytes(),
                        )
                        .unwrap();
                        assert_eq!(
                            v,
                            json!({"apiVersion":"v1","kind":"DeleteOptions","propagationPolicy":"Background","preconditions":{"uid":"uid-1","resourceVersion":"7"}})
                        );
                    }
                    Ok(Response::new(Body::from(
                        serde_json::to_vec(&object).unwrap(),
                    )))
                }
            })),
            "default",
        );
        let lease = connected(&engine, id, client).await;
        let mut r = request(kind);
        r.cluster_id = lease.cluster_id;
        r.session_id = lease.session_id;
        let out = engine.delete(r).await.unwrap();
        assert!(out.accepted && !out.deleted);
        assert!(!serde_json::to_string(&out).unwrap().contains("PRIVATE"));
    }
}

#[tokio::test]
async fn errors_never_relay_upstream_messages() {
    for (status, expected) in [
        (403, ConnectionError::PermissionDenied),
        (409, ConnectionError::Conflict),
        (401, ConnectionError::AuthFailed),
        (504, ConnectionError::Timeout),
    ] {
        let (_root, _catalog, engine, id, _) = fixture();
        let lease = connected(
            &engine,
            id,
            tests::fake(move |p| {
                if p == "/version" {
                    tests::normal(p)
                } else {
                    (status, b"PRIVATE_SECRET".to_vec())
                }
            }),
        )
        .await;
        let mut r = request(ResourceKind::Secrets);
        r.cluster_id = lease.cluster_id;
        r.session_id = lease.session_id;
        assert_eq!(engine.delete(r).await, Err(expected));
    }
}

#[tokio::test]
async fn blocked_requests_cancel_disconnect_and_timeout_without_hanging() {
    for mode in ["cancel", "disconnect", "shutdown"] {
        let (_root, _catalog, engine, id, _) = fixture();
        let started = Arc::new(tokio::sync::Notify::new());
        let signal = started.clone();
        let client = Client::new(
            transport::CancelService(tower::service_fn(move |r: Request<Body>| {
                let signal = signal.clone();
                async move {
                    if r.uri().path() == "/version" {
                        return Ok::<_, std::io::Error>(Response::new(Body::from(
                            br#"{"gitVersion":"v1.36"}"#.to_vec(),
                        )));
                    }
                    signal.notify_one();
                    std::future::pending::<std::result::Result<Response<Body>, std::io::Error>>()
                        .await
                }
            })),
            "default",
        );
        let lease = connected(&engine, id, client).await;
        let engine = Arc::new(engine);
        let mut r = request(ResourceKind::Pods);
        r.cluster_id = lease.cluster_id.clone();
        r.session_id = lease.session_id.clone();
        let worker = engine.clone();
        let work = tokio::spawn(async move { worker.delete(r).await });
        tokio::time::timeout(Duration::from_secs(2), started.notified())
            .await
            .expect("transport must start");
        engine
            .cancel_delete(DeleteCancelRequest {
                cluster_id: lease.cluster_id.clone(),
                session_id: lease.session_id.clone(),
                kind: ResourceKind::Pods,
                read_id: 2,
            })
            .unwrap();
        assert!(!work.is_finished());
        match mode {
            "cancel" => engine
                .cancel_delete(DeleteCancelRequest {
                    cluster_id: lease.cluster_id.clone(),
                    session_id: lease.session_id.clone(),
                    kind: ResourceKind::Pods,
                    read_id: 1,
                })
                .unwrap(),
            "disconnect" => engine.disconnect(tests::identity(&lease)).unwrap(),
            "shutdown" => engine.shutdown(),
            _ => {}
        }
        let result = tokio::time::timeout(Duration::from_secs(2), work)
            .await
            .expect("operation must stop")
            .unwrap();
        assert_eq!(result, Err(ConnectionError::StaleSession));
    }
}

#[tokio::test]
async fn invalid_success_bodies_and_preflight_versions_fail_closed() {
    for (status, body, expected) in [
        (200, json!({"kind":"Status","status":"Failure","message":"PRIVATE_SECRET"}).to_string().into_bytes(),ConnectionError::InvalidResponse),
        (202, b"PRIVATE_SECRET".to_vec(), ConnectionError::InvalidResponse),
        (200, vec![b' ';256*1024+1], ConnectionError::LimitExceeded),
        (200, json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":"sample","namespace":"default","uid":"uid-1","resourceVersion":"8"}}).to_string().into_bytes(),ConnectionError::Conflict)
    ] {
        let (_root,_catalog,engine,id,_)=fixture();
        let lease=connected(&engine,id,tests::fake(move |p| if p=="/version" {tests::normal(p)} else {(status,body.clone())})).await;
        let mut r=request(ResourceKind::Pods);r.cluster_id=lease.cluster_id;r.session_id=lease.session_id;
        assert_eq!(engine.delete(r).await,Err(expected));
    }
}

fn pod() -> Value {
    json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":"sample","namespace":"default","uid":"uid-1","resourceVersion":"7"}})
}

async fn transport_case(
    preflight: Value,
    status: u16,
    body: Vec<u8>,
) -> (Result<DeleteResult>, usize) {
    let (_root, _catalog, engine, id, _) = fixture();
    let deletes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = deletes.clone();
    let client = Client::new(
        transport::CancelService(tower::service_fn(move |r: Request<Body>| {
            let (status, bytes) = if r.uri().path() == "/version" {
                tests::normal("/version")
            } else if r.method() == "GET" {
                (200, serde_json::to_vec(&preflight).unwrap())
            } else {
                count.fetch_add(1, Ordering::SeqCst);
                (status, body.clone())
            };
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
    );
    let lease = connected(&engine, id, client).await;
    let mut r = request(ResourceKind::Pods);
    r.cluster_id = lease.cluster_id;
    r.session_id = lease.session_id;
    (engine.delete(r).await, deletes.load(Ordering::SeqCst))
}

#[tokio::test]
async fn preflight_distinguishes_replacement_from_malformed_identity_without_deleting() {
    for field in ["uid", "resourceVersion"] {
        let mut object = pod();
        object["metadata"][field] = json!("changed");
        assert_eq!(
            transport_case(object, 204, vec![]).await,
            (Err(ConnectionError::Conflict), 0)
        );
    }
    for (pointer, replacement) in [
        ("/metadata/name", json!("other")),
        ("/metadata/namespace", json!("other")),
        ("/metadata/uid", Value::Null),
        ("/metadata/resourceVersion", json!("")),
        ("/apiVersion", json!("other/v1")),
        ("/kind", json!("Secret")),
    ] {
        let mut object = pod();
        *object.pointer_mut(pointer).unwrap() = replacement;
        assert_eq!(
            transport_case(object, 204, vec![]).await,
            (Err(ConnectionError::InvalidResponse), 0)
        );
    }
}

#[tokio::test]
async fn delete_success_requires_identity_and_reports_acceptance_not_finalization() {
    let status = json!({"apiVersion":"v1","kind":"Status","status":"Success","details":{"name":"sample","uid":"uid-1","kind":"pods","group":""},"code":200});
    let mut finalizing = pod();
    finalizing["metadata"]["resourceVersion"] = json!("8");
    finalizing["metadata"]["deletionTimestamp"] = json!("2026-10-26T00:00:00Z");
    for (code, value) in [
        (200, status.clone()),
        (200, finalizing.clone()),
        (202, finalizing),
    ] {
        let (result, count) =
            transport_case(pod(), code, serde_json::to_vec(&value).unwrap()).await;
        let result = result.unwrap();
        assert_eq!(count, 1);
        assert!(result.accepted && !result.deleted);
    }
    let (result, _) = transport_case(pod(), 204, vec![]).await;
    let result = result.unwrap();
    assert!(result.deleted && !result.accepted);
    for (code, body) in [
        (204, b"null".to_vec()),
        (200, vec![]),
        (202, b"null".to_vec()),
        (201, serde_json::to_vec(&pod()).unwrap()),
    ] {
        assert_eq!(
            transport_case(pod(), code, body).await.0,
            Err(ConnectionError::InvalidResponse)
        );
    }
    for (pointer, replacement) in [
        ("/details/uid", json!("replacement")),
        ("/details/name", Value::Null),
        ("/details/group", json!("apps")),
        ("/details/kind", json!("secrets")),
        ("/code", json!(202)),
        ("/status", json!("Failure")),
    ] {
        let mut value = status.clone();
        *value.pointer_mut(pointer).unwrap() = replacement;
        assert_eq!(
            transport_case(pod(), 200, serde_json::to_vec(&value).unwrap())
                .await
                .0,
            Err(ConnectionError::InvalidResponse)
        );
    }
    let mut value = pod();
    value["metadata"]["uid"] = json!("replacement");
    assert_eq!(
        transport_case(pod(), 200, serde_json::to_vec(&value).unwrap())
            .await
            .0,
        Err(ConnectionError::InvalidResponse)
    );
}

#[tokio::test]
async fn dropped_futures_allow_new_ids_and_old_completion_cannot_clear_new_cancellation() {
    let (_root, _catalog, engine, id, _) = fixture();
    let started = Arc::new(tokio::sync::Notify::new());
    let signal = started.clone();
    let client = Client::new(
        transport::CancelService(tower::service_fn(move |r: Request<Body>| {
            let signal = signal.clone();
            async move {
                if r.uri().path() == "/version" {
                    return Ok::<_, std::io::Error>(Response::new(Body::from(
                        tests::normal("/version").1,
                    )));
                }
                signal.notify_one();
                std::future::pending::<std::result::Result<Response<Body>, std::io::Error>>().await
            }
        })),
        "default",
    );
    let lease = connected(&engine, id, client).await;
    let mut r = request(ResourceKind::Pods);
    r.cluster_id = lease.cluster_id.clone();
    r.session_id = lease.session_id.clone();
    let cancel = |read_id| DeleteCancelRequest {
        cluster_id: lease.cluster_id.clone(),
        session_id: lease.session_id.clone(),
        kind: ResourceKind::Pods,
        read_id,
    };
    let mut first = Box::pin(engine.delete(r.clone()));
    tokio::select! { biased; result = &mut first => panic!("unexpected completion: {result:?}"), _ = started.notified() => {} }
    drop(first);
    assert!(
        engine.session.lock().unwrap().as_ref().unwrap().deletes[&r.kind]
            .cancel
            .as_ref()
            .unwrap()
            .is_cancelled()
    );
    assert_eq!(
        engine.delete(r.clone()).await,
        Err(ConnectionError::StaleSession)
    );
    r.read_id = 2;
    let mut second = Box::pin(engine.delete(r.clone()));
    tokio::select! { biased; result = &mut second => panic!("unexpected completion: {result:?}"), _ = started.notified() => {} }
    r.read_id = 3;
    assert_eq!(engine.delete(r.clone()).await, Err(ConnectionError::Busy));
    engine.cancel_delete(cancel(2)).unwrap();
    engine.cancel_delete(cancel(3)).unwrap();
    assert_eq!(
        engine.delete(r.clone()).await,
        Err(ConnectionError::StaleSession)
    );
    r.read_id = 4;
    let mut fourth = Box::pin(engine.delete(r.clone()));
    tokio::select! { biased; result = &mut fourth => panic!("unexpected completion: {result:?}"), _ = started.notified() => {} }
    engine.cancel_delete(cancel(2)).unwrap();
    assert_eq!(second.await, Err(ConnectionError::StaleSession));
    assert!(
        !engine.session.lock().unwrap().as_ref().unwrap().deletes[&r.kind]
            .cancel
            .as_ref()
            .unwrap()
            .is_cancelled()
    );
    engine.cancel_delete(cancel(4)).unwrap();
    assert_eq!(fourth.await, Err(ConnectionError::StaleSession));
}

#[tokio::test]
async fn deadline_cancels_transport_and_releases_slot() {
    let (_root, _catalog, engine, id, _) = fixture();
    let started = Arc::new(tokio::sync::Notify::new());
    let signal = started.clone();
    let client = Client::new(
        transport::CancelService(tower::service_fn(move |r: Request<Body>| {
            let signal = signal.clone();
            async move {
                if r.uri().path() == "/version" {
                    return Ok::<_, std::io::Error>(Response::new(Body::from(
                        tests::normal("/version").1,
                    )));
                }
                signal.notify_one();
                std::future::pending::<std::result::Result<Response<Body>, std::io::Error>>().await
            }
        })),
        "default",
    );
    let lease = connected(&engine, id, client).await;
    assert_eq!(lease.status, "connected");
    let mut r = request(ResourceKind::Pods);
    r.cluster_id = lease.cluster_id;
    r.session_id = lease.session_id;
    // Pause only after connection setup's external blocking work has finished.
    tokio::time::pause();
    let mut work = Box::pin(engine.delete(r.clone()));
    tokio::select! { biased; result = &mut work => panic!("unexpected completion: {result:?}"), _ = started.notified() => {} }
    tokio::time::advance(DEADLINE + Duration::from_secs(1)).await;
    assert_eq!(work.await, Err(ConnectionError::Timeout));
    assert!(
        engine.session.lock().unwrap().as_ref().unwrap().deletes[&r.kind]
            .cancel
            .is_none()
    );
    tokio::time::resume();
}
