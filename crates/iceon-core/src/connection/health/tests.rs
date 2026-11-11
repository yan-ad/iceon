use super::*;
use crate::catalog::{ImportRequest, RemoveRequest};
use crate::connection::tests::{CONFIG, connected, fake, fixture, identity, normal};
use std::sync::atomic::AtomicUsize;

#[tokio::test]
async fn switched_workspace_keeps_only_read_only_health_and_latest_failure_wins() {
    let (root, catalog, engine, a, _) = fixture();
    let failing = Arc::new(AtomicBool::new(false));
    let requests = Arc::new(AtomicUsize::new(0));
    let fail = failing.clone();
    let calls = requests.clone();
    let first = connected(
        &engine,
        a.clone(),
        fake(move |path| {
            assert_eq!(path, "/version");
            calls.fetch_add(1, Ordering::SeqCst);
            if fail.load(Ordering::SeqCst) {
                (503, vec![])
            } else {
                normal(path)
            }
        }),
    )
    .await;
    let lease = engine
        .session
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .cancel
        .clone();
    let path = root.path().join("other");
    std::fs::write(&path, CONFIG).unwrap();
    let b = catalog
        .import_kubeconfig(ImportRequest {
            path: path.to_str().unwrap().into(),
        })
        .unwrap()
        .clusters
        .into_iter()
        .find(|c| c.id != a)
        .unwrap()
        .id;
    let second = connected(&engine, b.clone(), fake(normal)).await;
    assert!(lease.is_cancelled());
    assert_eq!(
        engine.status(identity(&first)),
        Err(ConnectionError::StaleSession)
    );
    failing.store(true, Ordering::SeqCst);
    let health = engine.refresh_cluster_health().await.unwrap();
    assert_eq!(
        health.iter().find(|h| h.cluster_id == a).unwrap().status,
        "error"
    );
    assert_eq!(
        health.iter().find(|h| h.cluster_id == b).unwrap().status,
        "connected"
    );
    assert_eq!(requests.load(Ordering::SeqCst), 2);
    engine.disconnect(identity(&first)).unwrap(); // stale disposal must not clear A
    assert_eq!(engine.cluster_health_snapshot().unwrap().len(), 2);
    engine.disconnect(identity(&second)).unwrap();
    assert_eq!(engine.cluster_health_snapshot().unwrap().len(), 1);
    failing.store(false, Ordering::SeqCst);
    assert_eq!(
        engine.refresh_cluster_health().await.unwrap()[0].status,
        "connected"
    );
    catalog.remove(RemoveRequest { id: a }).unwrap();
    assert!(engine.refresh_cluster_health().await.unwrap().is_empty());
}

#[tokio::test]
async fn active_success_does_not_hide_health_error_and_source_changes_drop_credentials() {
    let (_root, catalog, engine, id, target) = fixture();
    let fail = Arc::new(AtomicBool::new(false));
    let control = fail.clone();
    connected(
        &engine,
        id,
        fake(move |path| {
            if control.load(Ordering::SeqCst) {
                (401, vec![])
            } else {
                normal(path)
            }
        }),
    )
    .await;
    fail.store(true, Ordering::SeqCst);
    let health = engine.refresh_cluster_health().await.unwrap();
    assert_eq!(health[0].error, Some(ConnectionError::AuthFailed));
    let mut snapshot = catalog.list().unwrap();
    engine.project_catalog_statuses(&mut snapshot).unwrap();
    assert_eq!(snapshot.clusters[0].status, "error");
    std::fs::write(
        target.source_path,
        CONFIG.replace("fixture.invalid", "changed.invalid"),
    )
    .unwrap();
    assert!(engine.refresh_cluster_health().await.unwrap().is_empty());
    engine.project_catalog_statuses(&mut snapshot).unwrap();
    assert_eq!(snapshot.clusters[0].status, "disconnected");
}

fn gated_client(gate: Arc<tokio::sync::Notify>, started: Arc<tokio::sync::Notify>) -> Client {
    let calls = Arc::new(AtomicUsize::new(0));
    Client::new(
        tower::service_fn(move |request: http::Request<kube::client::Body>| {
            assert_eq!(request.uri().path(), "/version");
            let gate = gate.clone();
            let started = started.clone();
            let wait = calls.fetch_add(1, Ordering::SeqCst) > 0;
            async move {
                if wait {
                    started.notify_one();
                    gate.notified().await;
                }
                Ok::<_, std::io::Error>(http::Response::new(kube::client::Body::from(
                    normal("/version").1,
                )))
            }
        }),
        "default",
    )
}

