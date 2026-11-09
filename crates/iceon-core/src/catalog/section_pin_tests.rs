use super::{tests::*, *};

fn pin(engine: &CatalogEngine, id: &str, section: &str, pinned: bool) -> Result<CatalogSnapshot> {
    engine.set_section_pin(SectionPinRequest {
        cluster_id: id.into(),
        section: section.into(),
        pinned,
    })
}

#[test]
fn overview_and_catalog_pin_share_one_identity_and_upgrade_old_duplicates() {
    let (_dir, engine, input) = fixture();
    let initial = import(&engine, &input).unwrap();
    let id = &initial.clusters[0].id;
    let pinned = pin(&engine, id, "overview", true).unwrap();
    assert!(pinned.clusters[0].pinned);
    assert!(pinned.clusters[0].pinned_sections.is_empty());
    assert_eq!(
        engine
            .set_pin(PinRequest {
                id: id.clone(),
                pinned: true
            })
            .unwrap(),
        pinned
    );
    let unpinned = engine
        .set_pin(PinRequest {
            id: id.clone(),
            pinned: false,
        })
        .unwrap();
    assert!(!unpinned.clusters[0].pinned);
    assert_eq!(pin(&engine, id, "overview", false).unwrap(), unpinned);

    for catalog_pin in [false, true] {
        let connection =
            rusqlite::Connection::open(engine.directory.join("catalog.sqlite3")).unwrap();
        connection
            .execute(
                "UPDATE clusters SET pinned=?1, pinned_sections='[\"overview\",\"pods\"]'",
                [catalog_pin],
            )
            .unwrap();
        drop(connection);
        let loaded = engine.list().unwrap();
        assert!(loaded.clusters[0].pinned);
        assert_eq!(loaded.clusters[0].pinned_sections, ["pods"]);
        let unpinned = engine
            .set_pin(PinRequest {
                id: id.clone(),
                pinned: false,
            })
            .unwrap();
        assert!(!unpinned.clusters[0].pinned);
        assert_eq!(
            CatalogEngine::new(engine.directory.clone()).list().unwrap(),
            unpinned
        );
    }
    assert_eq!(
        pin(&engine, "retired-link", "overview", true),
        Err(CatalogError::NotFound)
    );
}

#[test]
fn section_ids_match_static_navigation_and_request_contract() {
    let navigation = include_str!("../../../src/app/cluster/navigation.ts");
    let ids: HashSet<_> = navigation
        .split("id: \"")
        .skip(1)
        .map(|tail| tail.split('"').next().unwrap())
        .collect();
    assert_eq!(ids, SECTIONS.iter().copied().collect());
    assert_eq!(SECTIONS.len(), ids.len());
    let request: SectionPinRequest = serde_json::from_value(serde_json::json!({
        "clusterId": "cluster-fixture", "section": "pods", "pinned": true
    }))
    .unwrap();
    assert_eq!(request.cluster_id, "cluster-fixture");
    for request in [
        serde_json::json!({"cluster_id": "fixture", "section": "pods", "pinned": true}),
        serde_json::json!({"clusterId": "fixture", "section": "pods"}),
        serde_json::json!({"clusterId": "fixture", "section": "pods", "pinned": true, "extra": true}),
    ] {
        assert!(serde_json::from_value::<SectionPinRequest>(request).is_err());
    }
}

