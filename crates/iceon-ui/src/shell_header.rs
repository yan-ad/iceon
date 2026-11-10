//! Compact workspace navigation. The component owns search, focus, keyboard navigation,
//! popup dismissal and accessibility; the application remains the authority for routing.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, IndexPath, Sizable as _, TitleBar,
    button::{Button, ButtonVariants as _},
    h_flex,
    select::{SearchableVec, SelectEvent, SelectItem, SelectState},
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use iceon_core::catalog::CatalogSnapshot;

use crate::{IceonApp, navigation::Route};

/// IDs, rather than display names, identify contexts (names need not be unique).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ClusterChoice {
    id: String,
    name: String,
    context_name: String,
}

#[cfg(test)]
mod tests {
    use super::cluster_choices;
    use gpui_kit::component::select::SelectItem as _;
    use iceon_core::catalog::{CatalogSnapshot, ClusterSummary};

    fn snapshot() -> CatalogSnapshot {
        CatalogSnapshot {
            revision: 1,
            discovery_error: None,
            pinned_items: vec![],
            clusters: ["east", "west"]
                .into_iter()
                .map(|id| ClusterSummary {
                    id: id.into(),
                    name: "Production".into(),
                    context_name: format!("{id}-context"),
                    source: "kubeconfig".into(),
                    status: "disconnected",
                    pinned: false,
                    pinned_sections: vec![],
                    labels: vec![],
                    color: "#4ba7a5".into(),
                })
                .collect(),
        }
    }

    #[test]
    fn duplicate_cluster_names_keep_distinct_selection_ids() {
        let choices = cluster_choices(&snapshot());
        assert_eq!(choices[0].title(), choices[1].title());
        assert_ne!(choices[0].value(), choices[1].value());
    }

    #[test]
    fn selector_searches_names_and_contexts_case_insensitively() {
        let choices = cluster_choices(&snapshot());
        assert!(choices[0].matches(" production "));
        assert!(choices[0].matches("EAST-context"));
        assert!(!choices[1].matches("east-context"));
        assert!(!choices[0].matches("missing"));
    }

    #[test]
    fn selector_choices_track_catalog_metadata_refreshes() {
        let mut catalog = snapshot();
        let initial = cluster_choices(&catalog);
        catalog.clusters[0].name = "Renamed".into();
        assert_ne!(initial, cluster_choices(&catalog));
        catalog.clusters.remove(0);
        assert_eq!(cluster_choices(&catalog)[0].value(), "west");
    }
}

impl SelectItem for ClusterChoice {
    type Value = String;

    fn title(&self) -> SharedString {
        self.name.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }

    fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        self.name.to_lowercase().contains(&query)
            || self.context_name.to_lowercase().contains(&query)
    }

    fn render(&self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        // Context text disambiguates clusters that share a display name.
        gpui_kit::component::v_flex()
            .min_w_0()
            .gap_0p5()
            .child(div().truncate().child(self.name.clone()))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .truncate()
                    .child(self.context_name.clone()),
            )
    }
}

pub(super) type ClusterSelectState = SelectState<SearchableVec<ClusterChoice>>;

fn cluster_choices(snapshot: &CatalogSnapshot) -> Vec<ClusterChoice> {
    snapshot
        .clusters
        .iter()
        .map(|cluster| ClusterChoice {
            id: cluster.id.clone(),
            name: cluster.name.clone(),
            context_name: cluster.context_name.clone(),
        })
        .collect()
}

