//! Catalog view: cluster contexts table, search filter, kubeconfig import via GPUI file dialog,
//! pin/unpin actions, and remove confirmation modal.

use gpui_kit::component::Selectable as _;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _, WindowExt as _,
    button::{Button, ButtonVariant, ButtonVariants as _},
    h_flex,
    input::Input,
    scroll::{ScrollableElement as _, ScrollbarAxis},
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::navigation::{Route, catalog_error_message};
use crate::{IceonApp, selectable_text::selectable_text};
use iceon_core::catalog::{CatalogSnapshot, ClusterSummary, PinRequest, RemoveRequest};

// Shared geometry keeps column boundaries identical for the header and every row.
// Source takes any spare width; the other columns never shrink into their neighbors.
#[derive(Clone, Copy)]
struct CatalogColumn {
    title: &'static str,
    width: f32,
    flexible: bool,
    separator: bool,
}

const CATALOG_COLUMNS: [CatalogColumn; 7] = [
    CatalogColumn {
        title: "Name",
        width: 240.0,
        flexible: false,
        separator: false,
    },
    CatalogColumn {
        title: "Source",
        width: 160.0,
        flexible: true,
        separator: false,
    },
    CatalogColumn {
        title: "Labels",
        width: 160.0,
        flexible: false,
        separator: false,
    },
    CatalogColumn {
        title: "Version",
        width: 150.0,
        flexible: false,
        separator: false,
    },
    CatalogColumn {
        title: "Distro",
        width: 90.0,
        flexible: false,
        separator: false,
    },
    CatalogColumn {
        title: "Status",
        width: 130.0,
        flexible: false,
        separator: false,
    },
    CatalogColumn {
        title: "Actions",
        width: 112.0,
        flexible: false,
        separator: false,
    },
];
const CATALOG_TABLE_MIN_WIDTH: f32 = 1042.0;

// Only explicit vendor suffixes in an observed server version are evidence.
fn catalog_distro(version: &str) -> &'static str {
    let suffix = version
        .find(['-', '+'])
        .map(|at| &version[at + 1..])
        .unwrap_or("")
        .to_ascii_lowercase();
    for (marker, name) in [
        ("gke", "GKE"),
        ("eks", "EKS"),
        ("k3s", "K3s"),
        ("rke2", "RKE2"),
        ("microk8s", "MicroK8s"),
    ] {
        if suffix.strip_prefix(marker).is_some_and(|tail| {
            tail.is_empty()
                || (marker == "rke2"
                    && tail
                        .strip_prefix('r')
                        .is_some_and(|revision| revision.starts_with(|c: char| c.is_ascii_digit())))
                || tail.starts_with(['-', '+', '.'])
                || tail.starts_with(|c: char| c.is_ascii_digit())
        }) {
            return name;
        }
    }
    "—"
}

fn catalog_version<'a>(cluster_id: &str, session: Option<(&str, &'a str)>) -> &'a str {
    session
        .filter(|(id, _)| *id == cluster_id)
        .map(|(_, version)| version.trim())
        .filter(|version| !version.is_empty())
        .unwrap_or("—")
}

fn catalog_cell(column: CatalogColumn, border: Hsla) -> Div {
    h_flex()
        .items_center()
        .px_3()
        .py_2()
        .flex_shrink_0()
        .when(column.flexible, |cell| {
            cell.flex_1().min_w(px(column.width))
        })
        .when(!column.flexible, |cell| cell.w(px(column.width)))
        .when(column.separator, |cell| {
            cell.border_r_1().border_color(border)
        })
}

