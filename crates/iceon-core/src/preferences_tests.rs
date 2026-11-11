use super::*;
use tempfile::tempdir;

// Compile resolver tests before its public module is wired by the application.
#[path = "kubernetes_tools.rs"]
mod kubernetes_tools;

#[test]
fn legacy_kubernetes_preferences_migrate_with_safe_defaults() {
    let dir = tempdir().unwrap();
    let engine = PreferencesEngine::new(dir.path().to_path_buf()).unwrap();
    let mut payload = serde_json::to_value(Preferences::default()).unwrap();
    payload["kubernetes"] = serde_json::json!({
        "helmBinaryPath": "", "helmServerSide": false, "discoverDefaultKubeconfig": true
    });
    engine
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE preferences_store SET payload = ?",
            params![payload.to_string()],
        )
        .unwrap();
    let mut snapshot = engine.snapshot().unwrap();
    assert!(!snapshot.preferences.kubernetes.kubectl_auto_download);
    assert_eq!(
        snapshot.preferences.kubernetes.kubectl_download_mirror,
        DownloadMirror::Default
    );
    assert!(snapshot.preferences.kubernetes.helm_repositories.is_empty());
    snapshot
        .preferences
        .kubernetes
        .kubeconfig_sync_paths
        .push("/tmp/config".into());
    snapshot
        .preferences
        .kubernetes
        .helm_repositories
        .push(HelmRepository {
            name: "stable".into(),
            url: "https://charts.example.com".into(),
        });
    let saved = engine
        .save(SavePreferencesRequest {
            revision: snapshot.revision,
            preferences: snapshot.preferences,
        })
        .unwrap();
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
fn tray_default_preserves_explicit_opt_out_and_accepts_missing_field() {
    let mut json = serde_json::to_value(AppPreferences::default()).unwrap();
    json.as_object_mut().unwrap().remove("showTrayIcon");
    let migrated: AppPreferences = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(migrated.show_tray_icon, cfg!(target_os = "macos"));
    json["showTrayIcon"] = serde_json::json!(false);
    let existing: AppPreferences = serde_json::from_value(json).unwrap();
    assert!(!existing.show_tray_icon);
}

#[test]
fn validates_kubernetes_options_before_persistence() {
    let mut prefs = Preferences::default();
    prefs.kubernetes.kubectl_download_mirror = DownloadMirror::Custom;
    for invalid in [
        "",
        "http://example.com",
        "file:///etc/passwd",
        "https://user:pass@example.com",
        "https://example.com?q=x",
    ] {
        prefs.kubernetes.kubectl_mirror_url = invalid.into();
        assert_eq!(validate(&prefs), Err(PreferencesError::InvalidInput));
    }
    prefs.kubernetes.kubectl_mirror_url = "http://127.0.0.1:1234/tools".into();
    assert_eq!(validate(&prefs), Ok(()));
    prefs
        .kubernetes
        .kubeconfig_sync_paths
        .push("bad\0path".into());
    assert_eq!(validate(&prefs), Err(PreferencesError::InvalidInput));
}

#[test]
fn default_preferences_match_specification() {
    let dir = tempdir().unwrap();
    let engine = PreferencesEngine::new(dir.path().to_path_buf()).unwrap();
    let snap = engine.snapshot().unwrap();
    assert_eq!(snap.revision, 1);
    assert_eq!(snap.preferences.app.theme, AppTheme::System);
    assert_eq!(snap.preferences.app.timezone, "system");
    assert!(!snap.preferences.app.persistent_search);
    assert_eq!(
        snap.preferences.app.show_tray_icon,
        cfg!(target_os = "macos")
    );
    assert_eq!(snap.preferences.editor.font_size, 12);
    assert_eq!(snap.preferences.editor.tab_size, 2);
    assert_eq!(snap.preferences.editor.line_numbers, LineNumbers::On);
    assert_eq!(snap.preferences.terminal.font_size, 12);
    assert_eq!(snap.preferences.terminal.theme, TerminalTheme::Match);
    assert!(!snap.preferences.terminal.copy_on_select);
}

#[test]
fn save_updates_revision_and_persists() {
    let dir = tempdir().unwrap();
    let engine = PreferencesEngine::new(dir.path().to_path_buf()).unwrap();
    let mut snap = engine.snapshot().unwrap();
    snap.preferences.app.theme = AppTheme::Dark;
    snap.preferences.editor.font_size = 14;
    snap.preferences.terminal.copy_on_select = true;

    let saved = engine
        .save(SavePreferencesRequest {
            revision: snap.revision,
            preferences: snap.preferences.clone(),
        })
        .unwrap();

    assert_eq!(saved.revision, 2);
    assert_eq!(saved.preferences.app.theme, AppTheme::Dark);
    assert_eq!(saved.preferences.editor.font_size, 14);

    // Re-open from disk
    let engine2 = PreferencesEngine::new(dir.path().to_path_buf()).unwrap();
    let snap2 = engine2.snapshot().unwrap();
    assert_eq!(snap2.revision, 2);
    assert_eq!(snap2.preferences.app.theme, AppTheme::Dark);
    assert_eq!(snap2.preferences.editor.font_size, 14);
    assert!(snap2.preferences.terminal.copy_on_select);
}

