//! Frozen 0.1.2 persistence compatibility, independent of current default/schema generation.
use crate::{
    catalog::{CatalogEngine, RemoveRequest},
    preferences::{PreferencesEngine, PreferencesError, SavePreferencesRequest},
    route_history::SavedRoute,
};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;

const DEFAULTS: &str = include_str!("fixtures/upgrade_0_1_2/preferences-default-macos.json");
const CUSTOM: &str = include_str!("fixtures/upgrade_0_1_2/preferences-custom.json");
const PREF_SCHEMA: &str = include_str!("fixtures/upgrade_0_1_2/preferences.sql");
const CATALOG_SCHEMA: &str = include_str!("fixtures/upgrade_0_1_2/catalog.sql");

fn seed_preferences(dir: &Path, payload: &str, route: Option<&str>) {
    let db = Connection::open(dir.join("preferences.sqlite3")).unwrap();
    db.execute_batch(PREF_SCHEMA).unwrap();
    db.execute(
        "INSERT INTO preferences_store VALUES (1,1,37,?1)",
        [payload],
    )
    .unwrap();
    if let Some(route) = route {
        db.execute("INSERT INTO route_history VALUES (1,?1)", [route])
            .unwrap();
    }
}

fn expected_preferences(payload: &str) -> Value {
    let mut expected: Value = serde_json::from_str(payload).unwrap();
    let app = expected["app"].as_object_mut().unwrap();
    // Intentional retirements: hotbar auto-hide removed; bundled themes fall back
    // to system. The new external Zed selection has no legacy equivalent.
    app.remove("autoHideHotbar");
    if !matches!(app["theme"].as_str().unwrap(), "system" | "dark" | "light") {
        app.insert("theme".into(), json!("system"));
    }
    app.insert("zedTheme".into(), Value::Null);
    app.insert("fontFamily".into(), json!(".SystemUIFont"));
    expected
}

#[test]
fn upgrade_0_1_2_preferences_preserve_every_nonretired_field_and_revision() {
    // macOS shipped true; the same release serialized false on Linux/Windows.
    let mut other_platform: Value = serde_json::from_str(DEFAULTS).unwrap();
    other_platform["app"]["showTrayIcon"] = json!(false);
    for payload in [
        DEFAULTS.to_owned(),
        other_platform.to_string(),
        CUSTOM.to_owned(),
    ] {
        let dir = tempfile::tempdir().unwrap();
        seed_preferences(dir.path(), &payload, None);
        let engine = PreferencesEngine::new(dir.path().into()).unwrap();
        let snapshot = engine.snapshot().unwrap();
        assert_eq!(snapshot.revision, 37);
        assert_eq!(
            serde_json::to_value(&snapshot.preferences).unwrap(),
            expected_preferences(&payload)
        );
        // Opening/snapshotting must not silently replace the historical payload.
        let db = Connection::open(dir.path().join("preferences.sqlite3")).unwrap();
        let stored: String = db
            .query_row("SELECT payload FROM preferences_store", [], |r| r.get(0))
            .unwrap();
        assert_eq!(stored, payload);
        drop(db);
        let saved = engine
            .save(SavePreferencesRequest {
                revision: 37,
                preferences: snapshot.preferences.clone(),
            })
            .unwrap();
        assert_eq!(saved.revision, 38);
        assert_eq!(
            engine.save(SavePreferencesRequest {
                revision: 37,
                preferences: snapshot.preferences,
            }),
            Err(PreferencesError::Conflict)
        );
        drop(engine);
        assert_eq!(
            PreferencesEngine::new(dir.path().into())
                .unwrap()
                .snapshot()
                .unwrap(),
            saved
        );
    }
}

