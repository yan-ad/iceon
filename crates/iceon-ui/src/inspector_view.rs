//! Right-hand Resource & Helm Inspector drawer with Summary, Metadata, Related, Events, and YAML tabs.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Selectable as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariant, ButtonVariants as _},
    h_flex,
    input::Input,
    scroll::ScrollableElement as _,
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use iceon_core::connection::{
    NodeInfo, ResourceKind,
    delete::DeleteRequest,
    detail_collections::{CollectionKind, CollectionRequest, CollectionState, ResourceCollection},
    details::{DetailRequest, ResourceDetail, ResourceReference},
    helm::{HelmDetail, HelmDetailRequest, HelmView},
    node_actions::{NodeAction, NodeActionRequest},
    node_drain::{
        DrainDisposition, DrainExecuteRequest, DrainOutcomeState, DrainResult, DrainReview,
        DrainReviewRequest, DrainState,
    },
    pod_runtime::PodStreamMode,
    pods::{PodAction, PodDetails, PodEventsState, PodMutationRequest, PodRequest},
    resource_edit::{EditMode, EditRequest, EditResult},
    service_forward::ServiceForwardRequest,
    workload_actions::{WorkflowRequest, WorkloadAction},
};

use crate::IceonApp;

#[path = "editor_surface.rs"]
pub(crate) mod editor_surface;
use editor_surface::EditorSurface;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteTarget {
    pub kind: ResourceKind,
    pub name: String,
    pub namespace: Option<String>,
    pub uid: String,
    pub resource_version: String,
}

fn workload_requires_confirmation(action: WorkloadAction) -> bool {
    matches!(action, WorkloadAction::Restart | WorkloadAction::Suspend)
}