#[test]
fn conflict_on_stale_revision() {
    let dir = tempdir().unwrap();
    let engine = PreferencesEngine::new(dir.path().to_path_buf()).unwrap();
    let snap = engine.snapshot().unwrap();

    let err = engine
        .save(SavePreferencesRequest {
            revision: snap.revision + 1,
            preferences: snap.preferences,
        })
        .unwrap_err();

    assert_eq!(err, PreferencesError::Conflict);
}

#[test]
fn invalid_preferences_rejected() {
    let dir = tempdir().unwrap();
    let engine = PreferencesEngine::new(dir.path().to_path_buf()).unwrap();
    let mut snap = engine.snapshot().unwrap();

    // Invalid editor font size < 10
    snap.preferences.editor.font_size = 5;
    assert_eq!(
        engine
            .save(SavePreferencesRequest {
                revision: snap.revision,
                preferences: snap.preferences.clone(),
            })
            .unwrap_err(),
        PreferencesError::InvalidInput
    );

    // Invalid editor font size > 50
    snap.preferences.editor.font_size = 60;
    assert_eq!(
        engine
            .save(SavePreferencesRequest {
                revision: snap.revision,
                preferences: snap.preferences.clone(),
            })
            .unwrap_err(),
        PreferencesError::InvalidInput
    );

    // Reset font size, test invalid proxy
    snap.preferences.editor.font_size = 12;
    snap.preferences.proxy.https_proxy = "not-a-valid-url".into();
    assert_eq!(
        engine
            .save(SavePreferencesRequest {
                revision: snap.revision,
                preferences: snap.preferences.clone(),
            })
            .unwrap_err(),
        PreferencesError::InvalidInput
    );
}

#[test]
fn corrupt_store_fails_closed() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("preferences.sqlite3");
    std::fs::write(&db_path, b"not a sqlite database").unwrap();

    let res = PreferencesEngine::new(dir.path().to_path_buf());
    // Either fails on new or snapshot
    if let Ok(engine) = res {
        assert!(engine.snapshot().is_err());
    }
}

#[test]
fn legacy_app_preferences_default_auto_hide_hotbar_to_false() {
    let mut json = serde_json::to_value(AppPreferences::default()).unwrap();
    json.as_object_mut().unwrap().remove("autoHideHotbar");
    let app: AppPreferences = serde_json::from_value(json).unwrap();
    assert!(!app.auto_hide_hotbar);
}

#[test]
fn integrated_save_rejects_failure_without_persisting_and_rolls_back_store_failure() {
    use app_integration::{AppIntegration, IntegrationError};
    #[derive(Default)]
    struct Fake {
        enabled: bool,
        calls: usize,
        fail: bool,
    }
    impl AppIntegration for Fake {
        fn apply(
            &mut self,
            _: &AppPreferences,
            next: &AppPreferences,
        ) -> std::result::Result<(), IntegrationError> {
            self.calls += 1;
            if self.fail {
                return Err(IntegrationError::Unsupported("test"));
            }
            self.enabled = next.open_at_login;
            Ok(())
        }
    }
    let dir = tempdir().unwrap();
    let engine = PreferencesEngine::new(dir.path().into()).unwrap();
    let original = engine.snapshot().unwrap();
    let mut next = original.preferences.clone();
    next.app.open_at_login = true;
    let mut fake = Fake {
        fail: true,
        ..Fake::default()
    };
    let request = || SavePreferencesRequest {
        revision: original.revision,
        preferences: next.clone(),
    };
    assert!(
        engine
            .save_with_app_integration(request(), &mut fake)
            .is_err()
    );
    assert_eq!(engine.snapshot().unwrap(), original);
    fake.fail = false;
    engine.connection.lock().unwrap().execute_batch("CREATE TRIGGER fail_save BEFORE UPDATE ON preferences_store BEGIN SELECT RAISE(ABORT, 'test failure'); END;").unwrap();
    assert!(
        engine
            .save_with_app_integration(request(), &mut fake)
            .is_err()
    );
    assert!(!fake.enabled);
    assert_eq!(fake.calls, 3);
    assert_eq!(engine.snapshot().unwrap(), original);
    engine
        .connection
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_save;")
        .unwrap();
    let saved = engine
        .save_with_app_integration(request(), &mut fake)
        .unwrap();
    assert!(fake.enabled);
    assert!(saved.preferences.app.open_at_login);
    let calls = fake.calls;
    assert!(matches!(
        engine.save_with_app_integration(request(), &mut fake),
        Err(ApplyPreferencesError::Store(PreferencesError::Conflict))
    ));
    assert_eq!(fake.calls, calls);
}
