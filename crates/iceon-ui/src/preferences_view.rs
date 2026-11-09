//! Preferences view: 5-section modal/panel (App, Proxy, Kubernetes, Editor, Terminal)
//! backed by `PreferencesEngine` with live GPUI theme updates via `Theme::change`.

use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    scroll::ScrollableElement as _,
    switch::Switch,
    tab::{Tab, TabBar},
    theme::{Theme, ThemeMode},
    v_flex, IconName,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::IceonApp;
use crate::navigation::preferences_error_message;
use iceon_core::preferences::{
    AppTheme, LineNumbers, MinimapSide, Preferences, SavePreferencesRequest, TerminalTheme,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreferencesSection {
    App,
    Proxy,
    Kubernetes,
    Editor,
    Terminal,
}

impl IceonApp {
    pub(crate) fn open_preferences(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Ok(snapshot) = self.preferences_engine.snapshot() {
            self.preferences_revision = snapshot.revision;
            self.preferences_draft = snapshot.preferences.clone();
            self.sync_preferences_inputs(window, cx);
        }
        self.preferences_error = None;
        self.preferences_open = true;
        cx.notify();
    }

    pub(crate) fn sync_preferences_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.preferences_draft.clone();
        self.pref_timezone_input
            .update(cx, |s, cx| s.set_value(draft.app.timezone.clone(), window, cx));
        self.pref_proxy_input
            .update(cx, |s, cx| s.set_value(draft.proxy.https_proxy.clone(), window, cx));
        self.pref_helm_path_input.update(cx, |s, cx| {
            s.set_value(draft.kubernetes.helm_binary_path.clone(), window, cx)
        });
        self.pref_editor_font_family_input.update(cx, |s, cx| {
            s.set_value(draft.editor.font_family.clone(), window, cx)
        });
        self.pref_editor_font_size_input.update(cx, |s, cx| {
            s.set_value(draft.editor.font_size.to_string(), window, cx)
        });
        self.pref_editor_tab_size_input.update(cx, |s, cx| {
            s.set_value(draft.editor.tab_size.to_string(), window, cx)
        });
        self.pref_terminal_font_family_input.update(cx, |s, cx| {
            s.set_value(draft.terminal.font_family.clone(), window, cx)
        });
        self.pref_terminal_font_size_input.update(cx, |s, cx| {
            s.set_value(draft.terminal.font_size.to_string(), window, cx)
        });
        self.pref_terminal_shell_input.update(cx, |s, cx| {
            s.set_value(draft.terminal.shell_path.clone(), window, cx)
        });
    }

    pub(crate) fn save_preferences(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut next: Preferences = self.preferences_draft.clone();
        next.app.timezone = self.pref_timezone_input.read(cx).value().trim().to_string();
        if next.app.timezone.is_empty() {
            next.app.timezone = "system".into();
        }
        next.proxy.https_proxy = self.pref_proxy_input.read(cx).value().trim().to_string();
        next.kubernetes.helm_binary_path = self
            .pref_helm_path_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        next.editor.font_family = self
            .pref_editor_font_family_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        if let Ok(size) = self
            .pref_editor_font_size_input
            .read(cx)
            .value()
            .trim()
            .parse::<u8>()
        {
            next.editor.font_size = size;
        }
        if let Ok(tab) = self
            .pref_editor_tab_size_input
            .read(cx)
            .value()
            .trim()
            .parse::<u8>()
        {
            next.editor.tab_size = tab;
        }
        next.terminal.font_family = self
            .pref_terminal_font_family_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        if let Ok(size) = self
            .pref_terminal_font_size_input
            .read(cx)
            .value()
            .trim()
            .parse::<u8>()
        {
            next.terminal.font_size = size;
        }
        next.terminal.shell_path = self
            .pref_terminal_shell_input
            .read(cx)
            .value()
            .trim()
            .to_string();

        match self.preferences_engine.save(SavePreferencesRequest {
            revision: self.preferences_revision,
            preferences: next,
        }) {
            Ok(snapshot) => {
                self.preferences_revision = snapshot.revision;
                self.preferences_draft = snapshot.preferences.clone();
                self.apply_theme(snapshot.preferences.app.theme, Some(window), cx);
                self.preferences_error = None;
                self.preferences_open = false;
            }
            Err(err) => {
                self.preferences_error = Some(preferences_error_message(err).to_string());
            }
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
        Theme::change(mode, window, cx);
    }

    pub(crate) fn render_preferences_modal(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let section = self.preferences_section;
        let draft = self.preferences_draft.clone();
        let error = self.preferences_error.clone();

        let tab_idx = match section {
            PreferencesSection::App => 0,
            PreferencesSection::Proxy => 1,
            PreferencesSection::Kubernetes => 2,
            PreferencesSection::Editor => 3,
            PreferencesSection::Terminal => 4,
        };

        v_flex()
            .w(px(680.0))
            .max_h(px(560.0))
            .rounded(px(10.0))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .text_color(cx.theme().popover_foreground)
            .overflow_hidden()
            .child(
                h_flex()
                    .w_full()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .items_center()
                    .justify_between()
                    .child(div().text_base().font_semibold().child("Preferences"))
                    .child(
                        Button::new("close-preferences-modal")
                            .xsmall()
                            .ghost()
                            .icon(IconName::Close)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.preferences_open = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .px_4()
                    .pt_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        TabBar::new("preferences-tabs")
                            .selected_index(tab_idx)
                            .on_click(cx.listener(|this, idx: &usize, _, cx| {
                                this.preferences_section = match *idx {
                                    0 => PreferencesSection::App,
                                    1 => PreferencesSection::Proxy,
                                    2 => PreferencesSection::Kubernetes,
                                    3 => PreferencesSection::Editor,
                                    _ => PreferencesSection::Terminal,
                                };
                                cx.notify();
                            }))
                            .child(Tab::new().label("App"))
                            .child(Tab::new().label("Proxy"))
                            .child(Tab::new().label("Kubernetes"))
                            .child(Tab::new().label("Editor"))
                            .child(Tab::new().label("Terminal")),
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
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(msg),
                )
            })
            .child(
                v_flex()
                    .flex_1()
                    .p_4()
                    .gap_3()
                    .overflow_y_scrollbar()
                    .child(match section {
                        PreferencesSection::App => v_flex()
                            .gap_3()
                            .child(
                                h_flex()
                                    .items_center()
                                    .justify_between()
                                    .child(div().text_sm().child("Theme"))
                                    .child(
                                        h_flex()
                                            .gap_1p5()
                                            .children(
                                                [
                                                    (AppTheme::System, "System"),
                                                    (AppTheme::Dark, "Dark"),
                                                    (AppTheme::Light, "Light"),
                                                ]
                                                .into_iter()
                                                .map(|(mode, label)| {
                                                    let active = draft.app.theme == mode;
                                                    Button::new(SharedString::from(format!(
                                                        "pref-theme-{label}"
                                                    )))
                                                    .xsmall()
                                                    .when(active, |b| b.primary())
                                                    .when(!active, |b| b.outline())
                                                    .label(label)
                                                    .on_click(cx.listener(
                                                        move |this, _, window, cx| {
                                                            this.preferences_draft.app.theme = mode;
                                                            this.apply_theme(
                                                                mode,
                                                                Some(window),
                                                                cx,
                                                            );
                                                            cx.notify();
                                                        },
                                                    ))
                                                }),
                                            ),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(div().text_xs().child("Timezone"))
                                    .child(Input::new(&self.pref_timezone_input).small()),
                            )
                            .child(
                                Switch::new("pref-persistent-search")
                                    .label("Keep search queries across resource navigation")
                                    .checked(draft.app.persistent_search)
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.preferences_draft.app.persistent_search = *checked;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Switch::new("pref-restore-last-page")
                                    .label("Restore last visited page on startup")
                                    .checked(draft.app.restore_last_page)
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.preferences_draft.app.restore_last_page = *checked;
                                        cx.notify();
                                    })),
                            )
                            .into_any_element(),
                        PreferencesSection::Proxy => v_flex()
                            .gap_3()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(div().text_xs().child("HTTPS Proxy URL"))
                                    .child(Input::new(&self.pref_proxy_input).small()),
                            )
                            .child(
                                Switch::new("pref-allow-untrusted-cas")
                                    .label("Allow untrusted Certificate Authorities (recorded preference)")
                                    .checked(draft.proxy.allow_untrusted_cas)
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.preferences_draft.proxy.allow_untrusted_cas = *checked;
                                        cx.notify();
                                    })),
                            )
                            .into_any_element(),
                        PreferencesSection::Kubernetes => v_flex()
                            .gap_3()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(div().text_xs().child("Custom Helm Binary Path"))
                                    .child(Input::new(&self.pref_helm_path_input).small()),
                            )
                            .child(
                                Switch::new("pref-helm-server-side")
                                    .label("Use server-side dry-run for Helm operations")
                                    .checked(draft.kubernetes.helm_server_side)
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.preferences_draft.kubernetes.helm_server_side =
                                            *checked;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Switch::new("pref-discover-default-kubeconfig")
                                    .label("Auto-discover ~/.kube/config in Catalog")
                                    .checked(draft.kubernetes.discover_default_kubeconfig)
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.preferences_draft
                                            .kubernetes
                                            .discover_default_kubeconfig = *checked;
                                        cx.notify();
                                    })),
                            )
                            .into_any_element(),
                        PreferencesSection::Editor => v_flex()
                            .gap_3()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(div().text_xs().child("Font Family"))
                                    .child(Input::new(&self.pref_editor_font_family_input).small()),
                            )
                            .child(
                                h_flex()
                                    .gap_3()
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .gap_1()
                                            .child(div().text_xs().child("Font Size (10-50)"))
                                            .child(
                                                Input::new(&self.pref_editor_font_size_input)
                                                    .small(),
                                            ),
                                    )
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .gap_1()
                                            .child(div().text_xs().child("Tab Size (1-16)"))
                                            .child(
                                                Input::new(&self.pref_editor_tab_size_input)
                                                    .small(),
                                            ),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .items_center()
                                    .justify_between()
                                    .child(div().text_sm().child("Line Numbers"))
                                    .child(
                                        h_flex()
                                            .gap_1p5()
                                            .children(
                                                [
                                                    (LineNumbers::On, "On"),
                                                    (LineNumbers::Off, "Off"),
                                                    (LineNumbers::Relative, "Relative"),
                                                ]
                                                .into_iter()
                                                .map(|(ln, label)| {
                                                    let active = draft.editor.line_numbers == ln;
                                                    Button::new(SharedString::from(format!(
                                                        "pref-ln-{label}"
                                                    )))
                                                    .xsmall()
                                                    .when(active, |b| b.primary())
                                                    .when(!active, |b| b.outline())
                                                    .label(label)
                                                    .on_click(cx.listener(
                                                        move |this, _, _, cx| {
                                                            this.preferences_draft
                                                                .editor
                                                                .line_numbers = ln;
                                                            cx.notify();
                                                        },
                                                    ))
                                                }),
                                            ),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        Switch::new("pref-editor-minimap")
                                            .label("Show Minimap")
                                            .checked(draft.editor.minimap)
                                            .on_click(cx.listener(
                                                |this, checked: &bool, _, cx| {
                                                    this.preferences_draft.editor.minimap =
                                                        *checked;
                                                    cx.notify();
                                                },
                                            )),
                                    )
                                    .child(
                                        h_flex()
                                            .gap_1p5()
                                            .children(
                                                [
                                                    (MinimapSide::Left, "Left"),
                                                    (MinimapSide::Right, "Right"),
                                                ]
                                                .into_iter()
                                                .map(|(side, label)| {
                                                    let active = draft.editor.minimap_side == side;
                                                    Button::new(SharedString::from(format!(
                                                        "pref-minimap-{label}"
                                                    )))
                                                    .xsmall()
                                                    .when(active, |b| b.primary())
                                                    .when(!active, |b| b.outline())
                                                    .label(label)
                                                    .on_click(cx.listener(
                                                        move |this, _, _, cx| {
                                                            this.preferences_draft
                                                                .editor
                                                                .minimap_side = side;
                                                            cx.notify();
                                                        },
                                                    ))
                                                }),
                                            ),
                                    ),
                            )
                            .into_any_element(),
                        PreferencesSection::Terminal => v_flex()
                            .gap_3()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(div().text_xs().child("Terminal Font Family"))
                                    .child(
                                        Input::new(&self.pref_terminal_font_family_input).small(),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_3()
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .gap_1()
                                            .child(div().text_xs().child("Font Size (10-50)"))
                                            .child(
                                                Input::new(&self.pref_terminal_font_size_input)
                                                    .small(),
                                            ),
                                    )
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .gap_1()
                                            .child(div().text_xs().child("Default Shell Path"))
                                            .child(
                                                Input::new(&self.pref_terminal_shell_input).small(),
                                            ),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .items_center()
                                    .justify_between()
                                    .child(div().text_sm().child("Terminal Theme"))
                                    .child(
                                        h_flex()
                                            .gap_1p5()
                                            .children(
                                                [
                                                    (TerminalTheme::Match, "Match App"),
                                                    (TerminalTheme::Dark, "Dark"),
                                                    (TerminalTheme::Light, "Light"),
                                                ]
                                                .into_iter()
                                                .map(|(tt, label)| {
                                                    let active = draft.terminal.theme == tt;
                                                    Button::new(SharedString::from(format!(
                                                        "pref-term-theme-{label}"
                                                    )))
                                                    .xsmall()
                                                    .when(active, |b| b.primary())
                                                    .when(!active, |b| b.outline())
                                                    .label(label)
                                                    .on_click(cx.listener(
                                                        move |this, _, _, cx| {
                                                            this.preferences_draft
                                                                .terminal
                                                                .theme = tt;
                                                            cx.notify();
                                                        },
                                                    ))
                                                }),
                                            ),
                                    ),
                            )
                            .child(
                                Switch::new("pref-term-copy-on-select")
                                    .label("Copy selection to clipboard automatically")
                                    .checked(draft.terminal.copy_on_select)
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.preferences_draft.terminal.copy_on_select = *checked;
                                        cx.notify();
                                    })),
                            )
                            .into_any_element(),
                    }),
            )
            .child(
                h_flex()
                    .w_full()
                    .px_4()
                    .py_3()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .items_center()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("cancel-preferences-btn")
                            .small()
                            .outline()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.preferences_open = false;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("save-preferences-btn")
                            .small()
                            .primary()
                            .label("Save Preferences")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save_preferences(window, cx);
                            })),
                    ),
            )
    }
}
