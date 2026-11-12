//! Catalog-metadata navigation built from gpui-kit controls.
use std::rc::Rc;

use gpui_kit::component::{
    ActiveTheme as _, Collapsible as _, Icon, IconName, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    sidebar::{Sidebar, SidebarItem, SidebarMenu, SidebarMenuItem},
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use gpui_kit_assets::IconName as AssetIconName;
use iceon_core::catalog::SectionPinRequest;

use crate::{
    IceonApp,
    navigation::{
        CLUSTER_SECTIONS, Route, catalog_error_message, destination_label, normalize_destination,
    },
};

fn section_asset_icon(section: &str) -> AssetIconName {
    use AssetIconName::*;
    match section {
        "overview" => LayoutDashboard,
        "nodes" => Server,
        "workloads-overview" => Gauge,
        "pods" => Box,
        "deployments" => Layers,
        "daemon-sets" => Network,
        "stateful-sets" => Database,
        "replica-sets" => Copy,
        "replication-controllers" => CopyCheck,
        "jobs" => ListChecks,
        "cron-jobs" => CalendarClock,
        "config-maps" => FileText,
        "secrets" => KeyRound,
        "resource-quotas" => ChartPie,
        "limit-ranges" => SlidersHorizontal,
        "horizontal-pod-autoscalers" => ArrowLeftRight,
        "vertical-pod-autoscalers" => ArrowUpDown,
        "pod-disruption-budgets" => ShieldCheck,
        "priority-classes" => ListOrdered,
        "runtime-classes" => Container,
        "leases" => Timer,
        "mutating-webhook-configs" => Webhook,
        "validating-webhook-configs" => BadgeCheck,
        "validating-admission-policies" => FileCheck,
        "validating-admission-policy-bindings" => ClipboardCheck,
        "services" => Plug,
        "endpoint-slices" => Blocks,
        "endpoints" => MapPin,
        "ingresses" => Route,
        "ingress-classes" => Signpost,
        "network-policies" => Shield,
        "port-forwarding" => Cable,
        "persistent-volume-claims" => HardDriveDownload,
        "persistent-volumes" => HardDrive,
        "storage-classes" => Archive,
        "namespaces" => Folder,
        "events" => Bell,
        "helm-charts" => Package,
        "helm-releases" => PackageCheck,
        "users" => Users,
        "service-accounts" => Contact,
        "cluster-roles" => Crown,
        "roles" => UserCog,
        "cluster-role-bindings" => Link,
        "role-bindings" => Link2,
        "pod-security-policies" => ShieldAlert,
        "custom-resource-definitions" | "custom-resources" => Puzzle,
        _ => Square,
    }
}

fn section_icon(section: &str) -> Icon {
    Icon::new(section_asset_icon(section))
}

/// Unlike SidebarGroup, standalone navigation menus reserve no empty heading slot.
#[derive(Clone)]
struct NavigationGroup {
    label: SharedString,
    menu: SidebarMenu,
    collapsed: bool,
}
impl NavigationGroup {
    fn new(label: impl Into<SharedString>, menu: SidebarMenu) -> Self {
        Self {
            label: label.into(),
            menu,
            collapsed: false,
        }
    }
}
impl gpui_kit::component::Collapsible for NavigationGroup {
    fn is_collapsed(&self) -> bool {
        self.collapsed
    }
    fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }
}
impl SidebarItem for NavigationGroup {
    fn render(
        self,
        id: impl Into<ElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        let id = id.into();
        gpui_kit::component::v_flex()
            .gap_1()
            .when(!self.label.is_empty() && !self.collapsed, |group| {
                group.child(
                    div()
                        .px_2()
                        .pt_2()
                        .pb_1()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(cx.theme().muted_foreground)
                        .child(self.label.clone()),
                )
            })
            .child(self.menu.collapsed(self.collapsed).render(id, window, cx))
    }
}

type SidebarClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// SidebarMenuItem 0.9 supplies tree-item semantics but its row is mouse-only.
/// A standard ghost Button suffix provides Tab / Enter / Space activation and
/// optional section pin toggling without forking the library's sidebar control.
#[allow(clippy::too_many_arguments)]
fn menu_item_with_pin(
    id_key: impl Into<SharedString>,
    label: impl Into<SharedString>,
    icon: Option<Icon>,
    active: bool,
    action_icon: Option<IconName>,
    pin_state: Option<(bool, SidebarClickHandler)>,
    handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> SidebarMenuItem {
    let id_key = id_key.into();
    let label = label.into();
    let handler = Rc::new(handler);
    SidebarMenuItem::new(label.clone())
        .accessibility_label(label.clone())
        .active(active)
        .when_some(icon, |item, icon| item.icon(icon))
        .on_click({
            let handler = handler.clone();
            move |event, window, cx| handler(event, window, cx)
        })
        .suffix(move |_, cx| {
            let foreground = cx.theme().sidebar_foreground;
            let muted = cx.theme().muted_foreground;
            let handler = handler.clone();
            let pin_state = pin_state.clone();
            let id_key = id_key.clone();
            let label = label.clone();
            h_flex()
                .items_center()
                .justify_end()
                .when_some(action_icon.clone(), |row, icon| {
                    row.child(
                        Button::new(SharedString::from(format!("activate-{id_key}")))
                            .ghost()
                            .text_color(foreground)
                            .xsmall()
                            .icon(icon)
                            .accessibility_label(label.clone())
                            .tooltip(label.clone())
                            .on_click(move |event, window, cx| {
                                cx.stop_propagation();
                                handler(event, window, cx);
                            }),
                    )
                })
                .when_some(pin_state, |row, (pinned, pin_handler)| {
                    let pin_tooltip: SharedString = if pinned {
                        format!("Unpin {label} from Favorites").into()
                    } else {
                        format!("Pin {label} to Favorites").into()
                    };
                    row.child(
                        Button::new(SharedString::from(format!("pin-{id_key}")))
                            .ghost()
                            .xsmall()
                            .selected(pinned)
                            .text_color(if pinned { foreground } else { muted })
                            .icon(
                                Icon::new(if pinned {
                                    AssetIconName::PinOff
                                } else {
                                    AssetIconName::Pin
                                })
                                .small(),
                            )
                            .accessibility_label(pin_tooltip.clone())
                            .tooltip(pin_tooltip)
                            .on_click(move |event, window, cx| {
                                cx.stop_propagation();
                                pin_handler(event, window, cx);
                            }),
                    )
                })
        })
}

impl IceonApp {
    pub(crate) fn toggle_section_pin(
        &mut self,
        cluster_id: String,
        section: String,
        pinned: bool,
        cx: &mut Context<Self>,
    ) {
        match self.catalog.set_section_pin(SectionPinRequest {
            cluster_id,
            section,
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

    pub(crate) fn render_cluster_sidebar(
        &self,
        cluster_id: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active = &self.active_cluster_section;
        let cluster_summary = self
            .catalog_snapshot
            .clusters
            .iter()
            .find(|c| c.id == cluster_id);
        let is_section_pinned = |sec: &str| -> bool {
            let Some(cluster) = cluster_summary else {
                return false;
            };
            if sec == "overview" {
                cluster.pinned
            } else {
                cluster.pinned_sections.iter().any(|s| s == sec)
            }
        };

        let favorites = self
            .catalog_snapshot
            .pinned_items
            .iter()
            .filter(|pin| pin.id == cluster_id)
            .map(|pin| {
                let destination = pin.section.clone().unwrap_or_else(|| "overview".into());
                let label: SharedString = if destination == "overview" {
                    "Overview".into()
                } else {
                    destination_label(&destination).into()
                };
                let cid = cluster_id.to_owned();
                let unpin_cid = cluster_id.to_owned();
                let unpin_dest = destination.clone();
                let unpin_handler: SidebarClickHandler =
                    Rc::new(cx.listener(move |this, _, _, cx| {
                        this.toggle_section_pin(unpin_cid.clone(), unpin_dest.clone(), false, cx);
                    }));
                menu_item_with_pin(
                    format!("fav-{destination}"),
                    label,
                    Some(Icon::new(IconName::Star)),
                    *active == normalize_destination(&destination),
                    None,
                    Some((true, unpin_handler)),
                    cx.listener(move |this, _, window, cx| {
                        this.navigate(
                            Route::Cluster {
                                cluster_id: cid.clone(),
                                section: destination.clone(),
                            },
                            true,
                            window,
                            cx,
                        );
                    }),
                )
            })
            .collect::<Vec<_>>();
        let mut groups = Vec::new();
        let cid = cluster_id.to_owned();
        let pin_cid = cluster_id.to_owned();
        let overview_pinned = is_section_pinned("overview");
        groups.push(NavigationGroup::new(
            "",
            SidebarMenu::new().child(menu_item_with_pin(
                "overview-first",
                "Overview",
                Some(section_icon("overview")),
                active == "overview",
                None,
                Some((
                    overview_pinned,
                    Rc::new(cx.listener(move |app, _, _, cx| {
                        app.toggle_section_pin(
                            pin_cid.clone(),
                            "overview".into(),
                            !overview_pinned,
                            cx,
                        )
                    })),
                )),
                cx.listener(move |app, _, window, cx| {
                    app.navigate(
                        Route::Cluster {
                            cluster_id: cid.clone(),
                            section: "overview".into(),
                        },
                        true,
                        window,
                        cx,
                    )
                }),
            )),
        ));
        if !favorites.is_empty() {
            groups.push(NavigationGroup::new(
                "Favorites",
                SidebarMenu::new().children(favorites),
            ));
        }
        for group in CLUSTER_SECTIONS
            .iter()
            .filter(|group| group.id != "overview")
        {
            let mut resources = Vec::new();
            let group_id = group.id;
            let leaf = group.children.is_empty();
            if leaf {
                let cid = cluster_id.to_owned();
                let pin_cid = cluster_id.to_owned();
                let pinned = is_section_pinned(group.default_child);
                resources.push(menu_item_with_pin(
                    format!("grp-{group_id}"),
                    group.label,
                    Some(section_icon(group_id)),
                    active == group.default_child,
                    None,
                    Some((
                        pinned,
                        Rc::new(cx.listener(move |app, _, _, cx| {
                            app.toggle_section_pin(
                                pin_cid.clone(),
                                group.default_child.into(),
                                !pinned,
                                cx,
                            )
                        })),
                    )),
                    cx.listener(move |app, _, window, cx| {
                        app.navigate(
                            Route::Cluster {
                                cluster_id: cid.clone(),
                                section: group.default_child.into(),
                            },
                            true,
                            window,
                            cx,
                        )
                    }),
                ));
            } else {
                resources.extend(group.children.iter().map(|child| {
                    let cid = cluster_id.to_owned();
                    let destination = child.id;
                    let pinned = is_section_pinned(destination);
                    let pin_cid = cluster_id.to_owned();
                    let pin_sec = destination.to_owned();
                    let pin_handler: SidebarClickHandler =
                        Rc::new(cx.listener(move |this, _, _, cx| {
                            this.toggle_section_pin(pin_cid.clone(), pin_sec.clone(), !pinned, cx);
                        }));
                    menu_item_with_pin(
                        format!("child-{destination}"),
                        child.label,
                        Some(section_icon(child.id)),
                        active == destination,
                        None,
                        Some((pinned, pin_handler)),
                        cx.listener(move |this, _, window, cx| {
                            this.navigate(
                                Route::Cluster {
                                    cluster_id: cid.clone(),
                                    section: destination.into(),
                                },
                                true,
                                window,
                                cx,
                            );
                        }),
                    )
                }));

                if group_id == "custom-resources" {
                    for (api_group, definitions) in
                        crate::custom_resource_navigation::grouped_definitions(
                            self.custom_definitions_page.as_ref(),
                        )
                    {
                        let contains_active = definitions
                            .iter()
                            .any(|definition| *active == format!("crd:{}", definition.name));
                        let expanded = self.expanded_custom_groups.contains(&api_group);
                        let toggle = api_group.clone();
                        resources.push(
                            menu_item_with_pin(
                                format!("crd-group-{api_group}"),
                                api_group,
                                None,
                                contains_active,
                                Some(if expanded {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                }),
                                None,
                                cx.listener(move |app, _, _, cx| {
                                    if let Some(index) = app
                                        .expanded_custom_groups
                                        .iter()
                                        .position(|group| group == &toggle)
                                    {
                                        app.expanded_custom_groups.remove(index);
                                    } else {
                                        app.expanded_custom_groups.push(toggle.clone());
                                    }
                                    cx.notify();
                                }),
                            )
                            .pl(px(32.0)),
                        );
                        if expanded {
                            for definition in definitions {
                                let cid = cluster_id.to_owned();
                                let destination = format!("crd:{}", definition.name);
                                let selected = *active == destination;
                                let label = definition
                                    .columns
                                    .get("kind")
                                    .cloned()
                                    .unwrap_or_else(|| definition.name.clone());
                                resources.push(
                                    menu_item_with_pin(
                                        format!("crd-nav-{}", definition.name),
                                        label,
                                        None,
                                        selected,
                                        None,
                                        None,
                                        cx.listener(move |app, _, window, cx| {
                                            app.navigate(
                                                Route::Cluster {
                                                    cluster_id: cid.clone(),
                                                    section: destination.clone(),
                                                },
                                                true,
                                                window,
                                                cx,
                                            )
                                        }),
                                    )
                                    .pl(px(48.0)),
                                );
                            }
                        }
                    }
                    if self.custom_discovery_loading {
                        resources.push(
                            menu_item_with_pin(
                                "crd-loading",
                                "Discovering resource types…",
                                None,
                                false,
                                None,
                                None,
                                |_, _, _| {},
                            )
                            .pl(px(32.0)),
                        );
                    }
                    if self.custom_discovery_error.is_some() {
                        resources.push(
                            menu_item_with_pin(
                                "crd-retry",
                                "Discovery failed / incomplete · Retry",
                                None,
                                false,
                                None,
                                None,
                                cx.listener(|app, _, window, cx| {
                                    app.discover_custom_resources(window, cx)
                                }),
                            )
                            .pl(px(32.0)),
                        );
                    }
                }
            }
            groups.push(NavigationGroup::new(
                if leaf { "" } else { group.label },
                SidebarMenu::new().children(resources),
            ));
        }
        Sidebar::new(SharedString::from(format!(
            "cluster-navigation-{cluster_id}"
        )))
        .w_full()
        .border_r_1()
        .border_color(cx.theme().sidebar_border)
        .collapsible(false)
        .children(groups)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn standard_sidebar_destinations_have_distinct_icons() {
        let mut icons = std::collections::HashMap::new();
        for group in crate::navigation::CLUSTER_SECTIONS {
            if group.id == "custom-resources" {
                continue;
            }
            let destinations: Vec<_> = if group.children.is_empty() {
                vec![group.default_child]
            } else {
                group.children.iter().map(|child| child.id).collect()
            };
            for destination in destinations {
                let icon = super::section_asset_icon(destination);
                assert_ne!(
                    icon,
                    gpui_kit_assets::IconName::Square,
                    "missing icon: {destination}"
                );
                assert!(
                    icons.insert(icon, destination).is_none(),
                    "duplicate icon: {destination}"
                );
            }
        }
    }
}
