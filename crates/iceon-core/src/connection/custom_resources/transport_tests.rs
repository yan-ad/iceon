use super::*;
use crate::connection::tests::{connected, fixture};
use serde_json::json;

#[tokio::test]
async fn instance_body_limit_is_enforced_before_projection() {
    let (_root, _catalog, engine, id, _) = fixture();
    let snapshot = connected(
        &engine,
        id,
        fake(|path| {
            if path == "/version" {
                (200, br#"{"gitVersion":"v1"}"#.to_vec())
            } else if path.contains("customresourcedefinitions") {
                (200, serde_json::to_vec(&crd()).unwrap())
            } else {
                (200, vec![b' '; BODY_LIMIT + 1])
            }
        }),
    )
    .await;
    assert_eq!(
        engine.custom_resources(request(&snapshot)).await,
        Err(ConnectionError::LimitExceeded)
    );
}

#[tokio::test]
async fn exact_cancel_stops_in_flight_read_and_old_disposal_cannot_cancel_replacement() {
    use http::{Request, Response};
    use kube::client::Body;
    let (_root, _catalog, engine, id, _) = fixture();
    let reached = std::sync::Arc::new(tokio::sync::Notify::new());
    let observed = reached.clone();
    let client = kube::Client::new(
        crate::connection::transport::CancelService(tower::service_fn(move |r: Request<Body>| {
            let reached = reached.clone();
            async move {
                if r.uri().path().ends_with("/selfsubjectaccessreviews") {
                    return Ok::<_, std::io::Error>(Response::new(Body::from(
                        serde_json::to_vec(&allow_review(r).await).unwrap(),
                    )));
                }
                if r.uri().path() != "/version" {
                    reached.notify_one();
                    std::future::pending::<()>().await;
                }
                Ok::<_, std::io::Error>(Response::new(Body::from(
                    serde_json::to_vec(&json!({"gitVersion":"v1"})).unwrap(),
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
        let mut replacement = r.clone();
        replacement.read_id = 2;
        engine.cancel_custom_resources(replacement).unwrap();
        engine.cancel_custom_resources(r.clone()).unwrap();
        let state = engine.session.lock().unwrap();
        assert_eq!(
            state
                .as_ref()
                .unwrap()
                .custom_instances
                .as_ref()
                .unwrap()
                .id,
            2
        );
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        tokio::join!(read, cancel)
    })
    .await
    .unwrap();
    assert_eq!(result, Err(ConnectionError::StaleSession));
}

#[tokio::test]
async fn metadata_list_uses_partial_object_metadata_negotiation() {
    use http::{Request, Response};
    use kube::client::Body;
    let (_root, _catalog, engine, id, _) = fixture();
    let client = kube::Client::new(
        crate::connection::transport::CancelService(tower::service_fn(
            move |r: Request<Body>| async move {
                let body = if r.uri().path() == "/version" {
                    json!({"gitVersion":"v1.35.8"})
                } else if r.uri().path().ends_with("/selfsubjectaccessreviews") {
                    allow_review(r).await
                } else if r.uri().path().contains("customresourcedefinitions") {
                    crd()
                } else {
                    assert_eq!(
                        r.headers().get("accept").unwrap().to_str().unwrap(),
                        "application/json;as=PartialObjectMetadata;g=meta.k8s.io;v=v1"
                    );
                    json!({"apiVersion":"meta.k8s.io/v1","kind":"PartialObjectMetadataList","metadata":{},"items":[]})
                };
                Ok::<_, std::io::Error>(Response::new(Body::from(
                    serde_json::to_vec(&body).unwrap(),
                )))
            },
        )),
        "default",
    );
    let snapshot = connected(&engine, id, client).await;
    let page = engine.custom_resources(request(&snapshot)).await.unwrap();
    assert!(page.items.is_empty());
    assert!(!page.truncated);
}

#[tokio::test]
async fn exact_preflights_fail_closed_before_protected_reads() {
    use http::{Request, Response};
    use http_body_util::BodyExt;
    use kube::client::Body;
    use std::sync::{Arc, Mutex};
    for (stage, status, expected) in [
        (
            0,
            json!({"allowed":false}),
            ConnectionError::PermissionDenied,
        ),
        (
            1,
            json!({"allowed":false}),
            ConnectionError::PermissionDenied,
        ),
        (
            0,
            json!({"allowed":true,"evaluationError":"unavailable"}),
            ConnectionError::PermissionReviewUnavailable,
        ),
        (
            1,
            json!({"allowed":true,"evaluationError":"unavailable"}),
            ConnectionError::PermissionReviewUnavailable,
        ),
        (1, json!({}), ConnectionError::PermissionReviewUnavailable),
    ] {
        let (_root, _, engine, id, _) = fixture();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let recorded = seen.clone();
        let reads = Arc::new(Mutex::new(Vec::new()));
        let dispatched = reads.clone();
        let client = kube::Client::new(
            crate::connection::transport::CancelService(tower::service_fn(
                move |r: Request<Body>| {
                    let seen = recorded.clone();
                    let reads = dispatched.clone();
                    let status = status.clone();
                    async move {
                        let body = if r.uri().path() == "/version" {
                            json!({"gitVersion":"v1"})
                        } else if r.uri().path().ends_with("/selfsubjectaccessreviews") {
                            assert_eq!(r.method(), http::Method::POST);
                            let mut review: Value = serde_json::from_slice(
                                &r.into_body().collect().await.unwrap().to_bytes(),
                            )
                            .unwrap();
                            let index = {
                                let mut seen = seen.lock().unwrap();
                                let index = seen.len();
                                seen.push(review["spec"]["resourceAttributes"].clone());
                                index
                            };
                            review["status"] = if index == stage {
                                status
                            } else {
                                json!({"allowed":true})
                            };
                            review
                        } else {
                            reads.lock().unwrap().push(r.uri().path().to_owned());
                            assert!(
                                r.uri().path().contains("customresourcedefinitions"),
                                "denied instance request dispatched"
                            );
                            crd()
                        };
                        Ok::<_, std::io::Error>(Response::new(Body::from(
                            serde_json::to_vec(&body).unwrap(),
                        )))
                    }
                },
            )),
            "default",
        );
        let snapshot = connected(&engine, id, client).await;
        assert_eq!(
            engine.custom_resources(request(&snapshot)).await,
            Err(expected)
        );
        let mut expected_attributes = vec![
            json!({"group":"apiextensions.k8s.io", "version":"v1", "resource":"customresourcedefinitions", "verb":"get", "name":"widgets.example.io"}),
        ];
        if stage == 1 {
            expected_attributes.push(json!({"group":"example.io", "version":"v1", "resource":"widgets", "verb":"list", "namespace":"default"}));
        }
        assert_eq!(*seen.lock().unwrap(), expected_attributes);
        assert_eq!(reads.lock().unwrap().len(), stage);
    }
}

#[tokio::test]
async fn discovery_denial_and_unavailable_review_never_dispatch_inventory() {
    use http::{Request, Response};
    use kube::client::Body;
    for (code, expected) in [
        (200, ConnectionError::PermissionDenied),
        (403, ConnectionError::PermissionReviewUnavailable),
    ] {
        let (_root, _, engine, id, _) = fixture();
        let client = kube::Client::new(
            crate::connection::transport::CancelService(tower::service_fn(
                move |r: Request<Body>| async move {
                    let (code, body) = if r.uri().path() == "/version" {
                        (200, json!({"gitVersion":"v1"}))
                    } else {
                        assert!(r.uri().path().ends_with("/selfsubjectaccessreviews"));
                        let mut review = allow_review(r).await;
                        assert_eq!(
                            review["spec"]["resourceAttributes"],
                            json!({"group":"apiextensions.k8s.io", "version":"v1", "resource":"customresourcedefinitions", "verb":"list"})
                        );
                        review["status"] = json!({"allowed":false});
                        (code, review)
                    };
                    Ok::<_, std::io::Error>(
                        Response::builder()
                            .status(code)
                            .body(Body::from(serde_json::to_vec(&body).unwrap()))
                            .unwrap(),
                    )
                },
            )),
            "default",
        );
        let snapshot = connected(&engine, id, client).await;
        assert_eq!(
            engine
                .custom_resource_definitions(discovery(&snapshot))
                .await,
            Err(expected)
        );
    }
}

#[tokio::test]
async fn dynamic_list_review_uses_served_version_plural_and_actual_scope() {
    use http::{Request, Response};
    use kube::client::Body;
    for scope in ["Cluster", "Namespaced"] {
        let (_root, _, engine, id, _) = fixture();
        let client = kube::Client::new(
            crate::connection::transport::CancelService(tower::service_fn(
                move |r: Request<Body>| async move {
                    let body = if r.uri().path() == "/version" {
                        json!({"gitVersion":"v1"})
                    } else if r.uri().path().ends_with("/selfsubjectaccessreviews") {
                        let review = allow_review(r).await;
                        if review["spec"]["resourceAttributes"]["verb"] == "list" {
                            assert_eq!(
                                review["spec"]["resourceAttributes"],
                                json!({"group":"example.io", "version":"v1beta1", "resource":"widgets", "verb":"list"})
                            );
                        }
                        review
                    } else if r.uri().path().contains("customresourcedefinitions") {
                        let mut value = crd();
                        value["spec"]["scope"] = json!(scope);
                        value["spec"]["versions"][1]["served"] = json!(false);
                        value
                    } else {
                        assert_eq!(
                            r.uri().to_string(),
                            "/apis/example.io/v1beta1/widgets?limit=100&continue=opaque%2B%2F%3D"
                        );
                        assert_eq!(r.headers()["accept"], METADATA_ACCEPT);
                        json!({"apiVersion":"meta.k8s.io/v1", "kind":"PartialObjectMetadataList", "items":[]})
                    };
                    Ok::<_, std::io::Error>(Response::new(Body::from(
                        serde_json::to_vec(&body).unwrap(),
                    )))
                },
            )),
            "default",
        );
        let snapshot = connected(&engine, id, client).await;
        let mut r = request(&snapshot);
        r.namespace = None;
        r.continue_token = Some("opaque+/=".into());
        assert!(engine.custom_resources(r).await.is_ok());
    }
}

#[tokio::test]
async fn epoch_change_after_review_or_response_rejects_dispatch_and_publication() {
    use http::{Request, Response};
    use kube::client::Body;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    for invalidate_at in [1, 2, 3] {
        let (_root, _, engine, id, _) = fixture();
        let generation = engine.authentication_generation.clone();
        let stage = Arc::new(AtomicUsize::new(0));
        let observed = stage.clone();
        let client = kube::Client::new(
            crate::connection::transport::CancelService(tower::service_fn(
                move |r: Request<Body>| {
                    let generation = generation.clone();
                    let stage = stage.clone();
                    async move {
                        let body = if r.uri().path() == "/version" {
                            json!({"gitVersion":"v1"})
                        } else if r.uri().path().ends_with("/selfsubjectaccessreviews") {
                            let review = allow_review(r).await;
                            let index = stage.fetch_add(1, Ordering::SeqCst) + 1;
                            if index == invalidate_at {
                                super::super::super::permissions::invalidate_authentication(
                                    &generation,
                                );
                            }
                            review
                        } else if r.uri().path().contains("customresourcedefinitions") {
                            assert_ne!(invalidate_at, 1, "stale CRD dispatch");
                            crd()
                        } else {
                            assert_eq!(invalidate_at, 3, "stale instance dispatch");
                            stage.fetch_add(1, Ordering::SeqCst);
                            super::super::super::permissions::invalidate_authentication(
                                &generation,
                            );
                            json!({"apiVersion":"meta.k8s.io/v1", "kind":"PartialObjectMetadataList", "items":[]})
                        };
                        Ok::<_, std::io::Error>(Response::new(Body::from(
                            serde_json::to_vec(&body).unwrap(),
                        )))
                    }
                },
            )),
            "default",
        );
        let snapshot = connected(&engine, id, client).await;
        assert_eq!(
            engine.custom_resources(request(&snapshot)).await,
            Err(ConnectionError::StaleSession)
        );
        assert_eq!(observed.load(Ordering::SeqCst), invalidate_at);
    }
}
