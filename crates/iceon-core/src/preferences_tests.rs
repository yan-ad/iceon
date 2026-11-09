use super::*;
use tempfile::tempdir;

#[test]
fn default_preferences_match_specification() {
    let dir = tempdir().unwrap();
    let engine = PreferencesEngine::new(dir.path().to_path_buf()).unwrap();
    let snap = engine.snapshot().unwrap();
    assert_eq!(snap.revision, 1);
    assert_eq!(snap.preferences.app.theme, AppTheme::System);
    assert_eq!(snap.preferences.app.timezone, "system");
    assert!(!snap.preferences.app.persistent_search);
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