pub fn supported_collections_for_kind(kind: ResourceKind) -> Vec<CollectionKind> {
    use ResourceKind::*;
    let mut out = match kind {
        Nodes | ReplicaSets | DaemonSets | StatefulSets | ReplicationControllers | Jobs => {
            vec![CollectionKind::Pods]
        }
        Deployments => vec![CollectionKind::Pods, CollectionKind::ReplicaSets],
        CronJobs => vec![CollectionKind::Jobs],
        Services => vec![CollectionKind::EndpointSlices],
        Namespaces => vec![CollectionKind::ResourceQuotas, CollectionKind::LimitRanges],
        _ => Vec::new(),
    };
    out.push(CollectionKind::Events);
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum InspectorTab {
    #[default]
    Summary,
    Metadata,
    Relations,
    Events,
    Yaml,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InspectorTarget {
    Resource(ResourceReference),
    Helm {
        view: HelmView,
        name: String,
        namespace: Option<String>,
        version: Option<String>,
    },
}

#[derive(Default)]
pub struct InspectorState {
    pub target: Option<InspectorTarget>,
    pub tab: InspectorTab,
    pub loading: bool,
    pub error: Option<String>,
    pub action_message: Option<String>,
    pub detail: Option<ResourceDetail>,
    pub pod_detail: Option<PodDetails>,
    pub collections: Vec<ResourceCollection>,
    pub edit_load: Option<EditResult>,
    pub helm_detail: Option<HelmDetail>,
    pub drain_review: Option<DrainReview>,
    pub drain_result: Option<DrainResult>,
}

impl IceonApp {
    /// Standard window-level confirmation, never an inspector-local overlay.
    /// Capture the lease so an old dialog cannot mutate a newly selected cluster.
    pub(crate) fn confirm_destructive_action(
        &self,
        title: impl Into<SharedString>,
        description: impl Into<SharedString>,
        confirm_label: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
        action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let title = title.into();
        let description = description.into();
        let app = cx.entity().downgrade();
        let lease = self
            .cluster_session
            .as_ref()
            .map(|session| (session.cluster_id.clone(), session.session_id.clone()));
        let action = std::rc::Rc::new(action);
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let app = app.clone();
            let lease = lease.clone();
            let action = action.clone();
            dialog
                .centered()
                .confirm()
                .title(title.clone())
                .description(description.clone())
                .ok_text(confirm_label)
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, window, cx| {
                    let _ = app.update(cx, |this, cx| {
                        let current = this.cluster_session.as_ref().map(|session| {
                            (session.cluster_id.clone(), session.session_id.clone())
                        });
                        if current == lease {
                            action(this, window, cx);
                        }
                    });
                    true
                })
        });
    }

    pub(crate) fn open_resource_inspector(
        &mut self,
        reference: ResourceReference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.inspector.target = Some(InspectorTarget::Resource(reference.clone()));
        self.inspector_open = true;
        self.inspector.loading = true;
        self.inspector.error = None;
        self.inspector.action_message = None;
        self.inspector.detail = None;
        self.inspector.pod_detail = None;
        self.inspector.collections.clear();
        self.inspector.edit_load = None;
        self.inspector.helm_detail = None;
        self.inspector.drain_review = None;
        self.inspector.drain_result = None;
        cx.notify();

        let Some(session) = self.cluster_session.clone() else {
            self.inspector.loading = false;
            self.inspector.error = Some("No active cluster session.".into());
            cx.notify();
            return;
        };

        let read_id = self.next_operation_id();
        let connection = self.connection.clone();
        let req = DetailRequest {
            cluster_id: session.cluster_id.clone(),
            session_id: session.session_id.clone(),
            read_id,
            reference: reference.clone(),
        };

        let is_pod = reference.kind == ResourceKind::Pods;
        let pod_ns = reference.namespace.clone().unwrap_or_default();
        let pod_name = reference.name.clone();
        let pod_uid = reference.uid.clone();
        let col_kinds = supported_collections_for_kind(reference.kind);
        let col_read_ids: Vec<(CollectionKind, u64)> = col_kinds
            .into_iter()
            .map(|c| (c, self.next_operation_id()))
            .collect();

        cx.spawn_in(window, async move |this, cx| {
            let detail_res = connection.resource_detail(req).await;
            let pod_res = if is_pod {
                Some(
                    connection
                        .pod_detail(PodRequest {
                            cluster_id: session.cluster_id.clone(),
                            session_id: session.session_id.clone(),
                            namespace: pod_ns,
                            name: pod_name,
                            uid: pod_uid,
                        })
                        .await,
                )
            } else {
                None
            };

            let mut collections = Vec::new();
            if let Ok(ref detail) = detail_res
                && detail.reference.uid.is_some()
            {
                for (collection, cid) in col_read_ids {
                    let creq = CollectionRequest {
                        cluster_id: session.cluster_id.clone(),
                        session_id: session.session_id.clone(),
                        read_id: cid,
                        reference: detail.reference.clone(),
                        collection,
                    };
                    if let Ok(col) = connection.resource_collection(creq).await {
                        collections.push(col);
                    }
                }
            }

            let _ = this.update_in(cx, |this, window, cx| {
                this.inspector.loading = false;
                match detail_res {
                    Ok(detail) => {
                        this.inspector.detail = Some(detail);
                    }
                    Err(err) => {
                        this.inspector.error = Some(format!("Inspector load failed: {err:?}"));
                    }
                }
                if let Some(Ok(pod)) = pod_res {
                    this.inspector.pod_detail = Some(pod);
                }
                this.inspector.collections = collections;
                if this.inspector.tab == InspectorTab::Yaml {
                    this.load_inspector_yaml(window, cx);
                }
                cx.notify();
            });
        })
        .detach();
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
        self.inspector_open = true;
        self.inspector.target = Some(InspectorTarget::Helm {
            view,
            name: name.clone(),
            namespace: namespace.clone(),
            version: version.clone(),
        });
        self.inspector.tab = InspectorTab::Summary;
        self.inspector.loading = true;
        self.inspector.error = None;
        self.inspector.action_message = None;
        self.inspector.detail = None;
        self.inspector.pod_detail = None;
        self.inspector.collections.clear();
        self.inspector.helm_detail = None;
        cx.notify();

        let Some(session) = self.cluster_session.clone() else {
            self.inspector.loading = false;
            return;
        };
        let read_id = self.next_operation_id();
        let helm_preferences =
            iceon_core::connection::helm::HelmPreferences::from(&self.preferences_draft);
        let connection = self.connection.clone();
        let req = HelmDetailRequest {
            cluster_id: session.cluster_id,
            session_id: session.session_id,
            read_id,
            view,
            name,
            namespace,
            version,
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = connection
                .helm_detail_with_preferences(req, helm_preferences)
                .await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.inspector.loading = false;
                match result {
                    Ok(detail) => this.inspector.helm_detail = Some(detail),
                    Err(err) => this.inspector.error = Some(format!("Helm detail error: {err:?}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn load_inspector_yaml(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.inspector.edit_load = None;
        self.inspector.action_message = Some("Loading YAML…".into());
        cx.notify();
        // Detail requests made from tables can contain only kind/name.  The
        // detail response also carries the UID on the item, so always build
        // the edit reference from the freshest identity we have.  The edit
        // endpoint deliberately rejects a missing UID (it is part of the
        // optimistic-concurrency guard).
        let reference = self
            .inspector
            .detail
            .as_ref()
            .and_then(|d| {
                let mut reference = d.reference.clone();
                if reference.uid.is_none() {
                    reference.uid = d.item.uid.clone();
                }
                reference.uid.as_deref().filter(|uid| !uid.is_empty())?;
                Some(reference)
            })
            .or_else(|| match self.inspector.target.clone() {
                Some(InspectorTarget::Resource(r)) if r.uid.is_some() => Some(r),
                _ => None,
            });
        let Some(reference) = reference else {
            self.inspector.action_message = Some(
                "YAML unavailable: inspector detail has no UID. Refresh the resource and try again."
                    .into(),
            );
            cx.notify();
            return;
        };
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let op_id = self.next_operation_id();
        let connection = self.connection.clone();
        let target = self.inspector.target.clone();
        let req = EditRequest {
            cluster_id: session.cluster_id,
            session_id: session.session_id,
            operation_id: op_id,
            reference,
            resource_version: None,
            yaml: None,
            mode: EditMode::Read,
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = connection.resource_edit(req).await;
            let _ = this.update_in(cx, |this, window, cx| {
                // A slow edit read must not populate the editor for a
                // resource selected after the request was started.
                if this.inspector.target != target {
                    return;
                }
                match result {
                    Ok(loaded) => {
                        let yaml = loaded.yaml.clone().unwrap_or_default();
                        this.inspector.edit_load = Some(loaded);
                        this.inspector_yaml_editor.update(cx, |editor, cx| {
                            editor.set_value(yaml, window, cx);
                        });
                    }
                    Err(err) => {
                        this.inspector.action_message =
                            Some(match err {
                                iceon_core::connection::ConnectionError::InvalidInput =>
                                    "YAML load unavailable: inspector reference is missing a UID or is stale. Refresh the resource and try again.".into(),
                                other => format!("YAML load unavailable: {other:?}"),
                            });
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_inspector_yaml(&mut self, dry_run: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(loaded) = self.inspector.edit_load.clone() else {
            return;
        };
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let yaml = self.inspector_yaml_editor.read(cx).value().to_string();
        if loaded.reference.uid.as_deref().is_none_or(str::is_empty) {
            self.inspector.action_message = Some(
                "Cannot apply YAML: loaded inspector reference has no UID. Reload the resource."
                    .into(),
            );
            cx.notify();
            return;
        }
        if loaded.resource_version.is_empty() {
            self.inspector.action_message = Some(
                "Cannot apply YAML: loaded resource has no resourceVersion. Reload the resource."
                    .into(),
            );
            cx.notify();
            return;
        }
        let op_id = self.next_operation_id();
        let connection = self.connection.clone();
        let target = self.inspector.target.clone();
        let req = EditRequest {
            cluster_id: session.cluster_id,
            session_id: session.session_id,
            operation_id: op_id,
            reference: loaded.reference.clone(),
            resource_version: Some(loaded.resource_version),
            yaml: Some(yaml),
            mode: if dry_run {
                EditMode::DryRun
            } else {
                EditMode::Apply
            },
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = connection.resource_edit(req).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.inspector.target != target {
                    return;
                }
                match result {
                    Ok(applied) => {
                        let was_dry_run = applied.mode == EditMode::DryRun;
                        this.inspector.action_message = Some(if was_dry_run {
                            format!("Dry-run validated (rv {})", applied.resource_version)
                        } else {
                            format!("Applied (rv {})", applied.resource_version)
                        });
                        if !was_dry_run {
                            this.load_inspector_yaml(window, cx);
                            this.reload_active_section(window, cx);
                        }
                    }
                    Err(err) => {
                        this.inspector.action_message = Some(format!("YAML apply failed: {err:?}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn confirm_inspector_pod_mutation(
        &self,
        namespace: String,
        name: String,
        uid: String,
        resource_version: Option<String>,
        action: PodAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_destructive_action(
            "Confirm destructive Pod action",
            format!("{action:?} {namespace}/{name}? This bypasses normal Pod cleanup and may leave workloads or resources running."),
            "Confirm", window, cx,
            move |this, window, cx| this.run_pod_mutation(
                namespace.clone(), name.clone(), uid.clone(), resource_version.clone(), action, window, cx,
            ),
        );
    }

    pub(crate) fn run_pod_mutation(
        &mut self,
        namespace: String,
        name: String,
        uid: String,
        resource_version: Option<String>,
        action: PodAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let Some(rv) = resource_version else {
            self.inspector.action_message =
                Some("Cannot mutate pod without resourceVersion.".into());
            cx.notify();
            return;
        };
        let connection = self.connection.clone();
        let req = PodMutationRequest {
            pod: PodRequest {
                cluster_id: session.cluster_id,
                session_id: session.session_id,
                namespace: namespace.clone(),
                name: name.clone(),
                uid: Some(uid),
            },
            action,
            resource_version: rv,
            yaml: None,
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = connection.pod_mutate(req).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(()) => {
                        this.inspector.action_message =
                            Some(format!("Pod {:?} completed for {namespace}/{name}", action));
                        if matches!(action, PodAction::Delete | PodAction::ForceDelete) {
                            this.inspector.target = None;
                        } else if let Some(InspectorTarget::Resource(r)) =
                            this.inspector.target.clone()
                        {
                            this.open_resource_inspector(r, window, cx);
                        }
                        this.reload_active_section(window, cx);
                    }
                    Err(err) => {
                        this.inspector.action_message =
                            Some(format!("Pod {:?} failed: {err:?}", action));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn run_workload_action_for(
        &mut self,
        reference: ResourceReference,
        resource_version: String,
        action: WorkloadAction,
        replicas: Option<u32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let op_id = self.next_operation_id();
        let connection = self.connection.clone();
        let req = WorkflowRequest {
            cluster_id: session.cluster_id,
            session_id: session.session_id,
            operation_id: op_id,
            reference: reference.clone(),
            resource_version,
            action,
            replicas,
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = connection.resource_workload_action(req).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(res) => {
                        let msg = if let Some(created) = res.created {
                            format!("Triggered Job {}", created.name)
                        } else {
                            format!("Workload {:?} accepted", res.action)
                        };
                        this.inspector.action_message = Some(msg);
                        if this.inspector.target
                            == Some(InspectorTarget::Resource(reference.clone()))
                        {
                            this.open_resource_inspector(reference, window, cx);
                        }
                        this.reload_active_section(window, cx);
                    }
                    Err(err) => {
                        this.inspector.action_message =
                            Some(format!("Action {action:?} failed: {err:?}"));
                        this.cluster_error = Some(format!("Workload {action:?} failed: {err:?}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn run_workload_action(
        &mut self,
        action: WorkloadAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(detail) = self.inspector.detail.clone() else {
            return;
        };
        let Some(rv) = detail.item.resource_version.clone() else {
            self.inspector.action_message = Some("Missing resourceVersion".into());
            cx.notify();
            return;
        };
        let Some(uid) = detail.reference.uid.clone().or(detail.item.uid.clone()) else {
            self.inspector.action_message = Some("Missing resource UID".into());
            cx.notify();
            return;
        };
        let replicas = if action == WorkloadAction::Scale {
            self.scale_replicas_input
                .read(cx)
                .value()
                .trim()
                .parse::<u32>()
                .ok()
        } else {
            None
        };
        let reference = ResourceReference {
            kind: detail.reference.kind,
            name: detail.reference.name,
            namespace: detail.reference.namespace,
            uid: Some(uid),
        };
        if workload_requires_confirmation(action) {
            self.confirm_destructive_action(
                format!("{action:?} workload?"),
                format!(
                    "{action:?} {}? This can interrupt running workloads.",
                    reference.name
                ),
                "Confirm",
                window,
                cx,
                move |this, window, cx| {
                    this.run_workload_action_for(
                        reference.clone(),
                        rv.clone(),
                        action,
                        replicas,
                        window,
                        cx,
                    );
                },
            );
        } else {
            self.run_workload_action_for(reference, rv, action, replicas, window, cx);
        }
    }

    pub(crate) fn run_node_action_for(
        &mut self,
        reference: ResourceReference,
        resource_version: String,
        action: NodeAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let op_id = self.next_operation_id();
        let connection = self.connection.clone();
        let req = NodeActionRequest {
            cluster_id: session.cluster_id,
            session_id: session.session_id,
            operation_id: op_id,
            reference: reference.clone(),
            resource_version,
            action,
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = connection.resource_node_action(req).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(_) => {
                        this.inspector.action_message = Some(format!("Node {action:?} completed"));
                        if this.inspector.target
                            == Some(InspectorTarget::Resource(reference.clone()))
                        {
                            this.open_resource_inspector(reference, window, cx);
                        }
                        this.reload_active_section(window, cx);
                    }
                    Err(err) => {
                        this.inspector.action_message =
                            Some(format!("Node {action:?} failed: {err:?}"));
                        this.cluster_error = Some(format!("Node {action:?} failed: {err:?}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn run_node_action(&mut self, action: NodeAction, window: &mut Window, cx: &mut Context<Self>) {
        let Some(detail) = self.inspector.detail.clone() else {
            return;
        };
        let (Some(uid), Some(rv)) = (
            detail.reference.uid.clone().or(detail.item.uid.clone()),
            detail.item.resource_version.clone(),
        ) else {
            return;
        };
        let reference = ResourceReference {
            kind: ResourceKind::Nodes,
            name: detail.reference.name,
            namespace: None,
            uid: Some(uid),
        };
        self.run_node_action_for(reference, rv, action, window, cx);
    }

    pub(crate) fn review_node_drain_for(
        &mut self,
        reference: ResourceReference,
        resource_version: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        if self.inspector.target != Some(InspectorTarget::Resource(reference.clone())) {
            self.open_resource_inspector(reference.clone(), window, cx);
        }
        let op_id = self.next_operation_id();
        let connection = self.connection.clone();
        let req = DrainReviewRequest {
            cluster_id: session.cluster_id,
            session_id: session.session_id,
            operation_id: op_id,
            reference,
            resource_version,
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = connection.node_drain_review(req).await;
            let _ = this.update_in(cx, |this, _, cx| {
                match result {
                    Ok(review) => {
                        this.inspector.drain_result = None;
                        this.inspector.drain_review = Some(review);
                    }
                    Err(err) => {
                        this.inspector.action_message =
                            Some(format!("Drain review failed: {err:?}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn review_node_drain(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(detail) = self.inspector.detail.clone() else {
            return;
        };
        let (Some(uid), Some(rv)) = (
            detail.reference.uid.clone().or(detail.item.uid.clone()),
            detail.item.resource_version.clone(),
        ) else {
            return;
        };
        let reference = ResourceReference {
            kind: ResourceKind::Nodes,
            name: detail.reference.name,
            namespace: None,
            uid: Some(uid),
        };
        self.review_node_drain_for(reference, rv, window, cx);
    }

    fn execute_node_drain(&mut self, plan_id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let op_id = self.next_operation_id();
        let connection = self.connection.clone();
        let req = DrainExecuteRequest {
            cluster_id: session.cluster_id,
            session_id: session.session_id,
            operation_id: op_id,
            plan_id,
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = connection.node_drain_execute(req).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(res) => {
                        this.inspector.action_message = Some(format!(
                            "Node drain {:?} (cordoned={})",
                            res.state, res.cordoned
                        ));
                        this.inspector.drain_review = None;
                        this.inspector.drain_result = Some(res);
                        this.reload_active_section(window, cx);
                    }
                    Err(err) => {
                        this.inspector.action_message =
                            Some(format!("Drain execute failed: {err:?}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn start_service_forward_for(
        &mut self,
        reference: ResourceReference,
        resource_version: String,
        service_port: u16,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let op_id = self.next_operation_id();
        let connection = self.connection.clone();
        let req = ServiceForwardRequest {
            cluster_id: session.cluster_id,
            session_id: session.session_id,
            operation_id: op_id,
            reference,
            resource_version,
            service_port,
            local_port: 0,
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = connection.service_forward_start(req).await;
            let _ = this.update_in(cx, |this, _, cx| {
                match result {
                    Ok(res) => {
                        this.inspector.action_message = Some(format!(
                            "Service port {} forwarded on local port {}",
                            res.service_port, res.forward.local_port
                        ));
                        this.port_forwards.push(res.forward);
                        this.pod_dock_open = true;
                    }
                    Err(err) => {
                        this.inspector.action_message =
                            Some(format!("Service forward failed: {err:?}"));
                        this.cluster_error = Some(format!("Service forward failed: {err:?}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn start_service_forward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(detail) = self.inspector.detail.clone() else {
            return;
        };
        let (Some(uid), Some(rv)) = (
            detail.reference.uid.clone().or(detail.item.uid.clone()),
            detail.item.resource_version.clone(),
        ) else {
            return;
        };
        let service_port = self
            .service_port_input
            .read(cx)
            .value()
            .trim()
            .parse::<u16>()
            .unwrap_or(80);
        let reference = ResourceReference {
            kind: ResourceKind::Services,
            name: detail.reference.name,
            namespace: detail.reference.namespace,
            uid: Some(uid),
        };
        self.start_service_forward_for(reference, rv, service_port, window, cx);
    }

    pub(crate) fn delete_resource_by_target(
        &mut self,
        target: DeleteTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let read_id = self.next_operation_id();
        let connection = self.connection.clone();
        let req = DeleteRequest {
            cluster_id: session.cluster_id,
            session_id: session.session_id,
            kind: target.kind,
            namespace: target.namespace.clone(),
            name: target.name.clone(),
            uid: target.uid.clone(),
            resource_version: target.resource_version.clone(),
            read_id,
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = connection.delete(req).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(res) => {
                        if let Some(InspectorTarget::Resource(r)) = &this.inspector.target
                            && r.kind == res.kind
                            && r.name == res.name
                            && r.namespace == res.namespace
                        {
                            this.inspector.target = None;
                        }
                        this.reload_active_section(window, cx);
                    }
                    Err(err) => {
                        this.inspector.action_message = Some(format!("Delete failed: {err:?}"));
                        this.cluster_error =
                            Some(format!("Failed to delete {}: {err:?}", target.name));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn delete_inspected_resource(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(detail) = self.inspector.detail.clone() else {
            return;
        };
        let (Some(uid), Some(rv)) = (
            detail.reference.uid.clone().or(detail.item.uid.clone()),
            detail.item.resource_version.clone(),
        ) else {
            self.inspector.action_message =
                Some("Cannot delete without UID and resourceVersion".into());
            cx.notify();
            return;
        };
        let target = DeleteTarget {
            kind: detail.reference.kind,
            name: detail.reference.name,
            namespace: detail.reference.namespace,
            uid,
            resource_version: rv,
        };
        self.confirm_destructive_action(
            "Delete resource?",
            format!("Delete {}? This action cannot be undone.", target.name),
            "Delete",
            window,
            cx,
            move |this, window, cx| this.delete_resource_by_target(target.clone(), window, cx),
        );
    }

    pub(crate) fn render_inspector_drawer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let title = match &self.inspector.target {
            Some(InspectorTarget::Resource(r)) => format!("{:?} · {}", r.kind, r.name),
            Some(InspectorTarget::Helm { name, .. }) => format!("Helm · {name}"),
            None => "Inspector".into(),
        };
        let is_resource = matches!(self.inspector.target, Some(InspectorTarget::Resource(_)));
        let active_tab = self.inspector.tab;

        let event_count = self
            .inspector
            .pod_detail
            .as_ref()
            .map(|p| p.events.len())
            .unwrap_or(0)
            + self
                .inspector
                .collections
                .iter()
                .filter(|c| c.collection == CollectionKind::Events)
                .map(|c| c.items.len())
                .sum::<usize>();

        let related_count = self
            .inspector
            .detail
            .as_ref()
            .map(|d| d.related.len())
            .unwrap_or(0)
            + self
                .inspector
                .collections
                .iter()
                .filter(|c| c.collection != CollectionKind::Events)
                .map(|c| c.items.len())
                .sum::<usize>();

        v_flex()
            .w_full()
            .flex_shrink_0()
            .h_full()
            .border_l_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .overflow_hidden()
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_2p5()
                    .justify_between()
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().table_head)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(title),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .when(is_resource, |el| {
                                el.child(
                                    Button::new("inspector-refresh-btn")
                                        .xsmall()
                                        .ghost()
                                        .icon(IconName::Undo2)
                                        .tooltip("Refresh Inspector")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            if let Some(InspectorTarget::Resource(r)) =
                                                this.inspector.target.clone()
                                            {
                                                this.open_resource_inspector(r, window, cx);
                                            }
                                        })),
                                )
                            })
                            .child(
                                Button::new("inspector-close-btn")
                                    .xsmall()
                                    .ghost()
                                    .icon(IconName::Close)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.inspector.target = None;
                                        this.inspector_open = false;
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .when(is_resource, |el| {
                el.child(
                    h_flex()
                        .w_full()
                        .px_3()
                        .py_1p5()
                        .gap_1()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().table_head)
                        .children(
                            [
                                (InspectorTab::Summary, "Summary".to_string()),
                                (InspectorTab::Metadata, "Metadata".to_string()),
                                (
                                    InspectorTab::Relations,
                                    if related_count > 0 {
                                        format!("Related ({related_count})")
                                    } else {
                                        "Related".to_string()
                                    },
                                ),
                                (
                                    InspectorTab::Events,
                                    if event_count > 0 {
                                        format!("Events ({event_count})")
                                    } else {
                                        "Events".to_string()
                                    },
                                ),
                                (InspectorTab::Yaml, "YAML".to_string()),
                            ]
                            .into_iter()
                            .map(|(tab, label)| {
                                Button::new(SharedString::from(format!("insp-tab-{label}")))
                                    .xsmall()
                                    .ghost()
                                    .selected(active_tab == tab)
                                    .label(label)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.inspector.tab = tab;
                                        if tab == InspectorTab::Yaml
                                            && this.inspector.edit_load.is_none()
                                        {
                                            this.load_inspector_yaml(window, cx);
                                        }
                                        cx.notify();
                                    }))
                            }),
                        ),
                )
            })
            .when_some(self.inspector.detail.clone(), |el, detail| {
                el.child(self.render_inspector_action_bar(&detail, window, cx))
            })
            .when_some(self.inspector.action_message.clone(), |el, msg| {
                el.child(
                    div()
                        .w_full()
                        .px_3()
                        .py_1p5()
                        .bg(cx.theme().secondary)
                        .text_xs()
                        .child(msg),
                )
            })
            .when_some(self.inspector.error.clone(), |el, err| {
                el.child(crate::empty_state::error_state(err))
            })
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .p_3()
                    .gap_3()
                    .overflow_y_scrollbar()
                    .child(if self.inspector.loading {
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("Loading resource details...")
                            .into_any_element()
                    } else if let Some(helm) = self.inspector.helm_detail.clone() {
                        self.render_helm_inspector(&helm, cx).into_any_element()
                    } else if let Some(detail) = self.inspector.detail.clone() {
                        match active_tab {
                            InspectorTab::Summary => self
                                .render_inspector_summary(&detail, window, cx)
                                .into_any_element(),
                            InspectorTab::Metadata => self
                                .render_inspector_metadata(&detail, cx)
                                .into_any_element(),
                            InspectorTab::Relations => self
                                .render_inspector_relations(&detail, cx)
                                .into_any_element(),
                            InspectorTab::Events => {
                                self.render_inspector_events(&detail, cx).into_any_element()
                            }
                            InspectorTab::Yaml => self
                                .render_inspector_yaml(&detail, window, cx)
                                .into_any_element(),
                        }
                    } else {
                        crate::empty_state::empty_state(
                            IconName::Search,
                            "No resource selected",
                            "Select a resource from the table to inspect it.",
                        )
                        .into_any_element()
                    }),
            )
    }

    fn render_inspector_action_bar(
        &self,
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
        let is_pod = kind == ResourceKind::Pods;

        h_flex()
            .w_full()
            .px_3()
            .py_2()
            .gap_1p5()
            .flex_wrap()
            .items_center()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().secondary)
            .when(can_scale, |el| el.child(self.scale_controls(cx)))
            .when(can_restart, |el| {
                el.child(
                    Button::new("insp-restart-btn")
                        .xsmall()
                        .danger()
                        .label("Restart")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.run_workload_action(WorkloadAction::Restart, window, cx);
                        })),
                )
            })
            .when(can_suspend, |el| {
                el.child(
                    Button::new("insp-suspend-btn")
                        .xsmall()
                        .danger()
                        .label("Suspend")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.run_workload_action(WorkloadAction::Suspend, window, cx);
                        })),
                )
                .child(
                    Button::new("insp-resume-btn")
                        .xsmall()
                        .outline()
                        .label("Resume")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.run_workload_action(WorkloadAction::Resume, window, cx);
                        })),
                )
            })
            .when(can_trigger, |el| {
                el.child(
                    Button::new("insp-trigger-btn")
                        .xsmall()
                        .primary()
                        .label("Trigger Run")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.run_workload_action(WorkloadAction::Trigger, window, cx);
                        })),
                )
            })
            .when(is_node, |el| el.child(self.node_controls(cx)))
            .when(is_service, |el| el.child(self.service_controls(cx)))
            .when_some(
                is_pod.then(|| self.pod_actions(detail, cx)).flatten(),
                |el, pod_bar| el.child(pod_bar),
            )
            .child(
                Button::new("insp-open-yaml-btn")
                    .xsmall()
                    .ghost()
                    .label("Edit YAML")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.inspector.tab = InspectorTab::Yaml;
                        if this.inspector.edit_load.is_none() {
                            this.load_inspector_yaml(window, cx);
                        }
                        cx.notify();
                    })),
            )
            .child(
                Button::new("insp-delete-btn")
                    .xsmall()
                    .danger()
                    .label("Delete")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.delete_inspected_resource(window, cx);
                    })),
            )
    }

    fn scale_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_1()
            .items_center()
            .child(
                div()
                    .w(px(64.0))
                    .child(Input::new(&self.scale_replicas_input).small()),
            )
            .child(
                Button::new("insp-scale-btn")
                    .xsmall()
                    .primary()
                    .label("Scale")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.run_workload_action(WorkloadAction::Scale, window, cx);
                    })),
            )
    }

    fn node_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_1()
            .items_center()
            .child(
                Button::new("insp-cordon-btn")
                    .xsmall()
                    .outline()
                    .label("Cordon")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.run_node_action(NodeAction::Cordon, window, cx);
                    })),
            )
            .child(
                Button::new("insp-uncordon-btn")
                    .xsmall()
                    .outline()
                    .label("Uncordon")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.run_node_action(NodeAction::Uncordon, window, cx);
                    })),
            )
            .child(
                Button::new("insp-drain-review-btn")
                    .xsmall()
                    .danger()
                    .label("Drain")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.review_node_drain(window, cx);
                    })),
            )
    }

    fn service_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_1()
            .items_center()
            .child(
                div()
                    .w(px(72.0))
                    .child(Input::new(&self.service_port_input).small()),
            )
            .child(
                Button::new("insp-svc-fwd-btn")
                    .xsmall()
                    .primary()
                    .label("Port-Forward")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.start_service_forward(window, cx);
                    })),
            )
    }

    fn pod_actions(&self, detail: &ResourceDetail, cx: &mut Context<Self>) -> Option<AnyElement> {
        let pod = self.inspector.pod_detail.as_ref()?;
        let ns = pod.namespace.clone();
        let name = pod.name.clone();
        let uid = pod.uid.clone();
        let rv = Some(pod.resource_version.clone())
            .filter(|s| !s.is_empty())
            .or_else(|| detail.item.resource_version.clone());
        let primary_container = pod
            .containers
            .iter()
            .find(|c| c.kind == "container")
            .or_else(|| pod.containers.first())
            .map(|c| c.name.clone());
        let has_finalizers = detail
            .metadata
            .as_ref()
            .is_some_and(|m| !m.finalizers.is_empty());
        let is_terminating = pod.deletion_timestamp.is_some()
            || detail
                .metadata
                .as_ref()
                .is_some_and(|m| m.deletion_timestamp.is_some());

        let ns_logs = ns.clone();
        let name_logs = name.clone();
        let uid_logs = uid.clone();
        let ns_shell = ns.clone();
        let name_shell = name.clone();
        let uid_shell = uid.clone();
        let ns_attach = ns.clone();
        let name_attach = name.clone();
        let uid_attach = uid.clone();
        let ns_force = ns.clone();
        let name_force = name.clone();
        let uid_force = uid.clone();
        let rv_force = rv.clone();
        let ns_fin = ns;
        let name_fin = name;
        let uid_fin = uid;
        let rv_fin = rv;

        Some(
            h_flex()
                .gap_1()
                .items_center()
                .when_some(primary_container, |row, cname| {
                    let c_logs = cname.clone();
                    let c_shell = cname.clone();
                    let c_attach = cname;
                    row.child(
                        Button::new("insp-pod-quick-logs")
                            .xsmall()
                            .primary()
                            .label("Logs")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_pod_stream(
                                    ns_logs.clone(),
                                    name_logs.clone(),
                                    uid_logs.clone(),
                                    c_logs.clone(),
                                    PodStreamMode::Logs,
                                    window,
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new("insp-pod-quick-shell")
                            .xsmall()
                            .outline()
                            .label("Shell")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_pod_stream(
                                    ns_shell.clone(),
                                    name_shell.clone(),
                                    uid_shell.clone(),
                                    c_shell.clone(),
                                    PodStreamMode::Shell,
                                    window,
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new("insp-pod-quick-attach")
                            .xsmall()
                            .ghost()
                            .label("Attach")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_pod_stream(
                                    ns_attach.clone(),
                                    name_attach.clone(),
                                    uid_attach.clone(),
                                    c_attach.clone(),
                                    PodStreamMode::Attach,
                                    window,
                                    cx,
                                );
                            })),
                    )
                })
                .child(
                    Button::new("insp-pod-force-delete")
                        .xsmall()
                        .danger()
                        .label("Force Delete (0s)")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.confirm_inspector_pod_mutation(
                                ns_force.clone(),
                                name_force.clone(),
                                uid_force.clone(),
                                rv_force.clone(),
                                PodAction::ForceDelete,
                                window,
                                cx,
                            );
                        })),
                )
                .when(is_terminating && has_finalizers, |row| {
                    row.child(
                        Button::new("insp-pod-strip-finalizers")
                            .xsmall()
                            .danger()
                            .label("Strip Finalizers")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.confirm_inspector_pod_mutation(
                                    ns_fin.clone(),
                                    name_fin.clone(),
                                    uid_fin.clone(),
                                    rv_fin.clone(),
                                    PodAction::Finalize,
                                    window,
                                    cx,
                                );
                            })),
                    )
                })
                .into_any_element(),
        )
    }

    fn render_inspector_summary(
        &self,
        detail: &ResourceDetail,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let uid_opt = detail.reference.uid.clone().or(detail.item.uid.clone());
        let uid_copy = uid_opt.clone().unwrap_or_default();
        let ns_ref = detail
            .reference
            .namespace
            .as_ref()
            .map(|ns| ResourceReference {
                kind: ResourceKind::Namespaces,
                name: ns.clone(),
                namespace: None,
                uid: None,
            });
        let node_info: Option<NodeInfo> = detail.item.node.clone().or_else(|| {
            (detail.reference.kind == ResourceKind::Nodes)
                .then(|| {
                    self.resource_page
                        .as_ref()
                        .filter(|p| p.kind == ResourceKind::Nodes)
                        .and_then(|p| p.items.iter().find(|i| i.name == detail.reference.name))
                        .and_then(|i| i.node.clone())
                })
                .flatten()
        });

        v_flex()
            .w_full()
            .gap_3()
            // Identity & Status card
            .child(
                v_flex()
                    .w_full()
                    .p_3()
                    .gap_1p5()
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().secondary)
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(detail.reference.name.clone()),
                            )
                            .child(status_tag(&detail.item.status)),
                    )
                    .when_some(detail.reference.namespace.clone(), |el, ns| {
                        el.child(
                            h_flex()
                                .justify_between()
                                .items_center()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!("Namespace: {ns}")),
                                )
                                .when_some(ns_ref, |row, target_ns| {
                                    row.child(
                                        Button::new("insp-jump-namespace")
                                            .xsmall()
                                            .ghost()
                                            .label("Inspect NS")
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.open_resource_inspector(
                                                    target_ns.clone(),
                                                    window,
                                                    cx,
                                                );
                                            })),
                                    )
                                }),
                        )
                    })
                    .when_some(detail.item.created_at.clone(), |el, created| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!(
                                    "Created: {}",
                                    crate::preference_runtime::timestamp(
                                        &created,
                                        &self.preferences_draft.app.timezone
                                    )
                                )),
                        )
                    })
                    .when_some(uid_opt, |el, uid| {
                        el.child(
                            h_flex()
                                .justify_between()
                                .items_center()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!("UID: {uid}")),
                                )
                                .child(
                                    Button::new("insp-copy-uid-btn")
                                        .xsmall()
                                        .ghost()
                                        .label("Copy UID")
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                uid_copy.clone(),
                                            ));
                                        })),
                                ),
                        )
                    })
                    .when_some(detail.item.resource_version.clone(), |el, rv| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("ResourceVersion: {rv}")),
                        )
                    }),
            )
            // Node Live Metrics & Conditions card when inspecting a Node
            .when_some(node_info, |el, ninfo| {
                el.child(self.render_node_info_card(&ninfo, cx))
            })
            // Node Drain Review card
            .when_some(self.inspector.drain_review.clone(), |el, review| {
                el.child(self.render_drain_review_card(&review, cx))
            })
            // Node Drain Execution Result card
            .when_some(self.inspector.drain_result.clone(), |el, result| {
                el.child(self.render_drain_result_card(&result, cx))
            })
            // Full Pod Runtime & Containers & Conditions card
            .when_some(self.inspector.pod_detail.clone(), |el, pod| {
                el.child(self.render_pod_summary_cards(&pod, cx))
            })
            // Event Diagnostic card when inspecting an Event
            .when_some(detail.event_diagnostic.clone(), |el, diag| {
                el.child(
                    v_flex()
                        .w_full()
                        .p_3()
                        .gap_1p5()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().secondary)
                        .child(
                            h_flex()
                                .justify_between()
                                .items_center()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("Event Diagnostic"),
                                )
                                .when(diag.truncated, |r| {
                                    r.child(Tag::warning().small().child("Truncated"))
                                }),
                        )
                        .when_some(diag.source, |r, src| {
                            r.child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!("Reporting Source: {src}")),
                            )
                        })
                        .when_some(diag.field_path, |r, fp| {
                            r.child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!("Field Path: {fp}")),
                            )
                        })
                        .child(div().text_xs().child(diag.message)),
                )
            })
            // Projected DetailSections for all 41 ResourceKinds
            .children(detail.sections.iter().map(|section| {
                v_flex()
                    .w_full()
                    .p_3()
                    .gap_2()
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().secondary)
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(cx.theme().muted_foreground)
                            .child(section.title.to_uppercase()),
                    )
                    .children(section.facts.iter().map(|fact| {
                        h_flex()
                            .w_full()
                            .justify_between()
                            .gap_3()
                            .items_start()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(fact.label.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(fact.value.clone()),
                            )
                    }))
            }))
    }

    fn render_node_info_card(&self, node: &NodeInfo, cx: &mut Context<Self>) -> impl IntoElement {
        let cpu_str = match (node.cpu.used, node.cpu.allocatable.or(node.cpu.capacity)) {
            (Some(u), Some(total)) if total > 0.0 => {
                format!("{u:.2} / {total:.2} cores ({:.0}%)", (u / total) * 100.0)
            }
            (None, Some(total)) => format!("{total:.2} cores allocatable"),
            _ => "Metrics unavailable".into(),
        };
        let mem_str = match (
            node.memory.used,
            node.memory.allocatable.or(node.memory.capacity),
        ) {
            (Some(u), Some(total)) if total > 0.0 => {
                let u_gi = u / (1024.0 * 1024.0 * 1024.0);
                let t_gi = total / (1024.0 * 1024.0 * 1024.0);
                format!("{u_gi:.2} / {t_gi:.2} GiB ({:.0}%)", (u / total) * 100.0)
            }
            (None, Some(total)) => {
                let t_gi = total / (1024.0 * 1024.0 * 1024.0);
                format!("{t_gi:.2} GiB allocatable")
            }
            _ => "Metrics unavailable".into(),
        };

        v_flex()
            .w_full()
            .p_3()
            .gap_1p5()
            .rounded(px(8.0))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().secondary)
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(cx.theme().muted_foreground)
                            .child("NODE TELEMETRY & CAPACITY"),
                    )
                    .child(if node.schedulable {
                        Tag::success().small().child("Schedulable")
                    } else {
                        Tag::warning().small().child("Cordoned / Unschedulable")
                    }),
            )
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("CPU Usage"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .child(cpu_str),
                    ),
            )
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Memory Usage"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .child(mem_str),
                    ),
            )
            .when(!node.internal_ips.is_empty(), |el| {
                el.child(
                    h_flex()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("Internal IPs"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .child(node.internal_ips.join(", ")),
                        ),
                )
            })
            .when_some(node.disk_allocatable.clone(), |el, disk| {
                el.child(
                    h_flex()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("Allocatable Ephemeral Storage"),
                        )
                        .child(div().text_xs().font_weight(FontWeight::MEDIUM).child(disk)),
                )
            })
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Taints Count"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .child(node.taints.to_string()),
                    ),
            )
            .when(!node.conditions.is_empty(), |el| {
                el.child(
                    h_flex()
                        .gap_1()
                        .flex_wrap()
                        .children(node.conditions.iter().map(|c| {
                            let healthy = (c.kind == "Ready" && c.status == "True")
                                || (c.kind != "Ready" && c.status == "False");
                            if healthy {
                                Tag::success()
                                    .small()
                                    .child(format!("{}: {}", c.kind, c.status))
                            } else {
                                Tag::warning()
                                    .small()
                                    .child(format!("{}: {}", c.kind, c.status))
                            }
                        })),
                )
            })
    }

    fn render_drain_review_card(
        &self,
        review: &DrainReview,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .w_full()
            .p_3()
            .gap_2()
            .rounded(px(8.0))
            .border_1()
            .border_color(cx.theme().danger)
            .bg(cx.theme().secondary)
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("Node Drain Review ({} pods)", review.pods.len())),
                    )
                    .child(if review.can_drain {
                        Tag::success().small().child("Ready to Evict")
                    } else {
                        Tag::danger().small().child("Blocked")
                    }),
            )
            .children(review.pods.iter().take(15).map(|p| {
                let disp_tag = match p.disposition {
                    DrainDisposition::Evict => Tag::warning().small().child("Evict"),
                    DrainDisposition::Skip => Tag::secondary().small().child("Skip"),
                    DrainDisposition::Wait => Tag::secondary().small().child("Wait"),
                    DrainDisposition::Block => Tag::danger().small().child("Block"),
                };
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(div().text_xs().child(format!("{}/{}", p.namespace, p.name)))
                    .child(
                        h_flex().gap_1().items_center().child(disp_tag).child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("{:?}", p.reason)),
                        ),
                    )
            }))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .when_some(review.plan_id.clone(), |row, pid| {
                        row.child(
                            Button::new("confirm-node-drain-btn")
                                .disabled(!review.can_drain)
                                .xsmall()
                                .danger()
                                .label("Confirm & Execute Eviction Plan")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    let pid = pid.clone();
                                    this.confirm_destructive_action(
                                        "Drain node?",
                                        "Execute the reviewed eviction plan? Pods will be evicted and the node will be cordoned.",
                                        "Drain", window, cx,
                                        move |this, window, cx| this.execute_node_drain(pid.clone(), window, cx),
                                    );
                                })),
                        )
                    })
                    .child(
                        Button::new("dismiss-node-drain-review")
                            .xsmall()
                            .ghost()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.inspector.drain_review = None;
                                cx.notify();
                            })),
                    ),
            )
    }

    fn render_drain_result_card(
        &self,
        result: &DrainResult,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let state_tag = match result.state {
            DrainState::Complete => Tag::success().small().child("Complete"),
            DrainState::Partial => Tag::warning().small().child("Partial"),
            DrainState::Blocked => Tag::danger().small().child("Blocked"),
            DrainState::Cancelled => Tag::secondary().small().child("Cancelled"),
            DrainState::Timeout => Tag::warning().small().child("Timeout"),
            DrainState::Uncertain => Tag::warning().small().child("Uncertain"),
        };

        v_flex()
            .w_full()
            .p_3()
            .gap_2()
            .rounded(px(8.0))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().secondary)
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("Node Drain Result · Cordoned: {}", result.cordoned)),
                    )
                    .child(state_tag),
            )
            .children(result.pods.iter().take(15).map(|p| {
                let tag = match p.state {
                    DrainOutcomeState::Evicted | DrainOutcomeState::Gone => {
                        Tag::success().small().child(format!("{:?}", p.state))
                    }
                    DrainOutcomeState::Skipped => Tag::secondary().small().child("Skipped"),
                    DrainOutcomeState::Blocked => Tag::danger().small().child("Blocked"),
                    _ => Tag::warning().small().child(format!("{:?}", p.state)),
                };
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(div().text_xs().child(format!("{}/{}", p.namespace, p.name)))
                    .child(tag)
            }))
    }

    fn render_pod_summary_cards(
        &self,
        pod: &PodDetails,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let node_ref = pod.node.as_ref().map(|node_name| ResourceReference {
            kind: ResourceKind::Nodes,
            name: node_name.clone(),
            namespace: None,
            uid: None,
        });
        let sa_ref = pod
            .service_account
            .as_ref()
            .map(|sa_name| ResourceReference {
                kind: ResourceKind::ServiceAccounts,
                name: sa_name.clone(),
                namespace: Some(pod.namespace.clone()),
                uid: None,
            });

        v_flex()
            .w_full()
            .gap_3()
            // Pod Runtime Summary Card
            .child(
                v_flex()
                    .w_full()
                    .p_3()
                    .gap_1p5()
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().secondary)
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(cx.theme().muted_foreground)
                                    .child("POD RUNTIME OVERVIEW"),
                            )
                            .child(
                                h_flex()
                                    .gap_1()
                                    .items_center()
                                    .child(status_tag(&pod.phase))
                                    .when_some(pod.qos.clone(), |r, qos| {
                                        r.child(Tag::secondary().small().child(qos))
                                    })
                                    .when_some(pod.deletion_timestamp.clone(), |r, dt| {
                                        r.child(Tag::danger().small().child(format!(
                                            "Terminating ({})",
                                            crate::preference_runtime::timestamp(
                                                &dt,
                                                &self.preferences_draft.app.timezone
                                            )
                                        )))
                                    }),
                            ),
                    )
                    .when(!pod.pod_ips.is_empty(), |el| {
                        el.child(
                            h_flex()
                                .justify_between()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("Pod IPs"),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(pod.pod_ips.join(", ")),
                                ),
                        )
                    })
                    .when(!pod.host_ips.is_empty(), |el| {
                        el.child(
                            h_flex()
                                .justify_between()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("Host IPs"),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(pod.host_ips.join(", ")),
                                ),
                        )
                    })
                    .when_some(pod.node.clone(), |el, node_name| {
                        el.child(
                            h_flex()
                                .justify_between()
                                .items_center()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!("Node: {node_name}")),
                                )
                                .when_some(node_ref, |r, target| {
                                    r.child(
                                        Button::new("insp-pod-node-link")
                                            .xsmall()
                                            .ghost()
                                            .label("Inspect Node")
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.open_resource_inspector(
                                                    target.clone(),
                                                    window,
                                                    cx,
                                                );
                                            })),
                                    )
                                }),
                        )
                    })
                    .when_some(pod.service_account.clone(), |el, sa_name| {
                        el.child(
                            h_flex()
                                .justify_between()
                                .items_center()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!("ServiceAccount: {sa_name}")),
                                )
                                .when_some(sa_ref, |r, target| {
                                    r.child(
                                        Button::new("insp-pod-sa-link")
                                            .xsmall()
                                            .ghost()
                                            .label("Inspect SA")
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.open_resource_inspector(
                                                    target.clone(),
                                                    window,
                                                    cx,
                                                );
                                            })),
                                    )
                                }),
                        )
                    }),
            )
            // Pod Conditions Card
            .when(!pod.conditions.is_empty(), |el| {
                el.child(
                    v_flex()
                        .w_full()
                        .p_3()
                        .gap_2()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().secondary)
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(cx.theme().muted_foreground)
                                .child("POD CONDITIONS"),
                        )
                        .child(
                            h_flex()
                                .gap_1p5()
                                .flex_wrap()
                                .children(pod.conditions.iter().map(|cond| {
                                    if cond.status == "True" {
                                        Tag::success().small().child(format!("{}: True", cond.kind))
                                    } else {
                                        Tag::warning()
                                            .small()
                                            .child(format!("{}: {}", cond.kind, cond.status))
                                    }
                                })),
                        ),
                )
            })
            // Pod Containers Card (App, Init, Ephemeral)
            .child(
                v_flex()
                    .w_full()
                    .p_3()
                    .gap_2()
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().secondary)
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("CONTAINERS ({})", pod.containers.len())),
                    )
                    .children(pod.containers.iter().map(|c| {
                        let listener_cx = &*cx;
                        let ns = pod.namespace.clone();
                        let pname = pod.name.clone();
                        let puid = pod.uid.clone();
                        let cname = c.name.clone();
                        let ns_shell = ns.clone();
                        let pname_shell = pname.clone();
                        let puid_shell = puid.clone();
                        let cname_shell = cname.clone();
                        let ns_attach = ns.clone();
                        let pname_attach = pname.clone();
                        let puid_attach = puid.clone();
                        let cname_attach = cname.clone();
                        let image_str = c.image.clone();
                        let image_copy = c.image.clone();

                        let state_badge = match c.state.as_str() {
                            "running" => Tag::success().small().child("running"),
                            "waiting" => Tag::warning().small().child("waiting"),
                            "terminated" => Tag::secondary().small().child("terminated"),
                            other => Tag::secondary().small().child(other.to_owned()),
                        };

                        v_flex()
                            .w_full()
                            .p_2p5()
                            .gap_1p5()
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().background)
                            .child(
                                h_flex()
                                    .justify_between()
                                    .items_center()
                                    .child(
                                        h_flex()
                                            .gap_1p5()
                                            .items_center()
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .child(c.name.clone()),
                                            )
                                            .child(Tag::secondary().small().child(c.kind))
                                            .child(state_badge)
                                            .child(if c.ready {
                                                Tag::success().small().child("Ready")
                                            } else {
                                                Tag::warning().small().child("Not Ready")
                                            })
                                            .when(c.restarts > 0, |r| {
                                                r.child(
                                                    Tag::warning()
                                                        .small()
                                                        .child(format!("Restarts: {}", c.restarts)),
                                                )
                                            }),
                                    )
                                    .child(
                                        h_flex()
                                            .gap_1()
                                            .child(
                                                Button::new(SharedString::from(format!(
                                                    "pod-logs-{}",
                                                    c.name
                                                )))
                                                .xsmall()
                                                .primary()
                                                .label("Logs")
                                                .on_click(listener_cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.open_pod_stream(
                                                            ns.clone(),
                                                            pname.clone(),
                                                            puid.clone(),
                                                            cname.clone(),
                                                            PodStreamMode::Logs,
                                                            window,
                                                            cx,
                                                        );
                                                    },
                                                )),
                                            )
                                            .child(
                                                Button::new(SharedString::from(format!(
                                                    "pod-shell-{}",
                                                    c.name
                                                )))
                                                .xsmall()
                                                .outline()
                                                .label("Shell")
                                                .on_click(listener_cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.open_pod_stream(
                                                            ns_shell.clone(),
                                                            pname_shell.clone(),
                                                            puid_shell.clone(),
                                                            cname_shell.clone(),
                                                            PodStreamMode::Shell,
                                                            window,
                                                            cx,
                                                        );
                                                    },
                                                )),
                                            )
                                            .child(
                                                Button::new(SharedString::from(format!(
                                                    "pod-attach-{}",
                                                    c.name
                                                )))
                                                .xsmall()
                                                .ghost()
                                                .label("Attach")
                                                .on_click(listener_cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.open_pod_stream(
                                                            ns_attach.clone(),
                                                            pname_attach.clone(),
                                                            puid_attach.clone(),
                                                            cname_attach.clone(),
                                                            PodStreamMode::Attach,
                                                            window,
                                                            cx,
                                                        );
                                                    },
                                                )),
                                            ),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .justify_between()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .truncate()
                                            .child(format!("Image: {image_str}")),
                                    )
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "copy-img-{}",
                                            c.name
                                        )))
                                        .xsmall()
                                        .ghost()
                                        .label("Copy")
                                        .on_click(
                                            listener_cx.listener(move |_, _, _, cx| {
                                                cx.write_to_clipboard(ClipboardItem::new_string(
                                                    image_copy.clone(),
                                                ));
                                            }),
                                        ),
                                    ),
                            )
                            .when(!c.ports.is_empty(), |el| {
                                let ns_fwd = pod.namespace.clone();
                                let pname_fwd = pod.name.clone();
                                let puid_fwd = pod.uid.clone();
                                el.child(h_flex().gap_1p5().flex_wrap().items_center().children(
                                    c.ports.iter().map(move |pt| {
                                        let port_num = pt.port;
                                        let ns_p = ns_fwd.clone();
                                        let pn_p = pname_fwd.clone();
                                        let pu_p = puid_fwd.clone();
                                        let port_label = match &pt.name {
                                            Some(n) => {
                                                format!("Forward {n} ({port_num}/{})", pt.protocol)
                                            }
                                            None => {
                                                format!("Forward :{port_num}/{}", pt.protocol)
                                            }
                                        };
                                        Button::new(SharedString::from(format!(
                                            "pod-fwd-{}-{port_num}",
                                            c.name
                                        )))
                                        .xsmall()
                                        .outline()
                                        .label(port_label)
                                        .on_click(
                                            listener_cx.listener(move |this, _, window, cx| {
                                                this.start_pod_forward(
                                                    ns_p.clone(),
                                                    pn_p.clone(),
                                                    pu_p.clone(),
                                                    port_num,
                                                    window,
                                                    cx,
                                                );
                                            }),
                                        )
                                    }),
                                ))
                            })
                    })),
            )
    }

    fn render_inspector_metadata(
        &self,
        detail: &ResourceDetail,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let Some(meta) = &detail.metadata else {
            return div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Metadata details are withheld or unavailable for this resource kind.")
                .into_any_element();
        };

        v_flex()
            .w_full()
            .gap_3()
            // Identity & Finalizers card
            .child(
                v_flex()
                    .w_full()
                    .p_3()
                    .gap_1p5()
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().secondary)
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("OBJECT METADATA"),
                            )
                            .when(meta.partial, |r| {
                                r.child(Tag::warning().small().child("Partial / Redacted"))
                            }),
                    )
                    .when_some(meta.deletion_timestamp.clone(), |el, dt| {
                        el.child(div().text_xs().text_color(cx.theme().danger).child(format!(
                            "Deletion Timestamp: {}",
                            crate::preference_runtime::timestamp(
                                &dt,
                                &self.preferences_draft.app.timezone
                            )
                        )))
                    })
                    .when(!meta.finalizers.is_empty(), |el| {
                        el.child(
                            v_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("Finalizers:"),
                                )
                                .child(
                                    h_flex().gap_1().flex_wrap().children(
                                        meta.finalizers
                                            .iter()
                                            .map(|fin| Tag::secondary().small().child(fin.clone())),
                                    ),
                                ),
                        )
                    }),
            )
            // Labels card
            .child(
                v_flex()
                    .w_full()
                    .p_3()
                    .gap_1p5()
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().secondary)
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("LABELS ({})", meta.labels.len())),
                    )
                    .children(meta.labels.iter().map(|entry| {
                        let kv = format!("{}={}", entry.key, entry.value);
                        let kv_copy = kv.clone();
                        h_flex()
                            .justify_between()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(entry.key.clone()),
                            )
                            .child(
                                h_flex()
                                    .gap_1()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(entry.value.clone()),
                                    )
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "copy-lbl-{}",
                                            entry.key
                                        )))
                                        .xsmall()
                                        .ghost()
                                        .label("Copy")
                                        .on_click(
                                            cx.listener(move |_, _, _, cx| {
                                                cx.write_to_clipboard(ClipboardItem::new_string(
                                                    kv_copy.clone(),
                                                ));
                                            }),
                                        ),
                                    ),
                            )
                    })),
            )
            // Reviewed Annotations card
            .child(
                v_flex()
                    .w_full()
                    .p_3()
                    .gap_1p5()
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().secondary)
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("REVIEWED ANNOTATIONS ({})", meta.annotations.len())),
                    )
                    .children(meta.annotations.iter().map(|entry| {
                        v_flex()
                            .w_full()
                            .py_0p5()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(entry.key.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(entry.value.clone()),
                            )
                    })),
            )
            // Managed Fields card
            .when(!meta.managed_fields.is_empty(), |el| {
                el.child(
                    v_flex()
                        .w_full()
                        .p_3()
                        .gap_1p5()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().secondary)
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(format!("MANAGED FIELDS ({})", meta.managed_fields.len())),
                        )
                        .children(meta.managed_fields.iter().map(|mf| {
                            v_flex()
                                .w_full()
                                .p_2()
                                .gap_0p5()
                                .rounded(px(6.0))
                                .border_1()
                                .border_color(cx.theme().border)
                                .child(
                                    h_flex()
                                        .justify_between()
                                        .items_center()
                                        .child(
                                            div()
                                                .text_xs()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child(mf.manager.clone()),
                                        )
                                        .child(
                                            Tag::secondary().small().child(mf.operation.clone()),
                                        ),
                                )
                                .when_some(mf.time.clone(), |r, t| {
                                    r.child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(format!(
                                                "Updated: {}",
                                                crate::preference_runtime::timestamp(
                                                    &t,
                                                    &self.preferences_draft.app.timezone
                                                )
                                            )),
                                    )
                                })
                        })),
                )
            })
            .into_any_element()
    }

    fn render_inspector_relations(
        &self,
        detail: &ResourceDetail,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .w_full()
            .gap_3()
            // Direct related links (Controller, Node, ServiceAccount, StorageClass, etc.)
            .when(!detail.related.is_empty(), |el| {
                el.child(
                    v_flex()
                        .w_full()
                        .p_3()
                        .gap_1p5()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().secondary)
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(format!("DIRECT REFERENCES ({})", detail.related.len())),
                        )
                        .children(detail.related.iter().enumerate().map(
                            |(idx, (relation, target_ref))| {
                                let target = target_ref.clone();
                                h_flex()
                                    .justify_between()
                                    .items_center()
                                    .child(
                                        h_flex()
                                            .gap_1p5()
                                            .items_center()
                                            .child(Tag::secondary().small().child(relation.clone()))
                                            .child(div().text_xs().child(format!(
                                                "{:?}/{}",
                                                target_ref.kind, target_ref.name
                                            ))),
                                    )
                                    .child(
                                        Button::new(SharedString::from(format!("rel-nav-{idx}")))
                                            .xsmall()
                                            .ghost()
                                            .label("Inspect")
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.open_resource_inspector(
                                                    target.clone(),
                                                    window,
                                                    cx,
                                                );
                                            })),
                                    )
                            },
                        )),
                )
            })
            // Scoped ResourceCollections (Pods, ReplicaSets, Jobs, EndpointSlices, ResourceQuotas, LimitRanges)
            .children(
                self.inspector
                    .collections
                    .iter()
                    .filter(|col| col.collection != CollectionKind::Events)
                    .map(|col| {
                        let is_pods = col.collection == CollectionKind::Pods;
                        let state_badge = match col.state {
                            CollectionState::Complete => Tag::success().small().child("Complete"),
                            CollectionState::Partial => Tag::warning().small().child("Partial"),
                            CollectionState::Denied => Tag::danger().small().child("Denied"),
                            CollectionState::Unavailable => {
                                Tag::secondary().small().child("Unavailable")
                            }
                        };
                        v_flex()
                            .w_full()
                            .p_3()
                            .gap_2()
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().secondary)
                            .child(
                                h_flex()
                                    .justify_between()
                                    .items_center()
                                    .child(div().text_xs().font_weight(FontWeight::SEMIBOLD).child(
                                        format!("{:?} ({})", col.collection, col.items.len()),
                                    ))
                                    .child(state_badge),
                            )
                            .children(col.items.iter().enumerate().map(|(idx, item)| {
                                let target = item.reference.clone();
                                let pod_ns = item.reference.namespace.clone().unwrap_or_default();
                                let pod_name = item.reference.name.clone();
                                let pod_ns_shell = pod_ns.clone();
                                let pod_name_shell = pod_name.clone();
                                let summary_cols: String = item
                                    .columns
                                    .iter()
                                    .take(3)
                                    .map(|(k, v)| format!("{k}:{v}"))
                                    .collect::<Vec<_>>()
                                    .join(" · ");

                                v_flex()
                                    .w_full()
                                    .p_2()
                                    .gap_1()
                                    .rounded(px(6.0))
                                    .border_1()
                                    .border_color(cx.theme().border)
                                    .bg(cx.theme().background)
                                    .child(
                                        h_flex()
                                            .justify_between()
                                            .items_center()
                                            .child(
                                                h_flex()
                                                    .gap_1p5()
                                                    .items_center()
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .font_weight(FontWeight::MEDIUM)
                                                            .child(item.reference.name.clone()),
                                                    )
                                                    .child(status_tag(&item.status)),
                                            )
                                            .child(
                                                h_flex()
                                                    .gap_1()
                                                    .items_center()
                                                    .when(is_pods, |r| {
                                                        r.child(
                                                            Button::new(SharedString::from(
                                                                format!("col-pod-logs-{idx}"),
                                                            ))
                                                            .xsmall()
                                                            .primary()
                                                            .label("Logs")
                                                            .on_click(cx.listener(
                                                                move |this, _, window, cx| {
                                                                    this.open_pod_stream_for_pod(
                                                                        pod_ns.clone(),
                                                                        pod_name.clone(),
                                                                        PodStreamMode::Logs,
                                                                        window,
                                                                        cx,
                                                                    );
                                                                },
                                                            )),
                                                        )
                                                        .child(
                                                            Button::new(SharedString::from(
                                                                format!("col-pod-shell-{idx}"),
                                                            ))
                                                            .xsmall()
                                                            .outline()
                                                            .label("Shell")
                                                            .on_click(cx.listener(
                                                                move |this, _, window, cx| {
                                                                    this.open_pod_stream_for_pod(
                                                                        pod_ns_shell.clone(),
                                                                        pod_name_shell.clone(),
                                                                        PodStreamMode::Shell,
                                                                        window,
                                                                        cx,
                                                                    );
                                                                },
                                                            )),
                                                        )
                                                    })
                                                    .child(
                                                        Button::new(SharedString::from(format!(
                                                            "col-item-{:?}-{idx}",
                                                            col.collection
                                                        )))
                                                        .xsmall()
                                                        .ghost()
                                                        .label("Inspect")
                                                        .on_click(cx.listener(
                                                            move |this, _, window, cx| {
                                                                this.open_resource_inspector(
                                                                    target.clone(),
                                                                    window,
                                                                    cx,
                                                                );
                                                            },
                                                        )),
                                                    ),
                                            ),
                                    )
                                    .when(!summary_cols.is_empty(), |el| {
                                        el.child(
                                            div()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(summary_cols),
                                        )
                                    })
                            }))
                    }),
            )
    }

    fn render_inspector_events(
        &self,
        detail: &ResourceDetail,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let pod_events = self.inspector.pod_detail.clone();
        let event_collections: Vec<ResourceCollection> = self
            .inspector
            .collections
            .iter()
            .filter(|c| c.collection == CollectionKind::Events)
            .cloned()
            .collect();

        v_flex()
            .w_full()
            .gap_3()
            .when_some(detail.event_diagnostic.clone(), |el, diag| {
                el.child(
                    v_flex()
                        .w_full()
                        .p_3()
                        .gap_1p5()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().secondary)
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("EVENT MESSAGE"),
                        )
                        .child(div().text_xs().child(diag.message)),
                )
            })
            .when_some(pod_events, |el, pod| {
                let state_tag = match pod.events_state {
                    PodEventsState::Complete => Tag::success().small().child("Complete"),
                    PodEventsState::Partial => Tag::warning().small().child("Partial"),
                    PodEventsState::Denied => Tag::danger().small().child("Denied"),
                    PodEventsState::Unavailable => Tag::secondary().small().child("Unavailable"),
                };
                el.child(
                    v_flex()
                        .w_full()
                        .p_3()
                        .gap_2()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().secondary)
                        .child(
                            h_flex()
                                .justify_between()
                                .items_center()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(format!("POD EVENTS ({})", pod.events.len())),
                                )
                                .child(state_tag),
                        )
                        .when(pod.events.is_empty(), |r| {
                            r.child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("No pod events recorded."),
                            )
                        })
                        .children(pod.events.iter().map(|ev| {
                            h_flex()
                                .justify_between()
                                .items_center()
                                .child(
                                    h_flex()
                                        .gap_1p5()
                                        .items_center()
                                        .child(if ev.kind == "Warning" {
                                            Tag::warning().small().child(ev.kind.clone())
                                        } else {
                                            Tag::secondary().small().child(ev.kind.clone())
                                        })
                                        .child(
                                            div()
                                                .text_xs()
                                                .font_weight(FontWeight::MEDIUM)
                                                .child(ev.reason.clone()),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!(
                                            "×{} · {}",
                                            ev.count,
                                            ev.last_seen.as_deref().unwrap_or("-")
                                        )),
                                )
                        })),
                )
            })
            .children(event_collections.into_iter().map(|col| {
                v_flex()
                    .w_full()
                    .p_3()
                    .gap_2()
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().secondary)
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("INVOLVED EVENTS ({})", col.items.len())),
                    )
                    .when(col.items.is_empty(), |r| {
                        r.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("No recent events for this object."),
                        )
                    })
                    .children(col.items.into_iter().enumerate().map(|(idx, item)| {
                        let target = item.reference.clone();
                        let reason = item.columns.get("reason").cloned().unwrap_or_default();
                        let count = item
                            .columns
                            .get("count")
                            .cloned()
                            .unwrap_or_else(|| "1".into());
                        v_flex()
                            .w_full()
                            .p_2()
                            .gap_1()
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(cx.theme().border)
                            .child(
                                h_flex()
                                    .justify_between()
                                    .items_center()
                                    .child(
                                        h_flex()
                                            .gap_1p5()
                                            .items_center()
                                            .child(status_tag(&item.status))
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .child(reason),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(format!("×{count}")),
                                            ),
                                    )
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "insp-ev-open-{idx}"
                                        )))
                                        .xsmall()
                                        .ghost()
                                        .label("Inspect Event")
                                        .on_click(
                                            cx.listener(move |this, _, window, cx| {
                                                this.open_resource_inspector(
                                                    target.clone(),
                                                    window,
                                                    cx,
                                                );
                                            }),
                                        ),
                                    ),
                            )
                    }))
            }))
    }

    fn render_inspector_yaml(
        &self,
        _detail: &ResourceDetail,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let yaml_copy = self.inspector_yaml_editor.read(cx).value().to_string();

        v_flex()
            .w_full()
            .flex_1()
            .min_h(px(420.0))
            .min_w_0()
            .gap_2()
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        h_flex()
                            .gap_1p5()
                            .items_center()
                            .child(
                                Button::new("yaml-dry-run-btn")
                                    .xsmall()
                                    .outline()
                                    .label("Server Dry-Run")
                                    .disabled(self.inspector.edit_load.is_none())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.apply_inspector_yaml(true, window, cx);
                                    })),
                            )
                            .child(
                                Button::new("yaml-apply-btn")
                                    .xsmall()
                                    .primary()
                                    .label("Apply Changes")
                                    .disabled(self.inspector.edit_load.is_none())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.apply_inspector_yaml(false, window, cx);
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(
                                Button::new("yaml-copy-btn")
                                    .xsmall()
                                    .ghost()
                                    .label("Copy YAML")
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            yaml_copy.clone(),
                                        ));
                                    })),
                            )
                            .child(
                                Button::new("yaml-reload-btn")
                                    .xsmall()
                                    .ghost()
                                    .label("Reload")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.load_inspector_yaml(window, cx);
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .w_full()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(px(6.0))
                    .overflow_hidden()
                    .child(EditorSurface::new(
                        &self.inspector_yaml_editor,
                        &self.preferences_draft.editor,
                    )),
            )
    }

    fn render_helm_inspector(&self, helm: &HelmDetail, cx: &mut Context<Self>) -> impl IntoElement {
        let is_chart = helm.view == HelmView::Charts;
        let chart_name = helm.name.clone();
        let chart_ver = helm.version.clone().unwrap_or_else(|| "latest".into());

        v_flex()
            .w_full()
            .gap_3()
            .when(is_chart, |el| {
                el.child(
                    h_flex()
                        .w_full()
                        .p_3()
                        .justify_between()
                        .items_center()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().secondary)
                        .child(
                            v_flex()
                                .gap_0p5()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(chart_name.clone()),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!("Version: {chart_ver}")),
                                ),
                        )
                        .child(
                            Button::new("insp-helm-install-chart-btn")
                                .xsmall()
                                .primary()
                                .label("Configure Install")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.helm_install_chart = Some(chart_name.clone());
                                    this.helm_install_version = Some(chart_ver.clone());
                                    this.helm_install_message = None;
                                    cx.notify();
                                })),
                        ),
                )
            })
            .when(!helm.facts.is_empty(), |el| {
                el.child(
                    v_flex()
                        .w_full()
                        .p_3()
                        .gap_1p5()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().secondary)
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("CHART METADATA"),
                        )
                        .children(helm.facts.iter().map(|f| {
                            h_flex()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(f.label.clone()),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(f.value.clone()),
                                )
                        })),
                )
            })
            .when(!helm.history.is_empty(), |el| {
                el.child(
                    v_flex()
                        .w_full()
                        .p_3()
                        .gap_2()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().secondary)
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(format!(
                                    "RELEASE REVISION HISTORY ({})",
                                    helm.history.len()
                                )),
                        )
                        .children(helm.history.iter().map(|rev| {
                            v_flex()
                                .w_full()
                                .p_2()
                                .gap_0p5()
                                .rounded(px(6.0))
                                .border_1()
                                .border_color(cx.theme().border)
                                .child(
                                    h_flex()
                                        .justify_between()
                                        .items_center()
                                        .child(
                                            div()
                                                .text_xs()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child(format!(
                                                    "Revision #{} · {}",
                                                    rev.revision, rev.chart
                                                )),
                                        )
                                        .child(status_tag(&rev.status)),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!(
                                            "App {} · Updated {}",
                                            rev.app_version, rev.updated
                                        )),
                                )
                        })),
                )
            })
    }
}