impl IceonApp {
    pub(crate) fn render_welcome_view(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .id("welcome-page")
            .size_full().min_h_0().min_w_0().overflow_y_scrollbar()
            .bg(cx.theme().background)
            .child(v_flex().w_full().min_h_full().items_center().justify_center().p_8()
                .child(v_flex().w_full().max_w(px(560.0)).items_center().gap_6()
                    .child(img("iceon/app-icon.png").size(px(220.0)).flex_shrink_0())
                    .child(div().text_3xl().font_semibold().text_center().child("Welcome to Iceon!"))
                    .child(div().w_full().text_base().text_center().text_color(cx.theme().muted_foreground)
                        .child("Your kubeconfig contexts are discovered automatically and collected in the cluster catalog—one place to explore your Kubernetes clusters and manage their resources."))
                    .child(v_flex().w_full().items_center().gap_1()
                        .child(div().text_base().text_center().text_color(cx.theme().muted_foreground)
                            .child("Have a question, found an issue, or want to share feedback?"))
                        .child(Button::new("welcome-github").ghost()
                            .text_color(cx.theme().link).label("Join us on GitHub")
                            .accessibility_label("Open Iceon GitHub repository")
                            .on_click(|_, _, cx| cx.open_url(iceon_core::project::REPOSITORY_URL))))
                    .child(v_flex().w_full().gap_2()
                        .child(Button::new("welcome-browse-clusters").w_full().h(px(44.0))
                            .child(h_flex().w_full().items_center().gap_3()
                                .child(Icon::new(IconName::LayoutDashboard).small())
                                .child(div().flex_1().text_left().child("Browse Clusters in Catalog"))
                                .child(Icon::new(IconName::ChevronRight).small()))
                            .accessibility_label("Browse all clusters in catalog")
                            .on_click(cx.listener(|this, _, _, cx| { this.home_catalog_visible = true; cx.notify(); })))
                        .child(Button::new("welcome-open-terminal").w_full().h(px(44.0))
                            .child(h_flex().w_full().items_center().gap_3()
                                .child(Icon::new(IconName::SquareTerminal).small())
                                .child(div().flex_1().text_left().child("Open a Terminal"))
                                .child(Icon::new(IconName::ChevronRight).small()))
                            .accessibility_label("Open a local terminal")
                            .on_click(cx.listener(|this, _, window, cx| this.open_device_terminal(window, cx)))))
                ))
            .into_any_element()
    }

    pub(crate) fn render_catalog_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // One scroll handle owns both axes and the header/body viewport. Keeping
        // the minimum width on the content (not the viewport) allows overflow.
        let table_scroll = window
            .use_keyed_state("catalog-table-scroll", cx, |_, _| ScrollHandle::default())
            .read(cx)
            .clone();
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

