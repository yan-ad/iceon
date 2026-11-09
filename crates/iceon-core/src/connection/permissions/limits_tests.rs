use super::*;
use crate::connection::tests::{fixture, identity};
use http::{Request, Response};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

// These fixtures hold both process-global review slots before releasing replies.
static BATCH_FIXTURE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct BatchActivity {
    active: AtomicUsize,
    maximum: AtomicUsize,
    started: Mutex<Vec<CancellationToken>>,
    started_permits: tokio::sync::Semaphore,
    release: Option<tokio::sync::Semaphore>,
    completed: Mutex<Vec<String>>,
    node_one_completed: tokio::sync::Notify,
}

impl Default for BatchActivity {
    fn default() -> Self {
        Self {
            active: AtomicUsize::new(0),
            maximum: AtomicUsize::new(0),
            started: Mutex::new(Vec::new()),
            started_permits: tokio::sync::Semaphore::new(0),
            release: None,
            completed: Mutex::new(Vec::new()),
            node_one_completed: tokio::sync::Notify::new(),
        }
    }
}

struct ActiveReview(Arc<BatchActivity>);

impl Drop for ActiveReview {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}

fn delayed_client(activity: Arc<BatchActivity>) -> Client {
    Client::new(
        tower::service_fn(move |request: Request<Body>| {
            let activity = activity.clone();
            async move {
                if request.uri().path() == "/version" {
                    return Ok::<_, std::io::Error>(Response::new(Body::from(
                        serde_json::to_vec(&json!({"gitVersion":"v1.36.0"})).unwrap(),
                    )));
                }
                assert_eq!(request.uri().path(), REVIEW_PATH);
                let cancel = request
                    .extensions()
                    .get::<CancellationToken>()
                    .unwrap()
                    .clone();
                let bytes = request.into_body().collect().await.unwrap().to_bytes();
                let mut value: Value = serde_json::from_slice(&bytes).unwrap();
                let name = value["spec"]["resourceAttributes"]["name"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                let delay = tokio::time::sleep(Duration::from_millis(400));
                activity.started.lock().unwrap().push(cancel);
                let active = activity.active.fetch_add(1, Ordering::SeqCst) + 1;
                activity.maximum.fetch_max(active, Ordering::SeqCst);
                let _active = ActiveReview(activity.clone());
                activity.started_permits.add_permits(1);
                if let Some(release) = &activity.release {
                    release.acquire().await.unwrap().forget();
                } else {
                    delay.await;
                    if name == "node-0" {
                        activity.node_one_completed.notified().await;
                    }
                }
                let notify_node_zero = name == "node-1";
                activity.completed.lock().unwrap().push(name);
                if notify_node_zero {
                    activity.node_one_completed.notify_one();
                }
                value["status"] = json!({"allowed":true});
                Ok(Response::new(Body::from(
                    serde_json::to_vec(&value).unwrap(),
                )))
            }
        }),
        "default",
    )
}

fn batch_request(identity: &SessionRequest) -> ReviewRequest {
    ReviewRequest {
        cluster_id: identity.cluster_id.clone(),
        session_id: identity.session_id.clone(),
        read_id: 1,
        checks: (0..32)
            .map(|index| PermissionCheck {
                kind: ResourceKind::Nodes,
                action: PermissionAction::Get,
                namespace: None,
                name: Some(format!("node-{index}")),
            })
            .collect(),
    }
}

#[tokio::test]
async fn full_batch_reviews_two_at_a_time_within_deadline_in_request_order() {
    let _batch = BATCH_FIXTURE.lock().await;
    let (_root, _catalog, engine, id, _) = fixture();
    let activity = Arc::new(BatchActivity::default());
    let snapshot =
        crate::connection::tests::connected(&engine, id, delayed_client(activity.clone())).await;
    let mut request = batch_request(&identity(&snapshot));
    tokio::time::pause();
    let started = tokio::time::Instant::now();
    let drive_clock = async {
        let wall_started = std::time::Instant::now();
        let mut previous_completed = None;
        loop {
            assert!(
                wall_started.elapsed() < Duration::from_secs(30),
                "review batch stopped making progress: started {}, active {}, completed {:?}, elapsed {:?}",
                activity.started.lock().unwrap().len(),
                activity.active.load(Ordering::SeqCst),
                activity.completed.lock().unwrap(),
                started.elapsed()
            );
            let completed = activity.completed.lock().unwrap().len();
            if completed == 32 {
                break;
            }
            let active = activity.active.load(Ordering::SeqCst);
            let all_started = activity.started.lock().unwrap().len() == 32;
            // Other test runtimes share REVIEW_WORKERS. Keep this runtime runnable
            // while they own a permit, rather than auto-advancing to our deadline.
            if previous_completed != Some(completed)
                && (active == 2 || (all_started && active == 1))
            {
                previous_completed = Some(completed);
                // Cross the timer wheel tick containing each 400 ms delay.
                tokio::time::advance(Duration::from_millis(401)).await;
            }
            tokio::task::yield_now().await;
        }
    };
    let (response, ()) = tokio::join!(engine.review_permissions(request.clone()), drive_clock);
    let response = response.unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "batch elapsed {:?}, completed {:?}",
        started.elapsed(),
        activity.completed.lock().unwrap()
    );
    assert_eq!(activity.maximum.load(Ordering::SeqCst), 2);
    assert_eq!(activity.active.load(Ordering::SeqCst), 0);
    assert_eq!(activity.started.lock().unwrap().len(), 32);
    assert_eq!(activity.completed.lock().unwrap()[0], "node-1");
    assert_eq!(
        response
            .items
            .iter()
            .map(|item| item.check.clone())
            .collect::<Vec<_>>(),
        request.checks
    );
    assert!(
        response
            .items
            .iter()
            .all(|item| item.decision == PermissionDecision::Allowed)
    );
    request.read_id += 1;
    engine.review_permissions(request).await.unwrap();
    assert_eq!(activity.started.lock().unwrap().len(), 32);
}

