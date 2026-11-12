//! Catalog-metadata navigation built from gpui-kit controls.
use std::rc::Rc;

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    sidebar::{Sidebar, SidebarGroup, SidebarMenu, SidebarMenuItem},
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

const LIGHT_INK_RGB: u32 = 0xffffff;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ClusterTilePalette {
    pub(crate) bg_rgb: u32,
    pub(crate) fg_rgb: u32,
    pub(crate) hover_rgb: u32,
    pub(crate) active_rgb: u32,
    pub(crate) border_rgb: u32,
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

/// Deterministic fallback RGB when a cluster record has no valid hex color.
fn fallback_cluster_rgb(seed: &str) -> u32 {
    const PALETTE: &[u32] = &[
        0x681848, // Deep plum (GPI reference)
        0x6d1a36, // Deep crimson (YK reference)
        0x1d4e78, // Deep navy (KI reference)
        0x1f5f5b, // Deep teal
        0x4b2e83, // Deep indigo
        0x2e5a36, // Deep forest
        0x7a3e18, // Deep amber-brown
        0x2b4570, // Slate blue
    ];
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in seed.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    PALETTE[(hash as usize) % PALETTE.len()]
}

fn srgb_channel_to_linear(channel: u8) -> f64 {
    let normalized = f64::from(channel) / 255.0;
    if normalized <= 0.04045 {
        normalized / 12.92
    } else {
        ((normalized + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG 2.1 relative luminance in `0.0..=1.0`.
fn wcag_relative_luminance(color: u32) -> f64 {
    let r = srgb_channel_to_linear(((color >> 16) & 0xff) as u8);
    let g = srgb_channel_to_linear(((color >> 8) & 0xff) as u8);
    let b = srgb_channel_to_linear((color & 0xff) as u8);
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

fn wcag_contrast_ratio(first: u32, second: u32) -> f64 {
    let l1 = wcag_relative_luminance(first);
    let l2 = wcag_relative_luminance(second);
    let (lighter, darker) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
    (lighter + 0.05) / (darker + 0.05)
}

fn rgb_to_hsl(color: u32) -> (f64, f64, f64) {
    let r = f64::from((color >> 16) & 0xff) / 255.0;
    let g = f64::from((color >> 8) & 0xff) / 255.0;
    let b = f64::from(color & 0xff) / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) * 0.5;
    let delta = max - min;
    if delta.abs() < 1e-6 {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 {
        delta / (2.0 - max - min)
    } else {
        delta / (max + min)
    };
    let mut h = if (max - r).abs() < 1e-6 {
        (g - b) / delta + if g < b { 6.0 } else { 0.0 }
    } else if (max - g).abs() < 1e-6 {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    h /= 6.0;
    (h, s, l)
}

fn hue_to_rgb(p: f64, q: f64, mut t: f64) -> f64 {
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        return p + (q - p) * 6.0 * t;
    }
    if t < 1.0 / 2.0 {
        return q;
    }
    if t < 2.0 / 3.0 {
        return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
    }
    p
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> u32 {
    let s = s.clamp(0.0, 1.0);
    let l = l.clamp(0.0, 1.0);
    let (r, g, b) = if s.abs() < 1e-6 {
        (l, l, l)
    } else {
        let q = if l < 0.5 {
            l * (1.0 + s)
        } else {
            l + s - l * s
        };
        let p = 2.0 * l - q;
        (
            hue_to_rgb(p, q, h + 1.0 / 3.0),
            hue_to_rgb(p, q, h),
            hue_to_rgb(p, q, h - 1.0 / 3.0),
        )
    };
    let r_u8 = (r * 255.0).round().clamp(0.0, 255.0) as u32;
    let g_u8 = (g * 255.0).round().clamp(0.0, 255.0) as u32;
    let b_u8 = (b * 255.0).round().clamp(0.0, 255.0) as u32;
    (r_u8 << 16) | (g_u8 << 8) | b_u8
}

/// Solid cluster surfaces retain the catalog hue and meet WCAG AAA (7:1)
/// with the same ink in idle, hover and pressed states.
pub(crate) fn cluster_tile_palette(color: &str, seed: &str, is_dark: bool) -> ClusterTilePalette {
    let base = cluster_rgb(color).unwrap_or_else(|| fallback_cluster_rgb(seed));
    let (h, s, l) = rgb_to_hsl(base);
    let saturation = if s < 0.06 { s } else { s.clamp(0.42, 0.75) };
    let lightness = l.clamp(0.22, if is_dark { 0.38 } else { 0.44 });
    let fg_rgb = LIGHT_INK_RGB;
    let accessible = |mut lightness: f64| {
        for _ in 0..=100 {
            let color = hsl_to_rgb(h, saturation, lightness);
            if wcag_contrast_ratio(color, fg_rgb) >= 7.0 {
                return color;
            }
            lightness = (lightness - 0.01).max(0.0);
        }
        0x000000
    };
    ClusterTilePalette {
        bg_rgb: accessible(lightness),
        fg_rgb,
        hover_rgb: accessible(lightness - 0.025),
        active_rgb: accessible(lightness - 0.05),
        border_rgb: accessible(lightness),
    }
}

pub(crate) fn cluster_badge_color(color: &str, seed: &str, is_dark: bool) -> Hsla {
    rgb(cluster_tile_palette(color, seed, is_dark).bg_rgb).into()
}

fn section_icon(section: &str) -> IconName {
    match section {
        "overview" => IconName::LayoutDashboard,
        "nodes" => IconName::Cpu,
        "workloads"
        | "pods"
        | "deployments"
        | "daemon-sets"
        | "stateful-sets"
        | "replica-sets"
        | "replication-controllers" => IconName::GalleryVerticalEnd,
        "workloads-overview" => IconName::LayoutDashboard,
        "jobs" | "cron-jobs" => IconName::Inbox,
        "config-maps" => IconName::FileText,
        "secrets" => IconName::FileText,
        "services" | "ingresses" | "endpoints" | "endpoint-slices" | "network-policies" => {
            IconName::Network
        }
        "persistent-volumes" | "persistent-volume-claims" | "storage-classes" => {
            IconName::HardDrive
        }
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

type SidebarClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// SidebarMenuItem 0.9 supplies tree-item semantics but its row is mouse-only.
/// A standard ghost Button suffix provides Tab / Enter / Space activation and
/// optional section pin toggling without forking the library's sidebar control.
#[allow(clippy::too_many_arguments)]
fn menu_item_with_pin(
    id_key: impl Into<SharedString>,
    label: impl Into<SharedString>,
    icon: Option<IconName>,
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
                    Some(IconName::Star),
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
        groups.push(
            SidebarGroup::new("").child(SidebarMenu::new().child(menu_item_with_pin(
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
            ))),
        );
        if !favorites.is_empty() {
            groups
                .push(SidebarGroup::new("Favorites").child(SidebarMenu::new().children(favorites)));
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
            groups.push(
                SidebarGroup::new(if leaf { "" } else { group.label })
                    .child(SidebarMenu::new().children(resources)),
            );
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
    use super::{LIGHT_INK_RGB, cluster_rgb, cluster_tile_palette, wcag_contrast_ratio};

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
    fn tile_states_meet_aaa_across_the_rgb_gamut() {
        for red in (0..=255).step_by(51) {
            for green in (0..=255).step_by(51) {
                for blue in (0..=255).step_by(51) {
                    let color = format!("#{red:02x}{green:02x}{blue:02x}");
                    for dark in [false, true] {
                        let palette = cluster_tile_palette(&color, "gamut", dark);
                        for surface in [palette.bg_rgb, palette.hover_rgb, palette.active_rgb] {
                            assert!(
                                wcag_contrast_ratio(surface, palette.fg_rgb) >= 7.0,
                                "{color}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn cluster_tile_palette_maintains_wcag_aaa_contrast_in_dark_and_light_modes() {
        for is_dark in [true, false] {
            for sample in [
                "#000000", "#ffffff", "#ffff00", "#681848", "#6d1a36", "#1d4e78", "#4ba7a5",
                "#121214", "#f6f6f7", "invalid",
            ] {
                let palette = cluster_tile_palette(sample, "cluster-seed", is_dark);
                for surface in [palette.bg_rgb, palette.hover_rgb, palette.active_rgb] {
                    let ratio = wcag_contrast_ratio(surface, palette.fg_rgb);
                    assert!(
                        ratio >= 7.0,
                        "expected WCAG AAA >= 7.0 contrast for {sample} (is_dark={is_dark}), got {ratio:.2}"
                    );
                    assert_eq!(palette.fg_rgb, LIGHT_INK_RGB);
                }
            }
        }
    }
}
