use super::*;
use crate::connection::tests::{connected, fake, fixture, identity};
use serde_json::json;

fn request(snapshot: &super::super::ConnectionSnapshot) -> RbacUsersRequest {
    RbacUsersRequest {
        cluster_id: snapshot.cluster_id.clone(),
        session_id: snapshot.session_id.clone(),
        read_id: 1,
        namespace: Some("team".into()),
    }
}
fn binding_row() -> Value {
    json!({"metadata":{"name":"team:read","namespace":"team","uid":"uid-1","resourceVersion":"12","annotations":{"private":"PRIVATE"}},"roleRef":{"apiGroup":"rbac.authorization.k8s.io","kind":"ClusterRole","name":"system:reader"},"subjects":[{"kind":"User","apiGroup":"rbac.authorization.k8s.io","name":"alice@example.test","extra":"PRIVATE"},{"kind":"Group","apiGroup":"rbac.authorization.k8s.io","name":"system:authenticated"},{"kind":"ServiceAccount","name":"robot"}]})
}
fn encoded(value: Value) -> (u16, Vec<u8>) {
    (200, serde_json::to_vec(&value).unwrap())
}
fn lists(path: &str) -> (u16, Vec<u8>) {
    match path {
        "/version" => encoded(json!({"gitVersion":"v1.36.0"})),
        "/apis/rbac.authorization.k8s.io/v1/namespaces/team/rolebindings?limit=100&timeoutSeconds=10" => {
            encoded(json!({"kind":"RoleBindingList","items":[binding_row()]}))
        }
        "/apis/rbac.authorization.k8s.io/v1/clusterrolebindings?limit=100&timeoutSeconds=10" => {
            (403, b"PRIVATE server credentials".to_vec())
        }
        "/api/v1/namespaces/team/serviceaccounts?limit=100&timeoutSeconds=10" => encoded(
            json!({"kind":"ServiceAccountList","items":[{"metadata":{"name":"robot","namespace":"team","uid":"sa-1"},"secrets":[{"name":"PRIVATE"}]}]}),
        ),
        _ => panic!("unexpected request: {path}"),
    }
}
#[tokio::test]
async fn scoped_index_retains_allowed_sources_and_projects_only_safe_references() {
    let (_root, _, engine, id, _) = fixture();
    let snapshot = connected(&engine, id, fake(lists)).await;
    let result = engine.rbac_users(request(&snapshot)).await.unwrap();
    assert_eq!(result.items.len(), 3);
    assert_eq!(
        result.coverage.iter().map(|c| c.state).collect::<Vec<_>>(),
        ["complete", "denied", "complete"]
    );
    assert!(!result.truncated);
    let sa = result
        .items
        .iter()
        .find(|s| s.kind == "ServiceAccount")
        .unwrap();
    assert_eq!(sa.namespace.as_deref(), Some("team"));
    assert_eq!(sa.bindings.len(), 1);
    assert_eq!(sa.bindings[0].uid, "uid-1");
    assert_eq!(sa.bindings[0].resource_version, "12");
    assert_eq!(sa.bindings[0].role_ref.name, "system:reader");
    let wire = serde_json::to_string(&result).unwrap();
    for secret in [
        "PRIVATE",
        "annotations",
        "secrets",
        "extra",
        "token",
        "headers",
    ] {
        assert!(!wire.contains(secret));
    }
    assert_eq!(
        engine.status(identity(&snapshot)).unwrap().status,
        "connected"
    );
}
#[test]
fn closed_requests_and_subject_identity_validation() {
    let base = json!({"clusterId":"cluster","sessionId":"session","readId":1,"namespace":null});
    for key in [
        "url",
        "headers",
        "credentials",
        "subjects",
        "continueToken",
        "kind",
    ] {
        let mut wire = base.clone();
        wire[key] = json!("PRIVATE");
        assert!(serde_json::from_value::<RbacUsersRequest>(wire).is_err());
    }
    for ns in ["Team", "../team", "team/other", ""] {
        let mut wire = base.clone();
        wire["namespace"] = json!(ns);
        assert_eq!(
            serde_json::from_value::<RbacUsersRequest>(wire)
                .unwrap()
                .validate(),
            Err(ConnectionError::InvalidInput)
        );
    }
    for kind in ["user", "group", "serviceaccount", "Unknown"] {
        assert!(
            subject(
                &json!({"kind":kind,"name":"name","apiGroup":"rbac.authorization.k8s.io"}),
                None
            )
            .is_none()
        );
    }
    for name in ["bad\nname", "bad\u{202e}name", ""] {
        assert!(
            subject(
                &json!({"kind":"User","name":name,"apiGroup":"rbac.authorization.k8s.io"}),
                None
            )
            .is_none()
        );
    }
    assert!(
        subject(
            &json!({"kind":"User","name":"a".repeat(1025),"apiGroup":"rbac.authorization.k8s.io"}),
            None
        )
        .is_none()
    );
    let record = subject(
        &json!({"kind":"User","name":"system:a@b:c","apiGroup":"rbac.authorization.k8s.io"}),
        None,
    )
    .unwrap();
    assert_eq!(record.name, "system:a@b:c");
    assert_eq!(record.namespace, None);
}
#[tokio::test]
async fn duplicates_unknown_subjects_and_pages_are_not_complete() {
    let (_root, _, engine, id, _) = fixture();
    let snapshot = connected(&engine,id,fake(|path| {
        if path.contains("/rolebindings?") {
            let mut row = binding_row();
            row["subjects"].as_array_mut().unwrap().push(json!({"kind":"Robot","name":"unknown"}));
            encoded(json!({"kind":"RoleBindingList","metadata":{"continue":"repeat"},"items":[row.clone(),row]}))
        } else { lists(path) }
    })).await;
    let result = engine.rbac_users(request(&snapshot)).await.unwrap();
    assert_eq!(result.coverage[0].state, "partial");
    assert!(result.truncated);
    assert_eq!(result.items.len(), 3);
    assert!(result.items.iter().all(|item| item.bindings.len() == 1));
}
#[tokio::test]
async fn empty_is_a_source_snapshot_not_a_directory_and_all_denied_is_explicit() {
    for denied in [false, true] {
        let (_root, _, engine, id, _) = fixture();
        let snapshot = connected(
            &engine,
            id,
            fake(move |path| {
                if path == "/version" {
                    return lists(path);
                }
                if denied {
                    return (403, vec![]);
                }
                let kind = if path.contains("/clusterrolebindings?") {
                    "ClusterRoleBindingList"
                } else if path.contains("/rolebindings?") {
                    "RoleBindingList"
                } else {
                    "ServiceAccountList"
                };
                encoded(json!({"kind":kind,"items":[]}))
            }),
        )
        .await;
        let mut req = request(&snapshot);
        req.namespace = None;
        let result = engine.rbac_users(req).await.unwrap();
        assert!(result.items.is_empty());
        assert!(
            result
                .coverage
                .iter()
                .all(|c| c.state == if denied { "denied" } else { "complete" })
        );
    }
}
#[tokio::test]
async fn cancellation_is_exact_monotonic_and_session_bound() {
    let (_root, _, engine, id, _) = fixture();
    let snapshot = connected(&engine, id, fake(lists)).await;
    let req = request(&snapshot);
    engine.cancel_rbac_users(req.clone()).unwrap();
    assert_eq!(
        engine.rbac_users(req.clone()).await,
        Err(ConnectionError::StaleSession)
    );
    let mut wrong = req.clone();
    wrong.namespace = None;
    assert_eq!(
        engine.cancel_rbac_users(wrong),
        Err(ConnectionError::InvalidInput)
    );
    let mut next = req.clone();
    next.read_id = 2;
    engine.rbac_users(next.clone()).await.unwrap();
    engine.cancel_rbac_users(req).unwrap();
    next.read_id = 3;
    engine.disconnect(identity(&snapshot)).unwrap();
    assert_eq!(
        engine.rbac_users(next).await,
        Err(ConnectionError::StaleSession)
    );
}
#[tokio::test]
async fn inflight_cancel_stops_http_and_releases_worker() {
    use http::Response;
    let (_root, _, engine, id, _) = fixture();
    let started = std::sync::Arc::new(tokio::sync::Notify::new());
    let notification = started.clone();
    let client = Client::new(
        super::super::transport::CancelService(tower::service_fn(move |req: Request<Body>| {
            let notification = notification.clone();
            async move {
                if req.uri().path() == "/version" {
                    return Ok::<_, std::io::Error>(Response::new(Body::from(
                        r#"{"gitVersion":"v1.36.0"}"#.as_bytes().to_vec(),
                    )));
                }
                notification.notify_one();
                std::future::pending::<std::result::Result<Response<Body>, std::io::Error>>().await
            }
        })),
        "default",
    );
    let snapshot = connected(&engine, id, client).await;
    let req = request(&snapshot);
    let read = engine.rbac_users(req.clone());
    let cancel = async {
        started.notified().await;
        engine.cancel_rbac_users(req).unwrap();
    };
    let (result, ()) = tokio::join!(read, cancel);
    assert_eq!(result, Err(ConnectionError::StaleSession));
    assert_eq!(engine.workers.available_permits(), 4);
}
#[test]
fn bounded_subject_and_binding_output() {
    let mut items = BTreeMap::new();
    let mut link = binding(&binding_row(), false, Some("team")).unwrap();
    for n in 0..65 {
        link.uid = format!("uid-{n}");
        let record = subject(
            &json!({"kind":"User","name":"a","apiGroup":"rbac.authorization.k8s.io"}),
            None,
        )
        .unwrap();
        assert_eq!(add_subject(&mut items, record, Some(&link)), n < 64);
    }
    let record = items.values().next().unwrap();
    assert_eq!(record.bindings.len(), 64);
    assert!(record.bindings_truncated);
}

