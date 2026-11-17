//! Marketplace bottom dock panel for browsing, installing, and managing themes and icon themes.
//! Designed as an extensible foundation for future extension and plugin marketplaces.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::themes_settings::ThemeCatalog;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum MarketplaceCategory {
    #[default]
    All,
    Themes,
    IconThemes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum MarketplaceFilter {
    #[default]
    All,
    Installed,
    NotInstalled,
}

#[derive(Clone, Debug)]
struct DisplayExtensionItem {
    id: String,
    name: String,
    version: String,
    description: String,
    download_count: u64,
    category: MarketplaceCategory,
    installed: bool,
}

pub(crate) struct MarketplaceDockView {
    pub(crate) category: MarketplaceCategory,
    pub(crate) filter: MarketplaceFilter,
    pub(crate) search_input: Entity<InputState>,
    pub(crate) scroll: ScrollHandle,
    search_task: Option<Task<()>>,
}

impl MarketplaceDockView {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        crate::themes_settings::init(cx);
        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search marketplace…"));

        cx.subscribe_in(&search_input, window, |view, input, event, window, cx| {
            if matches!(event, InputEvent::Change) {
                let query = input.read(cx).value().to_string();
                view.schedule_search(query, window, cx);
            }
        })
        .detach();

        // Initial fetch if marketplace has not been populated
        let catalog = cx.global::<ThemeCatalog>();
        if !catalog.is_marketplace_loaded() {
            ThemeCatalog::schedule_search("".into(), cx);
            ThemeCatalog::schedule_icon_search("".into(), cx);
        }

        Self {
            category: MarketplaceCategory::All,
            filter: MarketplaceFilter::All,
            search_input,
            scroll: ScrollHandle::new(),
            search_task: None,
        }
    }

    pub(crate) fn set_category(
        &mut self,
        category: MarketplaceCategory,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.category = category;
        self.scroll = ScrollHandle::new();
        cx.notify();
    }

    pub(crate) fn set_filter(&mut self, filter: MarketplaceFilter, cx: &mut Context<Self>) {
        self.filter = filter;
        self.scroll = ScrollHandle::new();
        cx.notify();
    }

    pub(crate) fn schedule_search(
        &mut self,
        query: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.scroll = ScrollHandle::new();
        self.search_task = Some(cx.spawn_in(window, async move |_, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(300))
                .await;
            let _ = cx.update(|_window, cx| {
                ThemeCatalog::schedule_search(query.clone(), cx);
                ThemeCatalog::schedule_icon_search(query, cx);
            });
        }));
        cx.notify();
    }

    pub(crate) fn refresh(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let query = self.search_input.read(cx).value().to_string();
        ThemeCatalog::schedule_search(query.clone(), cx);
        ThemeCatalog::schedule_icon_search(query, cx);
        cx.notify();
    }
}

