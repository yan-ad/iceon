use super::*;
use std::sync::Mutex;

fn definition(name: &str, schema_bytes: usize) -> serde_json::Value {
    json!({"metadata":{"name":name,"uid":"uid","resourceVersion":"7"},"spec":{
        "group":"example.io","scope":"Namespaced","names":{"kind":"Widget","plural":"widgets","shortNames":["wdg"]},
        "versions":[{"name":"v1","schema":{"openAPIV3Schema":{"description":"X".repeat(schema_bytes)}}}]
    }})
}

#[tokio::test]
async fn crd_large_schema_retries_same_opaque_token_at_one_and_projects_only_facts() {
    let paths = Arc::new(Mutex::new(Vec::new()));
    let observed = paths.clone();
    let client = fake(move |path| {
        observed.lock().unwrap().push(path.to_string());
        assert!(path.contains("continue=a%2B%2F%3D"));
        let count = if path.contains("limit=10&") {
            10
        } else {
            assert!(path.contains("limit=1&"));
            1
        };
        let items = (0..count)
            .map(|i| definition(&format!("widget-{i}.example.io"), 5 * 1024 * 1024))
            .collect::<Vec<_>>();
        (
            200,
            serde_json::to_vec(&json!({"metadata":{"continue":"next"},"items":items})).unwrap(),
        )
    });
    let mut r = request(ResourceKind::CustomResourceDefinitions);
    r.continue_token = Some("a+/=".into());
    let page = transport::resources(&client, &r, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(paths.lock().unwrap().len(), 2);
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.continue_token.as_deref(), Some("next"));
    assert!(page.truncated);
    let columns = &page.items[0].columns;
    for (key, value) in [
        ("kind", "Widget"),
        ("plural", "widgets"),
        ("group", "example.io"),
        ("versions", "v1"),
        ("shortNames", "wdg"),
        ("scope", "Namespaced"),
    ] {
        assert_eq!(columns[key], value);
    }
    let wire = serde_json::to_string(&page).unwrap();
    assert!(wire.len() < 64 * 1024);
    assert!(!wire.contains("schema"));
    assert!(!wire.contains("XXXXXXXX"));
}

#[tokio::test]
async fn crd_filtered_scan_retries_without_skipping_and_resumes_at_unscanned_token() {
    let paths = Arc::new(Mutex::new(Vec::new()));
    let observed = paths.clone();
    let client = fake(move |path| {
        observed.lock().unwrap().push(path.to_owned());
        let index = path
            .split("continue=")
            .nth(1)
            .unwrap_or("0")
            .parse::<usize>()
            .unwrap();
        if path.contains("limit=10&") {
            return (200, vec![b' '; 2 * 1024 * 1024 + 1]);
        }
        assert!(path.contains("limit=1&"));
        (200, serde_json::to_vec(&json!({"metadata":{"continue":(index+1).to_string()},"items":[definition(&format!("widget-{index}.example.io"), 0)]})).unwrap())
    });
    let mut r = request(ResourceKind::CustomResourceDefinitions);
    r.group = Some("example.io".into());
    r.query = Some("widget-10.".into());
    let page = transport::resources(&client, &r, &CancellationToken::new())
        .await
        .unwrap();
    assert!(page.items.is_empty());
    assert_eq!(page.continue_token.as_deref(), Some("10"));
    assert_eq!(paths.lock().unwrap().len(), 11);
    r.continue_token = page.continue_token;
    let page = transport::resources(&client, &r, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].name, "widget-10.example.io");
    assert_eq!(page.continue_token.as_deref(), Some("20"));
}

#[tokio::test]
async fn crd_single_object_cap_and_ordinary_inventory_budget_remain_enforced() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = calls.clone();
    let client = fake(move |_| {
        observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        (200, vec![b' '; 16 * 1024 * 1024 + 1])
    });
    assert_eq!(
        transport::resources(
            &client,
            &request(ResourceKind::CustomResourceDefinitions),
            &CancellationToken::new()
        )
        .await,
        Err(ConnectionError::LimitExceeded)
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    let client = fake(|path| {
        assert!(path.contains("limit=100&"));
        (200, vec![b' '; 2 * 1024 * 1024 + 1])
    });
    assert_eq!(
        transport::resources(
            &client,
            &request(ResourceKind::Namespaces),
            &CancellationToken::new()
        )
        .await,
        Err(ConnectionError::LimitExceeded)
    );
}

#[tokio::test]
async fn crd_retry_failure_is_not_an_empty_or_partial_success() {
    for (code, expected) in [
        (403, ConnectionError::PermissionDenied),
        (404, ConnectionError::ResourceUnavailable),
        (500, ConnectionError::ConnectionFailed),
    ] {
        let client = fake(move |path| {
            if path.contains("limit=10&") {
                (200, vec![b' '; 2 * 1024 * 1024 + 1])
            } else {
                (code, b"PRIVATE".to_vec())
            }
        });
        assert_eq!(
            transport::resources(
                &client,
                &request(ResourceKind::CustomResourceDefinitions),
                &CancellationToken::new()
            )
            .await,
            Err(expected)
        );
    }
}

#[tokio::test]
async fn crd_cancel_during_retry_releases_engine_read_slot() {
    let started = Arc::new(tokio::sync::Notify::new());
    let marker = started.clone();
    let client = Client::new(
        transport::CancelService(tower::service_fn(
            move |r: http::Request<kube::client::Body>| {
                let marker = marker.clone();
                async move {
                    let path = r.uri().to_string();
                    if path.contains("limit=1&") {
                        marker.notify_one();
                        return std::future::pending().await;
                    }
                    let bytes = if path == "/version" {
                        normal("/version").1
                    } else {
                        vec![b' '; 2 * 1024 * 1024 + 1]
                    };
                    Ok::<_, std::io::Error>(http::Response::new(kube::client::Body::from(bytes)))
                }
            },
        )),
        "default",
    );
    let (_root, _catalog, engine, id, _) = fixture();
    let s = connected(&engine, id, client).await;
    let r = resource(&s, ResourceKind::CustomResourceDefinitions);
    let (result, ()) = tokio::join!(engine.resources(r.clone()), async {
        started.notified().await;
        engine.cancel_resources(r.clone()).unwrap();
    });
    assert_eq!(result, Err(ConnectionError::StaleSession));
    // A newer read can enter the same lane after cancellation.
    let mut next = r;
    next.read_id += 1;
    let (result, ()) = tokio::join!(engine.resources(next.clone()), async {
        started.notified().await;
        engine.cancel_resources(next).unwrap();
    });
    assert_eq!(result, Err(ConnectionError::StaleSession));
}

#[tokio::test]
async fn crd_single_retry_rejects_ignored_limit_and_repeated_tokens() {
    for repeated in [false, true] {
        let client = fake(move |path| {
            if path.contains("limit=10&") {
                return (200, vec![b' '; 2 * 1024 * 1024 + 1]);
            }
            let items = if repeated {
                vec![definition("a.example.io", 0)]
            } else {
                vec![definition("a.example.io", 0), definition("b.example.io", 0)]
            };
            (
                200,
                serde_json::to_vec(&json!({"metadata":{"continue":"same"},"items":items})).unwrap(),
            )
        });
        let mut r = request(ResourceKind::CustomResourceDefinitions);
        r.continue_token = Some("same".into());
        assert_eq!(
            transport::resources(&client, &r, &CancellationToken::new()).await,
            Err(if repeated {
                ConnectionError::InvalidResponse
            } else {
                ConnectionError::LimitExceeded
            })
        );
    }
}
