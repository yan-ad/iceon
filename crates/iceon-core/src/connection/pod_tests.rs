use super::*;
use crate::connection::tests::{connected_authorized as connected, fake, fixture, normal};
use std::sync::{Arc, Mutex};

fn pod() -> Value {
    json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":"web-0","namespace":"default","uid":"uid-1","resourceVersion":"42","annotations":{"credential":"PRIVATE_ANNOTATION"},"finalizers":["fixture/finalizer"]},
        "spec":{"nodeName":"node-a","serviceAccountName":"default","containers":[{"name":"web","image":"web:v1","ports":[{"containerPort":8080}],"env":[{"name":"TOKEN","value":"PRIVATE_ENV"}],"command":["PRIVATE_COMMAND"]}],"initContainers":[{"name":"setup","image":"setup:v1"}],"ephemeralContainers":[{"name":"debug","image":"debug:v1"}],"volumes":[{"name":"secret","secret":{"secretName":"PRIVATE_SECRET"}}]},
        "status":{"phase":"Running","podIPs":[{"ip":"10.0.0.2"}],"hostIP":"10.0.0.1","qosClass":"Burstable","containerStatuses":[{"name":"web","ready":true,"restartCount":2,"state":{"running":{"startedAt":"now"}}}],"conditions":[{"type":"Ready","status":"True","message":"PRIVATE_CONDITION"}]}})
}

#[tokio::test]
async fn pod_mutation_denial_prevents_resource_dispatch() {
    for action in [
        PodAction::Delete,
        PodAction::ForceDelete,
        PodAction::Finalize,
        PodAction::Replace,
    ] {
        let (_root, _, engine, id, _) = fixture();
        let reviews = Arc::new(Mutex::new(Vec::new()));
        let lease = crate::connection::tests::connected(
            &engine,
            id,
            crate::connection::tests::denied_permission_client(reviews.clone(), Some(false)),
        )
        .await;
        let mut r = mutation(action);
        if action == PodAction::Replace {
            r.yaml = Some(serde_yaml_ng::to_string(&yaml_view(&pod())).unwrap());
        }
        r.pod.cluster_id = lease.cluster_id;
        r.pod.session_id = lease.session_id;
        assert_eq!(
            engine.pod_mutate(r).await,
            Err(ConnectionError::PermissionDenied)
        );
        assert_eq!(reviews.lock().unwrap()[0]["name"], "web-0");
    }
}

#[test]
fn reduced_yaml_keeps_server_lifecycle_timestamps_read_only() {
    let mut live = pod();
    live["metadata"]["creationTimestamp"] = json!("2026-01-01T00:00:00Z");
    live["metadata"]["deletionTimestamp"] = json!("2026-01-02T00:00:00Z");
    for field in ["creationTimestamp", "deletionTimestamp"] {
        let mut edited = yaml_view(&live);
        edited["metadata"][field] = json!("2027-01-01T00:00:00Z");
        let mut request = mutation(PodAction::Replace);
        request.yaml = Some(serde_yaml_ng::to_string(&edited).unwrap());
        assert_eq!(
            replacement(&live, &request),
            Err(ConnectionError::InvalidInput)
        );
    }
}

#[test]
fn paginated_events_never_publish_as_complete_details() {
    assert!(matches!(
        project(
            &pod(),
            &json!({"items":[],"metadata":{"continue":"next"}}),
            request()
        ),
        Err(ConnectionError::LimitExceeded)
    ));
}
fn request() -> PodRequest {
    PodRequest {
        cluster_id: "cluster".into(),
        session_id: "lease".into(),
        namespace: "default".into(),
        name: "web-0".into(),
        uid: Some("uid-1".into()),
    }
}
fn mutation(action: PodAction) -> PodMutationRequest {
    PodMutationRequest {
        pod: request(),
        action,
        resource_version: "42".into(),
        yaml: None,
    }
}

#[test]
fn validates_names_and_closed_wire_requests() {
    assert!(validate_pod_request(&request()).is_ok());
    for invalid in [
        "../pods", "a/b", "a?x=y", "a%2fb", "UPPER", "", "-bad", "bad.",
    ] {
        let mut r = request();
        r.name = invalid.into();
        assert_eq!(
            validate_pod_request(&r),
            Err(ConnectionError::InvalidInput),
            "{invalid}"
        );
    }
    let value = json!({"clusterId":"cluster","sessionId":"lease","namespace":"default","name":"web-0","uid":"uid-1","action":"delete","resourceVersion":"42"});
    assert!(serde_json::from_value::<PodMutationRequest>(value.clone()).is_ok());
    let mut invalid = value;
    invalid["url"] = json!("https://arbitrary.invalid");
    assert!(serde_json::from_value::<PodMutationRequest>(invalid).is_err());
}

