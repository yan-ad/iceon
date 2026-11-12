//! Data-only Zed theme installation and cached native settings controls.
use crate::empty_state::compact_empty_state;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use iceon_core::zed_themes::{self, InstalledExtension, MarketplaceExtension, ZedTheme};

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
    cx.set_global(catalog);
}

pub(crate) fn find(id: &str, cx: &App) -> Option<ZedTheme> {
    cx.try_global::<ThemeCatalog>()?
        .extensions
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
    extension_input: Entity<InputState>,
    visible: usize,
    variants: std::collections::HashMap<String, String>,
}

impl ThemesSettings {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        init(cx);
        let this = Self {
            visible: 20,
            variants: std::collections::HashMap::new(),
            extension_input: cx
                .new(|cx| InputState::new(window, cx).placeholder("Search theme extensions…")),
        };
        if !cx.global::<ThemeCatalog>().marketplace_loaded {
            Self::browse(String::new(), window, cx);
        }
        this
    }

    fn browse(query: String, window: &mut Window, cx: &mut Context<Self>) {
        if cx.global::<ThemeCatalog>().marketplace_loading {
            return;
        }
        let catalog = cx.global_mut::<ThemeCatalog>();
        catalog.marketplace_loading = true;
        catalog.marketplace_error = None;
        let handle = iceon_core::runtime::spawn(async move {
            (query.clone(), zed_themes::marketplace(&query).await)
        });
        let app = cx.to_async();
        cx.spawn_in(window, async move |_, _| {
            let result = handle.await;
            app.update(|cx| {
                let catalog = cx.global_mut::<ThemeCatalog>();
                catalog.marketplace_loading = false;
                match result {
                    Ok((query, Ok(items))) => {
                        catalog.marketplace = items;
                        catalog.marketplace_query = query;
                        catalog.marketplace_loaded = true;
                    }
                    Ok((_, Err(error))) => catalog.marketplace_error = Some(error),
                    Err(_) => catalog.marketplace_error = Some("Marketplace worker stopped".into()),
                }
                cx.refresh_windows();
            });
        })
        .detach();
        cx.notify();
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
        // Reqwest runs on Tokio, not the GPUI foreground executor. Store reads and
        // removals also stay off the UI thread. Detached completion reconciles the
        // global cache even if this settings entity has already been dismissed.
        let handle = iceon_core::runtime::spawn(async move {
            let operation = if remove {
                iceon_core::runtime::spawn_blocking(move || zed_themes::uninstall(&id))
                    .await
                    .map_err(|_| "Theme removal worker stopped".to_owned())
                    .and_then(|result| result)
            } else {
                zed_themes::install(&id).await.map(|_| ())
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
                                // Never continue advertising stale variants after a failed scan.
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
}

impl Render for ThemesSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let catalog = cx.global::<ThemeCatalog>();
        let loading = catalog.loading;
        let browsing = catalog.marketplace_loading;
        let extensions = catalog.extensions.clone();
        let error = catalog
            .error
            .clone()
            .or_else(|| catalog.marketplace_error.clone());
        let market = catalog.marketplace.clone();
        let query = catalog.marketplace_query.trim().to_lowercase();
        let active = crate::theme_picker::selected_zed(cx).map(str::to_owned);
        let current = cx
            .try_global::<crate::preferences_view::GlobalPreferencesState>()
            .map(|state| state.preferences.app.theme)
            .unwrap_or(iceon_core::preferences::AppTheme::System);
        let remaining: Vec<_> = market
            .into_iter()
            .filter(|item| !extensions.iter().any(|extension| extension.id == item.id))
            .collect();
        let more = remaining.len() > self.visible;
        let mut rows: Vec<AnyElement> = Vec::new();
        for (index, theme) in crate::theme_picker::DEFAULTS.into_iter().enumerate() {
            let selected = active.is_none() && current == theme;
            rows.push(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2()
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
                                    .child(theme.label()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(if theme == iceon_core::preferences::AppTheme::System {
                                        "Built-in · follows system appearance"
                                    } else {
                                        "Built-in appearance"
                                    }),
                            ),
                    )
                    .child(
                        Button::new(("default-theme-apply", index))
                            .small()
                            .ghost()
                            .label(if selected { "Applied" } else { "Apply" })
                            .disabled(selected)
                            .on_click(move |_, _, cx| {
                                crate::preferences_view::set_theme(cx, theme)
                            }),
                    )
                    .into_any_element(),
            );
        }
        let built_in_rows = rows.len();
        for extension in extensions.iter().filter(|extension| {
            query.is_empty()
                || extension.id.to_lowercase().contains(&query)
                || extension
                    .themes
                    .iter()
                    .any(|theme| theme.name.to_lowercase().contains(&query))
        }) {
            let id = extension.id.clone();
            let variant = self
                .variants
                .get(&id)
                .and_then(|selected| extension.themes.iter().find(|theme| &theme.id == selected))
                .or_else(|| {
                    extension
                        .themes
                        .iter()
                        .find(|theme| Some(&theme.id) == active.as_ref())
                })
                .or_else(|| extension.themes.first());
            let variant_id = variant.map(|theme| theme.id.clone());
            let applied = variant_id.is_some() && variant_id == active;
            let label = variant
                .map(|theme| theme.name.clone())
                .unwrap_or_else(|| "No variants".into());
            let variants = extension.themes.clone();
            let owner = cx.entity().downgrade();
            let extension_id = id.clone();
            let selected_variant = variant_id.clone();
            rows.push(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
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
                                    .truncate()
                                    .child(id.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(
                                        "Installed · {} · {} variants",
                                        extension.version,
                                        extension.themes.len()
                                    )),
                            ),
                    )
                    .child(
                        Button::new(SharedString::from(format!("variant-{id}")))
                            .small()
                            .outline()
                            .max_w(px(260.))
                            .label(label)
                            .dropdown_menu(move |mut menu, _, _| {
                                for theme in &variants {
                                    let owner = owner.clone();
                                    let extension_id = extension_id.clone();
                                    let theme_id = theme.id.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new(theme.name.clone())
                                            .checked(Some(&theme.id) == selected_variant.as_ref())
                                            .on_click(move |_, _, cx| {
                                                let _ = owner.update(cx, |view, cx| {
                                                    view.variants.insert(
                                                        extension_id.clone(),
                                                        theme_id.clone(),
                                                    );
                                                    cx.notify();
                                                });
                                            }),
                                    );
                                }
                                menu
                            }),
                    )
                    .child(
                        Button::new(SharedString::from(format!("apply-{id}")))
                            .small()
                            .ghost()
                            .label(if applied { "Applied" } else { "Apply" })
                            .disabled(loading || applied || variant_id.is_none())
                            .on_click(move |_, _, cx| {
                                crate::preferences_view::set_zed_theme(cx, variant_id.clone())
                            }),
                    )
                    .child(
                        Button::new(SharedString::from(format!("uninstall-{id}")))
                            .small()
                            .ghost()
                            .label("Uninstall")
                            .disabled(loading)
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.change(&id, true, window, cx)
                            })),
                    )
                    .into_any_element(),
            );
        }
        for item in remaining.into_iter().take(self.visible) {
            let id = item.id.clone();
            rows.push(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2()
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
                                    .truncate()
                                    .child(item.name),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .truncate()
                                    .child(format!(
                                        "{} · {} downloads",
                                        item.id, item.download_count
                                    )),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .truncate()
                                    .child(item.description),
                            ),
                    )
                    .child(
                        Button::new(SharedString::from(format!("install-{id}")))
                            .small()
                            .outline()
                            .label("Install")
                            .disabled(loading)
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.change(&id, false, window, cx)
                            })),
                    )
                    .into_any_element(),
            );
        }
        let no_search_results = !query.is_empty()
            && catalog.marketplace_loaded
            && !browsing
            && !loading
            && error.is_none()
            && rows.len() == built_in_rows;
        v_flex()
            .size_full()
            .min_h_0()
            .min_w_0()
            .p_5()
            .gap_3()
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Themes"),
            )
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.extension_input).small()),
                    )
                    .child(
                        Button::new("theme-search")
                            .small()
                            .outline()
                            .label("Search / Refresh")
                            .disabled(browsing)
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.visible = 20;
                                Self::browse(
                                    view.extension_input.read(cx).value().to_string(),
                                    window,
                                    cx,
                                );
                            })),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(if browsing {
                        "Searching marketplace…"
                    } else if loading {
                        "Updating theme installation…"
                    } else {
                        "Built-in, installed, and marketplace themes · theme data only"
                    }),
            )
            .when_some(error, |el, error| {
                el.child(div().text_sm().text_color(cx.theme().danger).child(error))
            })
            .child(
                v_flex()
                    .id("unified-theme-list")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scroll()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(px(8.))
                    .children(rows)
                    .when(no_search_results, |el| {
                        el.child(compact_empty_state(
                            IconName::Search,
                            "No themes found",
                            "Try a different theme name or extension ID.",
                        ))
                    })
                    .when(more, |el| {
                        el.child(
                            Button::new("marketplace-more")
                                .small()
                                .ghost()
                                .label("Show more")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.visible += 20;
                                    cx.notify();
                                })),
                        )
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