#[test]
fn section_pins_persist_with_idempotent_revisions_and_without_kubeconfig_reads() {
    let (dir, engine, input) = fixture();
    let first = import(&engine, &input).unwrap();
    let id = &first.clusters[0].id;
    assert!(first.clusters[0].pinned_sections.is_empty());
    let home = dir.path().join("home");
    fs::create_dir_all(home.join(".kube")).unwrap();
    fs::write(
        home.join(".kube/config"),
        CONFIG.replace("development", "unwanted"),
    )
    .unwrap();
    let engine = CatalogEngine::new_with_home(engine.directory.clone(), Some(home));
    fs::remove_file(&input).unwrap();
    let pinned = pin(&engine, id, "pods", true).unwrap();
    assert_eq!(pinned.revision, first.revision + 1);
    assert_eq!(pinned.clusters.len(), 1);
    assert_eq!(pinned.clusters[0].pinned_sections, ["pods"]);
    assert!(!pinned.clusters[0].pinned);
    let database = engine.directory.join("catalog.sqlite3");
    let bytes = fs::read(&database).unwrap();
    assert_eq!(pin(&engine, id, "pods", true).unwrap(), pinned);
    assert_eq!(pin(&engine, id, "nodes", false).unwrap(), pinned);
    assert_eq!(fs::read(&database).unwrap(), bytes);
    let restarted = CatalogEngine::new(engine.directory.clone());
    assert_eq!(restarted.list().unwrap(), pinned);
    assert_eq!(
        restarted.connection_target(id).unwrap().context_name,
        "development"
    );
    let unpinned = pin(&restarted, id, "pods", false).unwrap();
    assert_eq!(unpinned.revision, pinned.revision + 1);
    assert!(unpinned.clusters[0].pinned_sections.is_empty());
    assert_eq!(pin(&restarted, id, "pods", false).unwrap(), unpinned);
    assert_eq!(
        CatalogEngine::new(engine.directory.clone()).list().unwrap(),
        unpinned
    );
}

#[test]
fn section_pin_validation_rejects_nonclusters_and_unknown_sections() {
    let (_dir, engine, input) = fixture();
    let first = import(&engine, &input).unwrap();
    for (id, error) in [
        ("".into(), CatalogError::InvalidInput),
        (" padded ".into(), CatalogError::InvalidInput),
        ("bad\0id".into(), CatalogError::InvalidInput),
        ("x".repeat(MAX_NAME + 1), CatalogError::InvalidInput),
        ("missing".into(), CatalogError::NotFound),
        ("retired-link".into(), CatalogError::NotFound),
    ] {
        assert_eq!(pin(&engine, &id, "pods", true), Err(error));
    }
    for section in [
        "",
        "Pods",
        " pods",
        "pods/",
        "custom-resource:example.test/widgets",
        "pods\0",
        "catalog",
    ] {
        for pinned in [true, false] {
            assert_eq!(
                pin(&engine, &first.clusters[0].id, section, pinned),
                Err(CatalogError::InvalidInput)
            );
        }
    }
    assert_eq!(engine.list().unwrap(), first);
}

#[test]
fn all_static_sections_are_bounded_and_reimport_preserves_only_surviving_cluster_pins() {
    let (_dir, engine, input) = fixture();
    let first = import(&engine, &input).unwrap();
    let id = &first.clusters[0].id;
    for section in SECTIONS {
        pin(&engine, id, section, true).unwrap();
    }
    let pinned = engine.list().unwrap();
    assert_eq!(pinned.clusters[0].pinned_sections.len(), SECTIONS.len() - 1);
    assert!(pinned.clusters[0].pinned);
    assert_eq!(import(&engine, &input).unwrap(), pinned);
    for section in SECTIONS {
        assert_eq!(pin(&engine, id, section, true).unwrap(), pinned);
    }
    fs::write(&input, CONFIG.replace("development", "production")).unwrap();
    let replaced = import(&engine, &input).unwrap();
    assert!(replaced.clusters[0].pinned_sections.is_empty());
    assert_eq!(pin(&engine, id, "pods", true), Err(CatalogError::NotFound));
    let new_id = &replaced.clusters[0].id;
    pin(&engine, new_id, "overview", true).unwrap();
    engine.remove(RemoveRequest { id: new_id.clone() }).unwrap();
    assert!(engine.list().unwrap().clusters.is_empty());
    assert!(
        import(&engine, &input).unwrap().clusters[0]
            .pinned_sections
            .is_empty()
    );
}

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
    store.reconcile_pins();
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
        pin(&engine, &store.clusters[0].id, "nodes", true)
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
fn tampered_and_duplicate_section_pins_fail_closed_for_reads_and_mutations() {
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
        assert_eq!(
            pin(&engine, &first.clusters[0].id, "pods", false),
            Err(CatalogError::StoreCorrupt)
        );
        assert_eq!(import(&engine, &input), Err(CatalogError::StoreCorrupt));
        assert_eq!(
            engine.connection_target(&first.clusters[0].id),
            Err(CatalogError::StoreCorrupt)
        );
        assert_eq!(fs::read(&database).unwrap(), before);
    }
}
