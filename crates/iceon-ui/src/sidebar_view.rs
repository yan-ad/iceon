//! Catalog-metadata navigation built from gpui-kit controls.
use std::rc::Rc;

use gpui_kit::component::{
    ActiveTheme as _, Collapsible as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    sidebar::{Sidebar, SidebarItem, SidebarMenu, SidebarMenuItem},
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use gpui_kit_assets::IconName as AssetIconName;

use crate::{IceonApp, navigation::Route};

pub(crate) fn section_asset_icon(section: &str) -> AssetIconName {
    use AssetIconName::*;
    match section {
        "overview" => LayoutDashboard,
        "nodes" => Server,
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

/// Retain native tree semantics and a keyboard-accessible expander for dynamic groups.
fn menu_item(
    id_key: impl Into<SharedString>,
    label: impl Into<SharedString>,
    icon: Option<Icon>,
    active: bool,
    action_icon: Option<IconName>,
    handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> SidebarMenuItem {
    let id_key = id_key.into();
    let label = label.into();
    let handler: SidebarClickHandler = Rc::new(handler);
    SidebarMenuItem::new(label.clone())
        .accessibility_label(label.clone())
        .active(active)
        .when_some(icon, |item, icon| item.icon(icon))
        .on_click({
            let handler = handler.clone();
            move |event, window, cx| handler(event, window, cx)
        })
        .suffix(move |_, cx| {
            let handler = handler.clone();
            h_flex().when_some(action_icon.clone(), |row, icon| {
                row.child(
                    Button::new(SharedString::from(format!("activate-{id_key}")))
                        .ghost()
                        .xsmall()
                        .icon(icon)
                        .text_color(cx.theme().sidebar_foreground)
                        .accessibility_label(label.clone())
                        .on_click(move |event, window, cx| {
                            cx.stop_propagation();
                            handler(event, window, cx);
                        }),
                )
            })
        })
}

impl IceonApp {
    pub(crate) fn render_cluster_sidebar(
        &self,
        cluster_id: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active = &self.active_cluster_section;
        let mut groups = Vec::new();
        for group in crate::navigation::ordered_cluster_sections() {
            let mut resources = Vec::new();
            let group_id = group.id;
            let leaf = group.children.is_empty();
            if leaf {
                if !self.access_ready() || !self.route_allowed(group.default_child) {
                    continue;
                }
                let cid = cluster_id.to_owned();
                resources.push(menu_item(
                    format!("page-{group_id}"),
                    group.label,
                    Some(section_icon(group_id)),
                    active == group.default_child,
                    None,
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
                resources.extend(
                    group
                        .children
                        .iter()
                        .filter(|child| self.access_ready() && self.route_allowed(child.id))
                        .map(|child| {
                            let cid = cluster_id.to_owned();
                            let destination = child.id;
                            menu_item(
                                format!("child-{destination}"),
                                child.label,
                                Some(section_icon(destination)),
                                active == destination,
                                None,
                                cx.listener(move |app, _, window, cx| {
                                    app.navigate(
                                        Route::Cluster {
                                            cluster_id: cid.clone(),
                                            section: destination.into(),
                                        },
                                        true,
                                        window,
                                        cx,
                                    )
                                }),
                            )
                        }),
                );

                if group_id == "custom-resources" {
                    for (api_group, definitions) in
                        crate::custom_resource_navigation::grouped_definitions(
                            self.custom_definitions_page.as_ref(),
                        )
                    {
                        let definitions: Vec<_> = definitions
                            .into_iter()
                            .filter(|definition| {
                                self.access_ready()
                                    && self.route_allowed(&format!("crd:{}", definition.name))
                            })
                            .collect();
                        if definitions.is_empty() {
                            continue;
                        }
                        let contains_active = definitions
                            .iter()
                            .any(|definition| *active == format!("crd:{}", definition.name));
                        let expanded = self.expanded_custom_groups.contains(&api_group);
                        let toggle = api_group.clone();
                        resources.push(
                            menu_item(
                                format!("crd-group-{api_group}"),
                                api_group,
                                None,
                                contains_active,
                                Some(if expanded {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                }),
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
                                    menu_item(
                                        format!("crd-nav-{}", definition.name),
                                        label,
                                        None,
                                        selected,
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
                    if self.access_ready()
                        && self.route_allowed("custom-resource-definitions")
                        && self.custom_discovery_loading
                    {
                        resources.push(
                            menu_item(
                                "crd-loading",
                                "Discovering resource types…",
                                None,
                                false,
                                None,
                                |_, _, _| {},
                            )
                            .pl(px(32.0)),
                        );
                    }
                    if self.access_ready()
                        && self.route_allowed("custom-resource-definitions")
                        && self.custom_discovery_error.is_some()
                    {
                        resources.push(
                            menu_item(
                                "crd-retry",
                                "Discovery failed / incomplete · Retry",
                                None,
                                false,
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
            if resources.is_empty() {
                continue;
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
