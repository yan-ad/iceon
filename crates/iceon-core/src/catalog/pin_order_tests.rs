use super::{tests::*, *};

fn target(id: &str, section: Option<&str>) -> PinTarget {
    PinTarget {
        id: id.into(),
        section: section.map(str::to_owned),
    }
}

#[test]
fn schema_four_discards_retired_links_and_pins_preserving_cluster_order_and_metadata() {
    let (_dir, engine, input) = fixture();
    let cluster = import(&engine, &input).unwrap().clusters[0].id.clone();
    let additional = add(&engine, "additional").unwrap().clusters[0].id.clone();
    entry(&engine, &cluster, true);
    entry(&engine, &additional, true);
    section(&engine, &cluster, "pods", true);
    let desired = vec![
        target(&cluster, Some("pods")),
        target(&additional, None),
        target(&cluster, None),
    ];
    let before = reorder(&engine, desired.clone()).unwrap();
    let preferences = crate::preferences::PreferencesEngine::new(engine.directory.clone()).unwrap();
    let mut settings = preferences.snapshot().unwrap();
    settings.preferences.app.persistent_search = true;
    let saved_preferences = preferences
        .save(crate::preferences::SavePreferencesRequest {
            revision: settings.revision,
            preferences: settings.preferences,
        })
        .unwrap();
    drop(preferences);
    let database = engine.directory.join("catalog.sqlite3");
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection.execute_batch("
        ALTER TABLE metadata RENAME TO metadata_old;
        CREATE TABLE metadata (singleton INTEGER PRIMARY KEY CHECK(singleton=1), schema_version INTEGER NOT NULL CHECK(schema_version=4), revision TEXT NOT NULL, auto_discover INTEGER NOT NULL CHECK(auto_discover IN (0,1))) STRICT;
        INSERT INTO metadata SELECT singleton,4,revision,1 FROM metadata_old;
        DROP TABLE metadata_old;
        CREATE TABLE web_links (id TEXT PRIMARY KEY, name TEXT, url TEXT, pinned INTEGER, color TEXT);
        INSERT INTO web_links VALUES ('retired-link',NULL,'not a URL',99,'not a color');
        UPDATE pin_order SET position=position+10;
        INSERT INTO pin_order VALUES(0,'retired-link',NULL);
        INSERT INTO pin_order VALUES(1,'retired-corrupt',42);
    ").unwrap();
    drop(connection);
    let migrated = engine.list().unwrap();
    assert_eq!(migrated, before);
    assert_eq!(migrated.pinned_items, desired);
    let connection = storage::open(&engine.directory).unwrap();
    let stored = storage::load(&connection).unwrap();
    assert_eq!(stored.schema_version, 5);
    assert!(stored.auto_discover);
    assert_eq!(
        crate::preferences::PreferencesEngine::new(engine.directory.clone())
            .unwrap()
            .snapshot()
            .unwrap(),
        saved_preferences
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='web_links'",
                [],
                |row| row.get::<_, u32>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM pin_order WHERE id LIKE 'retired%'",
                [],
                |row| row.get::<_, u32>(0)
            )
            .unwrap(),
        0
    );
    drop(connection);
    assert_eq!(
        CatalogEngine::new(engine.directory.clone()).list().unwrap(),
        before
    );
    for id in ["retired-link", "retired-corrupt"] {
        assert_eq!(
            engine.set_pin(PinRequest {
                id: id.into(),
                pinned: true
            }),
            Err(CatalogError::NotFound)
        );
        assert_eq!(
            engine.remove(RemoveRequest { id: id.into() }),
            Err(CatalogError::NotFound)
        );
        assert_eq!(
            engine.validate_pin_target(&target(id, None)),
            Err(CatalogError::NotFound)
        );
    }
}
fn section(engine: &CatalogEngine, id: &str, value: &str, pinned: bool) -> CatalogSnapshot {
    engine
        .set_section_pin(SectionPinRequest {
            cluster_id: id.into(),
            section: value.into(),
            pinned,
        })
        .unwrap()
}
fn entry(engine: &CatalogEngine, id: &str, pinned: bool) -> CatalogSnapshot {
    engine
        .set_pin(PinRequest {
            id: id.into(),
            pinned,
        })
        .unwrap()
}
fn reorder(engine: &CatalogEngine, items: Vec<PinTarget>) -> Result<CatalogSnapshot> {
    engine.reorder_pins(ReorderPinsRequest { items })
}

