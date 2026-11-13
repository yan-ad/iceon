//! Shared visual-order tab targeting. Deferred closes retain identities, never indices.
use crate::IceonApp;
use gpui_kit::base::Tab;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _,
    menu::{ContextMenuExt as _, PopupMenuItem},
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConsoleTab {
    Device(u64),
    Pod(u64),
    Yaml(u64),
}

fn keyboard_target(order: &[ConsoleTab], current: ConsoleTab, key: &str) -> Option<ConsoleTab> {
    let index = order.iter().position(|tab| *tab == current)?;
    let next = match key {
        "left" => (index + order.len() - 1) % order.len(),
        "right" => (index + 1) % order.len(),
        "home" => 0,
        "end" => order.len() - 1,
        "enter" | "space" => index,
        _ => return None,
    };
    order.get(next).copied()
}

pub(crate) fn process_detail(pid: Option<u32>) -> String {
    pid.map_or_else(
        || "Process ID (PID): unavailable".into(),
        |pid| format!("Process ID (PID): {pid}"),
    )
}

fn boxed_style(
    background: Hsla,
    foreground: Hsla,
    border: Hsla,
    selected: bool,
) -> StyleRefinement {
    StyleRefinement::default()
        .h(px(33.))
        .rounded_none()
        .bg(background)
        .text_color(foreground)
        .border_r_1()
        .border_b(px(if selected { 0.0 } else { 1.0 }))
        .border_color(border)
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum CloseTabs {
    One,
    Others,
    All,
    Right,
    Left,
}

fn close_targets(order: &[ConsoleTab], index: usize, action: CloseTabs) -> Vec<ConsoleTab> {
    if index >= order.len() {
        return vec![];
    }
    order
        .iter()
        .enumerate()
        .filter(|(i, _)| match action {
            CloseTabs::One => *i == index,
            CloseTabs::Others => *i != index,
            CloseTabs::All => true,
            CloseTabs::Right => *i > index,
            CloseTabs::Left => *i < index,
        })
        .map(|(_, tab)| *tab)
        .collect()
}

impl IceonApp {
    pub(crate) fn boxed_console_tab(
        &self,
        target: ConsoleTab,
        title: String,
        detail: String,
        icon: Icon,
        cx: &Context<Self>,
    ) -> Tab {
        let selected = self.active_console_tab() == Some(target);
        let palette =
            crate::preference_runtime::terminal_palette(self.preferences_draft.terminal.theme, cx);
        let background = if matches!(target, ConsoleTab::Yaml(_)) {
            cx.theme()
                .highlight_theme
                .style
                .editor_background
                .unwrap_or(cx.theme().background)
        } else {
            palette.background
        };
        let foreground = if matches!(target, ConsoleTab::Yaml(_)) {
            cx.theme().foreground
        } else {
            palette.foreground
        };
        let fill = if selected {
            background
        } else {
            transparent_black()
        };
        let ink = if selected {
            foreground
        } else {
            cx.theme().tab_foreground
        };
        let border = cx.theme().border;
        let hover = cx.theme().secondary;
        let aria = format!("{title} · {detail}");
        let owner = cx.entity().downgrade();
        let keyboard_owner = owner.clone();
        let tooltip_title = title.clone();
        let tab = Tab::new(SharedString::from(format!("console-tab-{target:?}")))
            .selected(selected)
            .group("console-session-tab")
            .relative()
            .focusable()
            .tab_stop(selected)
            .set_position(
                self.console_tab_order()
                    .iter()
                    .position(|tab| *tab == target)
                    .unwrap_or(0)
                    + 1,
                self.console_tab_order().len(),
            )
            .accessibility_label(aria)
            .min_w(px(160.))
            .max_w(px(280.))
            .flex_shrink_0()
            .px_3()
            .gap_2()
            .text_sm()
            .map(|mut tab| {
                tab.style()
                    .refine(&boxed_style(fill, ink, border, selected));
                tab
            })
            .hover(move |tab| if selected { tab } else { tab.bg(hover) })
            .focus_visible(|style| style.border_color(cx.theme().primary))
            .on_key_down(move |event, window, cx| {
                if event.keystroke.modifiers.control
                    || event.keystroke.modifiers.platform
                    || event.keystroke.modifiers.alt
                {
                    return;
                }
                let key = event.keystroke.key.as_str();
                let _ = keyboard_owner.update(cx, |app, cx| {
                    if let Some(next) = keyboard_target(
                        &app.console_tab_order(),
                        app.active_console_tab().unwrap_or(target),
                        key,
                    ) {
                        let focus = window.focused(cx);
                        cx.stop_propagation();
                        app.select_console_tab(next, window, cx);
                        if !matches!(key, "enter" | "space") {
                            if let Some(focus) = focus {
                                window.focus(&focus, cx);
                            }
                        }
                    }
                });
            })
            .child(icon.small())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(crate::pod_dock_view::console_tab_title(&title)),
            )
            .tooltip(move |window, cx| {
                let title = tooltip_title.clone();
                let detail = detail.clone();
                Tooltip::element(move |_, cx| {
                    v_flex()
                        .min_w(px(220.))
                        .max_w(px(480.))
                        .gap_2()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .child(title.clone()),
                        )
                        .child(div().h(px(1.)).w_full().bg(cx.theme().border))
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(detail.clone()),
                        )
                })
                .p_3()
                .rounded(px(8.))
                .build(window, cx)
            })
            .on_click(move |_, window, cx| {
                let _ = owner.update(cx, |app, cx| app.select_console_tab(target, window, cx));
            });
        self.console_tab_menu(tab, target, cx)
    }
    pub(crate) fn console_tab_order(&self) -> Vec<ConsoleTab> {
        self.device_terminals
            .iter()
            .map(|t| ConsoleTab::Device(t.key))
            .chain(self.pod_dock_tabs.iter().map(|t| ConsoleTab::Pod(t.key)))
            .chain(self.yaml_dock_tabs.iter().map(|t| ConsoleTab::Yaml(t.key)))
            .collect()
    }
    pub(crate) fn active_console_tab(&self) -> Option<ConsoleTab> {
        if let Some(key) = self.active_yaml_tab {
            Some(ConsoleTab::Yaml(key))
        } else if self.local_terminal_active {
            self.active_device_terminal.map(ConsoleTab::Device)
        } else {
            self.active_pod_tab
                .and_then(|i| self.pod_dock_tabs.get(i))
                .map(|t| ConsoleTab::Pod(t.key))
        }
    }
    pub(crate) fn select_console_tab(
        &mut self,
        tab: ConsoleTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match tab {
            ConsoleTab::Yaml(key) => {
                self.select_yaml_tab(key, window, cx);
                return;
            }
            ConsoleTab::Device(key) => {
                let Some(tab) = self.device_terminals.iter().find(|t| t.key == key) else {
                    return;
                };
                self.active_device_terminal = Some(key);
                self.local_terminal_active = true;
                window.focus(&tab.terminal.focus_handle(cx), cx);
            }
            ConsoleTab::Pod(key) => {
                let Some(index) = self.pod_dock_tabs.iter().position(|t| t.key == key) else {
                    return;
                };
                self.active_pod_tab = Some(index);
                self.local_terminal_active = false;
                if let Some(terminal) = self.pod_dock_tabs[index].terminal.as_ref().filter(|_| {
                    self.pod_dock_tabs[index].mode
                        != iceon_core::connection::pod_runtime::PodStreamMode::Logs
                }) {
                    window.focus(&terminal.focus_handle(cx), cx);
                } else {
                    window.focus(&self.console_focus, cx);
                }
            }
        }
        self.active_yaml_tab = None;
        self.pod_dock_open = true;
        cx.notify();
    }
    pub(crate) fn close_focused_console_tab(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.console_focus.contains_focused(window, cx) {
            return false;
        }
        if let Some(tab) = self.active_console_tab() {
            self.request_close_console_tabs(tab, CloseTabs::One, window, cx);
        }
        true
    }
    pub(crate) fn request_close_console_tabs(
        &mut self,
        anchor: ConsoleTab,
        action: CloseTabs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let order = self.console_tab_order();
        let Some(index) = order.iter().position(|t| *t == anchor) else {
            return;
        };
        let targets = close_targets(&order, index, action);
        let dirty_count = self
            .yaml_dock_tabs
            .iter()
            .filter(|t| targets.contains(&ConsoleTab::Yaml(t.key)) && t.dirty(cx))
            .count();
        if dirty_count > 0 {
            self.confirm_destructive_action("Discard YAML changes?", format!("{dirty_count} selected YAML buffer(s) have unsaved changes. Discard changes and close the selected tabs? Cancel to keep all tabs open, or use Save & Close on each YAML buffer first."), "Discard", window, cx,
                move |app, window, cx| app.close_console_targets(&targets, window, cx));
        } else {
            self.close_console_targets(&targets, window, cx);
        }
    }
    fn close_console_targets(
        &mut self,
        targets: &[ConsoleTab],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let order = self.console_tab_order();
        let active = self.active_console_tab();
        let fallback = active.filter(|tab| !targets.contains(tab)).or_else(|| {
            let index = active
                .and_then(|tab| order.iter().position(|t| *t == tab))
                .unwrap_or(0);
            order
                .iter()
                .skip(index)
                .chain(order.iter().take(index).rev())
                .find(|tab| !targets.contains(tab))
                .copied()
        });
        for target in targets {
            match *target {
                ConsoleTab::Device(key) => self.close_device_terminal(key, cx),
                ConsoleTab::Pod(key) => {
                    if let Some(i) = self.pod_dock_tabs.iter().position(|t| t.key == key) {
                        self.close_pod_tab(i, cx);
                    }
                }
                ConsoleTab::Yaml(key) => self.close_yaml_tab(key, cx),
            }
        }
        if let Some(tab) = fallback {
            self.select_console_tab(tab, window, cx);
        } else {
            self.pod_dock_open = false;
            self.active_yaml_tab = None;
            self.active_pod_tab = None;
            self.active_device_terminal = None;
            self.local_terminal_active = false;
            window.focus(&self.console_focus, cx);
        }
        cx.notify();
    }
    pub(crate) fn console_tab_menu(&self, tab: Tab, target: ConsoleTab, cx: &Context<Self>) -> Tab {
        let owner = cx.entity().downgrade();
        tab.child(
            div()
                .id(SharedString::from(format!("console-menu-{target:?}")))
                .absolute()
                .inset_0()
                .context_menu(move |mut menu, _, cx| {
                    let order = owner
                        .upgrade()
                        .map(|app| app.read(cx).console_tab_order())
                        .unwrap_or_default();
                    for (label, action) in [
                        ("Close", CloseTabs::One),
                        ("Close Other Tabs", CloseTabs::Others),
                        ("Close All Tabs", CloseTabs::All),
                        ("Close Tabs to Right", CloseTabs::Right),
                        ("Close Tabs to Left", CloseTabs::Left),
                    ] {
                        let enabled = order
                            .iter()
                            .position(|t| *t == target)
                            .is_some_and(|i| !close_targets(&order, i, action).is_empty());
                        let owner = owner.clone();
                        menu = menu.item(PopupMenuItem::new(label).disabled(!enabled).on_click(
                            move |_, window, cx| {
                                let _ = owner.update(cx, |app, cx| {
                                    app.request_close_console_tabs(target, action, window, cx)
                                });
                            },
                        ));
                    }
                    menu
                }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{CloseTabs, ConsoleTab, close_targets};

    #[test]
    fn boxed_tab_active_fill_is_the_terminal_color_and_has_no_bottom_seam() {
        use gpui_kit::{Hsla, Styled as _, px, rgb};
        for background in [0x000000, 0xffffff, 0x242433] {
            let bg = Hsla::from(rgb(background));
            let active = super::boxed_style(bg, rgb(0xabcdef).into(), rgb(0x454545).into(), true);
            let inactive = super::boxed_style(
                gpui_kit::transparent_black(),
                rgb(0xabcdef).into(),
                rgb(0x454545).into(),
                false,
            );
            assert_eq!(
                active.background.as_ref().unwrap().color().unwrap(),
                bg.into()
            );
            assert_eq!(active.border_widths.bottom, Some(px(0.).into()));
            assert_eq!(inactive.border_widths.bottom, Some(px(1.).into()));
            assert_eq!(
                active.corner_radii,
                gpui_kit::StyleRefinement::default()
                    .rounded_none()
                    .corner_radii
            );
        }
    }

    #[test]
    fn process_hover_details_never_substitute_an_app_pid() {
        assert_eq!(
            super::process_detail(Some(86132)),
            "Process ID (PID): 86132"
        );
        assert_eq!(super::process_detail(None), "Process ID (PID): unavailable");
    }

    #[test]
    fn tab_keyboard_navigation_preserves_stable_session_order() {
        use super::keyboard_target;
        let order = [
            ConsoleTab::Device(9),
            ConsoleTab::Pod(7),
            ConsoleTab::Yaml(1),
        ];
        assert_eq!(keyboard_target(&order, order[0], "left"), Some(order[2]));
        assert_eq!(keyboard_target(&order, order[2], "right"), Some(order[0]));
        assert_eq!(keyboard_target(&order, order[1], "home"), Some(order[0]));
        assert_eq!(keyboard_target(&order, order[1], "end"), Some(order[2]));
        assert_eq!(keyboard_target(&order, order[1], "enter"), Some(order[1]));
        assert_eq!(keyboard_target(&[], order[0], "right"), None);
        assert_eq!(
            keyboard_target(&order, ConsoleTab::Device(88), "left"),
            None
        );
        assert_eq!(keyboard_target(&order, order[0], "x"), None);
    }
    #[test]
    fn targets_follow_device_pod_yaml_visual_order() {
        let order = [
            ConsoleTab::Device(9),
            ConsoleTab::Device(2),
            ConsoleTab::Pod(7),
            ConsoleTab::Yaml(1),
            ConsoleTab::Yaml(3),
        ];
        assert_eq!(close_targets(&order, 2, CloseTabs::One), order[2..3]);
        assert_eq!(close_targets(&order, 2, CloseTabs::Left), order[..2]);
        assert_eq!(close_targets(&order, 2, CloseTabs::Right), order[3..]);
        assert_eq!(close_targets(&order, 2, CloseTabs::All), order);
        assert_eq!(
            close_targets(&order, 2, CloseTabs::Others),
            vec![order[0], order[1], order[3], order[4]]
        );
        assert!(close_targets(&order, 0, CloseTabs::Left).is_empty());
        assert!(close_targets(&order, 4, CloseTabs::Right).is_empty());
    }
    #[test]
    fn empty_and_stale_indices_never_close_anything() {
        for action in [
            CloseTabs::One,
            CloseTabs::Others,
            CloseTabs::All,
            CloseTabs::Right,
            CloseTabs::Left,
        ] {
            assert!(close_targets(&[], 0, action).is_empty());
            assert!(close_targets(&[ConsoleTab::Yaml(1)], 1, action).is_empty());
            assert!(close_targets(&[ConsoleTab::Device(1)], usize::MAX, action).is_empty());
        }
    }
}
