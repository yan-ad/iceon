pub(super) fn save_fixture(engine: &CatalogEngine, store: &Store) {
    let _lock = storage::lock(&engine.directory).unwrap();
    let mut connection = storage::open(&engine.directory).unwrap();
    let transaction = connection.transaction().unwrap();
    storage::save(&transaction, store).unwrap();
    transaction.commit().unwrap();
}
use super::*;

pub(super) const CONFIG: &str = "apiVersion: v1\nkind: Config\nclusters:\n- name: fixture\n  cluster:\n    server: https://secret-server.invalid\n    certificate-authority-data: SECRET_CERT\ncontexts:\n- name: development\n  context:\n    cluster: fixture\n    user: secret-user\nusers:\n- name: secret-user\n  user:\n    token: SECRET_TOKEN\n    exec:\n      command: NEVER_EXECUTE\n";
pub(super) fn fixture() -> (tempfile::TempDir, CatalogEngine, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("fixture.yaml");
    fs::write(&input, CONFIG).unwrap();
    let engine = CatalogEngine::new(dir.path().join("data"));
    (dir, engine, input)
}
pub(super) fn import(engine: &CatalogEngine, path: &Path) -> Result<CatalogSnapshot> {
    engine.import_kubeconfig(ImportRequest {
        path: path.to_str().unwrap().into(),
    })
}
pub(super) fn add(engine: &CatalogEngine, name: &str) -> Result<CatalogSnapshot> {
    let input = tempfile::NamedTempFile::new().unwrap();
    let quoted = serde_json::to_string(name).unwrap();
    fs::write(input.path(), CONFIG.replace("development", &quoted)).unwrap();
    import(engine, input.path())
}
pub(super) fn cluster_record(index: usize) -> ClusterRecord {
    let source = format!("/fixtures/config{index}");
    ClusterRecord {
        id: cluster_id(&source, "development"),
        context_name: "development".into(),
        provenance_path: source.clone(),
        source_path: source,
        pinned: false,
        pinned_sections: vec![],
        color: format!("#{index:06X}"),
    }
}

#[test]
fn persistence_redaction_dedup_and_stale_reimport() {
    let (dir, engine, input) = fixture();
    assert_eq!(engine.list().unwrap().revision, 0);
    assert!(dir.path().join("data/catalog.sqlite3").exists());
    let first = import(&engine, &input).unwrap();
    assert_eq!(first.clusters[0].source, "kubeconfig");
    assert_eq!(first.clusters[0].status, "disconnected");
    assert_eq!(import(&engine, &input).unwrap(), first);
    let id = first.clusters[0].id.clone();
    let pinned = engine
        .set_pin(PinRequest {
            id: id.clone(),
            pinned: true,
        })
        .unwrap();
    assert_eq!(import(&engine, &input).unwrap(), pinned);
    let restarted = CatalogEngine::new(dir.path().join("data"));
    assert_eq!(restarted.list().unwrap(), pinned);
    let snapshot = serde_json::to_string(&pinned).unwrap();
    let stored =
        String::from_utf8_lossy(&fs::read(dir.path().join("data/catalog.sqlite3")).unwrap())
            .into_owned();
    for secret in [
        "SECRET_TOKEN",
        "SECRET_CERT",
        "NEVER_EXECUTE",
        "secret-user",
        "secret-server",
    ] {
        assert!(!snapshot.contains(secret));
        assert!(!stored.contains(secret));
    }
    assert_eq!(
        first.clusters[0].labels,
        vec![format!("file={}", input.display())]
    );
    fs::write(&input, CONFIG.replace("development", "production")).unwrap();
    let changed = import(&engine, &input).unwrap();
    assert_eq!(changed.clusters.len(), 1);
    assert!(!changed.clusters[0].pinned);
    assert_ne!(changed.clusters[0].id, id);
    assert_eq!(
        engine.set_pin(PinRequest { id, pinned: true }),
        Err(CatalogError::NotFound)
    );
    fs::write(
        &input,
        "apiVersion: v1\nkind: Config\nclusters: []\ncontexts: []\n",
    )
    .unwrap();
    assert!(import(&engine, &input).unwrap().clusters.is_empty());
}

