//! Pod Operational Dock: resizable bottom panel hosting up to 12 concurrent Pod streams
//! (`Logs`, `Shell` exec, `Attach`) and active port-forwards.

use std::time::Duration;

use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    scroll::ScrollableElement as _,
    tag::Tag,
    v_flex, IconName,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::IceonApp;
use crate::navigation::connection_error_message;
use iceon_core::connection::pod_runtime::{
    PodForward, PodForwardRequest, PodForwardStopRequest, PodInputRequest, PodStartRequest,
    PodStreamMode, PodStreamRequest,
};

#[derive(Clone)]
pub struct PodDockTab {
    pub stream_id: String,
    pub cluster_id: String,
    pub session_id: String,
    pub namespace: String,
    pub pod_name: String,
    pub container: String,
    pub mode: PodStreamMode,
    pub buffer: String,
    pub status: &'static str,
    pub error: Option<String>,
    pub truncated: bool,
}

impl IceonApp {
    pub(crate) fn open_pod_stream_tab(
        &mut self,
        namespace: String,
        pod_name: String,
        uid: String,
        container: String,
        mode: PodStreamMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let engine = self.connection.clone();
        self.pod_dock_open = true;
        cx.spawn_in(window, async move |this, cx| {
            let res = engine
                .pod_stream_open(PodStartRequest {
                    cluster_id: session.cluster_id.clone(),
                    session_id: session.session_id.clone(),
                    namespace: namespace.clone(),
                    name: pod_name.clone(),
                    uid,
                    container: container.clone(),
                    mode,
                    previous: false,
                    timestamps: true,
                    tail_lines: 500,
                    shell: (mode == PodStreamMode::Shell).then(|| "/bin/sh".into()),
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                match res {
                    Ok(snap) => {
                        let stream_id = snap.stream_id.clone();
                        this.pod_dock_tabs.push(PodDockTab {
                            stream_id: stream_id.clone(),
                            cluster_id: session.cluster_id,
                            session_id: session.session_id,
                            namespace,
                            pod_name,
                            container,
                            mode,
                            buffer: snap.output,
                            status: snap.status,
                            error: snap.error.map(|e| connection_error_message(e).into()),
                            truncated: snap.truncated,
                        });
                        this.active_pod_tab = Some(this.pod_dock_tabs.len() - 1);
                        this.spawn_pod_stream_poller(stream_id, window, cx);
                    }
                    Err(err) => {
                        this.cluster_error = Some(connection_error_message(err).into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn spawn_pod_stream_poller(
        &mut self,
        stream_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let engine = self.connection.clone();
        cx.spawn_in(window, async move |this, cx| {
            loop {
                tokio::time::sleep(Duration::from_millis(250)).await;
                let keep_going = this
                    .update(cx, |this, cx| {
                        let Some(tab) = this
                            .pod_dock_tabs
                            .iter_mut()
                            .find(|t| t.stream_id == stream_id)
                        else {
                            return false;
                        };
                        match engine.pod_stream_read(PodStreamRequest {
                            cluster_id: tab.cluster_id.clone(),
                            session_id: tab.session_id.clone(),
                            stream_id: tab.stream_id.clone(),
                        }) {
                            Ok(snap) => {
                                if !snap.output.is_empty() {
                                    tab.buffer.push_str(&snap.output);
                                    if tab.buffer.len() > 256 * 1024 {
                                        let drain = tab.buffer.len() - 256 * 1024;
                                        let mut idx = drain;
                                        while !tab.buffer.is_char_boundary(idx) {
                                            idx += 1;
                                        }
                                        tab.buffer.drain(..idx);
                                    }
                                    cx.notify();
                                }
                                tab.status = snap.status;
                                tab.truncated |= snap.truncated;
                                if let Some(err) = snap.error {
                                    tab.error = Some(connection_error_message(err).into());
                                    cx.notify();
                                }
                                snap.status == "opening" || snap.status == "running"
                            }
                            Err(_) => false,
                        }
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        })
        .detach();
    }

    pub(crate) fn send_pod_terminal_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(idx) = self.active_pod_tab else {
            return;
        };
        let Some(tab) = self.pod_dock_tabs.get(idx).cloned() else {
            return;
        };
        let line = self.pod_terminal_input.read(cx).value().to_string();
        if line.is_empty() {
            return;
        }
        let payload = format!("{line}\n");
        let _ = self.connection.pod_stream_write(PodInputRequest {
            stream: PodStreamRequest {
                cluster_id: tab.cluster_id,
                session_id: tab.session_id,
                stream_id: tab.stream_id,
            },
            data: payload,
        });
        self.pod_terminal_input
            .update(cx, |s, cx| s.set_value(String::new(), window, cx));
        cx.notify();
    }

    pub(crate) fn close_pod_stream_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.pod_dock_tabs.len() {
            return;
        }
        let removed = self.pod_dock_tabs.remove(index);
        let _ = self.connection.pod_stream_close(PodStreamRequest {
            cluster_id: removed.cluster_id,
            session_id: removed.session_id,
            stream_id: removed.stream_id,
        });
        if self.pod_dock_tabs.is_empty() {
            self.active_pod_tab = None;
            self.pod_dock_open = false;
        } else {
            self.active_pod_tab = Some(index.saturating_sub(1));
        }
        cx.notify();
    }

    pub(crate) fn start_pod_port_forward(
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
        let engine = self.connection.clone();
        cx.spawn_in(window, async move |this, cx| {
            let res = engine
                .pod_forward_start(PodForwardRequest {
                    cluster_id: session.cluster_id,
                    session_id: session.session_id,
                    namespace,
                    name: pod_name,
                    uid,
                    remote_port,
                    local_port: 0,
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                match res {
                    Ok(fwd) => {
                        this.inspector.status_banner = Some(format!(
                            "Port forward active: 127.0.0.1:{} -> {}:{}",
                            fwd.local_port, fwd.name, fwd.remote_port
                        ));
                        this.refresh_port_forwards(cx);
                    }
                    Err(err) => {
                        this.inspector.error = Some(connection_error_message(err).into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn refresh_port_forwards(&mut self, cx: &mut Context<Self>) {
        let Some(session) = &self.cluster_session else {
            self.port_forwards.clear();
            return;
        };
        if let Ok(list) = self
            .connection
            .pod_forward_list(iceon_core::connection::SessionRequest {
                cluster_id: session.cluster_id.clone(),
                session_id: session.session_id.clone(),
            })
        {
            self.port_forwards = list;
        }
        cx.notify();
    }

    pub(crate) fn stop_port_forward(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(session) = &self.cluster_session else {
            return;
        };
        let _ = self.connection.pod_forward_stop(PodForwardStopRequest {
            cluster_id: session.cluster_id.clone(),
            session_id: session.session_id.clone(),
            id,
        });
        self.refresh_port_forwards(cx);
    }

    pub(crate) fn render_pod_dock_panel(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active_idx = self.active_pod_tab.unwrap_or(0);
        let active_tab = self.pod_dock_tabs.get(active_idx).cloned();
        let forwards: Vec<PodForward> = self.port_forwards.clone();

        v_flex()
            .size_full()
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_1p5()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().secondary)
                    .items_center()
                    .justify_between()
                    .child(
                        h_flex()
                            .gap_1p5()
                            .items_center()
                            .overflow_x_scrollbar()
                            .children(self.pod_dock_tabs.iter().enumerate().map(|(idx, tab)| {
                                let selected = idx == active_idx;
                                let mode_str = match tab.mode {
                                    PodStreamMode::Logs => "Logs",
                                    PodStreamMode::Shell => "Shell",
                                    PodStreamMode::Attach => "Attach",
                                };
                                h_flex()
                                    .gap_1()
                                    .items_center()
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "dock-tab-{idx}"
                                        )))
                                        .xsmall()
                                        .when(selected, |b| b.primary())
                                        .when(!selected, |b| b.outline())
                                        .label(format!(
                                            "{mode_str}: {}/{}",
                                            tab.pod_name, tab.container
                                        ))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.active_pod_tab = Some(idx);
                                            cx.notify();
                                        })),
                                    )
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "dock-close-{idx}"
                                        )))
                                        .xsmall()
                                        .ghost()
                                        .icon(IconName::Close)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.close_pod_stream_tab(idx, cx);
                                        })),
                                    )
                            }))
                            .when(!forwards.is_empty(), |el| {
                                el.child(
                                    Tag::success()
                                        .child(format!("{} Port Forward(s)", forwards.len())),
                                )
                            }),
                    )
                    .child(
                        Button::new("hide-pod-dock-btn")
                            .xsmall()
                            .ghost()
                            .icon(IconName::ChevronDown)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.pod_dock_open = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(if let Some(tab) = active_tab {
                let interactive = tab.mode != PodStreamMode::Logs;
                v_flex()
                    .flex_1()
                    .w_full()
                    .overflow_hidden()
                    .child(
                        v_flex()
                            .flex_1()
                            .w_full()
                            .p_3()
                            .font_family("RobotoMono, monospace")
                            .text_xs()
                            .overflow_y_scrollbar()
                            .child(if tab.buffer.is_empty() {
                                format!("({} stream {}...)", tab.pod_name, tab.status)
                            } else {
                                tab.buffer
                            }),
                    )
                    .when(interactive, |el| {
                        el.child(
                            h_flex()
                                .w_full()
                                .px_3()
                                .py_2()
                                .gap_2()
                                .border_t_1()
                                .border_color(cx.theme().border)
                                .items_center()
                                .child(
                                    div()
                                        .flex_1()
                                        .child(Input::new(&self.pod_terminal_input).small()),
                                )
                                .child(
                                    Button::new("send-pod-stdin-btn")
                                        .small()
                                        .primary()
                                        .label("Send")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.send_pod_terminal_input(window, cx);
                                        })),
                                ),
                        )
                    })
                    .into_any_element()
            } else {
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Select a Pod container action (Logs, Shell, Attach) to open a stream.")
                    .into_any_element()
            })
    }
}
