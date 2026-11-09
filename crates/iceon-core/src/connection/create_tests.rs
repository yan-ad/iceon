use super::{
    create::*,
    tests::{connected_authorized as connected, fixture},
    *,
};
use http::{Request, Response};
use http_body_util::BodyExt;
use kube::client::Body;
use serde_json::{Value, json};

fn request(kind: ResourceKind) -> CreateRequest {
    let (api, kind_name) = creation_info(kind);
    CreateRequest {
        cluster_id: "cluster".into(),
        session_id: "session".into(),
        kind,
        namespace: None,
        yaml: format!("apiVersion: {api}\nkind: {kind_name}\nmetadata:\n  name: sample\n"),
        dry_run: true,
        read_id: 1,
    }
}

#[tokio::test]
async fn create_reviews_block_dry_run_and_commit_before_resource_dispatch() {
    for dry_run in [true, false] {
        for allowed in [Some(false), None] {
            let (_root, _, engine, id, _) = fixture();
            let reviews = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let lease = tests::connected(
                &engine,
                id,
                tests::denied_permission_client(reviews.clone(), allowed),
            )
            .await;
            let mut r = request(ResourceKind::Pods);
            r.cluster_id = lease.cluster_id;
            r.session_id = lease.session_id;
            r.dry_run = dry_run;
            assert!(engine.create(r).await.is_err());
            let reviews = reviews.lock().unwrap();
            assert_eq!(reviews.len(), 1);
            assert_eq!(reviews[0]["verb"], "create");
            assert_eq!(reviews[0]["resource"], "pods");
            assert_eq!(reviews[0]["namespace"], "default");
            assert!(reviews[0].get("name").is_none());
        }
    }
}
fn kinds() -> Vec<ResourceKind> {
    // Inventory wire names exercise all variants without maintaining a second kind mapping.
    let source = include_str!("../connection.rs");
    let block = source
        .split("pub enum ResourceKind {")
        .nth(1)
        .unwrap()
        .split('}')
        .next()
        .unwrap();
    block
        .lines()
        .filter_map(|line| {
            let s = line.trim().trim_end_matches(',');
            if s.is_empty() {
                return None;
            }
            let wire = format!("{}{}", s[..1].to_lowercase(), &s[1..]);
            Some(serde_json::from_value(json!(wire)).unwrap())
        })
        .collect()
}
#[test]
fn validates_closed_yaml_identity_and_metadata() {
    for kind in kinds() {
        assert!(validate_yaml(&request(kind)).is_ok(), "{kind:?}");
    }
    let r = request(ResourceKind::Secrets);
    for yaml in [
        "",
        "[]",
        "https://host.invalid",
        "apiVersion: v1\nkind: List",
        "apiVersion: v1\nkind: Secret\nmetadata: {name: x}\n---\n{}",
    ] {
        let mut bad = r.clone();
        bad.yaml = yaml.into();
        assert!(validate_yaml(&bad).is_err());
    }
    for key in [
        "uid",
        "resourceVersion",
        "creationTimestamp",
        "managedFields",
        "generation",
        "deletionTimestamp",
        "selfLink",
    ] {
        let mut bad = r.clone();
        bad.yaml.push_str(&format!("  {key}: PRIVATE\n"));
        assert!(validate_yaml(&bad).is_err());
    }
    for name in ["../a", "a/b", "UPPER", "-a", "a..b", "a?b"] {
        let mut bad = r.clone();
        bad.yaml = bad.yaml.replace("sample", name);
        assert!(validate_yaml(&bad).is_err());
    }
    let mut bad = r.clone();
    bad.yaml.push_str("status: {}\n");
    assert!(validate_yaml(&bad).is_err());
    bad.yaml = "x".repeat(256 * 1024 + 1);
    assert!(validate_yaml(&bad).is_err());
    let (body, _, ns) = validate_yaml(&r).unwrap();
    assert_eq!(ns.as_deref(), Some("default"));
    assert_eq!(body["metadata"]["namespace"], "default");
    let mut mismatch = r.clone();
    mismatch.namespace = Some("selected".into());
    mismatch.yaml.push_str("  namespace: other\n");
    assert!(validate_yaml(&mismatch).is_err());
    let mut scoped = request(ResourceKind::Nodes);
    scoped.namespace = Some("default".into());
    assert!(validate_yaml(&scoped).is_err());
    let wire = json!({"clusterId":"cluster","sessionId":"session","kind":"pods","yaml":"","dryRun":true,"readId":1,"url":"https://private"});
    assert!(serde_json::from_value::<CreateRequest>(wire).is_err());
}

