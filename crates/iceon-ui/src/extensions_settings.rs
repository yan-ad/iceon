//! Built-in native extension management view in Preferences.
//!
//! Ported natively from Freelens extensions, allowing users to toggle each
//! extension on or off with immediate persistence and live cluster sidebar updates.

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    switch::Switch,
    tag::Tag,
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use gpui_kit_assets::IconName as AssetIconName;
use iceon_extension::{
    ALL_BUILTIN_EXTENSIONS, ExtensionCategory, ExtensionManifest, ExtensionRegistry,
};

use crate::empty_state::compact_empty_state;
use crate::preferences_view::{current_prefs, mutate_preferences};

pub struct ExtensionsSettings {
    pub(crate) search_input: Entity<InputState>,
    pub(crate) selected_category: Option<ExtensionCategory>,
    pub(crate) registry: ExtensionRegistry,
}

impl ExtensionsSettings {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search extensions, categories, or CRDs…")
        });

        cx.subscribe(&search_input, |_, _, event: &InputEvent, cx| match event {
            InputEvent::Change | InputEvent::PressEnter { .. } => cx.notify(),
            _ => {}
        })
        .detach();

        Self {
            search_input,
            selected_category: None,
            registry: ExtensionRegistry::new(),
        }
    }

    fn extension_icon(manifest: &ExtensionManifest) -> AssetIconName {
        match manifest.id {
            "keda" => AssetIconName::Gauge,
            "fluxcd" => AssetIconName::PackageCheck,
            "cnpg" => AssetIconName::Database,
            "gateway-api" => AssetIconName::Network,
            "eso" => AssetIconName::KeyRound,
            "kafka" => AssetIconName::Layers,
            "rabbitmq" => AssetIconName::Blocks,
            "velero" => AssetIconName::Archive,
            "karpenter" => AssetIconName::Server,
            "kamaji" => AssetIconName::Crown,
            "sveltos" => AssetIconName::SlidersHorizontal,
            "gpu" => AssetIconName::Container,
            "vals" => AssetIconName::Shield,
            "resource-map" => AssetIconName::Route,
            "ai" => AssetIconName::BadgeCheck,
            "for-claude" => AssetIconName::Pencil,
            "agentbridge" => AssetIconName::Link,
            "kubeswift" => AssetIconName::Box,
            _ => AssetIconName::Puzzle,
        }
    }

    fn matches_filter(&self, manifest: &ExtensionManifest, query: &str) -> bool {
        if let Some(cat) = self.selected_category {
            if manifest.category != cat {
                return false;
            }
        }
        if query.is_empty() {
            return true;
        }
        let q = query.to_lowercase();
        manifest.name.to_lowercase().contains(&q)
            || manifest.package_name.to_lowercase().contains(&q)
            || manifest.description.to_lowercase().contains(&q)
            || manifest.category.display_name().to_lowercase().contains(&q)
            || manifest
                .crds
                .iter()
                .any(|c| c.kind.to_lowercase().contains(&q) || c.group.to_lowercase().contains(&q))
    }
}

impl Render for ExtensionsSettings {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let prefs = current_prefs(cx);
        let enabled_set = &prefs.app.enabled_extensions;
        let total_count = ALL_BUILTIN_EXTENSIONS.len();
        let enabled_count = self
            .registry
            .all()
            .iter()
            .filter(|ext| self.registry.is_enabled(ext.id, enabled_set))
            .count();

        let query = self.search_input.read(cx).text().to_string();
        let query_trimmed = query.trim().to_string();

        let filtered: Vec<&'static ExtensionManifest> = self
            .registry
            .all()
            .into_iter()
            .filter(|manifest| self.matches_filter(manifest, &query_trimmed))
            .collect();

