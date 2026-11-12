//! Shared theme choices: built-in appearances and locally installed Zed themes only.
use gpui_kit::component::{
    Selectable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::{PopupMenu, PopupMenuItem},
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use iceon_core::preferences::AppTheme;

pub(crate) const DEFAULTS: [AppTheme; 3] = [AppTheme::System, AppTheme::Dark, AppTheme::Light];

pub(crate) fn selected_zed(cx: &App) -> Option<&str> {
    cx.try_global::<crate::preferences_view::GlobalPreferencesState>()
        .and_then(|state| state.preferences.app.zed_theme.as_deref())
}

fn selected(cx: &App) -> AppTheme {
    cx.try_global::<crate::preferences_view::GlobalPreferencesState>()
        .map(|state| state.preferences.app.theme)
        .unwrap_or(AppTheme::System)
}

pub(crate) fn menu(mut menu: PopupMenu, cx: &App) -> PopupMenu {
    let current = selected(cx);
    let installed = selected_zed(cx);
    for theme in DEFAULTS {
        menu = menu.item(
            PopupMenuItem::new(theme.label())
                .checked(installed.is_none() && theme == current)
                .on_click(move |_, _, cx| crate::preferences_view::set_theme(cx, theme)),
        );
    }
    if let Some(catalog) = cx.try_global::<crate::themes_settings::ThemeCatalog>() {
        for extension in &catalog.extensions {
            menu = menu
                .separator()
                .item(PopupMenuItem::label(extension.id.clone()));
            for theme in &extension.themes {
                let id = theme.id.clone();
                menu = menu.item(
                    PopupMenuItem::new(theme.name.clone())
                        .checked(installed == Some(theme.id.as_str()))
                        .on_click(move |_, _, cx| {
                            crate::preferences_view::set_zed_theme(cx, Some(id.clone()))
                        }),
                );
            }
        }
    }
    menu
}

/// Compact appearance controls for callers that still embed them in settings.
pub(crate) fn appearance(cx: &App) -> AnyElement {
    let current = selected(cx);
    let installed = selected_zed(cx).is_some();
    h_flex()
        .flex_wrap()
        .gap_2()
        .children(DEFAULTS.into_iter().enumerate().map(|(ix, theme)| {
            Button::new(("default-theme", ix))
                .ghost()
                .label(theme.label())
                .selected(!installed && current == theme)
                .on_click(move |_, _, cx| crate::preferences_view::set_theme(cx, theme))
        }))
        .children(
            cx.try_global::<crate::themes_settings::ThemeCatalog>()
                .into_iter()
                .flat_map(|catalog| &catalog.extensions)
                .flat_map(|extension| &extension.themes)
                .enumerate()
                .map(|(ix, theme)| {
                    let id = theme.id.clone();
                    Button::new(("installed-appearance", ix))
                        .ghost()
                        .label(theme.name.clone())
                        .selected(selected_zed(cx) == Some(theme.id.as_str()))
                        .on_click(move |_, _, cx| {
                            crate::preferences_view::set_zed_theme(cx, Some(id.clone()))
                        })
                }),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::DEFAULTS;
    #[test]
    fn defaults_are_only_system_dark_and_light() {
        assert_eq!(
            DEFAULTS.map(|theme| theme.label()),
            ["System", "Dark", "Light"]
        );
    }
}
