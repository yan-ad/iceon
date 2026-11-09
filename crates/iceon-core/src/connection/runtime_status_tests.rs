use super::{tests::*, *};
use crate::catalog::{PinRequest, RemoveRequest};

fn projected(
    engine: &ConnectionEngine,
    catalog: &CatalogEngine,
) -> crate::catalog::CatalogSnapshot {
    let mut snapshot = catalog.list().unwrap();
    engine.project_catalog_statuses(&mut snapshot).unwrap();
    snapshot
}

#[tokio::test]
async fn status_projection_tracks_open_connect_failure_disconnect_and_shutdown() {
    let (_root, catalog, engine, id, _) = fixture();
    assert!(engine.catalog_connection_states().unwrap().is_empty());
    let opened = engine
        .open(OpenRequest {
            cluster_id: id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(
        projected(&engine, &catalog).clusters[0].status,
        "disconnected"
    );
    let failed = engine
        .connect_with(identity(&opened), |_| async {
            Err(ConnectionError::AuthFailed)
        })
        .await
        .unwrap();
    assert_eq!(failed.status, "error");
    assert_eq!(projected(&engine, &catalog).clusters[0].status, "error");
    let live = engine
        .connect_with(identity(&opened), |_| async { Ok(fake(normal)) })
        .await
        .unwrap();
    let states = engine.catalog_connection_states().unwrap();
    assert_eq!(
        serde_json::to_value(&states).unwrap(),
        serde_json::json!([
            {"clusterId": id, "status": "connected"}
        ])
    );
    assert_eq!(projected(&engine, &catalog).clusters[0].status, "connected");
    engine.disconnect(identity(&live)).unwrap();
    assert_eq!(
        projected(&engine, &catalog).clusters[0].status,
        "disconnected"
    );
    assert!(engine.catalog_connection_states().unwrap().is_empty());
    connected(&engine, id, fake(normal)).await;
    engine.shutdown();
    assert!(engine.catalog_connection_states().unwrap().is_empty());
    assert_eq!(
        projected(&engine, &catalog).clusters[0].status,
        "disconnected"
    );
}

#[tokio::test]
async fn connecting_is_observed_and_late_completion_cannot_replace_a_new_lease() {
    let (_root, catalog, engine, id, _) = fixture();
    let opened = engine
        .open(OpenRequest {
            cluster_id: id.clone(),
        })
        .await
        .unwrap();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let pending = engine.connect_with(identity(&opened), |_| async {
        started_tx.send(()).unwrap();
        std::future::pending::<Result<Client>>().await
    });
    tokio::pin!(pending);
    tokio::select! {
        _ = &mut pending => panic!("connection should wait"),
        _ = started_rx => {}
    }
    assert_eq!(
        projected(&engine, &catalog).clusters[0].status,
        "connecting"
    );
    let replacement = connected(&engine, id.clone(), fake(normal)).await;
    engine.disconnect(identity(&opened)).unwrap();
    assert_eq!(pending.await, Err(ConnectionError::StaleSession));
    assert_eq!(
        engine.status(identity(&replacement)).unwrap().status,
        "connected"
    );
    assert_eq!(projected(&engine, &catalog).clusters[0].status, "connected");

    let other_path = _root.path().join("other-config");
    std::fs::write(&other_path, CONFIG).unwrap();
    let other = catalog
        .import_kubeconfig(crate::catalog::ImportRequest {
            path: other_path.to_str().unwrap().into(),
        })
        .unwrap()
        .clusters
        .into_iter()
        .find(|cluster| cluster.id != id)
        .unwrap();
    connected(&engine, other.id.clone(), fake(normal)).await;
    engine.disconnect(identity(&replacement)).unwrap();
    let snapshot = projected(&engine, &catalog);
    assert_eq!(
        snapshot
            .clusters
            .iter()
            .find(|c| c.id == id)
            .unwrap()
            .status,
        "disconnected"
    );
    assert_eq!(
        snapshot
            .clusters
            .iter()
            .find(|c| c.id == other.id)
            .unwrap()
            .status,
        "connected"
    );
    assert_eq!(engine.catalog_connection_states().unwrap().len(), 1);
}

#[tokio::test]
async fn projection_is_ephemeral_preserves_metadata_and_does_not_invent_removed_rows() {
    let (_root, catalog, engine, id, _) = fixture();
    connected(&engine, id.clone(), fake(normal)).await;
    let mut saved = catalog
        .set_pin(PinRequest {
            id: id.clone(),
            pinned: true,
        })
        .unwrap();
    let baseline = saved.clone();
    engine.project_catalog_statuses(&mut saved).unwrap();
    assert_eq!(saved.clusters[0].status, "connected");
    saved.clusters[0].status = "disconnected";
    assert_eq!(saved, baseline);
    assert_eq!(catalog.list().unwrap(), baseline);
    let mut added = catalog.list().unwrap();
    engine.project_catalog_statuses(&mut added).unwrap();
    assert_eq!(added.clusters[0].status, "connected");
    let mut removed = catalog.remove(RemoveRequest { id }).unwrap();
    engine.project_catalog_statuses(&mut removed).unwrap();
    assert!(removed.clusters.is_empty());
    engine.shutdown();
    engine.project_catalog_statuses(&mut added).unwrap();
    assert_eq!(added.clusters[0].status, "disconnected");
}

#[tokio::test]
async fn cancelled_or_clientless_sessions_never_report_connected() {
    let (_root, catalog, engine, id, _) = fixture();
    connected(&engine, id, fake(normal)).await;
    engine.session.lock().unwrap().as_mut().unwrap().client = None;
    assert_eq!(
        projected(&engine, &catalog).clusters[0].status,
        "disconnected"
    );
    assert_eq!(
        engine.catalog_connection_states().unwrap()[0].status,
        "disconnected"
    );
    {
        let mut state = engine.session.lock().unwrap();
        let session = state.as_mut().unwrap();
        session.client = Some(fake(normal));
        session.cancel.cancel();
    }
    assert_eq!(
        projected(&engine, &catalog).clusters[0].status,
        "disconnected"
    );
    assert_eq!(
        engine.catalog_connection_states().unwrap()[0].status,
        "disconnected"
    );
}

#[test]
fn poisoned_session_fails_closed() {
    let (_root, catalog, engine, _id, _) = fixture();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = engine.session.lock().unwrap();
        panic!("poison fixture lock");
    }));
    assert_eq!(
        engine.catalog_connection_states(),
        Err(ConnectionError::StoreUnavailable)
    );
    let mut snapshot = catalog.list().unwrap();
    assert_eq!(
        engine.project_catalog_statuses(&mut snapshot),
        Err(ConnectionError::StoreUnavailable)
    );
}
