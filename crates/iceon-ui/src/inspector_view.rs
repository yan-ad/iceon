//! Resource Inspector view: right-hand resizable panel displaying `ResourceDetail`, `PodDetails`,
//! and `HelmDetail`, with actions for workloads, nodes, services, YAML editing, and related collections.

use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Editor, Input},
    scroll::ScrollableElement as _,
    tag::Tag,
    tab::{Tab, TabBar},
    v_flex, IconName,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::IceonApp;
use crate::navigation::{connection_error_message, format_age};
use iceon_core::connection::{
    ResourceKind,
    detail_collections::{CollectionKind, CollectionRequest, ResourceCollection},
    details::{DetailRequest, ResourceDetail, ResourceReference},
    helm::{HelmDetail, HelmDetailRequest, HelmView},
    node_actions::{NodeAction, NodeActionRequest},
    node_drain::{DrainExecuteRequest, DrainReview, DrainReviewRequest, DrainResult},
    pod_runtime::PodStreamMode,
    pods::{PodAction, PodDetails, PodMutationRequest, PodRequest},
    resource_edit::{EditMode, EditRequest},
    service_forward::ServiceForwardRequest,
    workload_actions::{WorkflowRequest, WorkloadAction},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InspectorTarget {
    Resource(ResourceReference),
    Helm {
        view: HelmView,
        name: String,
        namespace: Option<String>,
        version: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectorTab {
    Summary,
    Metadata,
    Relations,
    Yaml,
}

#[derive(Default, Clone)]
pub struct InspectorState {
    pub target: Option<InspectorTarget>,
    pub tab: Option<InspectorTab>,
    pub detail: Option<ResourceDetail>,
    pub pod_detail: Option<PodDetails>,
    pub helm_detail: Option<HelmDetail>,
    pub collections: Vec<ResourceCollection>,
    pub edit_yaml: Option<String>,
    pub edit_resource_version: Option<String>,
    pub drain_review: Option<DrainReview>,
    pub drain_result: Option<DrainResult>,
    pub loading: bool,
    pub error: Option<String>,
    pub status_banner: Option<String>,
}

impl IceonApp {
    pub(crate) fn open_resource_inspector(
        &mut self,
        reference: ResourceReference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.inspector.target = Some(InspectorTarget::Resource(reference.clone()));
        self.inspector.tab = Some(InspectorTab::Summary);
        self.inspector.detail = None;
        self.inspector.pod_detail = None;
        self.inspector.helm_detail = None;
        self.inspector.collections.clear();
        self.inspector.drain_review = None;
        self.inspector.drain_result = None;
        self.inspector.error = None;
        self.inspector.status_banner = None;
        self.refresh_inspector(window, cx);
    }

    pub(crate) fn open_helm_inspector(
        &mut self,
        view: HelmView,
        name: String,
        namespace: Option<String>,
        version: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.inspector.target = Some(InspectorTarget::Helm {
            view,
            name,
            namespace,
            version,
        });
        self.inspector.tab = Some(InspectorTab::Summary);
        self.inspector.detail = None;
        self.inspector.pod_detail = None;
        self.inspector.helm_detail = None;
        self.inspector.collections.clear();
        self.inspector.error = None;
        self.inspector.status_banner = None;
        self.refresh_inspector(window, cx);
    }

    pub(crate) fn refresh_inspector(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let Some(target) = self.inspector.target.clone() else {
            return;
        };
        self.inspector.loading = true;
        let read_id = self.next_operation_id();
        let engine = self.connection.clone();

        match target {
            InspectorTarget::Resource(reference) => {
                cx.spawn_in(window, async move |this, cx| {
                    let detail_res = engine
                        .resource_detail(DetailRequest {
                            cluster_id: session.cluster_id.clone(),
                            session_id: session.session_id.clone(),
                            read_id,
                            reference: reference.clone(),
                        })
                        .await;

                    let pod_res = if reference.kind == ResourceKind::Pods {
                        if let Some(ns) = reference.namespace.clone() {
                            Some(
                                engine
                                    .pod_detail(PodRequest {
                                        cluster_id: session.cluster_id.clone(),
                                        session_id: session.session_id.clone(),
                                        namespace: ns,
                                        name: reference.name.clone(),
                                        uid: reference.uid.clone(),
                                    })
                                    .await,
                            )
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    let mut collections = Vec::new();
                    if let Ok(detail) = &detail_res {
                        let resolved = detail.reference.clone();
                        let kinds = supported_collections(resolved.kind);
                        for (idx, col_kind) in kinds.into_iter().enumerate() {
                            if let Ok(col) = engine
                                .resource_collection(CollectionRequest {
                                    cluster_id: session.cluster_id.clone(),
                                    session_id: session.session_id.clone(),
                                    read_id: read_id + (idx as u64) + 1,
                                    reference: resolved.clone(),
                                    collection: col_kind,
                                })
                                .await
                            {
                                collections.push(col);
                            }
                        }
                    }

                    let _ = this.update(cx, |this, cx| {
                        this.inspector.loading = false;
                        match detail_res {
                            Ok(detail) => {
                                this.inspector.detail = Some(detail);
                                this.inspector.error = None;
                            }
                            Err(err) => {
                                this.inspector.error =
                                    Some(connection_error_message(err).to_string());
                            }
                        }
                        if let Some(Ok(pod)) = pod_res {
                            let yaml = pod.yaml.clone();
                            this.inspector.pod_detail = Some(pod);
                            this.inspector.edit_yaml = Some(yaml);
                        }
                        this.inspector.collections = collections;
                        cx.notify();
                    });
                })
                .detach();
            }
            InspectorTarget::Helm {
                view,
                name,
                namespace,
                version,
            } => {
                cx.spawn_in(window, async move |this, cx| {
                    let res = engine
                        .helm_detail(HelmDetailRequest {
                            cluster_id: session.cluster_id,
                            session_id: session.session_id,
                            read_id,
                            view,
                            name,
                            namespace,
                            version,
                        })
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        this.inspector.loading = false;
                        match res {
                            Ok(detail) => {
                                this.inspector.helm_detail = Some(detail);
                                this.inspector.error = None;
                            }
                            Err(err) => {
                                this.inspector.error =
                                    Some(connection_error_message(err).to_string());
                            }
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
        }
    }

    pub(crate) fn load_editable_yaml(
        &mut self,
        reference: ResourceReference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let op_id = self.next_operation_id();
        let engine = self.connection.clone();
        cx.spawn_in(window, async move |this, cx| {
            let res = engine
                .resource_edit(EditRequest {
                    cluster_id: session.cluster_id,
                    session_id: session.session_id,
                    operation_id: op_id,
                    reference,
                    resource_version: None,
                    yaml: None,
                    mode: EditMode::Read,
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                match res {
                    Ok(edit_res) => {
                        this.inspector.edit_resource_version = Some(edit_res.resource_version);
                        if let Some(yaml) = edit_res.yaml {
                            this.inspector.edit_yaml = Some(yaml.clone());
                            this.inspector_yaml_editor.update(cx, |ed, cx| {
                                ed.set_value(yaml, window, cx);
                            });
                        }
                        this.inspector.error = None;
                    }
                    Err(err) => {
                        this.inspector.error = Some(connection_error_message(err).to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn submit_editable_yaml(
        &mut self,
        mode: EditMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let Some(detail) = self.inspector.detail.clone() else {
            return;
        };
        let yaml = self.inspector_yaml_editor.read(cx).value().to_string();
        let op_id = self.next_operation_id();
        let engine = self.connection.clone();

        if detail.reference.kind == ResourceKind::Pods {
            if let Some(pod) = self.inspector.pod_detail.clone() {
                cx.spawn_in(window, async move |this, cx| {
                    let res = engine
                        .pod_mutate(PodMutationRequest {
                            pod: PodRequest {
                                cluster_id: session.cluster_id,
                                session_id: session.session_id,
                                namespace: pod.namespace,
                                name: pod.name,
                                uid: Some(pod.uid),
                            },
                            action: PodAction::Replace,
                            resource_version: pod.resource_version,
                            yaml: Some(yaml),
                        })
                        .await;
                    let _ = this.update_in(cx, |this, window, cx| {
                        match res {
                            Ok(()) => {
                                this.inspector.status_banner =
                                    Some("Pod spec updated on cluster.".into());
                                this.inspector.error = None;
                                this.refresh_inspector(window, cx);
                            }
                            Err(err) => {
                                this.inspector.error =
                                    Some(connection_error_message(err).to_string());
                            }
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            return;
        }

        let rv = self
            .inspector
            .edit_resource_version
            .clone()
            .or_else(|| detail.item.resource_version.clone());
        let reference = detail.reference.clone();
        cx.spawn_in(window, async move |this, cx| {
            let res = engine
                .resource_edit(EditRequest {
                    cluster_id: session.cluster_id,
                    session_id: session.session_id,
                    operation_id: op_id,
                    reference,
                    resource_version: rv,
                    yaml: Some(yaml),
                    mode,
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                match res {
                    Ok(edit_res) => {
                        this.inspector.edit_resource_version = Some(edit_res.resource_version);
                        this.inspector.status_banner = Some(if mode == EditMode::DryRun {
                            "Server dry-run validation succeeded.".into()
                        } else {
                            "Resource patch applied to cluster.".into()
                        });
                        this.inspector.error = None;
                        if mode == EditMode::Apply {
                            this.refresh_inspector(window, cx);
                        }
                    }
                    Err(err) => {
                        this.inspector.error = Some(connection_error_message(err).to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn run_workload_action(
        &mut self,
        action: WorkloadAction,
        replicas: Option<u32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let Some(detail) = self.inspector.detail.clone() else {
            return;
        };
        let Some(rv) = detail.item.resource_version.clone() else {
            return;
        };
        let op_id = self.next_operation_id();
        let engine = self.connection.clone();
        let reference = detail.reference;
        cx.spawn_in(window, async move |this, cx| {
            let res = engine
                .resource_workload_action(WorkflowRequest {
                    cluster_id: session.cluster_id,
                    session_id: session.session_id,
                    operation_id: op_id,
                    reference,
                    resource_version: rv,
                    action,
                    replicas,
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                match res {
                    Ok(out) => {
                        this.inspector.status_banner =
                            Some(format!("Workload action {:?} accepted.", out.action));
                        this.inspector.error = None;
                        this.refresh_inspector(window, cx);
                        this.refresh_active_cluster_view(window, cx);
                    }
                    Err(err) => {
                        this.inspector.error = Some(connection_error_message(err).to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn run_node_action(
        &mut self,
        action: NodeAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let Some(detail) = self.inspector.detail.clone() else {
            return;
        };
        let Some(rv) = detail.item.resource_version.clone() else {
            return;
        };
        let op_id = self.next_operation_id();
        let engine = self.connection.clone();
        let reference = detail.reference;
        cx.spawn_in(window, async move |this, cx| {
            let res = engine
                .resource_node_action(NodeActionRequest {
                    cluster_id: session.cluster_id,
                    session_id: session.session_id,
                    operation_id: op_id,
                    reference,
                    resource_version: rv,
                    action,
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                match res {
                    Ok(_) => {
                        this.inspector.status_banner =
                            Some(format!("Node {:?} completed.", action));
                        this.inspector.error = None;
                        this.refresh_inspector(window, cx);
                        this.refresh_active_cluster_view(window, cx);
                    }
                    Err(err) => {
                        this.inspector.error = Some(connection_error_message(err).to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn review_node_drain(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let Some(detail) = self.inspector.detail.clone() else {
            return;
        };
        let Some(rv) = detail.item.resource_version.clone() else {
            return;
        };
        let op_id = self.next_operation_id();
        let engine = self.connection.clone();
        let reference = detail.reference;
        cx.spawn_in(window, async move |this, cx| {
            let res = engine
                .node_drain_review(DrainReviewRequest {
                    cluster_id: session.cluster_id,
                    session_id: session.session_id,
                    operation_id: op_id,
                    reference,
                    resource_version: rv,
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                match res {
                    Ok(review) => {
                        this.inspector.drain_review = Some(review);
                        this.inspector.error = None;
                    }
                    Err(err) => {
                        this.inspector.error = Some(connection_error_message(err).to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn execute_node_drain(
        &mut self,
        plan_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let op_id = self.next_operation_id();
        let engine = self.connection.clone();
        cx.spawn_in(window, async move |this, cx| {
            let res = engine
                .node_drain_execute(DrainExecuteRequest {
                    cluster_id: session.cluster_id,
                    session_id: session.session_id,
                    operation_id: op_id,
                    plan_id,
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                match res {
                    Ok(outcome) => {
                        this.inspector.drain_result = Some(outcome);
                        this.inspector.drain_review = None;
                        this.inspector.error = None;
                        this.refresh_inspector(window, cx);
                    }
                    Err(err) => {
                        this.inspector.error = Some(connection_error_message(err).to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn start_service_port_forward(
        &mut self,
        service_port: u16,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let Some(detail) = self.inspector.detail.clone() else {
            return;
        };
        let Some(rv) = detail.item.resource_version.clone() else {
            return;
        };
        let op_id = self.next_operation_id();
        let engine = self.connection.clone();
        let reference = detail.reference;
        cx.spawn_in(window, async move |this, cx| {
            let res = engine
                .service_forward_start(ServiceForwardRequest {
                    cluster_id: session.cluster_id,
                    session_id: session.session_id,
                    operation_id: op_id,
                    reference,
                    resource_version: rv,
                    service_port,
                    local_port: 0,
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                match res {
                    Ok(fwd) => {
                        this.inspector.status_banner = Some(format!(
                            "Forwarding service port {} -> 127.0.0.1:{} (Pod {})",
                            fwd.service_port, fwd.forward.local_port, fwd.pod.name
                        ));
                        this.inspector.error = None;
                        this.refresh_port_forwards(cx);
                    }
                    Err(err) => {
                        this.inspector.error = Some(connection_error_message(err).to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn render_inspector_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let tab = self.inspector.tab.unwrap_or(InspectorTab::Summary);
        let tab_index = match tab {
            InspectorTab::Summary => 0,
            InspectorTab::Metadata => 1,
            InspectorTab::Relations => 2,
            InspectorTab::Yaml => 3,
        };

        let title = if let Some(detail) = &self.inspector.detail {
            format!(
                "{} · {}",
                iceon_core::connection::create::creation_info(detail.reference.kind).1,
                detail.reference.name
            )
        } else if let Some(helm) = &self.inspector.helm_detail {
            format!("Helm · {}", helm.name)
        } else {
            "Inspector".into()
        };

        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .border_l_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_2p5()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .truncate()
                            .child(title),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("refresh-inspector-btn")
                                    .xsmall()
                                    .ghost()
                                    .icon(IconName::Undo2)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.refresh_inspector(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("close-inspector-btn")
                                    .xsmall()
                                    .ghost()
                                    .icon(IconName::Close)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.inspector.target = None;
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .when_some(self.inspector.error.clone(), |el, msg| {
                el.child(
                    div()
                        .mx_3()
                        .mt_2()
                        .p_2()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(cx.theme().danger)
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(msg),
                )
            })
            .when_some(self.inspector.status_banner.clone(), |el, msg| {
                el.child(
                    div()
                        .mx_3()
                        .mt_2()
                        .p_2()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(cx.theme().success)
                        .text_xs()
                        .text_color(cx.theme().success)
                        .child(msg),
                )
            })
            .child(if let Some(helm) = self.inspector.helm_detail.clone() {
                v_flex()
                    .flex_1()
                    .p_3()
                    .gap_3()
                    .overflow_y_scrollbar()
                    .children(helm.facts.into_iter().map(|fact| {
                        h_flex()
                            .justify_between()
                            .gap_2()
                            .py_1()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .text_xs()
                            .child(
                                div()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(fact.label),
                            )
                            .child(div().font_medium().child(fact.value))
                    }))
                    .children(helm.history.into_iter().map(|rev| {
                        h_flex()
                            .justify_between()
                            .p_2()
                            .rounded(px(6.0))
                            .bg(cx.theme().secondary)
                            .text_xs()
                            .child(div().font_semibold().child(format!("Rev #{}", rev.revision)))
                            .child(div().child(rev.chart))
                            .child(Tag::secondary().child(rev.status))
                    }))
                    .into_any_element()
            } else if let Some(detail) = self.inspector.detail.clone() {
                let reference = detail.reference.clone();
                v_flex()
                    .flex_1()
                    .overflow_hidden()
                    .child(self.render_inspector_action_bar(&detail, window, cx))
                    .child(
                        div()
                            .px_3()
                            .pt_2()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(
                                TabBar::new("inspector-tabs")
                                    .selected_index(tab_index)
                                    .on_click(cx.listener(move |this, idx: &usize, window, cx| {
                                        let next_tab = match *idx {
                                            0 => InspectorTab::Summary,
                                            1 => InspectorTab::Metadata,
                                            2 => InspectorTab::Relations,
                                            _ => InspectorTab::Yaml,
                                        };
                                        this.inspector.tab = Some(next_tab);
                                        if next_tab == InspectorTab::Yaml {
                                            if this.inspector.detail.as_ref().is_some_and(|d| {
                                                d.reference.kind == ResourceKind::Pods
                                            }) {
                                                if let Some(yaml) =
                                                    this.inspector.edit_yaml.clone()
                                                {
                                                    this.inspector_yaml_editor.update(
                                                        cx,
                                                        |ed, cx| {
                                                            ed.set_value(yaml, window, cx);
                                                        },
                                                    );
                                                }
                                            } else {
                                                this.load_editable_yaml(
                                                    reference.clone(),
                                                    window,
                                                    cx,
                                                );
                                            }
                                        }
                                        cx.notify();
                                    }))
                                    .child(Tab::new().label("Summary"))
                                    .child(Tab::new().label("Metadata"))
                                    .child(Tab::new().label("Related"))
                                    .child(Tab::new().label("YAML")),
                            ),
                    )
                    .child(match tab {
                        InspectorTab::Summary => {
                            self.render_inspector_summary(&detail, cx).into_any_element()
                        }
                        InspectorTab::Metadata => {
                            self.render_inspector_metadata(&detail, cx).into_any_element()
                        }
                        InspectorTab::Relations => {
                            self.render_inspector_relations(&detail, cx).into_any_element()
                        }
                        InspectorTab::Yaml => {
                            self.render_inspector_yaml(&detail, cx).into_any_element()
                        }
                    })
                    .into_any_element()
            } else {
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .p_6()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Loading resource details...")
                    .into_any_element()
            })
    }

    fn render_inspector_action_bar(
        &mut self,
        detail: &ResourceDetail,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let kind = detail.reference.kind;
        let can_scale = matches!(
            kind,
            ResourceKind::Deployments
                | ResourceKind::StatefulSets
                | ResourceKind::ReplicaSets
                | ResourceKind::ReplicationControllers
        );
        let can_restart = matches!(
            kind,
            ResourceKind::Deployments | ResourceKind::DaemonSets | ResourceKind::StatefulSets
        );
        let can_suspend = matches!(kind, ResourceKind::Jobs | ResourceKind::CronJobs);
        let can_trigger = kind == ResourceKind::CronJobs;
        let is_node = kind == ResourceKind::Nodes;
        let is_service = kind == ResourceKind::Services;

        h_flex()
            .w_full()
            .px_3()
            .py_2()
            .gap_1p5()
            .flex_wrap()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().secondary)
            .when(can_scale, |el| {
                el.child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(
                            div()
                                .w(px(64.0))
                                .child(Input::new(&self.scale_replicas_input).xsmall()),
                        )
                        .child(
                            Button::new("workload-scale-btn")
                                .xsmall()
                                .outline()
                                .label("Scale")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    if let Ok(replicas) = this
                                        .scale_replicas_input
                                        .read(cx)
                                        .value()
                                        .trim()
                                        .parse::<u32>()
                                    {
                                        this.run_workload_action(
                                            WorkloadAction::Scale,
                                            Some(replicas),
                                            window,
                                            cx,
                                        );
                                    }
                                })),
                        ),
                )
            })
            .when(can_restart, |el| {
                el.child(
                    Button::new("workload-restart-btn")
                        .xsmall()
                        .outline()
                        .label("Restart Rollout")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.run_workload_action(WorkloadAction::Restart, None, window, cx);
                        })),
                )
            })
            .when(can_suspend, |el| {
                el.child(
                    Button::new("workload-suspend-btn")
                        .xsmall()
                        .outline()
                        .label("Suspend")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.run_workload_action(WorkloadAction::Suspend, None, window, cx);
                        })),
                )
                .child(
                    Button::new("workload-resume-btn")
                        .xsmall()
                        .outline()
                        .label("Resume")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.run_workload_action(WorkloadAction::Resume, None, window, cx);
                        })),
                )
            })
            .when(can_trigger, |el| {
                el.child(
                    Button::new("workload-trigger-btn")
                        .xsmall()
                        .primary()
                        .label("Trigger Job")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.run_workload_action(WorkloadAction::Trigger, None, window, cx);
                        })),
                )
            })
            .when(is_node, |el| {
                el.child(
                    Button::new("node-cordon-btn")
                        .xsmall()
                        .outline()
                        .label("Cordon")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.run_node_action(NodeAction::Cordon, window, cx);
                        })),
                )
                .child(
                    Button::new("node-uncordon-btn")
                        .xsmall()
                        .outline()
                        .label("Uncordon")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.run_node_action(NodeAction::Uncordon, window, cx);
                        })),
                )
                .child(
                    Button::new("node-drain-review-btn")
                        .xsmall()
                        .danger()
                        .label("Review Drain")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.review_node_drain(window, cx);
                        })),
                )
            })
            .when(is_service, |el| {
                el.child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(
                            div()
                                .w(px(72.0))
                                .child(Input::new(&self.service_port_input).xsmall()),
                        )
                        .child(
                            Button::new("service-forward-btn")
                                .xsmall()
                                .primary()
                                .label("Forward Port")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    if let Ok(port) = this
                                        .service_port_input
                                        .read(cx)
                                        .value()
                                        .trim()
                                        .parse::<u16>()
                                    {
                                        this.start_service_port_forward(port, window, cx);
                                    }
                                })),
                        ),
                )
            })
            .when_some(self.inspector.pod_actions(&detail.reference, cx), |el, pod_ui| {
                el.child(pod_ui)
            })
    }

    fn render_inspector_summary(
        &self,
        detail: &ResourceDetail,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .flex_1()
            .p_3()
            .gap_3()
            .overflow_y_scrollbar()
            .child(
                v_flex()
                    .p_2p5()
                    .gap_1p5()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(cx.theme().border)
                    .text_xs()
                    .child(
                        h_flex()
                            .justify_between()
                            .child(
                                div()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Status"),
                            )
                            .child(Tag::secondary().child(detail.item.status.clone())),
                    )
                    .child(
                        h_flex()
                            .justify_between()
                            .child(div().text_color(cx.theme().muted_foreground).child("Age"))
                            .child(
                                div().child(format_age(detail.item.created_at.as_deref())),
                            ),
                    )
                    .when_some(detail.reference.namespace.clone(), |el, ns| {
                        el.child(
                            h_flex()
                                .justify_between()
                                .child(
                                    div()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("Namespace"),
                                )
                                .child(div().child(ns)),
                        )
                    }),
            )
            .when_some(self.inspector.drain_review.clone(), |el, review| {
                let plan_id = review.plan_id.clone();
                el.child(
                    v_flex()
                        .p_3()
                        .gap_2()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(cx.theme().warning)
                        .bg(cx.theme().secondary)
                        .text_xs()
                        .child(
                            div().font_semibold().child(format!(
                                "Node Drain Review ({} pod(s), can_drain: {})",
                                review.pods.len(),
                                review.can_drain
                            )),
                        )
                        .children(review.pods.iter().take(12).map(|pod| {
                            h_flex()
                                .justify_between()
                                .child(div().child(format!("{}/{}", pod.namespace, pod.name)))
                                .child(
                                    div()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!("{:?} ({:?})", pod.disposition, pod.reason)),
                                )
                        }))
                        .when_some(plan_id, |el, pid| {
                            el.child(
                                Button::new("execute-drain-plan-btn")
                                    .xsmall()
                                    .danger()
                                    .label("Confirm & Execute Eviction Plan")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.execute_node_drain(pid.clone(), window, cx);
                                    })),
                            )
                        }),
                )
            })
            .when_some(self.inspector.pod_detail.clone(), |el, pod| {
                let ns = pod.namespace.clone();
                let name = pod.name.clone();
                let uid = pod.uid.clone();
                el.child(
                    v_flex()
                        .p_2p5()
                        .gap_2()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(cx.theme().border)
                        .child(
                            div()
                                .text_xs()
                                .font_semibold()
                                .child("Containers & Interactive Streams"),
                        )
                        .children(pod.containers.into_iter().map(|c| {
                            let c_name = c.name.clone();
                            let c_ns = ns.clone();
                            let c_pod = name.clone();
                            let c_uid = uid.clone();
                            let s_name = c.name.clone();
                            let s_ns = ns.clone();
                            let s_pod = name.clone();
                            let s_uid = uid.clone();
                            let a_name = c.name.clone();
                            let a_ns = ns.clone();
                            let a_pod = name.clone();
                            let a_uid = uid.clone();

                            v_flex()
                                .p_2()
                                .gap_1p5()
                                .rounded(px(4.0))
                                .bg(cx.theme().secondary)
                                .text_xs()
                                .child(
                                    h_flex()
                                        .justify_between()
                                        .child(div().font_semibold().child(format!(
                                            "{} ({})",
                                            c.name, c.kind
                                        )))
                                        .child(Tag::secondary().child(c.state)),
                                )
                                .child(
                                    div()
                                        .text_color(cx.theme().muted_foreground)
                                        .truncate()
                                        .child(c.image),
                                )
                                .child(
                                    h_flex()
                                        .gap_1p5()
                                        .flex_wrap()
                                        .child(
                                            Button::new(SharedString::from(format!(
                                                "pod-logs-{}",
                                                c.name
                                            )))
                                            .xsmall()
                                            .primary()
                                            .label("Logs")
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.open_pod_stream_tab(
                                                    c_ns.clone(),
                                                    c_pod.clone(),
                                                    c_uid.clone(),
                                                    c_name.clone(),
                                                    PodStreamMode::Logs,
                                                    window,
                                                    cx,
                                                );
                                            })),
                                        )
                                        .child(
                                            Button::new(SharedString::from(format!(
                                                "pod-shell-{}",
                                                c.name
                                            )))
                                            .xsmall()
                                            .outline()
                                            .label("Shell")
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.open_pod_stream_tab(
                                                    s_ns.clone(),
                                                    s_pod.clone(),
                                                    s_uid.clone(),
                                                    s_name.clone(),
                                                    PodStreamMode::Shell,
                                                    window,
                                                    cx,
                                                );
                                            })),
                                        )
                                        .child(
                                            Button::new(SharedString::from(format!(
                                                "pod-attach-{}",
                                                c.name
                                            )))
                                            .xsmall()
                                            .outline()
                                            .label("Attach")
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.open_pod_stream_tab(
                                                    a_ns.clone(),
                                                    a_pod.clone(),
                                                    a_uid.clone(),
                                                    a_name.clone(),
                                                    PodStreamMode::Attach,
                                                    window,
                                                    cx,
                                                );
                                            })),
                                        )
                                        .children(c.ports.into_iter().map(|p| {
                                            let f_ns = ns.clone();
                                            let f_pod = name.clone();
                                            let f_uid = uid.clone();
                                            let port = p.port;
                                            Button::new(SharedString::from(format!(
                                                "pod-fwd-{}-{port}",
                                                c.name
                                            )))
                                            .xsmall()
                                            .outline()
                                            .label(format!("Forward :{port}"))
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.start_pod_port_forward(
                                                    f_ns.clone(),
                                                    f_pod.clone(),
                                                    f_uid.clone(),
                                                    port,
                                                    window,
                                                    cx,
                                                );
                                            }))
                                        })),
                                )
                        })),
                )
            })
            .when_some(detail.event_diagnostic.clone(), |el, diag| {
                el.child(
                    v_flex()
                        .p_2p5()
                        .gap_1()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(cx.theme().border)
                        .text_xs()
                        .child(div().font_semibold().child("Event Message"))
                        .child(div().child(diag.message)),
                )
            })
            .children(detail.sections.iter().map(|sec| {
                v_flex()
                    .p_2p5()
                    .gap_1p5()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(cx.theme().border)
                    .text_xs()
                    .child(div().font_semibold().child(sec.title.clone()))
                    .children(sec.facts.iter().map(|f| {
                        h_flex()
                            .justify_between()
                            .gap_2()
                            .py_0p5()
                            .child(
                                div()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(f.label.clone()),
                            )
                            .child(div().font_medium().child(f.value.clone()))
                    }))
            }))
    }

    fn render_inspector_metadata(
        &self,
        detail: &ResourceDetail,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .flex_1()
            .p_3()
            .gap_3()
            .overflow_y_scrollbar()
            .child(if let Some(meta) = &detail.metadata {
                v_flex()
                    .gap_3()
                    .child(
                        v_flex()
                            .p_2p5()
                            .gap_1()
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(cx.theme().border)
                            .text_xs()
                            .child(div().font_semibold().child("Labels"))
                            .children(meta.labels.iter().map(|entry| {
                                h_flex()
                                    .justify_between()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(entry.key.clone()),
                                    )
                                    .child(div().child(entry.value.clone()))
                            })),
                    )
                    .child(
                        v_flex()
                            .p_2p5()
                            .gap_1()
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(cx.theme().border)
                            .text_xs()
                            .child(div().font_semibold().child("Reviewed Annotations"))
                            .children(meta.annotations.iter().map(|entry| {
                                h_flex()
                                    .justify_between()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(entry.key.clone()),
                                    )
                                    .child(div().child(entry.value.clone()))
                            })),
                    )
                    .into_any_element()
            } else {
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Metadata details are withheld for sensitive resource kinds.")
                    .into_any_element()
            })
    }

    fn render_inspector_relations(
        &self,
        detail: &ResourceDetail,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .flex_1()
            .p_3()
            .gap_3()
            .overflow_y_scrollbar()
            .children(detail.related.iter().map(|(relation, target)| {
                let target_ref = target.clone();
                h_flex()
                    .justify_between()
                    .items_center()
                    .p_2()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(cx.theme().border)
                    .text_xs()
                    .child(div().font_medium().child(format!(
                        "{relation}: {}",
                        target.name
                    )))
                    .child(
                        Button::new(SharedString::from(format!(
                            "inspect-rel-{relation}-{}",
                            target.name
                        )))
                        .xsmall()
                        .outline()
                        .label("Inspect")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_resource_inspector(target_ref.clone(), window, cx);
                        })),
                    )
            }))
            .children(self.inspector.collections.iter().map(|col| {
                v_flex()
                    .p_2p5()
                    .gap_1p5()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(cx.theme().border)
                    .text_xs()
                    .child(
                        div().font_semibold().child(format!(
                            "{:?} ({} items · {:?})",
                            col.collection,
                            col.items.len(),
                            col.state
                        )),
                    )
                    .children(col.items.iter().take(20).map(|item| {
                        let item_ref = item.reference.clone();
                        h_flex()
                            .justify_between()
                            .items_center()
                            .py_1()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(div().truncate().child(item.reference.name.clone()))
                            .child(
                                h_flex()
                                    .gap_1p5()
                                    .items_center()
                                    .child(Tag::secondary().child(item.status.clone()))
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "col-item-{:?}-{}",
                                            col.collection, item.reference.name
                                        )))
                                        .xsmall()
                                        .ghost()
                                        .label("Open")
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.open_resource_inspector(
                                                item_ref.clone(),
                                                window,
                                                cx,
                                            );
                                        })),
                                    ),
                            )
                    }))
            }))
    }

    fn render_inspector_yaml(
        &self,
        detail: &ResourceDetail,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_pod = detail.reference.kind == ResourceKind::Pods;
        v_flex()
            .flex_1()
            .p_3()
            .gap_2()
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Sanitized editable YAML projection"),
                    )
                    .child(
                        h_flex()
                            .gap_1p5()
                            .when(!is_pod, |el| {
                                el.child(
                                    Button::new("yaml-dry-run-btn")
                                        .xsmall()
                                        .outline()
                                        .label("Server Dry-Run")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.submit_editable_yaml(
                                                EditMode::DryRun,
                                                window,
                                                cx,
                                            );
                                        })),
                                )
                            })
                            .child(
                                Button::new("yaml-apply-btn")
                                    .xsmall()
                                    .primary()
                                    .label("Apply Changes")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.submit_editable_yaml(EditMode::Apply, window, cx);
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(px(6.0))
                    .overflow_hidden()
                    .child(Editor::new(&self.inspector_yaml_editor).h_full()),
            )
    }
}

impl InspectorState {
    fn pod_actions(
        &self,
        _reference: &ResourceReference,
        _cx: &mut Context<IceonApp>,
    ) -> Option<AnyElement> {
        None
    }
}

fn supported_collections(kind: ResourceKind) -> Vec<CollectionKind> {
    let mut out = vec![CollectionKind::Events];
    match kind {
        ResourceKind::Nodes
        | ResourceKind::ReplicaSets
        | ResourceKind::DaemonSets
        | ResourceKind::StatefulSets
        | ResourceKind::ReplicationControllers
        | ResourceKind::Jobs => out.insert(0, CollectionKind::Pods),
        ResourceKind::Deployments => {
            out.insert(0, CollectionKind::ReplicaSets);
            out.insert(0, CollectionKind::Pods);
        }
        ResourceKind::CronJobs => out.insert(0, CollectionKind::Jobs),
        ResourceKind::Services => out.insert(0, CollectionKind::EndpointSlices),
        ResourceKind::Namespaces => {
            out.push(CollectionKind::ResourceQuotas);
            out.push(CollectionKind::LimitRanges);
        }
        _ => {}
    }
    out
}
