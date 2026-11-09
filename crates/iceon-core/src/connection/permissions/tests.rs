use super::*;
use crate::connection::{
    OpenRequest,
    tests::{fixture, identity},
};
use http::{Request, Response};
use std::sync::{Arc, Mutex, atomic::AtomicUsize};

fn check(action: PermissionAction) -> PermissionCheck {
    PermissionCheck {
        kind: ResourceKind::Pods,
        action,
        namespace: Some("team-a".into()),
        name: Some("api".into()),
    }
}

#[test]
fn eviction_requires_pod_get_and_core_eviction_create_without_delete() {
    let mut value = check(PermissionAction::Evict);
    assert_eq!(
        requirements(&value).unwrap(),
        vec![
            json!({"group":"", "version":"v1", "resource":"pods", "verb":"get", "namespace":"team-a", "name":"api"}),
            json!({"group":"", "version":"v1", "resource":"pods", "subresource":"eviction", "verb":"create", "namespace":"team-a", "name":"api"}),
        ]
    );
    value.kind = ResourceKind::Nodes;
    value.namespace = None;
    assert_eq!(requirements(&value), Err(ConnectionError::InvalidInput));
    value = check(PermissionAction::Evict);
    value.name = None;
    assert_eq!(requirements(&value), Err(ConnectionError::InvalidInput));
    value = check(PermissionAction::Evict);
    value.namespace = None;
    assert_eq!(requirements(&value), Err(ConnectionError::InvalidInput));
}

