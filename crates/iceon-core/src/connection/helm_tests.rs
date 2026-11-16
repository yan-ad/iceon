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
        [
            "list",
            "--deployed",
            "--failed",
            "--pending",
            "--uninstalled",
            "--uninstalling",
            "--superseded",
            "--all-namespaces",
            "--output",
            "json",
        ]
        .map(OsString::from)
    );
    assert_eq!(
        arguments(HelmView::Releases, Some("dev")),
        [
            "list",
            "--deployed",
            "--failed",
            "--pending",
            "--uninstalled",
            "--uninstalling",
            "--superseded",
            "--namespace",
            "dev",
            "--output",
            "json",
        ]
        .map(OsString::from)
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

#[test]
fn repository_validation_rejects_flags_credentials_and_non_http_urls() {
    for (name, url) in [
        ("--flag", "https://example.com"),
        ("bad/name", "https://example.com"),
        ("ok", "file:///tmp/index.yaml"),
        ("ok", "https://user:password@example.com"),
        ("ok", "https://example.com?token=secret"),
        ("ok", "https://example.com/#fragment"),
    ] {
        assert!(
            validate_repository(&HelmRepository {
                name: name.into(),
                url: url.into()
            })
            .is_err()
        );
    }
    assert!(
        validate_repository(&HelmRepository {
            name: "stable".into(),
            url: "https://charts.example.com/releases".into()
        })
        .is_ok()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn fake_binary_repository_commands_receive_preferences_without_global_env_changes() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let binary = directory.path().join("custom-helm");
    let log = directory.path().join("commands");
    std::fs::write(
        &binary,
        format!(
            r#"#!/bin/sh
printf '%s\n' "$@" >> '{}'
printf 'proxy=%s\ninsecure-kube=%s\n' "$HTTPS_PROXY" "$HELM_KUBEINSECURE_SKIP_TLS_VERIFY" >> '{}'
case "$2" in list) printf '[{{"name":"stable","url":"https://charts.example.com"}}]';; esac
"#,
            log.display(),
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let preferences = HelmPreferences {
        binary_path: Some(binary.clone()),
        proxy: Some("http://proxy.example.com:8080".into()),
        allow_untrusted_cas: true,
        ..Default::default()
    };
    assert_eq!(
        executable_with_preferences(&preferences).unwrap(),
        std::fs::canonicalize(&binary).unwrap()
    );
    let engine = ConnectionEngine::new(std::sync::Arc::new(crate::catalog::CatalogEngine::new(
        directory.path().join("catalog"),
    )));
    let repository = HelmRepository {
        name: "stable".into(),
        url: "https://charts.example.com".into(),
    };
    let cancel = CancellationToken::new();
    engine
        .helm_repository_add(repository.clone(), preferences.clone(), cancel.clone())
        .await
        .unwrap();
    assert_eq!(
        engine
            .helm_repository_list(preferences.clone(), cancel.clone())
            .await
            .unwrap(),
        vec![repository]
    );
    engine
        .helm_repository_remove("stable".into(), preferences, cancel)
        .await
        .unwrap();
    let text = std::fs::read_to_string(log).unwrap();
    assert!(
        text.contains(
            "repo\nadd\nstable\nhttps://charts.example.com\n--insecure-skip-tls-verify\n"
        )
    );
    assert!(text.contains("repo\nlist\n--output\njson\n"));
    assert!(text.contains("repo\nremove\nstable\n"));
    assert!(text.contains("proxy=http://proxy.example.com:8080\ninsecure-kube=\n"));
    assert!(!text.contains("--kube-insecure"));
}

#[cfg(unix)]
#[tokio::test]
async fn process_proxy_bypasses_the_exact_kubernetes_host() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(b"clusters:\n- cluster:\n    server: https://kubernetes.example.com:6443\n")
        .unwrap();
    let result = supervise_configured("/bin/sh".into(), vec!["-c".into(), "printf '%s|%s|%s' \"$HTTPS_PROXY\" \"$NO_PROXY\" \"$HELM_KUBEINSECURE_SKIP_TLS_VERIFY\"".into()], file, CancellationToken::new(), std::time::Duration::from_secs(2), None, HelmPreferences { proxy: Some("http://proxy.example.com:8080".into()), ..Default::default() }).await.unwrap();
    assert_eq!(
        result,
        b"http://proxy.example.com:8080|kubernetes.example.com|"
    );
}

#[test]
fn persistent_preferences_are_converted_without_losing_runtime_options() {
    let mut stored = crate::preferences::Preferences::default();
    stored.kubernetes.helm_binary_path = "/opt/helm custom".into();
    stored.kubernetes.helm_server_side = true;
    stored.proxy.https_proxy = "https://proxy.example.com:443".into();
    stored.proxy.allow_untrusted_cas = true;
    let options = HelmPreferences::from(&stored);
    assert_eq!(options.binary_path, Some("/opt/helm custom".into()));
    assert!(options.server_side && options.allow_untrusted_cas);
    assert_eq!(
        options.proxy.as_deref(),
        Some("https://proxy.example.com:443")
    );
    assert!(validate_proxy(options.proxy.as_deref()).is_ok());
    assert_eq!(
        executable_with_preferences(&HelmPreferences {
            binary_path: Some("relative/helm".into()),
            ..Default::default()
        }),
        Err(ConnectionError::InvalidInput)
    );
}

#[cfg(unix)]
#[tokio::test]
async fn helm3_and_helm4_release_fixtures_accept_shared_exact_arguments() {
    use std::os::unix::fs::PermissionsExt;
    for major in [3, 4] {
        let directory = tempfile::tempdir().unwrap();
        let binary = directory.path().join(format!("helm{major}"));
        let calls = directory.path().join("calls");
        let argv = directory.path().join("argv");
        let fixture = directory.path().join("releases.json");
        let statuses = [
            "deployed",
            "failed",
            "pending-install",
            "pending-upgrade",
            "pending-rollback",
            "uninstalled",
            "uninstalling",
            "superseded",
        ];
        let rows: Vec<_> = statuses
            .iter()
            .enumerate()
            .map(|(index, status)| {
                json!({
                    "name": format!("release-{index}"), "namespace": "dev", "revision": "2",
                    "updated": "2026-11-16 10:00:00 +0000 UTC", "status": status,
                    "chart": "example-1.2.3", "app_version": "4.5.6"
                })
            })
            .collect();
        std::fs::write(&fixture, serde_json::to_vec(&rows).unwrap()).unwrap();
        std::fs::write(
            &binary,
            format!(
                r#"#!/bin/sh
printf 'call\n' >> '{}'
printf '%s\n' "$@" > '{}'
for arg do
    if [ "$arg" = '--all' ] && [ "{major}" = 4 ]; then
        printf 'Error: unknown flag: --all\n' >&2
        exit 1
    fi
done
/bin/cat '{}'
"#,
                calls.display(),
                argv.display(),
                fixture.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        for namespace in [None, Some("dev")] {
            let config = tempfile::NamedTempFile::new().unwrap();
            let config_path = config.path().to_owned();
            let output = supervise(
                binary.clone(),
                arguments(HelmView::Releases, namespace),
                config,
                CancellationToken::new(),
            )
            .await
            .unwrap();
            assert!(!config_path.exists());
            let scope = namespace.map_or("--all-namespaces\n".to_owned(), |ns| {
                format!("--namespace\n{ns}\n")
            });
            assert_eq!(
                std::fs::read_to_string(&argv).unwrap(),
                format!(
                    "list\n--deployed\n--failed\n--pending\n--uninstalled\n--uninstalling\n--superseded\n{scope}--output\njson\n"
                )
            );
            let mut request = request(HelmView::Releases);
            request.namespace = namespace.map(str::to_owned);
            let page = project(&request, &output).unwrap();
            assert_eq!(page.items.len(), statuses.len());
            for (item, status) in page.items.iter().zip(statuses) {
                assert_eq!(item.namespace.as_deref(), Some("dev"));
                assert_eq!(item.version, "example-1.2.3");
                assert_eq!(item.app_version, "4.5.6");
                assert_eq!(item.revision, Some(2));
                assert_eq!(item.status.as_deref(), Some(status));
                assert_eq!(
                    item.updated.as_deref(),
                    Some("2026-11-16 10:00:00 +0000 UTC")
                );
            }
        }
        // Helm 4's legacy-flag failure is sanitized and is never retried.
        if major == 4 {
            assert_eq!(
                supervise(
                    binary,
                    vec!["list".into(), "--all".into()],
                    tempfile::NamedTempFile::new().unwrap(),
                    CancellationToken::new()
                )
                .await,
                Err(ConnectionError::HelmCommandFailed)
            );
        }
        assert_eq!(
            std::fs::read_to_string(calls).unwrap().lines().count(),
            if major == 4 { 3 } else { 2 }
        );
    }
}

#[test]
fn release_json_contract_requires_an_array_including_empty_results() {
    let request = request(HelmView::Releases);
    assert!(project(&request, b"[]").unwrap().items.is_empty());
    for invalid in [b"{}".as_slice(), b"null", b"", b"{\"releases\":[]}"] {
        assert_eq!(
            project(&request, invalid),
            Err(ConnectionError::InvalidResponse)
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn nonzero_cli_diagnostics_are_classified_without_exposing_text() {
    for (diagnostic, expected) in [
        (
            "Error: unknown flag: --all",
            ConnectionError::HelmCommandFailed,
        ),
        (
            "Error: unknown shorthand flag: 'a' in -a",
            ConnectionError::HelmCommandFailed,
        ),
        (
            "Error: arbitrary command failure",
            ConnectionError::HelmCommandFailed,
        ),
        ("", ConnectionError::HelmCommandFailed),
        (
            "Error: Kubernetes cluster unreachable: x509: certificate signed by unknown authority",
            ConnectionError::TlsFailed,
        ),
        (
            "Error: Kubernetes cluster unreachable: the server has asked for the client to provide credentials",
            ConnectionError::AuthFailed,
        ),
        (
            "Error: Kubernetes cluster unreachable: context deadline exceeded",
            ConnectionError::Timeout,
        ),
        (
            "Error: Kubernetes cluster unreachable: connection refused",
            ConnectionError::ConnectionFailed,
        ),
        (
            "Error: Error from server (Forbidden): access denied",
            ConnectionError::PermissionDenied,
        ),
        (
            "Error: Error from server (Unauthorized): access denied",
            ConnectionError::AuthFailed,
        ),
        (
            "Error: Error from server (Conflict): object modified",
            ConnectionError::Conflict,
        ),
        (
            "warning containing conflict Forbidden Unauthorized x509:",
            ConnectionError::HelmCommandFailed,
        ),
    ] {
        // stderr is passed as a positional argument, not interpolated as shell code.
        let result = supervise(
            "/bin/sh".into(),
            vec![
                "-c".into(),
                "printf '%s\\n' \"$1\" >&2; exit 1".into(),
                "fixture".into(),
                diagnostic.into(),
            ],
            tempfile::NamedTempFile::new().unwrap(),
            CancellationToken::new(),
        )
        .await;
        assert_eq!(result, Err(expected));
        assert_eq!(
            serde_json::to_value(expected).unwrap().as_str().unwrap(),
            match expected {
                ConnectionError::HelmCommandFailed => "helmCommandFailed",
                ConnectionError::TlsFailed => "tlsFailed",
                ConnectionError::AuthFailed => "authFailed",
                ConnectionError::Timeout => "timeout",
                ConnectionError::ConnectionFailed => "connectionFailed",
                ConnectionError::PermissionDenied => "permissionDenied",
                ConnectionError::Conflict => "conflict",
                _ => unreachable!(),
            }
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn stderr_is_bounded_even_on_failure_and_never_replaces_success_output() {
    assert_eq!(
        supervise(
            "/bin/sh".into(),
            vec![
                "-c".into(),
                "printf 'Error: unknown flag: --all\\n' >&2; printf '[]'".into()
            ],
            tempfile::NamedTempFile::new().unwrap(),
            CancellationToken::new(),
        )
        .await,
        Ok(b"[]".to_vec())
    );
    assert_eq!(
        supervise(
            "/bin/sh".into(),
            vec!["-c".into(), "exec /usr/bin/yes diagnostic >&2".into()],
            tempfile::NamedTempFile::new().unwrap(),
            CancellationToken::new(),
        )
        .await,
        Err(ConnectionError::LimitExceeded)
    );
}

#[cfg(unix)]
#[tokio::test]
async fn empty_repositories_are_scoped_exact_read_results_not_mutation_success() {
    use std::os::unix::fs::PermissionsExt;
    for major in [3, 4] {
        for (command, subcommand, diagnostic, code, empty) in [
            (
                "search",
                "repo",
                "Error: no repositories configured\n",
                1,
                true,
            ),
            ("repo", "list", "Error: no repositories to show\n", 1, true),
            (
                "repo",
                "remove",
                "Error: no repositories configured\n",
                1,
                false,
            ),
            (
                "install",
                "app",
                "Error: no repositories configured\n",
                1,
                false,
            ),
            (
                "search",
                "repo",
                "Error: no repositories configured: private\n",
                1,
                false,
            ),
            (
                "search",
                "repo",
                "private\nError: no repositories configured\n",
                1,
                false,
            ),
            (
                "search",
                "repo",
                "Error: no repositories configured\n",
                2,
                false,
            ),
            ("search", "repo", "Error: unknown flag: --all\n", 1, false),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let binary = directory.path().join(format!("helm-{major}"));
            std::fs::write(
                &binary,
                format!("#!/bin/sh\nprintf '%s' '{diagnostic}' >&2\nexit {code}\n"),
            )
            .unwrap();
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
            let result = supervise(
                binary,
                vec![command.into(), subcommand.into()],
                tempfile::NamedTempFile::new().unwrap(),
                CancellationToken::new(),
            )
            .await;
            assert_eq!(
                result,
                if empty {
                    Ok(b"[]".to_vec())
                } else {
                    Err(ConnectionError::HelmCommandFailed)
                }
            );
        }
    }
}

#[cfg(unix)]
#[tokio::test]
async fn helm3_and_helm4_chart_and_history_command_contracts() {
    use std::os::unix::fs::PermissionsExt;
    for major in [3, 4] {
        let directory = tempfile::tempdir().unwrap();
        let binary = directory.path().join(format!("helm-{major}"));
        std::fs::write(
            &binary,
            r#"#!/bin/sh
case "$*" in
  'search repo -o json') printf '[]';;
  'show chart stable/nginx --version 1.2.3') printf 'name: nginx\nversion: 1.2.3\n';;
  'history app --namespace dev --max 100 --output json') printf '[]';;
  *) printf 'Error: unsupported fixture arguments\n' >&2; exit 1;;
esac
"#,
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let chart = HelmDetailRequest {
            cluster_id: "one".into(),
            session_id: "lease-1".into(),
            read_id: 1,
            view: HelmView::Charts,
            name: "stable/nginx".into(),
            namespace: None,
            version: Some("1.2.3".into()),
        };
        let history = HelmDetailRequest {
            view: HelmView::Releases,
            name: "app".into(),
            namespace: Some("dev".into()),
            version: None,
            ..chart.clone()
        };
        for (args, detail) in [
            (arguments(HelmView::Charts, None), None),
            (detail_arguments(&chart), Some(&chart)),
            (detail_arguments(&history), Some(&history)),
        ] {
            let output = supervise(
                binary.clone(),
                args,
                tempfile::NamedTempFile::new().unwrap(),
                CancellationToken::new(),
            )
            .await
            .unwrap();
            if let Some(detail) = detail {
                project_detail(detail, &output).unwrap();
            } else {
                assert_eq!(output, b"[]");
            }
        }
    }
}
