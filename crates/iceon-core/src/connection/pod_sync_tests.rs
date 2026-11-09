use super::*;
use serde_json::json;

fn request() -> ResourceRequest {
    ResourceRequest {
        cluster_id: "cluster".into(),
        session_id: "lease".into(),
        kind: ResourceKind::Pods,
        read_id: 1,
        namespace: Some("default".into()),
        continue_token: None,
        query: None,
        status: None,
        group: None,
    }
}

#[tokio::test]
async fn denied_live_sync_does_not_register_or_list_pods() {
    let (_root, _, engine, id, _) = crate::connection::tests::fixture();
    let reviews = Arc::new(Mutex::new(Vec::new()));
    let opened = crate::connection::tests::connected(
        &engine,
        id,
        crate::connection::tests::denied_permission_client(reviews.clone(), Some(false)),
    )
    .await;
    let mut r = request();
    r.cluster_id = opened.cluster_id;
    r.session_id = opened.session_id;
    assert!(matches!(
        engine.pod_sync_open(r).await,
        Err(ConnectionError::PermissionDenied)
    ));
    assert!(engine.pod_sync.registry.lock().unwrap().is_empty());
    let reviews = reviews.lock().unwrap();
    assert_eq!(reviews[0]["verb"], "list");
    assert_eq!(reviews[0]["namespace"], "default");
}
#[tokio::test]
async fn close_sync_accepts_expired_or_missing_client_but_not_another_lease() {
    for expired in [true, false] {
        let (_root, _, engine, id, _) = crate::connection::tests::fixture();
        let opened = engine
            .open(super::super::OpenRequest { cluster_id: id })
            .await
            .unwrap();
        let scope = crate::connection::tests::identity(&opened);
        let client = Client::new(
            tower::service_fn(|_: Request<Body>| async {
                std::future::pending::<std::result::Result<http::Response<Body>, std::io::Error>>()
                    .await
            }),
            "default",
        );
        engine.session.lock().unwrap().as_mut().unwrap().client =
            Some(crate::connection::tests::allow_permission_reviews(client));
        let mut resource = request();
        resource.cluster_id = scope.cluster_id.clone();
        resource.session_id = scope.session_id.clone();
        let snapshot = engine.pod_sync_open(resource.clone()).await.unwrap();
        let mut close = PodSyncRequest {
            cluster_id: scope.cluster_id.clone(),
            session_id: scope.session_id.clone(),
            sync_id: snapshot.sync_id,
        };
        let slot = engine.pod_sync.lookup(&close).unwrap();
        slot.lock().unwrap().page = Some(inventory(false).page(&resource));
        engine.session.lock().unwrap().as_mut().unwrap().client = expired.then(|| {
            kube::client::ClientBuilder::new(
                tower::service_fn(|_: Request<Body>| async {
                    std::future::pending::<
                            std::result::Result<http::Response<Body>, std::io::Error>,
                        >()
                        .await
                }),
                "default",
            )
            .with_valid_until(Some("2000-01-01T00:00:00Z".parse().unwrap()))
            .build()
        });
        assert!(
            matches!(engine.pod_sync_read(close.clone()), Err(error) if error == if expired { ConnectionError::AuthExpired } else { ConnectionError::NotConnected })
        );
        close.session_id = "other-lease".into();
        assert_eq!(
            engine.pod_sync_close(close.clone()),
            Err(ConnectionError::StaleSession)
        );
        {
            let state = slot.lock().unwrap();
            assert!(!state.cancel.is_cancelled());
            assert!(state.page.is_some());
        }
        close.session_id = scope.session_id;
        engine.pod_sync_close(close.clone()).unwrap();
        engine.pod_sync_close(close).unwrap();
        {
            let state = slot.lock().unwrap();
            assert!(state.cancel.is_cancelled());
            assert!(state.page.is_none());
            assert_eq!(state.status, "stopped");
        }
        tokio::time::timeout(Duration::from_secs(1), async {
            while !engine.pod_sync.tasks.is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        engine.pod_sync.drain().await;
        assert!(engine.pod_sync.registry.lock().unwrap().is_empty());
    }
}

fn pod(name: &str, rv: &str) -> Value {
    json!({"kind":"Pod","metadata":{"name":name,"namespace":"default","resourceVersion":rv,"annotations":{"private":"secret-marker"}},"spec":{"containers":[{"name":"app","env":[{"name":"PASSWORD","value":"secret-marker"}]}]},"status":{"phase":"Running"}})
}
fn inventory(truncated: bool) -> Inventory {
    parse_list(json!({"kind":"PodList","metadata":{"resourceVersion":"10","continue":if truncated {"next"} else {""}},"items":[pod("api","10")]}), Some("default")).unwrap()
}
fn fake(status: u16, body: Vec<u8>) -> Client {
    Client::new(
        tower::service_fn(move |_r: Request<Body>| {
            let body = body.clone();
            async move {
                Ok::<_, std::io::Error>(
                    http::Response::builder()
                        .status(status)
                        .body(Body::from(body))
                        .unwrap(),
                )
            }
        }),
        "default",
    )
}
#[test]
fn sync_start_from_native_command_thread_uses_app_runtime_and_drains() {
    let app_runtime = crate::runtime::handle();
    let client = {
        let _entered = app_runtime.enter();
        fake(403, Vec::new())
    };
    assert!(tokio::runtime::Handle::try_current().is_err());
    let runtime = PodSyncRuntime::default();
    let snapshot = runtime
        .start(client, request(), CancellationToken::new())
        .unwrap();
    assert_eq!(snapshot.status, "opening");
    assert!(snapshot.page.is_none());
    crate::runtime::block_on(async {
        tokio::time::timeout(Duration::from_secs(1), runtime.drain())
            .await
            .unwrap();
    });
    assert!(runtime.tasks.is_empty());
    assert!(runtime.registry.lock().unwrap().is_empty());
}
#[test]
fn requests_only_allow_first_pod_page_and_valid_scope() {
    let mut r = request();
    assert!(validate(&r).is_ok());
    r.kind = ResourceKind::Secrets;
    assert_eq!(validate(&r), Err(ConnectionError::InvalidInput));
    r.kind = ResourceKind::Pods;
    r.continue_token = Some(String::new());
    assert_eq!(validate(&r), Err(ConnectionError::InvalidInput));
    r.continue_token = None;
    r.namespace = Some("../secret".into());
    assert_eq!(validate(&r), Err(ConnectionError::InvalidInput));
    r.namespace = None;
    r.read_id = 0;
    assert_eq!(validate(&r), Err(ConnectionError::InvalidInput));
}
#[test]
fn list_validates_shape_scope_version_duplicates_and_item_bound() {
    for value in [
        json!({"kind":"SecretList","metadata":{"resourceVersion":"1"},"items":[]}),
        json!({"kind":"PodList","items":[]}),
        json!({"kind":"PodList","metadata":{"resourceVersion":"x".repeat(257)},"items":[]}),
        json!({"kind":"PodList","metadata":{"resourceVersion":"1"},"items":[pod("a","1"),pod("a","1")]}),
        json!({"kind":"PodList","metadata":{"resourceVersion":"1"},"items":[{"metadata":{"name":"a","namespace":"other"}}]}),
    ] {
        assert!(matches!(
            parse_list(value, Some("default")),
            Err(ConnectionError::InvalidResponse)
        ));
    }
    let too_many = (0..101)
        .map(|i| pod(&format!("pod-{i}"), "1"))
        .collect::<Vec<_>>();
    assert!(matches!(
        parse_list(
            json!({"kind":"PodList","metadata":{"resourceVersion":"1"},"items":too_many}),
            None
        ),
        Err(ConnectionError::LimitExceeded)
    ));
    let page = inventory(true).page(&request());
    assert!(page.truncated);
    assert!(page.continue_token.is_none());
    assert!(
        !serde_json::to_string(&page)
            .unwrap()
            .contains("secret-marker")
    );
}
#[test]
fn watch_updates_deletes_bookmarks_and_restarts_expired_history() {
    let mut inv = inventory(false);
    assert_eq!(
        inv.apply(WatchEvent::Added(pod("worker", "11")), None)
            .unwrap(),
        WatchStep::Continue
    );
    let mut changed = pod("worker", "12");
    changed["status"]["phase"] = "Pending".into();
    inv.apply(WatchEvent::Modified(changed), None).unwrap();
    assert_eq!(
        inv.rows[&("default".into(), "worker".into())].status,
        "Pending"
    );
    inv.apply(WatchEvent::Deleted(pod("worker", "13")), None)
        .unwrap();
    assert_eq!(inv.rows.len(), 1);
    inv.apply(
        WatchEvent::Bookmark(json!({"metadata":{"resourceVersion":"14"}})),
        None,
    )
    .unwrap();
    assert_eq!(inv.version, "14");
    assert_eq!(
        inv.apply(
            WatchEvent::Error(json!({"code":410,"message":"secret-marker"})),
            None
        )
        .unwrap(),
        WatchStep::Relist
    );
    assert_eq!(
        inv.apply(
            WatchEvent::Error(json!({"code":403,"message":"secret-marker"})),
            None
        ),
        Err(ConnectionError::PermissionDenied)
    );
}
#[test]
fn partial_inventory_never_claims_unseen_additions_are_a_complete_page() {
    let mut inv = inventory(true);
    assert_eq!(
        inv.apply(WatchEvent::Added(pod("new", "11")), None)
            .unwrap(),
        WatchStep::Relist
    );
    assert_eq!(inv.rows.len(), 1);
    assert_eq!(
        inv.apply(WatchEvent::Deleted(pod("api", "12")), None)
            .unwrap(),
        WatchStep::Relist
    );
    let mut inv = inventory(false);
    for i in 0..99 {
        inv.apply(WatchEvent::Added(pod(&format!("p-{i}"), "11")), None)
            .unwrap();
    }
    assert_eq!(inv.rows.len(), 100);
    assert_eq!(
        inv.apply(WatchEvent::Added(pod("overflow", "12")), None)
            .unwrap(),
        WatchStep::Relist
    );
    assert_eq!(inv.rows.len(), 100);
    assert!(inv.truncated);
}
#[test]
fn chunk_parser_handles_split_events_crlf_and_multiple_events() {
    let text = format!(
        "{}\r\n{}\n",
        json!({"type":"ADDED","object":pod("api","11")}),
        json!({"type":"BOOKMARK","object":{"metadata":{"resourceVersion":"12"}}})
    );
    let mut lines = Lines::default();
    let mut inv = inventory(false);
    for chunk in text.as_bytes().chunks(3) {
        lines.push(chunk, |event| inv.apply(event, None)).unwrap();
        assert!(lines.pending.len() <= EVENT_LIMIT);
    }
    lines.finish().unwrap();
    assert_eq!(inv.version, "12");
    assert_eq!(inv.rows.len(), 1);
}
#[test]
fn event_limit_is_enforced_before_json_and_incomplete_eof_fails() {
    let mut lines = Lines::default();
    lines
        .push(&vec![b' '; EVENT_LIMIT], |_| unreachable!())
        .unwrap();
    assert_eq!(
        lines.push(b"x", |_| unreachable!()),
        Err(ConnectionError::LimitExceeded)
    );
    assert_eq!(lines.pending.len(), EVENT_LIMIT);
    let mut lines = Lines::default();
    lines.push(b"{", |_| unreachable!()).unwrap();
    assert_eq!(lines.finish(), Err(ConnectionError::InvalidResponse));
    let mut lines = Lines::default();
    assert_eq!(
        lines.push(
            b"{\"type\":\"UNKNOWN\",\"object\":{}}\n",
            |_| unreachable!()
        ),
        Err(ConnectionError::InvalidResponse)
    );
}
#[tokio::test]
async fn list_body_limits_and_http_errors_are_redacted() {
    for (status, body, expected) in [
        (
            403,
            b"secret-marker".to_vec(),
            ConnectionError::PermissionDenied,
        ),
        (
            200,
            vec![b' '; LIST_LIMIT + 1],
            ConnectionError::LimitExceeded,
        ),
        (
            200,
            b"secret-marker".to_vec(),
            ConnectionError::InvalidResponse,
        ),
    ] {
        assert!(
            matches!(list(&fake(status, body), &request(), &CancellationToken::new()).await, Err(error) if error == expected)
        );
    }
}
#[tokio::test]
async fn watch_uses_fixed_scope_and_resource_version_query() {
    let client = Client::new(
        tower::service_fn(move |r: Request<Body>| async move {
            assert_eq!(r.method(), http::Method::GET);
            assert_eq!(r.uri().path(), "/api/v1/namespaces/default/pods");
            let query: HashMap<_, _> =
                url::form_urlencoded::parse(r.uri().query().unwrap().as_bytes())
                    .into_owned()
                    .collect();
            assert_eq!(query["watch"], "true");
            assert_eq!(query["resourceVersion"], "10");
            assert_eq!(query["timeoutSeconds"], "10");
            assert!(r.extensions().get::<CancellationToken>().is_some());
            Ok::<_, std::io::Error>(
                http::Response::builder()
                    .status(410)
                    .body(Body::from(b"private error".to_vec()))
                    .unwrap(),
            )
        }),
        "default",
    );
    let state = Mutex::new(SyncState {
        request: request(),
        id: "sync".into(),
        cancel: CancellationToken::new(),
        sequence: 0,
        page: None,
        status: "opening",
        error: None,
    });
    assert_eq!(
        watch(
            &client,
            &request(),
            &CancellationToken::new(),
            &mut inventory(false),
            &state
        )
        .await
        .unwrap(),
        WatchStep::Relist
    );
}
#[tokio::test]
async fn registry_scopes_limits_cancellation_and_drain() {
    let client = Client::new(
        tower::service_fn(move |_r: Request<Body>| async move {
            std::future::pending::<std::result::Result<http::Response<Body>, std::io::Error>>()
                .await
        }),
        "default",
    );
    let runtime = PodSyncRuntime::default();
    let root = CancellationToken::new();
    let mut snapshots = Vec::new();
    for _ in 0..4 {
        snapshots.push(
            runtime
                .start(client.clone(), request(), root.child_token())
                .unwrap(),
        );
    }
    assert!(matches!(
        runtime.start(client.clone(), request(), root.child_token()),
        Err(ConnectionError::Busy)
    ));
    let mut read = PodSyncRequest {
        cluster_id: "cluster".into(),
        session_id: "wrong".into(),
        sync_id: snapshots[0].sync_id.clone(),
    };
    assert!(matches!(
        runtime.lookup(&read),
        Err(ConnectionError::StaleSession)
    ));
    read.session_id = "lease".into();
    let slot = runtime.lookup(&read).unwrap();
    root.cancel();
    tokio::time::timeout(Duration::from_secs(1), runtime.drain())
        .await
        .unwrap();
    assert_eq!(slot.lock().unwrap().status, "stopped");
    assert!(slot.lock().unwrap().page.is_none());
    assert!(runtime.tasks.is_empty());
    assert!(runtime.registry.lock().unwrap().is_empty());
    assert!(matches!(
        runtime.start(client, request(), CancellationToken::new()),
        Err(ConnectionError::StaleSession)
    ));
}
#[tokio::test]
async fn runtime_publishes_list_then_watch_changes_without_raw_payloads() {
    let (seen, received) = tokio::sync::oneshot::channel();
    let seen = Arc::new(Mutex::new(Some(seen)));
    let client = Client::new(
        tower::service_fn(move |r: Request<Body>| {
            let seen = seen.clone();
            async move {
                let body = if r.uri().query().unwrap().contains("watch=true") {
                    if let Some(tx) = seen.lock().unwrap().take() {
                        let _ = tx.send(());
                    }
                    format!("{}\n", json!({"type":"MODIFIED","object":pod("api","11")}))
                } else {
                    json!({"kind":"PodList","metadata":{"resourceVersion":"10"},"items":[pod("api","10")]}).to_string()
                };
                Ok::<_, std::io::Error>(http::Response::new(Body::from(body.into_bytes())))
            }
        }),
        "default",
    );
    let runtime = PodSyncRuntime::default();
    let snapshot = runtime
        .start(client, request(), CancellationToken::new())
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), received)
        .await
        .unwrap()
        .unwrap();
    let read = PodSyncRequest {
        cluster_id: snapshot.cluster_id,
        session_id: snapshot.session_id,
        sync_id: snapshot.sync_id,
    };
    let slot = runtime.lookup(&read).unwrap();
    {
        let state = slot.lock().unwrap();
        assert_eq!(state.status, "syncing");
        assert!(state.sequence >= 1);
        let wire = serde_json::to_string(&state.snapshot()).unwrap();
        assert!(!wire.contains("secret-marker"));
        assert_eq!(state.page.as_ref().unwrap().items.len(), 1);
    }
    runtime.drain().await;
}

