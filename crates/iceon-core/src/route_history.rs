use crate::preferences::{PreferencesError, Result, decode_preferences};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavedRoute {
    pub path: String,
    #[serde(default)]
    pub section: Option<String>,
    #[serde(default)]
    pub definition: Option<String>,
}

const SECTIONS: &[&str] = &[
    "overview",
    "nodes",
    "namespaces",
    "events",
    "workloads",
    "workloads-overview",
    "pods",
    "deployments",
    "daemon-sets",
    "stateful-sets",
    "replica-sets",
    "replication-controllers",
    "jobs",
    "cron-jobs",
    "config",
    "config-maps",
    "secrets",
    "resource-quotas",
    "limit-ranges",
    "horizontal-pod-autoscalers",
    "vertical-pod-autoscalers",
    "pod-disruption-budgets",
    "priority-classes",
    "runtime-classes",
    "leases",
    "mutating-webhook-configs",
    "validating-webhook-configs",
    "validating-admission-policies",
    "validating-admission-policy-bindings",
    "network",
    "services",
    "endpoint-slices",
    "endpoints",
    "ingresses",
    "ingress-classes",
    "network-policies",
    "port-forwarding",
    "storage",
    "persistent-volume-claims",
    "persistent-volumes",
    "storage-classes",
    "helm",
    "helm-charts",
    "helm-releases",
    "access-control",
    "users",
    "service-accounts",
    "cluster-roles",
    "roles",
    "cluster-role-bindings",
    "role-bindings",
    "pod-security-policies",
    "custom-resources",
    "custom-resource-definitions",
    "custom-resource-instances",
];
fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
}
fn valid_definition(value: &str) -> bool {
    value.len() <= 253
        && value.contains('.')
        && value.split('.').all(|part| {
            !part.is_empty()
                && part.len() <= 63
                && part
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
                && !part.starts_with('-')
                && !part.ends_with('-')
        })
}
pub fn validate(route: &SavedRoute) -> Result<()> {
    let catalog = matches!(
        route.path.as_str(),
        "/catalog" | "/catalog/all" | "/catalog/clusters"
    );
    let secrets = route.path == "/secrets";
    let cluster = route.path.strip_prefix("/cluster/").is_some_and(valid_id);
    if !catalog && !secrets && !cluster
        || (catalog || secrets) && (route.section.is_some() || route.definition.is_some())
    {
        return Err(PreferencesError::InvalidInput);
    }
    if route
        .section
        .as_deref()
        .is_some_and(|s| !SECTIONS.contains(&s))
    {
        return Err(PreferencesError::InvalidInput);
    }
    if route.section.as_deref() == Some("custom-resource-instances") {
        if !route.definition.as_deref().is_some_and(valid_definition) {
            return Err(PreferencesError::InvalidInput);
        }
    } else if route.definition.is_some() {
        return Err(PreferencesError::InvalidInput);
    }
    Ok(())
}
fn enabled(conn: &Connection) -> Result<bool> {
    let json: String = conn
        .query_row(
            "SELECT payload FROM preferences_store WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .map_err(|_| PreferencesError::StoreUnavailable)?;
    Ok(decode_preferences(&json)
        .map_err(|_| PreferencesError::StoreCorrupt)?
        .app
        .restore_last_page)
}
pub fn get(connection: &Mutex<Connection>) -> Result<Option<SavedRoute>> {
    let conn = connection
        .lock()
        .map_err(|_| PreferencesError::StoreUnavailable)?;
    if !enabled(&conn)? {
        return Ok(None);
    }
    let value: Option<String> = conn
        .query_row(
            "SELECT payload FROM route_history WHERE singleton=1 AND length(payload)<=1024",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| PreferencesError::StoreUnavailable)?;
    value
        .map(|v| {
            let mut route: SavedRoute =
                serde_json::from_str(&v).map_err(|_| PreferencesError::StoreCorrupt)?;
            if route.path == "/catalog/web-links" {
                route.path = "/catalog".into();
            }
            validate(&route).map_err(|_| PreferencesError::StoreCorrupt)?;
            Ok(route)
        })
        .transpose()
}
pub fn save(connection: &Mutex<Connection>, route: SavedRoute) -> Result<()> {
    validate(&route)?;
    let conn = connection
        .lock()
        .map_err(|_| PreferencesError::StoreUnavailable)?;
    if !enabled(&conn)? {
        return Ok(());
    }
    let json = serde_json::to_string(&route).map_err(|_| PreferencesError::InvalidInput)?;
    conn.execute("INSERT INTO route_history(singleton,payload) VALUES(1,?) ON CONFLICT(singleton) DO UPDATE SET payload=excluded.payload", params![json]).map_err(|_| PreferencesError::StoreUnavailable)?;
    Ok(())
}
pub fn clear(connection: &Mutex<Connection>) -> Result<()> {
    let conn = connection
        .lock()
        .map_err(|_| PreferencesError::StoreUnavailable)?;
    conn.execute("DELETE FROM route_history", [])
        .map_err(|_| PreferencesError::StoreUnavailable)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::preferences::Preferences;
    use crate::preferences::{PreferencesEngine, SavePreferencesRequest};

    #[test]
    fn opt_in_persists_and_disable_clears_history() {
        let dir = tempfile::tempdir().unwrap();
        let engine = PreferencesEngine::new(dir.path().to_path_buf()).unwrap();
        let route = SavedRoute {
            path: "/catalog/clusters".into(),
            section: None,
            definition: None,
        };
        engine.route_history_save(route.clone()).unwrap();
        assert!(engine.route_history_get().unwrap().is_none());
        let mut snapshot = engine.snapshot().unwrap();
        snapshot.preferences.app.restore_last_page = true;
        engine
            .save(SavePreferencesRequest {
                revision: snapshot.revision,
                preferences: snapshot.preferences,
            })
            .unwrap();
        engine.route_history_save(route.clone()).unwrap();
        let reopened = PreferencesEngine::new(dir.path().to_path_buf()).unwrap();
        assert_eq!(reopened.route_history_get().unwrap(), Some(route));
        let mut snapshot = reopened.snapshot().unwrap();
        snapshot.preferences.app.restore_last_page = false;
        reopened
            .save(SavePreferencesRequest {
                revision: snapshot.revision,
                preferences: snapshot.preferences,
            })
            .unwrap();
        assert!(reopened.route_history_get().unwrap().is_none());
        let count: i64 = reopened
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM route_history", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn retired_catalog_route_restores_catalog_without_changing_preferences() {
        let dir = tempfile::tempdir().unwrap();
        let engine = PreferencesEngine::new(dir.path().to_path_buf()).unwrap();
        let mut settings = engine.snapshot().unwrap();
        settings.preferences.app.restore_last_page = true;
        let saved = engine
            .save(SavePreferencesRequest {
                revision: settings.revision,
                preferences: settings.preferences,
            })
            .unwrap();
        engine
            .connection
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO route_history(singleton,payload) VALUES(1,?1)",
                [r#"{"path":"/catalog/web-links"}"#],
            )
            .unwrap();
        assert_eq!(
            engine.route_history_get().unwrap(),
            Some(SavedRoute {
                path: "/catalog".into(),
                section: None,
                definition: None,
            })
        );
        assert_eq!(engine.snapshot().unwrap(), saved);
        assert_eq!(
            engine.route_history_save(SavedRoute {
                path: "/catalog/web-links".into(),
                section: None,
                definition: None,
            }),
            Err(PreferencesError::InvalidInput)
        );
        engine
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE route_history SET payload=?1",
                [r#"{"path":"/catalog/web-links","section":"pods"}"#],
            )
            .unwrap();
        assert_eq!(
            engine.route_history_get(),
            Err(PreferencesError::StoreCorrupt)
        );
    }

    #[test]
    fn secrets_history_contains_no_session_or_secret_path() {
        assert!(
            validate(&SavedRoute {
                path: "/secrets".into(),
                section: None,
                definition: None
            })
            .is_ok()
        );
        assert!(
            validate(&SavedRoute {
                path: "/secrets".into(),
                section: Some("private/path".into()),
                definition: None
            })
            .is_err()
        );
        assert!(
            validate(&SavedRoute {
                path: "/secrets/profile-id".into(),
                section: None,
                definition: None
            })
            .is_err()
        );
    }

    #[test]
    fn rejects_urls_query_credentials_and_unknown_sections() {
        for path in [
            "https://example.test",
            "/catalog?token=secret",
            "/cluster/a/b",
            "/cluster/../x",
            "/catalog/unknown",
        ] {
            assert!(
                validate(&SavedRoute {
                    path: path.into(),
                    section: None,
                    definition: None
                })
                .is_err()
            );
        }
        assert!(
            validate(&SavedRoute {
                path: "/cluster/local-id".into(),
                section: Some("unknown".into()),
                definition: None
            })
            .is_err()
        );
        assert!(
            validate(&SavedRoute {
                path: "/cluster/local-id".into(),
                section: Some("pods".into()),
                definition: None
            })
            .is_ok()
        );
    }

    #[test]
    fn old_snapshot_defaults_restoration_off_without_accepting_unknown_fields() {
        let mut old = serde_json::to_value(Preferences::default()).unwrap();
        old["app"]
            .as_object_mut()
            .unwrap()
            .remove("restoreLastPage");
        assert!(
            !serde_json::from_value::<Preferences>(old.clone())
                .unwrap()
                .app
                .restore_last_page
        );
        old["app"]["unknown"] = true.into();
        assert!(serde_json::from_value::<Preferences>(old).is_err());
    }
}