fn status_tag(status: &str) -> Tag {
    let lower = status.to_lowercase();
    if lower.contains("running")
        || lower.contains("ready")
        || lower.contains("active")
        || lower.contains("bound")
        || lower.contains("complete")
        || lower.contains("succeeded")
        || lower.contains("deployed")
        || lower == "true"
    {
        Tag::success().small().child(status.to_owned())
    } else if lower.contains("pending")
        || lower.contains("terminating")
        || lower.contains("warning")
        || lower.contains("waiting")
    {
        Tag::warning().small().child(status.to_owned())
    } else if lower.contains("fail")
        || lower.contains("error")
        || lower.contains("crash")
        || lower == "false"
    {
        Tag::danger().small().child(status.to_owned())
    } else {
        Tag::secondary().small().child(status.to_owned())
    }
}

#[cfg(test)]
mod destructive_confirmation_tests {
    use super::{DeleteTarget, ResourceKind, WorkloadAction, workload_requires_confirmation};

    #[test]
    fn disruptive_workload_actions_require_explicit_confirmation() {
        assert!(workload_requires_confirmation(WorkloadAction::Restart));
        assert!(workload_requires_confirmation(WorkloadAction::Suspend));
        for action in [
            WorkloadAction::Scale,
            WorkloadAction::Resume,
            WorkloadAction::Trigger,
        ] {
            assert!(!workload_requires_confirmation(action));
        }
    }

    #[test]
    fn delete_confirmation_snapshot_preserves_identity_preconditions() {
        let original = DeleteTarget {
            kind: ResourceKind::Pods,
            name: "example".into(),
            namespace: Some("default".into()),
            uid: "original-uid".into(),
            resource_version: "123".into(),
        };
        let captured = original.clone();
        assert_eq!(captured, original);
        assert_eq!(captured.uid, "original-uid");
        assert_eq!(captured.resource_version, "123");
    }
}
