//! Shared theme choices: built-in appearances and locally installed Zed themes only.
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
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
