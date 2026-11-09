use super::*;
use tempfile::{TempDir, tempdir_in};

fn fixture() -> TempDir {
    let dir = tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    let root = dir.path();
    fs::create_dir(root.join("src-tauri")).unwrap();
    fs::write(
        root.join("package.json"),
        r#"{"name":"iceon","version":"0.1.0"}"#,
    )
    .unwrap();
    fs::write(
        root.join("src-tauri/Cargo.toml"),
        "[package]\nname = \"iceon\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(
        root.join("src-tauri/tauri.conf.json"),
        r#"{"version":"0.1.0"}"#,
    )
    .unwrap();
    fs::write(
        root.join("src-tauri/Cargo.lock"),
        "version = 4\n[[package]]\nname = \"iceon\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(
        root.join("bun.lock"),
        "{\"workspaces\":{\"\":{\"name\":\"iceon\",},},}",
    )
    .unwrap();
    dir
}
fn options() -> Options {
    Options {
        bump: "patch".into(),
        dry_run: true,
        branch: "main".into(),
        remote: "origin".into(),
    }
}
fn repository(root: &Path) {
    git(root, &["init", "-b", "main"]).unwrap();
    git(root, &["config", "user.name", "Fixture"]).unwrap();
    git(root, &["config", "tag.gpgSign", "false"]).unwrap();
    git(root, &["config", "commit.gpgSign", "false"]).unwrap();
    git(root, &["config", "core.hooksPath", "/dev/null"]).unwrap();
    git(root, &["config", "user.email", "fixture@example.invalid"]).unwrap();
    // Tests never push: the fake remote is deliberately unreachable.
    git(
        root,
        &[
            "remote",
            "add",
            "origin",
            "/nonexistent/iceon-release-test.git",
        ],
    )
    .unwrap();
    git(root, &["config", "branch.main.remote", "origin"]).unwrap();
    git(root, &["config", "branch.main.merge", "refs/heads/main"]).unwrap();
    git(
        root,
        &["add", "--", "package.json", "bun.lock", "src-tauri"],
    )
    .unwrap();
    git(root, &["commit", "-m", "fixture"]).unwrap();
    git(root, &["update-ref", "refs/remotes/origin/main", "HEAD"]).unwrap();
}
#[test]
fn stable_versions_and_bumps() {
    let old = stable("1.2.3").unwrap();
    for invalid in [
        "v1.2.3",
        "01.2.3",
        "1.2",
        "1.2.3-beta",
        "1.2.3+build",
        "--help",
        "1.2.3;echo x",
    ] {
        assert!(stable(invalid).is_err(), "{invalid}");
    }
    assert_eq!(next(&old, "patch", false).unwrap().to_string(), "1.2.4");
    assert_eq!(next(&old, "minor", false).unwrap().to_string(), "1.3.0");
    assert_eq!(next(&old, "major", false).unwrap().to_string(), "2.0.0");
    assert!(next(&old, "1.2.3", false).is_err());
    assert!(next(&old, "1.1.0", false).is_err());
    assert!(next(&Version::new(u64::MAX, 0, 0), "major", false).is_err());
}
#[test]
fn version_plan_and_lock_validation() {
    let d = fixture();
    let plan = version_plan(d.path(), "0.2.0", false).unwrap();
    assert_eq!(plan.changes.len(), 4);
    assert!(plan.changes.iter().all(|c| c.after.contains("0.2.0")));
    fs::write(
        d.path().join("src-tauri/Cargo.lock"),
        "[[package]]\nname=\"iceon\"\nversion=\"0.0.0\"",
    )
    .unwrap();
    assert!(version_plan(d.path(), "patch", false).is_err());
}
#[test]
fn dry_run_is_local_read_only_with_fake_remote() {
    let d = fixture();
    repository(d.path());
    let head = git(d.path(), &["rev-parse", "HEAD"]).unwrap();
    let plan = version_plan(d.path(), "patch", false).unwrap();
    release(d.path(), &options()).unwrap();
    assert_eq!(git(d.path(), &["rev-parse", "HEAD"]).unwrap(), head);
    assert_eq!(git(d.path(), &["tag", "--list"]).unwrap(), "");
    clean(d.path()).unwrap();
    for change in plan.changes {
        assert_eq!(read(d.path(), change.path).unwrap(), change.before);
    }
}
#[test]
fn refuses_dirty_wrong_branch_existing_tag_and_wrong_remote() {
    let d = fixture();
    repository(d.path());
    let mut o = options();
    o.branch = "other".into();
    assert!(release(d.path(), &o).is_err());
    fs::write(d.path().join("untracked"), "x").unwrap();
    assert!(release(d.path(), &options()).is_err());
    fs::remove_file(d.path().join("untracked")).unwrap();
    git(d.path(), &["tag", "v0.1.1"]).unwrap();
    assert!(release(d.path(), &options()).is_err());
    git(d.path(), &["tag", "-d", "v0.1.1"]).unwrap();
    git(
        d.path(),
        &["remote", "set-url", "--push", "origin", "/different.git"],
    )
    .unwrap();
    assert!(release(d.path(), &options()).is_err());
}
#[test]
fn refuses_missing_updater_before_network_or_writes() {
    let d = fixture();
    repository(d.path());
    let mut o = options();
    o.dry_run = false;
    let error = release(d.path(), &o).unwrap_err();
    assert!(error.contains("createUpdaterArtifacts"));
    clean(d.path()).unwrap();
}
#[test]
fn bun_root_version_and_mismatch() {
    let d = fixture();
    fs::write(
        d.path().join("bun.lock"),
        r#"{"workspaces":{"":{"name":"iceon","version":"0.1.0"}}}"#,
    )
    .unwrap();
    let plan = version_plan(d.path(), "patch", false).unwrap();
    assert_eq!(plan.changes.len(), 5);
    fs::write(
        d.path().join("bun.lock"),
        r#"{"workspaces":{"":{"name":"iceon","version":"0.0.0"}}}"#,
    )
    .unwrap();
    assert!(version_plan(d.path(), "patch", false).is_err());
}
#[test]
fn unsupported_locks_and_symlinks_fail_closed() {
    let d = fixture();
    fs::write(d.path().join("package-lock.json"), "{}").unwrap();
    assert!(version_plan(d.path(), "patch", false).is_err());
    fs::remove_file(d.path().join("package-lock.json")).unwrap();
    #[cfg(unix)]
    {
        fs::rename(d.path().join("package.json"), d.path().join("actual.json")).unwrap();
        std::os::unix::fs::symlink("actual.json", d.path().join("package.json")).unwrap();
        assert!(version_plan(d.path(), "patch", false).is_err());
    }
}
#[test]
fn argument_validation() {
    let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert!(
        parse(&args(&["release", "patch", "--dry-run"]))
            .unwrap()
            .dry_run
    );
    for a in [
        vec!["release"],
        vec!["release", "patch", "--remote", "--evil"],
        vec!["release", "patch", "--dry-run", "--dry-run"],
    ] {
        assert!(parse(&args(&a)).is_err());
    }
}

#[test]
fn refuses_detached_head_and_nonancestor_upstream() {
    let d = fixture();
    repository(d.path());
    let head = git(d.path(), &["rev-parse", "HEAD"]).unwrap();
    git(d.path(), &["checkout", "--detach", &head]).unwrap();
    assert!(release(d.path(), &options()).is_err());
    git(d.path(), &["checkout", "main"]).unwrap();
    git(d.path(), &["checkout", "--orphan", "unrelated"]).unwrap();
    git(d.path(), &["commit", "-m", "unrelated"]).unwrap();
    let unrelated = git(d.path(), &["rev-parse", "HEAD"]).unwrap();
    git(d.path(), &["checkout", "main"]).unwrap();
    git(
        d.path(),
        &["update-ref", "refs/remotes/origin/main", &unrelated],
    )
    .unwrap();
    assert!(release(d.path(), &options()).is_err());
}
