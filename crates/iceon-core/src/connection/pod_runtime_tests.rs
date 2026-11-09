use super::*;

fn scope() -> SessionRequest {
    SessionRequest {
        cluster_id: "cluster".into(),
        session_id: "lease".into(),
    }
}

#[tokio::test]
async fn denied_startups_do_not_allocate_runtime_slots_or_bind_listeners() {
    for mode in [
        PodStreamMode::Logs,
        PodStreamMode::Shell,
        PodStreamMode::Attach,
    ] {
        let (_root, _, engine, id, _) = crate::connection::tests::fixture();
        let reviews = Arc::new(Mutex::new(Vec::new()));
        let opened = crate::connection::tests::connected(
            &engine,
            id,
            crate::connection::tests::denied_permission_client(reviews, Some(false)),
        )
        .await;
        let scope = crate::connection::tests::identity(&opened);
        let mut request = start_request(&scope);
        request.mode = mode;
        assert!(matches!(
            engine.pod_stream_open(request).await,
            Err(ConnectionError::PermissionDenied)
        ));
        assert!(matches!(
            engine.pod_forward_start(forward_request(&scope)).await,
            Err(ConnectionError::PermissionDenied)
        ));
        assert_eq!(
            engine.pod_runtime.stream_slots.available_permits(),
            STREAM_LIMIT
        );
        assert_eq!(
            engine.pod_runtime.forward_slots.available_permits(),
            FORWARD_LIMIT
        );
        let registry = engine.pod_runtime.registry.lock().unwrap();
        assert!(registry.streams.is_empty());
        assert!(registry.forwards.is_empty());
    }
}

