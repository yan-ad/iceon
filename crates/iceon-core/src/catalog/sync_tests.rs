use super::tests::{CONFIG, fixture, import};
use super::*;

fn preferences(paths: &[&Path]) -> KubernetesPreferences {
    KubernetesPreferences {
        discover_default_kubeconfig: false,
        kubeconfig_sync_paths: paths
            .iter()
            .map(|path| path.to_str().unwrap().into())
            .collect(),
        ..Default::default()
    }
}

#[cfg(unix)]
#[test]
fn dangling_sync_directory_symlink_retains_contexts_and_pins() {
    use std::os::unix::fs::symlink;

    let (directory, engine, _) = fixture();
    let target = directory.path().join("target");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("first.yaml"), CONFIG).unwrap();
    fs::write(
        target.join("second.yaml"),
        CONFIG.replace("development", "second"),
    )
    .unwrap();
    let alias = directory.path().join("sync");
    symlink(&target, &alias).unwrap();
    let settings = preferences(&[&alias]);
    let first = engine.list_with_preferences(&settings).unwrap();
    assert_eq!(first.clusters.len(), 2);
    assert!(first.discovery_error.is_none());
    for cluster in &first.clusters {
        engine
            .set_pin(PinRequest {
                id: cluster.id.clone(),
                pinned: true,
            })
            .unwrap();
    }
    let pinned = engine.list_with_preferences(&settings).unwrap();
    assert_eq!(pinned.pinned_items.len(), 2);

    fs::remove_dir_all(&target).unwrap();
    assert!(
        fs::symlink_metadata(&alias)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(!alias.exists());
    let restarted = CatalogEngine::new(directory.path().join("data"));
    for catalog in [&engine, &restarted] {
        let unavailable = catalog.list_with_preferences(&settings).unwrap();
        assert_eq!(
            unavailable.discovery_error,
            Some(CatalogError::InputUnavailable)
        );
        assert_eq!(unavailable.clusters, pinned.clusters);
        assert_eq!(unavailable.pinned_items, pinned.pinned_items);
        assert_eq!(unavailable.revision, pinned.revision);
    }
}

#[test]
fn sync_file_updates_pins_deletion_restart_and_redaction() {
    let (directory, engine, input) = fixture();
    let preferences = preferences(&[&input]);
    let first = engine.list_with_preferences(&preferences).unwrap();
    assert_eq!(first.clusters.len(), 1);
    assert!(first.discovery_error.is_none());
    assert_eq!(engine.list_with_preferences(&preferences).unwrap(), first);
    engine
        .set_pin(PinRequest {
            id: first.clusters[0].id.clone(),
            pinned: true,
        })
        .unwrap();
    fs::write(
        &input,
        CONFIG.replace(
            "users:",
            "- name: added\n  context: {cluster: fixture}\nusers:",
        ),
    )
    .unwrap();
    let second = engine.list_with_preferences(&preferences).unwrap();
    assert_eq!(second.clusters.len(), 2);
    let original = second
        .clusters
        .iter()
        .find(|row| row.name == "development")
        .unwrap();
    assert!(original.pinned);
    assert_eq!(original.color, first.clusters[0].color);
    fs::write(&input, CONFIG.replace("development", "production")).unwrap();
    let third = engine.list_with_preferences(&preferences).unwrap();
    assert_eq!(third.clusters.len(), 1);
    assert_eq!(third.clusters[0].name, "production");
    assert!(third.pinned_items.is_empty());
    let stored = fs::read(directory.path().join("data/catalog.sqlite3")).unwrap();
    for secret in [
        "SECRET_CERT",
        "SECRET_TOKEN",
        "NEVER_EXECUTE",
        "secret-server",
    ] {
        assert!(!String::from_utf8_lossy(&stored).contains(secret));
    }
    fs::remove_file(&input).unwrap();
    let restarted = CatalogEngine::new(directory.path().join("data"));
    let deleted = restarted.list_with_preferences(&preferences).unwrap();
    assert!(deleted.clusters.is_empty());
    assert!(deleted.discovery_error.is_none());
    assert!(deleted.revision > third.revision);
}

#[test]
fn sync_folder_is_shallow_and_tracks_additions_and_removals() {
    let (directory, engine, manual) = fixture();
    let folder = directory.path().join("sync");
    fs::create_dir(&folder).unwrap();
    let settings = preferences(&[&folder]);
    assert!(
        engine
            .list_with_preferences(&settings)
            .unwrap()
            .clusters
            .is_empty()
    );
    let first = folder.join("first");
    fs::write(&first, CONFIG).unwrap();
    assert_eq!(
        engine
            .list_with_preferences(&settings)
            .unwrap()
            .clusters
            .len(),
        1
    );
    fs::write(
        folder.join("second.yaml"),
        CONFIG.replace("development", "second"),
    )
    .unwrap();
    fs::create_dir(folder.join("nested")).unwrap();
    fs::write(folder.join("nested/ignored.yaml"), CONFIG).unwrap();
    assert_eq!(
        engine
            .list_with_preferences(&settings)
            .unwrap()
            .clusters
            .len(),
        2
    );
    import(&engine, &manual).unwrap();
    fs::remove_file(first).unwrap();
    assert_eq!(
        engine
            .list_with_preferences(&settings)
            .unwrap()
            .clusters
            .len(),
        2
    );
    fs::remove_dir_all(&folder).unwrap();
    let removed = engine.list_with_preferences(&settings).unwrap();
    assert_eq!(removed.clusters.len(), 1);
    assert_eq!(removed.clusters[0].name, "development");
}

