//! Data-only Zed theme and icon theme installation and cached native settings controls.
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use iceon_core::preferences::AppTheme;
use iceon_core::zed_icon_themes::{
    self as zed_icons, InstalledIconExtension, MarketplaceExtension as IconMarketplaceExtension,
    ZedIconTheme,
};
use iceon_core::zed_themes::{self, InstalledExtension, MarketplaceExtension, ZedTheme};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ThemesTab {
    #[default]
    ColorThemes,
    IconThemes,
}

#[derive(Default)]
pub(crate) struct ThemeCatalog {
    pub(crate) extensions: Vec<InstalledExtension>,
    pub(crate) error: Option<String>,
    pub(crate) loading: bool,
    marketplace: Vec<MarketplaceExtension>,
    marketplace_query: String,
    marketplace_loading: bool,
    marketplace_loaded: bool,
    marketplace_error: Option<String>,
    marketplace_generation: u64,

    // Icon themes
    pub(crate) icon_extensions: Vec<InstalledIconExtension>,
    pub(crate) icon_error: Option<String>,
    pub(crate) icon_loading: bool,
    marketplace_icons: Vec<IconMarketplaceExtension>,
    marketplace_icons_query: String,
    marketplace_icons_loading: bool,
    marketplace_icons_loaded: bool,
    marketplace_icons_error: Option<String>,
    marketplace_icons_generation: u64,
}
impl Global for ThemeCatalog {}

impl ThemeCatalog {
    pub(crate) fn is_marketplace_loaded(&self) -> bool {
        self.marketplace_loaded && self.marketplace_icons_loaded
    }

    pub(crate) fn is_marketplace_loading(&self) -> bool {
        self.marketplace_loading
    }

    pub(crate) fn is_marketplace_icons_loading(&self) -> bool {
        self.marketplace_icons_loading
    }

    pub(crate) fn marketplace_error(&self) -> Option<&str> {
        self.marketplace_error.as_deref()
    }

    pub(crate) fn marketplace_icons_error(&self) -> Option<&str> {
        self.marketplace_icons_error.as_deref()
    }

    pub(crate) fn marketplace_themes(&self) -> &[MarketplaceExtension] {
        &self.marketplace
    }

    pub(crate) fn marketplace_icons(&self) -> &[IconMarketplaceExtension] {
        &self.marketplace_icons
    }

    pub(crate) fn mutate_theme(input: &str, remove: bool, cx: &mut App) {
        init(cx);
        if cx.global::<ThemeCatalog>().loading {
            return;
        }
        let id = match extension_id(input) {
            Ok(id) => id,
            Err(error) => {
                cx.global_mut::<ThemeCatalog>().error = Some(error);
                cx.refresh_windows();
                return;
            }
        };
        let catalog = cx.global_mut::<ThemeCatalog>();
        catalog.loading = true;
        catalog.error = None;
        let changed_id = id.clone();
        let metadata = cx
            .global::<ThemeCatalog>()
            .marketplace
            .iter()
            .find(|item| item.id == id)
            .cloned();

        let handle = iceon_core::runtime::spawn(async move {
            let operation = if remove {
                iceon_core::runtime::spawn_blocking(move || zed_themes::uninstall(&id))
                    .await
                    .map_err(|_| "Theme removal worker stopped".to_owned())
                    .and_then(|result| result)
            } else {
                match metadata {
                    Some(item) => zed_themes::install_with_metadata(&item).await.map(|_| ()),
                    None => zed_themes::install(&id).await.map(|_| ()),
                }
            };
            let inventory = iceon_core::runtime::spawn_blocking(zed_themes::installed)
                .await
                .map_err(|_| "Theme catalog worker stopped".to_owned())
                .and_then(|result| result);
            (operation, inventory)
        });
        cx.spawn(async move |cx| {
            let result = handle.await;
            cx.update(|cx| {
                let catalog = cx.global_mut::<ThemeCatalog>();
                catalog.loading = false;
                let removed = match result {
                    Ok((operation, inventory)) => {
                        let removed = remove && operation.is_ok();
                        catalog.error = operation.err();
                        match inventory {
                            Ok(extensions) => catalog.extensions = extensions,
                            Err(error) => {
                                catalog.extensions.clear();
                                catalog.error.get_or_insert(error);
                            }
                        }
                        removed
                    }
                    Err(_) => {
                        catalog.error = Some(
                            "Theme worker stopped. Reopen the app to reload local themes.".into(),
                        );
                        false
                    }
                };
                if removed
                    && crate::theme_picker::selected_zed(cx).is_some_and(|id| {
                        id.split_once('/')
                            .is_some_and(|(extension, _)| extension == changed_id)
                    })
                {
                    crate::preferences_view::set_zed_theme(cx, None);
                }
                crate::shell_theme::apply(gpui_kit::component::theme::ThemeMode::Dark, None, cx);
                cx.refresh_windows();
            });
        })
        .detach();
        cx.refresh_windows();
    }

