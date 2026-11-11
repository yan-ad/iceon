use std::process::Command;

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_iceon-xtask"));
    command
        .env_remove("ICEON_UPDATE_PRIVATE_KEY")
        .env_remove("ICEON_UPDATE_PRIVATE_KEY_PASSWORD")
        .stdin(std::process::Stdio::null());
    command
}

#[test]
fn missing_environment_and_cli_errors_are_noninteractive_and_secret_safe() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("artifact.bin");
    std::fs::write(&data, b"payload").unwrap();
    for environment in [None, Some("not-base64-PRIVATE-SENTINEL")] {
        let mut cmd = command();
        cmd.arg("sign-update").arg(&data).arg("1.2.3");
        if let Some(key) = environment {
            cmd.env("ICEON_UPDATE_PRIVATE_KEY", key)
                .env("ICEON_UPDATE_PRIVATE_KEY_PASSWORD", "PASSWORD-SENTINEL");
        }
        let output = cmd.output().unwrap();
        assert!(!output.status.success());
        let errors = String::from_utf8(output.stderr).unwrap();
        assert!(!errors.contains("SENTINEL"));
        assert!(output.stdout.is_empty());
        assert!(!data.with_file_name("artifact.bin.sig").exists());
    }
    for args in [
        vec!["sign-update"],
        vec![
            "verify-update",
            "secret-sentinel",
            "artifact",
            "1.2.3",
            "extra",
        ],
    ] {
        let output = command().args(args).output().unwrap();
        assert!(!output.status.success());
        assert!(
            !String::from_utf8(output.stderr)
                .unwrap()
                .contains("secret-sentinel")
        );
    }
}

#[test]
fn pinned_cli_rejects_synthetic_key_without_creating_signature() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("artifact.bin");
    std::fs::write(&data, b"payload").unwrap();
    let pair = minisign::KeyPair::generate_encrypted_keypair(Some(String::new())).unwrap();
    use base64::Engine;
    let private = base64::engine::general_purpose::STANDARD
        .encode(pair.sk.to_box(None).unwrap().into_string());
    let output = command()
        .arg("sign-update")
        .arg(&data)
        .arg("1.2.3")
        .env("ICEON_UPDATE_PRIVATE_KEY", &private)
        .env("ICEON_UPDATE_PRIVATE_KEY_PASSWORD", "")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(!String::from_utf8(output.stderr).unwrap().contains(&private));
    assert!(!data.with_file_name("artifact.bin.sig").exists());
}