#[test]
fn mixed_order_restart_unpin_readd_and_noop_without_discovery() {
    let (dir, engine, input) = fixture();
    let cluster = import(&engine, &input).unwrap().clusters[0].id.clone();
    let additional = add(&engine, "additional").unwrap().clusters[0].id.clone();
    section(&engine, &cluster, "pods", true);
    entry(&engine, &additional, true);
    let pinned = entry(&engine, &cluster, true);
    let initial = vec![
        target(&cluster, Some("pods")),
        target(&additional, None),
        target(&cluster, None),
    ];
    assert_eq!(pinned.pinned_items, initial);
    let desired = vec![initial[1].clone(), initial[2].clone(), initial[0].clone()];
    let home = dir.path().join("home");
    fs::create_dir_all(home.join(".kube")).unwrap();
    fs::write(
        home.join(".kube/config"),
        CONFIG.replace("development", "unexpected"),
    )
    .unwrap();
    fs::remove_file(input).unwrap();
    let engine = CatalogEngine::new_with_home(engine.directory.clone(), Some(home));
    let ordered = reorder(&engine, desired.clone()).unwrap();
    assert_eq!(ordered.revision, pinned.revision + 1);
    assert_eq!(ordered.clusters.len(), 2);
    assert_eq!(ordered.pinned_items, desired);
    let database = engine.directory.join("catalog.sqlite3");
    let bytes = fs::read(&database).unwrap();
    let modified = fs::metadata(&database).unwrap().modified().unwrap();
    assert_eq!(reorder(&engine, desired.clone()).unwrap(), ordered);
    for item in &desired {
        engine.validate_pin_target(item).unwrap();
    }
    assert_eq!(fs::read(&database).unwrap(), bytes);
    assert_eq!(
        fs::metadata(&database).unwrap().modified().unwrap(),
        modified
    );
    let restarted = CatalogEngine::new(engine.directory.clone());
    assert_eq!(restarted.list().unwrap(), ordered);
    entry(&restarted, &additional, false);
    assert_eq!(
        engine.validate_pin_target(&target(&additional, None)),
        Err(CatalogError::NotFound)
    );
    let repinned = entry(&restarted, &additional, true);
    assert_eq!(
        repinned.pinned_items,
        vec![desired[1].clone(), desired[2].clone(), desired[0].clone()]
    );
    section(&restarted, &cluster, "pods", false);
    let repinned = section(&restarted, &cluster, "pods", true);
    assert_eq!(
        repinned.pinned_items.last(),
        Some(&target(&cluster, Some("pods")))
    );
}

#[test]
fn malformed_stale_permutations_are_atomic() {
    let (_dir, engine, input) = fixture();
    let cluster = import(&engine, &input).unwrap().clusters[0].id.clone();
    let pinned = entry(&engine, &cluster, true);
    let item = target(&cluster, None);
    let database = engine.directory.join("catalog.sqlite3");
    let bytes = fs::read(&database).unwrap();
    for items in [
        vec![],
        vec![item.clone(), item.clone()],
        vec![target("missing", None)],
        vec![target(&cluster, Some("overview"))],
        vec![target(&cluster, Some("pods"))],
        vec![target(&cluster, Some("unknown"))],
        vec![target("", None)],
    ] {
        assert_eq!(reorder(&engine, items), Err(CatalogError::InvalidInput));
        assert_eq!(fs::read(&database).unwrap(), bytes);
    }
    assert_eq!(engine.list().unwrap(), pinned);
    section(&engine, &cluster, "pods", true);
    assert_eq!(
        reorder(&engine, vec![item]),
        Err(CatalogError::InvalidInput)
    );
    for value in [
        serde_json::json!({"id":cluster,"section":null,"extra":true}),
        serde_json::json!({"id":4}),
    ] {
        assert!(serde_json::from_value::<PinTarget>(value).is_err());
    }
    assert!(
        serde_json::from_value::<ReorderPinsRequest>(serde_json::json!({"items":[],"extra":true}))
            .is_err()
    );
}

#[test]
fn deletion_and_reimport_remove_only_obsolete_pins() {
    let (_dir, engine, input) = fixture();
    let cluster = import(&engine, &input).unwrap().clusters[0].id.clone();
    let additional = add(&engine, "additional").unwrap().clusters[0].id.clone();
    entry(&engine, &cluster, true);
    entry(&engine, &additional, true);
    let pinned = section(&engine, &cluster, "pods", true);
    assert_eq!(import(&engine, &input).unwrap(), pinned);
    fs::write(&input, CONFIG.replace("development", "production")).unwrap();
    let replaced = import(&engine, &input).unwrap();
    assert_eq!(replaced.pinned_items, vec![target(&additional, None)]);
    fs::write(&input, CONFIG).unwrap();
    assert_eq!(
        import(&engine, &input).unwrap().pinned_items,
        replaced.pinned_items
    );
    entry(&engine, &cluster, true);
    engine.remove(RemoveRequest { id: additional }).unwrap();
    assert_eq!(
        engine.list().unwrap().pinned_items,
        vec![target(&cluster, None)]
    );
    engine.remove(RemoveRequest { id: cluster }).unwrap();
    assert!(import(&engine, &input).unwrap().pinned_items.is_empty());
}

