use super::*;

fn request() -> HelmInstallRequest {
    HelmInstallRequest {
        cluster_id: "cluster".into(),
        session_id: "session".into(),
        read_id: 1,
        namespace: "default".into(),
        release_name: "my-release".into(),
        chart: "stable/nginx".into(),
        version: Some("1.2.3".into()),
        values_yaml: "password: private\n".into(),
        dry_run: true,
    }
}
#[test]
fn input_boundary() {
    assert_eq!(validate(&request()), Ok(()));
    for chart in [
        "./chart",
        "/tmp/chart",
        "../chart",
        "repo/../../chart",
        "https://host/chart",
        "oci://host/chart",
        "repo/-flag",
        "repo/name/extra",
    ] {
        let mut r = request();
        r.chart = chart.into();
        assert_eq!(validate(&r), Err(ConnectionError::InvalidInput));
    }
    for yaml in [
        "",
        "null",
        "[]",
        "secret",
        "a: b\n---\nc: d",
        "!tag {}",
        "a: [",
    ] {
        let mut r = request();
        r.values_yaml = yaml.into();
        assert_eq!(
            validate(&r),
            Err(ConnectionError::InvalidInput),
            "values YAML: {yaml:?}"
        );
    }
    let mut r = request();
    r.values_yaml = "x".repeat(VALUES_LIMIT + 1);
    assert!(validate(&r).is_err());
    for value in ["--flag", "bad/name", "UPPER", "bad-", ""] {
        let mut r = request();
        r.release_name = value.into();
        assert!(validate(&r).is_err());
    }
    let mut r = request();
    r.release_name = "a".repeat(54);
    assert!(validate(&r).is_err());
    for v in ["--flag", "1\n2", "", "1\u{0}2"] {
        let mut r = request();
        r.version = Some(v.into());
        assert!(validate(&r).is_err());
    }
}
#[test]
fn values_require_an_untagged_mapping_document() {
    for yaml in ["{}", "---\nreplicas: 2\n", "replicas: 2\n..."] {
        let mut r = request();
        r.values_yaml = yaml.into();
        assert_eq!(validate(&r), Ok(()), "values YAML: {yaml:?}");
    }
    for yaml in ["!tag {}", "!tag\nreplicas: 2", "!values {replicas: 2}"] {
        let mut r = request();
        r.values_yaml = yaml.into();
        assert_eq!(
            validate(&r),
            Err(ConnectionError::InvalidInput),
            "values YAML: {yaml:?}"
        );
    }
}
#[test]
fn exact_id_cancellation_and_tombstones() {
    let runtime = InstallRuntime::default();
    let root = CancellationToken::new();
    let key = request().key();
    runtime.register(key.clone(), &root, true).unwrap();
    assert!(matches!(
        runtime.register(key, &root, false),
        Err(ConnectionError::StaleSession)
    ));
    let mut r = request();
    r.read_id = 2;
    let active = runtime.register(r.key(), &root, false).unwrap();
    runtime.register(request().key(), &root, true).unwrap();
    assert!(!active.is_cancelled());
    runtime.finish(&r.key());
    assert!(active.is_cancelled());
    assert!(runtime.register(r.key(), &root, false).is_err());
}
#[test]
fn owned_runtime_drop_and_disconnect_cancel() {
    let runtime = InstallRuntime::default();
    let root = CancellationToken::new();
    let token = runtime.register(request().key(), &root, false).unwrap();
    root.cancel();
    assert!(token.is_cancelled());
    let root = CancellationToken::new();
    let token = runtime.register(request().key(), &root, false).unwrap();
    drop(runtime);
    assert!(token.is_cancelled());
}
#[test]
fn dry_run_flags_and_version_gate() {
    let mut r = request();
    let args = install_arguments(&r, std::path::Path::new("/private/values"));
    assert!(args.contains(&"--dry-run=server".into()));
    assert!(args.contains(&"--hide-secret".into()));
    assert!(!format!("{args:?}").contains("password"));
    r.dry_run = false;
    let args = install_arguments(&r, std::path::Path::new("/private/values"));
    assert!(args.contains(&"--wait".into()));
    assert!(!args.contains(&"--dry-run=server".into()));
    for version in [b"v3.17.0".as_slice(), b"v4.0.0", b"v3.22.1+build"] {
        assert!(supported_version(version));
    }
    for version in [b"v3.14.0".as_slice(), b"v2.17.0", b"v5.0.0", b"garbage"] {
        assert!(!supported_version(version));
    }
}
#[cfg(unix)]
#[tokio::test]
async fn synthetic_install_discards_secrets_and_cleans_files() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().to_owned();
    let file = tempfile::NamedTempFile::new_in(&path).unwrap();
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        file.as_file().metadata().unwrap().permissions().mode() & 0o777,
        0o600
    );
    let result = supervise_owned(
        "/bin/sh".into(),
        vec!["-c".into(), "printf secret; printf private >&2".into()],
        file,
        CancellationToken::new(),
        Duration::from_secs(2),
        Some(directory),
    )
    .await;
    assert_eq!(result, Ok(vec![]));
    assert!(!path.exists());
}
#[cfg(unix)]
#[tokio::test]
async fn synthetic_install_cancel_timeout_and_output_limit() {
    for mode in 0..4 {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().to_owned();
        let marker = path.join("spawned");
        let file = tempfile::NamedTempFile::new_in(&path).unwrap();
        let cancel = CancellationToken::new();
        if mode == 0 {
            cancel.cancel();
        }
        let script = match mode {
            0 => format!("touch '{}'", marker.display()),
            1 => "exec /bin/sleep 30".into(),
            2 => "exec /usr/bin/yes aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            _ => "echo private >&2; exit 1".into(),
        };
        let result = supervise_owned(
            "/bin/sh".into(),
            vec!["-c".into(), script.into()],
            file,
            cancel,
            if mode == 1 {
                Duration::from_millis(100)
            } else {
                Duration::from_secs(10)
            },
            Some(directory),
        )
        .await;
        assert_eq!(
            result,
            Err(match mode {
                0 => ConnectionError::StaleSession,
                1 => ConnectionError::Timeout,
                2 => ConnectionError::LimitExceeded,
                _ => ConnectionError::Conflict,
            })
        );
        assert!(!path.exists());
    }
}
