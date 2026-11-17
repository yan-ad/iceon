//! Data-only Zed theme and icon theme installation and cached native settings controls.
use crate::empty_state::compact_empty_state;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
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
fn extension_id(input: &str) -> Result<String, String> {
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
    tab: ThemesTab,
    extension_input: Entity<InputState>,
    icon_extension_input: Entity<InputState>,
    marketplace_mode: bool,
    visible: usize,
    scroll: ScrollHandle,
    search_task: Option<Task<()>>,
}

impl ThemesSettings {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        init(cx);
        let extension_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search color theme extensions…"));
        cx.subscribe_in(
            &extension_input,
            window,
            |view, input, event, window, cx| {
                if matches!(event, InputEvent::Change)
                    && view.marketplace_mode
                    && view.tab == ThemesTab::ColorThemes
                {
                    let query = input.read(cx).value().to_string();
                    view.schedule_search(query, window, cx);
                }
            },
        )
        .detach();

        let icon_extension_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search icon theme extensions…"));
        cx.subscribe_in(
            &icon_extension_input,
            window,
            |view, input, event, window, cx| {
                if matches!(event, InputEvent::Change)
                    && view.marketplace_mode
                    && view.tab == ThemesTab::IconThemes
                {
                    let query = input.read(cx).value().to_string();
                    view.schedule_icon_search(query, window, cx);
                }
            },
        )
        .detach();

