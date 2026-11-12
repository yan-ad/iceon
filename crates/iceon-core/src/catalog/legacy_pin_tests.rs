use super::{tests::*, *};

fn downgrade_to_schema_two(engine: &CatalogEngine) {
    let connection = rusqlite::Connection::open(engine.directory.join("catalog.sqlite3")).unwrap();
    connection.execute_batch("
        DROP TABLE pin_order;
        ALTER TABLE clusters DROP COLUMN pinned_sections;
        ALTER TABLE metadata RENAME TO metadata_old;
        CREATE TABLE metadata (singleton INTEGER PRIMARY KEY CHECK(singleton=1), schema_version INTEGER NOT NULL CHECK(schema_version=2), revision TEXT NOT NULL, auto_discover INTEGER NOT NULL CHECK(auto_discover IN (0,1))) STRICT;
        INSERT INTO metadata SELECT singleton,2,revision,auto_discover FROM metadata_old;
        DROP TABLE metadata_old;
    ").unwrap();
}

#[test]
fn schema_two_upgrade_preserves_all_metadata_without_revision_changes() {
    let (_dir, engine, input) = fixture();
    import(&engine, &input).unwrap();
    add(&engine, "additional").unwrap();
    let connection = storage::open(&engine.directory).unwrap();
    let mut store = storage::load(&connection).unwrap();
    drop(connection);
    store.clusters[0].provenance_path = input
        .parent()
        .unwrap()
        .join("alias.yaml")
        .to_str()
        .unwrap()
        .into();
    store.clusters[0].pinned = true;
    store.auto_discover = true;
    store.revision = u64::MAX - 1;
    save_fixture(&engine, &store);
    downgrade_to_schema_two(&engine);
    let database = engine.directory.join("catalog.sqlite3");
    let before = fs::read(&database).unwrap();
    assert!(matches!(
        storage::read_existing(&engine.directory),
        Err(CatalogError::StoreCorrupt)
    ));
    assert_eq!(fs::read(&database).unwrap(), before);
    assert_eq!(engine.list().unwrap(), store.snapshot(None, None));
    let connection = storage::open(&engine.directory).unwrap();
    let migrated = storage::load(&connection).unwrap();
    assert_eq!(
        serde_json::to_value(&migrated).unwrap(),
        serde_json::to_value(&store).unwrap()
    );
    assert_eq!(
        CatalogEngine::new(engine.directory.clone()).list().unwrap(),
        store.snapshot(None, None)
    );
    assert_eq!(
        engine
            .remove(RemoveRequest {
                id: store.clusters[0].id.clone()
            })
            .unwrap()
            .revision,
        u64::MAX
    );
}

#[test]
fn invalid_schema_two_migration_rolls_back_unchanged() {
    let (_dir, engine, input) = fixture();
    import(&engine, &input).unwrap();
    downgrade_to_schema_two(&engine);
    let database = engine.directory.join("catalog.sqlite3");
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute("UPDATE clusters SET source_alias='relative'", [])
        .unwrap();
    drop(connection);
    let before = fs::read(&database).unwrap();
    assert_eq!(engine.list(), Err(CatalogError::StoreCorrupt));
    assert_eq!(fs::read(&database).unwrap(), before);
}

#[test]
fn tampered_legacy_pin_metadata_fails_closed() {
    for value in [
        "[\"unknown\"]",
        "[\"pods\",\"pods\"]",
        "{}",
        "null",
        "[1]",
        "not-json",
    ] {
        let (_dir, engine, input) = fixture();
        let first = import(&engine, &input).unwrap();
        let database = engine.directory.join("catalog.sqlite3");
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .execute("UPDATE clusters SET pinned_sections=?1", [value])
            .unwrap();
        drop(connection);
        let before = fs::read(&database).unwrap();
        assert_eq!(engine.list(), Err(CatalogError::StoreCorrupt));
        assert_eq!(import(&engine, &input), Err(CatalogError::StoreCorrupt));
        assert_eq!(
            engine.connection_target(&first.clusters[0].id),
            Err(CatalogError::StoreCorrupt)
        );
        assert_eq!(fs::read(&database).unwrap(), before);
    }
}

#[test]
fn legacy_pins_are_preserved_in_storage_but_never_activated_or_reconciled() {
    let (_dir, engine, input) = fixture();
    import(&engine, &input).unwrap();
    let connection = storage::open(&engine.directory).unwrap();
    let mut store = storage::load(&connection).unwrap();
    drop(connection);
    store.clusters[0].pinned = true;
    store.clusters[0].pinned_sections = vec!["overview".into(), "pods".into()];
    // Deliberately incomplete legacy ordering must not be automatically repaired.
    store.pin_order = vec![PinTarget {
        id: store.clusters[0].id.clone(),
        section: Some("pods".into()),
    }];
    save_fixture(&engine, &store);
    let database = engine.directory.join("catalog.sqlite3");
    let before = fs::read(&database).unwrap();
    let snapshot = engine.list().unwrap();
    assert_eq!(snapshot.revision, store.revision);
    assert!(!snapshot.clusters[0].pinned);
    assert!(snapshot.clusters[0].pinned_sections.is_empty());
    assert!(snapshot.pinned_items.is_empty());
    let serialized = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(serialized["clusters"][0]["pinned"], false);
    assert_eq!(
        serialized["clusters"][0]["pinnedSections"],
        serde_json::json!([])
    );
    assert_eq!(serialized["pinnedItems"], serde_json::json!([]));
    assert_eq!(import(&engine, &input).unwrap(), snapshot);
    assert_eq!(
        CatalogEngine::new(engine.directory.clone()).list().unwrap(),
        snapshot
    );
    assert_eq!(fs::read(&database).unwrap(), before);
    // A real metadata update also preserves inert legacy data without deriving pins.
    add(&engine, "additional").unwrap();
    let connection = storage::open(&engine.directory).unwrap();
    let loaded = storage::load(&connection).unwrap();
    let retained = loaded
        .clusters
        .iter()
        .find(|row| row.id == store.clusters[0].id)
        .unwrap();
    assert!(retained.pinned);
    assert_eq!(retained.pinned_sections, store.clusters[0].pinned_sections);
    assert_eq!(loaded.pin_order, store.pin_order);
}