fn state(cancel: CancellationToken) -> StreamState {
    StreamState {
        scope: scope(),
        id: "stream".into(),
        cancel,
        ring: Ring::default(),
        status: "running",
        error: None,
        input: None,
        resize: None,
        finished: false,
    }
}
#[test]
fn ring_drops_oldest_on_codepoint_boundaries_and_reports_once() {
    let mut ring = Ring::default();
    ring.push(&"🙂".repeat(RING_LIMIT));
    assert!(ring.text.len() <= RING_LIMIT);
    assert!(ring.text.chars().all(|c| c == '🙂'));
    let mut received = String::new();
    let (first, truncated) = ring.drain();
    assert!(truncated);
    assert!(first.len() <= DRAIN_LIMIT);
    received.push_str(&first);
    while !ring.text.is_empty() {
        let (chunk, truncated) = ring.drain();
        assert!(!truncated);
        assert!(chunk.len() <= DRAIN_LIMIT);
        received.push_str(&chunk);
    }
    assert_eq!(received.len(), RING_LIMIT);
    assert_eq!(ring.sequence, 1);
}
#[test]
fn decoder_preserves_split_unicode_and_replaces_invalid_data() {
    let mut decoder = Decoder::default();
    assert_eq!(decoder.decode(&[0xf0, 0x9f], false), "");
    assert_eq!(decoder.decode(&[0x99, 0x82, b'x'], false), "🙂x");
    assert_eq!(decoder.decode(&[0xff, 0xe2], false), "�");
    assert_eq!(decoder.decode(&[], true), "�");
    assert!(decoder.pending.is_empty());
}
#[test]
fn target_shell_and_port_validation_reject_path_injection() {
    assert!(validate_target("default", "api-0.example", "uid").is_ok());
    for namespace in ["", "../default", "Default", "a/b", "a."] {
        assert!(validate_target(namespace, "pod", "uid").is_err());
    }
    assert!(validate_target("default", "../pod", "uid").is_err());
    assert!(validate_target("default", "pod", "").is_err());
    for shell in ["/bin/sh", "sh", "bash", "ash", "powershell", "cmd"] {
        assert!(shell_command(Some(shell)).is_ok());
    }
    for shell in ["sh -c id", "kubectl", "/tmp/sh", "bash;id"] {
        assert!(shell_command(Some(shell)).is_err());
    }
    assert_eq!(validate_port(0), Err(ConnectionError::InvalidInput));
    assert!(validate_port(65535).is_ok());
    let request: PodForwardRequest = serde_json::from_value(serde_json::json!({"clusterId":"c","sessionId":"s","namespace":"n","name":"p","uid":"u","remotePort":80})).unwrap();
    assert_eq!(request.local_port, 0);
}
#[test]
fn flattened_contracts_reject_unknown_fields() {
    let input = serde_json::json!({"clusterId":"c","sessionId":"s","streamId":"id","data":"hello"});
    assert!(serde_json::from_value::<PodInputRequest>(input.clone()).is_ok());
    let mut invalid = input;
    invalid["unknown"] = true.into();
    assert!(serde_json::from_value::<PodInputRequest>(invalid).is_err());
}
#[test]
fn stream_lookup_rejects_wrong_lease_and_cancelled_scope() {
    let runtime = PodRuntime::default();
    let cancel = CancellationToken::new();
    runtime
        .registry
        .lock()
        .unwrap()
        .streams
        .insert("stream".into(), Arc::new(Mutex::new(state(cancel.clone()))));
    let mut request = PodStreamRequest {
        cluster_id: "cluster".into(),
        session_id: "other".into(),
        stream_id: "stream".into(),
    };
    assert!(matches!(
        runtime.stream(&request),
        Err(ConnectionError::StaleSession)
    ));
    request.session_id = "lease".into();
    assert!(runtime.stream(&request).is_ok());
    cancel.cancel();
    assert!(matches!(
        runtime.stream(&request),
        Err(ConnectionError::StaleSession)
    ));
}
#[tokio::test]
async fn root_cancellation_reaps_owned_work_and_discards_output() {
    let runtime = PodRuntime::default();
    let root = CancellationToken::new();
    let slot = Arc::new(Mutex::new(state(root.child_token())));
    slot.lock().unwrap().ring.push("private output");
    let (sender, mut receiver) = mpsc::channel(1);
    sender.try_send("queued input".to_owned()).unwrap();
    slot.lock().unwrap().input = Some(sender);
    runtime
        .registry
        .lock()
        .unwrap()
        .streams
        .insert("stream".into(), slot.clone());
    let owned = slot.clone();
    runtime.tasks.spawn(async move {
        let cancel = owned.lock().unwrap().cancel.clone();
        cancel.cancelled().await;
        receiver.close();
        while receiver.try_recv().is_ok() {}
        owned
            .lock()
            .unwrap()
            .finish(Err(ConnectionError::StaleSession));
    });
    root.cancel();
    tokio::time::timeout(Duration::from_secs(1), runtime.drain())
        .await
        .unwrap();
    let state = slot.lock().unwrap();
    assert!(state.finished);
    assert!(state.ring.text.is_empty());
    assert!(state.input.is_none());
    assert!(runtime.registry.lock().unwrap().streams.is_empty());
    assert!(runtime.tasks.is_empty());
}
#[tokio::test]
async fn bounded_input_queue_applies_backpressure() {
    let (sender, _receiver) = mpsc::channel(8);
    for _ in 0..8 {
        sender.try_send("x".repeat(INPUT_LIMIT)).unwrap();
    }
    assert!(matches!(
        sender.try_send("overflow".into()),
        Err(mpsc::error::TrySendError::Full(_))
    ));
}

