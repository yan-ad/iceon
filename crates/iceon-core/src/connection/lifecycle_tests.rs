use super::tests::{CONFIG, connected, fake, fixture, identity, normal, resource};
use super::*;
use crate::catalog::RemoveRequest;
use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

#[tokio::test]
async fn cancelled_read_cannot_start_late_or_cancel_replacement() {
    let (_root, _catalog, engine, id, _) = fixture();
    let requests = Arc::new(AtomicUsize::new(0));
    let count = requests.clone();
    let snapshot = connected(
        &engine,
        id,
        fake(move |path| {
            count.fetch_add(1, Ordering::SeqCst);
            normal(path)
        }),
    )
    .await;
    let old = resource(&snapshot, ResourceKind::Nodes);
    engine.cancel_resources(old.clone()).unwrap();
    assert_eq!(
        engine.resources(old.clone()).await,
        Err(ConnectionError::StaleSession)
    );
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    let mut replacement = old.clone();
    replacement.read_id = 2;
    engine.cancel_resources(old).unwrap();
    let page = engine.resources(replacement.clone()).await.unwrap();
    assert_eq!(page.read_id, 2);
    assert_eq!(
        engine.resources(replacement).await,
        Err(ConnectionError::StaleSession)
    );
}

#[tokio::test]
async fn malformed_read_ids_are_rejected_before_client_work() {
    let (_root, _catalog, engine, id, _) = fixture();
    let snapshot = connected(&engine, id, fake(normal)).await;
    for read_id in [0, 9_007_199_254_740_992] {
        let mut request = resource(&snapshot, ResourceKind::Nodes);
        request.read_id = read_id;
        assert_eq!(
            engine.resources(request.clone()).await,
            Err(ConnectionError::InvalidInput)
        );
        assert_eq!(
            engine.cancel_resources(request),
            Err(ConnectionError::InvalidInput)
        );
    }
}

#[tokio::test]
async fn removed_catalog_target_never_reaches_version_probe() {
    let (_root, catalog, engine, id, _) = fixture();
    let opened = engine
        .open(OpenRequest {
            cluster_id: id.clone(),
        })
        .await
        .unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let count = requests.clone();
    let failed = engine
        .connect_with(identity(&opened), |_| async {
            catalog.remove(RemoveRequest { id }).unwrap();
            Ok(fake(move |path| {
                count.fetch_add(1, Ordering::SeqCst);
                normal(path)
            }))
        })
        .await
        .unwrap();
    assert_eq!(failed.error, Some(ConnectionError::NotFound));
    assert_eq!(requests.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn client_construction_uses_selected_static_configuration_without_network() {
    let (_root, _catalog, _engine, _id, target) = fixture();
    let admission = Arc::new(tokio::sync::Semaphore::new(2));
    let result = config::build_client(
        target.clone(),
        admission.clone(),
        tokio_util::task::TaskTracker::new(),
    )
    .await;
    assert!(result.is_ok());
    fs::write(
        &target.source_path,
        CONFIG.replace(
            "token: PRIVATE_FIXTURE_TOKEN",
            "exec: {command: NEVER_EXECUTE}",
        ),
    )
    .unwrap();
    assert_eq!(
        config::build_client(target, admission, tokio_util::task::TaskTracker::new())
            .await
            .err(),
        Some(ConnectionError::KubeconfigInvalid)
    );
}

#[tokio::test]
async fn empty_page_is_valid_and_transport_failure_does_not_block_refresh() {
    let (_root, _catalog, engine, id, _) = fixture();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let snapshot = connected(
        &engine,
        id,
        fake(move |path| {
            if path == "/version" {
                normal(path)
            } else if count.fetch_add(1, Ordering::SeqCst) == 0 {
                (504, b"PRIVATE timeout".to_vec())
            } else {
                (200, br#"{"metadata":{},"items":[]}"#.to_vec())
            }
        }),
    )
    .await;
    let first = resource(&snapshot, ResourceKind::Namespaces);
    assert_eq!(
        engine.resources(first.clone()).await,
        Err(ConnectionError::Timeout)
    );
    let mut retry = first;
    retry.read_id = 2;
    let page = engine.resources(retry).await.unwrap();
    assert!(page.items.is_empty());
    assert!(!page.truncated);
}
