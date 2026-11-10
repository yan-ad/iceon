//! Native OS window for Settings / Preferences using `gpui_kit::component::setting::Settings`,
//! backed by `PreferencesEngine` with live GPUI theme updates across all open windows.

use std::sync::Arc;

use gpui_kit::component::{
    ActiveTheme as _, IconName, StyledExt as _, TitleBar,
    group_box::GroupBoxVariant,
    h_flex,
    setting::{NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings},
    theme::ThemeMode,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::IceonApp;
use crate::navigation::preferences_error_message;
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

            let res = state.engine.save(SavePreferencesRequest {
                revision: state.revision,
                preferences: next,
            });
            match &res {
                Ok(snapshot) => {
                    state.revision = snapshot.revision;
                    state.preferences = snapshot.preferences.clone();
                    state.error = None;
                }
                Err(err) => {
                    state.error = Some(preferences_error_message(err.clone()).to_string());
                }
            }

            let new_theme = state.preferences.app.theme;
            let theme_changed = (prev_theme != new_theme).then_some(new_theme);
            (theme_changed, state.main_app.clone(), res.ok())
        });

    if let Some(theme) = maybe_theme_change {
        let mode = match theme {
            AppTheme::Light => ThemeMode::Light,
            AppTheme::Dark | AppTheme::System => ThemeMode::Dark,
        };
        crate::shell_theme::apply(mode, None, cx);
    }

    if let Some(snapshot) = snapshot_result {
        let _ = main_app.update(cx, |app, cx| {
            app.preferences_revision = snapshot.revision;
            app.preferences_draft = snapshot.preferences;
            app.preferences_error = None;
            cx.notify();
        });
    }

    cx.refresh_windows();
}

pub struct PreferencesWindow {
    focus_handle: FocusHandle,
}

impl PreferencesWindow {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        Self { focus_handle }
    }

    fn build_pages() -> Vec<SettingPage> {
        let defaults = Preferences::default();

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
                            .default_value(SharedString::from("dark")),
                        )
                        .description("Select the application color scheme across all windows."),
                        SettingItem::new(
                            "Timezone",
                            SettingField::input(
                                |cx: &App| SharedString::from(current_prefs(cx).app.timezone),
                                |val: SharedString, cx: &mut App| {
                                    mutate_preferences(cx, |prefs| {
                                        let trimmed = val.trim();
                                        prefs.app.timezone = if trimmed.is_empty() {
                                            "system".to_string()
                                        } else {
                                            trimmed.to_string()
                                        };
                                    });
                                },
                            )
                            .default_value(SharedString::from(defaults.app.timezone)),
                        )
                        .description(
                            "IANA timezone identifier (such as UTC or Asia/Jakarta) or system.",
                        ),
                    ]),
                SettingGroup::new()
                    .title("Navigation & Session")
                    .description("Control how search filters and active routes behave.")
                    .items([
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
                .description("Used when connecting to remote Kubernetes API servers.")
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
                    .description("Optional upstream proxy address for cluster connections."),
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
                    .description("Allow custom certificate authorities recorded in preferences."),
                ])]);

        let kubernetes_page = SettingPage::new("Kubernetes")
            .icon(IconName::LayoutDashboard)
            .description("Configure local kubeconfig discovery and Helm integration.")
            .groups([SettingGroup::new()
                .title("Kubeconfig & Helm")
                .description("Manage how Iceon discovers clusters and runs Helm operations.")
                .items([
                    SettingItem::new(
                        "Auto-discover Default Kubeconfig",
                        SettingField::switch(
                            |cx: &App| {
                                current_prefs(cx).kubernetes.discover_default_kubeconfig
                            },
                            |checked: bool, cx: &mut App| {
                                mutate_preferences(cx, |prefs| {
                                    prefs.kubernetes.discover_default_kubeconfig = checked;
                                });
                            },
                        )
                        .default_value(defaults.kubernetes.discover_default_kubeconfig),
                    )
                    .description("Automatically scan and import contexts from default kubeconfig in Catalog."),
                    SettingItem::new(
                        "Custom Helm Binary Path",
                        SettingField::input(
                            |cx: &App| {
                                SharedString::from(current_prefs(cx).kubernetes.helm_binary_path)
                            },
                            |val: SharedString, cx: &mut App| {
                                mutate_preferences(cx, |prefs| {
                                    prefs.kubernetes.helm_binary_path = val.trim().to_string();
                                });
                            },
                        )
                        .default_value(SharedString::from(defaults.kubernetes.helm_binary_path)),
                    )
                    .description("Optional custom path to the helm binary."),
                    SettingItem::new(
                        "Helm Server-Side Dry Run",
                        SettingField::switch(
                            |cx: &App| current_prefs(cx).kubernetes.helm_server_side,
                            |checked: bool, cx: &mut App| {
                                mutate_preferences(cx, |prefs| {
                                    prefs.kubernetes.helm_server_side = checked;
                                });
                            },
                        )
                        .default_value(defaults.kubernetes.helm_server_side),
                    )
                    .description("Use server-side validation for Helm chart operations."),
                ])]);

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
                            ],
                            |cx: &App| match current_prefs(cx).editor.line_numbers {
                                LineNumbers::On => SharedString::from("on"),
                                LineNumbers::Off => SharedString::from("off"),
                                LineNumbers::Relative => SharedString::from("relative"),
                            },
                            |val: SharedString, cx: &mut App| {
                                mutate_preferences(cx, |prefs| {
                                    prefs.editor.line_numbers = match val.as_ref() {
                                        "off" => LineNumbers::Off,
                                        "relative" => LineNumbers::Relative,
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
                            SettingField::input(
                                |cx: &App| {
                                    SharedString::from(current_prefs(cx).terminal.font_family)
                                },
                                |val: SharedString, cx: &mut App| {
                                    mutate_preferences(cx, |prefs| {
                                        let trimmed = val.trim();
                                        if !trimmed.is_empty() {
                                            prefs.terminal.font_family = trimmed.to_string();
                                        }
                                    });
                                },
                            )
                            .default_value(SharedString::from(defaults.terminal.font_family)),
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
                                        if !trimmed.is_empty() {
                                            prefs.terminal.shell_path = trimmed.to_string();
                                        }
                                    });
                                },
                            )
                            .default_value(SharedString::from(defaults.terminal.shell_path)),
                        )
                        .description("Default shell binary used for Pod sessions."),
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .bg(cx.theme().background)
                    .overflow_hidden()
                    .child(
                        Settings::new("iceon-settings")
                            .sidebar_width(px(210.0))
                            .with_group_variant(GroupBoxVariant::Outline)
                            .pages(Self::build_pages()),
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

        if let Ok(handle) = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| PreferencesWindow::new(window, cx))
        }) {
            self.preferences_window = Some(handle.into());
        }
        cx.notify();
    }

    pub(crate) fn apply_theme(
        &self,
        theme: AppTheme,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        let mode = match theme {
            AppTheme::Light => ThemeMode::Light,
            AppTheme::Dark | AppTheme::System => ThemeMode::Dark,
        };
        crate::shell_theme::apply(mode, window, cx);
    }
}
