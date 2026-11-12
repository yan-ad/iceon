//! Bottom operational dock for Pod Logs, Interactive Exec Shell, Attach, and Port-Forwards.

use std::{rc::Rc, time::Duration};

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    menu::{DropdownMenu as _, PopupMenuItem},
    select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState},
    tab::{Tab, TabBar},
    tag::Tag,
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use iceon_core::connection::{
    ResourceKind,
    permissions::PermissionAction,
    pod_runtime::{
        PodForwardRequest, PodForwardStopRequest, PodInputRequest, PodResizeRequest,
        PodStartRequest, PodStreamMode, PodStreamRequest,
    },
    pods::PodRequest,
};

use crate::IceonApp;
use crate::terminal_surface::{TerminalEvent, TerminalSurface};

#[derive(Clone)]
pub(crate) struct PodTarget {
    pub namespace: String,
    pub name: String,
    pub uid: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ContainerChoice(String);
impl SelectItem for ContainerChoice {
    type Value = String;
    fn title(&self) -> SharedString {
        self.0.clone().into()
    }
    fn value(&self) -> &Self::Value {
        &self.0
    }
    fn matches(&self, query: &str) -> bool {
        self.0.to_lowercase().contains(&query.to_lowercase())
    }
}
type ContainerSelectState = SelectState<SearchableVec<ContainerChoice>>;

#[derive(Clone)]
pub struct PodDockTab {
    pub id: String,
    // UI identity is independent of the backend ID (opening/reconnect replace it).
    pub(crate) key: u64,
    pub pod_name: String,
    pub namespace: String,
    pub uid: String,
    pub container: String,
    pub available_containers: Vec<String>,
    pub mode: PodStreamMode,
    pub status: String,
    pub output_lines: Rc<Vec<String>>,
    pub previous: bool,
    pub timestamps: bool,
    pub tail_lines: u32,
    pub paused: bool,
    pub shell: String,
    pub cols: u16,
    pub rows: u16,
    pub command_history: Vec<String>,
    pub(crate) terminal: Option<Entity<TerminalSurface>>,
    pub log_scroll: UniformListScrollHandle,
    pub follow: bool,
    pub word_wrap: bool,
    pub log_pending: Rc<String>,
    container_select: Option<Entity<ContainerSelectState>>,
    log_columns: usize,
    presentation: Option<LogPresentation>,
    filter_observer: Option<Rc<Subscription>>,
}

impl std::fmt::Debug for PodDockTab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PodDockTab")
            .field("id", &self.id)
            .field("key", &self.key)
            .field("pod_name", &self.pod_name)
            .field("container", &self.container)
            .field("mode", &self.mode)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
struct LogPresentation {
    filter: String,
    timezone: String,
    columns: Option<usize>,
    rows: Rc<Vec<SharedString>>,
}

/// Bound visible labels by Unicode scalar count, keeping the shell/container suffix.
/// Full names remain available in the tab tooltip and accessibility label.
fn console_tab_title(title: &str) -> String {
    const MAX_CHARS: usize = 36;
    let chars = title.chars().collect::<Vec<_>>();
    if chars.len() <= MAX_CHARS {
        return title.to_owned();
    }
    chars[..23]
        .iter()
        .chain(std::iter::once(&'…'))
        .chain(chars[chars.len() - 12..].iter())
        .collect()
}

// Output-only terminal updates belong to TerminalSurface, not the workspace.
fn poll_needs_update(
    output: &str,
    old_status: &str,
    status: &str,
    old_truncated: bool,
    truncated: bool,
) -> bool {
    !output.is_empty() || old_status != status || old_truncated != truncated
}

fn poll_notifies_parent(
    status_changed: bool,
    logs: bool,
    active: bool,
    output_changed: bool,
) -> bool {
    status_changed || (logs && active && output_changed)
}

fn active_after_close(active: Option<usize>, closed: usize, remaining: usize) -> Option<usize> {
    if remaining == 0 {
        return None;
    }
    active.map(|active| {
        if active > closed {
            active - 1
        } else {
            active.min(remaining - 1)
        }
    })
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

fn stream_permission(mode: PodStreamMode) -> PermissionAction {
    match mode {
        PodStreamMode::Logs => PermissionAction::Logs,
        PodStreamMode::Shell => PermissionAction::Exec,
        PodStreamMode::Attach => PermissionAction::Attach,
    }
}

impl IceonApp {
    /// Opens a Pod Logs or Shell stream directly from a Pod table row or collection item
    /// by resolving the Pod's UID and container list first via `pod_detail`.
    pub(crate) fn can_pod_stream(&self, namespace: &str, name: &str, mode: PodStreamMode) -> bool {
        self.can_action(
            ResourceKind::Pods,
            PermissionAction::Get,
            Some(namespace),
            Some(name),
        ) && self.can_action(
            ResourceKind::Pods,
            stream_permission(mode),
            Some(namespace),
            Some(name),
        )
    }
    fn review_pod_stream_access(
        &mut self,
        namespace: &str,
        name: &str,
        mode: PodStreamMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        for action in [PermissionAction::Get, stream_permission(mode)] {
            self.review_resource_access(
                ResourceKind::Pods,
                action,
                Some(namespace),
                Some(name),
                window,
                cx,
            );
        }
        self.can_pod_stream(namespace, name, mode)
    }

    pub(crate) fn open_pod_stream_for_pod(
        &mut self,
        namespace: String,
        pod_name: String,
        mode: PodStreamMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.review_pod_stream_access(&namespace, &pod_name, mode, window, cx) {
            return;
        }
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
            cluster_id: session.cluster_id.clone(),
            session_id: session.session_id.clone(),
            namespace: namespace.clone(),
            name: pod_name.clone(),
            uid: None,
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = connection.pod_detail(req).await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(pod) => {
                    if !this.cluster_session.as_ref().is_some_and(|current| {
                        current.cluster_id == session.cluster_id
                            && current.session_id == session.session_id
                    }) {
                        return;
                    }
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
        target: PodTarget,
        container: String,
        mode: PodStreamMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let PodTarget {
            namespace,
            name: pod_name,
            uid,
        } = target;
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
        if !self.review_pod_stream_access(&namespace, &pod_name, mode, window, cx) {
            return;
        }
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        if self.pod_dock_tabs.len() + self.device_terminals.len() + self.yaml_dock_tabs.len() >= 12
        {
            self.cluster_error =
                Some("Close a pod console before opening another (maximum 12).".into());
            cx.notify();
            return;
        }
        let key = self.next_operation_id();
        let placeholder_id = format!("opening-{key}");
        // An empty default preserves the backend's /bin/sh fallback. Send an
        // explicit saved path unchanged: unsupported paths produce a real
        // backend InvalidInput error rather than silently launching another shell.
        let default_shell = if self.preferences_draft.terminal.shell_path.trim().is_empty() {
            shell_option_path("sh").to_string()
        } else {
            self.preferences_draft.terminal.shell_path.trim().to_owned()
        };

        let tab = PodDockTab {
            id: placeholder_id.clone(),
            key,
            pod_name: pod_name.clone(),
            namespace: namespace.clone(),
            uid: uid.clone(),
            container: container.clone(),
            available_containers,
            mode,
            status: "opening".into(),
            output_lines: Rc::new(Vec::new()),
            previous: false,
            timestamps: true,
            tail_lines: 500,
            paused: false,
            shell: default_shell.clone(),
            cols: 120,
            rows: 36,
            command_history: vec![],
            // Match the transport's requested initial PTY size.  The surface
            // must not start at its fallback 80x24 grid: the first prompt can
            // arrive before the first layout callback and otherwise gets
            // rendered using a stale grid until the deferred resize runs.
            terminal: Some(cx.new(|cx| TerminalSurface::with_size(120, 36, cx))),
            log_scroll: UniformListScrollHandle::new(),
            follow: true,
            word_wrap: false,
            log_pending: Rc::new(String::new()),
            container_select: None,
            log_columns: 120,
            presentation: None,
            filter_observer: None,
        };
        self.pod_dock_tabs.push(tab);
        // Keep the subscription with the tab: closing it releases the observer.
        // Editing the filter redraws only the selected log view, never restarts a stream.
        let observer = cx.observe(&self.pod_log_filter_input, move |this, _, cx| {
            if this
                .active_pod_tab
                .and_then(|index| this.pod_dock_tabs.get(index))
                .is_some_and(|tab| tab.key == key && tab.mode == PodStreamMode::Logs)
            {
                cx.notify();
            }
        });
        self.pod_dock_tabs.last_mut().unwrap().filter_observer = Some(Rc::new(observer));
        let tab_index = self.pod_dock_tabs.len() - 1;
        if let Some(terminal) = self.pod_dock_tabs[tab_index].terminal.clone() {
            cx.subscribe_in(
                &terminal,
                window,
                move |this, entity, event: &TerminalEvent, window, cx| {
                    let Some(idx) = this
                        .pod_dock_tabs
                        .iter()
                        .position(|tab| tab.terminal.as_ref() == Some(entity))
                    else {
                        return;
                    };
                    if let TerminalEvent::Resize { cols, rows } = event {
                        this.pod_dock_tabs[idx].cols = *cols;
                        this.pod_dock_tabs[idx].rows = *rows;
                    }
                    let Some(session) = this.cluster_session.clone() else {
                        return;
                    };
                    let Some(stream_id) = this.pod_dock_tabs.get(idx).map(|tab| tab.id.clone())
                    else {
                        return;
                    };
                    if stream_id.starts_with("opening-") {
                        return;
                    }
                    match event {
                        TerminalEvent::Input(data) => {
                            let connection = this.connection.clone();
                            let request = PodInputRequest {
                                stream: PodStreamRequest {
                                    cluster_id: session.cluster_id,
                                    session_id: session.session_id,
                                    stream_id,
                                },
                                data: data.clone(),
                            };
                            cx.spawn_in(window, async move |_, _| {
                                let _ = connection.pod_stream_write(request);
                            })
                            .detach();
                        }
                        TerminalEvent::Resize { cols, rows } => {
                            let connection = this.connection.clone();
                            let request = PodResizeRequest {
                                stream: PodStreamRequest {
                                    cluster_id: session.cluster_id,
                                    session_id: session.session_id,
                                    stream_id,
                                },
                                cols: *cols,
                                rows: *rows,
                            };
                            cx.spawn_in(window, async move |_, _| {
                                let _ = connection.pod_stream_resize(request);
                            })
                            .detach();
                        }
                    }
                },
            )
            .detach();
        }
        self.active_pod_tab = Some(tab_index);
        self.active_yaml_tab = None;
        self.local_terminal_active = false;
        self.pod_dock_open = true;
        if mode != PodStreamMode::Logs
            && let Some(terminal) = &self.pod_dock_tabs[tab_index].terminal
        {
            window.focus(&terminal.focus_handle(cx), cx);
        }
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
        if !self.review_pod_stream_access(
            &existing.namespace,
            &existing.pod_name,
            existing.mode,
            window,
            cx,
        ) {
            return;
        }
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
            Rc::make_mut(&mut tab.log_pending).clear();
            if let Some(terminal) = &tab.terminal {
                terminal.update(cx, |terminal, cx| terminal.reset(cx));
            }
            Rc::make_mut(&mut tab.output_lines).clear();
            tab.presentation = None;
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
        if !self.review_pod_stream_access(&namespace, &pod_name, mode, window, cx) {
            return;
        }
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
                            tab.status = format!("error: {err:?}");
                        }
                        cx.notify();
                    });
                    return;
                }
            };

            let stream_id = opened.stream_id.clone();
            let initial_output = opened.output;
            let initial_status = opened.status.to_string();

            let mut tab_present = false;
            let _ = this.update_in(cx, |this, _, cx| {
                if !this.cluster_session.as_ref().is_some_and(|session| {
                    session.cluster_id == cluster_id && session.session_id == session_id
                }) {
                    return;
                }
                if let Some(tab) = this
                    .pod_dock_tabs
                    .iter_mut()
                    .find(|t| t.id == placeholder_id)
                {
                    tab.id = stream_id.clone();
                    tab.status = initial_status;
                    if !initial_output.is_empty() {
                        feed_tab_output(tab, &initial_output, cx);
                    }
                    if mode != PodStreamMode::Logs {
                        let (cols, rows) = tab
                            .terminal
                            .as_ref()
                            .map(|terminal| terminal.read(cx).size())
                            .unwrap_or((tab.cols, tab.rows));
                        let _ = connection.pod_stream_resize(PodResizeRequest {
                            stream: PodStreamRequest {
                                cluster_id: cluster_id.clone(),
                                session_id: session_id.clone(),
                                stream_id: stream_id.clone(),
                            },
                            cols,
                            rows,
                        });
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

            let mut last_truncated = opened.truncated;
            loop {
                tokio::time::sleep(Duration::from_millis(120)).await;
                // Do not drain the backend ring while paused: resume reads the retained
                // bounded backlog, rather than silently discarding every paused chunk.
                let Ok(Some(paused)) = this.read_with(cx, |this, _| {
                    this.pod_dock_tabs
                        .iter()
                        .find(|tab| tab.id == stream_id)
                        .map(|tab| tab.mode == PodStreamMode::Logs && tab.paused)
                }) else {
                    break;
                };
                if paused {
                    continue;
                }
                let snapshot = match connection.pod_stream_read(PodStreamRequest {
                    cluster_id: cluster_id.clone(),
                    session_id: session_id.clone(),
                    stream_id: stream_id.clone(),
                }) {
                    Ok(snap) => snap,
                    Err(_) => break,
                };

                let is_done = matches!(snapshot.status, "exited" | "error");
                let status = snapshot
                    .error
                    .as_ref()
                    .map(|error| format!("error: {error:?}"))
                    .unwrap_or_else(|| snapshot.status.to_owned());
                let truncated = snapshot.truncated;
                let Ok(Some(needs_update)) = this.read_with(cx, |this, _| {
                    this.pod_dock_tabs
                        .iter()
                        .find(|tab| tab.id == stream_id)
                        .map(|tab| {
                            poll_needs_update(
                                &snapshot.output,
                                &tab.status,
                                &status,
                                last_truncated,
                                truncated,
                            )
                        })
                }) else {
                    break;
                };
                if !needs_update && !is_done {
                    continue;
                }
                let updated = this.update_in(cx, |this, _, cx| {
                    let active_key = this
                        .active_pod_tab
                        .and_then(|index| this.pod_dock_tabs.get(index))
                        .map(|tab| tab.key);
                    let Some(tab) = this
                        .pod_dock_tabs
                        .iter_mut()
                        .find(|tab| tab.id == stream_id)
                    else {
                        return false;
                    };
                    let status_changed = tab.status != status || last_truncated != truncated;
                    tab.status = status;
                    let output_changed = !snapshot.output.is_empty();
                    if output_changed {
                        feed_tab_output(tab, &snapshot.output, cx);
                    }
                    if poll_notifies_parent(
                        status_changed,
                        tab.mode == PodStreamMode::Logs,
                        active_key == Some(tab.key),
                        output_changed,
                    ) {
                        if let Some(dock) = &this.workspace_dock {
                            dock.notify_console(cx);
                        } else {
                            cx.notify();
                        }
                    }
                    true
                });
                last_truncated = truncated;
                if !matches!(updated, Ok(true)) || is_done {
                    break;
                }
            }
            // Idempotent close also covers dropped owners and failed polling updates.
            let _ = connection.pod_stream_close(PodStreamRequest {
                cluster_id,
                session_id,
                stream_id,
            });
        })
        .detach();
    }

    fn duplicate_pod_console(
        &mut self,
        key: u64,
        mode: Option<PodStreamMode>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self
            .pod_dock_tabs
            .iter()
            .find(|tab| tab.key == key)
            .cloned()
        else {
            return;
        };
        self.open_pod_stream_with_containers(
            tab.namespace,
            tab.pod_name,
            tab.uid,
            tab.container,
            tab.available_containers,
            mode.unwrap_or(tab.mode),
            window,
            cx,
        );
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
            self.local_terminal_active = true;
        } else {
            self.active_pod_tab =
                active_after_close(self.active_pod_tab, index, self.pod_dock_tabs.len());
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
        for action in [PermissionAction::Get, PermissionAction::PortForward] {
            self.review_resource_access(
                ResourceKind::Pods,
                action,
                Some(&namespace),
                Some(&pod_name),
                window,
                cx,
            );
        }
        if ![PermissionAction::Get, PermissionAction::PortForward]
            .into_iter()
            .all(|action| {
                self.can_action(
                    ResourceKind::Pods,
                    action,
                    Some(&namespace),
                    Some(&pod_name),
                )
            })
        {
            return;
        }
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

    /// Persistent session strip, rendered by the workspace outside the dock body.
    /// Rendering this strip never starts a shell or constructs pod output caches.
    pub(crate) fn render_console_tabs(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // Stable identities are shared by selection, menus, and deferred closes.
        let device_keys = self
            .device_terminals
            .iter()
            .map(|tab| tab.key)
            .collect::<Vec<_>>();
        let pod_keys = self
            .pod_dock_tabs
            .iter()
            .map(|tab| tab.key)
            .collect::<Vec<_>>();
        let pod_offset = device_keys.len();
        let yaml_offset = pod_offset + pod_keys.len();
        let yaml_keys = self
            .yaml_dock_tabs
            .iter()
            .map(|t| t.key)
            .collect::<Vec<_>>();
        let selected_index = if let Some(key) = self.active_yaml_tab {
            yaml_keys
                .iter()
                .position(|k| *k == key)
                .map(|i| yaml_offset + i)
                .unwrap_or(usize::MAX)
        } else if self.local_terminal_active {
            device_keys
                .iter()
                .position(|key| Some(*key) == self.active_device_terminal)
                .unwrap_or(usize::MAX)
        } else {
            self.active_pod_tab
                .filter(|index| *index < pod_keys.len())
                .map(|index| index + pod_offset)
                .unwrap_or(usize::MAX)
        };
        let mut tabs = Vec::new();
        tabs.extend(self.device_terminals.iter().map(|tab| {
            let key = tab.key;
            let title = tab.title.clone();
            let tooltip = format!("{} — {}", title, tab.status);
            self.console_tab_menu(
                Tab::new()
                    .min_w(px(160.))
                    .px_3()
                    .flex_shrink_0()
                    .label(console_tab_title(&title))
                    .aria_label(tooltip.clone())
                    .prefix(gpui_kit::component::Icon::new(IconName::SquareTerminal).small())
                    .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                    .suffix(
                        Button::new(SharedString::from(format!("close-device-console-{key}")))
                            .xsmall()
                            .ghost()
                            .icon(IconName::Close)
                            .tooltip("Close device terminal")
                            .accessibility_label("Close device terminal")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.request_close_console_tabs(
                                    crate::console_tabs::ConsoleTab::Device(key),
                                    crate::console_tabs::CloseTabs::One,
                                    window,
                                    cx,
                                );
                            })),
                    ),
                crate::console_tabs::ConsoleTab::Device(key),
                cx,
            )
        }));
        tabs.extend(self.pod_dock_tabs.iter().map(|tab| {
            let key = tab.key;
            let title = format!("{} — {}", tab.pod_name, tab.container);
            let tooltip = format!(
                "{} / {} — {} — {}",
                tab.namespace,
                title,
                tab.mode_label(),
                tab.status
            );
            self.console_tab_menu(
                Tab::new()
                    .min_w(px(160.))
                    .px_3()
                    .flex_shrink_0()
                    .label(console_tab_title(&title))
                    .aria_label(tooltip.clone())
                    .prefix(
                        gpui_kit::component::Icon::new(if tab.mode == PodStreamMode::Logs {
                            IconName::FileText
                        } else {
                            IconName::SquareTerminal
                        })
                        .small(),
                    )
                    .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                    .suffix(
                        Button::new(SharedString::from(format!("close-console-{key}")))
                            .xsmall()
                            .ghost()
                            .icon(IconName::Close)
                            .tooltip("Close pod console")
                            .accessibility_label("Close pod console")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.request_close_console_tabs(
                                    crate::console_tabs::ConsoleTab::Pod(key),
                                    crate::console_tabs::CloseTabs::One,
                                    window,
                                    cx,
                                );
                            })),
                    ),
                crate::console_tabs::ConsoleTab::Pod(key),
                cx,
            )
        }));
        tabs.extend(self.yaml_dock_tabs.iter().map(|tab| {
            let key = tab.key;
            let title = format!(
                "{}{}.yaml",
                if tab.dirty(cx) { "● " } else { "" },
                tab.name()
            );
            let tooltip = format!(
                "{:?} / {} / {} — {}",
                tab.kind(),
                tab.namespace().unwrap_or("cluster"),
                tab.name(),
                tab.message
            );
            self.console_tab_menu(
                Tab::new()
                    .min_w(px(160.))
                    .px_3()
                    .flex_shrink_0()
                    .label(console_tab_title(&title))
                    .aria_label(tooltip.clone())
                    .prefix(gpui_kit::component::Icon::new(IconName::FileText).small())
                    .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                    .suffix(
                        Button::new(SharedString::from(format!("close-yaml-{key}")))
                            .xsmall()
                            .ghost()
                            .icon(IconName::Close)
                            .tooltip("Close YAML editor")
                            .accessibility_label("Close YAML editor")
                            .on_click(cx.listener(move |app, _, window, cx| {
                                cx.stop_propagation();
                                app.request_close_yaml_tab(key, window, cx);
                            })),
                    ),
                crate::console_tabs::ConsoleTab::Yaml(key),
                cx,
            )
        }));
        let duplicate_key = (!self.local_terminal_active && self.active_yaml_tab.is_none())
            .then(|| {
                self.active_pod_tab
                    .and_then(|index| self.pod_dock_tabs.get(index))
                    .map(|tab| tab.key)
            })
            .flatten();
        let menu_owner = cx.entity().downgrade();
        let new_menu = Button::new("console-new-menu")
            .small()
            .ghost()
            .icon(IconName::Plus)
            .tooltip("New console session")
            .accessibility_label("New console session")
            .dropdown_menu(move |mut menu, _, cx| {
                let owner = menu_owner.clone();
                menu = menu.item(PopupMenuItem::new("Device Terminal").on_click(
                    move |_, window, cx| {
                        let _ = owner.update(cx, |this, cx| this.new_device_terminal(window, cx));
                    },
                ));
                for (label, mode) in [
                    ("Pod Shell", PodStreamMode::Shell),
                    ("Pod Logs", PodStreamMode::Logs),
                    ("Pod Attach", PodStreamMode::Attach),
                ] {
                    let owner = menu_owner.clone();
                    menu = menu.item(
                        PopupMenuItem::new(label)
                            .disabled(!menu_owner.upgrade().is_some_and(|owner| {
                                let app = owner.read(cx);
                                duplicate_key
                                    .and_then(|key| {
                                        app.pod_dock_tabs.iter().find(|tab| tab.key == key)
                                    })
                                    .is_some_and(|tab| {
                                        app.can_pod_stream(&tab.namespace, &tab.pod_name, mode)
                                    })
                            }))
                            .on_click(move |_, window, cx| {
                                let _ = owner.update(cx, |this, cx| {
                                    if let Some(key) = duplicate_key {
                                        this.duplicate_pod_console(key, Some(mode), window, cx);
                                    }
                                });
                            }),
                    );
                }
                menu
            });
        TabBar::new("pod-console-tabs")
            .segmented()
            .max_width(px(280.))
            .menu(false)
            .small()
            .h(px(33.))
            .w_full()
            .flex_shrink_0()
            .bg(cx.theme().sidebar)
            .text_color(cx.theme().sidebar_foreground)
            .selected_index(selected_index)
            .children(tabs)
            .last_empty_space(div().flex_shrink_0().px_1().child(new_menu))
            .suffix(
                h_flex()
                    .flex_shrink_0()
                    .gap_1()
                    .items_center()
                    .child(
                        Button::new("console-collapse")
                            .disabled(self.console_tab_order().is_empty())
                            .small()
                            .ghost()
                            .icon(if self.pod_dock_open {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronUp
                            })
                            .tooltip(if self.pod_dock_open {
                                "Collapse console"
                            } else {
                                "Expand console"
                            })
                            .accessibility_label(if self.pod_dock_open {
                                "Collapse console"
                            } else {
                                "Expand console"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.pod_dock_open =
                                    !this.pod_dock_open && !this.console_tab_order().is_empty();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("console-zoom")
                            .small()
                            .ghost()
                            .icon(IconName::Maximize)
                            .tooltip("Toggle console maximize")
                            .accessibility_label("Toggle console maximize")
                            .disabled(!self.pod_dock_open)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_console_zoom(window, cx)
                            })),
                    ),
            )
            .on_click(cx.listener(move |this, index: &usize, window, cx| {
                let target = device_keys
                    .get(*index)
                    .map(|key| crate::console_tabs::ConsoleTab::Device(*key))
                    .or_else(|| {
                        index
                            .checked_sub(pod_offset)
                            .and_then(|i| pod_keys.get(i))
                            .map(|key| crate::console_tabs::ConsoleTab::Pod(*key))
                    })
                    .or_else(|| {
                        index
                            .checked_sub(yaml_offset)
                            .and_then(|i| yaml_keys.get(i))
                            .map(|key| crate::console_tabs::ConsoleTab::Yaml(*key))
                    });
                if let Some(target) = target {
                    this.select_console_tab(target, window, cx);
                }
            }))
    }

    pub(crate) fn render_pod_dock_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.active_yaml_tab.is_some() {
            return self.render_yaml_dock(window, cx);
        }
        if self.local_terminal_active {
            return self.render_device_terminal(window, cx).into_any_element();
        }
        let active_idx = self.active_pod_tab.unwrap_or(0);
        // Each tab owns a persistent standard searchable Select. Identity lookup survives closes.
        if let Some(tab) = self.pod_dock_tabs.get(active_idx)
            && tab.container_select.is_none()
        {
            let items = tab
                .available_containers
                .iter()
                .cloned()
                .map(ContainerChoice)
                .collect::<Vec<_>>();
            let selected = tab.container.clone();
            let select = cx.new(|cx| {
                SelectState::new(SearchableVec::new(items), None, window, cx).searchable(true)
            });
            select.update(cx, |state, cx| {
                state.set_selected_value(&selected, window, cx)
            });
            cx.subscribe_in(
                &select,
                window,
                |this, entity, event: &SelectEvent<SearchableVec<ContainerChoice>>, window, cx| {
                    let SelectEvent::Confirm(Some(value)) = event else {
                        return;
                    };
                    if let Some(index) = this
                        .pod_dock_tabs
                        .iter()
                        .position(|tab| tab.container_select.as_ref() == Some(entity))
                        && this.pod_dock_tabs[index].container != *value
                    {
                        this.pod_dock_tabs[index].container = value.clone();
                        this.reopen_pod_tab(index, window, cx);
                    }
                },
            )
            .detach();
            self.pod_dock_tabs[active_idx].container_select = Some(select);
        }
        let filter = self.pod_log_filter_input.read(cx).value().to_lowercase();
        let timezone = self.preferences_draft.app.timezone.clone();
        if let Some(tab) = self
            .pod_dock_tabs
            .get_mut(active_idx)
            .filter(|tab| tab.mode == PodStreamMode::Logs)
        {
            let columns = tab.word_wrap.then_some(tab.log_columns);
            let valid = tab.presentation.as_ref().is_some_and(|cache| {
                cache.filter == filter && cache.timezone == timezone && cache.columns == columns
            });
            if !valid {
                let rows = tab
                    .output_lines
                    .iter()
                    .filter(|line| filter.is_empty() || line.to_lowercase().contains(&filter))
                    .flat_map(|line| {
                        visual_log_rows(
                            &crate::preference_runtime::log_line(line, &timezone),
                            columns,
                        )
                    })
                    .map(SharedString::from)
                    .collect();
                tab.presentation = Some(LogPresentation {
                    filter,
                    timezone,
                    columns,
                    rows: Rc::new(rows),
                });
            }
        }
        // All output and visual rows are Rc-backed: this snapshot never copies log history.
        let active = self.pod_dock_tabs.get(active_idx).cloned();
        v_flex()
            .size_full()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .bg(cx.theme().background)
            .when_some(active, |el, tab| {
                let logs = tab.mode == PodStreamMode::Logs;
                let active_key = tab.key;
                let lines = tab
                    .presentation
                    .as_ref()
                    .map(|cache| cache.rows.clone())
                    .unwrap_or_default();
                let terminal_preferences = &self.preferences_draft.terminal;
                let log_font = crate::preference_runtime::font_family(
                    &self.preferences_draft.terminal.font_family,
                    cx,
                );
                let log_font_size = f32::from(terminal_preferences.font_size);
                let (log_bg, log_fg) = crate::preference_runtime::terminal_colors(
                    terminal_preferences.theme,
                    cx.theme().mode.is_dark(),
                );
                el.when(logs, |el| {
                    el.child(
                        h_flex()
                            .w_full()
                            .px_2()
                            .py_1()
                            .gap_2()
                            .items_center()
                            .child(
                                Button::new("pod-console-target")
                                    .small()
                                    .ghost()
                                    .disabled(true)
                                    .label(format!("{} / {}", tab.namespace, tab.pod_name)),
                            )
                            .when_some(tab.container_select.clone(), |el, select| {
                                el.child(Select::new(&select).small().w(px(160.0)))
                            })
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(tab.status.clone()),
                            )
                            .when(logs, |el| {
                                el.child(
                                    div().flex_1().min_w_0().child(
                                        Input::new(&self.pod_log_filter_input)
                                            .small()
                                            .cleanable(true),
                                    ),
                                )
                            })
                            .child(
                                Button::new("pod-console-copy")
                                    .small()
                                    .ghost()
                                    .icon(IconName::Copy)
                                    .tooltip("Copy console output")
                                    .accessibility_label("Copy console output")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let Some(active_idx) = this
                                            .pod_dock_tabs
                                            .iter()
                                            .position(|tab| tab.key == active_key)
                                        else {
                                            return;
                                        };
                                        if logs {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                this.pod_dock_tabs
                                                    .get(active_idx)
                                                    .map(|tab| tab.output_lines.join("\n"))
                                                    .unwrap_or_default(),
                                            ));
                                        } else if let Some(terminal) = this
                                            .pod_dock_tabs
                                            .get(active_idx)
                                            .and_then(|tab| tab.terminal.clone())
                                        {
                                            terminal.update(cx, |terminal, cx| {
                                                terminal.copy_snapshot(cx)
                                            });
                                        }
                                    })),
                            )
                            .when(logs, |el| {
                                el.child(
                                    Button::new("pod-console-follow")
                                        .small()
                                        .ghost()
                                        .icon(IconName::ChevronDown)
                                        .tooltip("Toggle following output")
                                        .accessibility_label("Toggle following output")
                                        .selected(tab.follow)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            let Some(active_idx) = this
                                                .pod_dock_tabs
                                                .iter()
                                                .position(|tab| tab.key == active_key)
                                            else {
                                                return;
                                            };
                                            if let Some(tab) =
                                                this.pod_dock_tabs.get_mut(active_idx)
                                            {
                                                tab.follow = !tab.follow;
                                                if tab.follow {
                                                    tab.log_scroll.scroll_to_bottom();
                                                }
                                            }
                                            cx.notify();
                                        })),
                                )
                            })
                            .child(
                                Button::new("pod-console-reconnect")
                                    .disabled(!self.can_pod_stream(
                                        &tab.namespace,
                                        &tab.pod_name,
                                        tab.mode,
                                    ))
                                    .small()
                                    .ghost()
                                    .icon(IconName::Undo2)
                                    .tooltip("Reconnect stream")
                                    .accessibility_label("Reconnect stream")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        let Some(active_idx) = this
                                            .pod_dock_tabs
                                            .iter()
                                            .position(|tab| tab.key == active_key)
                                        else {
                                            return;
                                        };
                                        this.reopen_pod_tab(active_idx, window, cx)
                                    })),
                            ),
                    )
                })
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .bg(rgb(log_bg))
                        .overflow_hidden()
                        .when(logs, |el| {
                            el.on_scroll(cx.listener(move |this, _: &ScrollEvent, _, cx| {
                                let Some(active_idx) = this
                                    .pod_dock_tabs
                                    .iter()
                                    .position(|tab| tab.key == active_key)
                                else {
                                    return;
                                };
                                if let Some(tab) = this.pod_dock_tabs.get_mut(active_idx)
                                    && tab.follow
                                {
                                    tab.follow = false;
                                    cx.notify();
                                }
                            }))
                        })
                        .when(logs, |el| {
                            el.child(
                                uniform_list(
                                    ("pod-log-output", active_key),
                                    lines.len(),
                                    cx.processor(move |_, range: std::ops::Range<usize>, _, _| {
                                        range
                                            .map(|index| {
                                                div()
                                                    .h(px(log_font_size * 1.5))
                                                    .px_2()
                                                    .font_family(log_font.clone())
                                                    .text_size(px(log_font_size))
                                                    .text_color(rgb(log_fg))
                                                    .line_height(px(log_font_size * 1.5))
                                                    .whitespace_nowrap()
                                                    .child(gpui_kit::base::SelectableText::new(
                                                        (
                                                            SharedString::from(format!(
                                                                "pod-log-row-{active_key}"
                                                            )),
                                                            index,
                                                        ),
                                                        lines[index].clone(),
                                                    ))
                                            })
                                            .collect::<Vec<_>>()
                                    }),
                                )
                                .track_scroll(&tab.log_scroll)
                                .size_full(),
                            )
                        })
                        .when(logs, |el| {
                            let weak = cx.entity().downgrade();
                            let columns = tab.log_columns;
                            el.child(
                                canvas(
                                    move |bounds, window, _| {
                                        let measured = ((f32::from(bounds.size.width) - 16.0)
                                            / (log_font_size * 0.65))
                                            .floor()
                                            .max(1.0)
                                            as usize;
                                        if bounds.size.width > px(0.0) && measured != columns {
                                            window.on_next_frame(move |_, cx| {
                                                let _ = weak.update(cx, |this, cx| {
                                                    if let Some(tab) = this
                                                        .pod_dock_tabs
                                                        .iter_mut()
                                                        .find(|tab| tab.key == active_key)
                                                        && tab.log_columns != measured
                                                    {
                                                        tab.log_columns = measured;
                                                        if tab.word_wrap {
                                                            cx.notify();
                                                        }
                                                    }
                                                });
                                            });
                                        }
                                    },
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .size_full(),
                            )
                        })
                        .when(!logs, |el| {
                            el.when_some(tab.terminal.clone(), |el, terminal| el.child(terminal))
                        }),
                )
                .when(logs, |el| {
                    el.child(
                        h_flex()
                            .px_2()
                            .py_1()
                            .gap_3()
                            .items_center()
                            .child(
                                gpui_kit::component::checkbox::Checkbox::new("pod-log-timestamps")
                                    .label("Timestamps")
                                    .checked(tab.timestamps)
                                    .on_click(cx.listener(
                                        move |this, checked: &bool, window, cx| {
                                            let Some(active_idx) = this
                                                .pod_dock_tabs
                                                .iter()
                                                .position(|tab| tab.key == active_key)
                                            else {
                                                return;
                                            };
                                            if let Some(tab) =
                                                this.pod_dock_tabs.get_mut(active_idx)
                                            {
                                                tab.timestamps = *checked;
                                            }
                                            this.reopen_pod_tab(active_idx, window, cx);
                                        },
                                    )),
                            )
                            .child(
                                gpui_kit::component::checkbox::Checkbox::new("pod-log-previous")
                                    .label("Previous")
                                    .checked(tab.previous)
                                    .on_click(cx.listener(
                                        move |this, checked: &bool, window, cx| {
                                            let Some(active_idx) = this
                                                .pod_dock_tabs
                                                .iter()
                                                .position(|tab| tab.key == active_key)
                                            else {
                                                return;
                                            };
                                            if let Some(tab) =
                                                this.pod_dock_tabs.get_mut(active_idx)
                                            {
                                                tab.previous = *checked;
                                            }
                                            this.reopen_pod_tab(active_idx, window, cx);
                                        },
                                    )),
                            )
                            .child(
                                gpui_kit::component::checkbox::Checkbox::new("pod-log-wrap")
                                    .label("Word wrap")
                                    .checked(tab.word_wrap)
                                    .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                        let Some(active_idx) = this
                                            .pod_dock_tabs
                                            .iter()
                                            .position(|tab| tab.key == active_key)
                                        else {
                                            return;
                                        };
                                        if let Some(tab) = this.pod_dock_tabs.get_mut(active_idx) {
                                            tab.word_wrap = *checked;
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("pod-log-pause")
                                    .small()
                                    .ghost()
                                    .icon(if tab.paused {
                                        IconName::Play
                                    } else {
                                        IconName::Pause
                                    })
                                    .accessibility_label(if tab.paused {
                                        "Resume logs"
                                    } else {
                                        "Pause logs"
                                    })
                                    .tooltip("Pause reading; resume retained backend logs")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let Some(active_idx) = this
                                            .pod_dock_tabs
                                            .iter()
                                            .position(|tab| tab.key == active_key)
                                        else {
                                            return;
                                        };
                                        if let Some(tab) = this.pod_dock_tabs.get_mut(active_idx) {
                                            tab.paused = !tab.paused;
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("pod-log-bottom")
                                    .small()
                                    .ghost()
                                    .ml_auto()
                                    .label("To bottom")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let Some(active_idx) = this
                                            .pod_dock_tabs
                                            .iter()
                                            .position(|tab| tab.key == active_key)
                                        else {
                                            return;
                                        };
                                        if let Some(tab) = this.pod_dock_tabs.get_mut(active_idx) {
                                            tab.follow = true;
                                            tab.log_scroll.scroll_to_bottom();
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("pod-log-download")
                                    .small()
                                    .ghost()
                                    .label("Download")
                                    .tooltip("Save retained logs (up to 2500 lines / 1 MiB)")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        let Some(active_idx) = this
                                            .pod_dock_tabs
                                            .iter()
                                            .position(|tab| tab.key == active_key)
                                        else {
                                            return;
                                        };
                                        let Some(tab) = this.pod_dock_tabs.get(active_idx) else {
                                            return;
                                        };
                                        let text = tab.output_lines.join("\n");
                                        let name =
                                            format!("{}-{}.log", tab.pod_name, tab.container);
                                        let directory = std::env::var_os("HOME")
                                            .map(std::path::PathBuf::from)
                                            .unwrap_or_else(|| std::path::PathBuf::from("."));
                                        let prompt =
                                            cx.prompt_for_new_path(&directory, Some(&name));
                                        cx.spawn_in(window, async move |this, cx| {
                                            if let Ok(Ok(Some(path))) = prompt.await {
                                                let result = tokio::fs::write(path, text).await;
                                                if let Err(error) = result {
                                                    let _ = this.update_in(cx, |this, _, cx| {
                                                        this.cluster_error = Some(format!(
                                                            "Could not save logs: {error}"
                                                        ));
                                                        cx.notify();
                                                    });
                                                }
                                            }
                                        })
                                        .detach();
                                    })),
                            ),
                    )
                })
            })
            .when(!self.port_forwards.is_empty(), |el| {
                el.child(h_flex().px_2().gap_2().children(
                    self.port_forwards.clone().into_iter().map(|forward| {
                        let id = forward.id.clone();
                        let address = format!("localhost:{}", forward.local_port);
                        h_flex()
                            .gap_1()
                            .child(Tag::success().small().child(format!(
                                "{} → {}:{}",
                                address, forward.name, forward.remote_port
                            )))
                            .child(
                                Button::new(SharedString::from(format!("copy-forward-{id}")))
                                    .small()
                                    .ghost()
                                    .icon(IconName::Copy)
                                    .tooltip("Copy local address")
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            address.clone(),
                                        ))
                                    })),
                            )
                            .child(
                                Button::new(SharedString::from(format!("stop-forward-{id}")))
                                    .small()
                                    .ghost()
                                    .icon(IconName::Close)
                                    .tooltip("Stop port forward")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.stop_pod_forward(id.clone(), cx)
                                    })),
                            )
                    }),
                ))
            })
            .into_any_element()
    }
}

