//! Cluster setup fallback and background catalog operations.

use gpui_kit::component::{
    ActiveTheme as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::navigation::catalog_error_message;
use crate::{IceonApp, selectable_text::selectable_text};
use iceon_core::catalog::CatalogSnapshot;

impl IceonApp {
    pub(crate) fn render_cluster_setup(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .min_h_0()
            .min_w_0()
            .items_center()
            .justify_center()
            .gap_4()
            .p_6()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .text_lg()
                    .font_semibold()
                    .child("No configured clusters"),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Import a kubeconfig or configure cluster discovery in Preferences."),
            )
            .when_some(
                self.catalog_snapshot
                    .discovery_error
                    .map(catalog_error_message),
                |view, error| {
                    view.child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().warning)
                            .child(selectable_text(format!("Kubeconfig discovery: {error}"))),
                    )
                },
            )
            .when_some(self.catalog_error.clone(), |view, error| {
                view.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(selectable_text(error)),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("cluster-setup-import")
                            .primary()
                            .label("Import kubeconfig")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.prompt_import_kubeconfig(window, cx);
                            })),
                    )
                    .child(
                        Button::new("cluster-setup-preferences")
                            .label("Preferences")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_preferences(window, cx);
                            })),
                    ),
            )
    }

    pub(crate) fn refresh_catalog(&mut self, cx: &mut Context<Self>) {
        if self.catalog_refresh_pending {
            return;
        }
        self.catalog_refresh_pending = true;
        let catalog = self.catalog.clone();
        let connection = self.connection.clone();
        let preferences = self.preferences_draft.kubernetes.clone();
        cx.spawn(async move |owner, cx| {
            let result = iceon_core::runtime::spawn_blocking(move || {
                let mut snapshot = catalog.list_with_preferences(&preferences)?;
                let _ = connection.project_catalog_statuses(&mut snapshot);
                let health = connection.cluster_health_snapshot().unwrap_or_default();
                Ok::<_, iceon_core::catalog::CatalogError>((snapshot, health))
            })
            .await;
            let _ = owner.update(cx, |app, cx| {
                app.catalog_refresh_pending = false;
                let changed = match result {
                    Ok(Ok((snapshot, health))) => {
                        let changed = app.catalog_snapshot != snapshot
                            || app.cluster_health != health
                            || app.catalog_error.is_some();
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

    pub(crate) fn prompt_import_kubeconfig(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Select kubeconfig file".into()),
        });
        let catalog = self.catalog.clone();
        let connection = self.connection.clone();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let path_str = path.to_string_lossy().into_owned();
            let result = iceon_core::runtime::spawn_blocking(move || {
                let mut snapshot = catalog
                    .import_kubeconfig(iceon_core::catalog::ImportRequest { path: path_str })?;
                let _ = connection.project_catalog_statuses(&mut snapshot);
                Ok::<CatalogSnapshot, iceon_core::catalog::CatalogError>(snapshot)
            })
            .await;

            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(Ok(snapshot)) => {
                        this.catalog_snapshot = snapshot;
                        this.catalog_error = None;
                        this.ensure_cluster_route(window, cx);
                    }
                    Ok(Err(err)) => {
                        this.catalog_error = Some(catalog_error_message(err).to_string());
                    }
                    Err(_) => {
                        this.catalog_error =
                            Some("Failed to import kubeconfig on background worker.".into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
