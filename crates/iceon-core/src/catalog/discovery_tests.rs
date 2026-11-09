use super::tests::{CONFIG, add, import};
use super::*;

fn home_fixture() -> (tempfile::TempDir, PathBuf, CatalogEngine) {
    let directory = tempfile::tempdir().unwrap();
    let home = fs::canonicalize(directory.path()).unwrap().join("home");
    let engine = CatalogEngine::new_with_home(directory.path().join("data"), Some(home.clone()));
    (directory, home, engine)
}
fn config(home: &Path, contents: &str) {
    fs::create_dir_all(home.join(".kube")).unwrap();
    fs::write(home.join(".kube/config"), contents).unwrap();
}
#[test]
fn builtin_refresh_restart_errors_and_removal() {
    let (directory, home, engine) = home_fixture();
    config(&home, "INVALID_SECRET: [");
    assert!(!engine.directory.exists());
    let failed = engine.list().unwrap();
    assert_eq!(
        failed.discovery_error,
        Some(CatalogError::InvalidKubeconfig)
    );
    assert_eq!(failed.revision, 0);
    assert!(
        !serde_json::to_string(&failed)
            .unwrap()
            .contains("INVALID_SECRET")
    );
    assert!(
        serde_json::to_value(&failed)
            .unwrap()
            .get("autoDiscover")
            .is_none()
    );
    config(&home, CONFIG);
    let first = engine.list().unwrap();
    assert_eq!(first.clusters.len(), 1);
    assert_eq!(first.clusters[0].labels, ["file=~/.kube/config"]);
    assert!(first.discovery_error.is_none());
    let connection = storage::open(&engine.directory).unwrap();
    assert!(!storage::load(&connection).unwrap().auto_discover);
    drop(connection);
    let pinned = engine
        .set_pin(PinRequest {
            id: first.clusters[0].id.clone(),
            pinned: true,
        })
        .unwrap();
    assert_eq!(engine.list().unwrap(), pinned);
    let restarted = CatalogEngine::new_with_home(directory.path().join("data"), Some(home.clone()));
    assert_eq!(restarted.list().unwrap(), pinned);
    let removed = restarted
        .remove(RemoveRequest {
            id: first.clusters[0].id.clone(),
        })
        .unwrap();
    assert!(removed.clusters.is_empty());
    assert_eq!(restarted.list().unwrap().clusters.len(), 1);
    config(&home, &CONFIG.replace("development", "production"));
    let changed = restarted.list().unwrap();
    assert_ne!(changed.clusters[0].id, first.clusters[0].id);
    fs::remove_file(home.join(".kube/config")).unwrap();
    let missing = restarted.list().unwrap();
    assert_eq!(missing.clusters, changed.clusters);
    assert_eq!(missing.revision, changed.revision);
    assert_eq!(
        missing.discovery_error,
        Some(CatalogError::InputUnavailable)
    );
}
#[test]
fn missing_default_and_missing_injected_home_are_silent() {
    let (directory, home, engine) = home_fixture();
    for create_parent in [false, true] {
        if create_parent {
            fs::create_dir_all(home.join(".kube")).unwrap();
        }
        let snapshot = engine.list().unwrap();
        assert!(snapshot.clusters.is_empty());
        assert!(snapshot.discovery_error.is_none());
        assert_eq!(snapshot.revision, 0);
    }
    let engine = CatalogEngine::new(directory.path().join("no-home"));
    assert!(engine.list().unwrap().discovery_error.is_none());
    config(&home, CONFIG);
    let imported = import(&engine, &home.join(".kube/config")).unwrap();
    assert!(imported.clusters[0].labels[0].starts_with("file=/"));
}
#[test]
fn mutations_do_not_rescan_default() {
    let (_directory, home, engine) = home_fixture();
    config(&home, CONFIG);
    let first = engine.list().unwrap();
    config(&home, &CONFIG.replace("development", "production"));
    let added = add(&engine, "additional").unwrap();
    assert!(added.clusters.contains(&first.clusters[0]));
    assert!(!added.clusters.iter().any(|c| c.name == "production"));
    let pinned = engine
        .set_pin(PinRequest {
            id: first.clusters[0].id.clone(),
            pinned: true,
        })
        .unwrap();
    assert!(
        pinned
            .clusters
            .iter()
            .any(|c| c.name == "development" && c.pinned)
    );
    let explicit = home.join("explicit.yaml");
    fs::write(&explicit, CONFIG.replace("development", "explicit")).unwrap();
    let imported = import(&engine, &explicit).unwrap();
    assert!(!imported.clusters.iter().any(|c| c.name == "production"));
    assert!(
        engine
            .list()
            .unwrap()
            .clusters
            .iter()
            .any(|c| c.name == "production")
    );
}
#[test]
fn discovery_capacity_failure_retains_rows_and_legacy_setting() {
    let (_directory, home, engine) = home_fixture();
    let mut store = Store::empty();
    for i in 0..MAX_ENTRIES {
        store.clusters.push(super::tests::cluster_record(i));
    }
    super::tests::save_fixture(&engine, &store);
    config(&home, CONFIG);
    let snapshot = engine.list().unwrap();
    assert_eq!(snapshot.discovery_error, Some(CatalogError::LimitExceeded));
    assert_eq!(snapshot.clusters.len(), MAX_ENTRIES);
    assert_eq!(snapshot.revision, 0);
    let connection = storage::open(&engine.directory).unwrap();
    assert!(!storage::load(&connection).unwrap().auto_discover);
}
#[test]
fn colors_persist_and_collision_retry_is_deterministic() {
    let (directory, engine, input) = super::tests::fixture();
    let cluster = import(&engine, &input).unwrap().clusters.remove(0);
    let first = add(&engine, "additional").unwrap();
    assert!(valid_color(&cluster.color));
    let added = first.clusters.iter().find(|c| c.id != cluster.id).unwrap();
    assert!(valid_color(&added.color));
    assert_ne!(cluster.color, added.color);
    let pinned = engine
        .set_pin(PinRequest {
            id: added.id.clone(),
            pinned: true,
        })
        .unwrap();
    assert_eq!(import(&engine, &input).unwrap(), pinned);
    assert_eq!(
        CatalogEngine::new(directory.path().join("data"))
            .list()
            .unwrap(),
        pinned
    );
    let mut store = Store::empty();
    let mut cluster = super::tests::cluster_record(0);
    cluster.color = "#000000".into();
    store.clusters.push(cluster);
    let mut calls = 0;
    assert_eq!(
        color_with(&store, |bytes| {
            calls += 1;
            bytes.fill(if calls == 1 { 0 } else { 1 });
            Ok(())
        })
        .unwrap(),
        "#010101"
    );
    assert_eq!(calls, 2);
    assert_eq!(
        color_with(&store, |bytes| {
            bytes.fill(0);
            Ok(())
        }),
        Err(CatalogError::StoreUnavailable)
    );
}
#[test]
fn legacy_migration_preserves_identity_pins_and_u64_revision() {
    let (directory, home, engine) = home_fixture();
    config(&home, CONFIG);
    let source = fs::canonicalize(home.join(".kube/config"))
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    fs::create_dir_all(&engine.directory).unwrap();
    let legacy = serde_json::json!({"schemaVersion":1,"revision":u64::MAX,"clusters":[{
        "id":cluster_id(&source,"development"),"contextName":"development","sourcePath":source,"pinned":true}],
        "webLinks":{"obsolete":"malformed retired data is ignored"}});
    let bytes = serde_json::to_vec(&legacy).unwrap();
    fs::write(engine.directory.join("catalog.json"), &bytes).unwrap();
    let migrated = engine.list().unwrap();
    assert_eq!(migrated.revision, u64::MAX);
    assert!(migrated.clusters[0].pinned);
    assert_eq!(migrated.clusters[0].id, legacy["clusters"][0]["id"]);
    let connection = storage::open(&engine.directory).unwrap();
    assert!(!storage::load(&connection).unwrap().auto_discover);
    drop(connection);
    assert_eq!(
        fs::read(engine.directory.join("catalog.json.backup")).unwrap(),
        bytes
    );
    fs::write(
        engine.directory.join("catalog.json"),
        "ignored after migration",
    )
    .unwrap();
    assert_eq!(
        CatalogEngine::new_with_home(directory.path().join("data"), Some(home))
            .list()
            .unwrap(),
        migrated
    );
    assert_eq!(
        fs::read(engine.directory.join("catalog.json.backup")).unwrap(),
        bytes
    );
}
#[test]
fn corrupt_migration_and_backup_conflict_never_publish_database() {
    for bytes in [
        b"{SECRET".as_slice(),
        b"{}",
        br#"{"schemaVersion":2,"revision":0,"clusters":[],"webLinks":[]}"#,
    ] {
        let (_directory, _home, engine) = home_fixture();
        fs::create_dir_all(&engine.directory).unwrap();
        fs::write(engine.directory.join("catalog.json"), bytes).unwrap();
        assert_eq!(engine.list(), Err(CatalogError::StoreCorrupt));
        assert_eq!(add(&engine, "additional"), Err(CatalogError::StoreCorrupt));
        assert_eq!(
            fs::read(engine.directory.join("catalog.json")).unwrap(),
            bytes
        );
        assert!(!engine.directory.join("catalog.sqlite3").exists());
    }
    let (_directory, _home, engine) = home_fixture();
    fs::create_dir_all(&engine.directory).unwrap();
    fs::write(
        engine.directory.join("catalog.json"),
        br#"{"schemaVersion":1,"revision":0,"clusters":[],"webLinks":[]}"#,
    )
    .unwrap();
    fs::write(
        engine.directory.join("catalog.json.backup"),
        "existing backup",
    )
    .unwrap();
    assert_eq!(engine.list(), Err(CatalogError::StoreCorrupt));
    assert_eq!(
        fs::read_to_string(engine.directory.join("catalog.json.backup")).unwrap(),
        "existing backup"
    );
    assert!(!engine.directory.join("catalog.sqlite3").exists());
}
#[test]
fn independent_engines_serialize_updates() {
    let directory = tempfile::tempdir().unwrap();
    std::thread::scope(|scope| {
        for i in 0..12 {
            let path = directory.path().join("data");
            scope.spawn(move || add(&CatalogEngine::new(path), &format!("fixture{i}")).unwrap());
        }
    });
    let snapshot = CatalogEngine::new(directory.path().join("data"))
        .list()
        .unwrap();
    assert_eq!(snapshot.revision, 12);
    assert_eq!(snapshot.clusters.len(), 12);
}

