use super::*;
use crate::connection::tests::{connected, fixture};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Api {
    writes: Vec<(String, Value)>,
    pods: Vec<Value>,
    node_rv: String,
    cordoned: bool,
    continue_token: bool,
    eviction_status: u16,
    controller_missing: bool,
    inject_after_eviction: bool,
    replace_after_eviction: bool,
    deny: bool,
    denied_attributes: Vec<Value>,
    reviewed_attributes: Vec<Value>,
    reads: Vec<String>,
    permission_checks: usize,
    gate: Option<Arc<tokio::sync::Barrier>>,
    cordon_gate: Option<Arc<tokio::sync::Barrier>>,
    read_gate: Option<Arc<tokio::sync::Barrier>>,
    delete_terminating_on_get: bool,
    omit_list_types: bool,
    contradictory_list_type: bool,
}

#[tokio::test]
async fn list_items_without_typemeta_and_shared_controller_drain_normally() {
    let (_root, _catalog, engine, id, _) = fixture();
    let api = Arc::new(Mutex::new(Api {
        pods: vec![pod("one"), pod("two")],
        omit_list_types: true,
        ..Api::default()
    }));
    let s = connected(&engine, id, fake(api.clone())).await;
    let review = engine
        .node_drain_review(request(s.cluster_id.clone(), s.session_id.clone(), 1))
        .await
        .unwrap();
    assert!(review.can_drain);
    let controller_get = json!({"group":"apps", "version":"v1", "resource":"replicasets", "verb":"get", "namespace":"team-a", "name":"replicas"});
    assert_eq!(
        api.lock()
            .unwrap()
            .reviewed_attributes
            .iter()
            .filter(|a| **a == controller_get)
            .count(),
        1
    );
    let result = engine
        .node_drain_execute(DrainExecuteRequest {
            cluster_id: s.cluster_id,
            session_id: s.session_id,
            operation_id: 2,
            plan_id: review.plan_id.unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(result.state, DrainState::Complete);
    assert!(
        result
            .pods
            .iter()
            .all(|p| p.state == DrainOutcomeState::Evicted)
    );
    assert_eq!(api.lock().unwrap().writes.len(), 3);
}

#[tokio::test]
async fn contradictory_list_item_type_is_rejected_before_writes() {
    let (_root, _catalog, engine, id, _) = fixture();
    let api = Arc::new(Mutex::new(Api {
        pods: vec![pod("one")],
        contradictory_list_type: true,
        ..Api::default()
    }));
    let s = connected(&engine, id, fake(api.clone())).await;
    assert_eq!(
        engine
            .node_drain_review(request(s.cluster_id, s.session_id, 1))
            .await
            .unwrap_err(),
        ConnectionError::InvalidResponse
    );
    assert!(api.lock().unwrap().writes.is_empty());
}
#[tokio::test]
async fn same_name_replacement_at_final_inventory_is_a_new_uid() {
    let (_root, _catalog, engine, id, _) = fixture();
    let api = Arc::new(Mutex::new(Api {
        pods: vec![pod("one")],
        replace_after_eviction: true,
        ..Api::default()
    }));
    let s = connected(&engine, id, fake(api.clone())).await;
    let review = engine
        .node_drain_review(request(s.cluster_id.clone(), s.session_id.clone(), 1))
        .await
        .unwrap();
    let result = engine
        .node_drain_execute(DrainExecuteRequest {
            cluster_id: s.cluster_id,
            session_id: s.session_id,
            operation_id: 2,
            plan_id: review.plan_id.unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(result.state, DrainState::Partial);
    assert_eq!(result.pods.len(), 2);
    assert_eq!(result.pods[0].uid, "uid-one");
    assert_eq!(result.pods[0].state, DrainOutcomeState::Gone);
    assert_eq!(result.pods[1].name, "one");
    assert_eq!(result.pods[1].uid, "replacement-uid");
    assert_eq!(result.pods[1].reason, Some(DrainOutcomeReason::NewPods));
    assert_eq!(api.lock().unwrap().writes.len(), 2);
}
#[tokio::test]
async fn cancellation_while_cordon_response_is_withheld_is_uncertain() {
    let (_root, _catalog, engine, id, _) = fixture();
    let gate = Arc::new(tokio::sync::Barrier::new(2));
    let api = Arc::new(Mutex::new(Api {
        pods: vec![pod("one")],
        cordon_gate: Some(gate.clone()),
        ..Api::default()
    }));
    let s = connected(&engine, id, fake(api.clone())).await;
    let review = engine
        .node_drain_review(request(s.cluster_id.clone(), s.session_id.clone(), 1))
        .await
        .unwrap();
    let execute = engine.node_drain_execute(DrainExecuteRequest {
        cluster_id: s.cluster_id.clone(),
        session_id: s.session_id.clone(),
        operation_id: 2,
        plan_id: review.plan_id.unwrap(),
    });
    let cancel = async {
        gate.wait().await;
        engine
            .node_drain_cancel(DrainCancel {
                cluster_id: s.cluster_id.clone(),
                session_id: s.session_id.clone(),
                operation_id: 2,
            })
            .unwrap();
        assert!(
            engine
                .session
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .node_drain
                .active()
        );
        assert!(matches!(
            engine.drain_start(identity(&s.cluster_id, &s.session_id, 3).unwrap(), 3),
            Err(ConnectionError::Busy)
        ));
    };
    let (result, ()) = tokio::join!(execute, cancel);
    let result = result.unwrap();
    assert_eq!(result.state, DrainState::Uncertain);
    assert!(!result.cordoned);
    assert_eq!(result.pods[0].state, DrainOutcomeState::NotSubmitted);
    assert_eq!(result.pods[0].reason, Some(DrainOutcomeReason::Cancelled));
    let state = engine.session.lock().unwrap();
    let slot = &state.as_ref().unwrap().node_drain;
    assert!(!slot.active());
    assert!(slot.plan.is_none());
    assert_eq!(api.lock().unwrap().writes.len(), 1);
}
fn node_value(api: &Api) -> Value {
    json!({"apiVersion":"v1","kind":"Node", "metadata":{"name":"worker","uid":"node-uid","resourceVersion":if api.node_rv.is_empty() { "7" } else { &api.node_rv }},
        "spec":{"unschedulable":api.cordoned,"taints":[{"key":"preserved"}]}})
}
fn pod(name: &str) -> Value {
    json!({"apiVersion":"v1","kind":"Pod", "metadata":{"name":name,"namespace":"team-a","uid":format!("uid-{name}"),"resourceVersion":"1",
        "ownerReferences":[{"apiVersion":"apps/v1","kind":"ReplicaSet","name":"replicas","uid":"controller-uid","controller":true}]},
        "spec":{"nodeName":"worker","containers":[{"name":"app","image":"fixture"}]}})
}
fn request(cluster: String, session: String, operation_id: u64) -> DrainReviewRequest {
    DrainReviewRequest {
        cluster_id: cluster,
        session_id: session,
        operation_id,
        reference: ResourceReference {
            kind: ResourceKind::Nodes,
            namespace: None,
            name: "worker".into(),
            uid: Some("node-uid".into()),
        },
        resource_version: "7".into(),
    }
}
fn fake(api: Arc<Mutex<Api>>) -> Client {
    Client::new(
        super::super::transport::CancelService(tower::service_fn(move |request: Request<Body>| {
            let api = api.clone();
            async move {
                let method = request.method().clone();
                let path = request.uri().path().to_owned();
                let query = request.uri().query().unwrap_or_default().to_owned();
                let bytes = request.into_body().collect().await.unwrap().to_bytes();
                let body: Value = if bytes.is_empty() {
                    Value::Null
                } else {
                    serde_json::from_slice(&bytes).unwrap()
                };
                let (status, value, gate) = {
                    let mut api = api.lock().unwrap();
                    if path.ends_with("selfsubjectaccessreviews") {
                        api.permission_checks += 1;
                        let attributes = body["spec"]["resourceAttributes"].clone();
                        let denied = api.deny
                            || api.denied_attributes.iter().any(|denied| {
                                denied
                                    .as_object()
                                    .unwrap()
                                    .iter()
                                    .all(|(key, value)| attributes.get(key) == Some(value))
                            });
                        api.reviewed_attributes.push(attributes);
                        (
                            201,
                            json!({"apiVersion":"authorization.k8s.io/v1","kind":"SelfSubjectAccessReview","spec":body["spec"],"status":{"allowed":!denied}}),
                            None,
                        )
                    } else if method == Method::PATCH {
                        assert_eq!(path, "/api/v1/nodes/worker");
                        assert_eq!(body[0]["value"], "node-uid");
                        assert_eq!(body[1]["value"], "7");
                        assert_eq!(body[2]["path"], "/spec/unschedulable");
                        assert_eq!(body.as_array().unwrap().len(), 3);
                        api.writes.push((path, body));
                        api.cordoned = true;
                        api.node_rv = "8".into();
                        (200, node_value(&api), api.cordon_gate.clone())
                    } else if path.ends_with("/eviction") {
                        assert_eq!(method, Method::POST);
                        assert!(api.cordoned);
                        assert_eq!(body["apiVersion"], "policy/v1");
                        assert_eq!(body["deleteOptions"].as_object().unwrap().len(), 1);
                        assert_eq!(
                            body["deleteOptions"]["preconditions"]["resourceVersion"],
                            "1"
                        );
                        let name = body["metadata"]["name"].as_str().unwrap();
                        let status = if api.eviction_status == 0 {
                            201
                        } else {
                            api.eviction_status
                        };
                        if status == 201 {
                            api.pods.retain(|p| p["metadata"]["name"] != name);
                            if api.inject_after_eviction {
                                api.pods.push(pod("replacement"));
                            }
                            if api.replace_after_eviction {
                                let mut replacement = pod(name);
                                replacement["metadata"]["uid"] = json!("replacement-uid");
                                api.pods.push(replacement);
                            }
                        }
                        api.writes.push((path, body));
                        (
                            status,
                            json!({"message":"PRIVATE SERVER TEXT"}),
                            api.gate.clone(),
                        )
                    } else {
                        assert_eq!(method, Method::GET);
                        api.reads.push(path.clone());
                        if path == "/version" {
                            (200, json!({"gitVersion":"v1.35.0"}), None)
                        } else if path == "/api/v1/nodes/worker" {
                            (200, node_value(&api), None)
                        } else if path == "/api/v1/pods" {
                            let pairs: Vec<_> =
                                url::form_urlencoded::parse(query.as_bytes()).collect();
                            assert!(
                                pairs
                                    .iter()
                                    .any(|(k, v)| k == "fieldSelector"
                                        && v == "spec.nodeName=worker")
                            );
                            let mut items = api.pods.clone();
                            if api.omit_list_types {
                                for pod in &mut items {
                                    pod.as_object_mut().unwrap().remove("apiVersion");
                                    pod.as_object_mut().unwrap().remove("kind");
                                }
                            }
                            if api.contradictory_list_type && !items.is_empty() {
                                items[0]["kind"] = json!("Secret");
                            }
                            (
                                200,
                                json!({"apiVersion":"v1","kind":"PodList","metadata":{"continue":if api.continue_token {"next"} else {""}},"items":items}),
                                api.read_gate.clone(),
                            )
                        } else if path.contains("/replicasets/") || path.contains("/daemonsets/") {
                            if api.controller_missing {
                                (404, Value::Null, None)
                            } else {
                                let namespace = path.split('/').nth(5).unwrap();
                                (
                                    200,
                                    json!({"apiVersion":"apps/v1","kind":if path.contains("daemonsets") {"DaemonSet"} else {"ReplicaSet"},
                                "metadata":{"name":"replicas","namespace":namespace,"uid":"controller-uid"}}),
                                    None,
                                )
                            }
                        } else if path.starts_with("/api/v1/namespaces/") && path.contains("/pods/")
                        {
                            if api.cordoned && api.delete_terminating_on_get {
                                api.pods.retain(|p| {
                                    p["metadata"]["name"] != path.rsplit('/').next().unwrap()
                                });
                            }
                            match api.pods.iter().find(|p| {
                                p["metadata"]["name"] == path.rsplit('/').next().unwrap()
                                    && p["metadata"]["namespace"] == path.split('/').nth(4).unwrap()
                            }) {
                                Some(pod) => (200, pod.clone(), None),
                                None => (404, Value::Null, None),
                            }
                        } else {
                            panic!("unexpected fixture URL {path}");
                        }
                    }
                };
                if let Some(gate) = gate {
                    gate.wait().await;
                    gate.wait().await;
                }
                Ok::<_, std::io::Error>(
                    http::Response::builder()
                        .status(status)
                        .body(Body::from(serde_json::to_vec(&value).unwrap()))
                        .unwrap(),
                )
            }
        })),
        "default",
    )
}
#[test]
fn closed_dtos_and_scope() {
    let base = json!({"clusterId":"one","sessionId":"session","operationId":1,"reference":{"kind":"nodes","name":"worker","namespace":null,"uid":"node-uid"},"resourceVersion":"7"});
    let r: DrainReviewRequest = serde_json::from_value(base.clone()).unwrap();
    assert!(validate(&r).is_ok());
    for field in ["force", "deleteEmptyDirData", "ignoreDaemonSets"] {
        let mut v = base.clone();
        v[field] = json!(true);
        assert!(serde_json::from_value::<DrainReviewRequest>(v).is_err());
    }
    assert!(
        serde_json::from_value::<DrainExecuteRequest>(
            json!({"clusterId":"one","sessionId":"session","operationId":2,"planId":"id","pods":[]})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<DrainCancel>(
            json!({"clusterId":"one","sessionId":"session","operationId":2,"planId":"id"})
        )
        .is_err()
    );
    for mutate in 0..3 {
        let mut r = r.clone();
        match mutate {
            0 => r.reference.namespace = Some("team-a".into()),
            1 => r.reference.uid = None,
            _ => r.resource_version.clear(),
        }
        assert_eq!(validate(&r).unwrap_err(), ConnectionError::InvalidInput);
    }
}
#[test]
fn slot_tombstones_overlap_and_update_gate() {
    let mut slot = DrainSlot::default();
    slot.cancel(3);
    assert_eq!(
        slot.begin(3, CancellationToken::new()),
        Err(ConnectionError::StaleSession)
    );
    let token = CancellationToken::new();
    slot.begin(4, token.clone()).unwrap();
    slot.cancel(2);
    assert!(!token.is_cancelled());
    slot.cancel(4);
    assert!(token.is_cancelled());
    assert!(slot.active());
    assert_eq!(
        slot.begin(5, CancellationToken::new()),
        Err(ConnectionError::Busy)
    );
}
#[tokio::test]
async fn classification_is_conservative() {
    let api = Arc::new(Mutex::new(Api::default()));
    let client = fake(api.clone());
    let cancel = CancellationToken::new();
    let classify = |value| {
        let client = client.clone();
        let cancel = cancel.clone();
        async move {
            super::classify(&client, &cancel, "worker", &value)
                .await
                .unwrap()
        }
    };
    assert_eq!(
        classify(pod("managed")).await.disposition,
        DrainDisposition::Evict
    );
    let mut value = pod("unmanaged");
    value["metadata"]
        .as_object_mut()
        .unwrap()
        .remove("ownerReferences");
    value["metadata"]["deletionTimestamp"] = json!("2026-01-01T00:00:00Z");
    assert_eq!(classify(value).await.reason, DrainPodReason::Unmanaged);
    for volume in [
        json!({"name":"cache","emptyDir":{}}),
        json!({"name":"host","hostPath":{"path":"/host"}}),
        json!({"name":"unknown","futureVolume":{}}),
    ] {
        let mut value = pod("local");
        value["spec"]["volumes"] = json!([volume]);
        assert_eq!(classify(value).await.reason, DrainPodReason::LocalData);
    }
    let mut value = pod("mirror");
    value["metadata"]["annotations"] = json!({"kubernetes.io/config.mirror":"hash"});
    assert_eq!(classify(value).await.reason, DrainPodReason::Mirror);
    let mut value = pod("ds");
    value["metadata"]["ownerReferences"][0]["kind"] = json!("DaemonSet");
    assert_eq!(
        classify(value.clone()).await.reason,
        DrainPodReason::DaemonSet
    );
    api.lock().unwrap().controller_missing = true;
    assert_eq!(
        classify(value).await.reason,
        DrainPodReason::ControllerUnavailable
    );
    api.lock().unwrap().controller_missing = false;
    let mut value = pod("terminating");
    value["metadata"]["deletionTimestamp"] = json!("2026-01-01T00:00:00Z");
    assert_eq!(classify(value).await.disposition, DrainDisposition::Wait);
    let mut value = pod("unsupported");
    value["metadata"]["ownerReferences"][0]["apiVersion"] = json!("apps/v2");
    assert_eq!(
        classify(value).await.reason,
        DrainPodReason::UnsupportedController
    );
}
#[tokio::test]
async fn review_is_read_only_and_partial_never_creates_a_plan() {
    for partial in [false, true] {
        let (_root, _catalog, engine, id, _) = fixture();
        let api = Arc::new(Mutex::new(Api {
            pods: vec![pod("one")],
            continue_token: partial,
            ..Api::default()
        }));
        let session = connected(&engine, id, fake(api.clone())).await;
        let result = engine
            .node_drain_review(request(session.cluster_id, session.session_id, 1))
            .await
            .unwrap();
        assert_eq!(result.partial, partial);
        assert_eq!(result.can_drain, !partial);
        assert_eq!(result.plan_id.is_some(), !partial);
        assert_eq!(result.expires_at.is_some(), !partial);
        assert!(api.lock().unwrap().writes.is_empty());
        assert!(api.lock().unwrap().permission_checks > 0);
        assert_eq!(
            serde_json::to_value(result)
                .unwrap()
                .as_object()
                .unwrap()
                .len(),
            10
        );
    }
}
#[tokio::test]
async fn sequential_evictions_wait_for_observed_404_and_cannot_replay() {
    let (_root, _catalog, engine, id, _) = fixture();
    let api = Arc::new(Mutex::new(Api {
        pods: vec![pod("one"), pod("two")],
        ..Api::default()
    }));
    let session = connected(&engine, id, fake(api.clone())).await;
    let review = engine
        .node_drain_review(request(
            session.cluster_id.clone(),
            session.session_id.clone(),
            1,
        ))
        .await
        .unwrap();
    let execute = DrainExecuteRequest {
        cluster_id: session.cluster_id,
        session_id: session.session_id,
        operation_id: 2,
        plan_id: review.plan_id.unwrap(),
    };
    let result = engine.node_drain_execute(execute.clone()).await.unwrap();
    assert_eq!(result.state, DrainState::Complete);
    assert!(result.cordoned);
    assert!(
        result
            .pods
            .iter()
            .all(|p| p.state == DrainOutcomeState::Evicted)
    );
    let output = serde_json::to_value(result).unwrap();
    assert_eq!(output.as_object().unwrap().len(), 7);
    assert!(!output.to_string().contains("PRIVATE"));
    assert_eq!(api.lock().unwrap().writes.len(), 3);
    assert_eq!(
        engine.node_drain_execute(execute).await.unwrap_err(),
        ConnectionError::StaleSession
    );
    assert_eq!(api.lock().unwrap().writes.len(), 3);
}
#[tokio::test]
async fn pdb_429_stops_without_retry_or_delete() {
    let (_root, _catalog, engine, id, _) = fixture();
    let api = Arc::new(Mutex::new(Api {
        pods: vec![pod("one"), pod("two")],
        eviction_status: 429,
        ..Api::default()
    }));
    let s = connected(&engine, id, fake(api.clone())).await;
    let review = engine
        .node_drain_review(request(s.cluster_id.clone(), s.session_id.clone(), 1))
        .await
        .unwrap();
    let result = engine
        .node_drain_execute(DrainExecuteRequest {
            cluster_id: s.cluster_id,
            session_id: s.session_id,
            operation_id: 2,
            plan_id: review.plan_id.unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(result.state, DrainState::Partial);
    assert_eq!(
        result.pods[0].reason,
        Some(DrainOutcomeReason::DisruptionBudget)
    );
    assert_eq!(result.pods[1].state, DrainOutcomeState::NotSubmitted);
    assert_eq!(api.lock().unwrap().writes.len(), 2);
}
#[tokio::test]
async fn stale_snapshot_and_replacement_prevent_cordon() {
    for change in 0..3 {
        let (_root, _catalog, engine, id, _) = fixture();
        let api = Arc::new(Mutex::new(Api {
            pods: vec![pod("one")],
            ..Api::default()
        }));
        let s = connected(&engine, id, fake(api.clone())).await;
        let review = engine
            .node_drain_review(request(s.cluster_id.clone(), s.session_id.clone(), 1))
            .await
            .unwrap();
        {
            let mut api = api.lock().unwrap();
            match change {
                0 => api.pods[0]["metadata"]["resourceVersion"] = json!("2"),
                1 => api.pods[0]["metadata"]["uid"] = json!("replacement"),
                _ => api.node_rv = "9".into(),
            }
        }
        let result = engine
            .node_drain_execute(DrainExecuteRequest {
                cluster_id: s.cluster_id,
                session_id: s.session_id,
                operation_id: 2,
                plan_id: review.plan_id.unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(result.state, DrainState::Blocked);
        assert!(api.lock().unwrap().writes.is_empty());
    }
}
#[tokio::test]
async fn new_pods_at_final_inventory_prevent_complete() {
    let (_root, _catalog, engine, id, _) = fixture();
    let api = Arc::new(Mutex::new(Api {
        pods: vec![pod("one")],
        inject_after_eviction: true,
        ..Api::default()
    }));
    let s = connected(&engine, id, fake(api.clone())).await;
    let review = engine
        .node_drain_review(request(s.cluster_id.clone(), s.session_id.clone(), 1))
        .await
        .unwrap();
    let result = engine
        .node_drain_execute(DrainExecuteRequest {
            cluster_id: s.cluster_id,
            session_id: s.session_id,
            operation_id: 2,
            plan_id: review.plan_id.unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(result.state, DrainState::Partial);
    assert!(
        result
            .pods
            .iter()
            .any(|p| p.reason == Some(DrainOutcomeReason::NewPods))
    );
}
#[tokio::test]
async fn expired_wrong_and_epoch_stale_plans_are_single_use() {
    for change in 0..3 {
        let (_root, _catalog, engine, id, _) = fixture();
        let api = Arc::new(Mutex::new(Api::default()));
        let s = connected(&engine, id, fake(api.clone())).await;
        let review = engine
            .node_drain_review(request(s.cluster_id.clone(), s.session_id.clone(), 1))
            .await
            .unwrap();
        let mut execute = DrainExecuteRequest {
            cluster_id: s.cluster_id,
            session_id: s.session_id,
            operation_id: 2,
            plan_id: review.plan_id.unwrap(),
        };
        match change {
            0 => {
                engine
                    .session
                    .lock()
                    .unwrap()
                    .as_mut()
                    .unwrap()
                    .node_drain
                    .plan
                    .as_mut()
                    .unwrap()
                    .deadline = Instant::now()
            }
            1 => execute.plan_id = "wrong".into(),
            _ => super::super::permissions::invalidate_authentication(
                &engine.authentication_generation,
            ),
        }
        assert_eq!(
            engine
                .node_drain_execute(execute.clone())
                .await
                .unwrap_err(),
            ConnectionError::StaleSession
        );
        execute.operation_id = 3;
        assert_eq!(
            engine.node_drain_execute(execute).await.unwrap_err(),
            ConnectionError::StaleSession
        );
        assert!(api.lock().unwrap().writes.is_empty());
    }
}
#[tokio::test]
async fn permission_denial_never_writes() {
    let (_root, _catalog, engine, id, _) = fixture();
    let api = Arc::new(Mutex::new(Api {
        deny: true,
        ..Api::default()
    }));
    let s = connected(&engine, id, fake(api.clone())).await;
    assert_eq!(
        engine
            .node_drain_review(request(s.cluster_id, s.session_id, 1))
            .await
            .unwrap_err(),
        ConnectionError::PermissionDenied
    );
    assert!(api.lock().unwrap().writes.is_empty());
}
#[tokio::test]
async fn eviction_permissions_only_cover_eligible_pods_and_are_rechecked() {
    let (_root, _catalog, engine, id, _) = fixture();
    let mut daemon = pod("daemon");
    daemon["metadata"]["namespace"] = json!("team-b");
    daemon["metadata"]["ownerReferences"][0]["kind"] = json!("DaemonSet");
    let mut mirror = pod("mirror");
    mirror["metadata"]["namespace"] = json!("team-b");
    mirror["metadata"]["annotations"] = json!({"kubernetes.io/config.mirror":"hash"});
    let mut terminating = pod("terminating");
    terminating["metadata"]["namespace"] = json!("team-b");
    terminating["metadata"]["deletionTimestamp"] = json!("2026-01-01T00:00:00Z");
    let api = Arc::new(Mutex::new(Api {
        pods: vec![pod("eligible"), daemon, mirror, terminating],
        denied_attributes: vec![json!({"subresource":"eviction","namespace":"team-b"})],
        ..Api::default()
    }));
    let s = connected(&engine, id, fake(api.clone())).await;
    let review = engine
        .node_drain_review(request(s.cluster_id.clone(), s.session_id.clone(), 1))
        .await
        .unwrap();
    assert!(review.can_drain);
    assert_eq!(review.pods[0].disposition, DrainDisposition::Evict);
    assert_eq!(review.pods[1].disposition, DrainDisposition::Skip);
    assert_eq!(review.pods[2].disposition, DrainDisposition::Skip);
    assert_eq!(review.pods[3].disposition, DrainDisposition::Wait);
    let review_attributes = api.lock().unwrap().reviewed_attributes.clone();
    for name in ["daemon", "mirror", "terminating"] {
        assert!(review_attributes.iter().any(|a| a["resource"] == "pods"
            && a["verb"] == "get"
            && a["namespace"] == "team-b"
            && a["name"] == name));
    }
    for resource in ["replicasets", "daemonsets"] {
        assert!(review_attributes.iter().any(|a| a["resource"] == resource
            && a["verb"] == "get"
            && a["namespace"] == "team-b"
            && a["name"] == "replicas"));
    }
    assert!(
        !review_attributes
            .iter()
            .any(|a| a["subresource"] == "eviction" && a["namespace"] == "team-b")
    );
    assert_eq!(
        review_attributes
            .iter()
            .filter(|a| a["subresource"] == "eviction")
            .count(),
        1
    );
    {
        let mut api = api.lock().unwrap();
        api.reviewed_attributes.clear();
        api.denied_attributes
            .push(json!({"subresource":"eviction","namespace":"team-a","name":"eligible"}));
    }
    let result = engine
        .node_drain_execute(DrainExecuteRequest {
            cluster_id: s.cluster_id,
            session_id: s.session_id,
            operation_id: 2,
            plan_id: review.plan_id.unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(result.state, DrainState::Blocked);
    assert_eq!(
        result.pods[0].reason,
        Some(DrainOutcomeReason::PermissionDenied)
    );
    let api = api.lock().unwrap();
    assert_eq!(api.reviewed_attributes, review_attributes[3..]);
    assert!(api.writes.is_empty());
}
#[tokio::test]
async fn eligible_eviction_denial_rejects_review_before_writes() {
    let (_root, _catalog, engine, id, _) = fixture();
    let api = Arc::new(Mutex::new(Api {
        pods: vec![pod("eligible")],
        denied_attributes: vec![json!({"subresource":"eviction","name":"eligible"})],
        ..Api::default()
    }));
    let s = connected(&engine, id, fake(api.clone())).await;
    assert_eq!(
        engine
            .node_drain_review(request(s.cluster_id, s.session_id, 1))
            .await
            .unwrap_err(),
        ConnectionError::PermissionDenied
    );
    let api = api.lock().unwrap();
    assert!(
        api.reads
            .iter()
            .any(|path| path.ends_with("/replicasets/replicas"))
    );
    assert!(api.writes.is_empty());
    assert!(
        engine
            .session
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .node_drain
            .plan
            .is_none()
    );
}
#[tokio::test]
async fn blocked_and_partial_reviews_keep_inventory_without_eviction_permissions() {
    for partial in [false, true] {
        let (_root, _catalog, engine, id, _) = fixture();
        let mut pods = vec![pod("eligible")];
        if !partial {
            let mut blocked = pod("unmanaged");
            blocked["metadata"]
                .as_object_mut()
                .unwrap()
                .remove("ownerReferences");
            pods.push(blocked);
        }
        let api = Arc::new(Mutex::new(Api {
            pods,
            continue_token: partial,
            denied_attributes: vec![json!({"subresource":"eviction"})],
            ..Api::default()
        }));
        let s = connected(&engine, id, fake(api.clone())).await;
        let review = engine
            .node_drain_review(request(s.cluster_id.clone(), s.session_id.clone(), 1))
            .await
            .unwrap();
        assert!(!review.can_drain);
        assert_eq!(review.partial, partial);
        assert!(review.plan_id.is_none());
        assert_eq!(review.pods.len(), if partial { 1 } else { 2 });
        engine
            .node_drain_cancel(DrainCancel {
                cluster_id: s.cluster_id,
                session_id: s.session_id,
                operation_id: 1,
            })
            .unwrap();
        assert!(
            !engine
                .session
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .node_drain
                .active()
        );
        let api = api.lock().unwrap();
        assert!(
            !api.reviewed_attributes
                .iter()
                .any(|a| a["subresource"] == "eviction")
        );
        assert!(api.writes.is_empty());
    }
}
#[tokio::test]
async fn pod_and_controller_read_denials_precede_classification() {
    for resource in ["pods", "replicasets"] {
        let (_root, _catalog, engine, id, _) = fixture();
        let api = Arc::new(Mutex::new(Api {
            pods: vec![pod("one")],
            denied_attributes: vec![json!({"resource":resource,"verb":"get","namespace":"team-a"})],
            ..Api::default()
        }));
        let s = connected(&engine, id, fake(api.clone())).await;
        assert_eq!(
            engine
                .node_drain_review(request(s.cluster_id, s.session_id, 1))
                .await
                .unwrap_err(),
            ConnectionError::PermissionDenied
        );
        let api = api.lock().unwrap();
        assert!(!api.reads.iter().any(|path| path.contains("/replicasets/")));
        assert!(
            !api.reviewed_attributes
                .iter()
                .any(|a| a["subresource"] == "eviction")
        );
        assert!(api.writes.is_empty());
    }
}
#[tokio::test]
async fn cancellation_tombstone_prevents_send() {
    let (_root, _catalog, engine, id, _) = fixture();
    let api = Arc::new(Mutex::new(Api::default()));
    let s = connected(&engine, id, fake(api.clone())).await;
    engine
        .node_drain_cancel(DrainCancel {
            cluster_id: s.cluster_id.clone(),
            session_id: s.session_id.clone(),
            operation_id: 1,
        })
        .unwrap();
    assert_eq!(
        engine
            .node_drain_review(request(s.cluster_id, s.session_id, 1))
            .await
            .unwrap_err(),
        ConnectionError::StaleSession
    );
    assert_eq!(api.lock().unwrap().permission_checks, 0);
    assert!(api.lock().unwrap().writes.is_empty());
}
#[tokio::test]
async fn cancellation_after_eviction_dispatch_is_uncertain_and_stops_queue() {
    let (_root, _catalog, engine, id, _) = fixture();
    let gate = Arc::new(tokio::sync::Barrier::new(2));
    let api = Arc::new(Mutex::new(Api {
        pods: vec![pod("one"), pod("two")],
        gate: Some(gate.clone()),
        ..Api::default()
    }));
    let s = connected(&engine, id, fake(api.clone())).await;
    let review = engine
        .node_drain_review(request(s.cluster_id.clone(), s.session_id.clone(), 1))
        .await
        .unwrap();
    let execute = engine.node_drain_execute(DrainExecuteRequest {
        cluster_id: s.cluster_id.clone(),
        session_id: s.session_id.clone(),
        operation_id: 2,
        plan_id: review.plan_id.unwrap(),
    });
    let cancel = async {
        gate.wait().await;
        assert!(
            engine
                .session
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .node_drain
                .active()
        );
        engine
            .node_drain_cancel(DrainCancel {
                cluster_id: s.cluster_id.clone(),
                session_id: s.session_id.clone(),
                operation_id: 2,
            })
            .unwrap();
        assert!(
            engine
                .session
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .node_drain
                .active()
        );
    };
    let (result, ()) = tokio::join!(execute, cancel);
    let result = result.unwrap();
    assert_eq!(result.state, DrainState::Uncertain);
    assert_eq!(result.pods[0].state, DrainOutcomeState::Uncertain);
    assert_eq!(result.pods[1].state, DrainOutcomeState::NotSubmitted);
    assert!(
        !engine
            .session
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .node_drain
            .active()
    );
    assert_eq!(api.lock().unwrap().writes.len(), 2);
}
#[test]
fn malformed_fields_and_output_bounds_fail_closed() {
    for field in ["uid", "resourceVersion", "namespace", "name"] {
        let mut value = pod("one");
        value["metadata"][field] = Value::Null;
        assert!(pod_identity(&value, "worker").is_err());
    }
    let mut value = pod("one");
    value["metadata"]["ownerReferences"][0]["controller"] = json!("true");
    assert!(controller(&value).is_err());
    let mut value = pod("one");
    value["spec"]["volumes"] = json!({});
    assert!(local_data(&value).is_err());
    assert_eq!(
        bounded(&"x".repeat(OUTPUT_LIMIT)),
        Err(ConnectionError::LimitExceeded)
    );
}

#[tokio::test]
async fn cancellation_during_execute_preflight_never_cordons() {
    let (_root, _catalog, engine, id, _) = fixture();
    let api = Arc::new(Mutex::new(Api {
        pods: vec![pod("one")],
        ..Api::default()
    }));
    let s = connected(&engine, id, fake(api.clone())).await;
    let review = engine
        .node_drain_review(request(s.cluster_id.clone(), s.session_id.clone(), 1))
        .await
        .unwrap();
    let gate = Arc::new(tokio::sync::Barrier::new(2));
    api.lock().unwrap().read_gate = Some(gate.clone());
    let execute = engine.node_drain_execute(DrainExecuteRequest {
        cluster_id: s.cluster_id.clone(),
        session_id: s.session_id.clone(),
        operation_id: 2,
        plan_id: review.plan_id.unwrap(),
    });
    let cancel = async {
        gate.wait().await;
        engine
            .node_drain_cancel(DrainCancel {
                cluster_id: s.cluster_id.clone(),
                session_id: s.session_id.clone(),
                operation_id: 2,
            })
            .unwrap();
    };
    let (result, ()) = tokio::join!(execute, cancel);
    assert_eq!(result.unwrap().state, DrainState::Cancelled);
    assert!(api.lock().unwrap().writes.is_empty());
}
#[tokio::test]
async fn terminating_managed_pod_is_waited_without_eviction() {
    let (_root, _catalog, engine, id, _) = fixture();
    let mut value = pod("terminating");
    value["metadata"]["deletionTimestamp"] = json!("2026-01-01T00:00:00Z");
    let api = Arc::new(Mutex::new(Api {
        pods: vec![value],
        delete_terminating_on_get: true,
        denied_attributes: vec![json!({"subresource":"eviction"})],
        ..Api::default()
    }));
    let s = connected(&engine, id, fake(api.clone())).await;
    let review = engine
        .node_drain_review(request(s.cluster_id.clone(), s.session_id.clone(), 1))
        .await
        .unwrap();
    assert_eq!(review.pods[0].disposition, DrainDisposition::Wait);
    let result = engine
        .node_drain_execute(DrainExecuteRequest {
            cluster_id: s.cluster_id,
            session_id: s.session_id,
            operation_id: 2,
            plan_id: review.plan_id.unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(result.state, DrainState::Complete);
    assert_eq!(result.pods[0].state, DrainOutcomeState::Gone);
    assert_eq!(api.lock().unwrap().writes.len(), 1);
    assert!(
        !api.lock()
            .unwrap()
            .reviewed_attributes
            .iter()
            .any(|a| a["subresource"] == "eviction")
    );
}
#[tokio::test]
async fn list_count_and_bytes_are_bounded() {
    for oversize in [false, true] {
        let mut pods = if oversize {
            vec![pod("one")]
        } else {
            (0..101).map(|i| pod(&format!("pod-{i}"))).collect()
        };
        if oversize {
            pods[0]["metadata"]["annotations"] = json!({"private":"x".repeat(INPUT_LIMIT)});
        }
        let api = Arc::new(Mutex::new(Api {
            pods,
            ..Api::default()
        }));
        assert_eq!(
            inventory(&fake(api.clone()), &CancellationToken::new(), "worker")
                .await
                .unwrap_err(),
            ConnectionError::LimitExceeded
        );
        assert!(api.lock().unwrap().writes.is_empty());
    }
}
#[tokio::test]
async fn old_review_disposal_cannot_cancel_new_execute_and_drop_releases_gate() {
    let (_root, _catalog, engine, id, _) = fixture();
    let api = Arc::new(Mutex::new(Api::default()));
    let s = connected(&engine, id, fake(api)).await;
    let identity = identity(&s.cluster_id, &s.session_id, 1).unwrap();
    let (_, guard) = engine.drain_start(identity.clone(), 1).unwrap();
    guard.cancel.cancel();
    assert!(
        engine
            .session
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .node_drain
            .active()
    );
    drop(guard);
    assert!(
        !engine
            .session
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .node_drain
            .active()
    );
    let (_, guard) = engine.drain_start(identity, 2).unwrap();
    engine
        .node_drain_cancel(DrainCancel {
            cluster_id: s.cluster_id,
            session_id: s.session_id,
            operation_id: 1,
        })
        .unwrap();
    assert!(!guard.cancel.is_cancelled());
    drop(guard);
    assert!(
        !engine
            .session
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .node_drain
            .active()
    );
}
