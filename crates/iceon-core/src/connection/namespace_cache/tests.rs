use super::super::tests::{connected, fake, fixture, identity, normal};
use super::*;

async fn bind(engine: &ConnectionEngine, request: &SessionRequest, target: ConnectionTarget) {
    let identity = source_identity_async(target).await.unwrap();
    let mut state = engine.session.lock().unwrap();
    current_mut(&mut state, request)
        .unwrap()
        .namespace_cache
        .bind(
            identity,
            engine.authentication_generation.load(Ordering::SeqCst),
        );
}
fn client(user: &'static str) -> Client {
    fake(move |path| {
        if path == "/apis/authentication.k8s.io/v1/selfsubjectreviews" {
            (201, serde_json::to_vec(&json!({"apiVersion":"authentication.k8s.io/v1","kind":"SelfSubjectReview","status":{"userInfo":{"username":user,"groups":["team"]}}})).unwrap())
        } else {
            normal(path)
        }
    })
}
fn refresh(scope: &SessionRequest, read_id: u64) -> NamespaceRefreshRequest {
    NamespaceRefreshRequest {
        cluster_id: scope.cluster_id.clone(),
        session_id: scope.session_id.clone(),
        read_id,
    }
}
#[tokio::test]
async fn restart_snapshot_is_isolated_by_source_and_server_identity() {
    let (_root, catalog, engine, id, target) = fixture();
    let engine = Arc::new(engine);
    let session = connected(&engine, id.clone(), client("alice")).await;
    let scope = identity(&session);
    bind(&engine, &scope, target.clone()).await;
    assert!(
        engine
            .namespace_cache_get(scope.clone())
            .await
            .unwrap()
            .is_none()
    );
    let snapshot = engine
        .namespace_cache_refresh(refresh(&scope, 100))
        .await
        .unwrap();
    assert_eq!(snapshot.names, ["default"]);
    drop(engine);
    let restarted = ConnectionEngine::new(catalog.clone());
    let session = connected(&restarted, id.clone(), client("alice")).await;
    let scope = identity(&session);
    bind(&restarted, &scope, target.clone()).await;
    assert_eq!(
        restarted
            .namespace_cache_get(scope.clone())
            .await
            .unwrap()
            .unwrap()
            .names,
        ["default"]
    );
    let other = ConnectionEngine::new(catalog.clone());
    let session = connected(&other, id, client("bob")).await;
    let other_scope = identity(&session);
    bind(&other, &other_scope, target.clone()).await;
    assert!(
        other
            .namespace_cache_get(other_scope)
            .await
            .unwrap()
            .is_none()
    );
    std::fs::write(
        &target.source_path,
        super::super::tests::CONFIG.replace("PRIVATE_FIXTURE_TOKEN", "ROTATED_FIXTURE_TOKEN"),
    )
    .unwrap();
    assert!(
        restarted
            .namespace_cache_get(scope)
            .await
            .unwrap()
            .is_none()
    );
}
#[tokio::test]
async fn expired_or_malformed_snapshots_are_not_returned() {
    let (_root, catalog, engine, id, target) = fixture();
    let engine = Arc::new(engine);
    let session = connected(&engine, id, client("alice")).await;
    let scope = identity(&session);
    bind(&engine, &scope, target).await;
    engine
        .namespace_cache_refresh(refresh(&scope, 100))
        .await
        .unwrap();
    catalog
        .namespace_cache_storage(|connection| {
            connection
                .execute("UPDATE namespace_snapshots SET expires_at=1", [])
                .unwrap();
            Ok(())
        })
        .unwrap();
    assert!(
        engine
            .namespace_cache_get(scope.clone())
            .await
            .unwrap()
            .is_none()
    );
    engine
        .namespace_cache_refresh(refresh(&scope, 101))
        .await
        .unwrap();
    catalog
        .namespace_cache_storage(|connection| {
            connection
                .execute("UPDATE namespace_snapshots SET names='[\"bad/name\"]'", [])
                .unwrap();
            Ok(())
        })
        .unwrap();
    assert!(engine.namespace_cache_get(scope).await.unwrap().is_none());
}
#[tokio::test]
async fn failed_refresh_retains_snapshot_and_stale_sessions_cannot_read_it() {
    let (_root, _catalog, engine, id, target) = fixture();
    let engine = Arc::new(engine);
    let session = connected(&engine, id, client("alice")).await;
    let scope = identity(&session);
    bind(&engine, &scope, target).await;
    engine
        .namespace_cache_refresh(refresh(&scope, 100))
        .await
        .unwrap();
    {
        let mut state = engine.session.lock().unwrap();
        current_mut(&mut state, &scope).unwrap().client = Some(fake(|_| (500, Vec::new())));
    }
    assert!(
        engine
            .namespace_cache_refresh(refresh(&scope, 101))
            .await
            .is_err()
    );
    assert_eq!(
        engine
            .namespace_cache_get(scope.clone())
            .await
            .unwrap()
            .unwrap()
            .names,
        ["default"]
    );
    engine.disconnect(scope.clone()).unwrap();
    assert_eq!(
        engine.namespace_cache_get(scope).await.unwrap_err(),
        ConnectionError::StaleSession
    );
}