#[test]
fn upgrade_0_1_2_all_shipped_theme_ids_are_readable() {
    let ids: Vec<String> =
        serde_json::from_str(include_str!("fixtures/upgrade_0_1_2/theme-ids.json")).unwrap();
    assert_eq!(ids.len(), 19);
    for id in ids {
        let dir = tempfile::tempdir().unwrap();
        let mut payload: Value = serde_json::from_str(CUSTOM).unwrap();
        payload["app"]["theme"] = json!(id);
        let payload = payload.to_string();
        seed_preferences(dir.path(), &payload, None);
        let engine = PreferencesEngine::new(dir.path().into()).unwrap();
        assert_eq!(
            serde_json::to_value(engine.snapshot().unwrap().preferences).unwrap(),
            expected_preferences(&payload)
        );
    }
}

#[test]
fn upgrade_0_1_2_first_save_with_app_integration_reads_legacy_preferences() {
    use crate::preferences::{
        AppPreferences,
        app_integration::{AppIntegration, IntegrationError},
    };
    #[derive(Default)]
    struct RecordingIntegration(Vec<(AppPreferences, AppPreferences)>);
    impl AppIntegration for RecordingIntegration {
        fn apply(
            &mut self,
            previous: &AppPreferences,
            next: &AppPreferences,
        ) -> Result<(), IntegrationError> {
            self.0.push((previous.clone(), next.clone()));
            Ok(())
        }
    }
    let dir = tempfile::tempdir().unwrap();
    seed_preferences(dir.path(), CUSTOM, None);
    let engine = PreferencesEngine::new(dir.path().into()).unwrap();
    let snapshot = engine.snapshot().unwrap();
    let previous = snapshot.preferences.app.clone();
    let mut integration = RecordingIntegration::default();
    engine.apply_app_startup(&mut integration).unwrap();
    let saved = engine
        .save_with_app_integration(
            SavePreferencesRequest {
                revision: snapshot.revision,
                preferences: snapshot.preferences,
            },
            &mut integration,
        )
        .unwrap();
    assert_eq!(saved.revision, 38);
    assert_eq!(integration.0.len(), 2);
    assert_eq!(integration.0[0].1, previous);
    assert_eq!(integration.0[1], (previous.clone(), previous));
    drop(engine);
    assert_eq!(
        PreferencesEngine::new(dir.path().into())
            .unwrap()
            .snapshot()
            .unwrap(),
        saved
    );
}

#[test]
fn upgrade_0_1_2_route_history_is_available_before_first_preferences_save() {
    let dir = tempfile::tempdir().unwrap();
    let route = r#"{"path":"/cluster/cluster-legacy","section":"custom-resource-instances","definition":"widgets.example.test"}"#;
    seed_preferences(dir.path(), CUSTOM, Some(route));
    let expected: SavedRoute = serde_json::from_str(route).unwrap();
    let engine = PreferencesEngine::new(dir.path().into()).unwrap();
    // Startup restores routes before users have saved preferences in the new app.
    assert_eq!(engine.route_history_get().unwrap(), Some(expected.clone()));
    engine.route_history_save(expected.clone()).unwrap();
    let snapshot = engine.snapshot().unwrap();
    engine
        .save(SavePreferencesRequest {
            revision: snapshot.revision,
            preferences: snapshot.preferences,
        })
        .unwrap();
    drop(engine);
    let reopened = PreferencesEngine::new(dir.path().into()).unwrap();
    assert_eq!(reopened.route_history_get().unwrap(), Some(expected));
}