/// Call once from `IceonApp::new`, after computing the snapshot and initial route.
/// Store the returned pair in `cluster_select` and `cluster_select_items` respectively.
/// The detached subscription follows the select entity's lifetime, not individual renders.
pub(super) fn create_cluster_select(
    snapshot: &CatalogSnapshot,
    route: &Route,
    window: &mut Window,
    cx: &mut Context<IceonApp>,
) -> (Entity<ClusterSelectState>, Vec<ClusterChoice>) {
    let items = cluster_choices(snapshot);
    let selected_index = match route {
        Route::Cluster { cluster_id, .. } => items
            .iter()
            .position(|item| &item.id == cluster_id)
            .map(|row| IndexPath::default().row(row)),
        _ => None,
    };
    let select = cx.new(|cx| {
        SelectState::new(
            SearchableVec::new(items.clone()),
            selected_index,
            window,
            cx,
        )
        .searchable(true)
    });
    cx.subscribe_in(
        &select,
        window,
        |this, _, event: &SelectEvent<SearchableVec<ClusterChoice>>, window, cx| {
            let SelectEvent::Confirm(Some(cluster_id)) = event else {
                return;
            };
            // Ignore stale selections if a catalog refresh removed the context.
            if !this.catalog_snapshot.clusters.iter().any(|c| &c.id == cluster_id) {
                return;
            }
            if matches!(this.history.current(), Route::Cluster { cluster_id: current, .. } if current == cluster_id) {
                return;
            }
            this.navigate(
                Route::Cluster {
                    cluster_id: cluster_id.clone(),
                    section: "overview".into(),
                },
                true,
                window,
                cx,
            );
        },
    )
    .detach();
    (select, items)
}

impl IceonApp {
    pub(crate) fn render_title_bar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let items = cluster_choices(&self.catalog_snapshot);
        let selected_id = match self.history.current() {
            Route::Cluster { cluster_id, .. } => items
                .iter()
                .find(|item| &item.id == cluster_id)
                .map(|item| item.id.clone()),
            Route::Catalog | Route::Secrets => None,
        };
        let selected_cluster = items
            .iter()
            .find(|item| Some(&item.id) == selected_id.as_ref())
            .cloned();
        let current_namespace = self
            .selected_namespace
            .clone()
            .unwrap_or_else(|| "default".into());

        // Do not replace the delegate on ordinary renders: doing so would reset the
        // filtered view and keyboard cursor while the user types. Exact metadata
        // comparison also catches renames/imports/removals without refresh hooks.
        let items_changed = self.cluster_select_items != items;
        let selection_changed =
            self.cluster_select.read(cx).selected_value() != selected_id.as_ref();
        if items_changed || selection_changed {
            self.cluster_select.update(cx, |state, cx| {
                if items_changed {
                    state.set_items(SearchableVec::new(items.clone()), window, cx);
                }
                if let Some(id) = selected_id.as_ref() {
                    state.set_selected_value(id, window, cx);
                } else {
                    state.set_selected_index(None, window, cx);
                }
            });
            self.cluster_select_items = items;
        }

