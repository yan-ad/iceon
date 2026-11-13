//! Compact workspace navigation. The component owns search, focus, keyboard navigation,
//! popup dismissal and accessibility; the application remains the authority for routing.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, IndexPath, Sizable as _, TitleBar,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::{DropdownMenu as _, PopupMenuItem},
    select::{SearchableVec, SelectEvent, SelectItem, SelectState},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use iceon_core::{catalog::CatalogSnapshot, connection::ResourceKind};

use crate::{IceonApp, navigation::Route};

fn namespace_choices(known: &[String]) -> Vec<Option<String>> {
    let mut choices = vec![None];
    for namespace in known {
        let choice = Some(namespace.clone());
        if !choices.contains(&choice) {
            choices.push(choice);
        }
    }
    choices
}

fn toggle_namespace(selection: &mut Vec<String>, namespace: Option<&str>) {
    let Some(namespace) = namespace else {
        selection.clear();
        return;
    };
    if let Some(index) = selection.iter().position(|selected| selected == namespace) {
        selection.remove(index);
    } else {
        selection.push(namespace.to_owned());
    }
    selection.sort();
}

fn namespace_label(selection: &[String]) -> String {
    match selection {
        [] => "All Namespaces".into(),
        [namespace] => namespace.clone(),
        selection => format!("{} Namespaces", selection.len()),
    }
}

#[cfg(test)]
mod namespace_scope_tests {
    #[test]
    fn choices_only_include_discovered_namespaces() {
        assert_eq!(super::namespace_choices(&[]), vec![None]);
        assert_eq!(
            super::namespace_choices(&["dev".into(), "dev".into(), "team".into()]),
            vec![None, Some("dev".into()), Some("team".into())]
        );
    }

    #[test]
    fn namespace_selection_toggles_and_empty_means_all() {
        let mut selection = Vec::new();
        super::toggle_namespace(&mut selection, Some("team"));
        super::toggle_namespace(&mut selection, Some("dev"));
        assert_eq!(selection, vec!["dev", "team"]);
        super::toggle_namespace(&mut selection, Some("team"));
        assert_eq!(selection, vec!["dev"]);
        super::toggle_namespace(&mut selection, None);
        assert!(selection.is_empty());
    }

    #[test]
    fn namespace_label_describes_multi_selection() {
        assert_eq!(super::namespace_label(&[]), "All Namespaces");
        assert_eq!(super::namespace_label(&["dev".into()]), "dev");
        assert_eq!(
            super::namespace_label(&["dev".into(), "team".into()]),
            "2 Namespaces"
        );
    }
}

/// IDs, rather than display names, identify contexts (names need not be unique).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ClusterChoice {
    id: String,
    name: String,
    context_name: String,
    color: String,
}

impl SelectItem for ClusterChoice {
    type Value = String;

    fn title(&self) -> SharedString {
        self.name.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }

    fn display_title(&self) -> Option<AnyElement> {
        Some(
            h_flex()
                .items_center()
                .gap_2()
                .min_w_0()
                .child(
                    div()
                        .size(px(10.0))
                        .flex_shrink_0()
                        .rounded_full()
                        .bg(cluster_dot(&self.color)),
                )
                .child(div().truncate().child(self.name.clone()))
                .into_any_element(),
        )
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
            color: cluster.color.clone(),
        })
        .collect()
}

fn opened_path(section: &str) -> Vec<(String, String)> {
    if let Some(name) = section.strip_prefix("crd:") {
        return vec![(name.into(), section.into())];
    }
    for group in crate::navigation::CLUSTER_SECTIONS {
        if let Some(child) = group.children.iter().find(|child| child.id == section) {
            return vec![(child.label.into(), child.id.into())];
        }
        if group.id == section {
            return vec![(group.label.into(), group.default_child.into())];
        }
    }
    vec![("Overview".into(), "overview".into())]
}

fn breadcrumb_separator(cx: &App) -> impl IntoElement {
    Icon::new(IconName::ChevronRight)
        .xsmall()
        .flex_shrink_0()
        .text_color(cx.theme().muted_foreground)
}

fn connection_indicator(status: &str, cx: &App) -> Icon {
    let (icon, color) = match status {
        "connected" => (IconName::Check, cx.theme().success),
        "connecting" => (IconName::Loader, cx.theme().warning),
        "error" => (IconName::TriangleAlert, cx.theme().danger),
        _ => (IconName::Close, cx.theme().muted_foreground),
    };
    Icon::new(icon).xsmall().text_color(color)
}