fn downgrade_to_three(engine: &CatalogEngine) {
    let connection = rusqlite::Connection::open(engine.directory.join("catalog.sqlite3")).unwrap();
    connection.execute_batch("
        DROP TABLE pin_order;
        ALTER TABLE metadata RENAME TO metadata_old;
        CREATE TABLE metadata (singleton INTEGER PRIMARY KEY CHECK(singleton=1), schema_version INTEGER NOT NULL CHECK(schema_version=3), revision TEXT NOT NULL, auto_discover INTEGER NOT NULL CHECK(auto_discover IN (0,1))) STRICT;
        INSERT INTO metadata SELECT singleton,3,revision,auto_discover FROM metadata_old;
        DROP TABLE metadata_old;
    ").unwrap();
}

#[test]
fn schema_three_upgrade_restores_old_sidebar_order_and_normalizes_overview() {
    let (_dir, engine, input) = fixture();
    fs::write(
        &input,
        CONFIG.replace(
            "contexts:\n",
            "contexts:\n- name: zebra\n  context: {cluster: fixture}\n",
        ),
    )
    .unwrap();
    let clusters = import(&engine, &input).unwrap().clusters;
    let additional = add(&engine, "zzzz").unwrap().clusters[2].id.clone();
    for cluster in clusters.iter().rev() {
        section(&engine, &cluster.id, "nodes", true);
        section(&engine, &cluster.id, "pods", true);
        section(&engine, &cluster.id, "config", true);
    }
    entry(&engine, &additional, true);
    let before = entry(&engine, &clusters[1].id, true);
    downgrade_to_three(&engine);
    let connection = rusqlite::Connection::open(engine.directory.join("catalog.sqlite3")).unwrap();
    connection.execute("UPDATE clusters SET pinned_sections='[\"overview\",\"config\",\"pods\",\"nodes\"]' WHERE id=?1", [&clusters[0].id]).unwrap();
    drop(connection);
    let migrated = engine.list().unwrap();
    let mut expected = vec![
        target(&clusters[0].id, None),
        target(&clusters[1].id, None),
        target(&additional, None),
    ];
    for cluster in &clusters {
        for section in ["nodes", "pods", "config"] {
            expected.push(target(&cluster.id, Some(section)));
        }
    }
    assert_eq!(migrated.revision, before.revision);
    assert_eq!(migrated.pinned_items, expected);
    assert_eq!(
        CatalogEngine::new(engine.directory.clone()).list().unwrap(),
        migrated
    );
}

#[test]
fn invalid_schema_three_rows_roll_back_before_ddl() {
    let (_dir, engine, input) = fixture();
    import(&engine, &input).unwrap();
    downgrade_to_three(&engine);
    let database = engine.directory.join("catalog.sqlite3");
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute("UPDATE clusters SET pinned_sections='[\"unknown\"]'", [])
        .unwrap();
    drop(connection);
    let before = fs::read(&database).unwrap();
    assert_eq!(engine.list(), Err(CatalogError::StoreCorrupt));
    assert_eq!(fs::read(database).unwrap(), before);
}

#[test]
fn schema_four_failed_save_rolls_back_link_deletion_and_schema_changes() {
    let (_dir, engine, input) = fixture();
    import(&engine, &input).unwrap();
    let database = engine.directory.join("catalog.sqlite3");
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection.execute_batch("
        ALTER TABLE metadata RENAME TO metadata_old;
        CREATE TABLE metadata (singleton INTEGER PRIMARY KEY CHECK(singleton=1), schema_version INTEGER NOT NULL CHECK(schema_version=4), revision TEXT NOT NULL, auto_discover INTEGER NOT NULL CHECK(auto_discover IN (0,1))) STRICT;
        INSERT INTO metadata SELECT singleton,4,revision,auto_discover FROM metadata_old;
        DROP TABLE metadata_old;
        CREATE TABLE web_links (id TEXT PRIMARY KEY);
        INSERT INTO web_links VALUES ('retired-link');
        CREATE TRIGGER reject_cluster_save BEFORE DELETE ON clusters BEGIN SELECT RAISE(ABORT,'fixture'); END;
    ").unwrap();
    drop(connection);
    let before = fs::read(&database).unwrap();
    assert!(engine.list().is_err());
    assert_eq!(fs::read(&database).unwrap(), before);
    let connection = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT schema_version FROM metadata", [], |r| r
                .get::<_, u32>(0))
            .unwrap(),
        4
    );
    assert_eq!(
        connection
            .query_row("SELECT id FROM web_links", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "retired-link"
    );
}