        TitleBar::new()
            .bg(cx.theme().tokens.title_bar)
            .text_color(cx.theme().sidebar_foreground)
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .pr_3()
                    .gap_3()
                    .items_center()
                    .when(!cfg!(target_os = "macos"), |bar| {
                        bar.child(
                            div()
                                .w(px(280.0))
                                .h_full()
                                .child(crate::native_menu::create_menu_bar(cx)),
                        )
                    })
                    .justify_between()
                    .text_color(cx.theme().sidebar_foreground)
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .gap_1p5()
                            .child(
                                Button::new("nav-toggle-sidebar")
                                    .text_color(cx.theme().sidebar_foreground)
                                    .xsmall()
                                    .ghost()
                                    .icon(IconName::PanelLeft)
                                    .accessibility_label("Toggle sidebar")
                                    .tooltip("Toggle sidebar")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.sidebar_collapsed = !this.sidebar_collapsed;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("nav-back-btn")
                                    .text_color(cx.theme().sidebar_foreground)
                                    .xsmall()
                                    .ghost()
                                    .icon(IconName::ArrowLeft)
                                    .accessibility_label("Go back")
                                    .tooltip("Go back")
                                    .disabled(!self.history.can_go_back())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if let Some(route) = this.history.go_back() {
                                            this.navigate(route, false, window, cx);
                                        }
                                    })),
                            )
                            .child(
                                Button::new("nav-forward-btn")
                                    .text_color(cx.theme().sidebar_foreground)
                                    .xsmall()
                                    .ghost()
                                    .icon(IconName::ArrowRight)
                                    .accessibility_label("Go forward")
                                    .tooltip("Go forward")
                                    .disabled(!self.history.can_go_forward())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if let Some(route) = this.history.go_forward() {
                                            this.navigate(route, false, window, cx);
                                        }
                                    })),
                            )
                            .child(
                                h_flex()
                                    .id("titlebar-cluster-switcher")
                                    .ml_3()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        Button::new("app-brand-btn")
                                            .ghost()
                                            .xsmall()
                                            .text_color(cx.theme().sidebar_foreground)
                                            .label("icekube")
                                            .tooltip("Clusters Catalog")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.navigate(Route::Catalog, true, window, cx);
                                            })),
                                    )
                                    .when_some(selected_cluster, |bar, cluster| {
                                        bar.child(
                                            h_flex()
                                                .items_center()
                                                .gap_1p5()
                                                .child(
                                                    Button::new("titlebar-cluster-select-btn")
                                                        .ghost()
                                                        .xsmall()
                                                        .text_color(cx.theme().sidebar_foreground)
                                                        .icon(IconName::Network)
                                                        .label(cluster.name.clone())
                                                        .tooltip(format!(
                                                            "Cluster: {} ({}) · Click to switch",
                                                            cluster.name, cluster.context_name
                                                        ))
                                                        .on_click(cx.listener(|this, _, _, cx| {
                                                            this.search_modal_open =
                                                                !this.search_modal_open;
                                                            cx.notify();
                                                        })),
                                                )
                                                .child(
                                                    div()
                                                        .text_sm()
                                                        .text_color(cx.theme().muted_foreground)
                                                        .child("/"),
                                                )
                                                .child(
                                                    Button::new("titlebar-namespace-select-btn")
                                                        .ghost()
                                                        .xsmall()
                                                        .text_color(cx.theme().sidebar_foreground)
                                                        .icon(IconName::Folder)
                                                        .label(current_namespace.clone())
                                                        .tooltip(format!(
                                                            "Namespace: {} · Click to view namespaces",
                                                            current_namespace
                                                        ))
                                                        .on_click(cx.listener(
                                                            |this, _, window, cx| {
                                                                if let Route::Cluster {
                                                                    cluster_id,
                                                                    ..
                                                                } = this.history.current().clone() {
                                                                    this.navigate(
                                                                        Route::Cluster {
                                                                            cluster_id,
                                                                            section: "namespaces".into(),
                                                                        },
                                                                        true,
                                                                        window,
                                                                        cx,
                                                                    );
                                                                }
                                                            },
                                                        )),
                                                ),
                                        )
                                    })
                                    .when(selected_id.is_none(), |bar| {
                                        let section_name = match self.history.current() {
                                            Route::Catalog => "Clusters",
                                            Route::Secrets => "Vault Secrets",
                                            _ => "Select a cluster",
                                        };
                                        bar.child(
                                            h_flex()
                                                .items_center()
                                                .gap_1p5()
                                                .child(
                                                    div()
                                                        .text_sm()
                                                        .text_color(cx.theme().muted_foreground)
                                                        .child("/"),
                                                )
                                                .child(
                                                    Button::new("titlebar-section-label-btn")
                                                        .ghost()
                                                        .xsmall()
                                                        .text_color(cx.theme().sidebar_foreground)
                                                        .label(section_name)
                                                        .tooltip("Quick jump to cluster or section")
                                                        .on_click(cx.listener(|this, _, _, cx| {
                                                            this.search_modal_open =
                                                                !this.search_modal_open;
                                                            cx.notify();
                                                        })),
                                                ),
                                        )
                                    }),
                            ),
                    )
                    .child(
                        h_flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap_1p5()
                            .child(
                                Button::new("open-search-palette-btn")
                                    .text_color(cx.theme().sidebar_foreground)
                                    .xsmall()
                                    .ghost()
                                    .icon(IconName::Search)
                                    .accessibility_label("Quick jump")
                                    .tooltip("Quick jump")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.search_modal_open = !this.search_modal_open;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("titlebar-preferences-btn")
                                    .text_color(cx.theme().sidebar_foreground)
                                    .xsmall()
                                    .ghost()
                                    .icon(IconName::Settings)
                                    .accessibility_label("Settings")
                                    .tooltip("Settings")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_preferences(window, cx);
                                    })),
                            ),
                    ),
            )
    }
}
