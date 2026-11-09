//! Catalog-metadata navigation built from gpui-kit controls.
use std::rc::Rc;

use gpui_kit::component::{
    ActiveTheme as _, IconName, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    scroll::ScrollableElement as _,
    sidebar::{Sidebar, SidebarGroup, SidebarMenu, SidebarMenuItem},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::{
    IceonApp,
    navigation::{CLUSTER_SECTIONS, Route, destination_label, normalize_destination},
};
/// Catalog colors are user-controlled metadata. Only accept an exact RGB hex
/// triplet (with the conventional `#`), never CSS, paths or arbitrary strings.
fn cluster_rgb(value: &str) -> Option<u32> {
    let digits = value.strip_prefix('#')?;
    if digits.len() != 6 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(digits, 16).ok()
}

fn cluster_initials(name: &str) -> String {
    let mut words = name
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| !word.is_empty());
    let Some(first) = words.next() else {
        return "K".into();
    };
    let mut letters = first.chars();
    let mut initials = String::new();
    if let Some(ch) = letters.next() {
        initials.extend(ch.to_uppercase().take(1));
    }
    if let Some(ch) = words
        .next()
        .and_then(|word| word.chars().next())
        .or_else(|| letters.next())
    {
        initials.extend(ch.to_uppercase().take(1));
    }
    initials
}

fn section_icon(section: &str) -> IconName {
    match section {
        "overview" => IconName::LayoutDashboard,
        "nodes" => IconName::Cpu,
        "workloads" => IconName::GalleryVerticalEnd,
        "config" => IconName::Settings2,
        "network" => IconName::Network,
        "storage" => IconName::HardDrive,
        "namespaces" => IconName::Folder,
        "events" => IconName::Bell,
        "helm" => IconName::Inbox,
        "access-control" => IconName::User,
        "custom-resources" => IconName::FileText,
        _ => IconName::Square,
    }
}

/// SidebarMenuItem 0.9 supplies tree-item semantics but its row is mouse-only.
/// A standard ghost Button suffix provides Tab / Enter / Space activation too,
/// without reimplementing a button or forking the library's sidebar control.
fn menu_item(
    label: impl Into<SharedString>,
    icon: Option<IconName>,
    active: bool,
    action_icon: IconName,
    handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> SidebarMenuItem {
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
            let handler = handler.clone();
            Button::new("activate")
                .ghost()
                .text_color(foreground)
                .xsmall()
                .icon(action_icon.clone())
                .accessibility_label(label.clone())
                .tooltip(label.clone())
                .on_click(move |event, window, cx| {
                    cx.stop_propagation();
                    handler(event, window, cx);
                })
        })
}

