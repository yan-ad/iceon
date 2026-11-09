use super::*;
use serde_json::json;
use std::ffi::OsString;

#[test]
fn executable_lookup_rejects_relative_path_entries() {
    assert_eq!(
        lookup_executable(OsString::from("relative").as_os_str()),
        Err(ConnectionError::HelmUnavailable)
    );
}

fn cancellation(request: &HelmRequest) -> HelmCancelRequest {
    HelmCancelRequest {
        cluster_id: request.cluster_id.clone(),
        session_id: request.session_id.clone(),
        view: request.view,
        read_id: request.read_id,
    }
}

#[test]
fn cancellation_before_registration_prevents_late_open() {
    let runtime = HelmRuntime::default();
    let root = CancellationToken::new();
    let mut request = request(HelmView::Charts);
    runtime.cancel(&cancellation(&request), &root).unwrap();
    assert!(matches!(
        runtime.token(&request, &root),
        Err(ConnectionError::StaleSession)
    ));
    request.read_id += 1;
    assert!(!runtime.token(&request, &root).unwrap().is_cancelled());
}

#[test]
fn late_old_disposal_and_completion_do_not_cancel_replacement() {
    let runtime = HelmRuntime::default();
    let root = CancellationToken::new();
    let old = request(HelmView::Releases);
    let first = runtime.token(&old, &root).unwrap();
    let mut newer = old.clone();
    newer.read_id = 2;
    let second = runtime.token(&newer, &root).unwrap();
    assert!(first.is_cancelled());
    runtime.cancel(&cancellation(&old), &root).unwrap();
    runtime.finish(&old);
    assert!(!second.is_cancelled());
    root.cancel();
    assert!(second.is_cancelled());
}

#[test]
fn slots_are_bounded_and_canceled_leases_are_discarded() {
    let runtime = HelmRuntime::default();
    let root = CancellationToken::new();
    for i in 0..HelmRuntime::CAP {
        let mut request = request(HelmView::Charts);
        request.session_id = format!("session-{i}");
        runtime.cancel(&cancellation(&request), &root).unwrap();
    }
    assert_eq!(
        runtime.cancel(&cancellation(&request(HelmView::Charts)), &root),
        Err(ConnectionError::Busy)
    );
    root.cancel();
    let replacement = CancellationToken::new();
    runtime
        .token(&request(HelmView::Charts), &replacement)
        .unwrap();
    assert_eq!(runtime.slots.lock().unwrap().len(), 1);
}

#[test]
fn read_id_is_required_safe_and_response_is_scope_bound() {
    assert!(
        serde_json::from_value::<HelmRequest>(
            json!({"clusterId":"c", "sessionId":"s", "view":"charts"})
        )
        .is_err()
    );
    for id in [0, 9_007_199_254_740_992] {
        assert_eq!(validate_read_id(id), Err(ConnectionError::InvalidInput));
    }
    let mut request = request(HelmView::Releases);
    request.namespace = Some("dev".into());
    let page = project(&request, b"[]").unwrap();
    assert_eq!(page.read_id, request.read_id);
    assert_eq!(page.namespace, request.namespace);
}