fn breadcrumb_choices(section: &str, group_level: bool) -> Vec<(String, String)> {
    if group_level {
        return crate::navigation::ordered_cluster_sections()
            .into_iter()
            .map(|group| (group.label.into(), group.default_child.into()))
            .collect();
    }
    crate::navigation::CLUSTER_SECTIONS
        .iter()
        .find(|group| {
            group.id == section
                || group.default_child == section
                || group.children.iter().any(|child| child.id == section)
        })
        .map(|group| {
            group
                .children
                .iter()
                .map(|child| (child.label.into(), child.id.into()))
                .collect()
        })
        .unwrap_or_else(|| vec![("Definitions".into(), "custom-resource-definitions".into())])
}

/// GKE's generated context is `gke_<project>_<location>_<cluster>`.
/// Only shorten the display label; routing and dropdown identity stay unchanged.
fn header_controls_width(sidebar: f32, open: bool, macos: bool) -> f32 {
    let inset = if macos { 80.0 } else { 12.0 };
    if open {
        (sidebar - inset).max(96.0)
    } else {
        96.0
    }
}

fn compact_cluster_name(name: &str) -> &str {
    name.strip_prefix("gke_")
        .and_then(|context| {
            let mut parts = context.splitn(3, '_');
            let project = parts.next()?;
            let location = parts.next()?;
            let cluster = parts.next()?;
            (!project.is_empty() && !location.is_empty() && !cluster.is_empty()).then_some(cluster)
        })
        .unwrap_or(name)
}

