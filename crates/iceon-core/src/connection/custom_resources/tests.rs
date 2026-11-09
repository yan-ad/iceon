use super::*;
use crate::connection::tests::{connected, fake, fixture};
use serde_json::json;

#[path = "transport_tests.rs"]
mod transport_tests;

fn target() -> CustomResourceTarget {
    CustomResourceTarget {
        definition_name: "widgets.example.io".into(),
        definition_uid: "definition-uid".into(),
    }
}

fn discovery(snapshot: &crate::connection::ConnectionSnapshot) -> CustomDefinitionRequest {
    CustomDefinitionRequest {
        cluster_id: snapshot.cluster_id.clone(),
        session_id: snapshot.session_id.clone(),
        read_id: 1,
        continue_token: None,
    }
}

#[tokio::test]
async fn discovery_normal_crd_and_instances_have_independent_cancellation_slots() {
    use http::{Request, Response};
    use kube::client::Body;
    let (_root, _catalog, engine, id, _) = fixture();
    let reached = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
    let observed = reached.clone();
    let client = kube::Client::new(
        crate::connection::transport::CancelService(tower::service_fn(move |r: Request<Body>| {
            let reached = reached.clone();
            async move {
                if r.uri().path() != "/version" {
                    reached.add_permits(1);
                    std::future::pending::<()>().await;
                }
                Ok::<_, std::io::Error>(Response::new(Body::from(
                    br#"{"gitVersion":"v1"}"#.to_vec(),
                )))
            }
        })),
        "default",
    );
    let snapshot = connected(&engine, id, client).await;
    let d = discovery(&snapshot);
    let normal = d.slot_request().unwrap();
    let instances = request(&snapshot);
    let cancel = async {
        let _arrived = observed.acquire_many(3).await.unwrap();
        let (normal_token, discovery_token, instance_token) = {
            let state = engine.session.lock().unwrap();
            let s = state.as_ref().unwrap();
            (
                s.reads[&ResourceKind::CustomResourceDefinitions]
                    .cancel
                    .clone()
                    .unwrap(),
                s.custom_discovery.as_ref().unwrap().cancel.clone().unwrap(),
                s.custom_instances.as_ref().unwrap().cancel.clone().unwrap(),
            )
        };
        assert!(!normal_token.is_cancelled());
        assert!(!discovery_token.is_cancelled());
        assert!(!instance_token.is_cancelled());
        let mut newer = d.clone();
        newer.read_id = 2;
        engine
            .cancel_custom_resource_definitions(newer.clone())
            .unwrap();
        engine
            .cancel_custom_resource_definitions(d.clone())
            .unwrap();
        assert!(discovery_token.is_cancelled());
        assert!(!normal_token.is_cancelled());
        assert!(!instance_token.is_cancelled());
        let mut forged = newer;
        forged.continue_token = Some("other-page".into());
        assert_eq!(
            engine.cancel_custom_resource_definitions(forged),
            Err(ConnectionError::InvalidInput)
        );
        engine.cancel_custom_resources(instances.clone()).unwrap();
        assert!(instance_token.is_cancelled());
        assert!(!normal_token.is_cancelled());
        engine.cancel_resources(normal.clone()).unwrap();
    };
    let (normal_result, discovery_result, instance_result, ()) =
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            tokio::join!(
                engine.resources(normal.clone()),
                engine.custom_resource_definitions(d.clone()),
                engine.custom_resources(instances.clone()),
                cancel
            )
        })
        .await
        .unwrap();
    assert_eq!(normal_result, Err(ConnectionError::StaleSession));
    assert_eq!(discovery_result, Err(ConnectionError::StaleSession));
    assert_eq!(instance_result, Err(ConnectionError::StaleSession));
}