const MAX_LOG_LINES: usize = 2500;
const MAX_LOG_BYTES: usize = 1024 * 1024;
const MAX_PARTIAL_BYTES: usize = 256 * 1024;

/// Retain a UTF-8-safe suffix, including for a newline-free producer.
fn bounded_suffix(text: &str, limit: usize) -> &str {
    let mut start = text.len().saturating_sub(limit);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

/// Snapshots are chunks, not lines. Pending is mirrored in the final visible row;
/// remove that mirror before extending it. Process one segment at a time so a
/// single large read cannot temporarily construct an unbounded pending string.
fn append_log_chunk(lines: &mut Vec<String>, pending: &mut String, chunk: &str) {
    if chunk.is_empty() {
        return;
    }
    if !pending.is_empty() {
        lines.pop();
    }
    let mut bytes: usize = lines.iter().map(|line| line.len() + 1).sum();
    for segment in chunk.split_inclusive('\n') {
        let complete = segment.ends_with('\n');
        let content = segment.strip_suffix('\n').unwrap_or(segment);
        if content.len() >= MAX_PARTIAL_BYTES {
            pending.clear();
            pending.push_str(bounded_suffix(content, MAX_PARTIAL_BYTES));
        } else {
            // Bound before appending, rather than allocating arbitrary producer output.
            let keep = MAX_PARTIAL_BYTES - content.len();
            let suffix = bounded_suffix(pending, keep).to_owned();
            *pending = suffix;
            pending.push_str(content);
        }
        if complete {
            if pending.ends_with('\r') {
                pending.pop();
            }
            let row = std::mem::take(pending);
            bytes += row.len() + 1;
            lines.push(row);
            trim_log_rows(lines, &mut bytes);
        }
    }
    if !pending.is_empty() {
        bytes += pending.len() + 1;
        lines.push(pending.clone());
        trim_log_rows(lines, &mut bytes);
    }
}

fn trim_log_rows(lines: &mut Vec<String>, bytes: &mut usize) {
    let mut remove = 0;
    while lines.len() - remove > MAX_LOG_LINES || *bytes > MAX_LOG_BYTES {
        *bytes = bytes.saturating_sub(lines[remove].len() + 1);
        remove += 1;
    }
    if remove > 0 {
        lines.drain(..remove);
    }
}

/// Virtualize fixed-height visual rows instead of asking uniform_list to render
/// variable-height children. Only the presentation is wrapped; copy/export keep
/// the original retained lines. Wide Unicode and tabs consume display cells.
fn visual_log_rows(line: &str, columns: Option<usize>) -> Vec<String> {
    let Some(columns) = columns else {
        return vec![line.to_owned()];
    };
    let columns = columns.max(1);
    let mut rows = Vec::new();
    let mut row = String::new();
    let mut width = 0;
    for ch in line.chars() {
        let cells = match ch {
            '\t' => 4 - width % 4,
            '\u{0300}'..='\u{036f}' | '\u{200b}'..='\u{200f}' | '\u{fe00}'..='\u{fe0f}' => 0,
            '\u{1100}'..='\u{115f}'
            | '\u{2e80}'..='\u{a4cf}'
            | '\u{ac00}'..='\u{d7af}'
            | '\u{f900}'..='\u{faff}'
            | '\u{ff01}'..='\u{ff60}'
            | '\u{1f300}'..='\u{1faff}' => 2,
            _ => 1,
        };
        if width + cells > columns && !row.is_empty() {
            rows.push(std::mem::take(&mut row));
            width = 0;
        }
        if ch == '\t' {
            row.push_str(&" ".repeat(cells));
        } else {
            row.push(ch);
        }
        width += cells;
    }
    if !row.is_empty() || rows.is_empty() {
        rows.push(row);
    }
    rows
}

fn feed_tab_output(tab: &mut PodDockTab, chunk: &str, cx: &mut Context<IceonApp>) {
    if tab.mode == PodStreamMode::Logs {
        if chunk.is_empty() {
            return;
        }
        append_log_chunk(
            Rc::make_mut(&mut tab.output_lines),
            Rc::make_mut(&mut tab.log_pending),
            chunk,
        );
        tab.presentation = None;
        if tab.follow {
            tab.log_scroll.scroll_to_bottom();
        }
    } else if let Some(terminal) = &tab.terminal {
        terminal.update(cx, |terminal, cx| terminal.feed(chunk, cx));
    }
}

#[cfg(test)]
mod chunk_tests {
    use super::append_log_chunk;
    #[test]
    fn pod_console_modes_gate_the_matching_named_subresource() {
        assert_eq!(
            super::stream_permission(super::PodStreamMode::Logs),
            super::PermissionAction::Logs
        );
        assert_eq!(
            super::stream_permission(super::PodStreamMode::Shell),
            super::PermissionAction::Exec
        );
        assert_eq!(
            super::stream_permission(super::PodStreamMode::Attach),
            super::PermissionAction::Attach
        );
    }

    #[test]
    fn console_labels_are_bounded_without_losing_shell_suffix() {
        assert_eq!(super::console_tab_title("icekube — fish"), "icekube — fish");
        assert_eq!(super::console_tab_title("Terminal"), "Terminal");
        let title = format!("{} — fish", "界".repeat(80));
        let label = super::console_tab_title(&title);
        assert_eq!(label.chars().count(), 36);
        assert!(label.ends_with(" — fish"));
        assert!(label.contains('…'));
        assert_eq!(super::console_tab_title(&"a".repeat(36)), "a".repeat(36));
    }
    #[test]
    fn idle_snapshots_do_not_schedule_updates() {
        assert!(!super::poll_needs_update(
            "", "running", "running", false, false
        ));
        assert!(super::poll_needs_update(
            "chunk", "running", "running", false, false
        ));
        assert!(super::poll_needs_update(
            "", "running", "exited", false, false
        ));
        assert!(super::poll_needs_update(
            "", "running", "running", false, true
        ));
    }
    #[test]
    fn terminal_and_inactive_log_output_do_not_invalidate_workspace() {
        assert!(!super::poll_notifies_parent(false, false, true, true));
        assert!(!super::poll_notifies_parent(false, true, false, true));
        assert!(super::poll_notifies_parent(false, true, true, true));
        assert!(super::poll_notifies_parent(true, true, false, false));
        assert!(!super::poll_notifies_parent(false, true, true, false));
    }
    #[test]
    fn closing_a_background_tab_preserves_selection() {
        assert_eq!(super::active_after_close(Some(2), 0, 3), Some(1));
        assert_eq!(super::active_after_close(Some(0), 2, 3), Some(0));
        assert_eq!(super::active_after_close(Some(2), 2, 2), Some(1));
        assert_eq!(super::active_after_close(Some(0), 0, 0), None);
    }
    #[test]
    fn split_lines_and_crlf_are_preserved() {
        let (mut rows, mut pending) = (Vec::new(), String::new());
        for chunk in ["hel", "lo\r", "\nworld", "\n\nlast"] {
            append_log_chunk(&mut rows, &mut pending, chunk);
        }
        assert_eq!(rows, ["hello", "world", "", "last"]);
        assert_eq!(pending, "last");
    }
    #[test]
    fn giant_partial_is_utf8_safe_and_bounded() {
        let (mut rows, mut pending) = (Vec::new(), String::new());
        append_log_chunk(&mut rows, &mut pending, &"界".repeat(200_000));
        assert!(pending.len() <= super::MAX_PARTIAL_BYTES);
        assert_eq!(rows.last(), Some(&pending));
        append_log_chunk(&mut rows, &mut pending, " end\r");
        append_log_chunk(&mut rows, &mut pending, "\nnext");
        assert!(rows[0].ends_with(" end"));
        assert_eq!(pending, "next");
    }
    #[test]
    fn total_retained_bytes_are_bounded() {
        let (mut rows, mut pending) = (Vec::new(), String::new());
        for _ in 0..30 {
            append_log_chunk(
                &mut rows,
                &mut pending,
                &format!("{}\n", "x".repeat(100_000)),
            );
        }
        assert!(rows.iter().map(|line| line.len() + 1).sum::<usize>() <= super::MAX_LOG_BYTES);
        assert!(rows.len() <= super::MAX_LOG_LINES);
    }
    #[test]
    fn empty_reads_do_not_remove_partial_rows() {
        let (mut rows, mut pending) = (Vec::new(), String::new());
        append_log_chunk(&mut rows, &mut pending, "hello");
        append_log_chunk(&mut rows, &mut pending, "");
        assert_eq!(rows, ["hello"]);
    }
    #[test]
    fn visual_wrap_retains_content_and_unicode_boundaries() {
        assert_eq!(super::visual_log_rows("abcdef", Some(3)), ["abc", "def"]);
        assert_eq!(super::visual_log_rows("界界a", Some(4)), ["界界", "a"]);
        assert_eq!(super::visual_log_rows("", Some(4)), [""]);
        assert_eq!(super::visual_log_rows("abcdef", None), ["abcdef"]);
    }
    #[test]
    fn retention_is_bounded_including_partial_row() {
        let (mut rows, mut pending) = (Vec::new(), String::new());
        append_log_chunk(&mut rows, &mut pending, &"line\n".repeat(3000));
        append_log_chunk(&mut rows, &mut pending, "partial");
        assert_eq!(rows.len(), 2500);
        append_log_chunk(&mut rows, &mut pending, " end\n");
        assert_eq!(rows.len(), 2500);
        assert_eq!(rows.last().unwrap(), "partial end");
    }
}