    pub(crate) fn mutate_icon_theme(input: &str, remove: bool, cx: &mut App) {
        init(cx);
        if cx.global::<ThemeCatalog>().icon_loading {
            return;
        }
        let id = match extension_id(input) {
            Ok(id) => id,
            Err(error) => {
                cx.global_mut::<ThemeCatalog>().icon_error = Some(error);
                cx.refresh_windows();
                return;
            }
        };
        let catalog = cx.global_mut::<ThemeCatalog>();
        catalog.icon_loading = true;
        catalog.icon_error = None;
        let changed_id = id.clone();
        let metadata = cx
            .global::<ThemeCatalog>()
            .marketplace_icons
            .iter()
            .find(|item| item.id == id)
            .cloned();

        let handle = iceon_core::runtime::spawn(async move {
            let operation = if remove {
                iceon_core::runtime::spawn_blocking(move || zed_icons::uninstall(&id))
                    .await
                    .map_err(|_| "Icon theme removal worker stopped".to_owned())
                    .and_then(|result| result)
            } else {
                match metadata {
                    Some(item) => zed_icons::install_with_metadata(&item).await.map(|_| ()),
                    None => zed_icons::install(&id).await.map(|_| ()),
                }
            };
            let inventory = iceon_core::runtime::spawn_blocking(zed_icons::installed)
                .await
                .map_err(|_| "Icon theme catalog worker stopped".to_owned())
                .and_then(|result| result);
            (operation, inventory)
        });
        cx.spawn(async move |cx| {
            let result = handle.await;
            cx.update(|cx| {
                let catalog = cx.global_mut::<ThemeCatalog>();
                catalog.icon_loading = false;
                let removed = match result {
                    Ok((operation, inventory)) => {
                        let removed = remove && operation.is_ok();
                        catalog.icon_error = operation.err();
                        match inventory {
                            Ok(extensions) => catalog.icon_extensions = extensions,
                            Err(error) => {
                                catalog.icon_extensions.clear();
                                catalog.icon_error.get_or_insert(error);
                            }
                        }
                        removed
                    }
                    Err(_) => {
                        catalog.icon_error = Some(
                            "Icon theme worker stopped. Reopen the app to reload local icon themes."
                                .into(),
                        );
                        false
                    }
                };
                if removed
                    && crate::theme_picker::selected_zed_icon(cx).is_some_and(|id| {
                        id.split_once('/')
                            .is_some_and(|(extension, _)| extension == changed_id)
                    })
                {
                    crate::preferences_view::set_zed_icon_theme(cx, None);
                }
                cx.refresh_windows();
            });
        })
        .detach();
        cx.refresh_windows();
    }