        v_flex()
            .relative()
            .size_full()
            .min_h_0()
            .min_w_0()
            .p_4()
            .gap_3()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(div().text_sm().font_semibold().child(selectable_text("Clusters")))
                            .child(div().text_xs().text_color(cx.theme().muted_foreground)
                                .child(selectable_text(format!("{}", clusters.len())))),
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
                                    .ghost()
                                    .icon(IconName::RefreshCw)
                                    .tooltip("Refresh catalog")
                                    .accessibility_label("Refresh catalog")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.refresh_catalog(cx);
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
                                .child(selectable_text(format!("Default kubeconfig discovery warning: {msg}"))),
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
                                .child(selectable_text(msg)),
                        )
                        .child(
                            Button::new("dismiss-catalog-error")
                                .xsmall()
                                .ghost()
                                .icon(IconName::Close)
                                .tooltip("Dismiss catalog error")
                                .accessibility_label("Dismiss catalog error")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.catalog_error = None;
                                    cx.notify();
                                })),
                        ),
                )
            })
            .child(
                div()
                    .id("catalog-table-viewport")
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .w_full()
                    .overflow_scroll()
                    .track_scroll(&table_scroll)
                    .scrollbar(&table_scroll, ScrollbarAxis::Both)
                    .child(
                        v_flex()
                            .w_full()
                            .when(!clusters.is_empty(), |table| table.min_w(px(CATALOG_TABLE_MIN_WIDTH)))
                            .min_h_full()
                            .flex_none()
                            .when(!clusters.is_empty(), |table| table.child(
                                h_flex()
                                    .w_full()
                                    .flex_shrink_0()
                                    .items_stretch()
                                    .text_xs()
                                    .font_semibold()
                                    .text_color(cx.theme().muted_foreground)
                                    .children(CATALOG_COLUMNS.into_iter().map(|column| {
                                        catalog_cell(column, cx.theme().border).child(selectable_text(column.title))
                                    })),
                            ))
                            .child(if clusters.is_empty() {
                                crate::empty_state::empty_state(
                                    IconName::LayoutDashboard,
                                    "No clusters match the current view",
                                    if query.is_empty() { "Import a kubeconfig using + or place one at ~/.kube/config to get started." } else { "Try another name, context, or label, or clear the search." },
                                ).into_any_element()
                            } else {
                                v_flex()
                                    .flex_1()
                                    .w_full()
                                    .min_w_0()
                                    .children(clusters.into_iter().enumerate().map(|(idx, cluster)| {
                                        let cluster_id = cluster.id.clone();
                                        let open_id = cluster.id.clone();
                                        let pin_id = cluster.id.clone();
                                        let is_pinned = cluster.pinned;
                                        let remove_target = cluster.clone();
                                        let source = cluster.source.clone();
                                        let version = catalog_version(&cluster.id, self.cluster_session.as_ref().and_then(|session| session.api_version.as_deref().map(|version| (session.cluster_id.as_str(), version)))).to_owned();
                                        let distro = catalog_distro(&version);
                                        let row_id = cluster.id.clone();
                                        let status_badge = match cluster.status {
                                            "connected" => Tag::success().child(selectable_text("Connected")),
                                            "connecting" => Tag::warning().child(selectable_text("Connecting")),
                                            "error" => Tag::danger().child(selectable_text("Error")),
                                            _ => Tag::secondary().child(selectable_text("Disconnected")),
                                        };
                                        let badge_color = crate::sidebar_view::cluster_badge_color(
                                            &cluster.color,
                                            &cluster.id,
                                            cx.theme().mode.is_dark(),
                                        );

                                        h_flex()
                                            .id(SharedString::from(format!("catalog-row-{idx}")))
                                            .w_full()
                                            .flex_shrink_0()
                                            .items_stretch()
                                            .cursor_pointer()
                                            .hover(|row| row.bg(cx.theme().secondary))
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.navigate(Route::Cluster { cluster_id: row_id.clone(), section: "overview".into() }, true, window, cx);
                                            }))
                                            .child(
                                                catalog_cell(CATALOG_COLUMNS[0], cx.theme().border)
                                                    .gap_2()
                                                    .child(
                                                        div()
                                                            .w(px(10.0))
                                                            .h(px(10.0))
                                                            .flex_shrink_0()
                                                            .rounded_full()
                                                            .bg(badge_color),
                                                    )
                                                    .child(
                                                        v_flex()
                                                            .min_w_0()
                                                            .overflow_hidden()
                                                            .child(
                                                                div()
                                                                    .text_sm()
                                                                    .font_medium()
                                                                    .truncate()
                                                                    .child(selectable_text(cluster.name.clone())),
                                                            )
                                                            .child(
                                                                div()
                                                                    .text_xs()
                                                                    .text_color(
                                                                        cx.theme().muted_foreground,
                                                                    )
                                                                    .truncate()
                                                                    .child(selectable_text(cluster.context_name.clone())),
                                                            ),
                                                    ),
                                            )
                                            .child(
                                                catalog_cell(CATALOG_COLUMNS[1], cx.theme().border)
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(div().min_w_0().truncate().child(selectable_text(source))),
                                            )
                                            .child(
                                                catalog_cell(CATALOG_COLUMNS[2], cx.theme().border)
                                                    .text_xs().child(selectable_text(if cluster.labels.is_empty() { "—".to_owned() } else { cluster.labels.join(", ") })),
                                            )
                                            .child(catalog_cell(CATALOG_COLUMNS[3], cx.theme().border).text_xs().child(selectable_text(version)))
                                            .child(catalog_cell(CATALOG_COLUMNS[4], cx.theme().border).text_xs().child(selectable_text(distro)))
                                            .child(
                                                catalog_cell(CATALOG_COLUMNS[5], cx.theme().border)
                                                    .child(status_badge),
                                            )
                                            .child(
                                                catalog_cell(CATALOG_COLUMNS[6], cx.theme().border)
                                                    .gap_1()
                                                    .child(
                                                        Button::new(SharedString::from(format!(
                                                            "open-cluster-{cluster_id}"
                                                        )))
                                                        .xsmall()
                                                        .ghost()
                                                        .icon(IconName::ChevronRight)
                                                        .tooltip("Open cluster")
                                                        .accessibility_label("Open cluster")
                                                        .on_click(cx.listener(
                                                            move |this, _, window, cx| {
                                                                cx.stop_propagation();
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
                                                        .ghost()
                                                        .icon(IconName::Star)
                                                        .selected(is_pinned)
                                                        .tooltip(if is_pinned { "Unpin cluster" } else { "Pin cluster" })
                                                        .accessibility_label(if is_pinned { "Unpin cluster" } else { "Pin cluster" })
                                                        .on_click(cx.listener(move |this, _, _, cx| {
                                                            cx.stop_propagation();
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
                                                        .danger()
                                                        .icon(gpui_kit::assets::IconName::Trash)
                                                        .tooltip("Remove cluster context")
                                                        .accessibility_label("Remove cluster context")
                                                        .on_click(cx.listener(move |this, _, window, cx| {
                                                            cx.stop_propagation();
                                                            this.prompt_remove_cluster(remove_target.clone(), window, cx);
                                                        })),
                                                    ),
                                            )
                                    }))
                                    .into_any_element()
                            }),
                    ),
            )
            .child(div().absolute().right_4().bottom_4().child(
                Button::new("catalog-import-fab")
                    .primary().icon(IconName::Plus).size(px(48.0)).rounded_full()
                    .tooltip("Add kubeconfig").accessibility_label("Add kubeconfig")
                    .on_click(cx.listener(|this, _, window, cx| this.prompt_import_kubeconfig(window, cx))),
            ))
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
                Ok::<_, iceon_core::catalog::CatalogError>(snapshot)
            })
            .await;
            let _ = owner.update(cx, |app, cx| {
                app.catalog_refresh_pending = false;
                let changed = match result {
                    Ok(Ok(snapshot)) => {
                        let changed =
                            app.catalog_snapshot != snapshot || app.catalog_error.is_some();
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

    fn prompt_remove_cluster(
        &self,
        target: ClusterSummary,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let app = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let app = app.clone();
            let id = target.id.clone();
            dialog
                .centered()
                .confirm()
                .title("Remove cluster context?")
                .description(format!(
                    "Remove \"{}\" from Iceon's catalog? Your kubeconfig file on disk will not be modified.",
                    target.name
                ))
                .ok_text("Remove")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, _, cx| {
                    let _ = app.update(cx, |this, cx| this.confirm_remove_cluster(id.clone(), cx));
                    true
                })
        });
    }

    pub(crate) fn confirm_remove_cluster(&mut self, cluster_id: String, cx: &mut Context<Self>) {
        self.pending_remove_cluster = None;
        match self.catalog.remove(RemoveRequest {
            id: cluster_id.clone(),
        }) {
            Ok(mut snapshot) => {
                if let Some(session) = &self.cluster_session
                    && session.cluster_id == cluster_id
                {
                    let _ = self
                        .connection
                        .disconnect(iceon_core::connection::SessionRequest {
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

#[cfg(test)]
mod catalog_table_tests {
    use super::{CATALOG_COLUMNS, CATALOG_TABLE_MIN_WIDTH, catalog_distro, catalog_version};

    #[test]
    fn reference_columns_are_in_order() {
        assert_eq!(
            CATALOG_COLUMNS.map(|column| column.title),
            [
                "Name", "Source", "Labels", "Version", "Distro", "Status", "Actions"
            ]
        );
    }

    #[test]
    fn version_is_only_from_the_matching_current_session() {
        assert_eq!(
            catalog_version("a", Some(("a", "v1.31.1-gke.100"))),
            "v1.31.1-gke.100"
        );
        assert_eq!(catalog_version("b", Some(("a", "v1.31.1-gke.100"))), "—");
        assert_eq!(catalog_version("a", None), "—");
        assert_eq!(catalog_version("a", Some(("a", "  "))), "—");
    }

    #[test]
    fn distro_requires_an_explicit_known_version_suffix() {
        for (version, distro) in [
            ("v1.31.1-gke.100", "GKE"),
            ("v1.30.0-eks-abcd", "EKS"),
            ("v1.31.1+k3s1", "K3s"),
            ("v1.31.1+rke2r1", "RKE2"),
            ("v1.31.1-microk8s", "MicroK8s"),
        ] {
            assert_eq!(catalog_distro(version), distro);
        }
        for version in [
            "—",
            "",
            "v1.31.1",
            "gke-context",
            "v1.31.1-gkexample",
            "v1.31.1-custom.eks",
        ] {
            assert_eq!(catalog_distro(version), "—");
        }
    }

    #[test]
    fn table_minimum_fits_all_columns() {
        let total: f32 = CATALOG_COLUMNS.iter().map(|column| column.width).sum();
        assert_eq!(CATALOG_TABLE_MIN_WIDTH, total);
        assert!(CATALOG_COLUMNS.iter().all(|column| column.width > 0.0));
    }

    #[test]
    fn only_source_expands_and_columns_remain_compact() {
        assert_eq!(
            CATALOG_COLUMNS.map(|column| column.flexible),
            [false, true, false, false, false, false, false]
        );
        assert_eq!(CATALOG_COLUMNS.map(|column| column.separator), [false; 7]);
        assert_eq!(CATALOG_COLUMNS[1].title, "Source");
    }
}