#[test]
fn disable_default_and_remove_sync_root_preserve_manual_imports() {
    let (directory, _, input) = fixture();
    let home = directory.path().join("home");
    fs::create_dir_all(home.join(".kube")).unwrap();
    fs::write(home.join(".kube/config"), CONFIG).unwrap();
    let engine = CatalogEngine::new_with_home(directory.path().join("data"), Some(home));
    let disabled = preferences(&[]);
    assert!(
        engine
            .list_with_preferences(&disabled)
            .unwrap()
            .clusters
            .is_empty()
    );
    let enabled = KubernetesPreferences::default();
    assert_eq!(
        engine
            .list_with_preferences(&enabled)
            .unwrap()
            .clusters
            .len(),
        1
    );
    assert!(
        engine
            .list_with_preferences(&disabled)
            .unwrap()
            .clusters
            .is_empty()
    );
    let settings = preferences(&[&input]);
    engine.list_with_preferences(&settings).unwrap();
    import(&engine, &input).unwrap(); // Promote a synced source to explicitly managed.
    fs::remove_file(input).unwrap();
    assert_eq!(
        engine
            .list_with_preferences(&disabled)
            .unwrap()
            .clusters
            .len(),
        1
    );
}

#[test]
fn sync_errors_are_sanitized_and_retain_last_valid_rows() {
    let (_directory, engine, input) = fixture();
    let mut settings = preferences(&[&input]);
    let first = engine.list_with_preferences(&settings).unwrap();
    for bytes in [b"SECRET_TOKEN: [".to_vec(), vec![b'x'; MAX_INPUT + 1]] {
        fs::write(&input, bytes).unwrap();
        let failed = engine.list_with_preferences(&settings).unwrap();
        assert!(failed.discovery_error.is_some());
        assert_eq!(failed.clusters, first.clusters);
        assert_eq!(failed.revision, first.revision);
    }
    for invalid in ["", "relative.yaml", "/bad\0path", "/bad\npath"] {
        settings.kubeconfig_sync_paths = vec![invalid.into()];
        let failed = engine.list_with_preferences(&settings).unwrap();
        assert_eq!(failed.discovery_error, Some(CatalogError::InvalidInput));
        assert_eq!(failed.clusters, first.clusters);
    }
    settings.kubeconfig_sync_paths = vec![input.to_str().unwrap().into(); 65];
    assert_eq!(
        engine
            .list_with_preferences(&settings)
            .unwrap()
            .discovery_error,
        Some(CatalogError::LimitExceeded)
    );
}

#[cfg(unix)]
#[test]
fn sync_folder_does_not_follow_child_symlinks_and_explicit_alias_is_safe() {
    use std::os::unix::fs::symlink;
    let (directory, engine, input) = fixture();
    let folder = directory.path().join("sync");
    fs::create_dir(&folder).unwrap();
    let alias = folder.join("alias");
    symlink(&input, &alias).unwrap();
    symlink(&folder, folder.join("loop")).unwrap();
    assert!(
        engine
            .list_with_preferences(&preferences(&[&folder]))
            .unwrap()
            .clusters
            .is_empty()
    );
    let explicit = preferences(&[&alias]);
    assert_eq!(
        engine
            .list_with_preferences(&explicit)
            .unwrap()
            .clusters
            .len(),
        1
    );
    fs::remove_file(&input).unwrap();
    let broken = engine.list_with_preferences(&explicit).unwrap();
    assert_eq!(broken.discovery_error, Some(CatalogError::InputUnavailable));
    assert_eq!(broken.clusters.len(), 1);
    fs::remove_file(&alias).unwrap();
    assert!(
        engine
            .list_with_preferences(&explicit)
            .unwrap()
            .clusters
            .is_empty()
    );
}

#[test]
fn folder_budget_failure_does_not_prune_existing_rows() {
    let (directory, engine, _) = fixture();
    let folder = directory.path().join("sync");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("config"), CONFIG).unwrap();
    let settings = preferences(&[&folder]);
    let first = engine.list_with_preferences(&settings).unwrap();
    for index in 0..65 {
        fs::write(folder.join(format!("extra-{index}")), CONFIG).unwrap();
    }
    let failed = engine.list_with_preferences(&settings).unwrap();
    assert_eq!(failed.discovery_error, Some(CatalogError::LimitExceeded));
    assert_eq!(failed.clusters, first.clusters);
}

#[test]
fn manually_imported_configured_source_is_not_rewritten_by_sync() {
    let (_directory, engine, input) = fixture();
    let manual = import(&engine, &input).unwrap();
    fs::write(&input, CONFIG.replace("development", "changed")).unwrap();
    let settings = preferences(&[&input]);
    assert_eq!(engine.list_with_preferences(&settings).unwrap(), manual);
    fs::remove_file(&input).unwrap();
    assert_eq!(engine.list_with_preferences(&settings).unwrap(), manual);
}
