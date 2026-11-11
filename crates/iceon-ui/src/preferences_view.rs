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

use crate::IceonApp;
#[path = "kubernetes_preferences.rs"]
mod kubernetes_preferences;
use iceon_core::preferences::{
    AppTheme, LineNumbers, MinimapSide, Preferences, PreferencesEngine, SavePreferencesRequest,
    TerminalTheme,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreferencesSection {
    App,
    Proxy,
    Kubernetes,
    Editor,
    Terminal,
}

pub(crate) fn set_theme(cx: &mut App, theme: AppTheme) {
    mutate_preferences(cx, |preferences| preferences.app.theme = theme);
}

#[cfg(test)]
mod tests {
    use super::{PREFERENCE_SECTIONS, SelectItem as _, TimezoneChoice, timezone_choices};

    #[test]
    fn preferences_navigation_has_only_the_requested_sections_in_order() {
        assert_eq!(
            PREFERENCE_SECTIONS,
            ["App", "Proxy", "Kubernetes", "Editor", "Terminal"]
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

const PREFERENCE_SECTIONS: [&str; 5] = ["App", "Proxy", "Kubernetes", "Editor", "Terminal"];

pub(crate) struct GlobalPreferencesState {
    pub engine: Arc<PreferencesEngine>,
    pub revision: u64,
    pub preferences: Preferences,
    pub error: Option<String>,
    pub main_app: WeakEntity<IceonApp>,
}

impl Global for GlobalPreferencesState {}

fn current_prefs(cx: &App) -> Preferences {
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

fn mutate_preferences(cx: &mut App, mutate: impl FnOnce(&mut Preferences)) {
    if !cx.has_global::<GlobalPreferencesState>() {
        return;
    }

    let (maybe_theme_change, main_app, snapshot_result) = cx
        .update_global::<GlobalPreferencesState, _>(|state, _cx| {
            let prev_theme = state.preferences.app.theme;
            let mut next = state.preferences.clone();
            mutate(&mut next);
            if next.app.timezone.trim().is_empty() {
                next.app.timezone = "system".into();
            }

            let res = state.engine.save_with_app_integration(
                SavePreferencesRequest {
                    revision: state.revision,
                    preferences: next,
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

            let new_theme = state.preferences.app.theme;
            let theme_changed = (prev_theme != new_theme).then_some(new_theme);
            (theme_changed, state.main_app.clone(), res.ok())
        });

    if let Some(theme) = maybe_theme_change {
        let mode = match theme {
            AppTheme::Light => ThemeMode::Light,
            AppTheme::Dark => ThemeMode::Dark,
            AppTheme::System => ThemeMode::from(cx.window_appearance()),
        };
        crate::shell_theme::apply(mode, None, cx);
    }

    if let Some(snapshot) = snapshot_result {
        let _ = main_app.update(cx, |app, cx| {
            app.preferences_revision = snapshot.revision;
            let discovery_changed =
                app.preferences_draft.kubernetes != snapshot.preferences.kubernetes;
            app.preferences_draft = snapshot.preferences;
            if discovery_changed {
                app.refresh_catalog(cx);
            }
            app.preferences_error = None;
            cx.notify();
        });
    }

    cx.refresh_windows();
}

pub struct PreferencesWindow {
    focus_handle: FocusHandle,
    timezone_select: Entity<TimezoneSelectState>,
    terminal_font_select: Entity<SelectState<SearchableVec<String>>>,
    section: usize,
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
        let terminal_font = current_prefs(cx).terminal.font_family;
        let mut fonts = cx.text_system().all_font_names();
        fonts.push(terminal_font.clone());
        fonts.sort_unstable();
        fonts.dedup();
        let terminal_font_select = cx.new(|cx| {
            let mut state =
                SelectState::new(SearchableVec::new(fonts), None, window, cx).searchable(true);
            state.set_selected_value(&terminal_font, window, cx);
            state
        });
        cx.subscribe(
            &terminal_font_select,
            |_, _, event: &SelectEvent<SearchableVec<String>>, cx| {
                if let SelectEvent::Confirm(Some(font)) = event {
                    mutate_preferences(cx, |prefs| prefs.terminal.font_family = font.clone());
                }
            },
        )
        .detach();
        let kubernetes_preferences =
            cx.new(|cx| kubernetes_preferences::KubernetesPreferencesView::new(window, cx));
        Self {
            focus_handle,
            timezone_select,
            terminal_font_select,
            section: 0,
            kubernetes_preferences,
        }
    }

    fn build_pages(&self, cx: &App) -> Vec<SettingPage> {
        let defaults = Preferences::default();
        let timezone_select = self.timezone_select.clone();
        let terminal_font_select = self.terminal_font_select.clone();

        let app_page = SettingPage::new("App")
            .icon(IconName::Settings2)
            .description("Appearance, timezone, and startup navigation preferences.")
            .default_open(true)
            .groups([
                SettingGroup::new()
                    .title("Appearance & Regional")
                    .description("Customize Iceon color theme and timestamp formatting.")
                    .items([
                        SettingItem::new(
                            "Theme",
                            SettingField::dropdown(
                                vec![
                                    (SharedString::from("dark"), SharedString::from("Dark")),
                                    (SharedString::from("light"), SharedString::from("Light")),
                                    (SharedString::from("system"), SharedString::from("System")),
                                ],
                                |cx: &App| match current_prefs(cx).app.theme {
                                    AppTheme::Dark => SharedString::from("dark"),
                                    AppTheme::Light => SharedString::from("light"),
                                    AppTheme::System => SharedString::from("system"),
                                },
                                |val: SharedString, cx: &mut App| {
                                    mutate_preferences(cx, |prefs| {
                                        prefs.app.theme = match val.as_ref() {
                                            "light" => AppTheme::Light,
                                            "system" => AppTheme::System,
                                            _ => AppTheme::Dark,
                                        };
                                    });
                                },
                            )
                            .default_value(SharedString::from("system")),
                        )
                        .description("Select the application color scheme across all windows."),
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
                            "Auto-hide Hotbar",
                            SettingField::switch(
                                |cx: &App| current_prefs(cx).app.auto_hide_hotbar,
                                |enabled, cx| mutate_preferences(cx, |prefs| prefs.app.auto_hide_hotbar = enabled),
                            ).default_value(false),
                        ).description("Hide the cluster icon rail until you hover over the left edge."),
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
                                Button::new("reset-sidebar-order").label("Reset").on_click(|_, _, cx| {
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
                            SettingField::input(
                                |cx: &App| SharedString::from(current_prefs(cx).editor.font_family),
                                |val: SharedString, cx: &mut App| {
                                    mutate_preferences(cx, |prefs| {
                                        let trimmed = val.trim();
                                        if !trimmed.is_empty() {
                                            prefs.editor.font_family = trimmed.to_string();
                                        }
                                    });
                                },
                            )
                            .default_value(SharedString::from(defaults.editor.font_family)),
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
        let font = current_prefs(cx).terminal.font_family;
        if self.terminal_font_select.read(cx).selected_value() != Some(&font) {
            self.terminal_font_select
                .update(cx, |state, cx| state.set_selected_value(&font, window, cx));
        }
        let error = cx
            .try_global::<GlobalPreferencesState>()
            .and_then(|s| s.error.clone());

        v_flex()
            .track_focus(&self.focus_handle)
            .size_full()
            .text_color(cx.theme().foreground)
            .child(
                TitleBar::new().child(
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
                    .bg(cx.theme().background)
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
                                        .label(name)
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
                        div().flex_1().min_w_0().h_full().child(
                            Settings::new(("iceon-settings-page", self.section))
                                .sidebar_width(px(0.0))
                                .sidebar_size_range(px(0.0)..px(0.0))
                                .sidebar_style(&StyleRefinement {
                                    display: Some(Display::None),
                                    ..Default::default()
                                })
                                .with_group_variant(GroupBoxVariant::Normal)
                                .page(self.build_pages(cx).remove(self.section)),
                        ),
                    ),
            )
    }
}

impl IceonApp {
    pub(crate) fn open_preferences(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Ok(snapshot) = self.preferences_engine.snapshot() {
            self.preferences_revision = snapshot.revision;
            self.preferences_draft = snapshot.preferences.clone();
        }
        self.preferences_error = None;

        cx.set_global(GlobalPreferencesState {
            engine: self.preferences_engine.clone(),
            revision: self.preferences_revision,
            preferences: self.preferences_draft.clone(),
            error: None,
            main_app: cx.entity().downgrade(),
        });

        if let Some(existing) = self.preferences_window {
            let activated = existing
                .update(cx, |_, win, _| {
                    win.activate_window();
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
            window_background: crate::shell_theme::background_appearance(),
            ..TitleBar::window_options()
        };

        if let Ok((handle, _preferences)) = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| PreferencesWindow::new(window, cx))
        }) {
            self.preferences_window = Some(handle);
        }
        cx.notify();
    }
}