#[test]
fn upgrade_0_1_2_catalog_preserves_metadata_and_inert_pins_across_real_save() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir
        .path()
        .join("missing-kubeconfig.yaml")
        .to_str()
        .unwrap()
        .to_owned();
    let alias = dir
        .path()
        .join("original-alias.yaml")
        .to_str()
        .unwrap()
        .to_owned();
    // Exact 0.1.2 catalog::digest_id algorithm, not a current engine import.
    let id = |context: &str| {
        let mut hash = Sha256::new();
        for part in [source.as_str(), context] {
            hash.update((part.len() as u64).to_le_bytes());
            hash.update(part.as_bytes());
        }
        format!("cluster-{:x}", hash.finalize())
    };
    let retained = id("production");
    let removed = id("staging");
    let db = Connection::open(dir.path().join("catalog.sqlite3")).unwrap();
    db.execute_batch(CATALOG_SCHEMA).unwrap();
    db.execute("INSERT INTO metadata VALUES(1,5,'73',0)", [])
        .unwrap();
    for (id, context, color, pinned, sections) in [
        (
            &retained,
            "production",
            "#123ABC",
            true,
            r#"["pods","nodes"]"#,
        ),
        (&removed, "staging", "#ABC123", false, "[]"),
    ] {
        db.execute(
            "INSERT INTO clusters VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![id, context, source, alias, pinned, color, sections],
        )
        .unwrap();
    }
    db.execute(
        "INSERT INTO pin_order VALUES(0,?1,NULL),(1,?1,'pods'),(2,?1,'nodes')",
        [&retained],
    )
    .unwrap();
    db.execute(
        "INSERT INTO kubeconfig_sync_ownership VALUES(?1,?2)",
        params![retained, alias],
    )
    .unwrap();
    let rows = |db: &Connection| -> Vec<Value> {
        db.prepare("SELECT id,context_name,source_path,source_alias,pinned,color,pinned_sections FROM clusters WHERE id=?1").unwrap()
            .query_map([&retained], |r| Ok(json!([r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,bool>(4)?,r.get::<_,String>(5)?,r.get::<_,String>(6)?])))
            .unwrap().map(Result::unwrap).collect()
    };
    let before = rows(&db);
    drop(db);
    let engine = CatalogEngine::new_with_home(dir.path().into(), None);
    let snapshot = engine.list().unwrap();
    assert_eq!(snapshot.revision, 73);
    assert_eq!(snapshot.clusters.len(), 2);
    assert!(snapshot.pinned_items.is_empty());
    let cluster = snapshot.clusters.iter().find(|c| c.id == retained).unwrap();
    assert_eq!(cluster.context_name, "production");
    assert_eq!(cluster.color, "#123ABC");
    assert!(!cluster.pinned);
    assert!(cluster.pinned_sections.is_empty());
    let target = engine.connection_target(&retained).unwrap();
    assert_eq!(target.source_path, Path::new(&source));
    assert_eq!(target.provenance_path, Path::new(&alias));
    assert_eq!(
        engine
            .remove(RemoveRequest { id: removed })
            .unwrap()
            .revision,
        74
    );
    let reopened = CatalogEngine::new_with_home(dir.path().into(), None)
        .list()
        .unwrap();
    assert_eq!(reopened.revision, 74);
    assert_eq!(reopened.clusters, vec![cluster.clone()]);
    let db = Connection::open(dir.path().join("catalog.sqlite3")).unwrap();
    assert_eq!(rows(&db), before);
    let pins: Vec<(i64, String, Option<String>)> = db
        .prepare("SELECT position,id,section FROM pin_order ORDER BY position")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        pins,
        vec![
            (0, retained.clone(), None),
            (1, retained.clone(), Some("pods".into())),
            (2, retained.clone(), Some("nodes".into()))
        ]
    );
    assert_eq!(
        db.query_row(
            "SELECT alias FROM kubeconfig_sync_ownership WHERE id=?1",
            [&retained],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        alias
    );
    assert_eq!(
        db.query_row(
            "SELECT schema_version,auto_discover FROM metadata",
            [],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        )
        .unwrap(),
        (5, 0)
    );
}

#[test]
fn upgrade_0_1_2_runtime_data_directory_contract_is_unchanged() {
    // Static comparison deliberately avoids unsafe process-wide environment edits
    // in parallel tests and does not read or write the user's actual data directory.
    let source = include_str!("runtime.rs");
    let start = source.find("pub fn app_data_dir() -> PathBuf {").unwrap();
    let end = start + source[start..].find("\n}\n").unwrap() + 3;
    assert_eq!(
        &source[start..end],
        include_str!("fixtures/upgrade_0_1_2/app_data_dir.rs.txt")
    );
}