#[test]
fn busy_cross_process_lock_times_out_and_recovers() {
    let (_directory, _home, engine) = home_fixture();
    let held = storage::lock(&engine.directory).unwrap();
    let started = std::time::Instant::now();
    assert_eq!(engine.list(), Err(CatalogError::StoreUnavailable));
    assert!(started.elapsed() < std::time::Duration::from_secs(7));
    drop(held);
    assert!(engine.list().unwrap().clusters.is_empty());
}

#[test]
fn builtin_discovery_ignores_both_legacy_settings_and_preserves_them() {
    for enabled in [false, true] {
        let (_directory, home, engine) = home_fixture();
        let mut store = Store::empty();
        store.auto_discover = enabled;
        super::tests::save_fixture(&engine, &store);
        config(&home, CONFIG);
        let discovered = engine.list().unwrap();
        assert_eq!(discovered.clusters.len(), 1);
        let connection = storage::open(&engine.directory).unwrap();
        assert_eq!(storage::load(&connection).unwrap().auto_discover, enabled);
    }
}

#[test]
fn invalid_default_preserves_pins_colors_and_redacts_errors() {
    let (_directory, home, engine) = home_fixture();
    config(&home, CONFIG);
    let discovered = engine.list().unwrap();
    let pinned = engine
        .set_pin(PinRequest {
            id: discovered.clusters[0].id.clone(),
            pinned: true,
        })
        .unwrap();
    for (contents, error) in [
        (
            "SECRET_TOKEN: [".to_owned(),
            CatalogError::InvalidKubeconfig,
        ),
        ("x".repeat(MAX_INPUT + 1), CatalogError::LimitExceeded),
    ] {
        config(&home, &contents);
        let failed = engine.list().unwrap();
        assert_eq!(failed.discovery_error, Some(error));
        assert_eq!(failed.clusters, pinned.clusters);
        assert_eq!(failed.revision, pinned.revision);
        assert!(
            !serde_json::to_string(&failed)
                .unwrap()
                .contains("SECRET_TOKEN")
        );
    }
    config(&home, CONFIG);
    assert_eq!(engine.list().unwrap(), pinned);
}