    pub(crate) fn schedule_search(query: String, cx: &mut App) {
        init(cx);
        let catalog = cx.global_mut::<ThemeCatalog>();
        catalog.marketplace_generation += 1;
        let generation = catalog.marketplace_generation;
        catalog.marketplace_loading = true;
        catalog.marketplace_error = None;
        catalog.marketplace_query = query.clone();

        let handle =
            iceon_core::runtime::spawn(async move { zed_themes::marketplace(&query).await });
        cx.spawn(async move |cx| {
            let result = handle.await;
            cx.update(|cx| {
                let catalog = cx.global_mut::<ThemeCatalog>();
                if catalog.marketplace_generation != generation {
                    return;
                }
                catalog.marketplace_loading = false;
                match result {
                    Ok(Ok(items)) => {
                        catalog.marketplace = items;
                        catalog.marketplace_loaded = true;
                    }
                    Ok(Err(error)) => catalog.marketplace_error = Some(error),
                    Err(_) => catalog.marketplace_error = Some("Marketplace worker stopped".into()),
                }
                cx.refresh_windows();
            });
        })
        .detach();
    }

    pub(crate) fn schedule_icon_search(query: String, cx: &mut App) {
        init(cx);
        let catalog = cx.global_mut::<ThemeCatalog>();
        catalog.marketplace_icons_generation += 1;
        let generation = catalog.marketplace_icons_generation;
        catalog.marketplace_icons_loading = true;
        catalog.marketplace_icons_error = None;
        catalog.marketplace_icons_query = query.clone();

        let handle =
            iceon_core::runtime::spawn(async move { zed_icons::marketplace(&query).await });
        cx.spawn(async move |cx| {
            let result = handle.await;
            cx.update(|cx| {
                let catalog = cx.global_mut::<ThemeCatalog>();
                if catalog.marketplace_icons_generation != generation {
                    return;
                }
                catalog.marketplace_icons_loading = false;
                match result {
                    Ok(Ok(items)) => {
                        catalog.marketplace_icons = items;
                        catalog.marketplace_icons_loaded = true;
                    }
                    Ok(Err(error)) => catalog.marketplace_icons_error = Some(error),
                    Err(_) => {
                        catalog.marketplace_icons_error = Some("Marketplace worker stopped".into())
                    }
                }
                cx.refresh_windows();
            });
        })
        .detach();
    }
}

/// Read the local store once at startup. Never called from render or theme lookup.
pub(crate) fn init(cx: &mut App) {
    if cx.try_global::<ThemeCatalog>().is_some() {
        return;
    }
    let mut catalog = ThemeCatalog::default();
    match zed_themes::installed() {
        Ok(extensions) => catalog.extensions = extensions,
        Err(error) => catalog.error = Some(error),
    }
    match zed_icons::installed() {
        Ok(extensions) => catalog.icon_extensions = extensions,
        Err(error) => catalog.icon_error = Some(error),
    }
    cx.set_global(catalog);

    // Synchronize initial active icon theme from preferences
    if let Some(state) = cx.try_global::<crate::preferences_view::GlobalPreferencesState>()
        && let Some(icon_id) = &state.preferences.app.zed_icon_theme
    {
        let active = find_icon_theme(icon_id, cx);
        crate::app_assets::set_active_icon_theme(active);
    }
}

pub(crate) fn find(id: &str, cx: &App) -> Option<ZedTheme> {
    cx.try_global::<ThemeCatalog>()?
        .extensions
        .iter()
        .flat_map(|extension| &extension.themes)
        .find(|theme| theme.id == id)
        .cloned()
}

pub(crate) fn find_icon_theme(id: &str, cx: &App) -> Option<ZedIconTheme> {
    cx.try_global::<ThemeCatalog>()?
        .icon_extensions
        .iter()
        .flat_map(|extension| &extension.themes)
        .find(|theme| theme.id == id)
        .cloned()
}

/// Accept an exact marketplace URL or a bare extension id, never an arbitrary URL.
pub(crate) fn extension_id(input: &str) -> Result<String, String> {
    let input = input.trim();
    let id = input
        .strip_prefix("https://zed.dev/extensions/")
        .map(|id| id.strip_suffix('/').unwrap_or(id))
        .unwrap_or(input);
    if id.is_empty()
        || id.len() > 128
        || !id.as_bytes()[0].is_ascii_alphanumeric()
        || !id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
    {
        return Err("Enter a Zed extension ID or https://zed.dev/extensions/{id}.".into());
    }
    Ok(id.to_owned())
}