#[tokio::test]
async fn direct_delete_cannot_bypass_backend_denial() {
    let (_root, _catalog, engine, id, _) = fixture();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let snapshot = crate::connection::tests::connected(
        &engine,
        id,
        client(json!({"allowed":false}), seen.clone()),
    )
    .await;
    let result = engine
        .delete(crate::connection::delete::DeleteRequest {
            cluster_id: snapshot.cluster_id,
            session_id: snapshot.session_id,
            kind: ResourceKind::Pods,
            namespace: Some("team-a".into()),
            name: "api".into(),
            uid: "fixture-uid".into(),
            resource_version: "1".into(),
            read_id: 1,
        })
        .await;
    assert_eq!(result, Err(ConnectionError::PermissionDenied));
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn names_and_core_attributes_are_exact() {
    let mut value = check(PermissionAction::Get);
    for name in [
        "api/exec",
        "..",
        "api%2fexec",
        "api?watch=true",
        "api..test",
        "API",
    ] {
        value.name = Some(name.into());
        assert_eq!(requirements(&value), Err(ConnectionError::InvalidInput));
    }
    value.name = Some("api".into());
    assert_eq!(requirements(&value).unwrap()[0]["group"], "");
    value.kind = ResourceKind::Roles;
    value.name = Some("system:controller:fixture".into());
    assert_eq!(
        requirements(&value).unwrap()[0]["name"],
        "system:controller:fixture"
    );
}

#[tokio::test]
async fn old_authorization_cannot_dispatch_after_credential_epoch_change() {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let client = Client::new(
        tower::service_fn(move |_: Request<Body>| {
            count.fetch_add(1, Ordering::SeqCst);
            async { Ok::<_, std::io::Error>(Response::new(Body::empty())) }
        }),
        "default",
    );
    let generation = Arc::new(AtomicU64::new(1));
    let client = authorized_client(client, AuthorizationEpoch::capture(&generation));
    invalidate_authentication(&generation);
    assert!(client.send(Request::new(Body::empty())).await.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn engine_refresh_isolates_guards_and_cached_permissions() {
    let (_first_root, _, first, first_id, _) = fixture();
    let (_second_root, _, second, second_id, _) = fixture();
    let first_scope = identity(
        &first
            .open(OpenRequest {
                cluster_id: first_id,
            })
            .await
            .unwrap(),
    );
    let second_scope = identity(
        &second
            .open(OpenRequest {
                cluster_id: second_id,
            })
            .await
            .unwrap(),
    );
    let first_epoch = AuthorizationEpoch::capture(&first.authentication_generation);
    let second_epoch = AuthorizationEpoch::capture(&second.authentication_generation);
    assert_ne!(first_epoch, second_epoch);
    let denied = check(PermissionAction::Get);
    {
        let mut state = first.session.lock().unwrap();
        let permissions = &mut state.as_mut().unwrap().permissions;
        permissions.synchronize(&first.authentication_generation);
        permissions.cache.insert(
            denied.clone(),
            Cached {
                item: PermissionItem {
                    check: denied.clone(),
                    decision: PermissionDecision::Denied,
                    reason: Some(PermissionReason::Denied),
                },
                expires: now() + 10_000,
                deadline: tokio::time::Instant::now() + Duration::from_secs(10),
                used: 1,
            },
        );
    }
    second.refresh_permissions(second_scope).unwrap();
    assert_eq!(first_epoch.validate(), Ok(()));
    assert_eq!(second_epoch.validate(), Err(ConnectionError::StaleSession));
    assert_eq!(
        first.reject_known_denied_read(&first_scope, &denied),
        Err(ConnectionError::PermissionDenied)
    );
    first.refresh_permissions(first_scope.clone()).unwrap();
    assert_eq!(first_epoch.validate(), Err(ConnectionError::StaleSession));
    assert_eq!(
        first.reject_known_denied_read(&first_scope, &denied),
        Ok(())
    );
}

#[tokio::test]
async fn replacing_or_disconnecting_a_lease_invalidates_its_guards() {
    let (_root, _, engine, id, _) = fixture();
    let old = engine
        .open(OpenRequest {
            cluster_id: id.clone(),
        })
        .await
        .unwrap();
    let old_epoch = AuthorizationEpoch::capture(&engine.authentication_generation);
    let replacement = engine.open(OpenRequest { cluster_id: id }).await.unwrap();
    let replacement_epoch = AuthorizationEpoch::capture(&engine.authentication_generation);
    assert_eq!(old_epoch.validate(), Err(ConnectionError::StaleSession));
    engine.disconnect(identity(&old)).unwrap();
    assert_eq!(replacement_epoch.validate(), Ok(()));
    engine.disconnect(identity(&replacement)).unwrap();
    assert_eq!(
        replacement_epoch.validate(),
        Err(ConnectionError::StaleSession)
    );
}

fn client(status: Value, seen: Arc<Mutex<Vec<Value>>>) -> Client {
    Client::new(
        tower::service_fn(move |request: Request<Body>| {
            let status = status.clone();
            let seen = seen.clone();
            async move {
                let value = if request.uri().path() == "/version" {
                    json!({"gitVersion":"v1.36.0"})
                } else {
                    assert_eq!(request.method(), http::Method::POST);
                    assert_eq!(request.uri().path(), REVIEW_PATH);
                    assert!(request.headers().get("impersonate-user").is_none());
                    let bytes = request.into_body().collect().await.unwrap().to_bytes();
                    let mut value: Value = serde_json::from_slice(&bytes).unwrap();
                    seen.lock().unwrap().push(value.clone());
                    value["status"] = status;
                    value
                };
                Ok::<_, std::io::Error>(Response::new(Body::from(
                    serde_json::to_vec(&value).unwrap(),
                )))
            }
        }),
        "default",
    )
}

#[test]
fn closed_contract_and_scope_validation() {
    assert!(
        serde_json::from_value::<PermissionCheck>(
            json!({"kind":"pods","action":"list","namespace":null,"name":null,"user":"root"})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<PermissionCheck>(
            json!({"kind":"pods","action":"bind","namespace":null,"name":null})
        )
        .is_err()
    );
    let mut value = check(PermissionAction::List);
    value.name = None;
    value.namespace = None;
    assert!(requirements(&value).unwrap()[0].get("namespace").is_none());
    value.kind = ResourceKind::Nodes;
    value.namespace = Some("team-a".into());
    assert_eq!(requirements(&value), Err(ConnectionError::InvalidInput));
    let mut value = check(PermissionAction::Exec);
    value.name = None;
    assert_eq!(requirements(&value), Err(ConnectionError::InvalidInput));
    value = check(PermissionAction::Get);
    value.namespace = None;
    assert_eq!(requirements(&value), Err(ConnectionError::InvalidInput));
    assert_eq!(
        validate_checks(&vec![check(PermissionAction::Get); 33]),
        Err(ConnectionError::InvalidInput)
    );
}
#[test]
fn exact_subresources_match_kube_get_upgrade_transport() {
    let wire = kube::api::Request::new("/api/v1/namespaces/team-a/pods");
    let attach = kube::api::AttachParams::default();
    let requests = [
        wire.logs("api", &kube::api::LogParams::default()).unwrap(),
        wire.exec("api", ["sh"], &attach).unwrap(),
        wire.attach("api", &attach).unwrap(),
        wire.portforward("api", &[8080]).unwrap(),
    ];
    for request in requests {
        assert_eq!(request.method(), http::Method::GET);
    }
    for (action, subresource) in [
        (PermissionAction::Logs, "log"),
        (PermissionAction::Exec, "exec"),
        (PermissionAction::Attach, "attach"),
        (PermissionAction::PortForward, "portforward"),
    ] {
        let values = requirements(&check(action)).unwrap();
        assert_eq!(values.len(), 2);
        assert!(values[0].get("subresource").is_none());
        assert_eq!(values[1]["subresource"], subresource);
        for value in values {
            assert_eq!(value["verb"], "get");
            assert_eq!(value["resource"], "pods");
            assert_eq!(value["namespace"], "team-a");
            assert_eq!(value["name"], "api");
        }
    }
    for (action, verb) in [
        (PermissionAction::Update, "update"),
        (PermissionAction::Patch, "patch"),
    ] {
        assert_eq!(requirements(&check(action)).unwrap()[0]["verb"], verb);
    }
    let mut value = check(PermissionAction::Create);
    value.kind = ResourceKind::Roles;
    let attributes = &requirements(&value).unwrap()[0];
    assert_eq!(attributes["group"], "rbac.authorization.k8s.io");
    assert_eq!(attributes["resource"], "roles");
}
#[test]
fn evaluation_is_fail_closed_without_leaking_reason() {
    assert_eq!(
        evaluate_status(&json!({"allowed":false,"denied":false})),
        Ok(PermissionDecision::Denied)
    );
    assert_eq!(
        evaluate_status(&json!({"allowed":true})),
        Ok(PermissionDecision::Allowed)
    );
    for value in [
        json!({}),
        json!({"allowed":true,"denied":true}),
        json!({"allowed":true,"evaluationError":"PRIVATE_TOKEN"}),
        json!({"allowed":"true"}),
    ] {
        assert_eq!(evaluate_status(&value), Err(PermissionReason::Failed));
    }
}
#[tokio::test]
async fn review_cache_fresh_guards_cancel_and_invalidation() {
    let (_root, _catalog, engine, id, _) = fixture();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let snapshot = crate::connection::tests::connected(
        &engine,
        id,
        client(json!({"allowed":true}), seen.clone()),
    )
    .await;
    let identity = identity(&snapshot);
    let mut request = ReviewRequest {
        cluster_id: identity.cluster_id.clone(),
        session_id: identity.session_id.clone(),
        read_id: 1,
        checks: vec![check(PermissionAction::Get)],
    };
    let first = engine.review_permissions(request.clone()).await.unwrap();
    assert_eq!(first.items[0].decision, PermissionDecision::Allowed);
    assert!(first.expires_at <= first.checked_at + 30_000);
    request.read_id += 1;
    engine.review_permissions(request.clone()).await.unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
    engine
        .require_permissions(&identity, &request.checks)
        .await
        .unwrap();
    assert_eq!(seen.lock().unwrap().len(), 2);
    request.read_id += 1;
    engine.cancel_permissions(request.clone()).unwrap();
    assert!(matches!(
        engine.review_permissions(request.clone()).await,
        Err(ConnectionError::StaleSession)
    ));
    let mut mismatched = request.clone();
    mismatched.checks[0].name = Some("other".into());
    assert_eq!(
        engine.cancel_permissions(mismatched),
        Err(ConnectionError::InvalidInput)
    );
    engine.refresh_permissions(identity.clone()).unwrap();
    request.read_id += 1;
    let refreshed = engine.review_permissions(request.clone()).await.unwrap();
    assert_ne!(refreshed.authorization_epoch, first.authorization_epoch);
    assert_eq!(seen.lock().unwrap().len(), 3);
    engine.disconnect(identity).unwrap();
    assert!(matches!(
        engine.review_permissions(request).await,
        Err(ConnectionError::StaleSession)
    ));
}
#[tokio::test]
async fn unavailable_malformed_and_oversize_are_unknown() {
    for body in [
        vec![b'x'; 65_537],
        serde_json::to_vec(&json!({"kind":"wrong","status":{"allowed":true}})).unwrap(),
    ] {
        let client = Client::new(
            tower::service_fn(move |_: Request<Body>| {
                let body = body.clone();
                async move { Ok::<_, std::io::Error>(Response::new(Body::from(body))) }
            }),
            "default",
        );
        let item = review_check(
            &client,
            &check(PermissionAction::Get),
            &CancellationToken::new(),
        )
        .await;
        assert_eq!(item.decision, PermissionDecision::Unknown);
        assert_eq!(item.reason, Some(PermissionReason::Failed));
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let client = Client::new(
        tower::service_fn(move |_: Request<Body>| {
            count.fetch_add(1, Ordering::SeqCst);
            async {
                Ok::<_, std::io::Error>(
                    Response::builder()
                        .status(403)
                        .body(Body::from(b"PRIVATE_REASON".to_vec()))
                        .unwrap(),
                )
            }
        }),
        "default",
    );
    let item = review_check(
        &client,
        &check(PermissionAction::Get),
        &CancellationToken::new(),
    )
    .await;
    assert_eq!(item.decision, PermissionDecision::Unknown);
    assert!(!serde_json::to_string(&item).unwrap().contains("PRIVATE"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn fresh_denial_blocks_guard() {
    let (_root, _catalog, engine, id, _) = fixture();
    let opened = engine.open(OpenRequest { cluster_id: id }).await.unwrap();
    let request = identity(&opened);
    let connected = engine
        .connect_with(request.clone(), |_| async {
            Ok(client(
                json!({"allowed":false,"denied":false}),
                Arc::new(Mutex::new(Vec::new())),
            ))
        })
        .await
        .unwrap();
    assert_eq!(connected.status, "connected");
    assert_eq!(
        engine
            .require_permissions(&request, &[check(PermissionAction::Delete)])
            .await,
        Err(ConnectionError::PermissionDenied)
    );
}