#[test]
fn malformed_imports_are_sanitized_and_transactional() {
    let (_dir, engine, input) = fixture();
    let first = import(&engine, &input).unwrap();
    for content in [
        "SECRET_TOKEN: [",
        "apiVersion: wrong\nkind: Config\nclusters: []\ncontexts: []",
        "apiVersion: v1\nkind: Wrong\nclusters: []\ncontexts: []",
        "apiVersion: v1\nkind: Config\nclusters: []\ncontexts:\n- name: x\n  context: {cluster: missing}",
        "apiVersion: v1\nkind: Config\nclusters:\n- name: x\n- name: x\ncontexts: []",
    ] {
        fs::write(&input, content).unwrap();
        let error = import(&engine, &input).unwrap_err();
        assert_eq!(error, CatalogError::InvalidKubeconfig);
        assert_eq!(
            serde_json::to_string(&error).unwrap(),
            "\"invalidKubeconfig\""
        );
        assert_eq!(engine.list().unwrap(), first);
    }
    fs::write(
        &input,
        CONFIG.replace(
            "users:",
            "- name: development\n  context: {cluster: fixture}\nusers:",
        ),
    )
    .unwrap();
    assert_eq!(
        import(&engine, &input),
        Err(CatalogError::InvalidKubeconfig)
    );
    fs::write(&input, vec![b'x'; MAX_INPUT + 1]).unwrap();
    assert_eq!(import(&engine, &input), Err(CatalogError::LimitExceeded));
    assert_eq!(
        import(&engine, input.parent().unwrap()),
        Err(CatalogError::InputUnavailable)
    );
    assert_eq!(engine.list().unwrap(), first);
}

#[test]
fn cluster_entry_pins_and_removal() {
    let (_dir, engine, _) = fixture();
    let first = add(&engine, "production").unwrap();
    let id = first.clusters[0].id.clone();
    let pinned = engine
        .set_pin(PinRequest {
            id: id.clone(),
            pinned: true,
        })
        .unwrap();
    assert!(pinned.clusters[0].pinned);
    assert_eq!(
        engine
            .set_pin(PinRequest {
                id: id.clone(),
                pinned: true
            })
            .unwrap(),
        pinned
    );
    for id in ["clusters", "web-links", "catalog"] {
        assert_eq!(
            engine.set_pin(PinRequest {
                id: id.into(),
                pinned: true
            }),
            Err(CatalogError::NotFound)
        );
    }
    let removed = engine.remove(RemoveRequest { id: id.clone() }).unwrap();
    assert!(removed.clusters.is_empty());
    assert_eq!(
        engine.remove(RemoveRequest { id }),
        Err(CatalogError::NotFound)
    );
}

#[test]
fn corrupt_store_is_never_replaced() {
    let (dir, engine, input) = fixture();
    add(&engine, "development").unwrap();
    let path = dir.path().join("data/catalog.sqlite3");
    for content in [
        "{SECRET",
        "{}",
        "{\"schemaVersion\":2,\"revision\":0,\"clusters\":[],\"webLinks\":[]}",
    ] {
        fs::write(&path, content).unwrap();
        assert_eq!(engine.list(), Err(CatalogError::StoreCorrupt));
        assert_eq!(import(&engine, &input), Err(CatalogError::StoreCorrupt));
        assert_eq!(add(&engine, "production"), Err(CatalogError::StoreCorrupt));
        assert_eq!(fs::read(&path).unwrap(), content.as_bytes());
    }
    fs::write(&path, vec![b'x'; MAX_STORE + 1]).unwrap();
    assert_eq!(engine.list(), Err(CatalogError::StoreCorrupt));
}

#[cfg(unix)]
#[test]
fn connection_lookup_rejects_storage_symlinks_without_changes() {
    use std::os::unix::fs::symlink;
    for name in [
        "catalog.sqlite3",
        "catalog.lock",
        "catalog.sqlite3-journal",
        "catalog.sqlite3-wal",
        "catalog.sqlite3-shm",
    ] {
        let (dir, engine, input) = fixture();
        let saved = import(&engine, &input).unwrap();
        let path = engine.directory.join(name);
        let target = dir.path().join("target");
        if path.exists() {
            fs::rename(&path, &target).unwrap();
        } else {
            fs::write(&target, "fixture").unwrap();
        }
        let before = fs::read(&target).unwrap();
        symlink(&target, &path).unwrap();
        assert_eq!(
            engine.connection_target(&saved.clusters[0].id),
            Err(CatalogError::StoreCorrupt)
        );
        assert_eq!(fs::read(&target).unwrap(), before);
    }
}

#[test]
fn connection_lookup_never_initializes_or_migrates_missing_storage() {
    let (_dir, engine, _) = fixture();
    assert_eq!(
        engine.connection_target("missing"),
        Err(CatalogError::NotFound)
    );
    assert!(!engine.directory.exists());
    fs::create_dir_all(&engine.directory).unwrap();
    let legacy = engine.directory.join("catalog.json");
    fs::write(&legacy, "legacy fixture").unwrap();
    assert_eq!(
        engine.connection_target("missing"),
        Err(CatalogError::NotFound)
    );
    assert_eq!(fs::read_to_string(legacy).unwrap(), "legacy fixture");
    assert_eq!(fs::read_dir(&engine.directory).unwrap().count(), 1);
}