#[cfg(unix)]
#[test]
fn denied_default_and_denied_missing_parent_are_not_silent() {
    use std::os::unix::fs::PermissionsExt;
    let (_directory, home, engine) = home_fixture();
    config(&home, CONFIG);
    let input = home.join(".kube/config");
    let first = engine.list().unwrap();
    fs::set_permissions(&input, fs::Permissions::from_mode(0o000)).unwrap();
    // Privileged test runners may still read mode-000 files.
    if File::open(&input).is_err() {
        let failed = engine.list().unwrap();
        assert_eq!(failed.discovery_error, Some(CatalogError::InputUnavailable));
        assert_eq!(failed.clusters, first.clusters);
        assert_eq!(failed.revision, first.revision);
    }
    fs::set_permissions(&input, fs::Permissions::from_mode(0o600)).unwrap();
    fs::remove_file(&input).unwrap();
    let empty_engine = CatalogEngine::new_with_home(
        engine.directory.with_file_name("empty-data"),
        Some(home.clone()),
    );
    fs::set_permissions(home.join(".kube"), fs::Permissions::from_mode(0o000)).unwrap();
    if fs::symlink_metadata(&input)
        .is_err_and(|error| error.kind() == std::io::ErrorKind::PermissionDenied)
    {
        assert_eq!(
            empty_engine.list().unwrap().discovery_error,
            Some(CatalogError::InputUnavailable)
        );
    }
    fs::set_permissions(home.join(".kube"), fs::Permissions::from_mode(0o700)).unwrap();
}
