//! Bottom operational dock for Pod Logs, Interactive Exec Shell, Attach, and Port-Forwards.

use std::time::Duration;

use gpui_kit::component::{
    ActiveTheme as _, IconName, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    scroll::ScrollableElement as _,
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use iceon_core::connection::{
    pod_runtime::{
        PodForwardRequest, PodForwardStopRequest, PodInputRequest, PodResizeRequest,
        PodStartRequest, PodStreamMode, PodStreamRequest,
    },
    pods::PodRequest,
};

use crate::IceonApp;

#[derive(Clone, Debug)]
pub struct PodDockTab {
    pub id: String,
    pub pod_name: String,
    pub namespace: String,
    pub uid: String,
    pub container: String,
    pub available_containers: Vec<String>,
    pub mode: PodStreamMode,
    pub status: String,
    pub output_lines: Vec<String>,
    pub previous: bool,
    pub timestamps: bool,
    pub tail_lines: u32,
    pub paused: bool,
    pub shell: String,
    pub cols: u16,
    pub rows: u16,
    pub command_history: Vec<String>,
}

impl PodDockTab {
    pub fn mode_label(&self) -> &'static str {
        match self.mode {
            PodStreamMode::Logs => "LOGS",
            PodStreamMode::Shell => "SHELL",
            PodStreamMode::Attach => "ATTACH",
        }
    }
}

pub const TAIL_LINE_OPTIONS: &[u32] = &[100, 500, 2000, 10000];
pub const TERMINAL_SIZE_PRESETS: &[(u16, u16, &str)] =
    &[(80, 24, "80×24"), (120, 36, "120×36"), (160, 48, "160×48")];

pub fn shell_option_path(label: &str) -> &'static str {
    match label {
        "bash" => "/bin/bash",
        "ash" => "/bin/ash",
        _ => "/bin/sh",
    }
}

impl IceonApp {
    /// Opens a Pod Logs or Shell stream directly from a Pod table row or collection item
    /// by resolving the Pod's UID and container list first via `pod_detail`.
    pub(crate) fn open_pod_stream_for_pod(
        &mut self,
        namespace: String,
        pod_name: String,
        mode: PodStreamMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        if let Some(pod) = self
            .inspector
            .pod_detail
            .as_ref()
            .filter(|p| p.name == pod_name && p.namespace == namespace)
        {
            let containers: Vec<String> = pod.containers.iter().map(|c| c.name.clone()).collect();
            let default_container = pod
                .containers
                .iter()
                .find(|c| c.kind == "container")
                .or_else(|| pod.containers.first())
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "main".into());
            self.open_pod_stream_with_containers(
                namespace,
                pod_name,
                pod.uid.clone(),
                default_container,
                containers,
                mode,
                window,
                cx,
            );
            return;
        }