#[tokio::test]
async fn discovery_validates_request_and_retains_resource_projection_and_read_order() {
    let (_root, _catalog, engine, id, _) = fixture();
    let snapshot = connected(&engine, id, fake(|path| {
        let body = if path == "/version" {
            json!({"gitVersion":"v1"})
        } else {
            assert_eq!(path, "/apis/apiextensions.k8s.io/v1/customresourcedefinitions?limit=10&timeoutSeconds=10");
            json!({"metadata":{},"items":[]})
        };
        (200, serde_json::to_vec(&body).unwrap())
    })).await;
    let mut d = discovery(&snapshot);
    let page = engine.custom_resource_definitions(d.clone()).await.unwrap();
    assert_eq!(page.kind, ResourceKind::CustomResourceDefinitions);
    assert!(page.items.is_empty());
    assert_eq!(
        engine.custom_resource_definitions(d.clone()).await,
        Err(ConnectionError::StaleSession)
    );
    d.read_id = 2;
    d.continue_token = Some("bad\ntoken".into());
    assert_eq!(
        engine.custom_resource_definitions(d.clone()).await,
        Err(ConnectionError::InvalidInput)
    );
    d.continue_token = None;
    assert!(engine.custom_resource_definitions(d).await.is_ok());
    assert!(serde_json::from_value::<CustomDefinitionRequest>(json!({
        "clusterId":snapshot.cluster_id,"sessionId":snapshot.session_id,"readId":3,"query":"ignored?"
    })).is_err());
}

#[test]
fn own_slot_supersession_cancels_only_its_predecessor() {
    let parent = CancellationToken::new();
    let mut first_slot = None;
    let mut other_slot = None;
    let request = ResourceRequest {
        cluster_id: "cluster".into(),
        session_id: "session".into(),
        kind: ResourceKind::CustomResourceDefinitions,
        read_id: 1,
        namespace: None,
        continue_token: None,
        query: None,
        status: None,
        group: None,
    };
    let first = start_slot(&mut first_slot, request.clone(), &parent).unwrap();
    let other = start_slot(&mut other_slot, request.clone(), &parent).unwrap();
    let mut replacement = request;
    replacement.read_id = 2;
    let newer = start_slot(&mut first_slot, replacement, &parent).unwrap();
    assert!(first.is_cancelled());
    assert!(!other.is_cancelled());
    assert!(!newer.is_cancelled());
    parent.cancel();
    assert!(other.is_cancelled());
    assert!(newer.is_cancelled());
}