impl IceonApp {
    /// Always visible, even when the resource sidebar is hidden.
    pub(crate) fn render_primary_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let route = self.history.current().clone();
        v_flex()
            .w(px(60.0))
            .flex_shrink_0()
            .h_full()
            .bg(cx.theme().tokens.sidebar)
            .text_color(cx.theme().sidebar_foreground)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .py_2()
            .gap_2()
            .items_center()
            .child(Button::new("rail-catalog").ghost().small().size(px(40.0))
                .selected(matches!(route, Route::Catalog))
                .text_color(cx.theme().sidebar_foreground)
                .icon(IconName::LayoutDashboard)
                .accessibility_label("Clusters Catalog").tooltip("Clusters Catalog")
                .on_click(cx.listener(|this, _, window, cx| {
                    this.navigate(Route::Catalog, true, window, cx);
                })))
            .child(Button::new("rail-vault").ghost().small().size(px(40.0))
                .selected(matches!(route, Route::Secrets))
                .text_color(cx.theme().sidebar_foreground)
                .icon(IconName::Globe)
                .accessibility_label("Vault Secrets").tooltip("Vault Secrets")
                .on_click(cx.listener(|this, _, window, cx| {
                    this.navigate(Route::Secrets, true, window, cx);
                })))
            .child(v_flex().flex_1().min_h_0().w_full().gap_2().items_center()
                .overflow_y_scrollbar()
                // Every cluster is reachable, whether or not it has pinned resources.
                .children(self.catalog_snapshot.clusters.iter().map(|cluster| {
                    let cid = cluster.id.clone();
                    let selected = matches!(&route, Route::Cluster { cluster_id, .. } if cluster_id == &cid);
                    // This is user metadata, not the shell's brand/accent color.
                    let marker = cluster_rgb(&cluster.color);
                    Button::new(SharedString::from(format!("rail-cluster-{cid}")))
                        .ghost().small().size(px(40.0)).selected(selected)
                        .text_color(cx.theme().sidebar_foreground)
                        .label(cluster_initials(&cluster.name))
                        .when_some(marker, |button, marker| button.border_color(rgb(marker)))
                        .accessibility_label(format!("{}: {}", cluster.name, cluster.status))
                        .tooltip(format!("{} · {}", cluster.name, cluster.status))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.navigate(Route::Cluster { cluster_id: cid.clone(), section: "overview".into() }, true, window, cx);
                        }))
                })))
            .child(Button::new("rail-preferences").ghost().small().size(px(40.0))
                .selected(self.preferences_open)
                .text_color(cx.theme().sidebar_foreground)
                .icon(IconName::Settings)
                .accessibility_label("Preferences").tooltip("Preferences")
                .on_click(cx.listener(|this, _, window, cx| this.open_preferences(window, cx))))
    }

    pub(crate) fn render_cluster_sidebar(
        &self,
        cluster_id: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active = &self.active_cluster_section;
        let favorites = self
            .catalog_snapshot
            .pinned_items
            .iter()
            .filter(|pin| pin.id == cluster_id)
            .map(|pin| {
                let destination = pin.section.clone().unwrap_or_else(|| "overview".into());
                let label = if destination == "overview" {
                    "Cluster".into()
                } else {
                    destination_label(&destination)
                };
                let cid = cluster_id.to_owned();
                menu_item(
                    label,
                    Some(IconName::Star),
                    *active == normalize_destination(&destination),
                    IconName::ChevronRight,
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
        if !favorites.is_empty() {
            groups
                .push(SidebarGroup::new("Favorites").child(SidebarMenu::new().children(favorites)));
        }
        let mut resources = Vec::new();
        for group in CLUSTER_SECTIONS {
            let group_id = group.id;
            let expanded = self.expanded_cluster_groups.iter().any(|id| id == group_id);
            let leaf = group.children.is_empty();
            let cid = cluster_id.to_owned();
            resources.push(menu_item(
                if group_id == "overview" {
                    "Cluster"
                } else {
                    group.label
                },
                Some(section_icon(group_id)),
                leaf && active == group.default_child,
                if expanded && !leaf {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                },
                cx.listener(move |this, _, window, cx| {
                    if leaf {
                        this.navigate(
                            Route::Cluster {
                                cluster_id: cid.clone(),
                                section: group.default_child.into(),
                            },
                            true,
                            window,
                            cx,
                        );
                    } else {
                        // App-owned independent expansion, not the component's initial-only default_open.
                        if let Some(index) = this
                            .expanded_cluster_groups
                            .iter()
                            .position(|id| id == group_id)
                        {
                            this.expanded_cluster_groups.remove(index);
                        } else {
                            this.expanded_cluster_groups.push(group_id.into());
                        }
                        cx.notify();
                    }
                }),
            ));
            if expanded && !leaf {
                resources.extend(group.children.iter().map(|child| {
                    let cid = cluster_id.to_owned();
                    let destination = child.id;
                    menu_item(
                        child.label,
                        None,
                        active == destination,
                        IconName::ChevronRight,
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
                    .pl_6()
                }));
            }
        }
        groups.push(SidebarGroup::new("Resources").child(SidebarMenu::new().children(resources)));
        Sidebar::new(SharedString::from(format!(
            "cluster-navigation-{cluster_id}"
        )))
        .w(px(252.0))
        .collapsible(false)
        .children(groups)
    }
}
#[cfg(test)]
mod tests {
    use super::{cluster_initials, cluster_rgb};

    #[test]
    fn colors_accept_only_rgb_hex_metadata() {
        assert_eq!(cluster_rgb("#4ba7a5"), Some(0x4ba7a5));
        assert_eq!(cluster_rgb("#AABBCC"), Some(0xaabbcc));
        for invalid in [
            "red",
            "4ba7a5",
            "#fff",
            "#4ba7a5ff",
            "#zzzzzz",
            "#4ba7a5 ",
            "url(secret)",
        ] {
            assert_eq!(cluster_rgb(invalid), None);
        }
    }

    #[test]
    fn initials_are_compact_and_unicode_safe() {
        assert_eq!(cluster_initials("production-east"), "PE");
        assert_eq!(cluster_initials("local"), "LO");
        assert_eq!(cluster_initials("東京"), "東京");
        assert_eq!(cluster_initials(" -- "), "K");
    }
}