#[tokio::test]
async fn cancelling_or_invalidating_concurrent_batch_stops_queued_reviews() {
    let _batch = BATCH_FIXTURE.lock().await;
    for invalidate in [false, true] {
        let (_root, _catalog, engine, id, _) = fixture();
        let activity = Arc::new(BatchActivity {
            release: Some(tokio::sync::Semaphore::new(0)),
            ..BatchActivity::default()
        });
        let snapshot =
            crate::connection::tests::connected(&engine, id, delayed_client(activity.clone()))
                .await;
        let identity = identity(&snapshot);
        let request = batch_request(&identity);
        tokio::time::pause();
        let review = engine.review_permissions(request.clone());
        let cancel = async {
            let waiting = std::time::Instant::now();
            while activity.started_permits.available_permits() < 2 {
                assert!(
                    waiting.elapsed() < Duration::from_secs(30),
                    "review transports did not start before cancellation test deadline"
                );
                tokio::task::yield_now().await;
            }
            activity
                .started_permits
                .acquire_many(2)
                .await
                .unwrap()
                .forget();
            assert_eq!(activity.active.load(Ordering::SeqCst), 2);
            if invalidate {
                engine.refresh_permissions(identity).unwrap();
            } else {
                engine.cancel_permissions(request).unwrap();
            }
        };
        let (result, _) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(review, cancel)
        })
        .await
        .expect("cancelled or invalidated review batch must finish");
        assert!(matches!(result, Err(ConnectionError::StaleSession)));
        assert!(
            engine
                .session
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .permissions
                .cache
                .is_empty()
        );
        assert_eq!(activity.started.lock().unwrap().len(), 2);
        assert_eq!(activity.maximum.load(Ordering::SeqCst), 2);
        assert_eq!(activity.active.load(Ordering::SeqCst), 0);
        assert!(activity.completed.lock().unwrap().is_empty());
        assert!(
            activity
                .started
                .lock()
                .unwrap()
                .iter()
                .all(CancellationToken::is_cancelled)
        );
        tokio::time::resume();
    }
}