#[cfg(unix)]
#[tokio::test]
async fn synthetic_process_deadline_reaps_without_helm() {
    use std::time::Duration;
    let directory = tempfile::tempdir().unwrap();
    let pidfile = directory.path().join("pid");
    let script = format!("echo $$ > '{}'; exec /bin/sleep 30", pidfile.display());
    assert_eq!(
        supervise_with_deadline(
            PathBuf::from("/bin/sh"),
            vec!["-c".into(), script.into()],
            tempfile::NamedTempFile::new().unwrap(),
            CancellationToken::new(),
            Duration::from_millis(200)
        )
        .await,
        Err(ConnectionError::Timeout)
    );
    let pid = std::fs::read_to_string(pidfile).unwrap();
    let status = std::process::Command::new("/bin/kill")
        .args(["-0", pid.trim()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success(), "synthetic direct child was not reaped");
}

fn request(view: HelmView) -> HelmRequest {
    HelmRequest {
        cluster_id: "cluster".into(),
        session_id: "session".into(),
        view,
        namespace: None,
        read_id: 1,
    }
}

#[test]
fn requests_reject_unknown_fields_and_invalid_namespaces() {
    assert!(
        serde_json::from_value::<HelmRequest>(
            json!({"clusterId":"c", "sessionId":"s", "view":"charts", "command":"install"})
        )
        .is_err()
    );
    for namespace in ["--all", "dev;exit", "Dev", "", "dev\n"] {
        let mut request = request(HelmView::Releases);
        request.namespace = Some(namespace.into());
        assert_eq!(
            validate_namespace(&request),
            Err(ConnectionError::InvalidInput)
        );
    }
    let mut request = request(HelmView::Charts);
    request.namespace = Some("dev".into());
    assert_eq!(
        validate_namespace(&request),
        Err(ConnectionError::InvalidInput)
    );
}

#[test]
fn releases_are_projected_without_secret_fields() {
    let input = json!([{"name":"app", "namespace":"dev", "chart":"app-1.0", "app_version":"1", "revision":"2", "status":"deployed", "updated":"2026-10-26", "values":{"password":"secret"}, "config":"secret"}]);
    let page = project(
        &request(HelmView::Releases),
        &serde_json::to_vec(&input).unwrap(),
    )
    .unwrap();
    assert_eq!(page.items[0].revision, Some(2));
    let serialized = serde_json::to_string(&page).unwrap();
    assert!(!serialized.contains("secret"));
    assert!(!serialized.contains("values"));
    assert!(!page.truncated);
    let mut selected = request(HelmView::Releases);
    selected.namespace = Some("other".into());
    assert_eq!(
        project(&selected, &serde_json::to_vec(&input).unwrap()),
        Err(ConnectionError::InvalidResponse)
    );
}

#[test]
fn inventory_bounds_and_empty_states_are_honest() {
    assert!(
        project(&request(HelmView::Charts), b"[]")
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(
        project(&request(HelmView::Charts), b"{}"),
        Err(ConnectionError::InvalidResponse)
    );
    let rows = (0..501).map(|i| json!({"name":format!("repo/chart-{i}"), "version":"1", "app_version":"", "description":""})).collect::<Vec<_>>();
    let page = project(
        &request(HelmView::Charts),
        &serde_json::to_vec(&rows).unwrap(),
    )
    .unwrap();
    assert_eq!(page.items.len(), 500);
    assert!(page.truncated);
    let duplicate = vec![rows[0].clone(), rows[0].clone()];
    assert_eq!(
        project(
            &request(HelmView::Charts),
            &serde_json::to_vec(&duplicate).unwrap()
        ),
        Err(ConnectionError::InvalidResponse)
    );
}

#[tokio::test]
async fn fake_output_is_bounded_and_stderr_is_discarded() {
    assert_eq!(
        read_output(&b"sensitive diagnostic"[..], false)
            .await
            .unwrap(),
        Vec::<u8>::new()
    );
    assert_eq!(
        read_output(&vec![b'x'; OUTPUT_LIMIT + 1][..], true).await,
        Err(ConnectionError::LimitExceeded)
    );
}

#[tokio::test]
async fn cancelled_supervisor_never_starts_an_executable() {
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert_eq!(
        supervise(
            PathBuf::from("not-an-executable"),
            Vec::new(),
            tempfile::NamedTempFile::new().unwrap(),
            cancel
        )
        .await,
        Err(ConnectionError::StaleSession)
    );
}

#[test]
fn command_arguments_are_allowlisted() {
    assert_eq!(
        arguments(HelmView::Charts, None),
        ["search", "repo", "-o", "json"].map(OsString::from)
    );
    assert_eq!(
        arguments(HelmView::Releases, None),
        ["list", "--all", "--all-namespaces", "--output", "json"].map(OsString::from)
    );
    assert_eq!(
        arguments(HelmView::Releases, Some("dev")),
        ["list", "--all", "--namespace", "dev", "--output", "json"].map(OsString::from)
    );
}

#[test]
fn project_charts_without_placeholders() {
    let request = HelmRequest {
        cluster_id: "cluster".into(),
        session_id: "session".into(),
        view: HelmView::Charts,
        namespace: None,
        read_id: 1,
    };
    let page = project(&request, serde_json::to_vec(&json!([{"name":"repo/chart","version":"1.2.3","app_version":"4.5","description":"A chart"}])).unwrap().as_slice()).unwrap();
    assert_eq!(page.items[0].name, "repo/chart");
    assert_eq!(page.items[0].description.as_deref(), Some("A chart"));
}

#[test]
fn project_rejects_malformed_release() {
    let request = HelmRequest {
        cluster_id: "cluster".into(),
        session_id: "session".into(),
        view: HelmView::Releases,
        namespace: None,
        read_id: 1,
    };
    assert_eq!(
        project(&request, br#"[{"name":"app"}]"#),
        Err(ConnectionError::InvalidResponse)
    );
}
