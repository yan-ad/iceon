//! Explicit, device-local PTY sessions rendered by the shared native terminal surface.
use crate::{
    IceonApp,
    terminal_surface::{TerminalEvent, TerminalSurface},
};
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    h_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use iceon_core::local_terminal::{LocalTerminal, Status};
use std::{path::Path, sync::Arc, time::Duration};

pub(crate) struct DeviceTerminalTab {
    pub key: u64,
    pub title: String,
    pub status: String,
    pub terminal: Entity<TerminalSurface>,
    process: Option<Arc<LocalTerminal>>,
}

impl Drop for DeviceTerminalTab {
    fn drop(&mut self) {
        if let Some(process) = &self.process {
            process.close();
        }
    }
}

fn status_text(status: Status) -> String {
    match status {
        Status::Running => "running".into(),
        Status::Closed => "closed".into(),
        Status::Exited(code) => format!("exited ({code})"),
        Status::Failed => "failed".into(),
    }
}

/// Use the shell's inherited directory without enumerating its contents.
fn local_terminal_title(cwd: Option<&Path>, home: Option<&Path>, shell: &str) -> String {
    let directory = match cwd {
        Some(cwd) if Some(cwd) == home => "~".to_owned(),
        Some(cwd) => cwd
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| cwd.display().to_string()),
        None => "Device".into(),
    };
    format!("{directory} — {shell}")
}

fn local_poll_changes(
    has_output: bool,
    dropped: bool,
    old_status: &str,
    status: &str,
) -> (bool, bool) {
    (has_output || dropped, old_status != status)
}