pub(crate) struct ThemesSettings {
    pub(crate) tab: ThemesTab,
    scroll: ScrollHandle,
}

impl ThemesSettings {
    pub(crate) fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        init(cx);
        Self {
            tab: ThemesTab::ColorThemes,
            scroll: ScrollHandle::new(),
        }
    }

    pub(crate) fn set_tab(&mut self, tab: ThemesTab, cx: &mut Context<Self>) {
        self.tab = tab;
        self.scroll = ScrollHandle::new();
        cx.notify();
    }
}

impl Render for ThemesSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let catalog = cx.global::<ThemeCatalog>();
        let tab = self.tab;

        let loading = if tab == ThemesTab::ColorThemes {
            catalog.loading
        } else {
            catalog.icon_loading
        };
        let error = if tab == ThemesTab::ColorThemes {
            catalog.error.clone()
        } else {
            catalog.icon_error.clone()
        };

        let current_zed_theme = crate::theme_picker::selected_zed(cx);
        let current_zed_icon = crate::theme_picker::selected_zed_icon(cx);
        let current_pref_theme = crate::preferences_view::current_prefs(cx).app.theme;

        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                // Header navigation tabs: Color Themes | Icon Themes
                h_flex()
                    .w_full()
                    .px_4()
                    .py_3()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("tab-color-themes")
                                    .small()
                                    .when(tab == ThemesTab::ColorThemes, |b| b.secondary())
                                    .when(tab != ThemesTab::ColorThemes, |b| b.ghost())
                                    .child(
                                        h_flex()
                                            .gap_1p5()
                                            .items_center()
                                            .child(Icon::new(IconName::Palette).xsmall())
                                            .child("Color Themes"),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_tab(ThemesTab::ColorThemes, cx);
                                    })),
                            )
                            .child(
                                Button::new("tab-icon-themes")
                                    .small()
                                    .when(tab == ThemesTab::IconThemes, |b| b.secondary())
                                    .when(tab != ThemesTab::IconThemes, |b| b.ghost())
                                    .child(
                                        h_flex()
                                            .gap_1p5()
                                            .items_center()
                                            .child(
                                                Icon::new(gpui_kit_assets::IconName::Package)
                                                    .xsmall(),
                                            )
                                            .child("Icon Themes"),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_tab(ThemesTab::IconThemes, cx);
                                    })),
                            )
                            .child(
                                Button::new("open-marketplace-dock")
                                    .small()
                                    .secondary()
                                    .icon(gpui_kit_assets::IconName::Download)
                                    .label(if tab == ThemesTab::ColorThemes {
                                        "Install Themes…"
                                    } else {
                                        "Install Icon Themes…"
                                    })
                                    .on_click(cx.listener(move |this, _, _win, cx| {
                                        if this.tab == ThemesTab::ColorThemes {
                                            cx.dispatch_action(&crate::native_menu::InstallTheme);
                                        } else {
                                            cx.dispatch_action(
                                                &crate::native_menu::InstallIconTheme,
                                            );
                                        }
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(if tab == ThemesTab::ColorThemes {
                                "Installed Color Themes"
                            } else {
                                "Installed Icon Themes"
                            }),
                    ),
            )
            .when_some(error, |el, msg| {
                el.child(
                    div()
                        .mx_4()
                        .mt_3()
                        .p_2p5()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(cx.theme().danger)
                        .bg(cx.theme().danger.opacity(0.1))
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(msg),
                )
            })
            // Scrollable installed themes list
            .child(
                v_flex()
                    .id("installed-themes-scroll")
                    .flex_1()
                    .min_h_0()
                    .p_4()
                    .gap_4()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(if tab == ThemesTab::ColorThemes {
                        // --- COLOR THEMES ---
                        let extensions = catalog.extensions.clone();
                        div()
                            .w_full()
                            .flex()
                            .flex_col()
                            .gap_4()
                            // 1. Built-in default themes card
                            .child(
                                v_flex()
                                    .w_full()
                                    .p_4()
                                    .rounded(px(8.0))
                                    .border_1()
                                    .border_color(cx.theme().border)
                                    .bg(cx.theme().secondary.opacity(0.15))
                                    .gap_3()
                                    .child(
                                        h_flex()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                Icon::new(IconName::Palette)
                                                    .size(px(18.0))
                                                    .text_color(cx.theme().primary),
                                            )
                                            .child(
                                                v_flex()
                                                    .gap_0p5()
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .font_weight(FontWeight::SEMIBOLD)
                                                            .child("Default Themes"),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(
                                                                cx.theme().muted_foreground,
                                                            )
                                                            .child(
                                                                "Built-in native Iceon palettes",
                                                            ),
                                                    ),
                                            ),
                                    )
                                    // Variants list
                                    .child(
                                        v_flex()
                                            .w_full()
                                            .gap_1p5()
                                            .children([
                                                (AppTheme::System, "System", "Matches operating system appearance"),
                                                (AppTheme::Dark, "Dark", "Default dark palette"),
                                                (AppTheme::Light, "Light", "Default light palette"),
                                            ].into_iter().map(|(val, label, desc)| {
                                                let is_active = current_zed_theme.is_none() && current_pref_theme == val;
                                                h_flex()
                                                    .w_full()
                                                    .items_center()
                                                    .justify_between()
                                                    .px_3()
                                                    .py_2()
                                                    .rounded(px(6.0))
                                                    .bg(cx.theme().background)
                                                    .border_1()
                                                    .border_color(cx.theme().border.opacity(0.6))
                                                    .child(
                                                        v_flex()
                                                            .gap_0p5()
                                                            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(label))
                                                            .child(div().text_xs().text_color(cx.theme().muted_foreground).child(desc)),
                                                    )
                                                    .child(
                                                        if is_active {
                                                            h_flex()
                                                                .items_center()
                                                                .gap_1()
                                                                .px_2()
                                                                .py_1()
                                                                .rounded(px(4.0))
                                                                .bg(cx.theme().primary.opacity(0.15))
                                                                .text_color(cx.theme().primary)
                                                                .text_xs()
                                                                .font_weight(FontWeight::MEDIUM)
                                                                .child(Icon::new(IconName::Check).xsmall())
                                                                .child("Active")
                                                                .into_any_element()
                                                        } else {
                                                            Button::new(SharedString::from(format!("apply-builtin-{label}")))
                                                                .small()
                                                                .ghost()
                                                                .label("Apply")
                                                                .on_click(cx.listener(move |_, _, _, cx| {
                                                                    crate::preferences_view::mutate_preferences(cx, |p| {
                                                                        p.app.theme = val;
                                                                        p.app.zed_theme = None;
                                                                    });
                                                                    let mode = match val {
                                                                        AppTheme::Light => gpui_kit::component::theme::ThemeMode::Light,
                                                                        AppTheme::Dark => gpui_kit::component::theme::ThemeMode::Dark,
                                                                        _ => gpui_kit::component::theme::ThemeMode::Dark,
                                                                    };
                                                                    crate::shell_theme::apply(mode, None, cx);
                                                                    cx.refresh_windows();
                                                                }))
                                                                .into_any_element()
                                                        }
                                                    )
                                            }))
                                    ),
                            )
                            // 2. Installed extensions
                            .children(extensions.into_iter().map(|ext| {
                                let id = ext.id.clone();
                                let name = if ext.name.is_empty() { ext.id.clone() } else { ext.name.clone() };
                                let description = ext.description.clone();

                                v_flex()
                                    .w_full()
                                    .p_4()
                                    .rounded(px(8.0))
                                    .border_1()
                                    .border_color(cx.theme().border)
                                    .bg(cx.theme().secondary.opacity(0.15))
                                    .gap_3()
                                    // Card header
                                    .child(
                                        h_flex()
                                            .w_full()
                                            .items_center()
                                            .justify_between()
                                            .child(
                                                v_flex()
                                                    .gap_0p5()
                                                    .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(name.clone()))
                                                    .child(div().text_xs().text_color(cx.theme().muted_foreground).child(format!("{} · v{}", ext.id, ext.version)))
                                                    .when(!description.is_empty(), |el| {
                                                        el.child(div().text_xs().text_color(cx.theme().muted_foreground).child(description))
                                                    }),
                                            )
                                            .child(
                                                Button::new(SharedString::from(format!("uninstall-theme-{id}")))
                                                    .small()
                                                    .ghost()
                                                    .icon(crate::action_icons::delete())
                                                    .tooltip(format!("Uninstall {name}"))
                                                    .accessibility_label(format!("Uninstall {name}"))
                                                    .disabled(loading)
                                                    .on_click({
                                                        let id = id.clone();
                                                        cx.listener(move |_, _, _, cx| {
                                                            ThemeCatalog::mutate_theme(&id, true, cx);
                                                        })
                                                    }),
                                            ),
                                    )
                                    // Variants list
                                    .child(
                                        v_flex()
                                            .w_full()
                                            .gap_1p5()
                                            .children(ext.themes.into_iter().map(|theme| {
                                                let theme_id = theme.id.clone();
                                                let theme_name = theme.name.clone();
                                                let appearance = theme.appearance.clone();
                                                let is_active = current_zed_theme == Some(&theme_id);

                                                h_flex()
                                                    .w_full()
                                                    .items_center()
                                                    .justify_between()
                                                    .px_3()
                                                    .py_2()
                                                    .rounded(px(6.0))
                                                    .bg(cx.theme().background)
                                                    .border_1()
                                                    .border_color(cx.theme().border.opacity(0.6))
                                                    .child(
                                                        h_flex()
                                                            .items_center()
                                                            .gap_2()
                                                            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(theme_name))
                                                            .child(
                                                                div()
                                                                    .px_1p5()
                                                                    .py_0p5()
                                                                    .rounded(px(3.0))
                                                                    .bg(cx.theme().secondary.opacity(0.6))
                                                                    .text_xs()
                                                                    .text_color(cx.theme().muted_foreground)
                                                                    .child(appearance.clone()),
                                                            ),
                                                    )
                                                    .child(
                                                        if is_active {
                                                            h_flex()
                                                                .items_center()
                                                                .gap_1()
                                                                .px_2()
                                                                .py_1()
                                                                .rounded(px(4.0))
                                                                .bg(cx.theme().primary.opacity(0.15))
                                                                .text_color(cx.theme().primary)
                                                                .text_xs()
                                                                .font_weight(FontWeight::MEDIUM)
                                                                .child(Icon::new(IconName::Check).xsmall())
                                                                .child("Active")
                                                                .into_any_element()
                                                        } else {
                                                            Button::new(SharedString::from(format!("apply-variant-{theme_id}")))
                                                                .small()
                                                                .ghost()
                                                                .label("Apply")
                                                                .on_click(cx.listener(move |_, _, window, cx| {
                                                                    let mode = if appearance == "light" {
                                                                        gpui_kit::component::theme::ThemeMode::Light
                                                                    } else {
                                                                        gpui_kit::component::theme::ThemeMode::Dark
                                                                    };
                                                                    crate::preferences_view::set_zed_theme(cx, Some(theme_id.clone()));
                                                                    crate::shell_theme::apply(mode, Some(window), cx);
                                                                    cx.refresh_windows();
                                                                }))
                                                                .into_any_element()
                                                        }
                                                    )
                                            }))
                                    )
                            }))
                            .into_any_element()
                    } else {
                        // --- ICON THEMES ---
                        let icon_extensions = catalog.icon_extensions.clone();
                        div()
                            .w_full()
                            .flex()
                            .flex_col()
                            .gap_4()
                            // 1. Built-in default icon pack card
                            .child(
                                v_flex()
                                    .w_full()
                                    .p_4()
                                    .rounded(px(8.0))
                                    .border_1()
                                    .border_color(cx.theme().border)
                                    .bg(cx.theme().secondary.opacity(0.15))
                                    .gap_3()
                                    .child(
                                        h_flex()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                Icon::new(gpui_kit_assets::IconName::Package)
                                                    .size(px(18.0))
                                                    .text_color(cx.theme().primary),
                                            )
                                            .child(
                                                v_flex()
                                                    .gap_0p5()
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .font_weight(FontWeight::SEMIBOLD)
                                                            .child("Default Icon Pack"),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(
                                                                cx.theme().muted_foreground,
                                                            )
                                                            .child(
                                                                "Built-in native Lucide icons",
                                                            ),
                                                    ),
                                            ),
                                    )
                                    // Variants list
                                    .child(
                                        v_flex()
                                            .w_full()
                                            .gap_1p5()
                                            .child({
                                                let is_active = current_zed_icon.is_none();
                                                h_flex()
                                                    .w_full()
                                                    .items_center()
                                                    .justify_between()
                                                    .px_3()
                                                    .py_2()
                                                    .rounded(px(6.0))
                                                    .bg(cx.theme().background)
                                                    .border_1()
                                                    .border_color(cx.theme().border.opacity(0.6))
                                                    .child(
                                                        v_flex()
                                                            .gap_0p5()
                                                            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child("Default (Built-in)"))
                                                            .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Standard Lucide icons bundled with Iceon")),
                                                    )
                                                    .child(
                                                        if is_active {
                                                            h_flex()
                                                                .items_center()
                                                                .gap_1()
                                                                .px_2()
                                                                .py_1()
                                                                .rounded(px(4.0))
                                                                .bg(cx.theme().primary.opacity(0.15))
                                                                .text_color(cx.theme().primary)
                                                                .text_xs()
                                                                .font_weight(FontWeight::MEDIUM)
                                                                .child(Icon::new(IconName::Check).xsmall())
                                                                .child("Active")
                                                                .into_any_element()
                                                        } else {
                                                            Button::new("apply-builtin-icons")
                                                                .small()
                                                                .ghost()
                                                                .label("Apply")
                                                                .on_click(cx.listener(move |_, _, _, cx| {
                                                                    crate::preferences_view::set_zed_icon_theme(cx, None);
                                                                    cx.refresh_windows();
                                                                }))
                                                                .into_any_element()
                                                        }
                                                    )
                                            })
                                    ),
                            )
                            // 2. Installed icon extensions
                            .children(icon_extensions.into_iter().map(|ext| {
                                let id = ext.id.clone();
                                let name = if ext.name.is_empty() { ext.id.clone() } else { ext.name.clone() };
                                let description = ext.description.clone();

                                v_flex()
                                    .w_full()
                                    .p_4()
                                    .rounded(px(8.0))
                                    .border_1()
                                    .border_color(cx.theme().border)
                                    .bg(cx.theme().secondary.opacity(0.15))
                                    .gap_3()
                                    // Card header
                                    .child(
                                        h_flex()
                                            .w_full()
                                            .items_center()
                                            .justify_between()
                                            .child(
                                                v_flex()
                                                    .gap_0p5()
                                                    .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(name.clone()))
                                                    .child(div().text_xs().text_color(cx.theme().muted_foreground).child(format!("{} · v{}", ext.id, ext.version)))
                                                    .when(!description.is_empty(), |el| {
                                                        el.child(div().text_xs().text_color(cx.theme().muted_foreground).child(description))
                                                    }),
                                            )
                                            .child(
                                                Button::new(SharedString::from(format!("uninstall-icon-{id}")))
                                                    .small()
                                                    .ghost()
                                                    .icon(crate::action_icons::delete())
                                                    .tooltip(format!("Uninstall {name}"))
                                                    .accessibility_label(format!("Uninstall {name}"))
                                                    .disabled(loading)
                                                    .on_click({
                                                        let id = id.clone();
                                                        cx.listener(move |_, _, _, cx| {
                                                            ThemeCatalog::mutate_icon_theme(&id, true, cx);
                                                        })
                                                    }),
                                            ),
                                    )
                                    // Variants list
                                    .child(
                                        v_flex()
                                            .w_full()
                                            .gap_1p5()
                                            .children(ext.themes.into_iter().map(|theme| {
                                                let theme_id = theme.id.clone();
                                                let theme_name = theme.name.clone();
                                                let appearance = theme.appearance.clone();
                                                let is_active = current_zed_icon == Some(&theme_id);

                                                h_flex()
                                                    .w_full()
                                                    .items_center()
                                                    .justify_between()
                                                    .px_3()
                                                    .py_2()
                                                    .rounded(px(6.0))
                                                    .bg(cx.theme().background)
                                                    .border_1()
                                                    .border_color(cx.theme().border.opacity(0.6))
                                                    .child(
                                                        h_flex()
                                                            .items_center()
                                                            .gap_2()
                                                            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(theme_name))
                                                            .child(
                                                                div()
                                                                    .px_1p5()
                                                                    .py_0p5()
                                                                    .rounded(px(3.0))
                                                                    .bg(cx.theme().secondary.opacity(0.6))
                                                                    .text_xs()
                                                                    .text_color(cx.theme().muted_foreground)
                                                                    .child(appearance),
                                                            ),
                                                    )
                                                    .child(
                                                        if is_active {
                                                            h_flex()
                                                                .items_center()
                                                                .gap_1()
                                                                .px_2()
                                                                .py_1()
                                                                .rounded(px(4.0))
                                                                .bg(cx.theme().primary.opacity(0.15))
                                                                .text_color(cx.theme().primary)
                                                                .text_xs()
                                                                .font_weight(FontWeight::MEDIUM)
                                                                .child(Icon::new(IconName::Check).xsmall())
                                                                .child("Active")
                                                                .into_any_element()
                                                        } else {
                                                            Button::new(SharedString::from(format!("apply-icon-variant-{theme_id}")))
                                                                .small()
                                                                .ghost()
                                                                .label("Apply")
                                                                .on_click(cx.listener(move |_, _, _, cx| {
                                                                    crate::preferences_view::set_zed_icon_theme(cx, Some(theme_id.clone()));
                                                                    cx.refresh_windows();
                                                                }))
                                                                .into_any_element()
                                                        }
                                                    )
                                            }))
                                    )
                            }))
                            .into_any_element()
                    }),
            )
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::extension_id;

    #[test]
    fn accepts_ids_and_exact_marketplace_urls() {
        for input in [
            "catppuccin",
            " catppuccin ",
            "https://zed.dev/extensions/catppuccin",
            "https://zed.dev/extensions/catppuccin/",
        ] {
            assert_eq!(extension_id(input).unwrap(), "catppuccin");
        }
        assert_eq!(extension_id("github-theme").unwrap(), "github-theme");
        assert_eq!(extension_id("theme_123").unwrap(), "theme_123");
    }

    #[test]
    fn rejects_other_urls_paths_and_invalid_ids() {
        for input in [
            "",
            "../theme",
            "UPPER",
            "-theme",
            "a.b",
            "http://zed.dev/extensions/a",
            "https://evil.test/extensions/a",
            "https://zed.dev.evil.test/extensions/a",
            "https://zed.dev/extensions/a/b",
            "https://zed.dev/extensions/a?x=1",
            "https://zed.dev/extensions/a#theme",
            "https://zed.dev/extensions/%61",
            "https://zed.dev/extensions/a//",
        ] {
            assert!(extension_id(input).is_err(), "{input}");
        }
        assert!(extension_id(&"a".repeat(129)).is_err());
    }
}