#[test]
fn detail_projects_all_container_kinds_and_omits_sensitive_payloads() {
    let events = json!({"items":[{"type":"Normal","reason":"Started","count":2,"lastTimestamp":"now","message":"PRIVATE_EVENT","involvedObject":{"uid":"uid-1"}}, {"type":"Warning","reason":"WrongPod","involvedObject":{"uid":"other"}}]});
    let details = project(&pod(), &events, request()).unwrap();
    assert_eq!(details.containers.len(), 3);
    assert_eq!(details.containers[0].kind, "container");
    assert_eq!(details.containers[1].kind, "init");
    assert_eq!(details.containers[2].kind, "ephemeral");
    assert_eq!(details.containers[0].ports[0].port, 8080);
    assert_eq!(details.containers[0].state, "running");
    assert!(details.containers[0].ready);
    assert_eq!(details.containers[0].restarts, 2);
    assert_eq!(details.conditions[0].status, "True");
    assert_eq!(details.events.len(), 1);
    assert!(!serde_json::to_string(&details).unwrap().contains("PRIVATE"));
}

#[test]
fn reduced_yaml_preserves_live_protected_fields_and_rejects_identity_or_field_injection() {
    let live = pod();
    let mut edited = yaml_view(&live);
    edited["spec"]["containers"][0]["image"] = json!("web:v2");
    let mut r = mutation(PodAction::Replace);
    r.yaml = Some(serde_yaml_ng::to_string(&edited).unwrap());
    let replaced = replacement(&live, &r).unwrap();
    assert_eq!(replaced["spec"]["containers"][0]["image"], "web:v2");
    assert_eq!(
        replaced["spec"]["containers"][0]["env"],
        live["spec"]["containers"][0]["env"]
    );
    assert_eq!(
        replaced["metadata"]["annotations"],
        live["metadata"]["annotations"]
    );
    assert_eq!(replaced["spec"]["volumes"], live["spec"]["volumes"]);
    for pointer in [
        "/apiVersion",
        "/kind",
        "/metadata/name",
        "/metadata/namespace",
        "/metadata/uid",
        "/metadata/resourceVersion",
        "/spec/containers/0/name",
    ] {
        let mut invalid = edited.clone();
        *invalid.pointer_mut(pointer).unwrap() = json!("changed");
        r.yaml = Some(serde_yaml_ng::to_string(&invalid).unwrap());
        assert_eq!(replacement(&live, &r), Err(ConnectionError::InvalidInput));
    }
    edited["spec"]["containers"][0]["env"] = json!([{"name":"TOKEN","value":"[redacted]"}]);
    r.yaml = Some(serde_yaml_ng::to_string(&edited).unwrap());
    assert_eq!(replacement(&live, &r), Err(ConnectionError::InvalidInput));
}

#[tokio::test]
async fn errors_and_body_limits_are_fixed_and_never_include_server_text() {
    for (status, expected) in [
        (401, ConnectionError::AuthFailed),
        (403, ConnectionError::PermissionDenied),
        (404, ConnectionError::NotFound),
        (409, ConnectionError::Conflict),
        (422, ConnectionError::InvalidInput),
        (429, ConnectionError::Busy),
        (504, ConnectionError::Timeout),
    ] {
        let client = fake(move |_| (status, b"PRIVATE_SERVER_RESPONSE".to_vec()));
        let result = send_json(
            &client,
            &CancellationToken::new(),
            Method::GET,
            &request().path(),
            None,
            "application/json",
        )
        .await;
        assert_eq!(result, Err(expected));
    }
    let client = fake(|_| (200, vec![b' '; BODY_LIMIT + 1]));
    assert_eq!(
        send_json(
            &client,
            &CancellationToken::new(),
            Method::GET,
            &request().path(),
            None,
            "application/json"
        )
        .await,
        Err(ConnectionError::LimitExceeded)
    );
}

