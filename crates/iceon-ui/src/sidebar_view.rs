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
        "namespaces" => Layers2,
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

/// Children and API groups are text-only in the tree. Section parents have
/// icons, including Custom Resources; API groups use a fallback in the rail.
#[derive(Clone, Copy)]
enum RowKind<'a> {
    Leaf(&'a str),
    Child,
    Parent(&'a str),
    ApiGroup,
}
fn row_asset_icon(kind: RowKind<'_>, rail: bool) -> Option<AssetIconName> {
    use AssetIconName::*;
    match kind {
        RowKind::Leaf(section) => Some(section_asset_icon(section)),
        RowKind::Child => None,
        RowKind::ApiGroup => rail.then_some(Puzzle),
        RowKind::Parent("custom-resources") => Some(Puzzle),
        RowKind::Parent(section) => Some(match section {
            "workloads" => Layers,
            "config" => SlidersHorizontal,
            "network" => Network,
            "storage" => HardDrive,
            "helm" => Package,
            "access-control" => Shield,
            _ => Puzzle,
        }),
    }
}

fn parent_item(label: SharedString, children: Vec<SidebarMenuItem>) -> SidebarMenuItem {
    // Selection must never implicitly open a standard or discovered API group.
    SidebarMenuItem::new(label.clone())
        .accessibility_label(label)
        .default_open(false)
        .click_to_toggle(true)
        .children(children)
}

/// The pinned kit hides submenus in rail mode; it does not provide an automatic
/// dropdown. Activating a rail parent restores the tree and opens that parent.
#[derive(Clone)]
struct NavigationGroup {
    section: &'static str,
    label: SharedString,
    items: Vec<SidebarMenuItem>,
    collapsed: bool,
    rail_activate: SidebarClickHandler,
}
impl NavigationGroup {
    fn new(
        section: &'static str,
        label: impl Into<SharedString>,
        items: Vec<SidebarMenuItem>,
        rail_activate: SidebarClickHandler,
    ) -> Self {
        Self {
            section,
            label: label.into(),
            items,
            collapsed: false,
            rail_activate,
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
        let menu = if self.label.is_empty() {
            SidebarMenu::new().children(self.items)
        } else {
            let rail = self.collapsed;
            let handler = self.rail_activate;
            SidebarMenu::new().child(
                parent_item(self.label, self.items)
                    .when_some(
                        row_asset_icon(RowKind::Parent(self.section), rail),
                        |item, icon| item.icon(Icon::new(icon)),
                    )
                    .click_to_open(rail)
                    .on_click(move |event, window, cx| {
                        if rail {
                            handler(event, window, cx);
                        }
                    }),
            )
        };
        // Share the parent's keyed expansion state across tree and rail. The
        // kit's click_to_open sets it before the rail handler restores the tree.
        menu.collapsed(self.collapsed).render(id, window, cx)
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
                if !self.route_visible(group.default_child) {
                    continue;
                }
                let cid = cluster_id.to_owned();
                resources.push(menu_item(
                    format!("page-{group_id}"),
                    group.label,
                    row_asset_icon(RowKind::Leaf(group_id), false).map(Icon::new),
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
                        .filter(|child| self.route_visible(child.id))
                        .map(|child| {
                            let cid = cluster_id.to_owned();
                            let destination = child.id;
                            menu_item(
                                format!("child-{destination}"),
                                child.label,
                                row_asset_icon(RowKind::Child, false).map(Icon::new),
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
                        let mut children = Vec::new();
                        for definition in definitions {
                            let cid = cluster_id.to_owned();
                            let destination = format!("crd:{}", definition.name);
                            let selected = *active == destination;
                            let label = definition
                                .columns
                                .get("kind")
                                .cloned()
                                .unwrap_or_else(|| definition.name.clone());
                            children.push(menu_item(
                                format!("crd-nav-{}", definition.name),
                                label,
                                row_asset_icon(RowKind::Child, false).map(Icon::new),
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
                            ));
                        }
                        resources.push(
                            parent_item(api_group.into(), children)
                                .when_some(
                                    row_asset_icon(RowKind::ApiGroup, false),
                                    |item, icon| item.icon(Icon::new(icon)),
                                )
                                .active(contains_active),
                        );
                    }
                    if self.access_ready()
                        && self.route_allowed("custom-resource-definitions")
                        && self.custom_discovery_loading
                    {
                        resources.push(
                            menu_item(
                                "crd-loading",
                                "Discovering resource types…",
                                row_asset_icon(RowKind::Child, false).map(Icon::new),
                                false,
                                None,
                                |_, _, _| {},
                            )
                            .disable(true)
                            .when(!self.sidebar_collapsed, |item| item.pl(px(32.0))),
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
                                row_asset_icon(RowKind::Child, false).map(Icon::new),
                                false,
                                None,
                                cx.listener(|app, _, window, cx| {
                                    app.discover_custom_resources(window, cx)
                                }),
                            )
                            .when(!self.sidebar_collapsed, |item| item.pl(px(32.0))),
                        );
                    }
                }
            }
            if resources.is_empty() {
                continue;
            }
            groups.push(NavigationGroup::new(
                group_id,
                if leaf { "" } else { group.label },
                resources,
                Rc::new(cx.listener(|app, _, _, cx| {
                    app.sidebar_collapsed = false;
                    cx.notify();
                })),
            ));
        }

        let ext_registry = iceon_extension::ExtensionRegistry::new();
        let enabled_exts = &self.preferences_draft.app.enabled_extensions;
        let active_exts = ext_registry.enabled_extensions(enabled_exts);
        if !active_exts.is_empty() {
            let mut ext_resources = Vec::new();
            for ext in active_exts {
                for item in ext.sidebar_items {
                    if item.parent_id.is_none() {
                        let cid = cluster_id.to_owned();
                        let destination = format!("ext:{}", item.id);
                        let selected = active == &destination;
                        ext_resources.push(menu_item(
                            format!("ext-nav-{}", item.id),
                            item.title,
                            row_asset_icon(RowKind::Child, false).map(Icon::new),
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
                                );
                            }),
                        ));
                    }
                }
            }
            if !ext_resources.is_empty() {
                groups.push(NavigationGroup::new(
                    "extensions",
                    "Extensions",
                    ext_resources,
                    Rc::new(cx.listener(|app, _, _, cx| {
                        app.sidebar_collapsed = false;
                        cx.notify();
                    })),
                ));
            }
        }

        Sidebar::new(SharedString::from(format!(
            "cluster-navigation-{cluster_id}"
        )))
        .w_full()
        .border_r_1()
        .border_color(cx.theme().sidebar_border)
        .collapsible(true)
        .collapsed(self.sidebar_collapsed)
        .children(groups)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn children_are_iconless_and_section_parents_have_icons_in_both_modes() {
        use super::{RowKind, row_asset_icon};
        for rail in [false, true] {
            assert!(row_asset_icon(RowKind::Child, rail).is_none());
            for group in crate::navigation::CLUSTER_SECTIONS {
                if group.children.is_empty() {
                    assert!(row_asset_icon(RowKind::Leaf(group.id), rail).is_some());
                } else {
                    let icon = row_asset_icon(RowKind::Parent(group.id), rail).unwrap();
                    assert_ne!(icon, gpui_kit_assets::IconName::Square);
                }
            }
        }
        for rail in [false, true] {
            assert_eq!(
                row_asset_icon(RowKind::Parent("custom-resources"), rail),
                Some(gpui_kit_assets::IconName::Puzzle)
            );
        }
        assert!(row_asset_icon(RowKind::ApiGroup, false).is_none());
        assert!(row_asset_icon(RowKind::ApiGroup, true).is_some());
        assert!(row_asset_icon(RowKind::Parent("extensions"), true).is_some());
    }

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
