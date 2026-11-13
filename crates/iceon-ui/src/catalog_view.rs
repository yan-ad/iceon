//! Cluster setup fallback and background catalog operations.

#[path = "cluster_config.rs"]
mod cluster_config;

use crate::{IceonApp, navigation::catalog_error_message};
use gpui_kit::*;

impl IceonApp {
    pub(crate) fn render_cluster_setup(&self, cx: &mut Context<Self>) -> AnyElement {
        self.render_connection_guide(false, cx)
    }

    pub(crate) fn refresh_catalog(&mut self, cx: &mut Context<Self>) {
        self.catalog_refresh_generation = self.catalog_refresh_generation.wrapping_add(1);
        if self.catalog_refresh_pending {
            return;
        }
        let generation = self.catalog_refresh_generation;
        self.catalog_refresh_pending = true;
        let catalog = self.catalog.clone();
        let connection = self.connection.clone();
        let preferences = self.preferences_draft.kubernetes.clone();
        cx.spawn(async move |owner, cx| {
            let result = iceon_core::runtime::spawn_blocking(move || {
                let mut snapshot = catalog.list_with_preferences(&preferences)?;
                let _ = connection.project_catalog_statuses(&mut snapshot);
                let health = connection.cluster_health_snapshot().unwrap_or_default();
                let versions = catalog.access_versions()?;
                Ok::<_, iceon_core::catalog::CatalogError>((snapshot, health, versions))
            })
            .await;
            let _ = owner.update(cx, |app, cx| {
                app.catalog_refresh_pending = false;
                if app.catalog_refresh_generation != generation {
                    app.refresh_catalog(cx);
                    return;
                }
                let changed = match result {
                    Ok(Ok((snapshot, health, versions))) => {
                        let changed = app.catalog_snapshot != snapshot
                            || app.cluster_versions != versions
                            || app.cluster_health != health
                            || app.catalog_error.is_some();
                        app.cluster_versions = versions;
                        app.cluster_health = health;
                        app.catalog_snapshot = snapshot;
                        app.catalog_error = None;
                        changed
                    }
                    result => {
                        let error = match result {
                            Ok(Err(error)) => catalog_error_message(error).to_string(),
                            _ => "Catalog refresh worker unavailable".to_string(),
                        };
                        let changed = app.catalog_error.as_ref() != Some(&error);
                        app.catalog_error = Some(error);
                        changed
                    }
                };
                if changed {
                    cx.notify();
                }
            });
        })
        .detach();
    }
}