fn cluster_dot(color: &str) -> Hsla {
    let rgb = color
        .strip_prefix('#')
        .filter(|digits| digits.len() == 6)
        .and_then(|digits| u32::from_str_radix(digits, 16).ok());
    gpui_kit::rgb(rgb.unwrap_or(crate::shell_theme::PRIMARY_RGB)).into()
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
    fn toggle_header_namespace(
        &mut self,
        namespace: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        toggle_namespace(&mut self.selected_namespaces, namespace.as_deref());
        self.selected_namespace =
            (self.selected_namespaces.len() == 1).then(|| self.selected_namespaces[0].clone());
        self.current_continue_token = None;
        self.continue_token_history.clear();
        // reload_active_section invokes ensure_access for the new scope.
        self.reload_active_section(window, cx);
    }

    pub(crate) fn namespace_dropdown(
        &self,
        id: &'static str,
        label: String,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let owner = cx.entity().downgrade();
        Button::new(id)
            .ghost()
            .xsmall()
            .flex_shrink_0()
            .text_color(cx.theme().sidebar_foreground)
            .label(label)
            .accessibility_label("Choose namespace")
            .dropdown_menu(move |mut menu, _, cx| {
                if let Some(entity) = owner.upgrade() {
                    let app = entity.read(cx);
                    let cluster = app
                        .cluster_session
                        .as_ref()
                        .map(|session| (session.cluster_id.clone(), session.session_id.clone()));
                    for namespace in namespace_choices(&app.namespaces) {
                        let selected = match namespace.as_deref() {
                            None => app.selected_namespaces.is_empty(),
                            Some(namespace) => app
                                .selected_namespaces
                                .iter()
                                .any(|selected| selected == namespace),
                        };
                        let owner = owner.clone();
                        let cluster = cluster.clone();
                        menu = menu.item(
                            PopupMenuItem::new(
                                namespace.clone().unwrap_or_else(|| "All Namespaces".into()),
                            )
                            .checked(selected)
                            .on_click(move |_, window, cx| {
                                let _ = owner.update(cx, |app, cx| {
                                    let current = app.cluster_session.as_ref().map(|session| {
                                        (session.cluster_id.clone(), session.session_id.clone())
                                    });
                                    if current != cluster
                                        || namespace
                                            .as_ref()
                                            .is_some_and(|name| !app.namespaces.contains(name))
                                    {
                                        return;
                                    }
                                    app.toggle_header_namespace(namespace.clone(), window, cx);
                                });
                            }),
                        );
                    }
                    if app.namespaces.is_empty() {
                        menu =
                            menu.item(PopupMenuItem::new("No namespaces available").disabled(true));
                    }
                }
                menu
            })
    }

    fn header_cluster_metadata(&self, id: &str, fallback_status: &str) -> (String, String) {
        let health = self
            .cluster_health
            .iter()
            .find(|health| health.cluster_id == id);
        let session = self
            .cluster_session
            .as_ref()
            .filter(|session| session.cluster_id == id);
        let status = session
            .filter(|session| matches!(session.status, "connecting" | "error"))
            .map(|session| session.status)
            .or_else(|| health.map(|health| health.status))
            .unwrap_or(fallback_status)
            .to_owned();
        let version = self
            .cluster_versions
            .get(id)
            .map(String::as_str)
            .or_else(|| session.and_then(|session| session.api_version.as_deref()))
            .filter(|version| !version.trim().is_empty())
            .unwrap_or("—")
            .to_owned();
        (status, version)
    }

    pub(crate) fn render_title_bar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        if self.access_ready() && self.can_list(ResourceKind::Namespaces) {
            self.sync_namespace_select(window, cx);
        }
        let items = cluster_choices(&self.catalog_snapshot);
        let selected_id = match self.history.current() {
            Route::Cluster { cluster_id, .. } => items
                .iter()
                .find(|item| &item.id == cluster_id)
                .map(|item| item.id.clone()),
            Route::Setup | Route::Secrets => None,
        };
        let selected_cluster = items
            .iter()
            .find(|item| Some(&item.id) == selected_id.as_ref())
            .cloned();
        let current_namespace = namespace_label(&self.selected_namespaces);
        let sidebar_open = matches!(self.history.current(), Route::Cluster { .. })
            && !self.sidebar_collapsed
            && !self.cluster_sidebar_collapsed;
        let sidebar_width = self.navigation_widths.sidebar_width();

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

        div()
            .relative()
            .w_full()
            .flex_shrink_0()
            .child(crate::shell_theme::chrome_background(sidebar_open.then_some(sidebar_width), cx))
            .child(TitleBar::new()
            .relative()
            .bg(gpui_kit::transparent_black())
            .text_color(cx.theme().sidebar_foreground)
            .child(
                h_flex()
                    .w_full().h_full()
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
                            .flex_1().h_full()
                            .min_w_0()
                            .items_center()
                            .gap_0()
                            .child(h_flex().items_center().gap_1p5().h_full().flex_shrink_0().pr_2()
                                 .w(px(header_controls_width(sidebar_width, sidebar_open, cfg!(target_os = "macos"))))
                                .border_r_1().border_color(cx.theme().sidebar_border)
                            .when(matches!(self.history.current(), Route::Cluster { .. }), |navigation| navigation.child(
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
                            ))
                            .child(
                                Button::new("nav-back-btn")
                                    .text_color(cx.theme().sidebar_foreground)
                                    .xsmall()
                                    .ghost()
                                    .icon(IconName::ArrowLeft)
                                    .accessibility_label("Go back")
                                    .tooltip("Go back")
                                    .disabled(!self.history.can_go_back())
                                    .when(!self.history.can_go_back(), |button| {
                                        button.opacity(0.4).cursor_not_allowed()
                                    })
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
                                    .when(!self.history.can_go_forward(), |button| {
                                        button.opacity(0.4).cursor_not_allowed()
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if let Some(route) = this.history.go_forward() {
                                            this.navigate(route, false, window, cx);
                                        }
                                    })),
                            )
                            .child(div().flex_1())
                            .child(
                                Button::new("open-search-palette-btn")
                                    .text_color(cx.theme().sidebar_foreground)
                                    .xsmall()
                                    .ghost()
                                    .icon(IconName::Search)
                                    .accessibility_label("Quick jump")
                                    .tooltip("Quick jump")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.toggle_command_palette(window, cx);
                                    })),
                            )
                            )
                            .when(!matches!(self.history.current(), Route::Setup), |navigation| navigation.child(
                                h_flex()
                                    .id("titlebar-cluster-switcher")
                                    .min_w_0()
                                    .pl_3()
                                    .items_center()
                                    .gap_2()
                                    .when_some(selected_cluster, |bar, cluster| {
                                        let fallback = self.catalog_snapshot.clusters.iter().find(|item| item.id == cluster.id).map(|item| item.status).unwrap_or("disconnected");
                                        let (status, version) = self.header_cluster_metadata(&cluster.id, fallback);
                                        bar.child(
                                            h_flex()
                                                .id("workspace-breadcrumb")
                                                .role(gpui_kit::accesskit::Role::Navigation)
                                                .aria_label("Workspace breadcrumb")
                                                .min_w_0()
                                                .items_center()
                                                .gap_2()
                                                .child(
                                                    Button::new("titlebar-cluster-select")
                                                        .ghost().xsmall().min_w_0().max_w(px(340.0))
                                                        .text_color(cx.theme().sidebar_foreground)
                                                        .child(h_flex().items_center().gap_2().min_w_0()
                                                            .child(div().min_w_0().max_w(px(180.0)).truncate().child(compact_cluster_name(&cluster.name).to_owned()))
                                                            .child(h_flex().flex_shrink_0().items_center().gap_1()
                                                                .child(connection_indicator(&status, cx))
                                                                .child(div().text_xs().max_w(px(140.0)).truncate().text_color(cx.theme().muted_foreground).child(version.clone()))))
                                                        .tooltip(format!("{} · {} · {}", cluster.name, status, version))
                                                        .accessibility_label("Switch cluster")
                                                        .dropdown_menu({
                                                            let owner = cx.entity().downgrade();
                                                            move |mut menu, _, cx| {
                                                                if let Some(app) = owner.upgrade() {
                                                                    let app = app.read(cx);
                                                                    for item in &app.catalog_snapshot.clusters {
                                                                        let id = item.id.clone();
                                                                        let selected = matches!(app.history.current(), Route::Cluster { cluster_id, .. } if *cluster_id == id);
                                                                        let owner = owner.clone();
                                                                        let name = item.name.clone();
                                                                        let (status, version) = app.header_cluster_metadata(&item.id, item.status);
                                                                        menu = menu.item(PopupMenuItem::element(move |_, cx| {
                                                                            v_flex().w(px(360.0)).min_w_0().max_w(px(392.0)).gap_1()
                                                                                .child(div().w_full().truncate().child(name.clone()))
                                                                                .child(h_flex().items_center().gap_2().text_xs().text_color(cx.theme().muted_foreground)
                                                                                    .child(connection_indicator(&status, cx))
                                                                                    .child(div().truncate().child(format!("Version {version}"))))
                                                                        }).checked(selected).on_click(move |_, window, cx| {
                                                                            let _ = owner.update(cx, |app, cx| app.navigate(Route::Cluster { cluster_id: id.clone(), section: "overview".into() }, true, window, cx));
                                                                        }));
                                                                    }
                                                                }
                                                                let config_owner = owner.clone();
                                                                menu.separator().item(PopupMenuItem::new("Add Config…").on_click(move |_, window, cx| {
                                                                    let _ = config_owner.update(cx, |app, cx| app.open_add_config(window, cx));
                                                                }))
                                                            }
                                                        }),
                                                )
                                                .children(opened_path(&self.active_cluster_section).into_iter().enumerate().map(|(index, (label, section))| {
                                                    let cluster_id = cluster.id.clone();
                                                    h_flex().id(("breadcrumb-path", index)).items_center().gap_2()
                                                        .child(breadcrumb_separator(cx))
                                                        // Use the same button size as the cluster and namespace:
                                                        // BreadcrumbItem otherwise inherits the larger title-bar font.
                                                        .child(Button::new(("titlebar-breadcrumb", index))
                                                            .ghost()
                                                            .xsmall()
                                                            .label(label)
                                                            .text_color(cx.theme().sidebar_foreground)
                                                            .dropdown_menu({
                                                                let owner = cx.entity().downgrade();
                                                                let choices: Vec<_> = breadcrumb_choices(&self.active_cluster_section, index == 0 && opened_path(&self.active_cluster_section).len() > 1).into_iter().filter(|(_, destination)| self.route_visible(destination)).collect();
                                                                move |mut menu, _, _| {
                                                                    for (label, destination) in &choices {
                                                                        let owner = owner.clone();
                                                                        let cluster_id = cluster_id.clone();
                                                                        let destination = destination.clone();
                                                                        menu = menu.item(PopupMenuItem::new(label.clone()).checked(destination == section).on_click(move |_, window, cx| {
                                                                            let _ = owner.update(cx, |app, cx| { if app.route_visible(&destination) { app.navigate(Route::Cluster { cluster_id: cluster_id.clone(), section: destination.clone() }, true, window, cx); } });
                                                                        }));
                                                                    }
                                                                     menu
                                                                }
                                                            }))
                                                }))
                                                .child(breadcrumb_separator(cx))
                                                 .child(
                                                     self.namespace_dropdown("titlebar-namespace-select", current_namespace.clone(), cx),
                                                 )
                                                 .child(self.render_status_breadcrumb(cx)),
                                        )
                                    })
                                    .when(selected_id.is_none(), |bar| {
                                        let section_name = match self.history.current() {
                                            Route::Setup => "Cluster setup",
                                            _ => "Select a cluster",
                                        };
                                        bar.child(
                                            h_flex()
                                                .items_center()
                                                .gap_1p5()
                                                .child(breadcrumb_separator(cx))
                                                .child(
                                                    Button::new("titlebar-section-label-btn")
                                                        .ghost()
                                                        .xsmall()
                                                        .text_color(cx.theme().sidebar_foreground)
                                                        .label(section_name)
                                                        .tooltip("Quick jump to cluster or section")
                                                        .on_click(cx.listener(|this, _, window, cx| {
                                                            this.toggle_command_palette(window, cx);
                                                        })),
                                                ),
                                        )
                                    }),
                            ))
                    )
                    .child(
                        h_flex()
                            .flex_shrink_0().h_full()
                            .items_center()
                            .gap_1p5()
                            .child(self.render_header_resource_controls(window, cx)),
                    ),
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::cluster_choices;
    #[test]
    fn header_split_tracks_sidebar_width_and_native_inset() {
        assert_eq!(super::header_controls_width(252., true, true) + 80., 252.);
        assert_eq!(super::header_controls_width(420., true, true) + 80., 420.);
        assert_eq!(super::header_controls_width(252., true, false) + 12., 252.);
        assert_eq!(super::header_controls_width(252., false, true), 96.);
    }

    #[test]
    fn expanded_sidebar_and_breadcrumbs_share_overview_first_and_group_labels() {
        let groups = crate::navigation::CLUSTER_SECTIONS;
        assert_eq!(groups[0].id, "overview");
        assert_eq!(
            super::opened_path("overview"),
            vec![("Overview".into(), "overview".into())]
        );
        assert_eq!(super::opened_path("config-maps")[0].0, "Config Maps");
        assert_eq!(super::opened_path("services")[0].0, "Services");
        assert_eq!(super::breadcrumb_choices("pods", false)[0].1, "pods");
    }

    #[test]
    fn compact_gke_labels_preserve_cluster_name_and_other_contexts() {
        assert_eq!(
            super::compact_cluster_name(
                "gke_perfect-impulse-344607_asia-southeast2-a_kaj-prd-cluster"
            ),
            "kaj-prd-cluster"
        );
        assert_eq!(
            super::compact_cluster_name("gke_project_zone_cluster_extra"),
            "cluster_extra"
        );
        for name in [
            "yanuar@kubernetes",
            "production",
            "gke_project",
            "gke_project_zone_",
            "gke__zone_cluster",
        ] {
            assert_eq!(super::compact_cluster_name(name), name);
        }
    }
    #[test]
    fn breadcrumb_menus_match_sidebar_groups_and_workload_pages() {
        let groups = super::breadcrumb_choices("pods", true);
        assert!(groups.iter().any(|(label, _)| label == "Workloads"));
        let pages = super::breadcrumb_choices("pods", false);
        assert!(pages.iter().any(|(_, section)| section == "pods"));
        assert!(pages.iter().any(|(_, section)| section == "deployments"));
        assert!(!pages.iter().any(|(_, section)| section == "nodes"));
    }

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
    fn breadcrumbs_show_resource_path_and_track_cluster_color() {
        assert_eq!(
            super::opened_path("pods"),
            vec![("Pods".into(), "pods".into())]
        );
        assert_eq!(
            super::opened_path("nodes"),
            vec![("Nodes".into(), "nodes".into())]
        );
        let mut catalog = snapshot();
        let before = cluster_choices(&catalog);
        catalog.clusters[0].color = "#ff0000".into();
        assert_ne!(before, cluster_choices(&catalog));
        assert_eq!(
            super::cluster_dot("#ff0000"),
            gpui_kit::Hsla::from(gpui_kit::rgb(0xff0000))
        );
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
