//! Catalog-metadata navigation built from gpui-kit controls.
use std::rc::Rc;

use gpui_kit::component::{
    ActiveTheme as _, IconName, Selectable as _, Sizable as _, StyledExt as _,
    button::{Button, ButtonCustomVariant, ButtonVariants as _},
    h_flex,
    scroll::ScrollableElement as _,
    sidebar::{Sidebar, SidebarGroup, SidebarMenu, SidebarMenuItem},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use iceon_core::catalog::SectionPinRequest;

use crate::{
    IceonApp,
    navigation::{
        CLUSTER_SECTIONS, Route, catalog_error_message, destination_label, normalize_destination,
    },
};

const LIGHT_INK_RGB: u32 = 0xffffff;
const DARK_INK_RGB: u32 = 0x18181b;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ClusterTilePalette {
    pub(crate) bg_rgb: u32,
    pub(crate) fg_rgb: u32,
    pub(crate) hover_rgb: u32,
    pub(crate) active_rgb: u32,
    pub(crate) border_rgb: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DragCluster(pub String);

pub(crate) struct ClusterDragPreview {
    pub(crate) label: SharedString,
    pub(crate) bg_rgb: u32,
    pub(crate) fg_rgb: u32,
}

impl Render for ClusterDragPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size(px(40.0))
            .rounded(px(8.0))
            .bg(rgb(self.bg_rgb))
            .text_color(rgb(self.fg_rgb))
            .flex()
            .items_center()
            .justify_center()
            .text_xs()
            .font_semibold()
            .shadow_md()
            .child(self.label.clone())
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
    let initial = hsl_to_rgb(h, saturation, lightness);
    let fg_rgb = if wcag_contrast_ratio(initial, LIGHT_INK_RGB)
        >= wcag_contrast_ratio(initial, DARK_INK_RGB)
    {
        LIGHT_INK_RGB
    } else {
        DARK_INK_RGB
    };
    let accessible = |mut lightness: f64| {
        for _ in 0..=100 {
            let color = hsl_to_rgb(h, saturation, lightness);
            if wcag_contrast_ratio(color, fg_rgb) >= 7.0 {
                return color;
            }
            lightness =
                (lightness + if fg_rgb == LIGHT_INK_RGB { -0.01 } else { 0.01 }).clamp(0.0, 1.0);
        }
        if fg_rgb == LIGHT_INK_RGB {
            0x000000
        } else {
            0xffffff
        }
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

fn cluster_initials(name: &str) -> String {
    let words: Vec<&str> = name
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let Some(&first) = words.first() else {
        return "K".into();
    };
    if words.len() >= 2 {
        let mut initials = String::new();
        for word in words.into_iter().take(3) {
            if let Some(ch) = word.chars().next() {
                initials.extend(ch.to_uppercase().take(1));
            }
        }
        return initials;
    }
    let mut letters = first.chars();
    let mut initials = String::new();
    if let Some(ch) = letters.next() {
        initials.extend(ch.to_uppercase().take(1));
    }
    if let Some(ch) = letters.next() {
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
/// A standard ghost Button suffix provides Tab / Enter / Space activation and
/// optional section pin toggling without forking the library's sidebar control.
#[allow(clippy::too_many_arguments)]
fn menu_item_with_pin(
    id_key: impl Into<SharedString>,
    label: impl Into<SharedString>,
    icon: Option<IconName>,
    active: bool,
    action_icon: Option<IconName>,
    pin_state: Option<(bool, Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>)>,
    handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> SidebarMenuItem {
    let id_key = id_key.into();
    let label = label.into();
    let handler = Rc::new(handler);
    SidebarMenuItem::new(if action_icon.is_some() {
        label.clone()
    } else {
        "".into()
    })
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
            .w_full()
            .justify_between()
            .child(
                Button::new(SharedString::from(format!("activate-{id_key}")))
                    .ghost()
                    .text_color(foreground)
                    .xsmall()
                    .when_some(action_icon.clone(), |button, icon| button.icon(icon))
                    .when(action_icon.is_none(), |button| {
                        button
                            .label(label.clone())
                            .flex_1()
                            .justify_start()
                            .text_left()
                    })
                    .accessibility_label(label.clone())
                    .tooltip(label.clone())
                    .on_click(move |event, window, cx| {
                        cx.stop_propagation();
                        handler(event, window, cx);
                    }),
            )
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
                        .icon(IconName::Star)
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

    /// The hotbar is the narrow cluster rail, not the resource sidebar. Retain
    /// an 8px hover target when auto-hidden; the full rail remains independent
    /// of resource-sidebar visibility.
    pub(crate) fn render_primary_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let route = self.history.current().clone();
        let is_dark = cx.theme().mode.is_dark();
        let disconnected_dot_fill: Hsla = if is_dark {
            rgb(0x27272a).into()
        } else {
            rgb(0xd4d4d8).into()
        };
        let disconnected_dot_border: Hsla = if is_dark {
            rgb(0x18181b).into()
        } else {
            rgb(0x71717a).into()
        };

        let rail = v_flex()
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
            .child(
                Button::new("rail-catalog")
                    .ghost()
                    .small()
                    .size(px(40.0))
                    .selected(matches!(route, Route::Catalog))
                    .text_color(cx.theme().sidebar_foreground)
                    .icon(IconName::LayoutDashboard)
                    .accessibility_label("Clusters Catalog")
                    .tooltip("Clusters Catalog")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.navigate(Route::Catalog, true, window, cx);
                    })),
            )
            .child(div().w(px(32.0)).h(px(1.0)).bg(cx.theme().sidebar_border))
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .gap_2()
                    .py_1()
                    .items_center()
                    .overflow_y_scrollbar()
                    .children({
                        let mut ordered = Vec::new();
                        for id in &self.cluster_order {
                            if let Some(c) =
                                self.catalog_snapshot.clusters.iter().find(|c| &c.id == id)
                            {
                                ordered.push(c.clone());
                            }
                        }
                        for c in &self.catalog_snapshot.clusters {
                            if !self.cluster_order.contains(&c.id) {
                                ordered.push(c.clone());
                            }
                        }
                        let sidebar_foreground = cx.theme().sidebar_foreground;
                        let success = cx.theme().success;
                        let warning = cx.theme().warning;
                        let danger = cx.theme().danger;
                        ordered
                            .into_iter()
                            .map(|cluster| {
                                let cid = cluster.id.clone();
                                let selected = matches!(
                                    &route,
                                    Route::Cluster { cluster_id, .. } if cluster_id == &cid
                                );
                                let palette =
                                    cluster_tile_palette(&cluster.color, &cluster.id, is_dark);
                                let tile_bg: Hsla = rgb(palette.bg_rgb).into();
                                let tile_fg: Hsla = rgb(palette.fg_rgb).into();
                                let tile_hover: Hsla = rgb(palette.hover_rgb).into();
                                let tile_active: Hsla = rgb(palette.active_rgb).into();
                                let tile_border: Hsla = if selected {
                                    sidebar_foreground
                                } else {
                                    rgb(palette.border_rgb).into()
                                };

                                let (status_fill, status_border): (Hsla, Hsla) =
                                    match cluster.status {
                                        "connected" => (success, disconnected_dot_border),
                                        "connecting" => (warning, disconnected_dot_border),
                                        "error" => (danger, disconnected_dot_border),
                                        _ => (disconnected_dot_fill, disconnected_dot_border),
                                    };

                                let raw_initials = cluster_initials(&cluster.name);
                                let is_three_chars = raw_initials.chars().count() >= 3;
                                let initials: SharedString = raw_initials.into();
                                let preview_label = initials.clone();

                                div()
                                    .id(SharedString::from(format!("rail-cluster-drag-{cid}")))
                                    .w_full()
                                    .flex()
                                    .justify_center()
                                    .on_drag(DragCluster(cid.clone()), move |_, _, _, cx| {
                                        cx.new(|_| ClusterDragPreview {
                                            label: preview_label.clone(),
                                            bg_rgb: palette.bg_rgb,
                                            fg_rgb: palette.fg_rgb,
                                        })
                                    })
                                    .drag_over::<DragCluster>(|style, _, _, cx| {
                                        style.rounded(px(8.0)).bg(cx.theme().tokens.sidebar_accent)
                                    })
                                    .on_drop(cx.listener({
                                        let target_id = cid.clone();
                                        move |this, dragged: &DragCluster, window, cx| {
                                            this.reorder_rail_cluster(
                                                &dragged.0, &target_id, window, cx,
                                            );
                                        }
                                    }))
                                    .child(
                                        h_flex()
                                            .relative()
                                            .w_full()
                                            .justify_center()
                                            .when(selected, |row| {
                                                row.child(
                                                    div()
                                                        .absolute()
                                                        .left_0()
                                                        .top(px(8.0))
                                                        .bottom(px(8.0))
                                                        .w(px(3.0))
                                                        .rounded_r(px(2.0))
                                                        .bg(cx.theme().sidebar_foreground),
                                                )
                                            })
                                            .child(
                                                div()
                                                    .relative()
                                                    .size(px(40.0))
                                                    .child(
                                                        Button::new(SharedString::from(format!(
                                                            "rail-cluster-{cid}"
                                                        )))
                                                        .small()
                                                        .size(px(40.0))
                                                        .rounded(px(8.0))
                                                        .border_1()
                                                        .border_color(tile_border)
                                                        .custom(
                                                            ButtonCustomVariant::new(cx)
                                                                .color(tile_bg)
                                                                .foreground(tile_fg)
                                                                .hover(tile_hover)
                                                                .active(tile_active),
                                                        )
                                                        .accessibility_label(format!(
                                                            "{}: {}",
                                                            cluster.name, cluster.status
                                                        ))
                                                        .tooltip(format!(
                                                            "{} · {}",
                                                            cluster.name, cluster.status
                                                        ))
                                                        .child(
                                                            div()
                                                                .relative()
                                                                .size_full()
                                                                .flex()
                                                                .items_center()
                                                                .justify_center()
                                                                .when(is_three_chars, |el| {
                                                                    el.text_xs()
                                                                })
                                                                .when(!is_three_chars, |el| {
                                                                    el.text_sm()
                                                                })
                                                                .font_semibold()
                                                                .child(initials),
                                                        )
                                                        .on_click(cx.listener(
                                                            move |this, _, window, cx| {
                                                                this.navigate(
                                                                    Route::Cluster {
                                                                        cluster_id: cid.clone(),
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
                                                        div()
                                                            .absolute()
                                                            .right(px(-2.0))
                                                            .bottom(px(-2.0))
                                                            .size(px(10.0))
                                                            .rounded_full()
                                                            .bg(status_fill)
                                                            .border_2()
                                                            .border_color(status_border),
                                                    ),
                                            ),
                                    )
                            })
                            .collect::<Vec<_>>()
                    }),
            )
            .child(
                Button::new("rail-preferences")
                    .ghost()
                    .small()
                    .size(px(40.0))
                    .selected(self.preferences_open)
                    .text_color(cx.theme().sidebar_foreground)
                    .icon(IconName::Settings)
                    .accessibility_label("Preferences")
                    .tooltip("Preferences")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_preferences(window, cx);
                    })),
            );
        div()
            .h_full()
            .flex_shrink_0()
            .overflow_hidden()
            .w(px(60.0))
            .when(self.preferences_draft.app.auto_hide_hotbar, |el| {
                el.w(px(8.0)).hover(|style| style.w(px(60.0)))
            })
            .child(rail)
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
                    "Cluster".into()
                } else {
                    destination_label(&destination).into()
                };
                let cid = cluster_id.to_owned();
                let unpin_cid = cluster_id.to_owned();
                let unpin_dest = destination.clone();
                let unpin_handler: Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)> =
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
            let pin_state = if leaf {
                let pinned = is_section_pinned(group.default_child);
                let pin_cid = cluster_id.to_owned();
                let pin_sec = group.default_child.to_owned();
                let handler: Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)> =
                    Rc::new(cx.listener(move |this, _, _, cx| {
                        this.toggle_section_pin(pin_cid.clone(), pin_sec.clone(), !pinned, cx);
                    }));
                Some((pinned, handler))
            } else {
                None
            };
            resources.push(menu_item_with_pin(
                format!("grp-{group_id}"),
                if group_id == "overview" {
                    "Cluster"
                } else {
                    group.label
                },
                Some(section_icon(group_id)),
                leaf && active == group.default_child,
                if leaf {
                    None
                } else if expanded {
                    Some(IconName::ChevronDown)
                } else {
                    Some(IconName::ChevronRight)
                },
                pin_state,
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
                    let pinned = is_section_pinned(destination);
                    let pin_cid = cluster_id.to_owned();
                    let pin_sec = destination.to_owned();
                    let pin_handler: Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)> =
                        Rc::new(cx.listener(move |this, _, _, cx| {
                            this.toggle_section_pin(pin_cid.clone(), pin_sec.clone(), !pinned, cx);
                        }));
                    menu_item_with_pin(
                        format!("child-{destination}"),
                        child.label,
                        None,
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
                    .pl_6()
                }));

                // Dynamically append discovered Custom Resource Definitions under Custom Resources
                if group_id == "custom-resources"
                    && let Some(defs_page) = &self.custom_definitions_page
                {
                    resources.extend(defs_page.items.iter().take(25).map(|crd| {
                        let cid = cluster_id.to_owned();
                        let crd_dest = format!("crd:{}", crd.name);
                        let is_active = *active == crd_dest;
                        let label = crd
                            .columns
                            .get("kind")
                            .cloned()
                            .unwrap_or_else(|| crd.name.clone());
                        menu_item_with_pin(
                            format!("crd-nav-{}", crd.name),
                            label,
                            None,
                            is_active,
                            None,
                            None,
                            cx.listener(move |this, _, window, cx| {
                                this.navigate(
                                    Route::Cluster {
                                        cluster_id: cid.clone(),
                                        section: crd_dest.clone(),
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
        }
        groups.push(SidebarGroup::new("Resources").child(SidebarMenu::new().children(resources)));
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
    use super::{
        DARK_INK_RGB, LIGHT_INK_RGB, cluster_initials, cluster_rgb, cluster_tile_palette,
        wcag_contrast_ratio,
    };

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
        assert_eq!(cluster_initials("gke-prod-indo"), "GPI");
        assert_eq!(cluster_initials("local"), "LO");
        assert_eq!(cluster_initials("東京"), "東京");
        assert_eq!(cluster_initials(" -- "), "K");
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
                    assert!(matches!(palette.fg_rgb, LIGHT_INK_RGB | DARK_INK_RGB));
                }
            }
        }
    }
}