#[tokio::test]
async fn requests_are_get_only_with_json_accept_and_no_renderer_headers() {
    use http::Response;
    let (_root, _, engine, id, _) = fixture();
    let client = Client::new(
        super::super::transport::CancelService(tower::service_fn(
            |req: Request<Body>| async move {
                assert_eq!(req.method(), http::Method::GET);
                assert_eq!(req.headers().get("accept").unwrap(), "application/json");
                assert!(req.headers().get("authorization").is_none());
                assert!(req.headers().get("impersonate-user").is_none());
                assert!(req.headers().get("impersonate-group").is_none());
                let (status, bytes) = lists(&req.uri().to_string());
                Ok::<_, std::io::Error>(
                    Response::builder()
                        .status(status)
                        .body(Body::from(bytes))
                        .unwrap(),
                )
            },
        )),
        "default",
    );
    let snapshot = connected(&engine, id, client).await;
    engine.rbac_users(request(&snapshot)).await.unwrap();
}

#[tokio::test]
async fn oversized_and_invalid_sources_preserve_other_coverage_without_raw_errors() {
    for oversized in [true, false] {
        let (_root, _, engine, id, _) = fixture();
        let snapshot = connected(
            &engine,
            id,
            fake(move |path| {
                if path.contains("/rolebindings?") {
                    (
                        200,
                        if oversized {
                            vec![b' '; BODY_LIMIT + 1]
                        } else {
                            b"PRIVATE malformed response".to_vec()
                        },
                    )
                } else {
                    lists(path)
                }
            }),
        )
        .await;
        let response = engine.rbac_users(request(&snapshot)).await.unwrap();
        assert_eq!(
            response.coverage[0].state,
            if oversized { "partial" } else { "unavailable" }
        );
        assert_eq!(response.coverage[1].state, "denied");
        assert_eq!(response.coverage[2].state, "complete");
        assert_eq!(response.items.len(), 1);
        assert!(
            !serde_json::to_string(&response)
                .unwrap()
                .contains("PRIVATE")
        );
    }
}

#[test]
fn subject_limit_and_composite_ids_are_bounded_and_collision_free() {
    let mut items = BTreeMap::new();
    for n in 0..=SUBJECT_LIMIT {
        let record = subject(&json!({"kind":"User","name":format!("user:{n}"),"apiGroup":"rbac.authorization.k8s.io"}), None).unwrap();
        assert_eq!(add_subject(&mut items, record, None), n < SUBJECT_LIMIT);
    }
    assert_eq!(items.len(), SUBJECT_LIMIT);
    let user = subject(
        &json!({"kind":"User","name":"same","apiGroup":"rbac.authorization.k8s.io"}),
        None,
    )
    .unwrap();
    let group = subject(
        &json!({"kind":"Group","name":"same","apiGroup":"rbac.authorization.k8s.io"}),
        None,
    )
    .unwrap();
    assert_ne!(user.id, group.id);
}
