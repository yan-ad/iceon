use super::*;
use kube::config::{Context, NamedAuthInfo, NamedCluster, NamedContext, NamedExtension};

fn exec() -> ExecConfig {
    ExecConfig {
        command: Some("synthetic-helper".into()),
        api_version: Some(V1.into()),
        interactive_mode: Some(ExecInteractiveMode::Never),
        ..Default::default()
    }
}
fn response(status: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"apiVersion": V1, "kind": "ExecCredential", "status": status}))
        .unwrap()
}
fn future() -> String {
    Timestamp::from_second(Timestamp::now().as_second() + 3600)
        .unwrap()
        .to_string()
}
fn config(auth: AuthInfo) -> Kubeconfig {
    Kubeconfig {
        current_context: Some("selected".into()),
        contexts: vec![NamedContext {
            name: "selected".into(),
            context: Some(Context {
                cluster: "cluster".into(),
                user: Some("selected".into()),
                ..Default::default()
            }),
            ..Default::default()
        }],
        clusters: vec![NamedCluster {
            name: "cluster".into(),
            cluster: Some(Cluster {
                server: Some("https://synthetic.invalid".into()),
                ..Default::default()
            }),
            ..Default::default()
        }],
        auth_infos: vec![NamedAuthInfo {
            name: "selected".into(),
            auth_info: Some(auth),
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn certificate_response_is_validated_and_encoded() {
    let credentials = parse_response(
        &response(json!({
            "clientCertificateData": CERTIFICATE,
            "clientKeyData": PRIVATE_KEY,
            "expirationTimestamp": future(),
        })),
        V1,
    )
    .unwrap();
    let mut auth = AuthInfo {
        token: Some("old".into()),
        ..Default::default()
    };
    apply(&mut auth, credentials);
    let value = serde_json::to_value(auth).unwrap();
    assert_eq!(
        value["client-certificate-data"],
        STANDARD.encode(CERTIFICATE)
    );
    assert_eq!(value["client-key-data"], STANDARD.encode(PRIVATE_KEY));
    assert!(value.get("token").is_none());
    let malformed = "-----BEGIN CERTIFICATE-----\nYWJj\n-----END CERTIFICATE-----";
    assert_eq!(
        validate_pem(malformed, PRIVATE_KEY),
        Err(ConnectionError::AuthHelperInvalidResponse)
    );
    assert_eq!(
        validate_pem(
            CERTIFICATE,
            "-----BEGIN PRIVATE KEY-----\nYWJj\n-----END PRIVATE KEY-----"
        ),
        Err(ConnectionError::AuthHelperInvalidResponse)
    );
}

#[test]
fn response_contract_and_expiry() {
    let credentials = parse_response(
        &response(json!({"token":"synthetic-token", "expirationTimestamp": future()})),
        V1,
    )
    .unwrap();
    assert_eq!(credentials.token.as_deref(), Some("synthetic-token"));
    assert!(credentials.expiry.is_some());
    assert!(
        parse_response(&response(json!({"token":"synthetic-token"})), V1)
            .unwrap()
            .expiry
            .is_none()
    );
    for status in [
        json!({}),
        json!({"token":""}),
        json!({"token":"bad\r\nheader"}),
        json!({"token":"with space"}),
        json!({"token":"t", "clientCertificateData":"partial"}),
        json!({"clientKeyData":"partial"}),
        json!({"clientCertificateData":"garbage", "clientKeyData":"garbage"}),
    ] {
        assert!(matches!(
            parse_response(&response(status), V1),
            Err(ConnectionError::AuthHelperInvalidResponse)
        ));
    }
    for date in ["2020-01-01T00:00:00Z", "2000-01-01T00:00:00+00:00"] {
        assert_eq!(parse_expiry(date), Err(ConnectionError::AuthExpired));
    }
    let almost = Timestamp::from_second(Timestamp::now().as_second() + 10).unwrap();
    assert_eq!(
        parse_expiry(&almost.to_string()),
        Err(ConnectionError::AuthExpired)
    );
    for date in [
        "2099-01-01",
        "2099-01-01 00:00:00Z",
        "2099-01-01T00:00:00Z[UTC]",
        "2099-01-01T00:00:00",
        "2099-01-01T00:00:00+0000",
    ] {
        assert_eq!(
            parse_expiry(date),
            Err(ConnectionError::AuthHelperInvalidResponse)
        );
    }
    assert!(parse_expiry("2099-01-01T00:00:00.123+05:30").is_ok());
    assert!(matches!(
        parse_response(&response(json!({"token":"t"})), BETA),
        Err(ConnectionError::AuthHelperInvalidResponse)
    ));
    assert!(matches!(
        parse_response(b"not json", V1),
        Err(ConnectionError::AuthHelperInvalidResponse)
    ));
    assert!(matches!(
        parse_response(
            &serde_json::to_vec(&json!({"apiVersion": V1, "kind":"Other", "status":{"token":"t"}}))
                .unwrap(),
            V1
        ),
        Err(ConnectionError::AuthHelperInvalidResponse)
    ));
    assert_eq!(
        validate_token(&"x".repeat(TOKEN_LIMIT + 1)),
        Err(ConnectionError::LimitExceeded)
    );
    assert_eq!(
        validate_pem(&"x".repeat(PEM_LIMIT + 1), "key"),
        Err(ConnectionError::LimitExceeded)
    );
}

#[test]
fn request_is_noninteractive_exact_and_cluster_opt_in() {
    let base = Path::new("/synthetic/config-directory");
    let mut exec = exec();
    exec.command = Some("./tools/helper".into());
    exec.args = Some(vec!["literal $HOME; `id`".into()]);
    let cluster = Cluster {
        server: Some("https://synthetic.invalid".into()),
        certificate_authority_data: Some(STANDARD.encode("synthetic-ca")),
        extensions: Some(vec![NamedExtension {
            name: "client.authentication.k8s.io/exec".into(),
            extension: json!({"audience":"synthetic"}),
        }]),
        ..Default::default()
    };
    let request = helper_request(&exec, &cluster, base).unwrap();
    assert_eq!(request.command, base.join("./tools/helper"));
    assert_eq!(request.args, vec!["literal $HOME; `id`"]);
    let info: Value = serde_json::from_str(&request.info).unwrap();
    assert_eq!(
        info,
        json!({"apiVersion":V1,"kind":"ExecCredential","spec":{"interactive":false}})
    );
    exec.provide_cluster_info = true;
    let request = helper_request(&exec, &cluster, base).unwrap();
    let info: Value = serde_json::from_str(&request.info).unwrap();
    assert_eq!(
        info["spec"]["cluster"]["certificate-authority-data"],
        STANDARD.encode("synthetic-ca")
    );
    assert_eq!(info["spec"]["cluster"]["config"]["audience"], "synthetic");
    exec.interactive_mode = None;
    assert!(matches!(
        helper_request(&exec, &cluster, base),
        Err(ConnectionError::KubeconfigInvalid)
    ));
    exec.api_version = Some(BETA.into());
    assert!(helper_request(&exec, &cluster, base).is_ok());
    exec.interactive_mode = Some(ExecInteractiveMode::Always);
    assert!(matches!(
        helper_request(&exec, &cluster, base),
        Err(ConnectionError::AuthInteractiveRequired)
    ));
    exec.interactive_mode = Some(ExecInteractiveMode::Never);
    exec.env = Some(vec![
        [
            ("name".into(), EXEC_INFO.into()),
            ("value".into(), "untrusted".into()),
        ]
        .into(),
    ]);
    assert!(matches!(
        helper_request(&exec, &cluster, base),
        Err(ConnectionError::KubeconfigInvalid)
    ));
    exec.env = None;
    exec.api_version = Some("unsupported".into());
    assert!(matches!(
        helper_request(&exec, &cluster, base),
        Err(ConnectionError::KubeconfigInvalid)
    ));
}

#[test]
fn provider_cache_only_and_static_conflicts_removed() {
    let provider = |name: &str, entries: &[(&str, String)]| AuthProviderConfig {
        name: name.into(),
        config: entries
            .iter()
            .map(|(name, value)| ((*name).into(), value.clone()))
            .collect(),
        ..Default::default()
    };
    let gcp = provider(
        "gcp",
        &[
            ("access-token", "cached".into()),
            ("expiry", future()),
            ("cmd-path", "/must/not/run".into()),
        ],
    );
    let credentials = cached_provider(&gcp).unwrap();
    let mut auth = AuthInfo {
        token: Some("conflicting".into()),
        username: Some("old".into()),
        password: Some("old".into()),
        client_certificate_data: Some("old".into()),
        client_key_data: Some("old".into()),
        token_file: Some("old".into()),
        exec: Some(exec()),
        auth_provider: Some(gcp),
        ..Default::default()
    };
    apply(&mut auth, credentials);
    let serialized = serde_json::to_value(&auth).unwrap();
    assert_eq!(serialized, json!({"token":"cached"}));
    assert!(matches!(
        cached_provider(&provider("gcp", &[("cmd-path", "/must/not/run".into())])),
        Err(ConnectionError::AuthFailed)
    ));
    let azure = provider(
        "azure",
        &[
            ("access-token", "cached".into()),
            (
                "expires-on",
                (Timestamp::now().as_second() + 3600).to_string(),
            ),
        ],
    );
    assert!(cached_provider(&azure).is_ok());
    let jwt = format!(
        "e30.{}.signature",
        URL_SAFE_NO_PAD.encode(format!(
            "{{\"exp\":{}}}",
            Timestamp::now().as_second() + 3600
        ))
    );
    assert!(cached_provider(&provider("oidc", &[("id-token", jwt)])).is_ok());
    assert!(matches!(
        cached_provider(&provider(
            "oidc",
            &[("id-token", "e30.e30.signature".into())]
        )),
        Err(ConnectionError::AuthHelperInvalidResponse)
    ));
    assert!(matches!(
        cached_provider(&provider(
            "azure",
            &[
                ("access-token", "cached".into()),
                ("expires-on", "1".into())
            ]
        )),
        Err(ConnectionError::AuthExpired)
    ));
    assert!(matches!(
        cached_provider(&provider("unknown", &[])),
        Err(ConnectionError::UnsupportedAuth)
    ));
}

#[tokio::test]
async fn resolves_selected_user_only_and_preserves_on_failure() {
    let provider = AuthProviderConfig {
        name: "azure".into(),
        config: [
            ("access-token".into(), "synthetic".into()),
            (
                "expires-on".into(),
                (Timestamp::now().as_second() + 3600).to_string(),
            ),
        ]
        .into(),
        ..Default::default()
    };
    let mut config = config(AuthInfo {
        auth_provider: Some(provider),
        ..Default::default()
    });
    config.auth_infos.push(NamedAuthInfo {
        name: "unselected".into(),
        auth_info: Some(AuthInfo {
            exec: Some(exec()),
            ..Default::default()
        }),
        ..Default::default()
    });
    assert!(
        resolve(&mut config, Path::new("/synthetic"))
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        config.auth_infos[0]
            .auth_info
            .as_ref()
            .unwrap()
            .auth_provider
            .is_none()
    );
    assert!(
        config.auth_infos[1]
            .auth_info
            .as_ref()
            .unwrap()
            .exec
            .is_some()
    );
    config.auth_infos[0].auth_info.as_mut().unwrap().exec = Some(ExecConfig {
        api_version: Some("unsupported".into()),
        ..exec()
    });
    let before = serde_json::to_value(&config).unwrap();
    assert_eq!(
        resolve(&mut config, Path::new("/synthetic")).await,
        Err(ConnectionError::KubeconfigInvalid)
    );
    assert_eq!(before, serde_json::to_value(config).unwrap());
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt};
    fn script(directory: &Path, body: &str) -> HelperRequest {
        let path = directory.join("helper");
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        HelperRequest {
            command: path,
            args: Vec::new(),
            env: Vec::new(),
            info: "{}".into(),
            fallback_home: None,
        }
    }
    #[tokio::test]
    async fn missing_nonzero_malformed_and_output_caps() {
        let directory = tempfile::tempdir().unwrap();
        let request = HelperRequest {
            command: directory.path().join("missing"),
            fallback_home: None,
            args: vec![],
            env: vec![],
            info: "{}".into(),
        };
        assert_eq!(
            run(request, Duration::from_secs(2)).await,
            Err(ConnectionError::AuthHelperMissing)
        );
        assert_eq!(
            run(
                script(directory.path(), "printf secret >&2; exit 4"),
                Duration::from_secs(2)
            )
            .await,
            Err(ConnectionError::AuthHelperFailed)
        );
        let bytes = run(
            script(directory.path(), "printf 'malformed-secret'"),
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        assert!(matches!(
            parse_response(&bytes, V1),
            Err(ConnectionError::AuthHelperInvalidResponse)
        ));
        for redirection in ["", " >&2"] {
            let request = script(
                directory.path(),
                &format!("head -c {} /dev/zero{redirection}", STDOUT_LIMIT + 1),
            );
            assert_eq!(
                run(request, Duration::from_secs(2)).await,
                Err(ConnectionError::LimitExceeded)
            );
        }
    }
    #[tokio::test]
    async fn exact_args_env_exec_info_and_resolve_token() {
        let directory = tempfile::tempdir().unwrap();
        let request = script(
            directory.path(),
            r#"[ "$1" = 'literal $HOME; `id`' ] || exit 1
[ "$SYNTHETIC_AUTH" = 'literal value' ] || exit 2
[ "$KUBERNETES_EXEC_INFO" = '{"apiVersion":"client.authentication.k8s.io/v1","kind":"ExecCredential","spec":{"interactive":false}}' ] || exit 3
printf '%s' '{"apiVersion":"client.authentication.k8s.io/v1","kind":"ExecCredential","status":{"token":"synthetic-output"}}'"#,
        );
        let mut exec = exec();
        exec.command = Some(request.command.to_str().unwrap().into());
        exec.args = Some(vec!["literal $HOME; `id`".into()]);
        exec.env = Some(vec![
            [
                ("name".into(), "SYNTHETIC_AUTH".into()),
                ("value".into(), "literal value".into()),
            ]
            .into(),
        ]);
        let mut config = config(AuthInfo {
            exec: Some(exec),
            token: Some("must-be-overridden".into()),
            ..Default::default()
        });
        assert_eq!(resolve(&mut config, directory.path()).await, Ok(None));
        assert_eq!(
            serde_json::to_value(config.auth_infos[0].auth_info.as_ref().unwrap()).unwrap(),
            json!({"token":"synthetic-output"})
        );
    }
    async fn pid(path: &Path) -> String {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Ok(value) = fs::read_to_string(path)
                    && !value.trim().is_empty()
                {
                    break value.trim().to_owned();
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap()
    }
    async fn assert_dead(pid: &str) {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let status = tokio::process::Command::new("/bin/ps")
                    .args(["-o", "stat=", "-p", pid])
                    .output()
                    .await
                    .unwrap();
                let state = String::from_utf8_lossy(&status.stdout);
                if !status.status.success()
                    || state.trim().is_empty()
                    || state.trim().starts_with('Z')
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("helper descendant remains alive");
    }
    #[tokio::test]
    async fn timeout_kills_descendant_after_leader_exits_with_pipes() {
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("descendant.pid");
        let request = script(
            directory.path(),
            &format!("sleep 30 &\necho $! > '{}'\nexit 0", pid_file.display()),
        );
        // Allow cold script startup on macOS before timing out the inherited pipes.
        assert_eq!(
            run(request, Duration::from_secs(5)).await,
            Err(ConnectionError::Timeout)
        );
        assert_dead(&pid(&pid_file).await).await;
    }
    #[tokio::test]
    async fn cancellation_after_leader_exit_still_kills_pipe_holders() {
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("descendant.pid");
        let request = script(
            directory.path(),
            &format!("sleep 30 &\necho $! > '{}'\nexit 0", pid_file.display()),
        );
        let handle = tokio::spawn(run(request, Duration::from_secs(10)));
        let child = pid(&pid_file).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        handle.abort();
        let _ = handle.await;
        assert_dead(&child).await;
    }

    #[tokio::test]
    async fn cancellation_kills_tree_and_reaps_direct_child() {
        let directory = tempfile::tempdir().unwrap();
        let child_pid_file = directory.path().join("descendant.pid");
        let parent_pid_file = directory.path().join("parent.pid");
        let request = script(
            directory.path(),
            &format!(
                "echo $$ > '{}'\nsleep 30 &\necho $! > '{}'\nwait",
                parent_pid_file.display(),
                child_pid_file.display()
            ),
        );
        let handle = tokio::spawn(run(request, Duration::from_secs(10)));
        let child = pid(&child_pid_file).await;
        let parent = pid(&parent_pid_file).await;
        handle.abort();
        let _ = handle.await;
        assert_dead(&child).await;
        assert_dead(&parent).await;
    }
    #[tokio::test]
    async fn success_also_removes_background_descendants_without_pipes() {
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("descendant.pid");
        let request = script(
            directory.path(),
            &format!(
                "sleep 30 >/dev/null 2>&1 &\necho $! > '{}'\nprintf '{{}}'\nexit 0",
                pid_file.display()
            ),
        );
        assert_eq!(run(request, Duration::from_secs(2)).await.unwrap(), b"{}");
        assert_dead(&pid(&pid_file).await).await;
    }
}

// Synthetic, untrusted test identity. Never used for cluster connections.
const CERTIFICATE: &str = r#"-----BEGIN CERTIFICATE-----
MIIC8DCCAdigAwIBAgIJAPq8/5l8AwihMA0GCSqGSIb3DQEBCwUAMBwxGjAYBgNV
BAMMEXN5bnRoZXRpYy5pbnZhbGlkMB4XDTI2MTAyMzEyNDI0MloXDTI2MTAyNDEy
NDI0MlowHDEaMBgGA1UEAwwRc3ludGhldGljLmludmFsaWQwggEiMA0GCSqGSIb3
DQEBAQUAA4IBDwAwggEKAoIBAQDsmaXws92V/1HaeoXbvRsQ1m9eVgaNAaTAYxJr
1r4G7NEB2KEHEis7QDlFmGY2ViwmF5UQ9aGPWQUWV6PxirhncrG0OEapnFM3pNBs
XHqCXeK4oTLo+uXaJlHFdrAO83S+PkKExsymZIKBK3/lHFddSf3Qhcgxh95bidps
TJMOE8E13X+EgBBcqyAho6Dwc2qtXjVerZYjXMKJn1A1JTUQ18CXzIZGwNa2QrDU
Rmn+ufQ67dLNGVVHiUPKsQ7zUmWhoysz9SoEG6iPHEhQAxxz0mjXzPyVhWK6Euhi
n9xylPP5UC51z7FHWc+mwqRjM50SNT6a6Uky0OrZFp/rAFVRAgMBAAGjNTAzMAwG
A1UdEwEB/wQCMAAwDgYDVR0PAQH/BAQDAgeAMBMGA1UdJQQMMAoGCCsGAQUFBwMC
MA0GCSqGSIb3DQEBCwUAA4IBAQC7ipLuXgN+oUOZWdzWVBkCR6ABRZCO/5ZGLPwC
VvfPRA5CXLXiShxh10U8rXX+VKj/tu6gq4L0S8/PgMfYiYu3e/ZK+gjE1JOvZYP9
jD8RKlT8Biuth1EVyCsaH16Harr6xLbbirprvSraHa8x1hObg6YGNwvKC/lSzx2j
/zwWpta6I1mfdxkWHy32lFaYBnVPGntMXrWr31YNWQr90h+6I8rCHMXwLJ0bJWpV
zwkPfqL+HAv3NrljxIhgAD9p+lCUawdrtNSabrNOut/AgvLR32DBM9Xu9PxFvnmH
3mSdbF3asbS1fSy6dNFfCuiZCLdeWICTtQ1RoWszJy4w3L3c
-----END CERTIFICATE-----
"#;
const PRIVATE_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQDsmaXws92V/1Ha
eoXbvRsQ1m9eVgaNAaTAYxJr1r4G7NEB2KEHEis7QDlFmGY2ViwmF5UQ9aGPWQUW
V6PxirhncrG0OEapnFM3pNBsXHqCXeK4oTLo+uXaJlHFdrAO83S+PkKExsymZIKB
K3/lHFddSf3Qhcgxh95bidpsTJMOE8E13X+EgBBcqyAho6Dwc2qtXjVerZYjXMKJ
n1A1JTUQ18CXzIZGwNa2QrDURmn+ufQ67dLNGVVHiUPKsQ7zUmWhoysz9SoEG6iP
HEhQAxxz0mjXzPyVhWK6Euhin9xylPP5UC51z7FHWc+mwqRjM50SNT6a6Uky0OrZ
Fp/rAFVRAgMBAAECggEAGjIIxVWkuu7ByUa3e1S8omibe/5aS1JbQ1G0wGvOLkYZ
/fJWIcpJLw41SRdOecWgrbb+shoAATcB8S9S33NUZ1+zfRid+jlsnUSKLvUuOzDZ
4Uc3s8PTnwuPEiXJ6F3cFxQdmUXJcsK+ED8Yse0pXxLKH0QwngtE+y2QGV2BCpW+
1z+7gR9Dnwgb+Iu37Z6kmwQt5jLgZN9sRk2oui93hsJnEsmvHwWVKLN3o5a4RgRb
dNFpzFpzEpYhywJ3TqAQZA5wgmi3f/uLViLe8VTvr/bGiBbtBVZZ+chbbpHvxPjo
om8T2FcO7UQ5zMaz5LmvL1DmBzKUjlWwFjzqY6+14QKBgQD2YIa3PfKWMmic1UTf
D461iSXMb63+YZYDZECqAq/2SEr1QGx7Stp2d4KAO3orXKZB43F1n0BfKytFcVAO
zI3rOGoCa4V71rB8VO4WcfpLv6Vo8HH5yX/ko3RnYr2thOkvvHSJDRaXAn0odmkK
HBWgCwpD9Mhg1FtazIeeVt25iwKBgQD11111i+NlVrk73b1LSWBBFBq9zoMrqhLp
2IecCqds2kN8aSFQ9PXAjXnl1YoP5Ra/2SYLZ9dJwtoHV0C+EQ52UG023Y/KGGy/
yMePJL37tw1w1+voHXj06RhAZv9QRbVyjL6im2zr09ndECfnNfxDRdMC1K2eGuwp
vIFvonSwEwKBgFWPKdliyua79LZaHcO03GNQkG7RbcQEUZs16nrf2gmWC2nmCIY8
dWdkk05C2WdLknUZLiKhvjIOhMVl1158TOCdw7q5ai6aXMmgmNn0lO5PTHsH3RzY
i/Bzjw4ELUif+nbYZEPf5rNkauBtVobrP9tHWjZFQHzX4i3DD2m+UtuHAoGBAJcQ
WraTWgz4aGR4EgyELyhuQSxnDMU1AeA+FL528YB68kMtmrJTg/tpNAFd4TN5Myx6
M92IVotVt4IRRJBGjNt4+lNffC7UKWcD6lR0YGWfixvPIoAENEJSRXu1jqd13lRg
Vfg3h4u2J0am2c5CpxtzeYFgzLq9kmbnFaZSbl+PAoGAUIeFiC7ob5NHl65lgSwW
G51wz+8gVxCbhaMxNzoV5f/T/3AZJYU33V1DTjyC3VrnqiG+D5u/1siIxMjYbRd3
Ped8HJOuRWI+Cuat87QeDoL4390vn5QzCt+4Ijfdt2nOJuCjifsU1E92LBIafijb
0CJWSSMM++TqN3d4WvmJVko=
-----END PRIVATE KEY-----
"#;

#[tokio::test]
async fn selected_token_file_is_reread_without_exec_or_provider() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("rotating-token");
    std::fs::write(&path, "fixture-first").unwrap();
    let source = config(AuthInfo {
        token_file: Some(path.to_string_lossy().into_owned()),
        ..Default::default()
    });
    assert!(renewable(&source));
    let mut first = source.clone();
    resolve(&mut first, root.path()).await.unwrap();
    assert_eq!(
        serde_json::to_value(&first.auth_infos[0].auth_info).unwrap()["token"],
        "fixture-first"
    );
    std::fs::write(&path, "fixture-second\n").unwrap();
    let mut next = source;
    resolve(&mut next, root.path()).await.unwrap();
    assert_eq!(
        serde_json::to_value(&next.auth_infos[0].auth_info).unwrap()["token"],
        "fixture-second"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn renewal_reuses_only_selected_configured_exec_helper() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let helper = root.path().join("fixture-auth-helper");
    let marker = root.path().join("calls");
    std::fs::write(&helper, format!(
        "#!/bin/sh\nprintf x >> '{}'\nprintf '%s' '{{\"apiVersion\":\"client.authentication.k8s.io/v1\",\"kind\":\"ExecCredential\",\"status\":{{\"token\":\"fixture-exec-token\",\"expirationTimestamp\":\"{}\"}}}}'\n",
        marker.display(), future()
    )).unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut selected_exec = exec();
    selected_exec.command = Some(helper.to_string_lossy().into_owned());
    let source = config(AuthInfo {
        exec: Some(selected_exec),
        ..Default::default()
    });
    assert!(renewable(&source));
    for _ in 0..2 {
        let mut selected = source.clone();
        assert!(resolve(&mut selected, root.path()).await.unwrap().is_some());
        assert!(
            selected.auth_infos[0]
                .auth_info
                .as_ref()
                .unwrap()
                .exec
                .is_none()
        );
    }
    assert_eq!(std::fs::read(marker).unwrap(), b"xx");
}
