use super::tests::{connected, fake, fixture, normal, resource};
use super::*;
use serde_json::{Value, json};

fn facts() -> Value {
    json!({"metadata":{"name":"node-a","creationTimestamp":"2026-10-25T01:02:03.123Z","labels":{
        "node-role.kubernetes.io/worker":"PRIVATE_VALUE", "node-role.kubernetes.io/control-plane":"",
        "kubernetes.io/role":"worker", "unrelated":"PRIVATE_LABEL"},"annotations":{"token":"PRIVATE_ANNOTATION"}},
        "spec":{"unschedulable":true,"taints":[{"key":"PRIVATE_KEY","value":"PRIVATE_TAINT","effect":"NoSchedule"}]},
        "status":{"conditions":[{"type":"Ready","status":"True","reason":"PRIVATE_REASON","message":"PRIVATE_MESSAGE"},
        {"type":"MemoryPressure","status":"False"},{"type":"CustomCondition","status":"Unknown"}],
        "addresses":[{"type":"InternalIP","address":"10.0.0.1"},{"type":"InternalIP","address":"2001:db8::1"},
        {"type":"InternalIP","address":"10.0.0.1"},{"type":"ExternalIP","address":"PRIVATE_EXTERNAL"}],
        "nodeInfo":{"kubeletVersion":"v1.36.0"},"allocatable":{"cpu":"7500m","memory":"16Gi","ephemeral-storage":"123456Ki"}}})
}
async fn read(nodes: Value, metrics_status: u16, metrics_body: Vec<u8>) -> Result<ResourcePage> {
    let (_root, _catalog, engine, id, _) = fixture();
    let snapshot = connected(
        &engine,
        id,
        fake(move |path| {
            if path.starts_with("/apis/metrics.") {
                assert_eq!(
                    path,
                    "/apis/metrics.k8s.io/v1beta1/nodes?limit=100&timeoutSeconds=3"
                );
                (metrics_status, metrics_body.clone())
            } else if path.starts_with("/api/v1/nodes?") {
                (200, json!({"items":nodes}).to_string().into_bytes())
            } else {
                normal(path)
            }
        }),
    )
    .await;
    engine
        .resources(resource(&snapshot, ResourceKind::Nodes))
        .await
}
#[tokio::test]
async fn complete_facts_safe_wire_projection_and_name_only_metrics_join() {
    let timestamp = jiff::Timestamp::now().to_string();
    let metrics = json!({"items":[
        {"metadata":{"name":"other-node"},"timestamp":timestamp,"usage":{"cpu":"999m","memory":"999Gi"}},
        {"metadata":{"name":"node-a"},"timestamp":timestamp,"usage":{"cpu":"123456n","memory":"123Ki"}}
    ]});
    let page = read(
        json!([facts(), {"metadata":{"name":"node-b"}}]),
        200,
        metrics.to_string().into_bytes(),
    )
    .await
    .unwrap();
    assert_eq!(page.metrics, Some(MetricState::Partial));
    let node = page.items[0].node.as_ref().unwrap();
    assert_eq!(
        node,
        &NodeInfo {
            roles: vec!["control-plane".into(), "worker".into()],
            taints: 1,
            version: Some("v1.36.0".into()),
            internal_ips: vec!["10.0.0.1".into(), "2001:db8::1".into()],
            schedulable: false,
            conditions: vec![
                NodeCondition {
                    kind: "Ready".into(),
                    status: "True".into()
                },
                NodeCondition {
                    kind: "MemoryPressure".into(),
                    status: "False".into()
                },
                NodeCondition {
                    kind: "CustomCondition".into(),
                    status: "Unknown".into()
                }
            ],
            cpu: Usage {
                used: Some(0.000123456),
                allocatable: Some(7.5),
                capacity: None
            },
            memory: Usage {
                used: Some(125952.0),
                allocatable: Some(17179869184.0),
                capacity: None
            },
            disk_allocatable: Some("123456Ki".into()),
            metrics_timestamp: Some(timestamp),
        }
    );
    let missing = page.items[1].node.as_ref().unwrap();
    assert!(
        missing.schedulable
            && missing.roles.is_empty()
            && missing.conditions.is_empty()
            && missing.internal_ips.is_empty()
    );
    assert_eq!(page.items[1].created_at, None);
    assert_eq!(missing.version, None);
    assert_eq!(missing.cpu.used, None);
    let wire = serde_json::to_value(&page).unwrap();
    assert_eq!(wire["metrics"], "partial");
    assert_eq!(wire["items"][0]["createdAt"], "2026-10-25T01:02:03.123Z");
    assert_eq!(wire["items"][0]["node"]["version"], "v1.36.0");
    assert_eq!(
        wire["items"][0]["node"]["conditions"][0],
        json!({"type":"Ready","status":"True"})
    );
    assert!(!wire.to_string().contains("PRIVATE"));
}