        let connection = self.connection.clone();
        let req = PodRequest {
            cluster_id: session.cluster_id,
            session_id: session.session_id,
            namespace: namespace.clone(),
            name: pod_name.clone(),
            uid: None,
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = connection.pod_detail(req).await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(pod) => {
                    let containers: Vec<String> =
                        pod.containers.iter().map(|c| c.name.clone()).collect();
                    let default_container = pod
                        .containers
                        .iter()
                        .find(|c| c.kind == "container")
                        .or_else(|| pod.containers.first())
                        .map(|c| c.name.clone())
                        .unwrap_or_else(|| "main".into());
                    this.open_pod_stream_with_containers(
                        namespace,
                        pod_name,
                        pod.uid,
                        default_container,
                        containers,
                        mode,
                        window,
                        cx,
                    );
                }
                Err(err) => {
                    this.cluster_error = Some(format!(
                        "Failed to resolve pod {namespace}/{pod_name} for {:?}: {err:?}",
                        mode
                    ));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(crate) fn open_pod_stream(
        &mut self,
        namespace: String,
        pod_name: String,
        uid: String,
        container: String,
        mode: PodStreamMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut available_containers = self
            .inspector
            .pod_detail
            .as_ref()
            .filter(|p| p.name == pod_name && p.namespace == namespace)
            .map(|p| {
                p.containers
                    .iter()
                    .map(|c| c.name.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !available_containers.contains(&container) {
            available_containers.push(container.clone());
        }
        self.open_pod_stream_with_containers(
            namespace,
            pod_name,
            uid,
            container,
            available_containers,
            mode,
            window,
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn open_pod_stream_with_containers(
        &mut self,
        namespace: String,
        pod_name: String,
        uid: String,
        container: String,
        available_containers: Vec<String>,
        mode: PodStreamMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let placeholder_id = format!("opening-{}", self.next_operation_id());
        let default_shell = shell_option_path("sh").to_string();

        let tab = PodDockTab {
            id: placeholder_id.clone(),
            pod_name: pod_name.clone(),
            namespace: namespace.clone(),
            uid: uid.clone(),
            container: container.clone(),
            available_containers,
            mode,
            status: "opening".into(),
            output_lines: vec![format!(
                "Connecting {:?} stream to {namespace}/{pod_name} ({container})...",
                mode
            )],
            previous: false,
            timestamps: true,
            tail_lines: 500,
            paused: false,
            shell: default_shell.clone(),
            cols: 120,
            rows: 36,
            command_history: vec![],
        };
        self.pod_dock_tabs.push(tab);
        let tab_index = self.pod_dock_tabs.len() - 1;
        self.active_pod_tab = Some(tab_index);
        self.pod_dock_open = true;
        cx.notify();

        self.spawn_pod_stream_worker(
            session.cluster_id,
            session.session_id,
            placeholder_id,
            namespace,
            pod_name,
            uid,
            container,
            mode,
            false,
            true,
            500,
            default_shell,
            window,
            cx,
        );
    }

    pub(crate) fn reopen_pod_tab(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let Some(existing) = self.pod_dock_tabs.get(index).cloned() else {
            return;
        };
        if !existing.id.starts_with("opening-") {
            let _ = self.connection.pod_stream_close(PodStreamRequest {
                cluster_id: session.cluster_id.clone(),
                session_id: session.session_id.clone(),
                stream_id: existing.id,
            });
        }

        let placeholder_id = format!("opening-{}", self.next_operation_id());
        if let Some(tab) = self.pod_dock_tabs.get_mut(index) {
            tab.id = placeholder_id.clone();
            tab.status = "opening".into();
            tab.paused = false;
            tab.output_lines = vec![format!(
                "Reconnecting {:?} stream to {}/{} ({})...",
                tab.mode, tab.namespace, tab.pod_name, tab.container
            )];
        }
        cx.notify();

        self.spawn_pod_stream_worker(
            session.cluster_id,
            session.session_id,
            placeholder_id,
            existing.namespace,
            existing.pod_name,
            existing.uid,
            existing.container,
            existing.mode,
            existing.previous,
            existing.timestamps,
            existing.tail_lines,
            existing.shell,
            window,
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_pod_stream_worker(
        &mut self,
        cluster_id: String,
        session_id: String,
        placeholder_id: String,
        namespace: String,
        pod_name: String,
        uid: String,
        container: String,
        mode: PodStreamMode,
        previous: bool,
        timestamps: bool,
        tail_lines: u32,
        shell: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let connection = self.connection.clone();
        let req = PodStartRequest {
            cluster_id: cluster_id.clone(),
            session_id: session_id.clone(),
            namespace,
            name: pod_name,
            uid,
            container,
            mode,
            previous,
            timestamps,
            tail_lines,
            shell: if mode == PodStreamMode::Shell {
                Some(shell)
            } else {
                None
            },
        };

        cx.spawn_in(window, async move |this, cx| {
            let opened = match connection.pod_stream_open(req).await {
                Ok(snapshot) => snapshot,
                Err(err) => {
                    let _ = this.update_in(cx, |this, _, cx| {
                        if let Some(tab) = this
                            .pod_dock_tabs
                            .iter_mut()
                            .find(|t| t.id == placeholder_id)
                        {
                            tab.status = "error".into();
                            tab.output_lines.push(format!("Stream error: {err:?}"));
                        }
                        cx.notify();
                    });
                    return;
                }
            };

            let stream_id = opened.stream_id.clone();
            let initial_output = opened.output.clone();
            let initial_status = opened.status.to_string();

            let mut tab_present = false;
            let _ = this.update_in(cx, |this, _, cx| {
                if let Some(tab) = this
                    .pod_dock_tabs
                    .iter_mut()
                    .find(|t| t.id == placeholder_id)
                {
                    tab.id = stream_id.clone();
                    tab.status = initial_status;
                    if !initial_output.is_empty() {
                        for line in initial_output.lines() {
                            tab.output_lines.push(line.to_owned());
                        }
                    }
                    tab_present = true;
                }
                cx.notify();
            });

            if !tab_present {
                let _ = connection.pod_stream_close(PodStreamRequest {
                    cluster_id,
                    session_id,
                    stream_id,
                });
                return;
            }

            loop {
                tokio::time::sleep(Duration::from_millis(120)).await;
                let snapshot = match connection.pod_stream_read(PodStreamRequest {
                    cluster_id: cluster_id.clone(),
                    session_id: session_id.clone(),
                    stream_id: stream_id.clone(),
                }) {
                    Ok(snap) => snap,
                    Err(_) => break,
                };

                let is_done = matches!(snapshot.status, "exited" | "error");
                let mut still_open = false;
                let _ = this.update_in(cx, |this, _, cx| {
                    if let Some(tab) = this.pod_dock_tabs.iter_mut().find(|t| t.id == stream_id) {
                        still_open = true;
                        tab.status = snapshot.status.to_string();
                        if !tab.paused && !snapshot.output.is_empty() {
                            for line in snapshot.output.lines() {
                                tab.output_lines.push(line.to_owned());
                            }
                            if tab.output_lines.len() > 2500 {
                                let drain_count = tab.output_lines.len() - 2500;
                                tab.output_lines.drain(0..drain_count);
                            }
                        }
                        if let Some(err) = snapshot.error {
                            tab.output_lines.push(format!("[stream error: {err:?}]"));
                        } else if snapshot.status == "exited" {
                            tab.output_lines.push("[stream closed]".into());
                        }
                        cx.notify();
                    }
                });

                if !still_open || is_done {
                    break;
                }
            }
        })
        .detach();
    }

    pub(crate) fn close_pod_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.pod_dock_tabs.len() {
            return;
        }
        let removed = self.pod_dock_tabs.remove(index);
        if !removed.id.starts_with("opening-")
            && let Some(session) = &self.cluster_session
        {
            let _ = self.connection.pod_stream_close(PodStreamRequest {
                cluster_id: session.cluster_id.clone(),
                session_id: session.session_id.clone(),
                stream_id: removed.id,
            });
        }
        if self.pod_dock_tabs.is_empty() {
            self.active_pod_tab = None;
            self.pod_dock_open = false;
        } else {
            self.active_pod_tab = Some(index.min(self.pod_dock_tabs.len() - 1));
        }
        cx.notify();
    }

    pub(crate) fn send_pod_terminal_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active_idx) = self.active_pod_tab else {
            return;
        };
        let Some(tab) = self.pod_dock_tabs.get_mut(active_idx) else {
            return;
        };
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let input_text = self.pod_terminal_input.read(cx).value().to_string();
        if input_text.is_empty() || tab.id.starts_with("opening-") {
            return;
        }
        let stream_id = tab.id.clone();
        tab.output_lines.push(format!("$ {input_text}"));
        if tab.command_history.last() != Some(&input_text) {
            tab.command_history.push(input_text.clone());
            if tab.command_history.len() > 20 {
                tab.command_history.remove(0);
            }
        }
        self.pod_terminal_input
            .update(cx, |state, cx| state.set_value("", window, cx));

        let payload = format!("{input_text}\n");
        let _ = self.connection.pod_stream_write(PodInputRequest {
            stream: PodStreamRequest {
                cluster_id: session.cluster_id,
                session_id: session.session_id,
                stream_id,
            },
            data: payload,
        });
        cx.notify();
    }

    pub(crate) fn send_pod_terminal_raw(&mut self, raw: &str, label: &str, cx: &mut Context<Self>) {
        let Some(active_idx) = self.active_pod_tab else {
            return;
        };
        let Some(tab) = self.pod_dock_tabs.get_mut(active_idx) else {
            return;
        };
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        if tab.id.starts_with("opening-") {
            return;
        }
        let stream_id = tab.id.clone();
        if label == "clear" {
            tab.output_lines.clear();
        } else {
            tab.output_lines.push(format!("[sent {label}]"));
        }
        let _ = self.connection.pod_stream_write(PodInputRequest {
            stream: PodStreamRequest {
                cluster_id: session.cluster_id,
                session_id: session.session_id,
                stream_id,
            },
            data: raw.to_owned(),
        });
        cx.notify();
    }

    pub(crate) fn resize_active_pod_terminal(
        &mut self,
        cols: u16,
        rows: u16,
        cx: &mut Context<Self>,
    ) {
        let Some(active_idx) = self.active_pod_tab else {
            return;
        };
        let Some(tab) = self.pod_dock_tabs.get_mut(active_idx) else {
            return;
        };
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        tab.cols = cols;
        tab.rows = rows;
        if !tab.id.starts_with("opening-") {
            let _ = self.connection.pod_stream_resize(PodResizeRequest {
                stream: PodStreamRequest {
                    cluster_id: session.cluster_id,
                    session_id: session.session_id,
                    stream_id: tab.id.clone(),
                },
                cols,
                rows,
            });
        }
        cx.notify();
    }

    pub(crate) fn start_pod_forward(
        &mut self,
        namespace: String,
        pod_name: String,
        uid: String,
        remote_port: u16,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let connection = self.connection.clone();
        let req = PodForwardRequest {
            cluster_id: session.cluster_id,
            session_id: session.session_id,
            namespace,
            name: pod_name,
            uid,
            remote_port,
            local_port: 0,
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = connection.pod_forward_start(req).await;
            let _ = this.update_in(cx, |this, _, cx| {
                match result {
                    Ok(fwd) => {
                        this.port_forwards.push(fwd);
                        this.pod_dock_open = true;
                    }
                    Err(err) => {
                        this.cluster_error = Some(format!("Port-forward failed: {err:?}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn stop_pod_forward(&mut self, id: String, cx: &mut Context<Self>) {
        if let Some(session) = &self.cluster_session {
            let _ = self.connection.pod_forward_stop(PodForwardStopRequest {
                cluster_id: session.cluster_id.clone(),
                session_id: session.session_id.clone(),
                id: id.clone(),
            });
        }
        self.port_forwards.retain(|f| f.id != id);
        cx.notify();
    }

    pub(crate) fn render_pod_dock_panel(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active_idx = self.active_pod_tab.unwrap_or(0);
        let active_tab = self.pod_dock_tabs.get(active_idx).cloned();
        let forwards = self.port_forwards.clone();
        let log_filter = self
            .pod_log_filter_input
            .read(cx)
            .value()
            .trim()
            .to_lowercase();

        v_flex()
            .w_full()
            .h(px(300.0))
            .flex_shrink_0()
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .overflow_hidden()
            .child(
                h_flex()
                    .w_full()
                    .h(px(36.0))
                    .px_3()
                    .justify_between()
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().table_head)
                    .child(
                        h_flex()
                            .gap_1p5()
                            .items_center()
                            .overflow_x_scrollbar()
                            .children(self.pod_dock_tabs.iter().enumerate().map(|(idx, tab)| {
                                let selected = Some(idx) == self.active_pod_tab;
                                let status_dot = match tab.status.as_str() {
                                    "running" => "●",
                                    "opening" => "◐",
                                    "error" => "✕",
                                    _ => "○",
                                };
                                h_flex()
                                    .gap_1()
                                    .items_center()
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "pod-dock-tab-{idx}"
                                        )))
                                        .xsmall()
                                        .ghost()
                                        .selected(selected)
                                        .label(format!(
                                            "{status_dot} [{}] {}:{}",
                                            tab.mode_label(),
                                            tab.pod_name,
                                            tab.container
                                        ))
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.active_pod_tab = Some(idx);
                                                cx.notify();
                                            }),
                                        ),
                                    )
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "pod-dock-close-{idx}"
                                        )))
                                        .xsmall()
                                        .ghost()
                                        .icon(IconName::Close)
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.close_pod_tab(idx, cx);
                                            }),
                                        ),
                                    )
                            })),
                    )
                    .child(
                        h_flex()
                            .gap_1p5()
                            .items_center()
                            .children(forwards.into_iter().map(|fwd| {
                                let fid = fwd.id.clone();
                                let copy_fid = fwd.id.clone();
                                let local_addr = format!("localhost:{}", fwd.local_port);
                                h_flex()
                                    .gap_1()
                                    .items_center()
                                    .child(Tag::success().small().child(format!(
                                        "localhost:{} → {}:{}",
                                        fwd.local_port, fwd.name, fwd.remote_port
                                    )))
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "copy-fwd-{copy_fid}"
                                        )))
                                        .xsmall()
                                        .ghost()
                                        .label("Copy")
                                        .on_click(
                                            cx.listener(move |_, _, _, cx| {
                                                cx.write_to_clipboard(ClipboardItem::new_string(
                                                    local_addr.clone(),
                                                ));
                                            }),
                                        ),
                                    )
                                    .child(
                                        Button::new(SharedString::from(format!("stop-fwd-{fid}")))
                                            .xsmall()
                                            .ghost()
                                            .icon(IconName::Close)
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.stop_pod_forward(fid.clone(), cx);
                                            })),
                                    )
                            }))
                            .child(
                                Button::new("collapse-pod-dock-btn")
                                    .xsmall()
                                    .ghost()
                                    .icon(IconName::ChevronDown)
                                    .label("Hide Dock")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.pod_dock_open = false;
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .when_some(active_tab.clone(), |el, tab| {
                let tab_idx = active_idx;
                let containers = tab.available_containers.clone();
                let current_container = tab.container.clone();
                let is_logs = tab.mode == PodStreamMode::Logs;
                let is_shell = tab.mode == PodStreamMode::Shell;
                let is_interactive =
                    matches!(tab.mode, PodStreamMode::Shell | PodStreamMode::Attach);
                let prev_enabled = tab.previous;
                let ts_enabled = tab.timestamps;
                let paused = tab.paused;
                let tail_lines = tab.tail_lines;
                let active_shell = tab.shell.clone();
                let active_cols = tab.cols;
                let active_rows = tab.rows;
                let copy_lines = tab.output_lines.join("\n");

                el.child(
                    h_flex()
                        .w_full()
                        .px_3()
                        .py_1p5()
                        .gap_2()
                        .justify_between()
                        .items_center()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().secondary)
                        .child(
                            h_flex()
                                .gap_1()
                                .items_center()
                                .child(
                                    match tab.status.as_str() {
                                        "running" => Tag::success().small(),
                                        "opening" => Tag::warning().small(),
                                        "error" => Tag::danger().small(),
                                        _ => Tag::secondary().small(),
                                    }
                                    .child(tab.status.clone()),
                                )
                                .when(containers.len() > 1, |row| {
                                    row.child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child("Container:"),
                                    )
                                    .children(
                                        containers.into_iter().map(|cname| {
                                            let selected = cname == current_container;
                                            let target_cname = cname.clone();
                                            Button::new(SharedString::from(format!(
                                                "dock-container-switch-{tab_idx}-{cname}"
                                            )))
                                            .xsmall()
                                            .ghost()
                                            .selected(selected)
                                            .label(cname)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                if let Some(t) = this.pod_dock_tabs.get_mut(tab_idx)
                                                {
                                                    t.container = target_cname.clone();
                                                }
                                                this.reopen_pod_tab(tab_idx, window, cx);
                                            }))
                                        }),
                                    )
                                }),
                        )
                        .when(is_logs, |bar| {
                            bar.child(
                                h_flex()
                                    .gap_1()
                                    .items_center()
                                    .child(
                                        div().w(px(190.0)).child(
                                            Input::new(&self.pod_log_filter_input)
                                                .small()
                                                .cleanable(true),
                                        ),
                                    )
                                    .child(
                                        Button::new("pod-log-pause-toggle")
                                            .xsmall()
                                            .ghost()
                                            .selected(!paused)
                                            .label(if paused { "Paused" } else { "Following" })
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if let Some(t) = this.pod_dock_tabs.get_mut(tab_idx)
                                                {
                                                    t.paused = !t.paused;
                                                    cx.notify();
                                                }
                                            })),
                                    )
                                    .child(
                                        Button::new("pod-log-timestamps-toggle")
                                            .xsmall()
                                            .ghost()
                                            .selected(ts_enabled)
                                            .label("Timestamps")
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                if let Some(t) = this.pod_dock_tabs.get_mut(tab_idx)
                                                {
                                                    t.timestamps = !t.timestamps;
                                                }
                                                this.reopen_pod_tab(tab_idx, window, cx);
                                            })),
                                    )
                                    .child(
                                        Button::new("pod-log-previous-toggle")
                                            .xsmall()
                                            .ghost()
                                            .selected(prev_enabled)
                                            .label("Previous")
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                if let Some(t) = this.pod_dock_tabs.get_mut(tab_idx)
                                                {
                                                    t.previous = !t.previous;
                                                }
                                                this.reopen_pod_tab(tab_idx, window, cx);
                                            })),
                                    )
                                    .children(TAIL_LINE_OPTIONS.iter().copied().map(|lines| {
                                        Button::new(SharedString::from(format!(
                                            "pod-log-tail-{lines}"
                                        )))
                                        .xsmall()
                                        .ghost()
                                        .selected(tail_lines == lines)
                                        .label(format!("{lines}L"))
                                        .on_click(
                                            cx.listener(move |this, _, window, cx| {
                                                if let Some(t) = this.pod_dock_tabs.get_mut(tab_idx)
                                                {
                                                    t.tail_lines = lines;
                                                }
                                                this.reopen_pod_tab(tab_idx, window, cx);
                                            }),
                                        )
                                    }))
                                    .child(
                                        Button::new("pod-log-copy-btn")
                                            .xsmall()
                                            .outline()
                                            .label("Copy")
                                            .on_click(cx.listener(move |_, _, _, cx| {
                                                cx.write_to_clipboard(ClipboardItem::new_string(
                                                    copy_lines.clone(),
                                                ));
                                            })),
                                    )
                                    .child(
                                        Button::new("pod-log-clear-btn")
                                            .xsmall()
                                            .ghost()
                                            .label("Clear")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if let Some(t) = this.pod_dock_tabs.get_mut(tab_idx)
                                                {
                                                    t.output_lines.clear();
                                                    cx.notify();
                                                }
                                            })),
                                    )
                                    .child(
                                        Button::new("pod-log-reconnect-btn")
                                            .xsmall()
                                            .ghost()
                                            .icon(IconName::Undo2)
                                            .label("Reconnect")
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.reopen_pod_tab(tab_idx, window, cx);
                                            })),
                                    ),
                            )
                        })
                        .when(is_interactive, |bar| {
                            bar.child(
                                h_flex()
                                    .gap_1()
                                    .items_center()
                                    .when(is_shell, |sbar| {
                                        sbar.children(["sh", "bash", "ash"].into_iter().map(
                                            |sh_label| {
                                                let sh_path = shell_option_path(sh_label);
                                                let selected = active_shell == sh_path;
                                                let target_sh = sh_path.to_owned();
                                                Button::new(SharedString::from(format!(
                                                    "pod-shell-opt-{sh_label}"
                                                )))
                                                .xsmall()
                                                .ghost()
                                                .selected(selected)
                                                .label(sh_label)
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        if let Some(t) =
                                                            this.pod_dock_tabs.get_mut(tab_idx)
                                                        {
                                                            t.shell = target_sh.clone();
                                                        }
                                                        this.reopen_pod_tab(tab_idx, window, cx);
                                                    },
                                                ))
                                            },
                                        ))
                                    })
                                    .children(TERMINAL_SIZE_PRESETS.iter().copied().map(
                                        |(cols, rows, label)| {
                                            Button::new(SharedString::from(format!(
                                                "pod-pty-size-{cols}x{rows}"
                                            )))
                                            .xsmall()
                                            .ghost()
                                            .selected(active_cols == cols && active_rows == rows)
                                            .label(label)
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.resize_active_pod_terminal(cols, rows, cx);
                                            }))
                                        },
                                    ))
                                    .child(
                                        Button::new("pod-term-sigint")
                                            .xsmall()
                                            .outline()
                                            .label("Ctrl+C")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.send_pod_terminal_raw("\x03", "Ctrl+C", cx);
                                            })),
                                    )
                                    .child(
                                        Button::new("pod-term-eof")
                                            .xsmall()
                                            .outline()
                                            .label("Ctrl+D")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.send_pod_terminal_raw("\x04", "Ctrl+D", cx);
                                            })),
                                    )
                                    .child(
                                        Button::new("pod-term-clear")
                                            .xsmall()
                                            .ghost()
                                            .label("Clear")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.send_pod_terminal_raw("\x0c", "clear", cx);
                                            })),
                                    )
                                    .child(
                                        Button::new("pod-term-reconnect")
                                            .xsmall()
                                            .ghost()
                                            .icon(IconName::Undo2)
                                            .label("Reconnect")
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.reopen_pod_tab(tab_idx, window, cx);
                                            })),
                                    ),
                            )
                        }),
                )
            })
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .p_3()
                    .gap_0p5()
                    .overflow_y_scrollbar()
                    .children(
                        active_tab
                            .as_ref()
                            .map(|t| {
                                t.output_lines
                                    .iter()
                                    .filter(|line| {
                                        t.mode != PodStreamMode::Logs
                                            || log_filter.is_empty()
                                            || line.to_lowercase().contains(&log_filter)
                                    })
                                    .cloned()
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default()
                            .into_iter()
                            .map(|line| {
                                let is_err = line.contains("ERROR")
                                    || line.contains("error")
                                    || line.starts_with("[stream error");
                                div()
                                    .text_xs()
                                    .text_color(if is_err {
                                        cx.theme().danger
                                    } else {
                                        cx.theme().foreground
                                    })
                                    .child(line)
                            }),
                    ),
            )
            .when_some(
                active_tab
                    .filter(|t| matches!(t.mode, PodStreamMode::Shell | PodStreamMode::Attach)),
                |el, tab| {
                    let recent_cmds: Vec<String> =
                        tab.command_history.iter().rev().take(4).cloned().collect();
                    el.child(
                        h_flex()
                            .w_full()
                            .px_3()
                            .py_2()
                            .gap_2()
                            .items_center()
                            .border_t_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().secondary)
                            .child(
                                Tag::secondary()
                                    .small()
                                    .child(format!("{}$", tab.container)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .child(Input::new(&self.pod_terminal_input).small()),
                            )
                            .children(recent_cmds.into_iter().enumerate().map(|(i, cmd)| {
                                let run_cmd = cmd.clone();
                                Button::new(SharedString::from(format!("pod-hist-cmd-{i}")))
                                    .xsmall()
                                    .ghost()
                                    .label(cmd)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.pod_terminal_input.update(cx, |state, cx| {
                                            state.set_value(run_cmd.clone(), window, cx);
                                        });
                                        this.send_pod_terminal_input(window, cx);
                                    }))
                            }))
                            .child(
                                Button::new("pod-terminal-send-btn")
                                    .xsmall()
                                    .primary()
                                    .label("Send")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.send_pod_terminal_input(window, cx);
                                    })),
                            ),
                    )
                },
            )
    }
}