#[tokio::test]
async fn dedicated_reads_share_worker_budget_without_consuming_ids_when_busy() {
    let (_root, _catalog, engine, id, _) = fixture();
    let snapshot = connected(
        &engine,
        id,
        fake(|_| (200, br#"{"gitVersion":"v1"}"#.to_vec())),
    )
    .await;
    let permits = engine
        .workers
        .acquire_many(engine.workers.available_permits() as u32)
        .await
        .unwrap();
    assert_eq!(
        engine
            .custom_resource_definitions(discovery(&snapshot))
            .await,
        Err(ConnectionError::Busy)
    );
    assert_eq!(
        engine.custom_resources(request(&snapshot)).await,
        Err(ConnectionError::Busy)
    );
    let state = engine.session.lock().unwrap();
    let session = state.as_ref().unwrap();
    assert!(session.custom_discovery.is_none());
    assert!(session.custom_instances.is_none());
    drop(state);
    drop(permits);
}

#[tokio::test]
async fn dedicated_discovery_deadline_cancels_transport_and_releases_worker() {
    use http::{Request, Response};
    use kube::client::Body;
    let (_root, _catalog, engine, id, _) = fixture();
    let reached = std::sync::Arc::new(tokio::sync::Notify::new());
    let observed = reached.clone();
    let client = kube::Client::new(
        crate::connection::transport::CancelService(tower::service_fn(move |r: Request<Body>| {
            let reached = reached.clone();
            async move {
                if r.uri().path() != "/version" {
                    reached.notify_one();
                    std::future::pending::<()>().await;
                }
                Ok::<_, std::io::Error>(Response::new(Body::from(
                    br#"{"gitVersion":"v1"}"#.to_vec(),
                )))
            }
        })),
        "default",
    );
    let snapshot = connected(&engine, id, client).await;
    let available = engine.workers.available_permits();
    tokio::time::pause();
    let mut read = Box::pin(engine.custom_resource_definitions(discovery(&snapshot)));
    tokio::select! { biased; result = &mut read => panic!("unexpected completion: {result:?}"), _ = observed.notified() => {} }
    tokio::time::advance(super::super::DEADLINE + std::time::Duration::from_secs(1)).await;
    assert_eq!(read.await, Err(ConnectionError::Timeout));
    assert_eq!(engine.workers.available_permits(), available);
    assert!(
        engine
            .session
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .custom_discovery
            .as_ref()
            .unwrap()
            .cancel
            .is_none()
    );
    tokio::time::resume();
}
fn crd() -> Value {
    json!({"apiVersion":"apiextensions.k8s.io/v1","kind":"CustomResourceDefinition","metadata":{"name":"widgets.example.io","uid":"definition-uid"},"spec":{"group":"example.io","scope":"Namespaced","names":{"plural":"widgets","kind":"Widget"},"versions":[{"name":"v1beta1","served":true,"storage":false},{"name":"v1","served":true,"storage":true}]},"status":{"conditions":[{"type":"Established","status":"True"}]}})
}
fn request(snapshot: &crate::connection::ConnectionSnapshot) -> CustomResourceRequest {
    CustomResourceRequest {
        cluster_id: snapshot.cluster_id.clone(),
        session_id: snapshot.session_id.clone(),
        read_id: 1,
        target: target(),
        namespace: Some("default".into()),
        continue_token: None,
    }
}
fn metadata() -> Value {
    json!({"apiVersion":"meta.k8s.io/v1","kind":"PartialObjectMetadataList","metadata":{"continue":"next+/="},"items":[{"metadata":{"name":"one","namespace":"default","uid":"instance-uid","labels":{"PRIVATE":"PRIVATE"},"annotations":{"PRIVATE":"PRIVATE"}},"spec":{"password":"PRIVATE"},"status":{"token":"PRIVATE"}}]})
}
#[test]
fn definition_validation_and_served_version_selection() {
    let d = definition(&target(), crd()).unwrap();
    assert_eq!(d.version, "v1");
    let mut value = crd();
    value["spec"]["versions"][1]["served"] = json!(false);
    assert_eq!(definition(&target(), value).unwrap().version, "v1beta1");
    for (pointer, forged) in [
        ("/spec/group", "../evil"),
        ("/spec/names/plural", "widgets/secrets"),
        ("/spec/versions/1/name", "v1?watch=true"),
        ("/spec/scope", "Unknown"),
    ] {
        let mut value = crd();
        *value.pointer_mut(pointer).unwrap() = json!(forged);
        assert!(matches!(
            definition(&target(), value),
            Err(ConnectionError::InvalidResponse)
        ));
    }
    let mut value = crd();
    value["metadata"]["uid"] = json!("replacement");
    assert!(matches!(
        definition(&target(), value),
        Err(ConnectionError::SourceChanged)
    ));
    let mut value = crd();
    value["status"]["conditions"] = json!([]);
    assert!(matches!(
        definition(&target(), value),
        Err(ConnectionError::ResourceUnavailable)
    ));
}
#[tokio::test]
async fn real_client_requests_are_bound_to_definition_and_projection_is_metadata_only() {
    let (_root, _catalog, engine, id, _) = fixture();
    let client = fake(|path| {
        let body = match path {
            "/version" => json!({"gitVersion":"v1.36.0"}),
            "/apis/apiextensions.k8s.io/v1/customresourcedefinitions/widgets.example.io" => crd(),
            "/apis/example.io/v1/namespaces/default/widgets?limit=100" => metadata(),
            other => panic!("unexpected endpoint: {other}"),
        };
        (200, serde_json::to_vec(&body).unwrap())
    });
    let snapshot = connected(&engine, id, client).await;
    let r = request(&snapshot);
    let page = engine.custom_resources(r.clone()).await.unwrap();
    assert_eq!(page.items[0].label_count, 1);
    assert_eq!(page.continue_token.as_deref(), Some("next+/="));
    assert!(page.truncated);
    assert!(!serde_json::to_string(&page).unwrap().contains("PRIVATE"));
    assert_eq!(
        engine.custom_resources(r.clone()).await,
        Err(ConnectionError::StaleSession)
    );
    let mut newer = r.clone();
    newer.read_id = 2;
    engine.cancel_custom_resources(newer.clone()).unwrap();
    assert_eq!(
        engine.custom_resources(newer).await,
        Err(ConnectionError::StaleSession)
    );
    let mut forged = r;
    forged.read_id = 2;
    forged.target.definition_uid = "different".into();
    assert_eq!(
        engine.cancel_custom_resources(forged),
        Err(ConnectionError::InvalidInput)
    );
}
#[tokio::test]
async fn invalid_input_and_uid_replacement_never_reach_instance_endpoint() {
    let (_root, _catalog, engine, id, _) = fixture();
    let snapshot = connected(
        &engine,
        id,
        fake(|path| {
            let body = if path == "/version" {
                json!({"gitVersion":"v1"})
            } else {
                assert_eq!(
                    path,
                    "/apis/apiextensions.k8s.io/v1/customresourcedefinitions/widgets.example.io"
                );
                let mut value = crd();
                value["metadata"]["uid"] = json!("replaced");
                value
            };
            (200, serde_json::to_vec(&body).unwrap())
        }),
    )
    .await;
    for bad in [
        "../secrets",
        "widgets.example.io?watch=true",
        "widgets/example.io",
        "widgets..example.io",
    ] {
        let mut r = request(&snapshot);
        r.target.definition_name = bad.into();
        assert_eq!(
            engine.custom_resources(r).await,
            Err(ConnectionError::InvalidInput)
        );
    }
    let mut r = request(&snapshot);
    r.namespace = Some("../default".into());
    assert_eq!(
        engine.custom_resources(r).await,
        Err(ConnectionError::InvalidInput)
    );
    assert_eq!(
        engine.custom_resources(request(&snapshot)).await,
        Err(ConnectionError::SourceChanged)
    );
}
#[tokio::test]
async fn scope_pagination_and_projection_validation() {
    let (_root, _catalog, engine, id, _) = fixture();
    let snapshot = engine
        .open(crate::connection::OpenRequest { cluster_id: id })
        .await
        .unwrap();
    let mut r = request(&snapshot);
    r.continue_token = Some("opaque+/=&x".into());
    let d = definition(&target(), crd()).unwrap();
    assert_eq!(
        instance_path(&d, &r).unwrap(),
        "/apis/example.io/v1/namespaces/default/widgets?limit=100&continue=opaque%2B%2F%3D%26x"
    );
    let mut value = crd();
    value["spec"]["scope"] = json!("Cluster");
    assert_eq!(
        instance_path(&definition(&target(), value).unwrap(), &r),
        Err(ConnectionError::InvalidInput)
    );
    let mut value = metadata();
    value["items"][0]["metadata"]["namespace"] = json!("other");
    assert_eq!(project(&r, d, value), Err(ConnectionError::InvalidResponse));
    let mut value = metadata();
    value["items"] = json!(vec![value["items"][0].clone(); 101]);
    assert_eq!(
        project(&r, definition(&target(), crd()).unwrap(), value),
        Err(ConnectionError::LimitExceeded)
    );
    r.continue_token = Some("next+/=".into());
    assert_eq!(
        project(&r, definition(&target(), crd()).unwrap(), metadata()),
        Err(ConnectionError::InvalidResponse)
    );
}
#[tokio::test]
async fn server_errors_remain_typed_and_private_bodies_are_discarded() {
    for (status, expected) in [
        (401, ConnectionError::AuthFailed),
        (403, ConnectionError::PermissionDenied),
        (404, ConnectionError::ResourceUnavailable),
        (504, ConnectionError::Timeout),
    ] {
        let (_root, _catalog, engine, id, _) = fixture();
        let snapshot = connected(
            &engine,
            id,
            fake(move |path| {
                if path == "/version" {
                    (200, br#"{"gitVersion":"v1"}"#.to_vec())
                } else {
                    (status, b"PRIVATE ERROR".to_vec())
                }
            }),
        )
        .await;
        assert_eq!(
            engine.custom_resources(request(&snapshot)).await,
            Err(expected)
        );
    }
}
#[tokio::test]
async fn disconnect_cancels_pending_transport_and_rejects_late_result() {
    use http::{Request, Response};
    use kube::client::Body;
    let (_root, _catalog, engine, id, _) = fixture();
    let reached = std::sync::Arc::new(tokio::sync::Notify::new());
    let observed = reached.clone();
    let client = kube::Client::new(
        crate::connection::transport::CancelService(tower::service_fn(move |r: Request<Body>| {
            let reached = reached.clone();
            async move {
                if r.uri().path() != "/version" {
                    reached.notify_one();
                    std::future::pending::<()>().await;
                }
                Ok::<_, std::io::Error>(Response::new(Body::from(
                    br#"{"gitVersion":"v1"}"#.to_vec(),
                )))
            }
        })),
        "default",
    );
    let snapshot = connected(&engine, id, client).await;
    let r = request(&snapshot);
    let read = engine.custom_resources(r.clone());
    let cancel = async {
        observed.notified().await;
        engine.disconnect(r.session()).unwrap();
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        tokio::join!(read, cancel)
    })
    .await
    .unwrap();
    assert_eq!(result, Err(ConnectionError::StaleSession));
}
