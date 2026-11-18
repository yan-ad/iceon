//! Native OS window for Settings / Preferences using `gpui_kit::component::setting::Settings`,
//! backed by `PreferencesEngine` with live GPUI theme updates across all open windows.

use std::sync::Arc;

use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _, StyledExt as _, TitleBar,
    button::{Button, ButtonVariants as _},
    group_box::GroupBoxVariant,
    h_flex,
    select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState},
    setting::{NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings},
    theme::ThemeMode,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use gpui_kit_assets::IconName as AssetIconName;

use crate::IceonApp;
#[path = "kubernetes_preferences.rs"]
mod kubernetes_preferences;
use iceon_core::preferences::{
    AppTheme, LineNumbers, MinimapSide, Preferences, PreferencesEngine, PreferencesSnapshot,
    TerminalTheme,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreferencesSection {
    General,
    Themes,
    Extensions,
    Proxy,
    Kubernetes,
    Editor,
    Terminal,
}

pub(crate) fn set_theme(cx: &mut App, theme: AppTheme) {
    mutate_preferences(cx, |preferences| {
        preferences.app.theme = theme;
        preferences.app.zed_theme = None;
    });
}

pub(crate) fn set_zed_theme(cx: &mut App, id: Option<String>) {
    if id
        .as_ref()
        .is_some_and(|id| crate::themes_settings::find(id, cx).is_none())
    {
        return;
    }
    let dark = id
        .as_ref()
        .and_then(|id| crate::themes_settings::find(id, cx))
        .map(|theme| theme.appearance == "dark");
    mutate_preferences(cx, |preferences| {
        preferences.app.zed_theme = id;
        preferences.app.theme = match dark {
            Some(true) => AppTheme::Dark,
            Some(false) => AppTheme::Light,
            None => AppTheme::System,
        };
    });
}

pub(crate) fn set_zed_icon_theme(cx: &mut App, id: Option<String>) {
    if id
        .as_ref()
        .is_some_and(|id| crate::themes_settings::find_icon_theme(id, cx).is_none())
    {
        return;
    }
    let active = id
        .as_ref()
        .and_then(|id| crate::themes_settings::find_icon_theme(id, cx));
    crate::app_assets::set_active_icon_theme(active);
    mutate_preferences(cx, |preferences| {
        preferences.app.zed_icon_theme = id;
    });
}

#[cfg(test)]
mod tests {
    use super::{PREFERENCE_SECTIONS, SelectItem as _, TimezoneChoice, timezone_choices};

    #[test]
    fn delayed_import_snapshot_cannot_revert_newer_main_or_global_font_state() {
        use iceon_core::preferences::{Preferences, PreferencesSnapshot};
        let delayed = PreferencesSnapshot {
            revision: 2,
            preferences: Preferences::default(),
        };
        let mut newest = delayed.clone();
        newest.revision = 3;
        newest.preferences.app.font_family = "Fixture Sans".into();
        newest.preferences.editor.font_size = 18;
        for _cache in ["main", "global"] {
            let mut revision = 1;
            let mut preferences = Preferences::default();
            assert!(super::accept_preferences_snapshot(
                &mut revision,
                &mut preferences,
                &newest,
            ));
            assert!(!super::accept_preferences_snapshot(
                &mut revision,
                &mut preferences,
                &delayed,
            ));
            assert_eq!(revision, newest.revision);
            assert_eq!(preferences, newest.preferences);
        }
    }

    #[test]
    fn preferences_navigation_has_only_the_requested_sections_in_order() {
        assert_eq!(
            PREFERENCE_SECTIONS,
            [
                "General",
                "Themes",
                "Extensions",
                "Proxy",
                "Kubernetes",
                "Editor",
                "Terminal"
            ]
        );
    }

    #[test]
    fn timezone_choices_pin_system_and_utc_before_sorted_unique_names() {
        let choices = timezone_choices("Asia/Jakarta");
        assert_eq!(choices[0].value(), "system");
        assert_eq!(choices[0].title().as_ref(), "System");
        assert_eq!(choices[1].value(), "UTC");
        assert!(choices[2..].windows(2).all(|pair| pair[0].0 < pair[1].0));
        for name in ["system", "UTC", "Asia/Jakarta"] {
            assert_eq!(choices.iter().filter(|choice| choice.0 == name).count(), 1);
        }
    }

    #[test]
    fn timezone_choices_include_the_available_iana_database() {
        let choices = timezone_choices("system");
        for name in [
            "Africa/Abidjan",
            "America/New_York",
            "Europe/London",
            "Asia/Tokyo",
        ] {
            assert!(
                choices.iter().any(|choice| choice.0 == name),
                "missing {name}"
            );
        }
    }

    #[test]
    fn timezone_choices_preserve_saved_values_not_in_the_database() {
        let choices = timezone_choices("Legacy/Timezone");
        assert!(choices.iter().any(|choice| choice.0 == "Legacy/Timezone"));
        assert!(
            !timezone_choices("")
                .iter()
                .any(|choice| choice.0.is_empty())
        );
    }

    #[test]
    fn timezone_search_matches_regions_and_cities_case_insensitively() {
        let jakarta = TimezoneChoice("Asia/Jakarta".into());
        assert!(jakarta.matches("jakarta"));
        assert!(jakarta.matches(" ASIA/ "));
        assert!(!jakarta.matches("London"));
        assert!(TimezoneChoice("system".into()).matches("System"));
    }

    #[test]
    fn icon_theme_setting_item_is_valid_and_does_not_panic() {
        use gpui_kit::component::setting::{SettingField, SettingItem};
        use gpui_kit::{App, SharedString};

        let field = SettingField::dropdown(
            vec![(
                SharedString::from("default"),
                SharedString::from("Default (Built-in)"),
            )],
            |_: &App| SharedString::from("default"),
            |_: SharedString, _: &mut App| {},
        )
        .default_value(SharedString::from("default"))
        .on_reset(|_| false, |_, _| {});

        let item = SettingItem::new("Icon Theme", field);
        match item {
            SettingItem::Item { title, .. } => {
                assert_eq!(title.as_ref(), "Icon Theme");
            }
            _ => panic!("Expected SettingItem::Item"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TimezoneChoice(String);

impl SelectItem for TimezoneChoice {
    type Value = String;

    fn title(&self) -> SharedString {
        if self.0 == "system" {
            "System".into()
        } else {
            self.0.clone().into()
        }
    }

    fn value(&self) -> &Self::Value {
        &self.0
    }

    fn matches(&self, query: &str) -> bool {
        self.0.to_lowercase().contains(&query.trim().to_lowercase())
    }
}

fn timezone_choices(current: &str) -> Vec<TimezoneChoice> {
    let mut names: Vec<String> = jiff::tz::db()
        .available()
        .map(|name| name.to_string())
        .collect();
    // Keep existing preferences visible even if this host's database lacks the name.
    names.push(current.to_string());
    names.retain(|name| !name.is_empty() && name != "system" && name != "UTC");
    names.sort_unstable();
    names.dedup();

    ["system", "UTC"]
        .into_iter()
        .map(String::from)
        .chain(names)
        .map(TimezoneChoice)
        .collect()
}

type TimezoneSelectState = SelectState<SearchableVec<TimezoneChoice>>;

const PREFERENCE_SECTIONS: [&str; 7] = [
    "General",
    "Themes",
    "Extensions",
    "Proxy",
    "Kubernetes",
    "Editor",
    "Terminal",
];

pub(crate) struct GlobalPreferencesState {
    pub engine: Arc<PreferencesEngine>,
    pub connection: Arc<iceon_core::connection::ConnectionEngine>,
    pub revision: u64,
    pub preferences: Preferences,
    pub error: Option<String>,
    pub main_app: WeakEntity<IceonApp>,
}

impl Global for GlobalPreferencesState {}

/// Worker completions may arrive after a newer settings action. Cache updates
/// must never move backwards, even though the underlying store is serialized.
pub(crate) fn accept_preferences_snapshot(
    revision: &mut u64,
    preferences: &mut Preferences,
    snapshot: &PreferencesSnapshot,
) -> bool {
    if snapshot.revision < *revision {
        return false;
    }
    *revision = snapshot.revision;
    *preferences = snapshot.preferences.clone();
    true
}

pub(crate) fn current_prefs(cx: &App) -> Preferences {
    cx.try_global::<GlobalPreferencesState>()
        .map(|s| s.preferences.clone())
        .unwrap_or_default()
}

/// OS hooks are applied inside the preference transaction, including rollback.
pub(crate) struct RuntimeAppIntegration;
pub(crate) struct StartupAppIntegration;
impl iceon_core::preferences::app_integration::AppIntegration for StartupAppIntegration {
    fn apply(
        &mut self,
        previous: &iceon_core::preferences::AppPreferences,
        next: &iceon_core::preferences::AppPreferences,
    ) -> Result<(), iceon_core::preferences::app_integration::IntegrationError> {
        use iceon_core::preferences::app_integration::NativeAppIntegration;
        let mut previous = previous.clone();
        let mut next = next.clone();
        previous.show_tray_icon = false;
        next.show_tray_icon = false;
        NativeAppIntegration.apply(&previous, &next)
    }
}
impl iceon_core::preferences::app_integration::AppIntegration for RuntimeAppIntegration {
    fn apply(
        &mut self,
        previous: &iceon_core::preferences::AppPreferences,
        next: &iceon_core::preferences::AppPreferences,
    ) -> Result<(), iceon_core::preferences::app_integration::IntegrationError> {
        use iceon_core::preferences::app_integration::IntegrationError;
        if previous.open_at_login == next.open_at_login
            && previous.show_tray_icon == next.show_tray_icon
        {
            return Ok(());
        }
        crate::tray::set_visible(next.show_tray_icon).map_err(IntegrationError::Tray)?;
        if previous.open_at_login != next.open_at_login
            && let Err(operation) = StartupAppIntegration.apply(previous, next)
        {
            if let Err(rollback) = crate::tray::set_visible(previous.show_tray_icon) {
                return Err(IntegrationError::Rollback {
                    operation: Box::new(operation),
                    rollback: Box::new(IntegrationError::Tray(rollback)),
                });
            }
            return Err(operation);
        }
        Ok(())
    }
}

pub(crate) fn mutate_preferences(cx: &mut App, mutate: impl FnOnce(&mut Preferences)) {
    if !cx.has_global::<GlobalPreferencesState>() {
        return;
    }

    let (maybe_theme_change, main_app, snapshot_result) = cx
        .update_global::<GlobalPreferencesState, _>(|state, _cx| {
            let prev_theme = (
                state.preferences.app.theme,
                state.preferences.app.zed_theme.clone(),
            );
            let prev_icon = state.preferences.app.zed_icon_theme.clone();
            // Global/main caches can lag behind background imports or another
            // preferences writer. Persist only this action against the latest
            // document inside the engine transaction, never retry a stale draft.
            let res = state.engine.mutate_with_app_integration(
                |next| {
                    mutate(next);
                    if next.app.timezone.trim().is_empty() {
                        next.app.timezone = "system".into();
                    }
                },
                &mut RuntimeAppIntegration,
            );
            match &res {
                Ok(snapshot) => {
                    state.revision = snapshot.revision;
                    state.preferences = snapshot.preferences.clone();
                    state.error = None;
                }
                Err(err) => {
                    state.error = Some(err.to_string());
                }
            }

            if prev_icon != state.preferences.app.zed_icon_theme {
                let active = state
                    .preferences
                    .app
                    .zed_icon_theme
                    .as_ref()
                    .and_then(|id| crate::themes_settings::find_icon_theme(id, _cx));
                crate::app_assets::set_active_icon_theme(active);
            }

            let new_theme = state.preferences.app.theme;
            let theme_changed = (prev_theme
                != (new_theme, state.preferences.app.zed_theme.clone()))
                .then_some(new_theme);
            (theme_changed, state.main_app.clone(), res.ok())
        });

    if let Some(theme) = maybe_theme_change {
        let mode = if theme.is_dark().unwrap_or(matches!(
            cx.window_appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        )) {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        };
        crate::shell_theme::apply(mode, None, cx);
    }

    if let Some(snapshot) = snapshot_result {
        let _ = main_app.update(cx, |app, cx| {
            let discovery_changed =
                app.preferences_draft.kubernetes != snapshot.preferences.kubernetes;
            if !accept_preferences_snapshot(
                &mut app.preferences_revision,
                &mut app.preferences_draft,
                &snapshot,
            ) {
                return;
            }
            if discovery_changed {
                app.refresh_catalog(cx);
            }
            app.preferences_error = None;
            cx.notify();
        });
    }

    crate::shell_theme::apply(gpui_kit::component::theme::Theme::global(cx).mode, None, cx);
    cx.refresh_windows();
}

pub struct PreferencesWindow {
    focus_handle: FocusHandle,
    timezone_select: Entity<TimezoneSelectState>,
    terminal_font_select: Entity<SelectState<SearchableVec<String>>>,
    ui_font_select: Entity<SelectState<SearchableVec<String>>>,
    editor_font_select: Entity<SelectState<SearchableVec<String>>>,
    section: usize,
    themes_settings: Entity<crate::themes_settings::ThemesSettings>,
    extensions_settings: Entity<crate::extensions_settings::ExtensionsSettings>,
    kubernetes_preferences: Entity<kubernetes_preferences::KubernetesPreferencesView>,
}

impl PreferencesWindow {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let timezone = current_prefs(cx).app.timezone;
        let timezone_select = cx.new(|cx| {
            let mut state = SelectState::new(
                SearchableVec::new(timezone_choices(&timezone)),
                None,
                window,
                cx,
            )
            .searchable(true);
            state.set_selected_value(&timezone, window, cx);
            state
        });
        cx.subscribe(
            &timezone_select,
            |_, _, event: &SelectEvent<SearchableVec<TimezoneChoice>>, cx| {
                if let SelectEvent::Confirm(Some(timezone)) = event {
                    mutate_preferences(cx, |prefs| {
                        prefs.app.timezone = timezone.clone();
                    });
                }
            },
        )
        .detach();
        let terminal_font_select =
            Self::font_select(current_prefs(cx).terminal.font_family, window, cx);
        let ui_font_select = Self::font_select(current_prefs(cx).app.font_family, window, cx);
        let editor_font_select =
            Self::font_select(current_prefs(cx).editor.font_family, window, cx);
        for (selector, target) in [
            (&terminal_font_select, 0),
            (&ui_font_select, 1),
            (&editor_font_select, 2),
        ] {
            cx.subscribe(
                selector,
                move |_, _, event: &SelectEvent<SearchableVec<String>>, cx| {
                    if let SelectEvent::Confirm(Some(font)) = event {
                        mutate_preferences(cx, |prefs| match target {
                            0 => prefs.terminal.font_family = font.clone(),
                            1 => prefs.app.font_family = font.clone(),
                            _ => prefs.editor.font_family = font.clone(),
                        });
                    }
                },
            )
            .detach();
        }
        let kubernetes_preferences =
            cx.new(|cx| kubernetes_preferences::KubernetesPreferencesView::new(window, cx));
        let extensions_settings =
            cx.new(|cx| crate::extensions_settings::ExtensionsSettings::new(window, cx));
        Self {
            focus_handle,
            timezone_select,
            terminal_font_select,
            ui_font_select,
            editor_font_select,
            section: 0,
            themes_settings: cx.new(|cx| crate::themes_settings::ThemesSettings::new(window, cx)),
            extensions_settings,
            kubernetes_preferences,
        }
    }

    fn font_select(
        font: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SelectState<SearchableVec<String>>> {
        let mut fonts = cx.text_system().all_font_names();
        fonts.push(font.clone());
        fonts.push(Preferences::default().app.font_family);
        fonts.push(Preferences::default().editor.font_family);
        fonts.sort_unstable();
        fonts.dedup();
        cx.new(|cx| {
            let mut state =
                SelectState::new(SearchableVec::new(fonts), None, window, cx).searchable(true);
            state.set_selected_value(&font, window, cx);
            state
        })
    }

    fn build_pages(&self, cx: &App) -> Vec<SettingPage> {
        let defaults = Preferences::default();
        let timezone_select = self.timezone_select.clone();
        let terminal_font_select = self.terminal_font_select.clone();
        let ui_font_select = self.ui_font_select.clone();
        let editor_font_select = self.editor_font_select.clone();

        let app_page = SettingPage::new("General")
            .icon(IconName::Settings2)
            .description("Timezone, startup, and navigation preferences.")
            .default_open(true)
            .groups([
                SettingGroup::new()
                    .items([
                        SettingItem::new("UI Font Family", SettingField::render(move |options, _, _| {
                                Select::new(&ui_font_select)
                                    .accessibility_label("UI font family")
                                    .search_placeholder("Search installed fonts...")
                                    .menu_max_h(px(320.0))
                                    .with_size(options.size())
                                    .w(px(240.0))
                                    .when(options.layout() == Axis::Vertical, |el| el.w_full())
                            }).on_reset(
                                |cx| current_prefs(cx).app.font_family != Preferences::default().app.font_family,
                                |_, cx| mutate_preferences(cx, |prefs| prefs.app.font_family = Preferences::default().app.font_family),
                            ))
                            .description("Font for navigation, tables, dialogs and application controls. Reset restores the native system font."),
                        SettingItem::new(
                            "Timezone",
                            SettingField::render(move |options, _, _| {
                                Select::new(&timezone_select)
                                    .accessibility_label("Timezone")
                                    .search_placeholder("Search timezones...")
                                    .menu_max_h(px(320.0))
                                    .with_size(options.size())
                                    .disabled(options.is_disabled())
                                    .w(px(240.0))
                                    .when(options.layout() == Axis::Vertical, |el| el.w_full())
                            })
                            .on_reset(
                                |cx| {
                                    current_prefs(cx).app.timezone
                                        != Preferences::default().app.timezone
                                 },
                                |_, cx| {
                                    mutate_preferences(cx, |prefs| {
                                        prefs.app.timezone = Preferences::default().app.timezone;
                                    });
                                },
                            ),
                        )
                        .description(
                            "Choose a timezone for timestamps, or System to use your device's timezone.",
                        ),
                        SettingItem::new(
                            "Icon Theme",
                            SettingField::dropdown(
                                {
                                    let mut options = vec![(
                                        SharedString::from("default"),
                                        SharedString::from("Default (Built-in)"),
                                    )];
                                    if let Some(catalog) =
                                        cx.try_global::<crate::themes_settings::ThemeCatalog>()
                                    {
                                        for ext in &catalog.icon_extensions {
                                            for theme in &ext.themes {
                                                options.push((
                                                    SharedString::from(theme.id.clone()),
                                                    SharedString::from(format!(
                                                        "{} ({})",
                                                        theme.name, theme.appearance
                                                    )),
                                                ));
                                            }
                                        }
                                    }
                                    options
                                },
                                |cx: &App| {
                                    current_prefs(cx)
                                        .app
                                        .zed_icon_theme
                                        .as_ref()
                                        .map(SharedString::from)
                                        .unwrap_or_else(|| SharedString::from("default"))
                                },
                                |val: SharedString, cx: &mut App| {
                                    let id = if val.as_ref() == "default" {
                                        None
                                    } else {
                                        Some(val.to_string())
                                    };
                                    set_zed_icon_theme(cx, id);
                                },
                            )
                            .default_value(SharedString::from("default"))
                            .on_reset(
                                |cx| current_prefs(cx).app.zed_icon_theme.is_some(),
                                |_, cx| set_zed_icon_theme(cx, None),
                            ),
                        )
                        .description(
                            "Choose between the default Lucide icon pack and installed Zed icon themes.",
                        ),
                    ]),
                SettingGroup::new()
                    .title("Navigation & Session")
                    .description("Control how search filters and active routes behave.")
                    .items([
                        SettingItem::new(
                            "Open at Login",
                            SettingField::switch(
                                |cx: &App| current_prefs(cx).app.open_at_login,
                                |enabled, cx| mutate_preferences(cx, |prefs| prefs.app.open_at_login = enabled),
                            ).default_value(false),
                        ).disabled(!cfg!(target_os = "macos"))
                            .description("Start Iceon when you sign in to macOS. Changes update Iceon's login registration."),
                        SettingItem::new(
                            "Show Tray Icon",
                            SettingField::switch(
                                |cx: &App| current_prefs(cx).app.show_tray_icon,
                                |enabled, cx| mutate_preferences(cx, |prefs| prefs.app.show_tray_icon = enabled),
                            ),
                        ).disabled(!crate::tray::supported()).description("Show a macOS menu-bar icon. Click it to bring Iceon forward."),
                        SettingItem::new(
                            "Reset Sidebar Order",
                            SettingField::render(|_, _, _| {
                                Button::new("reset-sidebar-order")
                                    .small()
                                    .icon(IconName::Undo2)
                                    .accessibility_label("Reset cluster sidebar order")
                                    .label("Reset").on_click(|_, _, cx| {
                                    let main = cx.try_global::<GlobalPreferencesState>().map(|state| state.main_app.clone());
                                    if let Some(main) = main {
                                        let _ = main.update(cx, |app, cx| {
                                            app.cluster_order = app.catalog_snapshot.clusters.iter().map(|cluster| cluster.id.clone()).collect();
                                            cx.notify();
                                        });
                                    }
                                })
                            }),
                        ).description("Restore the cluster icon rail to catalog order."),
                        SettingItem::new(
                            "Persistent Search",
                            SettingField::switch(
                                |cx: &App| current_prefs(cx).app.persistent_search,
                                |checked: bool, cx: &mut App| {
                                    mutate_preferences(cx, |prefs| {
                                        prefs.app.persistent_search = checked;
                                    });
                                },
                            )
                            .default_value(defaults.app.persistent_search),
                        )
                        .description(
                            "Keep search queries active when switching between resource kinds.",
                        ),
                        SettingItem::new(
                            "Restore Last Page on Startup",
                            SettingField::switch(
                                |cx: &App| current_prefs(cx).app.restore_last_page,
                                |checked: bool, cx: &mut App| {
                                    mutate_preferences(cx, |prefs| {
                                        prefs.app.restore_last_page = checked;
                                    });
                                },
                            )
                            .default_value(defaults.app.restore_last_page),
                        )
                        .description(
                            "Reopen the last visited cluster or workspace when launching Iceon.",
                        ),
                    ]),
            ]);

        let proxy_page = SettingPage::new("Proxy")
            .icon(IconName::Globe)
            .description("Configure outbound proxy routing and certificate settings.")
            .groups([SettingGroup::new()
                .title("Upstream Proxy")
                .description("For kubectl downloads and Helm chart/repository requests; cluster API TLS policy is unchanged.")
                .items([
                    SettingItem::new(
                        "HTTPS Proxy URL",
                        SettingField::input(
                            |cx: &App| SharedString::from(current_prefs(cx).proxy.https_proxy),
                            |val: SharedString, cx: &mut App| {
                                mutate_preferences(cx, |prefs| {
                                    prefs.proxy.https_proxy = val.trim().to_string();
                                });
                            },
                        )
                        .default_value(SharedString::from(defaults.proxy.https_proxy)),
                    )
                    .description("HTTP(S) proxy for application downloads and Helm. Does not route cluster API traffic."),
                    SettingItem::new(
                        "Allow Untrusted Certificate Authorities",
                        SettingField::switch(
                            |cx: &App| current_prefs(cx).proxy.allow_untrusted_cas,
                            |checked: bool, cx: &mut App| {
                                mutate_preferences(cx, |prefs| {
                                    prefs.proxy.allow_untrusted_cas = checked;
                                });
                            },
                        )
                        .default_value(defaults.proxy.allow_untrusted_cas),
                    )
                    .description("Skip certificate validation for application downloads and Helm repositories only. Kubernetes API certificate verification is unchanged."),
                ])]);

        let kubernetes_page = SettingPage::new("Kubernetes")
            .description("Kubernetes tools, Helm repositories, and kubeconfig synchronization.")
            .resettable(false)
            .groups(kubernetes_preferences::groups(
                &self.kubernetes_preferences,
                cx,
            ));

        let editor_page = SettingPage::new("Editor")
            .icon(IconName::File)
            .description("Customize YAML manifest editor font, indentation, and minimap.")
            .groups([
                SettingGroup::new()
                    .title("Typography & Indentation")
                    .items([
                        SettingItem::new(
                            "Font Family",
                            SettingField::render(move |options, _, _| {
                                Select::new(&editor_font_select)
                                    .accessibility_label("Editor font family")
                                    .search_placeholder("Search installed fonts...")
                                    .menu_max_h(px(320.0))
                                    .with_size(options.size())
                                    .w(px(240.0))
                                    .when(options.layout() == Axis::Vertical, |el| el.w_full())
                            })
                            .on_reset(
                                |cx| {
                                    current_prefs(cx).editor.font_family
                                        != Preferences::default().editor.font_family
                                },
                                |_, cx| {
                                    mutate_preferences(cx, |prefs| {
                                        prefs.editor.font_family =
                                            Preferences::default().editor.font_family
                                    })
                                },
                            ),
                        )
                        .description(
                            "Monospace font family used in YAML editors and resource diffs.",
                        ),
                        SettingItem::new(
                            "Font Size",
                            SettingField::number_input(
                                NumberFieldOptions {
                                    min: 10.0,
                                    max: 50.0,
                                    step: 1.0,
                                },
                                |cx: &App| current_prefs(cx).editor.font_size as f64,
                                |val: f64, cx: &mut App| {
                                    mutate_preferences(cx, |prefs| {
                                        prefs.editor.font_size =
                                            val.round().clamp(10.0, 50.0) as u8;
                                    });
                                },
                            )
                            .default_value(defaults.editor.font_size as f64),
                        )
                        .description("Editor font size in pixels (10 - 50)."),
                        SettingItem::new(
                            "Tab Size",
                            SettingField::number_input(
                                NumberFieldOptions {
                                    min: 1.0,
                                    max: 16.0,
                                    step: 1.0,
                                },
                                |cx: &App| current_prefs(cx).editor.tab_size as f64,
                                |val: f64, cx: &mut App| {
                                    mutate_preferences(cx, |prefs| {
                                        prefs.editor.tab_size = val.round().clamp(1.0, 16.0) as u8;
                                    });
                                },
                            )
                            .default_value(defaults.editor.tab_size as f64),
                        )
                        .description("Spaces per indentation level in YAML manifests (1 - 16)."),
                    ]),
                SettingGroup::new().title("Gutter & Minimap").items([
                    SettingItem::new(
                        "Line Numbers",
                        SettingField::dropdown(
                            vec![
                                (SharedString::from("on"), SharedString::from("On")),
                                (SharedString::from("off"), SharedString::from("Off")),
                                (
                                    SharedString::from("relative"),
                                    SharedString::from("Relative"),
                                ),
                                (
                                    SharedString::from("interval"),
                                    SharedString::from("Interval"),
                                ),
                            ],
                            |cx: &App| match current_prefs(cx).editor.line_numbers {
                                LineNumbers::On => SharedString::from("on"),
                                LineNumbers::Off => SharedString::from("off"),
                                LineNumbers::Relative => SharedString::from("relative"),
                                LineNumbers::Interval => SharedString::from("interval"),
                            },
                            |val: SharedString, cx: &mut App| {
                                mutate_preferences(cx, |prefs| {
                                    prefs.editor.line_numbers = match val.as_ref() {
                                        "off" => LineNumbers::Off,
                                        "relative" => LineNumbers::Relative,
                                        "interval" => LineNumbers::Interval,
                                        _ => LineNumbers::On,
                                    };
                                });
                            },
                        )
                        .default_value(SharedString::from("on")),
                    )
                    .description("Display line numbers in the YAML editor gutter."),
                    SettingItem::new(
                        "Show Minimap",
                        SettingField::switch(
                            |cx: &App| current_prefs(cx).editor.minimap,
                            |checked: bool, cx: &mut App| {
                                mutate_preferences(cx, |prefs| {
                                    prefs.editor.minimap = checked;
                                });
                            },
                        )
                        .default_value(defaults.editor.minimap),
                    )
                    .description("Render a document overview minimap beside the YAML editor."),
                    SettingItem::new(
                        "Minimap Side",
                        SettingField::dropdown(
                            vec![
                                (SharedString::from("right"), SharedString::from("Right")),
                                (SharedString::from("left"), SharedString::from("Left")),
                            ],
                            |cx: &App| match current_prefs(cx).editor.minimap_side {
                                MinimapSide::Left => SharedString::from("left"),
                                MinimapSide::Right => SharedString::from("right"),
                            },
                            |val: SharedString, cx: &mut App| {
                                mutate_preferences(cx, |prefs| {
                                    prefs.editor.minimap_side = match val.as_ref() {
                                        "left" => MinimapSide::Left,
                                        _ => MinimapSide::Right,
                                    };
                                });
                            },
                        )
                        .default_value(SharedString::from("right")),
                    )
                    .description("Choose which side of the editor displays the minimap."),
                ]),
            ]);

        let terminal_page = SettingPage::new("Terminal")
            .icon(IconName::SquareTerminal)
            .description("Configure Pod terminal typography and clipboard behavior.")
            .groups([
                SettingGroup::new()
                    .title("Typography & Default Shell")
                    .items([
                        SettingItem::new(
                            "Terminal Font Family",
                            SettingField::render(move |options, _, _| {
                                Select::new(&terminal_font_select)
                                    .accessibility_label("Terminal font family")
                                    .search_placeholder("Search installed fonts...")
                                    .menu_max_h(px(320.0))
                                    .with_size(options.size())
                                    .w(px(240.0))
                                    .when(options.layout() == Axis::Vertical, |el| el.w_full())
                            })
                            .on_reset(
                                |cx| {
                                    current_prefs(cx).terminal.font_family
                                        != Preferences::default().terminal.font_family
                                },
                                |_, cx| {
                                    mutate_preferences(cx, |prefs| {
                                        prefs.terminal.font_family =
                                            Preferences::default().terminal.font_family
                                    })
                                },
                            ),
                        )
                        .description(
                            "Monospace font family used in Pod logs and interactive sessions.",
                        ),
                        SettingItem::new(
                            "Font Size",
                            SettingField::number_input(
                                NumberFieldOptions {
                                    min: 10.0,
                                    max: 50.0,
                                    step: 1.0,
                                },
                                |cx: &App| current_prefs(cx).terminal.font_size as f64,
                                |val: f64, cx: &mut App| {
                                    mutate_preferences(cx, |prefs| {
                                        prefs.terminal.font_size =
                                            val.round().clamp(10.0, 50.0) as u8;
                                    });
                                },
                            )
                            .default_value(defaults.terminal.font_size as f64),
                        )
                        .description("Terminal and log viewer font size in pixels (10 - 50)."),
                        SettingItem::new(
                            "Default Shell",
                            SettingField::input(
                                |cx: &App| {
                                    SharedString::from(current_prefs(cx).terminal.shell_path)
                                },
                                |val: SharedString, cx: &mut App| {
                                    mutate_preferences(cx, |prefs| {
                                        let trimmed = val.trim();
                                        prefs.terminal.shell_path = trimmed.to_string();
                                    });
                                },
                            )
                            .default_value(SharedString::from(defaults.terminal.shell_path)),
                        )
                        .description("Shell executable inside the container for new Pod sessions; it must exist in the selected container. Empty uses /bin/sh."),
                    ]),
                SettingGroup::new().title("Appearance & Clipboard").items([
                    SettingItem::new(
                        "Terminal Theme",
                        SettingField::dropdown(
                            vec![
                                (
                                    SharedString::from("match"),
                                    SharedString::from("Match App Theme"),
                                ),
                                (SharedString::from("dark"), SharedString::from("Dark")),
                                (SharedString::from("light"), SharedString::from("Light")),
                            ],
                            |cx: &App| match current_prefs(cx).terminal.theme {
                                TerminalTheme::Match => SharedString::from("match"),
                                TerminalTheme::Dark => SharedString::from("dark"),
                                TerminalTheme::Light => SharedString::from("light"),
                            },
                            |val: SharedString, cx: &mut App| {
                                mutate_preferences(cx, |prefs| {
                                    prefs.terminal.theme = match val.as_ref() {
                                        "dark" => TerminalTheme::Dark,
                                        "light" => TerminalTheme::Light,
                                        _ => TerminalTheme::Match,
                                    };
                                });
                            },
                        )
                        .default_value(SharedString::from("match")),
                    )
                    .description("Color scheme for the bottom Pod dock terminal and log stream."),
                    SettingItem::new(
                        "Copy on Select",
                        SettingField::switch(
                            |cx: &App| current_prefs(cx).terminal.copy_on_select,
                            |checked: bool, cx: &mut App| {
                                mutate_preferences(cx, |prefs| {
                                    prefs.terminal.copy_on_select = checked;
                                });
                            },
                        )
                        .default_value(defaults.terminal.copy_on_select),
                    )
                    .description("Automatically copy selected terminal text to the clipboard."),
                ]),
            ]);

        vec![
            app_page,
            SettingPage::new("Themes"),
            proxy_page,
            kubernetes_page,
            editor_page,
            terminal_page,
        ]
    }
}

impl Render for PreferencesWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let timezone = current_prefs(cx).app.timezone;
        if self.timezone_select.read(cx).selected_value() != Some(&timezone) {
            self.timezone_select.update(cx, |state, cx| {
                state.set_items(SearchableVec::new(timezone_choices(&timezone)), window, cx);
                state.set_selected_value(&timezone, window, cx);
            });
        }
        let prefs = current_prefs(cx);
        for (selector, font) in [
            (&self.terminal_font_select, prefs.terminal.font_family),
            (&self.ui_font_select, prefs.app.font_family),
            (&self.editor_font_select, prefs.editor.font_family),
        ] {
            if selector.read(cx).selected_value() != Some(&font) {
                selector.update(cx, |state, cx| state.set_selected_value(&font, window, cx));
            }
        }
        let error = cx
            .try_global::<GlobalPreferencesState>()
            .and_then(|s| s.error.clone());

        v_flex()
            .track_focus(&self.focus_handle)
            .size_full()
            .text_color(cx.theme().foreground)
            .child(
                TitleBar::new().bg(cx.theme().tokens.title_bar).child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .justify_between()
                        .pr_3()
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(div().text_sm().font_semibold().child("Settings"))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("Preferences"),
                                ),
                        ),
                ),
            )
            .when_some(error, |el, msg| {
                el.child(
                    div()
                        .mx_4()
                        .mt_2()
                        .p_2p5()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(cx.theme().danger)
                        .bg(cx.theme().background)
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(msg),
                )
            })
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .bg(cx.theme().tokens.sidebar)
                    .overflow_hidden()
                    .child(
                        v_flex()
                            .w(px(150.0))
                            .h_full()
                            .flex_shrink_0()
                            .p_3()
                            .gap_1()
                            .border_r_1()
                            .border_color(cx.theme().border)
                            .children(PREFERENCE_SECTIONS.into_iter().enumerate().map(
                                |(ix, name)| {
                                    Button::new(("preferences-section", ix))
                                        .small()
                                        .child(
                                            h_flex()
                                                .w_full()
                                                .gap_2()
                                                .items_center()
                                                .child(gpui_kit::component::Icon::new(match ix {
                                                    0 => AssetIconName::Settings2,
                                                    1 => AssetIconName::Palette,
                                                    2 => AssetIconName::Puzzle,
                                                    3 => AssetIconName::Network,
                                                    4 => AssetIconName::LayoutDashboard,
                                                    5 => AssetIconName::FileText,
                                                    _ => AssetIconName::Square,
                                                }))
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .min_w_0()
                                                        .text_left()
                                                        .child(name),
                                                ),
                                        )
                                        .accessibility_label(name)
                                        .ghost()
                                        .w_full()
                                        .justify_start()
                                        .when(self.section == ix, |button| {
                                            button.bg(cx.theme().secondary)
                                        })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.section = ix;
                                            cx.notify();
                                        }))
                                },
                            )),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .child(if self.section == 1 {
                                self.themes_settings.clone().into_any_element()
                            } else if self.section == 2 {
                                self.extensions_settings.clone().into_any_element()
                            } else {
                                let page_idx = if self.section > 2 {
                                    self.section - 1
                                } else {
                                    self.section
                                };
                                Settings::new(("iceon-settings-page", self.section))
                                    .small()
                                    .sidebar_width(px(0.0))
                                    .sidebar_size_range(px(0.0)..px(0.0))
                                    .sidebar_style(
                                        &StyleRefinement::default()
                                            .w(px(0.0))
                                            .min_w(px(0.0))
                                            .max_w(px(0.0))
                                            .overflow_hidden(),
                                    )
                                    .with_group_variant(GroupBoxVariant::Normal)
                                    .page(self.build_pages(cx).remove(page_idx))
                                    .into_any_element()
                            }),
                    ),
            )
    }
}