#[tokio::test]
async fn terminal_requires_running_container_but_logs_allow_missing_status() {
    let client = crate::connection::tests::fake(|_| {
        (200, br#"{"apiVersion":"v1","kind":"Pod","metadata":{"uid":"uid"},"spec":{"containers":[{"name":"web","image":"web"}]}}"#.to_vec())
    });
    let api = Api::<Pod>::namespaced(client, "default");
    let mut request: PodStartRequest = serde_json::from_value(serde_json::json!({
        "clusterId":"cluster", "sessionId":"lease", "namespace":"default", "name":"web", "uid":"uid", "container":"web", "mode":"shell"
    })).unwrap();
    assert_eq!(
        verify_process(&api, &request, &CancellationToken::new()).await,
        Err(ConnectionError::ProcessUnavailable)
    );
    request.mode = PodStreamMode::Logs;
    assert!(
        verify_process(&api, &request, &CancellationToken::new())
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn runtime_identity_fetch_has_a_body_bound() {
    let client = crate::connection::tests::fake(|_| (200, vec![b' '; 1024 * 1024 + 1]));
    let api = Api::<Pod>::namespaced(client, "default");
    assert!(matches!(
        fetch_pod(&api, "web", &CancellationToken::new()).await,
        Err(ConnectionError::LimitExceeded)
    ));
}

fn start_request(scope: &SessionRequest) -> PodStartRequest {
    PodStartRequest {
        cluster_id: scope.cluster_id.clone(),
        session_id: scope.session_id.clone(),
        namespace: "default".into(),
        name: "web".into(),
        uid: "uid".into(),
        container: "web".into(),
        mode: PodStreamMode::Logs,
        previous: false,
        timestamps: true,
        tail_lines: 500,
        shell: None,
    }
}
fn forward_request(scope: &SessionRequest) -> PodForwardRequest {
    PodForwardRequest {
        cluster_id: scope.cluster_id.clone(),
        session_id: scope.session_id.clone(),
        namespace: "default".into(),
        name: "web".into(),
        uid: "uid".into(),
        remote_port: 80,
        local_port: 0,
    }
}
async fn runtime_engine(
    client: kube::Client,
) -> (tempfile::TempDir, ConnectionEngine, SessionRequest) {
    let (root, _, engine, id, _) = crate::connection::tests::fixture();
    let opened = engine
        .open(super::super::OpenRequest { cluster_id: id })
        .await
        .unwrap();
    engine.session.lock().unwrap().as_mut().unwrap().client =
        Some(crate::connection::tests::allow_permission_reviews(client));
    (root, engine, crate::connection::tests::identity(&opened))
}
fn pod_body() -> Vec<u8> {
    br#"{"apiVersion":"v1","kind":"Pod","metadata":{"uid":"uid"},"spec":{"containers":[{"name":"web"}]}}"#.to_vec()
}

#[tokio::test]
async fn lease_cleanup_closes_streams_and_forwards_without_usable_client() {
    for expired in [true, false] {
        let client = crate::connection::tests::fake(|_| (200, pod_body()));
        let (_root, engine, scope) = runtime_engine(client).await;
        let forward = engine
            .pod_forward_start(forward_request(&scope))
            .await
            .unwrap();
        let cancel = CancellationToken::new();
        let mut stream = state(cancel.clone());
        stream.scope = scope.clone();
        stream.ring.push("private output");
        let (input, mut input_rx) = mpsc::channel(1);
        let (resize, mut resize_rx) = mpsc::channel(1);
        stream.input = Some(input);
        stream.resize = Some(resize);
        let slot = Arc::new(Mutex::new(stream));
        engine
            .pod_runtime
            .registry
            .lock()
            .unwrap()
            .streams
            .insert("stream".into(), slot.clone());

        engine.session.lock().unwrap().as_mut().unwrap().client = expired.then(|| {
            kube::client::ClientBuilder::new(
                tower::service_fn(|_: http::Request<kube::client::Body>| async {
                    Err::<http::Response<kube::client::Body>, _>(std::io::Error::other(
                        "cleanup must not make a Kubernetes request",
                    ))
                }),
                "default",
            )
            .with_valid_until(Some("2000-01-01T00:00:00Z".parse().unwrap()))
            .build()
        });
        assert_eq!(
            engine.check_pod_session(&scope),
            Err(if expired {
                ConnectionError::AuthExpired
            } else {
                ConnectionError::NotConnected
            })
        );
        let mut close = PodStreamRequest {
            cluster_id: scope.cluster_id.clone(),
            session_id: "other-lease".into(),
            stream_id: "stream".into(),
        };
        let mut stop = PodForwardStopRequest {
            cluster_id: scope.cluster_id.clone(),
            session_id: "other-lease".into(),
            id: forward.id.clone(),
        };
        assert_eq!(
            engine.pod_stream_close(close.clone()),
            Err(ConnectionError::StaleSession)
        );
        assert_eq!(
            engine.pod_forward_stop(PodForwardStopRequest {
                cluster_id: stop.cluster_id.clone(),
                session_id: stop.session_id.clone(),
                id: stop.id.clone(),
            }),
            Err(ConnectionError::StaleSession)
        );
        assert!(!cancel.is_cancelled());
        assert_eq!(slot.lock().unwrap().ring.text, "private output");
        assert!(
            TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, forward.local_port))
                .await
                .is_err()
        );

        close.session_id = scope.session_id.clone();
        stop.session_id = scope.session_id.clone();
        engine.pod_stream_close(close.clone()).unwrap();
        engine
            .pod_forward_stop(PodForwardStopRequest {
                cluster_id: stop.cluster_id.clone(),
                session_id: stop.session_id.clone(),
                id: stop.id.clone(),
            })
            .unwrap();
        engine.pod_stream_close(close).unwrap();
        engine.pod_forward_stop(stop).unwrap();
        assert!(cancel.is_cancelled());
        {
            let stream = slot.lock().unwrap();
            assert!(stream.ring.text.is_empty());
            assert!(stream.input.is_none());
            assert!(stream.resize.is_none());
        }
        assert!(input_rx.recv().await.is_none());
        assert!(resize_rx.recv().await.is_none());
        tokio::time::timeout(Duration::from_secs(1), async {
            while !engine.pod_runtime.tasks.is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            engine.pod_runtime.forward_slots.available_permits(),
            FORWARD_LIMIT
        );
        {
            let registry = engine.pod_runtime.registry.lock().unwrap();
            assert!(registry.streams.is_empty());
            assert!(registry.forwards.is_empty());
        }
        TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, forward.local_port))
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn stalled_startups_use_one_worker_each_and_reject_before_network() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let client = kube::Client::new(
        super::super::transport::CancelService(tower::service_fn(
            move |_: http::Request<kube::client::Body>| {
                observed.fetch_add(1, Ordering::SeqCst);
                std::future::pending::<
                    std::result::Result<http::Response<kube::client::Body>, std::io::Error>,
                >()
            },
        )),
        "default",
    );
    let (_root, engine, scope) = runtime_engine(client).await;
    let engine = Arc::new(engine);
    let mut starts = Vec::new();
    for index in 0..4 {
        let engine = engine.clone();
        let scope = scope.clone();
        starts.push(tokio::spawn(async move {
            if index % 2 == 0 {
                engine
                    .pod_stream_open(start_request(&scope))
                    .await
                    .map(|_| ())
            } else {
                engine
                    .pod_forward_start(forward_request(&scope))
                    .await
                    .map(|_| ())
            }
        }));
    }
    tokio::time::timeout(Duration::from_secs(1), async {
        while calls.load(Ordering::SeqCst) != 4 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        engine.pod_stream_open(start_request(&scope)).await,
        Err(ConnectionError::Busy)
    ));
    assert!(matches!(
        engine.pod_forward_start(forward_request(&scope)).await,
        Err(ConnectionError::Busy)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    engine.disconnect(scope).unwrap();
    for start in starts {
        assert_eq!(start.await.unwrap(), Err(ConnectionError::StaleSession));
    }
    assert_eq!(engine.workers.available_permits(), 4);
    assert_eq!(
        engine.pod_runtime.stream_slots.available_permits(),
        STREAM_LIMIT
    );
    assert_eq!(
        engine.pod_runtime.forward_slots.available_permits(),
        FORWARD_LIMIT
    );
}

#[tokio::test]
async fn opening_stream_slots_outlive_registry_removal_and_release_on_root_cancel() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let client = kube::Client::new(
        super::super::transport::CancelService(tower::service_fn(
            move |request: http::Request<kube::client::Body>| {
                observed.fetch_add(1, Ordering::SeqCst);
                let logs = request.uri().path().ends_with("/log");
                async move {
                    if logs {
                        std::future::pending::<()>().await;
                    }
                    Ok::<_, std::io::Error>(http::Response::new(kube::client::Body::from(
                        pod_body(),
                    )))
                }
            },
        )),
        "default",
    );
    let (_root, engine, scope) = runtime_engine(client).await;
    let mut streams = Vec::new();
    for _ in 0..STREAM_LIMIT {
        streams.push(engine.pod_stream_open(start_request(&scope)).await.unwrap());
    }
    let before = calls.load(Ordering::SeqCst);
    assert!(matches!(
        engine.pod_stream_open(start_request(&scope)).await,
        Err(ConnectionError::LimitExceeded)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), before);
    engine
        .pod_stream_close(PodStreamRequest {
            cluster_id: scope.cluster_id.clone(),
            session_id: scope.session_id.clone(),
            stream_id: streams[0].stream_id.clone(),
        })
        .unwrap();
    assert_eq!(
        engine.pod_runtime.registry.lock().unwrap().streams.len(),
        STREAM_LIMIT - 1
    );
    assert_eq!(engine.pod_runtime.stream_slots.available_permits(), 0);
    assert!(matches!(
        engine.pod_stream_open(start_request(&scope)).await,
        Err(ConnectionError::LimitExceeded)
    ));
    engine.disconnect(scope).unwrap();
    tokio::time::timeout(Duration::from_secs(1), engine.pod_runtime.drain())
        .await
        .unwrap();
    assert_eq!(
        engine.pod_runtime.stream_slots.available_permits(),
        STREAM_LIMIT
    );
}

#[tokio::test]
async fn forward_slots_bound_listeners_and_retiring_tasks_before_fetch_or_bind() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let client = crate::connection::tests::fake(move |_| {
        observed.fetch_add(1, Ordering::SeqCst);
        (200, pod_body())
    });
    let (_root, engine, scope) = runtime_engine(client).await;
    let mut forwards = Vec::new();
    for _ in 0..FORWARD_LIMIT {
        forwards.push(
            engine
                .pod_forward_start(forward_request(&scope))
                .await
                .unwrap(),
        );
    }
    assert_eq!(engine.pod_runtime.forward_slots.available_permits(), 0);
    let before = calls.load(Ordering::SeqCst);
    assert!(matches!(
        engine.pod_forward_start(forward_request(&scope)).await,
        Err(ConnectionError::LimitExceeded)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), before);
    engine
        .pod_forward_stop(PodForwardStopRequest {
            cluster_id: scope.cluster_id.clone(),
            session_id: scope.session_id.clone(),
            id: forwards[0].id.clone(),
        })
        .unwrap();
    assert_eq!(engine.pod_runtime.forward_slots.available_permits(), 0);
    assert!(matches!(
        engine.pod_forward_start(forward_request(&scope)).await,
        Err(ConnectionError::LimitExceeded)
    ));
    engine.disconnect(scope).unwrap();
    tokio::time::timeout(Duration::from_secs(1), engine.pod_runtime.drain())
        .await
        .unwrap();
    assert_eq!(
        engine.pod_runtime.forward_slots.available_permits(),
        FORWARD_LIMIT
    );
    for forward in forwards {
        TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, forward.local_port))
            .await
            .unwrap();
    }
}