#[tokio::test]
async fn diagnostic_search_matches_only_safe_typed_node_facts() {
    let (_root, _catalog, engine, id, _) = fixture();
    let mut node = facts();
    node["status"]["conditions"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":"HostUpgrades","status":"True"}));
    let snapshot = connected(
        &engine,
        id,
        fake(move |path| {
            if path.starts_with("/apis/metrics.") {
                (404, b"{}".to_vec())
            } else if path.starts_with("/api/v1/nodes?") {
                (
                    200,
                    json!({"items":[node, {"metadata":{"name":"node-a-other"},
                    "status":{"conditions":[{"type":"Ready","status":"True"}]}}]})
                    .to_string()
                    .into_bytes(),
                )
            } else {
                normal(path)
            }
        }),
    )
    .await;
    for (index, query) in [
        "10.0.0.1",
        "2001:DB8::1",
        "mEmOrYpReSsUrE",
        "FALSE",
        "customcondition",
        "uNkNoWn",
        "HOSTUPGRADES",
        "CONTROL-PLANE",
        "WoRkEr",
        "V1.36.0",
        "PRIVATE_VALUE",
        "PRIVATE_LABEL",
        "PRIVATE_ANNOTATION",
        "PRIVATE_TAINT",
        "PRIVATE_REASON",
        "PRIVATE_MESSAGE",
        "PRIVATE_EXTERNAL",
        "no-match",
    ]
    .into_iter()
    .enumerate()
    {
        let mut request = resource(&snapshot, ResourceKind::Nodes);
        request.read_id = index as u64 + 1;
        request.query = Some(query.into());
        let page = engine.resources(request).await.unwrap();
        let expected = if index < 10 { vec!["node-a"] } else { vec![] };
        assert_eq!(
            page.items
                .iter()
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>(),
            expected,
            "query: {query}"
        );
    }
    let mut request = resource(&snapshot, ResourceKind::Nodes);
    request.read_id = 100;
    request.query = Some("2001:db8::1".into());
    request.status = Some("NotReady".into());
    assert!(engine.resources(request).await.unwrap().items.is_empty());
}
#[tokio::test]
async fn optional_metrics_errors_never_discard_or_partially_enrich_facts() {
    let valid = json!({"metadata":{"name":"node-a"},"timestamp":jiff::Timestamp::now().to_string(),"usage":{"cpu":"1m","memory":"2Ki"}});
    for (status, body, expected) in [
        (403, b"PRIVATE API error".to_vec(), MetricState::Denied),
        (
            404,
            b"PRIVATE unavailable".to_vec(),
            MetricState::Unavailable,
        ),
        (503, b"PRIVATE unavailable".to_vec(), MetricState::Error),
        (504, b"PRIVATE timeout".to_vec(), MetricState::Error),
        (200, b"PRIVATE malformed".to_vec(), MetricState::Error),
        (200, vec![b'x'; 2 * 1024 * 1024 + 1], MetricState::Error),
        (
            200,
            json!({"items":vec![valid.clone();101]})
                .to_string()
                .into_bytes(),
            MetricState::Error,
        ),
        (
            200,
            json!({"items":[valid.clone(),valid.clone()]})
                .to_string()
                .into_bytes(),
            MetricState::Error,
        ),
        (
            200,
            json!({"items":[valid.clone(),{"metadata":{"name":"other"},"usage":{"cpu":"NaN"}}]})
                .to_string()
                .into_bytes(),
            MetricState::Error,
        ),
        (
            200,
            json!({"items":[{"metadata":{"name":"INVALID"},"usage":{}}]})
                .to_string()
                .into_bytes(),
            MetricState::Error,
        ),
    ] {
        let page = read(json!([facts()]), status, body).await.unwrap();
        assert_eq!(page.metrics, Some(expected));
        let node = page.items[0].node.as_ref().unwrap();
        assert_eq!(node.cpu.allocatable, Some(7.5));
        assert_eq!(node.cpu.used, None);
        assert_eq!(node.memory.used, None);
        assert!(!serde_json::to_string(&page).unwrap().contains("PRIVATE"));
    }
}
#[tokio::test]
async fn invalid_projected_facts_fail_the_read() {
    let labels: serde_json::Map<String, Value> = (0..101)
        .map(|index| (format!("node-role.kubernetes.io/role-{index}"), json!("")))
        .collect();
    let mut too_many_roles = facts();
    too_many_roles["metadata"]["labels"] = Value::Object(labels);
    assert_eq!(
        read(json!([too_many_roles]), 200, br#"{"items":[]}"#.to_vec()).await,
        Err(ConnectionError::LimitExceeded)
    );
    for (pointer, value, expected) in [
        (
            "/metadata/creationTimestamp",
            json!("not-a-time"),
            ConnectionError::InvalidResponse,
        ),
        (
            "/metadata/creationTimestamp",
            json!("x".repeat(65)),
            ConnectionError::InvalidResponse,
        ),
        (
            "/metadata/labels/kubernetes.io~1role",
            json!("bad\nrole"),
            ConnectionError::InvalidResponse,
        ),
        (
            "/status/nodeInfo/kubeletVersion",
            json!("x".repeat(257)),
            ConnectionError::InvalidResponse,
        ),
        (
            "/status/addresses/0/address",
            json!("not-an-ip"),
            ConnectionError::InvalidResponse,
        ),
        (
            "/status/conditions/0/type",
            json!("bad\ncondition"),
            ConnectionError::InvalidResponse,
        ),
        (
            "/status/conditions/0/status",
            json!("PRIVATE"),
            ConnectionError::InvalidResponse,
        ),
        (
            "/status/conditions",
            json!([{"type":"Ready","status":"True"},{"type":"Ready","status":"False"}]),
            ConnectionError::InvalidResponse,
        ),
        (
            "/status/allocatable/cpu",
            json!("NaN"),
            ConnectionError::InvalidResponse,
        ),
        (
            "/status/allocatable/memory",
            json!("1GB"),
            ConnectionError::InvalidResponse,
        ),
        (
            "/status/allocatable/ephemeral-storage",
            json!("1e+"),
            ConnectionError::InvalidResponse,
        ),
        (
            "/spec/taints",
            json!(vec![json!({}); 101]),
            ConnectionError::LimitExceeded,
        ),
        (
            "/status/conditions",
            json!(vec![json!({"type":"Ready","status":"True"}); 101]),
            ConnectionError::LimitExceeded,
        ),
        (
            "/status/addresses",
            json!(vec![json!({"type":"InternalIP","address":"10.0.0.1"}); 101]),
            ConnectionError::LimitExceeded,
        ),
    ] {
        let mut node = facts();
        *node.pointer_mut(pointer).unwrap() = value;
        assert_eq!(
            read(json!([node]), 200, br#"{"items":[]}"#.to_vec()).await,
            Err(expected),
            "{pointer}"
        );
    }
}
#[tokio::test]
async fn namespaces_have_neither_node_details_nor_metrics_requests() {
    let (_root, _catalog, engine, id, _) = fixture();
    let snapshot = connected(
        &engine,
        id,
        fake(|path| {
            assert!(!path.starts_with("/apis/"));
            normal(path)
        }),
    )
    .await;
    let page = engine
        .resources(resource(&snapshot, ResourceKind::Namespaces))
        .await
        .unwrap();
    assert_eq!(page.metrics, None);
    assert_eq!(page.items[0].node, None);
}
#[tokio::test]
async fn hanging_metrics_times_out_without_losing_facts() {
    let (_root, _catalog, engine, id, _) = fixture();
    let client = kube::Client::new(
        transport::CancelService(tower::service_fn(
            |request: http::Request<kube::client::Body>| async move {
                if request.uri().path().starts_with("/apis/") {
                    std::future::pending::<()>().await;
                }
                Ok::<_, std::io::Error>(http::Response::new(kube::client::Body::from(
                    normal(&request.uri().to_string()).1,
                )))
            },
        )),
        "default",
    );
    let snapshot = connected(&engine, id, client).await;
    let page = engine
        .resources(resource(&snapshot, ResourceKind::Nodes))
        .await
        .unwrap();
    assert_eq!(page.metrics, Some(MetricState::Error));
    assert_eq!(page.items.len(), 2);
}
#[tokio::test]
async fn disposal_during_metrics_returns_stale_session_not_optional_error() {
    let (_root, _catalog, engine, id, _) = fixture();
    let started = Arc::new(tokio::sync::Notify::new());
    let marker = started.clone();
    let client = kube::Client::new(
        transport::CancelService(tower::service_fn(
            move |request: http::Request<kube::client::Body>| {
                let marker = marker.clone();
                async move {
                    if request.uri().path().starts_with("/apis/") {
                        marker.notify_one();
                        std::future::pending::<()>().await;
                    }
                    Ok::<_, std::io::Error>(http::Response::new(kube::client::Body::from(
                        normal(&request.uri().to_string()).1,
                    )))
                }
            },
        )),
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
}
