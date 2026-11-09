use super::*;
use tempfile::{TempDir, tempdir_in};

fn fixture() -> TempDir {
    let dir = tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    let root = dir.path();
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nresolver = \"2\"\nmembers = [\"crates/iceon-app\", \"crates/iceon-core\", \"crates/iceon-ui\"]\n\n[workspace.package]\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    let mut lock = String::from("version = 4\n");
    for name in ["iceon-app", "iceon-core", "iceon-ui"] {
        let member = root.join("crates").join(name);
        fs::create_dir_all(member.join("src")).unwrap();
        fs::write(
            member.join("Cargo.toml"),
            format!(
                "[package]\nname = {name:?}\nversion.workspace = true\nedition.workspace = true\n"
            ),
        )
        .unwrap();
        fs::write(
            member.join(if name == "iceon-app" {
                "src/main.rs"
            } else {
                "src/lib.rs"
            }),
            if name == "iceon-app" {
                "fn main() {}\n"
            } else {
                ""
            },
        )
        .unwrap();
        lock.push_str(&format!(
            "\n[[package]]\nname = {name:?}\nversion = \"0.1.0\"\n"
        ));
    }
    // Registry dependencies are not part of the local release version bump.
    lock.push_str("\n[[package]]\nname = \"iceon-core\"\nversion = \"9.0.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n");
    fs::write(root.join("Cargo.lock"), lock).unwrap();
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
    git(root, &["add", "--", "Cargo.toml", "Cargo.lock", "crates"]).unwrap();
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
    assert_eq!(plan.old, Version::new(0, 1, 0));
    assert_eq!(plan.new, Version::new(0, 2, 0));
    assert_eq!(
        plan.changes.iter().map(|c| c.path).collect::<Vec<_>>(),
        ["Cargo.toml", "Cargo.lock"]
    );
    let manifest = doc(&plan.changes[0].after).unwrap();
    assert_eq!(
        manifest["workspace"]["package"]["version"].as_str(),
        Some("0.2.0")
    );
    let lock = doc(&plan.changes[1].after).unwrap();
    for entry in lock["package"].as_array_of_tables().unwrap() {
        assert_eq!(
            entry["version"].as_str(),
            Some(if entry.get("source").is_some() {
                "9.0.0"
            } else {
                "0.2.0"
            })
        );
    }
    fs::write(
        d.path().join("Cargo.lock"),
        read(d.path(), "Cargo.lock")
            .unwrap()
            .replace("0.1.0", "0.0.0"),
    )
    .unwrap();
    assert!(
        version_plan(d.path(), "patch", false)
            .err()
            .unwrap()
            .contains("Cargo.lock: version/name differs")
    );
    fs::write(
        d.path().join("Cargo.lock"),
        "[[package]]\nname=\"unrelated\"\nversion=\"0.1.0\"\n",
    )
    .unwrap();
    assert!(
        version_plan(d.path(), "patch", false)
            .err()
            .unwrap()
            .contains("at least one local iceon workspace package")
    );
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
    assert!(!d.path().join(".git/FETCH_HEAD").exists());
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
    assert!(
        release(d.path(), &o)
            .unwrap_err()
            .contains("check out the intended release branch")
    );
    fs::write(d.path().join("untracked"), "x").unwrap();
    assert!(
        release(d.path(), &options())
            .unwrap_err()
            .contains("working tree/index must be clean")
    );
    fs::remove_file(d.path().join("untracked")).unwrap();
    git(d.path(), &["tag", "v0.1.1"]).unwrap();
    assert!(
        release(d.path(), &options())
            .unwrap_err()
            .contains("release tag already exists locally")
    );
    git(d.path(), &["tag", "-d", "v0.1.1"]).unwrap();
    git(
        d.path(),
        &["remote", "set-url", "--push", "origin", "/different.git"],
    )
    .unwrap();
    assert!(
        release(d.path(), &options())
            .unwrap_err()
            .contains("remote must have exactly one identical fetch/push URL")
    );
}
#[test]
fn refuses_unapproved_publication_remote_before_network_or_writes() {
    let d = fixture();
    repository(d.path());
    let mut o = options();
    o.dry_run = false;
    let head = git(d.path(), &["rev-parse", "HEAD"]).unwrap();
    let plan = version_plan(d.path(), "patch", false).unwrap();
    let error = release(d.path(), &o).unwrap_err();
    assert!(
        error
            .contains("actual publication is restricted to the configured yan-ad/iceon repository"),
        "{error}"
    );
    assert_eq!(git(d.path(), &["rev-parse", "HEAD"]).unwrap(), head);
    assert_eq!(git(d.path(), &["tag", "--list"]).unwrap(), "");
    assert!(!d.path().join(".git/FETCH_HEAD").exists());
    for change in plan.changes {
        assert_eq!(read(d.path(), change.path).unwrap(), change.before);
    }
    clean(d.path()).unwrap();
}
#[test]
fn rejects_js_workspaces_and_locks() {
    let d = fixture();
    for path in [
        "package.json",
        "package-lock.json",
        "yarn.lock",
        "pnpm-lock.yaml",
        "bun.lock",
        "bun.lockb",
    ] {
        fs::write(d.path().join(path), "{}").unwrap();
        let error = version_plan(d.path(), "patch", false).err().unwrap();
        assert!(
            error.contains(&format!("unsupported JS/TS workspace/lock: {path}")),
            "{error}"
        );
        fs::remove_file(d.path().join(path)).unwrap();
    }
    version_plan(d.path(), "patch", false).unwrap();
}
#[test]
fn symlinked_cargo_version_files_fail_closed() {
    let d = fixture();
    #[cfg(unix)]
    for path in ["Cargo.toml", "Cargo.lock"] {
        let actual = format!("actual-{path}");
        fs::rename(d.path().join(path), d.path().join(&actual)).unwrap();
        std::os::unix::fs::symlink(&actual, d.path().join(path)).unwrap();
        let error = version_plan(d.path(), "patch", false).err().unwrap();
        assert!(
            error.contains(&format!("{path}: symlink paths are not supported")),
            "{error}"
        );
        fs::remove_file(d.path().join(path)).unwrap();
        fs::rename(d.path().join(actual), d.path().join(path)).unwrap();
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
