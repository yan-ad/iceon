//! Compact, keyboard-accessible workspace rail and cluster navigation.
//!
//! This view consumes only the catalog's safe metadata projection; selecting a
//! destination delegates connection/session ownership to `IceonApp::navigate`.

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonCustomVariant, ButtonVariants as _},
    h_flex,
    scroll::ScrollableElement as _,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::{
    IceonApp,
    navigation::{CLUSTER_SECTIONS, Route, destination_label, normalize_destination},
};

#[derive(Clone, Copy)]
struct SidebarColors {
    rail: Hsla,
    pane: Hsla,
    foreground: Hsla,
    muted: Hsla,
    border: Hsla,
    hover: Hsla,
    selected: Hsla,
    selected_foreground: Hsla,
}

impl SidebarColors {
    fn new(cx: &App) -> Self {
        let theme = cx.theme();
        if theme.mode.is_dark() {
            Self {
                rail: rgb(0x25292b).into(),
                pane: rgb(0x34383c).into(),
                foreground: rgb(0xd8dcdf).into(),
                muted: rgb(0x9ca5ab).into(),
                border: rgb(0x202427).into(),
                hover: rgb(0x41464b).into(),
                selected: rgb(0x4ba7a5).into(),
                selected_foreground: rgb(0xffffff).into(),
            }
        } else {
            Self {
                rail: theme.sidebar,
                pane: theme.background,
                foreground: theme.sidebar_foreground,
                muted: theme.muted_foreground,
                border: theme.border,
                hover: theme.sidebar_accent,
                selected: rgb(0x287e7c).into(),
                selected_foreground: rgb(0xffffff).into(),
            }
        }
    }

    fn button(self, cx: &App, selected: bool) -> ButtonCustomVariant {
        ButtonCustomVariant::new(cx)
            .foreground(if selected {
                self.selected_foreground
            } else {
                self.foreground
            })
            .color(if selected {
                self.selected
            } else {
                cx.theme().transparent
            })
            .hover(if selected { self.selected } else { self.hover })
            .active(self.selected)
    }
}

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

/// A full-width child, rather than Button::label, avoids the component's
/// intentionally centered internal label layout while retaining its focus,
/// Enter/Space activation, tooltip and accessibility implementations.
fn navigation_button(
    id: String,
    label: String,
    icon: Option<IconName>,
    selected: bool,
    colors: SidebarColors,
    cx: &App,
) -> Button {
    Button::new(SharedString::from(id))
        .small()
        .custom(colors.button(cx, selected))
        .w_full()
        .h(px(30.0))
        .px_2()
        .rounded(px(3.0))
        .accessibility_label(label.clone())
        .tooltip(label.clone())
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .gap_2()
                .justify_start()
                .when_some(icon, |row, icon| {
                    row.child(Icon::new(icon).size(px(15.0)).text_color(if selected {
                        colors.selected_foreground
                    } else {
                        colors.muted
                    }))
                })
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .text_ellipsis()
                        .child(label),
                ),
        )
}

