use super::*;
use crate::connection::tests::{connected, fake, fixture};
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