#[tokio::test]
async fn posts_every_inventory_kind_to_allowlisted_collection() {
    let (_root, _catalog, engine, id, _) = fixture();
    let client = Client::new(
        super::transport::CancelService(tower::service_fn(|r: Request<Body>| async move {
            if r.uri().path() == "/version" {
                return Ok::<_, std::io::Error>(Response::new(Body::from(
                    br#"{"gitVersion":"v1.36"}"#.to_vec(),
                )));
            }
            assert_eq!(r.method(), "POST");
            assert_eq!(r.uri().query(), Some("dryRun=All"));
            let path = r.uri().path().to_owned();
            let value: Value =
                serde_json::from_slice(&r.into_body().collect().await.unwrap().to_bytes()).unwrap();
            let api = value["apiVersion"].as_str().unwrap();
            assert!(path.starts_with(if api == "v1" { "/api/v1/" } else { "/apis/" }));
            if value.pointer("/metadata/namespace").is_some() {
                assert!(path.contains("/namespaces/default/"));
            }
            Ok(Response::builder()
                .status(201)
                .body(Body::from(serde_json::to_vec(&value).unwrap()))
                .unwrap())
        })),
        "default",
    );
    let lease = connected(&engine, id, client).await;
    for kind in kinds() {
        let mut r = request(kind);
        r.cluster_id = lease.cluster_id.clone();
        r.session_id = lease.session_id.clone();
        let result = engine.create(r.clone()).await.unwrap();
        assert!(!result.created);
        assert_eq!(result.name, "sample");
        assert!(!serde_json::to_string(&result).unwrap().contains("yaml"));
        assert_eq!(engine.create(r).await, Err(ConnectionError::StaleSession));
    }
}

#[tokio::test]
async fn sanitizes_server_errors_and_cancels_exact_owner() {
    for (status, expected) in [
        (400, ConnectionError::InvalidInput),
        (401, ConnectionError::AuthFailed),
        (403, ConnectionError::PermissionDenied),
        (404, ConnectionError::ResourceUnavailable),
        (409, ConnectionError::Conflict),
        (422, ConnectionError::InvalidInput),
        (504, ConnectionError::Timeout),
    ] {
        let (_root, _catalog, engine, id, _) = fixture();
        let lease = connected(
            &engine,
            id,
            tests::fake(move |path| {
                if path == "/version" {
                    tests::normal(path)
                } else {
                    (status, b"PRIVATE_SECRET".to_vec())
                }
            }),
        )
        .await;
        let mut r = request(ResourceKind::Pods);
        r.cluster_id = lease.cluster_id;
        r.session_id = lease.session_id;
        assert_eq!(engine.create(r).await, Err(expected));
    }
    let (_root, _catalog, engine, id, _) = fixture();
    let lease = connected(&engine, id, tests::fake(tests::normal)).await;
    let mut r = request(ResourceKind::Pods);
    r.cluster_id = lease.cluster_id.clone();
    r.session_id = lease.session_id.clone();
    engine
        .cancel_create(CreateCancelRequest {
            cluster_id: lease.cluster_id,
            session_id: lease.session_id,
            kind: r.kind,
            read_id: 1,
        })
        .unwrap();
    assert_eq!(engine.create(r).await, Err(ConnectionError::StaleSession));
}

#[tokio::test]
async fn real_create_is_not_dry_run_and_never_returns_secret_data() {
    let (_root, _catalog, engine, id, _) = fixture();
    let client = Client::new(
        transport::CancelService(tower::service_fn(|r: Request<Body>| async move {
            if r.uri().path() == "/version" {
                return Ok::<_, std::io::Error>(Response::new(Body::from(
                    br#"{"gitVersion":"v1.36"}"#.to_vec(),
                )));
            }
            assert_eq!(r.uri().to_string(), "/api/v1/namespaces/default/secrets");
            let value: Value =
                serde_json::from_slice(&r.into_body().collect().await.unwrap().to_bytes()).unwrap();
            assert_eq!(value["stringData"]["password"], "PRIVATE_SECRET");
            Ok(Response::builder()
                .status(201)
                .body(Body::from(serde_json::to_vec(&value).unwrap()))
                .unwrap())
        })),
        "default",
    );
    let lease = connected(&engine, id, client).await;
    let mut r = request(ResourceKind::Secrets);
    r.cluster_id = lease.cluster_id;
    r.session_id = lease.session_id;
    r.dry_run = false;
    r.yaml.push_str("stringData: {password: PRIVATE_SECRET}\n");
    let result = engine.create(r).await.unwrap();
    assert!(result.created);
    assert!(!serde_json::to_string(&result).unwrap().contains("PRIVATE"));
}