#[test]
fn list_and_watch_share_pod_health_projection() {
    let mut value = pod("api", "10");
    value["status"]["conditions"] = json!([{"type":"Ready","status":"False"}]);
    value["status"]["containerStatuses"] = json!([{"name":"app","ready":false,"state":{"waiting":{"reason":"CrashLoopBackOff","message":"secret-marker"}}}]);
    let mut inv = parse_list(
        json!({"kind":"PodList","metadata":{"resourceVersion":"10"},"items":[value.clone()]}),
        Some("default"),
    )
    .unwrap();
    let expected =
        crate::connection::resources::project(ResourceKind::Pods, value.clone()).unwrap();
    assert_eq!(inv.page(&request()).items[0], expected);
    assert_eq!(expected.columns["health"], "warning");
    value["metadata"]["resourceVersion"] = json!("11");
    value["metadata"]["deletionTimestamp"] = json!("2026-10-26T12:00:00Z");
    inv.apply(WatchEvent::Modified(value.clone()), Some("default"))
        .unwrap();
    let expected = crate::connection::resources::project(ResourceKind::Pods, value).unwrap();
    assert_eq!(inv.page(&request()).items[0], expected);
    assert_eq!(expected.status, "Terminating");
    assert_eq!(expected.columns["health"], "terminating");
    assert!(
        !serde_json::to_string(&expected)
            .unwrap()
            .contains("secret-marker")
    );
}