#[tokio::test]
async fn mutations_use_preconditions_and_fixed_paths() {
    for action in [
        PodAction::Delete,
        PodAction::ForceDelete,
        PodAction::Finalize,
        PodAction::Replace,
    ] {
        let (_root, _catalog, engine, id, _) = fixture();
        let sent = Arc::new(Mutex::new(Vec::new()));
        let captured = sent.clone();
        let client = Client::new(
            super::super::transport::CancelService(tower::service_fn(
                move |request: http::Request<Body>| {
                    let captured = captured.clone();
                    async move {
                        let path = request.uri().to_string();
                        let method = request.method().clone();
                        let bytes = request.into_body().collect().await.unwrap().to_bytes();
                        captured.lock().unwrap().push((
                            method.clone(),
                            path.clone(),
                            bytes.to_vec(),
                        ));
                        let value = if path == "/version" {
                            json!({"gitVersion":"v1.36.0"})
                        } else {
                            pod()
                        };
                        Ok::<_, std::io::Error>(http::Response::new(Body::from(
                            serde_json::to_vec(&value).unwrap(),
                        )))
                    }
                },
            )),
            "default",
        );
        let session = connected(&engine, id, client).await;
        let mut r = mutation(action);
        r.pod.cluster_id = session.cluster_id;
        r.pod.session_id = session.session_id;
        if action == PodAction::Replace {
            r.yaml = Some(serde_yaml_ng::to_string(&yaml_view(&pod())).unwrap());
        }
        engine.pod_mutate(r).await.unwrap();
        let sent = sent.lock().unwrap();
        let (method, path, bytes) = sent.last().unwrap();
        assert_eq!(path, "/api/v1/namespaces/default/pods/web-0");
        let body: Value = serde_json::from_slice(bytes).unwrap();
        match action {
            PodAction::Delete | PodAction::ForceDelete => {
                assert_eq!(method, Method::DELETE);
                assert_eq!(
                    body["preconditions"],
                    json!({"uid":"uid-1","resourceVersion":"42"})
                );
                assert_eq!(
                    body.get("gracePeriodSeconds"),
                    (action == PodAction::ForceDelete).then_some(&json!(0))
                );
            }
            PodAction::Finalize => {
                assert_eq!(method, Method::PATCH);
                assert_eq!(body[0]["value"], "uid-1");
                assert_eq!(body[1]["value"], "42");
            }
            PodAction::Replace => {
                assert_eq!(method, Method::PUT);
                assert_eq!(
                    body["spec"]["containers"][0]["env"][0]["value"],
                    "PRIVATE_ENV"
                );
            }
        }
    }
}

#[tokio::test]
async fn stale_mutation_identity_never_writes_and_event_failure_is_not_hidden() {
    let (_root, _catalog, engine, id, _) = fixture();
    let client = fake(|path| {
        if path == "/version" {
            normal(path)
        } else if path.contains("/events?") {
            (403, b"PRIVATE".to_vec())
        } else {
            (200, serde_json::to_vec(&pod()).unwrap())
        }
    });
    let session = connected(&engine, id, client).await;
    let mut r = mutation(PodAction::Delete);
    r.pod.cluster_id = session.cluster_id.clone();
    r.pod.session_id = session.session_id.clone();
    r.resource_version = "old".into();
    assert_eq!(
        engine.pod_mutate(r.clone()).await,
        Err(ConnectionError::Conflict)
    );
    r.resource_version = "42".into();
    r.pod.uid = Some("old-uid".into());
    assert_eq!(engine.pod_mutate(r).await, Err(ConnectionError::Conflict));
    let mut r = request();
    r.cluster_id = session.cluster_id;
    r.session_id = session.session_id;
    let details = engine.pod_detail(r).await.unwrap();
    assert_eq!(details.events_state, PodEventsState::Denied);
    assert!(details.events.is_empty());
    assert_eq!(details.uid, "uid-1");
}

#[tokio::test]
async fn cancellation_drops_blocked_pod_request() {
    let (_root, _catalog, engine, id, _) = fixture();
    let client = Client::new(
        super::super::transport::CancelService(tower::service_fn(
            |request: http::Request<Body>| async move {
                if request.uri().path() != "/version" {
                    std::future::pending::<()>().await;
                }
                Ok::<_, std::io::Error>(http::Response::new(Body::from(
                    br#"{"gitVersion":"v1.36.0"}"#.to_vec(),
                )))
            },
        )),
        "default",
    );
    let session = connected(&engine, id, client).await;
    let mut r = request();
    r.cluster_id = session.cluster_id.clone();
    r.session_id = session.session_id.clone();
    let detail = engine.pod_detail(r);
    let cancel = async {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        engine
            .disconnect(SessionRequest {
                cluster_id: session.cluster_id,
                session_id: session.session_id,
            })
            .unwrap();
    };
    let (result, _) = tokio::join!(detail, cancel);
    assert!(matches!(result, Err(ConnectionError::StaleSession)));
}