impl IceonApp {
    /// Always present: collapsing the wide navigation must not hide this rail.
    pub(crate) fn render_primary_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = SidebarColors::new(cx);
        let route = self.history.current().clone();
        v_flex()
            .w(px(60.0))
            .flex_shrink_0()
            .h_full()
            .bg(colors.rail)
            .text_color(colors.foreground)
            .border_r_1()
            .border_color(colors.border)
            .py_2()
            .gap_2()
            .items_center()
            .child(
                Button::new("rail-catalog")
                    .small()
                    .custom(colors.button(cx, matches!(route, Route::Catalog)))
                    .size(px(40.0))
                    .icon(IconName::LayoutDashboard)
                    .accessibility_label("Clusters Catalog")
                    .tooltip("Clusters Catalog")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.navigate(Route::Catalog, true, window, cx);
                    })),
            )
            .child(
                Button::new("rail-vault")
                    .small()
                    .custom(colors.button(cx, matches!(route, Route::Secrets)))
                    .size(px(40.0))
                    .icon(IconName::Globe)
                    .accessibility_label("Vault Secrets")
                    .tooltip("Vault Secrets")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.navigate(Route::Secrets, true, window, cx);
                    })),
            )
            .child(div().w(px(32.0)).h(px(1.0)).bg(colors.border))
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .overflow_y_scrollbar()
                    // Include every cluster, not just the pinned subset.
                    .children(self.catalog_snapshot.clusters.iter().map(|cluster| {
                        let cid = cluster.id.clone();
                        let selected = matches!(&route, Route::Cluster { cluster_id, .. } if cluster_id == &cid);
                        let color = cluster_rgb(&cluster.color).unwrap_or(0x52616b);
                        let tile: Hsla = rgb(color).into();
                        let luminance = ((color >> 16) & 255) * 299
                            + ((color >> 8) & 255) * 587 + (color & 255) * 114;
                        let ink: Hsla = rgb(if luminance > 140_000 { 0x172126 } else { 0xffffff }).into();
                        let status: Hsla = match cluster.status {
                            "connected" => rgb(0x73c9a0).into(),
                            "connecting" => rgb(0xe3bd6a).into(),
                            "error" => rgb(0xdc8585).into(),
                            _ => colors.muted,
                        };
                        h_flex()
                            .relative()
                            .w_full()
                            .justify_center()
                            .when(selected, |row| row.child(
                                div().absolute().left_0().top(px(8.0)).bottom(px(8.0))
                                    .w(px(3.0)).bg(colors.selected),
                            ))
                            .child(
                                Button::new(SharedString::from(format!("rail-cluster-{cid}")))
                                    .small()
                                    .size(px(38.0))
                                    .rounded(px(6.0))
                                    .custom(ButtonCustomVariant::new(cx)
                                        .color(tile).foreground(ink).hover(tile).active(tile))
                                    .border_1()
                                    .border_color(if selected { colors.selected } else { tile })
                                    .accessibility_label(cluster.name.clone())
                                    .tooltip(format!("{} · {}", cluster.name, cluster.status))
                                    .child(
                                        div().relative().size_full().flex().items_center().justify_center()
                                            .text_sm().font_semibold().child(cluster_initials(&cluster.name))
                                            .child(div().absolute().right_0().bottom_0()
                                                .size(px(7.0)).rounded_full().bg(status)
                                                .border_1().border_color(colors.rail)),
                                    )
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.navigate(Route::Cluster {
                                            cluster_id: cid.clone(), section: "overview".into(),
                                        }, true, window, cx);
                                    })),
                            )
                    })),
            )
            .child(
                Button::new("rail-preferences")
                    .small()
                    .size(px(40.0))
                    .custom(colors.button(cx, self.preferences_open))
                    .icon(IconName::Settings)
                    .accessibility_label("Preferences")
                    .tooltip("Preferences")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_preferences(window, cx);
                    })),
            )
    }

    pub(crate) fn render_cluster_sidebar(
        &self,
        cluster_id: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = SidebarColors::new(cx);
        let active = self.active_cluster_section.clone();
        let cid = cluster_id.to_owned();
        let favorites = self
            .catalog_snapshot
            .pinned_items
            .iter()
            .filter(|pin| pin.id == cluster_id)
            .collect::<Vec<_>>();

        v_flex()
            .w(px(252.0))
            .flex_shrink_0()
            .h_full()
            .min_h_0()
            .bg(colors.pane)
            .text_color(colors.foreground)
            .border_r_1()
            .border_color(colors.border)
            .p_2()
            .gap_0p5()
            .overflow_y_scrollbar()
            // The title-bar selector owns the cluster heading; do not repeat it.
            .when(!favorites.is_empty(), |pane| {
                pane.child(
                    v_flex()
                        .gap_0p5()
                        .pb_2()
                        .mb_1()
                        .border_b_1()
                        .border_color(colors.border)
                        .child(
                            div()
                                .px_2()
                                .py_1()
                                .text_xs()
                                .font_semibold()
                                .text_color(colors.muted)
                                .child("Favorites"),
                        )
                        .children(favorites.into_iter().enumerate().map(|(index, pin)| {
                            let destination =
                                pin.section.clone().unwrap_or_else(|| "overview".into());
                            let label = if destination == "overview" {
                                "Cluster".into()
                            } else {
                                destination_label(&destination)
                            };
                            let cid = cid.clone();
                            navigation_button(
                                format!("cluster-favorite-{index}"),
                                label,
                                Some(IconName::Star),
                                active == normalize_destination(&destination),
                                colors,
                                cx,
                            )
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.navigate(
                                        Route::Cluster {
                                            cluster_id: cid.clone(),
                                            section: destination.clone(),
                                        },
                                        true,
                                        window,
                                        cx,
                                    );
                                },
                            ))
                        })),
                )
            })
            .children(CLUSTER_SECTIONS.iter().map(|group| {
                let group_id = group.id;
                let expanded = self.expanded_cluster_groups.iter().any(|id| id == group_id);
                let leaf = group.children.is_empty();
                let cid_group = cid.clone();
                v_flex()
                    .gap_0p5()
                    .child(
                        navigation_button(
                            format!("cluster-nav-grp-{group_id}"),
                            if group_id == "overview" {
                                "Cluster".into()
                            } else {
                                group.label.into()
                            },
                            Some(section_icon(group_id)),
                            leaf && active == group.default_child,
                            colors,
                            cx,
                        )
                        .when(!leaf, |button| {
                            button.toggled(expanded).child(
                                Icon::new(if expanded {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .size(px(12.0))
                                .text_color(colors.muted),
                            )
                        })
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                if leaf {
                                    this.navigate(
                                        Route::Cluster {
                                            cluster_id: cid_group.clone(),
                                            section: group.default_child.into(),
                                        },
                                        true,
                                        window,
                                        cx,
                                    );
                                } else {
                                    // Expansion is independent of navigation; multiple groups
                                    // may remain open like the FreeLens navigation rail.
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
                            },
                        )),
                    )
                    .when(expanded && !leaf, |section| {
                        section.child(v_flex().pl_5().gap_0p5().children(
                            group.children.iter().map(|child| {
                                let cid = cid.clone();
                                let destination = child.id;
                                navigation_button(
                                    format!("cluster-nav-child-{destination}"),
                                    child.label.into(),
                                    None,
                                    active == destination,
                                    colors,
                                    cx,
                                )
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        this.navigate(
                                            Route::Cluster {
                                                cluster_id: cid.clone(),
                                                section: destination.into(),
                                            },
                                            true,
                                            window,
                                            cx,
                                        );
                                    },
                                ))
                            }),
                        ))
                    })
            }))
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