impl Render for MarketplaceDockView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let catalog = cx.global::<ThemeCatalog>();
        let is_loading = catalog.loading
            || catalog.icon_loading
            || catalog.is_marketplace_loading()
            || catalog.is_marketplace_icons_loading();

        let error = catalog
            .marketplace_error()
            .or(catalog.marketplace_icons_error())
            .or(catalog.error.as_deref())
            .or(catalog.icon_error.as_deref());

        let query = self.search_input.read(cx).value().to_lowercase();

        // Aggregate items
        let mut items: Vec<DisplayExtensionItem> = Vec::new();

        // 1. Color themes
        if matches!(
            self.category,
            MarketplaceCategory::All | MarketplaceCategory::Themes
        ) {
            let installed_ids: std::collections::HashSet<String> =
                catalog.extensions.iter().map(|e| e.id.clone()).collect();

            // Include installed
            for ext in &catalog.extensions {
                items.push(DisplayExtensionItem {
                    id: ext.id.clone(),
                    name: if ext.name.is_empty() {
                        ext.id.clone()
                    } else {
                        ext.name.clone()
                    },
                    version: ext.version.clone(),
                    description: ext.description.clone(),
                    download_count: ext.download_count.unwrap_or(0),
                    category: MarketplaceCategory::Themes,
                    installed: true,
                });
            }

            // Include marketplace extensions not installed
            for item in catalog.marketplace_themes() {
                if !installed_ids.contains(&item.id) {
                    items.push(DisplayExtensionItem {
                        id: item.id.clone(),
                        name: item.name.clone(),
                        version: String::new(),
                        description: item.description.clone(),
                        download_count: item.download_count,
                        category: MarketplaceCategory::Themes,
                        installed: false,
                    });
                }
            }
        }

        // 2. Icon themes
        if matches!(
            self.category,
            MarketplaceCategory::All | MarketplaceCategory::IconThemes
        ) {
            let installed_icon_ids: std::collections::HashSet<String> = catalog
                .icon_extensions
                .iter()
                .map(|e| e.id.clone())
                .collect();

            // Include installed
            for ext in &catalog.icon_extensions {
                items.push(DisplayExtensionItem {
                    id: ext.id.clone(),
                    name: if ext.name.is_empty() {
                        ext.id.clone()
                    } else {
                        ext.name.clone()
                    },
                    version: ext.version.clone(),
                    description: ext.description.clone(),
                    download_count: ext.download_count.unwrap_or(0),
                    category: MarketplaceCategory::IconThemes,
                    installed: true,
                });
            }

            // Include marketplace icon extensions not installed
            for item in catalog.marketplace_icons() {
                if !installed_icon_ids.contains(&item.id) {
                    items.push(DisplayExtensionItem {
                        id: item.id.clone(),
                        name: item.name.clone(),
                        version: String::new(),
                        description: item.description.clone(),
                        download_count: item.download_count,
                        category: MarketplaceCategory::IconThemes,
                        installed: false,
                    });
                }
            }
        }

        // Filter by tab
        let filtered_items: Vec<DisplayExtensionItem> = items
            .into_iter()
            .filter(|item| match self.filter {
                MarketplaceFilter::All => true,
                MarketplaceFilter::Installed => item.installed,
                MarketplaceFilter::NotInstalled => !item.installed,
            })
            .filter(|item| {
                if query.is_empty() {
                    return true;
                }
                item.name.to_lowercase().contains(&query)
                    || item.id.to_lowercase().contains(&query)
                    || item.description.to_lowercase().contains(&query)
            })
            .collect();

        v_flex()
            .size_full()
            .bg(cx.theme().background)
            // Header toolbar
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .items_center()
                    .justify_between()
                    .gap_3()
                    // Left: Filter tabs & Category chips
                    .child(
                        h_flex()
                            .items_center()
                            .gap_3()
                            // Status filter tabs: All | Installed | Not Installed
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_1()
                                    .p_0p5()
                                    .rounded(px(6.0))
                                    .bg(cx.theme().secondary.opacity(0.3))
                                    .child(
                                        Button::new("filter-all")
                                            .xsmall()
                                            .when(self.filter == MarketplaceFilter::All, |b| {
                                                b.secondary()
                                            })
                                            .when(self.filter != MarketplaceFilter::All, |b| {
                                                b.ghost()
                                            })
                                            .label("All")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.set_filter(MarketplaceFilter::All, cx);
                                            })),
                                    )
                                    .child(
                                        Button::new("filter-installed")
                                            .xsmall()
                                            .when(
                                                self.filter == MarketplaceFilter::Installed,
                                                |b| b.secondary(),
                                            )
                                            .when(
                                                self.filter != MarketplaceFilter::Installed,
                                                |b| b.ghost(),
                                            )
                                            .label("Installed")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.set_filter(MarketplaceFilter::Installed, cx);
                                            })),
                                    )
                                    .child(
                                        Button::new("filter-not-installed")
                                            .xsmall()
                                            .when(
                                                self.filter == MarketplaceFilter::NotInstalled,
                                                |b| b.secondary(),
                                            )
                                            .when(
                                                self.filter != MarketplaceFilter::NotInstalled,
                                                |b| b.ghost(),
                                            )
                                            .label("Not Installed")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.set_filter(
                                                    MarketplaceFilter::NotInstalled,
                                                    cx,
                                                );
                                            })),
                                    ),
                            )
                            // Divider
                            .child(div().h(px(18.0)).w(px(1.0)).bg(cx.theme().border))
                            // Category chips: All | Themes | Icon Themes
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_1p5()
                                    .child(
                                        Button::new("chip-cat-all")
                                            .xsmall()
                                            .rounded_full()
                                            .when(self.category == MarketplaceCategory::All, |b| {
                                                b.secondary()
                                            })
                                            .when(self.category != MarketplaceCategory::All, |b| {
                                                b.ghost()
                                            })
                                            .label("All Categories")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.set_category(
                                                    MarketplaceCategory::All,
                                                    window,
                                                    cx,
                                                );
                                            })),
                                    )
                                    .child(
                                        Button::new("chip-cat-themes")
                                            .xsmall()
                                            .rounded_full()
                                            .when(
                                                self.category == MarketplaceCategory::Themes,
                                                |b| b.secondary(),
                                            )
                                            .when(
                                                self.category != MarketplaceCategory::Themes,
                                                |b| b.ghost(),
                                            )
                                            .child(
                                                h_flex()
                                                    .gap_1()
                                                    .items_center()
                                                    .child(Icon::new(IconName::Palette).xsmall())
                                                    .child("Themes"),
                                            )
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.set_category(
                                                    MarketplaceCategory::Themes,
                                                    window,
                                                    cx,
                                                );
                                            })),
                                    )
                                    .child(
                                        Button::new("chip-cat-icon-themes")
                                            .xsmall()
                                            .rounded_full()
                                            .when(
                                                self.category == MarketplaceCategory::IconThemes,
                                                |b| b.secondary(),
                                            )
                                            .when(
                                                self.category != MarketplaceCategory::IconThemes,
                                                |b| b.ghost(),
                                            )
                                            .child(
                                                h_flex()
                                                    .gap_1()
                                                    .items_center()
                                                    .child(
                                                        Icon::new(
                                                            gpui_kit_assets::IconName::Package,
                                                        )
                                                        .xsmall(),
                                                    )
                                                    .child("Icon Themes"),
                                            )
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.set_category(
                                                    MarketplaceCategory::IconThemes,
                                                    window,
                                                    cx,
                                                );
                                            })),
                                    ),
                            ),
                    )
                    // Right: Search & Refresh
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Input::new(&self.search_input)
                                    .small()
                                    .cleanable(true)
                                    .w(px(200.0)),
                            )
                            .child(
                                Button::new("refresh-marketplace")
                                    .small()
                                    .ghost()
                                    .icon(gpui_kit_assets::IconName::RefreshCw)
                                    .tooltip("Refresh marketplace")
                                    .accessibility_label("Refresh marketplace")
                                    .disabled(is_loading)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.refresh(window, cx);
                                    })),
                            ),
                    ),
            )
            // Error banner if any
            .when_some(error, |el, err| {
                el.child(
                    h_flex()
                        .mx_3()
                        .mt_2()
                        .px_3()
                        .py_2()
                        .rounded(px(6.0))
                        .bg(cx.theme().danger.opacity(0.1))
                        .border_1()
                        .border_color(cx.theme().danger)
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().danger)
                                .child(err.to_string()),
                        )
                        .child(
                            Button::new("retry-marketplace")
                                .xsmall()
                                .ghost()
                                .label("Retry")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.refresh(window, cx);
                                })),
                        ),
                )
            })
            // Items list
            .child(
                v_flex()
                    .id("marketplace-scroll")
                    .flex_1()
                    .min_h_0()
                    .p_3()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(if is_loading && filtered_items.is_empty() {
                        div()
                            .size_full()
                            .p_8()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("Loading marketplace extensions…")
                            .into_any_element()
                    } else if filtered_items.is_empty() {
                        div()
                            .size_full()
                            .p_8()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("No extensions found matching your filter.")
                            .into_any_element()
                    } else {
                        div()
                            .w_full()
                            .gap_2p5()
                            .flex()
                            .flex_col()
                            .children(filtered_items.into_iter().map(|item| {
                                let id = item.id.clone();
                                let is_installed = item.installed;
                                let is_icon = item.category == MarketplaceCategory::IconThemes;
                                let download_text = if item.download_count > 0 {
                                    format!(" · {} downloads", item.download_count)
                                } else {
                                    String::new()
                                };
                                let version_text = if !item.version.is_empty() {
                                    format!(" · v{}", item.version)
                                } else {
                                    String::new()
                                };

                                h_flex()
                                    .w_full()
                                    .items_center()
                                    .justify_between()
                                    .p_3()
                                    .rounded(px(6.0))
                                    .border_1()
                                    .border_color(cx.theme().border)
                                    .bg(cx.theme().secondary.opacity(0.2))
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_1()
                                            .child(
                                                h_flex()
                                                    .items_center()
                                                    .gap_2()
                                                    // Category chip
                                                    .child(
                                                        div()
                                                            .px_2()
                                                            .py_0p5()
                                                            .rounded_full()
                                                            .text_xs()
                                                            .font_weight(FontWeight::SEMIBOLD)
                                                            .when(!is_icon, |el| {
                                                                el.bg(cx
                                                                    .theme()
                                                                    .primary
                                                                    .opacity(0.15))
                                                                    .text_color(cx.theme().primary)
                                                                    .child("Theme")
                                                            })
                                                            .when(is_icon, |el| {
                                                                el.bg(cx
                                                                    .theme()
                                                                    .accent
                                                                    .opacity(0.15))
                                                                    .text_color(cx.theme().accent)
                                                                    .child("Icon Theme")
                                                            }),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .font_weight(FontWeight::SEMIBOLD)
                                                            .child(item.name),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(format!(
                                                        "{}{}{}",
                                                        item.id, version_text, download_text
                                                    )),
                                            )
                                            .when(!item.description.is_empty(), |el| {
                                                el.child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(cx.theme().muted_foreground)
                                                        .child(item.description),
                                                )
                                            }),
                                    )
                                    .child(if is_installed {
                                        let uninstall_id = id.clone();
                                        h_flex()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .px_2()
                                                    .py_1()
                                                    .rounded(px(4.0))
                                                    .bg(cx.theme().success.opacity(0.15))
                                                    .text_color(cx.theme().success)
                                                    .text_xs()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .child("Installed"),
                                            )
                                            .child(
                                                Button::new(SharedString::from(format!(
                                                    "uninstall-marketplace-{uninstall_id}"
                                                )))
                                                .small()
                                                .ghost()
                                                .icon(crate::action_icons::delete())
                                                .tooltip("Uninstall")
                                                .accessibility_label("Uninstall extension")
                                                .disabled(is_loading)
                                                .on_click(cx.listener(move |_, _, _window, cx| {
                                                    if is_icon {
                                                        ThemeCatalog::mutate_icon_theme(
                                                            &uninstall_id,
                                                            true,
                                                            cx,
                                                        );
                                                    } else {
                                                        ThemeCatalog::mutate_theme(
                                                            &uninstall_id,
                                                            true,
                                                            cx,
                                                        );
                                                    }
                                                })),
                                            )
                                            .into_any_element()
                                    } else {
                                        let install_id = id.clone();
                                        h_flex()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                Button::new(SharedString::from(format!(
                                                    "install-marketplace-{install_id}"
                                                )))
                                                .small()
                                                .secondary()
                                                .icon(gpui_kit_assets::IconName::Download)
                                                .label("Install")
                                                .disabled(is_loading)
                                                .on_click(cx.listener(move |_, _, _window, cx| {
                                                    if is_icon {
                                                        ThemeCatalog::mutate_icon_theme(
                                                            &install_id,
                                                            false,
                                                            cx,
                                                        );
                                                    } else {
                                                        ThemeCatalog::mutate_theme(
                                                            &install_id,
                                                            false,
                                                            cx,
                                                        );
                                                    }
                                                })),
                                            )
                                            .into_any_element()
                                    })
                            }))
                            .into_any_element()
                    }),
            )
    }
}