impl IceonApp {
    pub(crate) fn open_device_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.active_yaml_tab = None;
        self.local_terminal_active = true;
        self.pod_dock_open = true;
        let existing = self
            .active_device_terminal
            .and_then(|key| self.device_terminals.iter().position(|tab| tab.key == key))
            .or_else(|| (!self.device_terminals.is_empty()).then_some(0));
        if let Some(index) = existing {
            let tab = &self.device_terminals[index];
            self.active_device_terminal = Some(tab.key);
            window.focus(&tab.terminal.focus_handle(cx), cx);
            cx.notify();
        } else {
            self.new_device_terminal(window, cx);
        }
    }

    pub(crate) fn new_device_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.device_terminals.len() + self.pod_dock_tabs.len() + self.yaml_dock_tabs.len() >= 12
        {
            self.cluster_error = Some("Console session limit reached (12)".into());
            cx.notify();
            return;
        }
        let cwd = std::env::current_dir().ok();
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".into());
        let shell = Path::new(&shell)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("sh");
        let title = local_terminal_title(cwd.as_deref(), home.as_deref(), shell);
        let key = self.next_operation_id();
        let terminal = cx.new(|cx| {
            // Keep the local PTY and vt parser in the same initial grid. This
            // also makes a prompt received before the first layout pass visible.
            let mut terminal = TerminalSurface::with_size(80, 24, cx);
            terminal.enable_device_replies();
            terminal
        });
        cx.subscribe_in(
            &terminal,
            window,
            move |app, _, event: &TerminalEvent, _, cx| {
                let Some(tab) = app.device_terminals.iter_mut().find(|tab| tab.key == key) else {
                    return;
                };
                let Some(process) = &tab.process else {
                    return;
                };
                let result = match event {
                    TerminalEvent::Input(input) => process.write(input.as_bytes()),
                    TerminalEvent::Resize { cols, rows } => process.resize(*cols, *rows),
                };
                if let Err(error) = result {
                    tab.status = error;
                    if let Some(dock) = &app.workspace_dock {
                        dock.notify_console(cx);
                    }
                }
            },
        )
        .detach();
        window.focus(&terminal.focus_handle(cx), cx);
        self.device_terminals.push(DeviceTerminalTab {
            key,
            title,
            status: "opening".into(),
            terminal: terminal.clone(),
            process: None,
        });
        self.active_device_terminal = Some(key);
        self.active_yaml_tab = None;
        self.local_terminal_active = true;
        self.pod_dock_open = true;
        cx.notify();
        cx.spawn_in(window, async move |owner, cx| {
            let opened = iceon_core::runtime::spawn_blocking(|| LocalTerminal::open(80, 24)).await;
            let process = match opened {
                Ok(Ok(process)) => Arc::new(process),
                result => {
                    let message = match result {
                        Ok(Err(error)) => error,
                        Err(_) => "Unable to start device shell".into(),
                        _ => unreachable!(),
                    };
                    let _ = owner.update_in(cx, |app, _, cx| {
                        if let Some(tab) =
                            app.device_terminals.iter_mut().find(|tab| tab.key == key)
                        {
                            tab.status = message.clone();
                            tab.terminal.update(cx, |surface, cx| {
                                surface.feed(&format!("\r\n{message}\r\n"), cx)
                            });
                        }
                        if let Some(dock) = &app.workspace_dock {
                            dock.notify_console(cx);
                        }
                    });
                    return;
                }
            };
            let retained = owner.update_in(cx, |app, _, cx| {
                let Some(tab) = app.device_terminals.iter_mut().find(|tab| tab.key == key) else {
                    return false;
                };
                tab.title =
                    local_terminal_title(cwd.as_deref(), home.as_deref(), process.shell_name());
                tab.status = "running".into();
                let (cols, rows) = tab.terminal.read(cx).size();
                let _ = process.resize(cols, rows);
                tab.process = Some(process.clone());
                if let Some(dock) = &app.workspace_dock {
                    dock.notify_console(cx);
                }
                true
            });
            if !matches!(retained, Ok(true)) {
                process.close();
                return;
            }
            let mut last_status = "running".to_owned();
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(40))
                    .await;
                let present = owner.read_with(cx, |app, _| {
                    app.device_terminals.iter().any(|tab| tab.key == key)
                });
                if !matches!(present, Ok(true)) {
                    process.close();
                    break;
                }
                let snapshot = process.read();
                let status = status_text(snapshot.status);
                let finished = snapshot.status != Status::Running;
                let (output_changed, status_changed) = local_poll_changes(
                    !snapshot.output.is_empty(),
                    snapshot.dropped,
                    &last_status,
                    &status,
                );
                // Feed the terminal independently: raw PTY output must not take a mutable
                // workspace borrow (device-query reply events may update the owner).
                if output_changed
                    && cx
                        .update(|_, cx| {
                            terminal.update(cx, |surface, cx| {
                                if snapshot.dropped {
                                    surface.reset(cx);
                                }
                                if !snapshot.output.is_empty() {
                                    surface.feed_bytes(&snapshot.output, cx);
                                }
                            });
                        })
                        .is_err()
                {
                    process.close();
                    break;
                }
                if status_changed {
                    last_status = status.clone();
                    let updated = owner.update_in(cx, |app, _, cx| {
                        let Some(tab) = app.device_terminals.iter_mut().find(|tab| tab.key == key)
                        else {
                            return false;
                        };
                        tab.status = status;
                        if let Some(dock) = &app.workspace_dock {
                            dock.notify_console(cx);
                        }
                        true
                    });
                    if !matches!(updated, Ok(true)) {
                        process.close();
                        break;
                    }
                }
                if finished {
                    break;
                }
            }
        })
        .detach();
    }

    pub(crate) fn close_device_terminal(&mut self, key: u64, cx: &mut Context<Self>) {
        if let Some(index) = self.device_terminals.iter().position(|tab| tab.key == key) {
            self.device_terminals.remove(index); // Drop closes exactly this owned PTY.
            if self.active_device_terminal == Some(key) {
                self.active_device_terminal = self
                    .device_terminals
                    .get(index.min(self.device_terminals.len().saturating_sub(1)))
                    .map(|tab| tab.key);
            }
            cx.notify();
        }
    }

    pub(crate) fn toggle_console_zoom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(dock) = &self.workspace_dock {
            dock.toggle_console_zoom(window, cx);
        }
    }

    pub(crate) fn render_device_terminal(
        &mut self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = self
            .active_device_terminal
            .and_then(|key| self.device_terminals.iter().find(|tab| tab.key == key))
            .or_else(|| self.device_terminals.first());
        if let Some(tab) = selected {
            return div()
                .size_full()
                .min_w_0()
                .min_h_0()
                .overflow_hidden()
                .child(tab.terminal.clone())
                .into_any_element();
        }
        h_flex()
            .size_full()
            .items_center()
            .justify_center()
            .child(
                Button::new("open-device-terminal")
                    .ghost()
                    .label("Open device terminal")
                    .tooltip("Open device terminal")
                    .accessibility_label("Open device terminal")
                    .on_click(
                        cx.listener(|app, _, window, cx| app.open_device_terminal(window, cx)),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{local_poll_changes, local_terminal_title};
    use std::path::Path;

    #[test]
    fn local_titles_identify_directory_and_shell() {
        assert_eq!(
            local_terminal_title(Some(Path::new("/work/icekube")), None, "fish"),
            "icekube — fish"
        );
        assert_eq!(
            local_terminal_title(
                Some(Path::new("/home/me")),
                Some(Path::new("/home/me")),
                "zsh"
            ),
            "~ — zsh"
        );
        assert_eq!(
            local_terminal_title(Some(Path::new("/")), None, "sh"),
            "/ — sh"
        );
        assert_eq!(local_terminal_title(None, None, "sh"), "Device — sh");
    }

    #[test]
    fn output_and_status_have_independent_dirty_flags() {
        assert_eq!(
            local_poll_changes(false, false, "running", "running"),
            (false, false)
        );
        assert_eq!(
            local_poll_changes(true, false, "running", "running"),
            (true, false)
        );
        assert_eq!(
            local_poll_changes(false, true, "running", "running"),
            (true, false)
        );
        assert_eq!(
            local_poll_changes(false, false, "running", "exited (0)"),
            (false, true)
        );
        assert_eq!(
            local_poll_changes(true, false, "running", "failed"),
            (true, true)
        );
    }
}