#[test]
fn finished_stream_output_remains_readable_in_bounded_chunks() {
    let mut state = state(CancellationToken::new());
    state.ring.push(&"x".repeat(DRAIN_LIMIT + 17));
    state.finish(Ok(()));
    assert_eq!(state.snapshot().output.len(), DRAIN_LIMIT);
    let last = state.snapshot();
    assert_eq!(last.output.len(), 17);
    assert_eq!(last.status, "exited");
}

#[tokio::test]
async fn logs_revalidate_uid_after_open_before_exposing_bytes() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let opened = Arc::new(AtomicBool::new(false));
    let observed = opened.clone();
    let client = crate::connection::tests::fake(move |path| {
        if path.contains("/log?") {
            observed.store(true, Ordering::SeqCst);
            (200, b"replacement pod output".to_vec())
        } else {
            assert!(observed.load(Ordering::SeqCst));
            (200, br#"{"apiVersion":"v1","kind":"Pod","metadata":{"uid":"replacement"},"spec":{"containers":[{"name":"web"}]}}"#.to_vec())
        }
    });
    let api = Api::<Pod>::namespaced(client, "default");
    let state = Arc::new(Mutex::new(state(CancellationToken::new())));
    assert_eq!(
        run_logs(
            &api,
            &start_request(&scope()),
            &state,
            &CancellationToken::new()
        )
        .await,
        Err(ConnectionError::SourceChanged)
    );
    assert!(opened.load(Ordering::SeqCst));
    assert!(state.lock().unwrap().ring.text.is_empty());
}