impl IceonApp {
    pub(crate) fn open_kubernetes_preferences(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_preferences(window, cx);
        if let Some(handle) = self.preferences_window {
            let _ = handle.update(cx, |_, window, cx| {
                if let Some(root) = window.root::<gpui_kit::component::Root>().flatten() {
                    let view = root.read(cx).view().clone();
                    if let Ok(settings) = view.downcast::<PreferencesWindow>() {
                        settings.update(cx, |settings, cx| {
                            settings.section = 4;
                            cx.notify();
                        });
                    }
                }
            });
        }
    }

    pub(crate) fn open_theme_management(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_preferences_internal(
            1,
            Some(crate::themes_settings::ThemesTab::ColorThemes),
            window,
            cx,
        );
    }

    pub(crate) fn open_icon_theme_management(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_preferences_internal(
            1,
            Some(crate::themes_settings::ThemesTab::IconThemes),
            window,
            cx,
        );
    }

    pub(crate) fn open_extensions_preferences(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_preferences_internal(2, None, window, cx);
    }

    pub(crate) fn open_preferences(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_preferences_internal(0, None, window, cx);
    }

    pub(crate) fn open_preferences_internal(
        &mut self,
        section: usize,
        theme_tab: Option<crate::themes_settings::ThemesTab>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Ok(snapshot) = self.preferences_engine.snapshot() {
            self.preferences_revision = snapshot.revision;
            self.preferences_draft = snapshot.preferences.clone();
        }
        self.preferences_error = None;

        cx.set_global(GlobalPreferencesState {
            engine: self.preferences_engine.clone(),
            connection: self.connection.clone(),
            revision: self.preferences_revision,
            preferences: self.preferences_draft.clone(),
            error: None,
            main_app: cx.entity().downgrade(),
        });

        if let Some(existing) = self.preferences_window {
            let activated = existing
                .update(cx, |_, win, cx| {
                    win.activate_window();
                    if let Some(root) = win.root::<gpui_kit::component::Root>().flatten() {
                        let view = root.read(cx).view().clone();
                        if let Ok(settings) = view.downcast::<PreferencesWindow>() {
                            settings.update(cx, |settings, cx| {
                                settings.section = section;
                                if let Some(tab) = theme_tab {
                                    settings.themes_settings.update(cx, |themes, cx| {
                                        themes.set_tab(tab, cx);
                                    });
                                }
                                cx.notify();
                            });
                        }
                    }
                })
                .is_ok();
            if activated {
                return;
            }
            self.preferences_window = None;
        }

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(880.0), px(620.0)),
                cx,
            ))),
            window_min_size: Some(size(px(680.0), px(480.0))),
            window_background: crate::shell_theme::background_appearance(cx),
            ..TitleBar::window_options()
        };

        let app_handle = cx.entity().downgrade();
        cx.defer(move |cx| {
            if let Ok((handle, _preferences)) =
                crate::shell_theme::open_window(options, cx, |window, cx| {
                    cx.new(|cx| {
                        let mut win = PreferencesWindow::new(window, cx);
                        win.section = section;
                        if let Some(tab) = theme_tab {
                            win.themes_settings.update(cx, |themes, cx| {
                                themes.set_tab(tab, cx);
                            });
                        }
                        win
                    })
                })
            {
                let _ = app_handle.update(cx, |this, _| {
                    this.preferences_window = Some(handle);
                });
            }
        });
        cx.notify();
    }
}
