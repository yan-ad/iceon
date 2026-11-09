//! Catalog view: cluster contexts table, search filter, kubeconfig import via GPUI file dialog,
//! pin/unpin actions, and remove confirmation modal.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    scroll::ScrollableElement as _,
    tag::Tag,
    v_flex, IconName,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::IceonApp;
use crate::navigation::{Route, catalog_error_message};
use iceon_core::catalog::{CatalogSnapshot, ClusterSummary, PinRequest, RemoveRequest};

impl IceonApp {
    pub(crate) fn render_catalog_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let query = self
            .catalog_search_input
            .read(cx)
            .value()
            .trim()
            .to_lowercase();
        let clusters: Vec<ClusterSummary> = self
            .catalog_snapshot
            .clusters
            .iter()
            .filter(|c| {
                query.is_empty()
                    || c.name.to_lowercase().contains(&query)
                    || c.context_name.to_lowercase().contains(&query)
                    || c.labels.iter().any(|l| l.to_lowercase().contains(&query))
            })
            .cloned()
            .collect();

        let discovery_error = self
            .catalog_snapshot
            .discovery_error
            .map(catalog_error_message);
        let catalog_error = self.catalog_error.clone();
        let pending_remove = self.pending_remove_cluster.clone();

        v_flex()
            .size_full()
            .p_5()
            .gap_4()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_lg()
                                    .font_semibold()
                                    .child("Kubernetes Clusters"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(
                                        "{} imported context(s) · metadata-only discovery",
                                        self.catalog_snapshot.clusters.len()
                                    )),
                            ),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .w(px(260.0))
                                    .child(
                                        Input::new(&self.catalog_search_input)
                                            .small()
                                            .cleanable(true),
                                    ),
                            )
                            .child(
                                Button::new("catalog-refresh-btn")
                                    .small()
                                    .outline()
                                    .icon(IconName::Undo2)
                                    .label("Refresh")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.refresh_catalog(cx);
                                    })),
                            )
                            .child(
                                Button::new("catalog-import-btn")
                                    .small()
                                    .primary()
                                    .icon(IconName::FolderOpen)
                                    .label("Add Kubeconfig")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.prompt_import_kubeconfig(window, cx);
                                    })),
                            ),
                    ),
            )
            .when_some(discovery_error, |el, msg| {
                el.child(
                    h_flex()
                        .w_full()
                        .p_3()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(cx.theme().warning)
                        .bg(cx.theme().secondary)
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().warning)
                                .child(format!("Default kubeconfig discovery warning: {msg}")),
                        ),
                )
            })
            .when_some(catalog_error, |el, msg| {
                el.child(
                    h_flex()
                        .w_full()
                        .p_3()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(cx.theme().danger)
                        .bg(cx.theme().secondary)
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().danger)
                                .child(msg),
                        )
                        .child(
                            Button::new("dismiss-catalog-error")
                                .xsmall()
                                .ghost()
                                .icon(IconName::Close)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.catalog_error = None;
                                    cx.notify();
                                })),
                        ),
                )
            })
            .when_some(pending_remove, |el, target| {
                let target_id = target.id.clone();
                let target_name = target.name.clone();
                el.child(
                    h_flex()
                        .w_full()
                        .p_3()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(cx.theme().danger)
                        .bg(cx.theme().secondary)
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(
                            div().text_xs().child(format!(
                                "Remove cluster context \"{target_name}\" from Iceon catalog? (Your kubeconfig file on disk will not be modified.)"
                            )),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Button::new("cancel-remove-cluster")
                                        .xsmall()
                                        .outline()
                                        .label("Cancel")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.pending_remove_cluster = None;
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Button::new("confirm-remove-cluster")
                                        .xsmall()
                                        .danger()
                                        .label("Remove")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.confirm_remove_cluster(target_id.clone(), cx);
                                        })),
                                ),
                        ),
                )
            })
            .child(
                v_flex()
                    .flex_1()
                    .w_full()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(px(8.0))
                    .overflow_hidden()
                    .child(
                        h_flex()
                            .w_full()
                            .px_4()
                            .py_2()
                            .bg(cx.theme().table_head)
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .text_xs()
                            .font_semibold()
                            .text_color(cx.theme().muted_foreground)
                            .child(div().w(px(260.0)).child("Name / Context"))
                            .child(div().flex_1().child("Source"))
                            .child(div().w(px(130.0)).child("Status"))
                            .child(div().w(px(220.0)).child("Actions")),
                    )
                    .child(if clusters.is_empty() {
                        v_flex()
                            .flex_1()
                            .w_full()
                            .items_center()
                            .justify_center()
                            .p_8()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .font_medium()
                                    .child("No clusters match the current view"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(
                                        "Import a kubeconfig file or place one at ~/.kube/config to get started.",
                                    ),
                            )
                            .into_any_element()
                    } else {
                        v_flex()
                            .flex_1()
                            .w_full()
                            .overflow_y_scrollbar()
                            .children(clusters.into_iter().enumerate().map(|(idx, cluster)| {
                                let cluster_id = cluster.id.clone();
                                let open_id = cluster.id.clone();
                                let pin_id = cluster.id.clone();
                                let is_pinned = cluster.pinned;
                                let remove_target = cluster.clone();
                                let source = cluster
                                    .labels
                                    .first()
                                    .cloned()
                                    .unwrap_or_else(|| cluster.source.clone());
                                let status_badge = match cluster.status {
                                    "connected" => Tag::success().child("Connected"),
                                    "connecting" => Tag::warning().child("Connecting"),
                                    "error" => Tag::danger().child("Error"),
                                    _ => Tag::secondary().child("Disconnected"),
                                };

                                h_flex()
                                    .id(SharedString::from(format!("catalog-row-{idx}")))
                                    .w_full()
                                    .px_4()
                                    .py_2p5()
                                    .border_b_1()
                                    .border_color(cx.theme().border)
                                    .items_center()
                                    .hover(|s| s.bg(cx.theme().table_hover))
                                    .child(
                                        h_flex()
                                            .w(px(260.0))
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .w(px(10.0))
                                                    .h(px(10.0))
                                                    .rounded_full()
                                                    .bg(cx.theme().primary),
                                            )
                                            .child(
                                                v_flex()
                                                    .overflow_hidden()
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .font_medium()
                                                            .truncate()
                                                            .child(cluster.name.clone()),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(
                                                                cx.theme().muted_foreground,
                                                            )
                                                            .truncate()
                                                            .child(cluster.context_name.clone()),
                                                    ),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .truncate()
                                            .child(source),
                                    )
                                    .child(div().w(px(130.0)).child(status_badge))
                                    .child(
                                        h_flex()
                                            .w(px(220.0))
                                            .items_center()
                                            .gap_1p5()
                                            .child(
                                                Button::new(SharedString::from(format!(
                                                    "open-cluster-{cluster_id}"
                                                )))
                                                .xsmall()
                                                .primary()
                                                .label("Open")
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.navigate(
                                                            Route::Cluster {
                                                                cluster_id: open_id.clone(),
                                                                section: "overview".into(),
                                                            },
                                                            true,
                                                            window,
                                                            cx,
                                                        );
                                                    },
                                                )),
                                            )
                                            .child(
                                                Button::new(SharedString::from(format!(
                                                    "pin-cluster-{cluster_id}"
                                                )))
                                                .xsmall()
                                                .outline()
                                                .icon(IconName::Star)
                                                .label(if is_pinned { "Unpin" } else { "Pin" })
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.toggle_cluster_pin(
                                                        pin_id.clone(),
                                                        !is_pinned,
                                                        cx,
                                                    );
                                                })),
                                            )
                                            .child(
                                                Button::new(SharedString::from(format!(
                                                    "remove-cluster-{cluster_id}"
                                                )))
                                                .xsmall()
                                                .ghost()
                                                .icon(gpui_kit::assets::IconName::Trash)
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.pending_remove_cluster =
                                                        Some(remove_target.clone());
                                                    cx.notify();
                                                })),
                                            ),
                                    )
                            }))
                            .into_any_element()
                    }),
            )
    }

    pub(crate) fn refresh_catalog(&mut self, cx: &mut Context<Self>) {
        match self.catalog.list() {
            Ok(mut snapshot) => {
                let _ = self.connection.project_catalog_statuses(&mut snapshot);
                self.catalog_snapshot = snapshot;
                self.catalog_error = None;
            }
            Err(error) => {
                self.catalog_error = Some(catalog_error_message(error).to_string());
            }
        }
        cx.notify();
    }

    pub(crate) fn prompt_import_kubeconfig(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
                let mut snapshot =
                    catalog.import_kubeconfig(iceon_core::catalog::ImportRequest { path: path_str })?;
                let _ = connection.project_catalog_statuses(&mut snapshot);
                Ok::<CatalogSnapshot, iceon_core::catalog::CatalogError>(snapshot)
            })
            .await;

            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(Ok(snapshot)) => {
                        this.catalog_snapshot = snapshot;
                        this.catalog_error = None;
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

    pub(crate) fn toggle_cluster_pin(
        &mut self,
        cluster_id: String,
        pinned: bool,
        cx: &mut Context<Self>,
    ) {
        match self.catalog.set_pin(PinRequest {
            id: cluster_id,
            pinned,
        }) {
            Ok(mut snapshot) => {
                let _ = self.connection.project_catalog_statuses(&mut snapshot);
                self.catalog_snapshot = snapshot;
                self.catalog_error = None;
            }
            Err(err) => {
                self.catalog_error = Some(catalog_error_message(err).to_string());
            }
        }
        cx.notify();
    }

    pub(crate) fn confirm_remove_cluster(
        &mut self,
        cluster_id: String,
        cx: &mut Context<Self>,
    ) {
        self.pending_remove_cluster = None;
        match self.catalog.remove(RemoveRequest {
            id: cluster_id.clone(),
        }) {
            Ok(mut snapshot) => {
                if let Some(session) = &self.cluster_session
                    && session.cluster_id == cluster_id
                {
                    let _ = self.connection.disconnect(iceon_core::connection::SessionRequest {
                        cluster_id: session.cluster_id.clone(),
                        session_id: session.session_id.clone(),
                    });
                    self.cluster_session = None;
                }
                let _ = self.connection.project_catalog_statuses(&mut snapshot);
                self.catalog_snapshot = snapshot;
                self.catalog_error = None;
            }
            Err(err) => {
                self.catalog_error = Some(catalog_error_message(err).to_string());
            }
        }
        cx.notify();
    }
}
