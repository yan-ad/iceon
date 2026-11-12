//! Persistent Rust-owned user preferences.
use rusqlite::{Connection, OpenFlags, params};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

const DATABASE: &str = "preferences.sqlite3";
const MAX_DATABASE: u64 = 8 * 1024 * 1024;
const DEFAULT_FONT_FAMILY: &str = "RobotoMono, monospace";

#[path = "app_integration.rs"]
pub mod app_integration;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PreferencesError {
    InvalidInput,
    Conflict,
    StoreUnavailable,
    StoreCorrupt,
}

// A missing preference uses the native platform default. Explicit persisted
// false remains false: an upgrade must not override the user's opt-out.
fn default_show_tray_icon() -> bool {
    cfg!(target_os = "macos")
}

#[derive(Debug, thiserror::Error)]
pub enum ApplyPreferencesError {
    #[error("preference store: {0:?}")]
    Store(PreferencesError),
    #[error(transparent)]
    Integration(#[from] app_integration::IntegrationError),
    #[error("preference store: {store:?}; OS rollback failed: {rollback}")]
    Rollback {
        store: PreferencesError,
        rollback: app_integration::IntegrationError,
    },
}

/// HTTP is accepted only for literal loopback hosts (local mirrors/tests).
pub(crate) fn secure_download_url(value: &str) -> Option<url::Url> {
    let u = url::Url::parse(value).ok()?;
    let loopback = match u.host()? {
        url::Host::Ipv4(ip) => ip.is_loopback(),
        url::Host::Ipv6(ip) => ip.is_loopback(),
        url::Host::Domain(host) => host.eq_ignore_ascii_case("localhost"),
    };
    (value.len() <= 4096
        && !value.chars().any(char::is_control)
        && (u.scheme() == "https" || (u.scheme() == "http" && loopback))
        && u.username().is_empty()
        && u.password().is_none()
        && u.query().is_none()
        && u.fragment().is_none())
    .then_some(u)
}

pub type Result<T> = std::result::Result<T, PreferencesError>;

// Theme identities and palette roles are generated from themes/*.json.
include!(concat!(env!("OUT_DIR"), "/themes.rs"));

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppPreferences {
    pub theme: AppTheme,
    pub timezone: String,
    pub persistent_search: bool,
    pub open_at_login: bool,
    #[serde(default = "default_show_tray_icon")]
    pub show_tray_icon: bool,
    #[serde(default)]
    pub zed_theme: Option<String>,
    #[serde(default)]
    pub restore_last_page: bool,
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            theme: AppTheme::System,
            timezone: "system".into(),
            persistent_search: false,
            open_at_login: false,
            show_tray_icon: default_show_tray_icon(),
            zed_theme: None,
            restore_last_page: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProxyPreferences {
    pub https_proxy: String,
    pub allow_untrusted_cas: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelmRepository {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DownloadMirror {
    #[default]
    Default,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KubernetesPreferences {
    pub helm_binary_path: String,
    pub helm_server_side: bool,
    pub discover_default_kubeconfig: bool,
    #[serde(default)]
    pub kubectl_auto_download: bool,
    #[serde(default)]
    pub kubectl_download_mirror: DownloadMirror,
    #[serde(default)]
    pub kubectl_mirror_url: String,
    #[serde(default)]
    pub binaries_directory: String,
    #[serde(default)]
    pub kubectl_binary_path: String,
    #[serde(default)]
    pub kubeconfig_sync_paths: Vec<String>,
    #[serde(default)]
    pub helm_repositories: Vec<HelmRepository>,
}

impl Default for KubernetesPreferences {
    fn default() -> Self {
        Self {
            helm_binary_path: String::new(),
            helm_server_side: false,
            discover_default_kubeconfig: true,
            kubectl_auto_download: false,
            kubectl_download_mirror: DownloadMirror::Default,
            kubectl_mirror_url: String::new(),
            binaries_directory: String::new(),
            kubectl_binary_path: String::new(),
            kubeconfig_sync_paths: Vec::new(),
            helm_repositories: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum LineNumbers {
    On,
    Off,
    Relative,
    Interval,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MinimapSide {
    Left,
    Right,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditorPreferences {
    pub font_size: u8,
    pub font_family: String,
    pub tab_size: u8,
    pub line_numbers: LineNumbers,
    pub minimap: bool,
    pub minimap_side: MinimapSide,
}

impl Default for EditorPreferences {
    fn default() -> Self {
        Self {
            font_size: 12,
            font_family: DEFAULT_FONT_FAMILY.into(),
            tab_size: 2,
            line_numbers: LineNumbers::On,
            minimap: true,
            minimap_side: MinimapSide::Right,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TerminalTheme {
    Match,
    Dark,
    Light,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalPreferences {
    pub font_size: u8,
    pub font_family: String,
    pub theme: TerminalTheme,
    pub copy_on_select: bool,
    pub shell_path: String,
}

impl Default for TerminalPreferences {
    fn default() -> Self {
        Self {
            font_size: 12,
            font_family: DEFAULT_FONT_FAMILY.into(),
            theme: TerminalTheme::Match,
            copy_on_select: false,
            shell_path: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Preferences {
    pub app: AppPreferences,
    pub proxy: ProxyPreferences,
    pub kubernetes: KubernetesPreferences,
    pub editor: EditorPreferences,
    pub terminal: TerminalPreferences,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PreferencesSnapshot {
    pub revision: u64,
    pub preferences: Preferences,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavePreferencesRequest {
    pub revision: u64,
    pub preferences: Preferences,
}

fn valid_font_family(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 256
        && !s.contains('\0')
        && s.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | ',' | '_' | '\'' | '"' | '-' | '.'))
}

pub(crate) fn decode_preferences(
    payload: &str,
) -> std::result::Result<Preferences, serde_json::Error> {
    let mut value: serde_json::Value = serde_json::from_str(payload)?;
    if let Some(app) = value
        .get_mut("app")
        .and_then(serde_json::Value::as_object_mut)
    {
        // Discard the removed legacy rail setting without retaining its API.
        let legacy = ["auto", "Hide", "Hot", "bar"].concat();
        app.remove(&legacy);
        if app
            .get("theme")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|theme| !matches!(theme, "system" | "dark" | "light"))
        {
            app.insert("theme".into(), serde_json::Value::String("system".into()));
        }
    }
    serde_json::from_value(value)
}

fn validate(p: &Preferences) -> Result<()> {
    if p.app
        .zed_theme
        .as_ref()
        .is_some_and(|id| id.len() > 512 || id.chars().any(char::is_control) || !id.contains('/'))
    {
        return Err(PreferencesError::InvalidInput);
    }

    if p.app.timezone.len() > 64 || p.app.timezone.contains('\0') {
        return Err(PreferencesError::InvalidInput);
    }
    if !p.proxy.https_proxy.is_empty() {
        if p.proxy.https_proxy.len() > 2048 {
            return Err(PreferencesError::InvalidInput);
        }
        let url =
            url::Url::parse(&p.proxy.https_proxy).map_err(|_| PreferencesError::InvalidInput)?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err(PreferencesError::InvalidInput);
        }
    }
    if p.kubernetes.helm_binary_path.len() > 4096 || p.kubernetes.helm_binary_path.contains('\0') {
        return Err(PreferencesError::InvalidInput);
    }
    let k = &p.kubernetes;
    if (k.kubectl_download_mirror == DownloadMirror::Custom || !k.kubectl_mirror_url.is_empty())
        && secure_download_url(&k.kubectl_mirror_url).is_none()
    {
        return Err(PreferencesError::InvalidInput);
    }
    for value in [
        &k.kubectl_mirror_url,
        &k.binaries_directory,
        &k.kubectl_binary_path,
    ] {
        if value.len() > 4096 || value.contains('\0') {
            return Err(PreferencesError::InvalidInput);
        }
    }
    if k.kubeconfig_sync_paths.len() > 64
        || k.kubeconfig_sync_paths
            .iter()
            .any(|path| path.len() > 4096 || path.contains('\0'))
    {
        return Err(PreferencesError::InvalidInput);
    }
    if k.helm_repositories.len() > 128
        || k.helm_repositories.iter().any(|repo| {
            repo.name.is_empty()
                || repo.name.len() > 256
                || repo.name.contains('\0')
                || repo.url.len() > 4096
                || repo.url.contains('\0')
                || url::Url::parse(&repo.url)
                    .map(|u| !matches!(u.scheme(), "http" | "https") || u.host_str().is_none())
                    .unwrap_or(true)
        })
    {
        return Err(PreferencesError::InvalidInput);
    }
    if !(10..=50).contains(&p.editor.font_size)
        || !(1..=16).contains(&p.editor.tab_size)
        || !valid_font_family(&p.editor.font_family)
    {
        return Err(PreferencesError::InvalidInput);
    }
    if !(10..=50).contains(&p.terminal.font_size)
        || !valid_font_family(&p.terminal.font_family)
        || p.terminal.shell_path.len() > 4096
        || p.terminal.shell_path.contains('\0')
    {
        return Err(PreferencesError::InvalidInput);
    }
    Ok(())
}

fn check_symlink(path: &Path, directory: bool) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|_| PreferencesError::StoreUnavailable)?;
    if metadata.file_type().is_symlink()
        || (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
    {
        return Err(PreferencesError::StoreCorrupt);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(
            path,
            fs::Permissions::from_mode(if directory { 0o700 } else { 0o600 }),
        );
    }
    Ok(())
}

pub struct PreferencesEngine {
    _dir: PathBuf,
    pub(crate) connection: Mutex<Connection>,
}

impl PreferencesEngine {
    /// Startup hook: reconcile persisted settings with the actual OS. Callers
    /// must surface errors (in particular settings unsupported by this build).
    pub fn apply_app_startup(
        &self,
        integration: &mut impl app_integration::AppIntegration,
    ) -> std::result::Result<(), ApplyPreferencesError> {
        let snapshot = self.snapshot().map_err(ApplyPreferencesError::Store)?;
        integration.apply(&AppPreferences::default(), &snapshot.preferences.app)?;
        Ok(())
    }

    /// Save hook for the UI: validate/conflict-check under a SQLite transaction,
    /// apply OS effects before committing, and restore OS state on store failure.
    /// Do not use plain `save` for changes to OS preferences.
    pub fn save_with_app_integration(
        &self,
        request: SavePreferencesRequest,
        integration: &mut impl app_integration::AppIntegration,
    ) -> std::result::Result<PreferencesSnapshot, ApplyPreferencesError> {
        validate(&request.preferences).map_err(ApplyPreferencesError::Store)?;
        let mut conn = self
            .connection
            .lock()
            .map_err(|_| ApplyPreferencesError::Store(PreferencesError::StoreUnavailable))?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| ApplyPreferencesError::Store(PreferencesError::StoreUnavailable))?;
        let (revision, payload): (i64, String) = tx
            .query_row(
                "SELECT revision, payload FROM preferences_store WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| ApplyPreferencesError::Store(PreferencesError::StoreUnavailable))?;
        if revision as u64 != request.revision {
            return Err(ApplyPreferencesError::Store(PreferencesError::Conflict));
        }
        let previous: Preferences = decode_preferences(&payload)
            .map_err(|_| ApplyPreferencesError::Store(PreferencesError::StoreCorrupt))?;
        let next_revision = revision.checked_add(1).ok_or(ApplyPreferencesError::Store(
            PreferencesError::StoreUnavailable,
        ))?;
        let json = serde_json::to_string(&request.preferences)
            .map_err(|_| ApplyPreferencesError::Store(PreferencesError::InvalidInput))?;
        integration.apply(&previous.app, &request.preferences.app)?;
        let store_result = (|| {
            tx.execute(
                "UPDATE preferences_store SET revision = ?, payload = ? WHERE singleton = 1",
                params![next_revision, json],
            )
            .map_err(|_| PreferencesError::StoreUnavailable)?;
            if !request.preferences.app.restore_last_page {
                tx.execute("DELETE FROM route_history", [])
                    .map_err(|_| PreferencesError::StoreUnavailable)?;
            }
            tx.commit().map_err(|_| PreferencesError::StoreUnavailable)
        })();
        if let Err(store) = store_result {
            return match integration.apply(&request.preferences.app, &previous.app) {
                Ok(()) => Err(ApplyPreferencesError::Store(store)),
                Err(rollback) => Err(ApplyPreferencesError::Rollback { store, rollback }),
            };
        }
        Ok(PreferencesSnapshot {
            revision: next_revision as u64,
            preferences: request.preferences,
        })
    }
    pub fn route_history_get(&self) -> Result<Option<crate::route_history::SavedRoute>> {
        crate::route_history::get(&self.connection)
    }
    pub fn route_history_save(&self, route: crate::route_history::SavedRoute) -> Result<()> {
        crate::route_history::save(&self.connection, route)
    }
    pub fn route_history_clear(&self) -> Result<()> {
        crate::route_history::clear(&self.connection)
    }
    pub fn new(data_dir: PathBuf) -> Result<Arc<Self>> {
        if let Ok(metadata) = fs::symlink_metadata(&data_dir)
            && (!metadata.is_dir() || metadata.file_type().is_symlink())
        {
            return Err(PreferencesError::StoreUnavailable);
        }
        fs::create_dir_all(&data_dir).map_err(|_| PreferencesError::StoreUnavailable)?;
        check_symlink(&data_dir, true)?;

        let db_path = data_dir.join(DATABASE);
        if db_path.exists() {
            check_symlink(&db_path, false)?;
            let len = fs::metadata(&db_path)
                .map_err(|_| PreferencesError::StoreUnavailable)?
                .len();
            if len > MAX_DATABASE {
                return Err(PreferencesError::StoreCorrupt);
            }
        }

        let connection = Connection::open_with_flags(
            &db_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|_| PreferencesError::StoreUnavailable)?;

        connection
            .pragma_update(None, "journal_mode", "DELETE")
            .map_err(|_| PreferencesError::StoreUnavailable)?;
        connection
            .pragma_update(None, "synchronous", "NORMAL")
            .map_err(|_| PreferencesError::StoreUnavailable)?;

        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS preferences_store (
                    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                    schema_version INTEGER NOT NULL CHECK(schema_version = 1),
                    revision INTEGER NOT NULL,
                    payload TEXT NOT NULL
                ) STRICT;
                CREATE TABLE IF NOT EXISTS route_history (
                    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                    payload TEXT NOT NULL
                ) STRICT;",
            )
            .map_err(|_| PreferencesError::StoreUnavailable)?;

        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM preferences_store WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .map_err(|_| PreferencesError::StoreUnavailable)?;

        if count == 0 {
            let defaults = Preferences::default();
            let json =
                serde_json::to_string(&defaults).map_err(|_| PreferencesError::StoreCorrupt)?;
            connection
                .execute(
                    "INSERT INTO preferences_store (singleton, schema_version, revision, payload) VALUES (1, 1, 1, ?)",
                    params![json],
                )
                .map_err(|_| PreferencesError::StoreUnavailable)?;
        }

        Ok(Arc::new(Self {
            _dir: data_dir,
            connection: Mutex::new(connection),
        }))
    }

    pub fn snapshot(&self) -> Result<PreferencesSnapshot> {
        let conn = self
            .connection
            .lock()
            .map_err(|_| PreferencesError::StoreUnavailable)?;
        let (revision, payload): (i64, String) = conn
            .query_row(
                "SELECT revision, payload FROM preferences_store WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|e| match e {
                rusqlite::Error::SqliteFailure(ref f, _)
                    if matches!(
                        f.code,
                        rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase
                    ) =>
                {
                    PreferencesError::StoreCorrupt
                }
                _ => PreferencesError::StoreUnavailable,
            })?;

        let preferences: Preferences =
            decode_preferences(&payload).map_err(|_| PreferencesError::StoreCorrupt)?;
        validate(&preferences).map_err(|_| PreferencesError::StoreCorrupt)?;
        Ok(PreferencesSnapshot {
            revision: revision as u64,
            preferences,
        })
    }

    pub fn save(&self, request: SavePreferencesRequest) -> Result<PreferencesSnapshot> {
        validate(&request.preferences)?;
        let mut conn = self
            .connection
            .lock()
            .map_err(|_| PreferencesError::StoreUnavailable)?;
        let tx = conn
            .transaction()
            .map_err(|_| PreferencesError::StoreUnavailable)?;

        let current_revision: i64 = tx
            .query_row(
                "SELECT revision FROM preferences_store WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .map_err(|_| PreferencesError::StoreUnavailable)?;

        if current_revision as u64 != request.revision {
            return Err(PreferencesError::Conflict);
        }

        let next_revision = current_revision
            .checked_add(1)
            .ok_or(PreferencesError::StoreUnavailable)?;
        let json = serde_json::to_string(&request.preferences)
            .map_err(|_| PreferencesError::InvalidInput)?;

        tx.execute(
            "UPDATE preferences_store SET revision = ?, payload = ? WHERE singleton = 1",
            params![next_revision, json],
        )
        .map_err(|_| PreferencesError::StoreUnavailable)?;

        if !request.preferences.app.restore_last_page {
            tx.execute("DELETE FROM route_history", [])
                .map_err(|_| PreferencesError::StoreUnavailable)?;
        }
        tx.commit()
            .map_err(|_| PreferencesError::StoreUnavailable)?;

        Ok(PreferencesSnapshot {
            revision: next_revision as u64,
            preferences: request.preferences,
        })
    }
}

#[cfg(test)]
#[path = "preferences_tests.rs"]
mod tests;