#[test]
fn transaction_failure_keeps_previous_state() {
    let (_dir, engine, _) = fixture();
    let first = add(&engine, "development").unwrap();
    let connection = rusqlite::Connection::open(engine.directory.join("catalog.sqlite3")).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_insert BEFORE INSERT ON clusters BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
    assert_eq!(
        add(&engine, "production"),
        Err(CatalogError::StoreUnavailable)
    );
    assert_eq!(engine.list().unwrap(), first);
}

#[test]
fn bounds_schema_and_wire_contract() {
    let (_dir, engine, input) = fixture();
    for name in [
        "".into(),
        " padded ".into(),
        "x".repeat(MAX_NAME + 1),
        "bad\nname".into(),
    ] {
        assert_eq!(add(&engine, &name), Err(CatalogError::InvalidKubeconfig));
    }
    fs::write(
        &input,
        CONFIG.replace("development", &"x".repeat(MAX_NAME + 1)),
    )
    .unwrap();
    assert_eq!(
        import(&engine, &input),
        Err(CatalogError::InvalidKubeconfig)
    );
    let mut config =
        "apiVersion: v1\nkind: Config\nclusters:\n- name: fixture\ncontexts:\n".to_owned();
    for i in 0..=MAX_ENTRIES {
        config.push_str(&format!(
            "- name: ctx{i}\n  context: {{cluster: fixture}}\n"
        ));
    }
    fs::write(&input, config).unwrap();
    assert_eq!(import(&engine, &input), Err(CatalogError::LimitExceeded));
    fs::write(&input, CONFIG).unwrap();
    let value = serde_json::to_value(import(&engine, &input).unwrap()).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 4);
    assert!(value.get("autoDiscover").is_none());
    assert!(value["discoveryError"].is_null());
    assert!(value.get("webLinks").is_none());
    let cluster = value["clusters"][0].as_object().unwrap();
    assert_eq!(cluster.len(), 9);
    assert_eq!(cluster["pinnedSections"], serde_json::json!([]));
    assert!(cluster.contains_key("contextName"));
}

#[cfg(unix)]
#[test]
fn private_permissions_and_storage_symlink_rejection() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let (dir, engine, input) = fixture();
    import(&engine, &input).unwrap();
    assert_eq!(
        fs::metadata(dir.path().join("data"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(dir.path().join("data/catalog.sqlite3"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let link = dir.path().join("link.yaml");
    symlink(&input, &link).unwrap();
    assert!(import(&engine, &link).is_ok());
    fs::remove_file(dir.path().join("data/catalog.sqlite3")).unwrap();
    symlink(&input, dir.path().join("data/catalog.sqlite3")).unwrap();
    assert_eq!(engine.list(), Err(CatalogError::StoreCorrupt));
}

#[test]
fn concurrent_updates_are_serialized() {
    let (_dir, engine, _) = fixture();
    std::thread::scope(|scope| {
        for i in 0..12 {
            let engine = &engine;
            scope.spawn(move || {
                add(engine, &format!("fixture{i}")).unwrap();
            });
        }
    });
    let snapshot = engine.list().unwrap();
    assert_eq!(snapshot.clusters.len(), 12);
    assert_eq!(snapshot.revision, 12);
}

#[test]
fn catalog_capacity_and_revision_overflow_preserve_disk() {
    let (dir, engine, _) = fixture();
    let mut store = Store::empty();
    for i in 0..MAX_ENTRIES {
        store.clusters.push(cluster_record(i));
    }
    save_fixture(&engine, &store);
    let original = fs::read(dir.path().join("data/catalog.sqlite3")).unwrap();
    assert_eq!(add(&engine, "extra"), Err(CatalogError::LimitExceeded));
    assert_eq!(
        fs::read(dir.path().join("data/catalog.sqlite3")).unwrap(),
        original
    );
    store.clusters.clear();
    store.revision = u64::MAX;
    save_fixture(&engine, &store);
    assert_eq!(add(&engine, "extra"), Err(CatalogError::LimitExceeded));
    assert_eq!(engine.list().unwrap().revision, u64::MAX);
}

#[test]
fn tampered_store_identity_fails_closed() {
    let (_dir, engine, _) = fixture();
    add(&engine, "development").unwrap();
    let connection = storage::open(&engine.directory).unwrap();
    let mut store = storage::load(&connection).unwrap();
    store.clusters[0].id = "catalog".into();
    connection
        .execute("UPDATE clusters SET id=?1", [&store.clusters[0].id])
        .unwrap();
    assert_eq!(engine.list(), Err(CatalogError::StoreCorrupt));
}