#[tokio::test]
async fn malformed_oversize_and_mismatched_responses_are_not_success() {
    for (body, expected) in [(b"PRIVATE_SECRET".to_vec(), ConnectionError::InvalidResponse), (vec![b' '; 1024 * 1024 + 1], ConnectionError::LimitExceeded), (serde_json::to_vec(&json!({"apiVersion":"v1","kind":"Secret","metadata":{"name":"other","namespace":"default"}})).unwrap(), ConnectionError::InvalidResponse)] {
        let (_root, _catalog, engine, id, _) = fixture();
        let lease = connected(&engine, id, tests::fake(move |path| if path == "/version" { tests::normal(path) } else { (201, body.clone()) })).await;
        let mut r = request(ResourceKind::Secrets); r.cluster_id = lease.cluster_id; r.session_id = lease.session_id;
        assert_eq!(engine.create(r).await, Err(expected));
    }
}

#[tokio::test]
async fn in_flight_cancel_disconnect_and_timeout_bound_transport() {
    for mode in ["cancel", "disconnect", "timeout"] {
        let (_root, _catalog, engine, id, _) = fixture();
        let started = std::sync::Arc::new(tokio::sync::Notify::new());
        let signal = started.clone();
        let service = tower::service_fn(move |r: Request<Body>| {
            let signal = signal.clone();
            async move {
                if r.uri().path() == "/version" {
                    return Ok::<_, std::io::Error>(Response::new(Body::from(
                        br#"{"gitVersion":"v1.36"}"#.to_vec(),
                    )));
                }
                signal.notify_one();
                std::future::pending::<std::result::Result<Response<Body>, std::io::Error>>().await
            }
        });
        // Keep the transport's equal deadline from racing the engine timeout assertion.
        let client = if mode == "timeout" {
            Client::new(service, "default")
        } else {
            Client::new(transport::CancelService(service), "default")
        };
        let lease = connected(&engine, id, client).await;
        assert_eq!(lease.status, "connected", "{mode}: {lease:?}");
        let engine = std::sync::Arc::new(engine);
        let mut r = request(ResourceKind::Pods);
        r.cluster_id = lease.cluster_id.clone();
        r.session_id = lease.session_id.clone();
        let worker = engine.clone();
        let work = tokio::spawn(async move { worker.create(r).await });
        tokio::time::timeout(Duration::from_secs(5), started.notified())
            .await
            .unwrap_or_else(|_| panic!("{mode}: create transport did not start"));
        // A different operation ID must not cancel this request.
        engine
            .cancel_create(CreateCancelRequest {
                cluster_id: lease.cluster_id.clone(),
                session_id: lease.session_id.clone(),
                kind: ResourceKind::Pods,
                read_id: 2,
            })
            .unwrap();
        assert!(!work.is_finished());
        match mode {
            "cancel" => engine
                .cancel_create(CreateCancelRequest {
                    cluster_id: lease.cluster_id,
                    session_id: lease.session_id,
                    kind: ResourceKind::Pods,
                    read_id: 1,
                })
                .unwrap(),
            "disconnect" => engine.disconnect(tests::identity(&lease)).unwrap(),
            _ => {
                tokio::time::pause();
                tokio::time::advance(DEADLINE + Duration::from_secs(1)).await;
                tokio::time::resume();
            }
        }
        let error = tokio::time::timeout(Duration::from_secs(5), work)
            .await
            .unwrap_or_else(|_| panic!("{mode}: create task did not finish"))
            .unwrap()
            .unwrap_err();
        assert_eq!(
            error,
            if mode == "timeout" {
                ConnectionError::Timeout
            } else {
                ConnectionError::StaleSession
            }
        );
    }
}
