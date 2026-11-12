//! Shared visual-order tab targeting. Deferred closes retain identities, never indices.
use crate::IceonApp;
use gpui_kit::component::{
    menu::{ContextMenuExt as _, PopupMenuItem},
    tab::Tab,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConsoleTab {
    Device(u64),
    Pod(u64),
    Yaml(u64),
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
    pub(crate) fn console_tab_order(&self) -> Vec<ConsoleTab> {
        self.device_terminals
            .iter()
            .map(|t| ConsoleTab::Device(t.key))
            .chain(self.pod_dock_tabs.iter().map(|t| ConsoleTab::Pod(t.key)))
            .chain(self.yaml_dock_tabs.iter().map(|t| ConsoleTab::Yaml(t.key)))
            .collect()
    }
    fn active_console_tab(&self) -> Option<ConsoleTab> {
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
        // TabBar accepts Tab rather than arbitrary wrappers. An absolute child
        // covers its tab without changing label layout; left clicks bubble to Tab.
        tab.child(
            div()
                .id(SharedString::from(format!("console-menu-{target:?}")))
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
                })
                .absolute()
                .inset_0(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{CloseTabs, ConsoleTab, close_targets};
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