        Self {
            tab: ThemesTab::ColorThemes,
            extension_input,
            icon_extension_input,
            marketplace_mode: false,
            visible: 20,
            scroll: ScrollHandle::new(),
            search_task: None,
        }
    }

    /// Preferences opens installed management by default. Menu/picker integrations
    /// can explicitly open this same view in marketplace mode.
    pub(crate) fn open_marketplace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.marketplace_mode = true;
        self.visible = 20;
        self.scroll = ScrollHandle::new();
        if self.tab == ThemesTab::ColorThemes {
            let query = self.extension_input.read(cx).value().to_string();
            self.schedule_search(query, window, cx);
        } else {
            let query = self.icon_extension_input.read(cx).value().to_string();
            self.schedule_icon_search(query, window, cx);
        }
        cx.notify();
    }

    pub(crate) fn open_icon_marketplace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tab = ThemesTab::IconThemes;
        self.open_marketplace(window, cx);
    }

    fn schedule_search(&mut self, query: String, window: &mut Window, cx: &mut Context<Self>) {
        self.visible = 20;
        self.scroll = ScrollHandle::new();
        let catalog = cx.global_mut::<ThemeCatalog>();
        catalog.marketplace_generation += 1;
        let generation = catalog.marketplace_generation;
        catalog.marketplace_loading = true;
        catalog.marketplace_loaded = false;
        catalog.marketplace.clear();
        catalog.marketplace_error = None;
        catalog.marketplace_query = query.clone();
        self.search_task = Some(cx.spawn_in(window, async move |view, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(300))
                .await;
            let _ = view.update_in(cx, |_, window, cx| {
                Self::browse(query, generation, window, cx);
            });
        }));
        cx.notify();
    }

    fn browse(query: String, generation: u64, window: &mut Window, cx: &mut Context<Self>) {
        if cx.global::<ThemeCatalog>().marketplace_generation != generation {
            return;
        }
        let handle =
            iceon_core::runtime::spawn(async move { zed_themes::marketplace(&query).await });
        let app = cx.to_async();
        cx.spawn_in(window, async move |_, _| {
            let result = handle.await;
            app.update(|cx| {
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

    fn schedule_icon_search(&mut self, query: String, window: &mut Window, cx: &mut Context<Self>) {
        self.visible = 20;
        self.scroll = ScrollHandle::new();
        let catalog = cx.global_mut::<ThemeCatalog>();
        catalog.marketplace_icons_generation += 1;
        let generation = catalog.marketplace_icons_generation;
        catalog.marketplace_icons_loading = true;
        catalog.marketplace_icons_loaded = false;
        catalog.marketplace_icons.clear();
        catalog.marketplace_icons_error = None;
        catalog.marketplace_icons_query = query.clone();
        self.search_task = Some(cx.spawn_in(window, async move |view, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(300))
                .await;
            let _ = view.update_in(cx, |_, window, cx| {
                Self::browse_icons(query, generation, window, cx);
            });
        }));
        cx.notify();
    }

    fn browse_icons(query: String, generation: u64, window: &mut Window, cx: &mut Context<Self>) {
        if cx.global::<ThemeCatalog>().marketplace_icons_generation != generation {
            return;
        }
        let handle =
            iceon_core::runtime::spawn(async move { zed_icons::marketplace(&query).await });
        let app = cx.to_async();
        cx.spawn_in(window, async move |_, _| {
            let result = handle.await;
            app.update(|cx| {
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

    fn change(&mut self, input: &str, remove: bool, window: &mut Window, cx: &mut Context<Self>) {
        if cx.global::<ThemeCatalog>().loading {
            return;
        }
        let id = match extension_id(input) {
            Ok(id) => id,
            Err(error) => {
                cx.global_mut::<ThemeCatalog>().error = Some(error);
                cx.notify();
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
        let app = cx.to_async();
        cx.spawn_in(window, async move |_, _| {
            let result = handle.await;
            app.update(|cx| {
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
        cx.notify();
        cx.refresh_windows();
    }

    fn change_icon(
        &mut self,
        input: &str,
        remove: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if cx.global::<ThemeCatalog>().icon_loading {
            return;
        }
        let id = match extension_id(input) {
            Ok(id) => id,
            Err(error) => {
                cx.global_mut::<ThemeCatalog>().icon_error = Some(error);
                cx.notify();
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
        let app = cx.to_async();
        cx.spawn_in(window, async move |_, _| {
            let result = handle.await;
            app.update(|cx| {
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
        cx.notify();
        cx.refresh_windows();
    }
}

impl Render for ThemesSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let catalog = cx.global::<ThemeCatalog>();
        let tab = self.tab;
        let marketplace_mode = self.marketplace_mode;

        let loading = if tab == ThemesTab::ColorThemes {
            catalog.loading
        } else {
            catalog.icon_loading
        };
        let browsing = if tab == ThemesTab::ColorThemes {
            catalog.marketplace_loading
        } else {
            catalog.marketplace_icons_loading
        };
        let error = if tab == ThemesTab::ColorThemes {
            catalog.error.clone().or_else(|| {
                marketplace_mode
                    .then(|| catalog.marketplace_error.clone())
                    .flatten()
            })
        } else {
            catalog.icon_error.clone().or_else(|| {
                marketplace_mode
                    .then(|| catalog.marketplace_icons_error.clone())
                    .flatten()
            })
        };

        let mut rows: Vec<AnyElement> = Vec::new();

        if tab == ThemesTab::ColorThemes {
            let extensions = catalog.extensions.clone();
            let market = catalog.marketplace.clone();
            if !marketplace_mode {
                for extension in &extensions {
                    let id = extension.id.clone();
                    let name = if extension.name.is_empty() {
                        id.clone()
                    } else {
                        extension.name.clone()
                    };
                    let downloads = extension
                        .download_count
                        .map(|n| format!(" · {n} downloads"))
                        .unwrap_or_default();
                    rows.push(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .items_center()
                            .gap_3()
                            .px_3()
                            .py_3()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(name.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(format!(
                                                "{} · v{} · {} variants{}",
                                                id,
                                                extension.version,
                                                extension.themes.len(),
                                                downloads
                                            )),
                                    )
                                    .when(!extension.description.is_empty(), |el| {
                                        el.child(
                                            div()
                                                .text_sm()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(extension.description.clone()),
                                        )
                                    }),
                            )
                            .child(
                                Button::new(SharedString::from(format!("uninstall-{id}")))
                                    .small()
                                    .ghost()
                                    .icon(crate::action_icons::delete())
                                    .tooltip(format!("Uninstall {name}"))
                                    .accessibility_label(format!("Uninstall {name}"))
                                    .disabled(loading)
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.change(&id, true, window, cx)
                                    })),
                            )
                            .into_any_element(),
                    );
                }
            } else {
                for item in market.into_iter().take(self.visible) {
                    let id = item.id.clone();
                    let installed = extensions.iter().any(|extension| extension.id == id);
                    let label = if installed {
                        format!("{} is installed", item.name)
                    } else {
                        format!("Install {}", item.name)
                    };
                    rows.push(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .items_center()
                            .gap_3()
                            .px_3()
                            .py_3()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(item.name),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(format!(
                                                "{} · {} downloads{}",
                                                item.id,
                                                item.download_count,
                                                if installed { " · Installed" } else { "" }
                                            )),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(item.description),
                                    ),
                            )
                            .child(
                                Button::new(SharedString::from(format!("install-{id}")))
                                    .small()
                                    .ghost()
                                    .icon(if installed {
                                        IconName::Check
                                    } else {
                                        IconName::Plus
                                    })
                                    .tooltip(label.clone())
                                    .accessibility_label(label)
                                    .disabled(loading || installed)
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.change(&id, false, window, cx)
                                    })),
                            )
                            .into_any_element(),
                    );
                }
            }
        } else {
            // Icon Themes
            let icon_extensions = catalog.icon_extensions.clone();
            let market_icons = catalog.marketplace_icons.clone();
            if !marketplace_mode {
                // Default Icon Pack Row
                let is_default_active = crate::theme_picker::selected_zed_icon(cx).is_none();
                rows.push(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .items_center()
                        .gap_3()
                        .px_3()
                        .py_3()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child("Default Icon Pack"),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("Built-in native Lucide icons"),
                                ),
                        )
                        .child(if is_default_active {
                            Button::new("active-default-icons")
                                .small()
                                .ghost()
                                .disabled(true)
                                .label("Active")
                        } else {
                            Button::new("use-default-icons")
                                .small()
                                .outline()
                                .label("Use")
                                .on_click(cx.listener(|_, _, _, cx| {
                                    crate::preferences_view::set_zed_icon_theme(cx, None);
                                }))
                        })
                        .into_any_element(),
                );

                for extension in &icon_extensions {
                    let id = extension.id.clone();
                    let name = if extension.name.is_empty() {
                        id.clone()
                    } else {
                        extension.name.clone()
                    };
                    let downloads = extension
                        .download_count
                        .map(|n| format!(" · {n} downloads"))
                        .unwrap_or_default();

                    let themes_list = extension
                        .themes
                        .iter()
                        .map(|theme| {
                            let theme_id = theme.id.clone();
                            let is_active =
                                crate::theme_picker::selected_zed_icon(cx) == Some(&theme_id);
                            h_flex()
                                .items_center()
                                .justify_between()
                                .py_1()
                                .px_2()
                                .rounded_md()
                                .bg(cx.theme().muted.opacity(0.3))
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .items_center()
                                        .child(
                                            div()
                                                .text_xs()
                                                .font_weight(FontWeight::MEDIUM)
                                                .child(theme.name.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(format!("({})", theme.appearance)),
                                        ),
                                )
                                .child(if is_active {
                                    Button::new(SharedString::from(format!("active-{}", theme_id)))
                                        .xsmall()
                                        .ghost()
                                        .disabled(true)
                                        .label("Active")
                                } else {
                                    Button::new(SharedString::from(format!("use-{}", theme_id)))
                                        .xsmall()
                                        .outline()
                                        .label("Use")
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            crate::preferences_view::set_zed_icon_theme(
                                                cx,
                                                Some(theme_id.clone()),
                                            );
                                        }))
                                })
                        })
                        .collect::<Vec<_>>();

                    rows.push(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .items_start()
                            .gap_3()
                            .px_3()
                            .py_3()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(name.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(format!(
                                                "{} · v{} · {} variants{}",
                                                id,
                                                extension.version,
                                                extension.themes.len(),
                                                downloads
                                            )),
                                    )
                                    .when(!extension.description.is_empty(), |el| {
                                        el.child(
                                            div()
                                                .text_sm()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(extension.description.clone()),
                                        )
                                    })
                                    .child(v_flex().gap_1().pt_1().children(themes_list)),
                            )
                            .child(
                                Button::new(SharedString::from(format!("uninstall-icon-{id}")))
                                    .small()
                                    .ghost()
                                    .icon(crate::action_icons::delete())
                                    .tooltip(format!("Uninstall {name}"))
                                    .accessibility_label(format!("Uninstall {name}"))
                                    .disabled(loading)
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.change_icon(&id, true, window, cx)
                                    })),
                            )
                            .into_any_element(),
                    );
                }
            } else {
                for item in market_icons.into_iter().take(self.visible) {
                    let id = item.id.clone();
                    let installed = icon_extensions.iter().any(|extension| extension.id == id);
                    let label = if installed {
                        format!("{} is installed", item.name)
                    } else {
                        format!("Install {}", item.name)
                    };
                    rows.push(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .items_center()
                            .gap_3()
                            .px_3()
                            .py_3()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(item.name),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(format!(
                                                "{} · {} downloads{}",
                                                item.id,
                                                item.download_count,
                                                if installed { " · Installed" } else { "" }
                                            )),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(item.description),
                                    ),
                            )
                            .child(
                                Button::new(SharedString::from(format!("install-icon-{id}")))
                                    .small()
                                    .ghost()
                                    .icon(if installed {
                                        IconName::Check
                                    } else {
                                        IconName::Plus
                                    })
                                    .tooltip(label.clone())
                                    .accessibility_label(label)
                                    .disabled(loading || installed)
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.change_icon(&id, false, window, cx)
                                    })),
                            )
                            .into_any_element(),
                    );
                }
            }
        }

        let is_color = tab == ThemesTab::ColorThemes;
        let extensions_count = if is_color {
            catalog.extensions.len()
        } else {
            catalog.icon_extensions.len()
        };
        let market_len = if is_color {
            catalog.marketplace.len()
        } else {
            catalog.marketplace_icons.len()
        };
        let more = marketplace_mode && market_len > self.visible;
        let loaded = if is_color {
            catalog.marketplace_loaded
        } else {
            catalog.marketplace_icons_loaded
        };
        let no_results =
            marketplace_mode && loaded && !browsing && error.is_none() && market_len == 0;

        v_flex()
            .size_full()
            .min_h_0()
            .min_w_0()
            .p_5()
            .gap_3()
            // Tab switcher
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("tab-colors")
                                    .small()
                                    .when(is_color, |b| b.outline())
                                    .when(!is_color, |b| b.ghost())
                                    .label("Color Themes")
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.tab = ThemesTab::ColorThemes;
                                        view.marketplace_mode = false;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("tab-icons")
                                    .small()
                                    .when(!is_color, |b| b.outline())
                                    .when(is_color, |b| b.ghost())
                                    .label("Icon Themes")
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.tab = ThemesTab::IconThemes;
                                        view.marketplace_mode = false;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        Button::new("themes-mode")
                            .small()
                            .outline()
                            .label(if marketplace_mode {
                                "Installed Themes"
                            } else {
                                "Browse Marketplace"
                            })
                            .on_click(cx.listener(|view, _, window, cx| {
                                if view.marketplace_mode {
                                    view.marketplace_mode = false;
                                    view.search_task = None;
                                    let catalog = cx.global_mut::<ThemeCatalog>();
                                    catalog.marketplace_generation += 1;
                                    catalog.marketplace_loading = false;
                                    catalog.marketplace_icons_generation += 1;
                                    catalog.marketplace_icons_loading = false;
                                    view.scroll = ScrollHandle::new();
                                    cx.notify();
                                } else {
                                    view.open_marketplace(window, cx);
                                }
                            })),
                    ),
            )
            .when(marketplace_mode && is_color, |el| {
                el.child(Input::new(&self.extension_input).small())
            })
            .when(marketplace_mode && !is_color, |el| {
                el.child(Input::new(&self.icon_extension_input).small())
            })
            .child(
                div().text_xs().text_color(cx.theme().muted_foreground).child(
                    if browsing && marketplace_mode {
                        "Searching marketplace…"
                    } else if loading {
                        "Updating theme installation…"
                    } else if marketplace_mode {
                        "Browse Zed extensions · scroll for more · theme data only"
                    } else if is_color {
                        "Manage installed color themes. Choose appearance and variants in the theme picker."
                    } else {
                        "Manage installed icon themes. Select active icon packs alongside the default built-in icons."
                    },
                ),
            )
            .when_some(error, |el, error| {
                el.child(div().text_sm().text_color(cx.theme().danger).child(error))
            })
            .child(
                v_flex()
                    .id(if marketplace_mode {
                        "marketplace-theme-list"
                    } else {
                        "installed-theme-list"
                    })
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .on_scroll(cx.listener(move |view, _: &ScrollEvent, _, cx| {
                        if more && view.scroll.max_offset().y + view.scroll.offset().y <= px(240.) {
                            view.visible = view.visible.saturating_add(20);
                            cx.notify();
                        }
                    }))
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(px(8.))
                    .children(rows)
                    .when(!marketplace_mode && extensions_count == 0 && is_color, |el| {
                        el.child(compact_empty_state(
                            IconName::Palette,
                            "No installed themes",
                            "Install a theme extension from the marketplace. Built-in appearances are available in the theme picker.",
                        ))
                    })
                    .when(no_results, |el| {
                        el.child(compact_empty_state(
                            IconName::Search,
                            "No themes found",
                            "Try a different theme name or extension ID.",
                        ))
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
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