#[tokio::test]
async fn in_flight_shutdown_disconnect_removal_and_reconnect_cannot_publish_stale_health() {
    for action in ["shutdown", "disconnect", "remove", "reconnect", "source"] {
        let (_root, catalog, engine, id, target) = fixture();
        let gate = Arc::new(tokio::sync::Notify::new());
        let started = Arc::new(tokio::sync::Notify::new());
        let session = connected(
            &engine,
            id.clone(),
            gated_client(gate.clone(), started.clone()),
        )
        .await;
        let pending = engine.refresh_cluster_health();
        tokio::pin!(pending);
        tokio::select! { _ = &mut pending => panic!("probe should wait"), _ = started.notified() => {} }
        assert_eq!(
            engine.refresh_cluster_health().await,
            Err(ConnectionError::Busy)
        );
        match action {
            "shutdown" => engine.shutdown(),
            "disconnect" => engine.disconnect(identity(&session)).unwrap(),
            "remove" => {
                catalog.remove(RemoveRequest { id: id.clone() }).unwrap();
            }
            "source" => {
                std::fs::write(target.source_path, "changed").unwrap();
            }
            "reconnect" => {
                connected(&engine, id, fake(|_| (503, vec![]))).await;
            }
            _ => unreachable!(),
        }
        gate.notify_one();
        let health = pending.await.unwrap();
        assert!(health.is_empty(), "{action}");
    }
}

#[tokio::test]
async fn timeout_is_bounded_and_health_limit_evicts_and_cancels_oldest() {
    let (_root, _catalog, engine, id, target) = fixture();
    let gate = Arc::new(tokio::sync::Notify::new());
    let started = Arc::new(tokio::sync::Notify::new());
    connected(&engine, id.clone(), gated_client(gate, started)).await;
    tokio::time::pause();
    assert_eq!(
        engine.refresh_cluster_health().await.unwrap()[0].error,
        Some(ConnectionError::Timeout)
    );
    let source = source_identity(target.clone()).await.unwrap();
    let old_cancel = engine.health.entries.lock().unwrap()[&id].cancel.clone();
    for n in 0..MAX_CLUSTERS {
        engine
            .health
            .register(
                format!("test-{n}"),
                target.clone(),
                source,
                fake(normal),
                "v1".into(),
            )
            .unwrap();
    }
    assert!(old_cancel.is_cancelled());
    assert_eq!(engine.health.entries.lock().unwrap().len(), MAX_CLUSTERS);
    engine.shutdown();
    assert!(engine.health.entries.lock().unwrap().is_empty());
}

#[tokio::test]
async fn expired_credentials_never_issue_a_health_request() {
    let (_root, _catalog, engine, id, _) = fixture();
    connected(&engine, id.clone(), fake(normal)).await;
    {
        let mut entries = engine.health.entries.lock().unwrap();
        entries.get_mut(&id).unwrap().client = fake(|_| panic!("expired credentials sent"))
            .with_valid_until(Some("2000-01-01T00:00:00Z".parse().unwrap()));
    }
    assert_eq!(
        engine.refresh_cluster_health().await.unwrap()[0].error,
        Some(ConnectionError::AuthExpired)
    );
}

#[tokio::test]
async fn health_concurrency_is_separate_and_never_exceeds_two() {
    let (root, catalog, engine, first, _) = fixture();
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    for n in 0..5 {
        let id = if n == 0 {
            first.clone()
        } else {
            let path = root.path().join(format!("source-{n}"));
            std::fs::write(&path, CONFIG).unwrap();
            catalog
                .import_kubeconfig(ImportRequest {
                    path: path.to_str().unwrap().into(),
                })
                .unwrap();
            catalog.connection_target(&first).unwrap();
            // Select only the newly imported source rather than relying on catalog ordering.
            let ids = catalog.list().unwrap().clusters;
            ids.into_iter()
                .find(|c| catalog.connection_target(&c.id).unwrap().provenance_path == path)
                .unwrap()
                .id
        };
        let active = active.clone();
        let peak = peak.clone();
        let client = Client::new(
            tower::service_fn(move |request: http::Request<kube::client::Body>| {
                assert_eq!(request.uri().path(), "/version");
                let active = active.clone();
                let peak = peak.clone();
                async move {
                    let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(count, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    Ok::<_, std::io::Error>(http::Response::new(kube::client::Body::from(
                        normal("/version").1,
                    )))
                }
            }),
            "default",
        );
        connected(&engine, id, client).await;
    }
    let _workspace = engine.workers.acquire_many(4).await.unwrap();
    assert_eq!(engine.refresh_cluster_health().await.unwrap().len(), 5);
    assert_eq!(peak.load(Ordering::SeqCst), 2);
}
