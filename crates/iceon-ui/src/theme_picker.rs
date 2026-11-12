//! Shared theme choices: built-in appearances and locally installed Zed themes only.
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

impl crate::IceonApp {
    pub(crate) fn toggle_theme_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.theme_picker_open = !self.theme_picker_open;
        self.search_modal_open = false;
        if self.theme_picker_open {
            self.theme_command_state.update(cx, |state, cx| {
                state.set_query("", window, cx);
                state.focus(window, cx);
            });
        } else {
            self.shell_focus.focus(window, cx);
        }
        cx.notify();
    }
    pub(crate) fn render_theme_picker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::{
            Sizable as _,
            button::{Button, ButtonVariants as _},
            command::{Command, CommandItem},
            h_flex,
        };
        let mut choices: Vec<(String, Option<String>, AppTheme)> = DEFAULTS
            .into_iter()
            .map(|theme| (theme.label().into(), None, theme))
            .collect();
        if let Some(catalog) = cx.try_global::<crate::themes_settings::ThemeCatalog>() {
            let mut installed = catalog
                .extensions
                .iter()
                .flat_map(|e| &e.themes)
                .map(|t| (t.name.clone(), Some(t.id.clone()), AppTheme::System))
                .collect::<Vec<_>>();
            installed.sort_by(|a, b| a.0.cmp(&b.0));
            choices.extend(installed);
        }
        let items = choices
            .iter()
            .map(|(label, id, theme)| {
                CommandItem::new()
                    .label(label.clone())
                    .checked(if let Some(id) = id {
                        selected_zed(cx) == Some(id.as_str())
                    } else {
                        selected_zed(cx).is_none() && selected(cx) == *theme
                    })
            })
            .collect::<Vec<_>>();
        let owner = cx.entity().downgrade();
        let cancel = owner.clone();
        let install = owner.clone();
        Command::new(&self.theme_command_state)
            .w(px(560.))
            .max_w(relative(0.9))
            .max_h(px(420.))
            .placeholder("Select Theme…")
            .items(items)
            .on_confirm(move |index, window, cx| {
                if let Some((_, id, theme)) = choices.get(index.row) {
                    if let Some(id) = id {
                        crate::preferences_view::set_zed_theme(cx, Some(id.clone()));
                    } else {
                        crate::preferences_view::set_theme(cx, *theme);
                    }
                    let _ = owner.update(cx, |app, cx| {
                        app.theme_picker_open = false;
                        app.shell_focus.focus(window, cx);
                        cx.notify();
                    });
                }
            })
            .on_cancel(move |window, cx| {
                let _ = cancel.update(cx, |app, cx| {
                    app.theme_picker_open = false;
                    app.shell_focus.focus(window, cx);
                    cx.notify();
                });
            })
            .footer(move |_, _, _| {
                let install = install.clone();
                h_flex()
                    .px_3()
                    .py_2()
                    .justify_between()
                    .child("↑↓ Navigate · Enter Apply")
                    .child(
                        Button::new("install-themes")
                            .small()
                            .ghost()
                            .label("Install Themes")
                            .on_click(move |_, window, cx| {
                                let _ = install.update(cx, |app, cx| {
                                    app.theme_picker_open = false;
                                    app.open_theme_management(window, cx);
                                });
                            }),
                    )
            })
    }
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