        v_flex()
            .size_full()
            .p_4()
            .gap_3()
            .overflow_hidden()
            // Header
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_base()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(cx.theme().foreground)
                                            .child("Extensions"),
                                    )
                                    .child(
                                        Tag::new()
                                            .small()
                                            .child(format!("{enabled_count} of {total_count} Active")),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        Button::new("enable-all-extensions")
                                            .xsmall()
                                            .ghost()
                                            .label("Enable All")
                                            .on_click(cx.listener(|_, _, _, cx| {
                                                mutate_preferences(cx, |prefs| {
                                                    for &ext in ALL_BUILTIN_EXTENSIONS {
                                                        prefs.app.enabled_extensions.insert(ext.id.to_string());
                                                    }
                                                });
                                            })),
                                    )
                                    .child(
                                        Button::new("disable-all-extensions")
                                            .xsmall()
                                            .ghost()
                                            .label("Disable All")
                                            .on_click(cx.listener(|_, _, _, cx| {
                                                mutate_preferences(cx, |prefs| {
                                                    prefs.app.enabled_extensions.clear();
                                                });
                                            })),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Native extensions ported from Freelens. Enable extensions to add cluster views, custom resource definitions, and specialized inspectors."),
                    ),
            )
            // Search Input
            .child(
                Input::new(&self.search_input)
                    .w_full()
                    .small(),
            )
            // Category Filter Chips
            .child(
                h_flex()
                    .w_full()
                    .gap_1p5()
                    .flex_wrap()
                    .pb_1()
                    .child(
                        Button::new("cat-filter-all")
                            .xsmall()
                            .when(self.selected_category.is_none(), |b| b.primary())
                            .when(self.selected_category.is_some(), |b| b.ghost())
                            .label(format!("All ({total_count})"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.selected_category = None;
                                cx.notify();
                            })),
                    )
                    .children(ExtensionCategory::all().iter().map(|&cat| {
                        let is_sel = self.selected_category == Some(cat);
                        let name = cat.display_name();
                        Button::new(SharedString::from(format!("cat-filter-{}", name)))
                            .xsmall()
                            .when(is_sel, |b| b.primary())
                            .when(!is_sel, |b| b.ghost())
                            .label(name)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.selected_category = if this.selected_category == Some(cat) {
                                    None
                                } else {
                                    Some(cat)
                                };
                                cx.notify();
                            }))
                    })),
            )
            // Extension Cards List
            .child(
                v_flex()
                    .id("extensions-settings-list")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scroll()
                    .pr_1()
                    .child(
                        if filtered.is_empty() {
                            compact_empty_state(
                                IconName::Search,
                                "No extensions found",
                                "Try a different extension name, category, or CRD.",
                            )
                            .into_any_element()
                        } else {
                            v_flex()
                                .w_full()
                                .gap_2()
                                .children(filtered.into_iter().map(|manifest| {
                                    let id = manifest.id;
                                    let is_active = self.registry.is_enabled(id, enabled_set);
                                    let icon_name = Self::extension_icon(manifest);
                                    let homepage = manifest.homepage;

                                    v_flex()
                                        .w_full()
                                        .p_3()
                                        .rounded(px(6.0))
                                        .border_1()
                                        .border_color(if is_active {
                                            cx.theme().primary
                                        } else {
                                            cx.theme().border
                                        })
                                        .bg(cx.theme().secondary)
                                        .gap_2()
                                        // Header Row: Icon + Name + Category + Ported badge + Version ... GitHub link + Switch toggle
                                        .child(
                                            h_flex()
                                                .w_full()
                                                .items_center()
                                                .justify_between()
                                                .child(
                                                    h_flex()
                                                        .gap_2()
                                                        .items_center()
                                                        .child(
                                                            Icon::new(icon_name)
                                                                .size_4()
                                                                .text_color(if is_active {
                                                                    cx.theme().primary
                                                                } else {
                                                                    cx.theme().muted_foreground
                                                                }),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_sm()
                                                                .font_weight(FontWeight::SEMIBOLD)
                                                                .text_color(cx.theme().foreground)
                                                                .child(manifest.name),
                                                        )
                                                        .child(
                                                            Tag::new()
                                                                .small()
                                                                .child(manifest.category.display_name()),
                                                        )
                                                        .child(
                                                            div()
                                                                .id(SharedString::from(format!("ported-tag-{}", id)))
                                                                .cursor_pointer()
                                                                .tooltip(move |window, cx| {
                                                                    Tooltip::new(format!("Ported from {}", homepage)).build(window, cx)
                                                                })
                                                                .on_mouse_down(
                                                                    MouseButton::Left,
                                                                    cx.listener(move |_, _, _, cx| {
                                                                        cx.open_url(homepage);
                                                                    }),
                                                                )
                                                                .child(
                                                                    Tag::info()
                                                                        .small()
                                                                        .child("Ported ↗"),
                                                                ),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_xs()
                                                                .text_color(cx.theme().muted_foreground)
                                                                .child(format!("v{}", manifest.version)),
                                                        ),
                                                )
                                                .child(
                                                    h_flex()
                                                        .gap_3()
                                                        .items_center()
                                                        .child(
                                                            Button::new(SharedString::from(format!("gh-link-{}", id)))
                                                                .xsmall()
                                                                .ghost()
                                                                .icon(AssetIconName::Github)
                                                                .label("GitHub")
                                                                .tooltip(format!("Open upstream {} repository", manifest.name))
                                                                .on_click(cx.listener(move |_, _, _, cx| {
                                                                    cx.open_url(homepage);
                                                                })),
                                                        )
                                                        .child(
                                                            Switch::new(SharedString::from(format!("toggle-ext-{}", id)))
                                                                .checked(is_active)
                                                                .accessibility_label(format!(
                                                                    "Toggle extension {}",
                                                                    manifest.name
                                                                ))
                                                                .on_click(cx.listener(move |_, checked: &bool, _, cx| {
                                                                    let ext_id = id.to_string();
                                                                    let should_enable = *checked;
                                                                    mutate_preferences(cx, move |prefs| {
                                                                        if should_enable {
                                                                            prefs.app.enabled_extensions.insert(ext_id);
                                                                        } else {
                                                                            prefs.app.enabled_extensions.remove(&ext_id);
                                                                        }
                                                                    });
                                                                    cx.notify();
                                                                })),
                                                        ),
                                                ),
                                        )
                                        // Description Row
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(manifest.description),
                                        )
                                        // Supported CRDs tags (if any)
                                        .when(!manifest.crds.is_empty(), |card| {
                                            card.child(
                                                h_flex()
                                                    .gap_1p5()
                                                    .items_center()
                                                    .flex_wrap()
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .font_weight(FontWeight::SEMIBOLD)
                                                            .text_color(cx.theme().muted_foreground)
                                                            .child("CRDs:"),
                                                    )
                                                    .children(manifest.crds.iter().map(|crd| {
                                                        Tag::new()
                                                            .small()
                                                            .child(crd.kind)
                                                    })),
                                            )
                                        })
                                }))
                                .into_any_element()
                        },
                    ),
            )
    }
}