#[tokio::test]
async fn concurrent_batch_rejects_cached_item_expiring_before_publication() {
    let _batch = BATCH_FIXTURE.lock().await;
    let (_root, _catalog, engine, id, _) = fixture();
    let activity = Arc::new(BatchActivity {
        release: Some(tokio::sync::Semaphore::new(0)),
        ..BatchActivity::default()
    });
    let snapshot =
        crate::connection::tests::connected(&engine, id, delayed_client(activity.clone())).await;
    let mut request = batch_request(&identity(&snapshot));
    request.checks.truncate(3);
    let check = request.checks[0].clone();
    tokio::time::pause();
    {
        let mut state = engine.session.lock().unwrap();
        let permissions = &mut state.as_mut().unwrap().permissions;
        permissions.synchronize(&engine.authentication_generation);
        permissions.cache.insert(
            check.clone(),
            Cached {
                item: PermissionItem {
                    check,
                    decision: PermissionDecision::Allowed,
                    reason: None,
                },
                expires: now() + 30_000,
                deadline: tokio::time::Instant::now() + Duration::from_secs(1),
                used: 1,
            },
        );
    }
    let expire_during_review = async {
        let waiting = std::time::Instant::now();
        // Shared review workers can be owned by a different test runtime.
        while activity.started_permits.available_permits() < 2 {
            assert!(
                waiting.elapsed() < Duration::from_secs(30),
                "uncached reviews did not start before cached-expiry test deadline"
            );
            tokio::task::yield_now().await;
        }
        activity
            .started_permits
            .acquire_many(2)
            .await
            .unwrap()
            .forget();
        tokio::time::advance(Duration::from_secs(2)).await;
        activity.release.as_ref().unwrap().add_permits(2);
    };
    let (result, ()) = tokio::join!(engine.review_permissions(request), expire_during_review);
    assert!(matches!(result, Err(ConnectionError::StaleSession)));
    assert_eq!(activity.started.lock().unwrap().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn review_timeout_and_cancel_drop_pending_transport() {
    let pending = Client::new(
        tower::service_fn(|_: Request<Body>| async {
            std::future::pending::<std::result::Result<Response<Body>, std::io::Error>>().await
        }),
        "default",
    );
    let check = PermissionCheck {
        kind: ResourceKind::Nodes,
        action: PermissionAction::List,
        namespace: None,
        name: None,
    };
    let started = tokio::time::Instant::now();
    let item = review_check(&pending, &check, &CancellationToken::new()).await;
    assert_eq!(item.decision, PermissionDecision::Unknown);
    assert!(started.elapsed() <= Duration::from_secs(5));
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert_eq!(
        review_check(&pending, &check, &cancel).await.reason,
        Some(PermissionReason::Unavailable)
    );
}

#[tokio::test]
async fn monotonic_expiry_rejects_a_cached_deny_after_ttl() {
    let (_root, _catalog, engine, id, _) = fixture();
    let snapshot = engine
        .open(crate::connection::OpenRequest { cluster_id: id })
        .await
        .unwrap();
    let identity = identity(&snapshot);
    let check = PermissionCheck {
        kind: ResourceKind::Nodes,
        action: PermissionAction::List,
        namespace: None,
        name: None,
    };
    {
        let mut state = engine.session.lock().unwrap();
        let permissions = &mut state.as_mut().unwrap().permissions;
        permissions.synchronize(&engine.authentication_generation);
        permissions.cache.insert(
            check.clone(),
            Cached {
                item: PermissionItem {
                    check: check.clone(),
                    decision: PermissionDecision::Denied,
                    reason: Some(PermissionReason::Denied),
                },
                expires: now() + 60_000,
                deadline: tokio::time::Instant::now() + Duration::from_secs(10),
                used: 1,
            },
        );
    }
    tokio::time::pause();
    assert_eq!(
        engine.reject_known_denied_read(&identity, &check),
        Err(ConnectionError::PermissionDenied)
    );
    tokio::time::advance(Duration::from_secs(11)).await;
    assert_eq!(engine.reject_known_denied_read(&identity, &check), Ok(()));
}

#[tokio::test]
async fn replaced_batch_does_not_publish_late_permission() {
    let (_root, _catalog, engine, id, _) = fixture();
    let started = Arc::new(tokio::sync::Semaphore::new(0));
    let transport_started = started.clone();
    let client = Client::new(
        tower::service_fn(move |request: Request<Body>| {
            let started = transport_started.clone();
            async move {
                if request.uri().path() == "/version" {
                    return Ok::<_, std::io::Error>(Response::new(Body::from(
                        serde_json::to_vec(&json!({"gitVersion":"v1.36.0"})).unwrap(),
                    )));
                }
                started.add_permits(1);
                std::future::pending::<std::result::Result<Response<Body>, std::io::Error>>().await
            }
        }),
        "default",
    );
    let snapshot = crate::connection::tests::connected(&engine, id, client).await;
    let identity = identity(&snapshot);
    let request = ReviewRequest {
        cluster_id: identity.cluster_id,
        session_id: identity.session_id,
        read_id: 1,
        checks: vec![PermissionCheck {
            kind: ResourceKind::Nodes,
            action: PermissionAction::List,
            namespace: None,
            name: None,
        }],
    };
    let review = engine.review_permissions(request.clone());
    let cancel = async {
        started.acquire().await.unwrap().forget();
        let mut newer = request.clone();
        newer.read_id = 2;
        engine.cancel_permissions(newer).unwrap();
    };
    let (result, _) = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(review, cancel)
    })
    .await
    .expect("replaced review batch must finish");
    assert!(matches!(result, Err(ConnectionError::StaleSession)));
    let state = engine.session.lock().unwrap();
    assert!(state.as_ref().unwrap().permissions.cache.is_empty());
}

#[test]
fn epoch_change_clears_cache() {
    let generation = Arc::new(AtomicU64::new(1));
    let mut scope = PermissionSession::default();
    scope.synchronize(&generation);
    let check = PermissionCheck {
        kind: ResourceKind::Nodes,
        action: PermissionAction::List,
        namespace: None,
        name: None,
    };
    scope.cache.insert(
        check.clone(),
        Cached {
            item: unknown(&check, PermissionReason::Unavailable),
            expires: now() + 2_000,
            deadline: tokio::time::Instant::now() + Duration::from_secs(2),
            used: 1,
        },
    );
    invalidate_authentication(&generation);
    scope.synchronize(&generation);
    assert!(scope.cache.is_empty());
    let _ = Mutex::new(scope);
}
