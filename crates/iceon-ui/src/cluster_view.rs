//! Cluster Workspace View: session lifecycle, namespace selector, Cluster Overview,
//! Workloads Overview, generic Resource Table (all 41 kinds + live Pod sync + YAML create + delete),
//! Helm Charts/Releases & Install modal, Port Forwarding table, Access Control > Users (Managed RBAC),
//! and Custom Resources.

use std::time::Duration;

use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, StyledExt as _,
    badge::Badge,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Editor, Input},
    resizable::{h_resizable, resizable_panel, v_resizable},
    scroll::ScrollableElement as _,
    switch::Switch,
    v_flex, IconName,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::IceonApp;
use crate::navigation::{
    ResourceView, cell_value, connection_error_message, destination_label, format_age,
    normalize_destination, resource_view, yaml_scaffold,
};
use iceon_core::connection::{
    ConnectionSnapshot, OpenRequest, ResourceItem, ResourceKind, ResourcePage, ResourceRequest,
    SessionRequest,
    create::CreateRequest,
    custom_resources::{
        CustomDefinitionRequest, CustomResourcePage, CustomResourceRequest, CustomResourceTarget,
    },
    delete::DeleteRequest,
    details::ResourceReference,
    helm::{HelmPage, HelmRequest, HelmView, install::HelmInstallRequest},
    namespace_cache::NamespaceRefreshRequest,
    pod_sync::PodSyncRequest,
    rbac::{RbacUsersRequest, RbacUsersResponse},
    rbac_management::{
        ApplyRequest, ClusterAccess, ManagementRequest, ManagementSnapshot, ManagedUser,
        PlanOperation, PlanRequest, UserPlan,
    },
};

#[derive(Default, Clone)]
pub struct OverviewSnapshot {
    pub nodes: Vec<ResourceItem>,
    pub pods: Vec<ResourceItem>,
    pub deployments: Vec<ResourceItem>,
    pub daemon_sets: Vec<ResourceItem>,
    pub stateful_sets: Vec<ResourceItem>,
    pub replica_sets: Vec<ResourceItem>,
    pub replication_controllers: Vec<ResourceItem>,
    pub jobs: Vec<ResourceItem>,
    pub cron_jobs: Vec<ResourceItem>,
    pub events: Vec<ResourceItem>,
}

impl IceonApp {
    pub(crate) fn open_cluster_workspace(
        &mut self,
        cluster_id: String,
        section: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let normalized = normalize_destination(&section);
        self.active_cluster_section = normalized;
        self.cluster_error = None;

        let already_open = self
            .cluster_session
            .as_ref()
            .is_some_and(|s| s.cluster_id == cluster_id);
        if already_open {
            self.refresh_active_cluster_view(window, cx);
            return;
        }

        self.close_active_pod_sync();
        self.resource_page = None;
        self.helm_page = None;
        self.custom_definitions_page = None;
        self.custom_instances_page = None;
        self.rbac_users_response = None;
        self.rbac_management_snapshot = None;
        self.rbac_active_plan = None;
        self.namespaces = vec![];
        self.selected_namespace = None;
        self.inspector.target = None;

        let engine = self.connection.clone();
        cx.spawn_in(window, async move |this, cx| {
            let opened = engine
                .open(OpenRequest {
                    cluster_id: cluster_id.clone(),
                })
                .await;
            let Ok(snapshot) = opened else {
                let _ = this.update(cx, |this, cx| {
                    if let Err(err) = opened {
                        this.cluster_error = Some(connection_error_message(err).into());
                    }
                    cx.notify();
                });
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.cluster_session = Some(snapshot.clone());
                this.refresh_catalog(cx);
            });

            let connected = engine
                .connect(SessionRequest {
                    cluster_id: snapshot.cluster_id.clone(),
                    session_id: snapshot.session_id.clone(),
                })
                .await;

            let _ = this.update(cx, |this, cx| {
                match connected {
                    Ok(snap) => {
                        let is_connected = snap.status == "connected";
                        this.cluster_session = Some(snap);
                        this.refresh_catalog(cx);
                        if is_connected {
                            this.refresh_namespaces(window, cx);
                            this.refresh_active_cluster_view(window, cx);
                        }
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

    pub(crate) fn reconnect_active_cluster(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        self.cluster_session = None;
        let section = self.active_cluster_section.clone();
        self.open_cluster_workspace(session.cluster_id, section, window, cx);
    }

    pub(crate) fn disconnect_active_cluster(&mut self, cx: &mut Context<Self>) {
        self.close_active_pod_sync();
        if let Some(session) = self.cluster_session.take() {
            let _ = self.connection.disconnect(SessionRequest {
                cluster_id: session.cluster_id,
                session_id: session.session_id,
            });
        }
        self.refresh_catalog(cx);
    }

    pub(crate) fn refresh_namespaces(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let read_id = self.next_operation_id();
        let engine = self.connection.clone();
        cx.spawn_in(window, async move |this, cx| {
            let cached = engine
                .namespace_cache_get(SessionRequest {
                    cluster_id: session.cluster_id.clone(),
                    session_id: session.session_id.clone(),
                })
                .await;
            if let Ok(Some(snap)) = cached {
                let _ = this.update(cx, |this, cx| {
                    this.namespaces = snap.names;
                    cx.notify();
                });
            }
            if let Ok(fresh) = engine
                .namespace_cache_refresh(NamespaceRefreshRequest {
                    cluster_id: session.cluster_id,
                    session_id: session.session_id,
                    read_id,
                })
                .await
            {
                let _ = this.update(cx, |this, cx| {
                    this.namespaces = fresh.names;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(crate) fn refresh_active_cluster_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        if session.status != "connected" {
            return;
        }
        let section = self.active_cluster_section.clone();
        self.cluster_loading = true;
        self.cluster_error = None;

        if section == "overview" || section == "workloads-overview" {
            self.load_cluster_overview(window, cx);
            return;
        }
        if section == "helm-charts" || section == "helm-releases" {
            let view = if section == "helm-charts" {
                HelmView::Charts
            } else {
                HelmView::Releases
            };
            self.load_helm_view(view, window, cx);
            return;
        }
        if section == "port-forwarding" {
            self.cluster_loading = false;
            self.refresh_port_forwards(cx);
            return;
        }
        if section == "users" {
            self.load_rbac_users_view(window, cx);
            return;
        }
        if section == "custom-resource-definitions" {
            self.load_custom_definitions(window, cx);
            return;
        }
        if let Some(crd_name) = section.strip_prefix("crd:") {
            self.load_custom_instances(crd_name.to_string(), window, cx);
            return;
        }
        if let Some(view) = resource_view(&section) {
            self.load_resource_table(view, window, cx);
        } else {
            self.cluster_loading = false;
        }
    }

    fn load_cluster_overview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let ns = self.selected_namespace.clone();
        let base_id = self.next_operation_id();
        let engine = self.connection.clone();
        cx.spawn_in(window, async move |this, cx| {
            let req = |kind: ResourceKind, offset: u64, namespaced: bool| ResourceRequest {
                cluster_id: session.cluster_id.clone(),
                session_id: session.session_id.clone(),
                kind,
                read_id: base_id + offset,
                namespace: if namespaced { ns.clone() } else { None },
                continue_token: None,
                query: None,
                status: None,
                group: None,
            };
            let nodes = engine
                .resources(req(ResourceKind::Nodes, 1, false))
                .await
                .map(|p| p.items)
                .unwrap_or_default();
            let pods = engine
                .resources(req(ResourceKind::Pods, 2, true))
                .await
                .map(|p| p.items)
                .unwrap_or_default();
            let deployments = engine
                .resources(req(ResourceKind::Deployments, 3, true))
                .await
                .map(|p| p.items)
                .unwrap_or_default();
            let daemon_sets = engine
                .resources(req(ResourceKind::DaemonSets, 4, true))
                .await
                .map(|p| p.items)
                .unwrap_or_default();
            let stateful_sets = engine
                .resources(req(ResourceKind::StatefulSets, 5, true))
                .await
                .map(|p| p.items)
                .unwrap_or_default();
            let replica_sets = engine
                .resources(req(ResourceKind::ReplicaSets, 6, true))
                .await
                .map(|p| p.items)
                .unwrap_or_default();
            let replication_controllers = engine
                .resources(req(ResourceKind::ReplicationControllers, 7, true))
                .await
                .map(|p| p.items)
                .unwrap_or_default();
            let jobs = engine
                .resources(req(ResourceKind::Jobs, 8, true))
                .await
                .map(|p| p.items)
                .unwrap_or_default();
            let cron_jobs = engine
                .resources(req(ResourceKind::CronJobs, 9, true))
                .await
                .map(|p| p.items)
                .unwrap_or_default();
            let events = engine
                .resources(req(ResourceKind::Events, 10, true))
                .await
                .map(|p| p.items)
                .unwrap_or_default();

            let _ = this.update(cx, |this, cx| {
                this.cluster_loading = false;
                this.overview_snapshot = OverviewSnapshot {
                    nodes,
                    pods,
                    deployments,
                    daemon_sets,
                    stateful_sets,
                    replica_sets,
                    replication_controllers,
                    jobs,
                    cron_jobs,
                    events,
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn load_resource_table(
        &mut self,
        view: &'static ResourceView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let read_id = self.next_operation_id();
        let query = {
            let q = self.resource_filter_input.read(cx).value().trim().to_string();
            (!q.is_empty()).then_some(q)
        };
        let status = self.selected_status_filter.clone();
        let namespace = if view.namespaced {
            self.selected_namespace.clone()
        } else {
            None
        };
        let engine = self.connection.clone();
        cx.spawn_in(window, async move |this, cx| {
            let res = engine
                .resources(ResourceRequest {
                    cluster_id: session.cluster_id,
                    session_id: session.session_id,
                    kind: view.kind,
                    read_id,
                    namespace,
                    continue_token: None,
                    query,
                    status,
                    group: None,
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.cluster_loading = false;
                match res {
                    Ok(page) => {
                        this.resource_page = Some(page);
                        this.cluster_error = None;
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

    pub(crate) fn toggle_pod_live_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_pod_sync_id.is_some() {
            self.close_active_pod_sync();
            cx.notify();
            return;
        }
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let read_id = self.next_operation_id();
        let namespace = self.selected_namespace.clone();
        let engine = self.connection.clone();
        cx.spawn_in(window, async move |this, cx| {
            let res = engine
                .pod_sync_open(ResourceRequest {
                    cluster_id: session.cluster_id.clone(),
                    session_id: session.session_id.clone(),
                    kind: ResourceKind::Pods,
                    read_id,
                    namespace,
                    continue_token: None,
                    query: None,
                    status: None,
                    group: None,
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                match res {
                    Ok(snap) => {
                        let sync_id = snap.sync_id.clone();
                        this.active_pod_sync_id = Some(sync_id.clone());
                        if let Some(page) = snap.page {
                            this.resource_page = Some(page);
                        }
                        this.spawn_pod_sync_poller(
                            session.cluster_id,
                            session.session_id,
                            sync_id,
                            window,
                            cx,
                        );
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

    fn spawn_pod_sync_poller(
        &mut self,
        cluster_id: String,
        session_id: String,
        sync_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let engine = self.connection.clone();
        cx.spawn_in(window, async move |this, cx| {
            loop {
                tokio::time::sleep(Duration::from_millis(800)).await;
                let keep = this
                    .update(cx, |this, cx| {
                        if this.active_pod_sync_id.as_deref() != Some(sync_id.as_str()) {
                            return false;
                        }
                        match engine.pod_sync_read(PodSyncRequest {
                            cluster_id: cluster_id.clone(),
                            session_id: session_id.clone(),
                            sync_id: sync_id.clone(),
                        }) {
                            Ok(snap) => {
                                if let Some(page) = snap.page {
                                    this.resource_page = Some(page);
                                    cx.notify();
                                }
                                snap.status != "stopped" && snap.status != "error"
                            }
                            Err(_) => {
                                this.active_pod_sync_id = None;
                                false
                            }
                        }
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
    }

    pub(crate) fn close_active_pod_sync(&mut self) {
        if let (Some(session), Some(sync_id)) =
            (self.cluster_session.clone(), self.active_pod_sync_id.take())
        {
            let _ = self.connection.pod_sync_close(PodSyncRequest {
                cluster_id: session.cluster_id,
                session_id: session.session_id,
                sync_id,
            });
        }
    }

    pub(crate) fn open_create_resource_modal(
        &mut self,
        kind: ResourceKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let scaffold = yaml_scaffold(kind, self.selected_namespace.as_deref());
        self.create_yaml_editor
            .update(cx, |ed, cx| ed.set_value(scaffold, window, cx));
        self.creating_resource_kind = Some(kind);
        self.create_modal_message = None;
        cx.notify();
    }

    pub(crate) fn submit_create_resource(
        &mut self,
        dry_run: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let Some(kind) = self.creating_resource_kind else {
            return;
        };
        let yaml = self.create_yaml_editor.read(cx).value().to_string();
        let read_id = self.next_operation_id();
        let namespace = self.selected_namespace.clone();
        let engine = self.connection.clone();
        cx.spawn_in(window, async move |this, cx| {
            let res = engine
                .create(CreateRequest {
                    cluster_id: session.cluster_id,
                    session_id: session.session_id,
                    kind,
                    namespace,
                    yaml,
                    dry_run,
                    read_id,
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                match res {
                    Ok(out) => {
                        if dry_run {
                            this.create_modal_message = Some(format!(
                                "Dry-run validated: {} ({:?})",
                                out.name, out.kind
                            ));
                        } else {
                            this.creating_resource_kind = None;
                            this.create_modal_message = None;
                            this.refresh_active_cluster_view(window, cx);
                        }
                    }
                    Err(err) => {
                        this.create_modal_message =
                            Some(format!("Error: {}", connection_error_message(err)));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn delete_resource_row(
        &mut self,
        kind: ResourceKind,
        item: ResourceItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let (Some(uid), Some(rv)) = (item.uid, item.resource_version) else {
            return;
        };
        let read_id = self.next_operation_id();
        let engine = self.connection.clone();
        cx.spawn_in(window, async move |this, cx| {
            let res = engine
                .delete(DeleteRequest {
                    cluster_id: session.cluster_id,
                    session_id: session.session_id,
                    kind,
                    namespace: item.namespace,
                    name: item.name,
                    uid,
                    resource_version: rv,
                    read_id,
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                match res {
                    Ok(_) => {
                        this.refresh_active_cluster_view(window, cx);
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

    fn load_helm_view(&mut self, view: HelmView, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let read_id = self.next_operation_id();
        let namespace = if view == HelmView::Releases {
            self.selected_namespace.clone()
        } else {
            None
        };
        let engine = self.connection.clone();
        cx.spawn_in(window, async move |this, cx| {
            let res = engine
                .helm_table(HelmRequest {
                    cluster_id: session.cluster_id,
                    session_id: session.session_id,
                    view,
                    namespace,
                    read_id,
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.cluster_loading = false;
                match res {
                    Ok(page) => {
                        this.helm_page = Some(page);
                        this.cluster_error = None;
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

    pub(crate) fn submit_helm_install(
        &mut self,
        dry_run: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let Some(chart) = self.helm_install_chart.clone() else {
            return;
        };
        let release_name = self
            .helm_release_name_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let namespace = self
            .helm_namespace_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let values_yaml = self.helm_values_editor.read(cx).value().to_string();
        let version = self.helm_install_version.clone();
        let read_id = self.next_operation_id();
        let engine = self.connection.clone();
        cx.spawn_in(window, async move |this, cx| {
            let res = engine
                .helm_install(HelmInstallRequest {
                    cluster_id: session.cluster_id,
                    session_id: session.session_id,
                    read_id,
                    namespace,
                    release_name,
                    chart,
                    version,
                    values_yaml,
                    dry_run,
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                match res {
                    Ok(out) => {
                        if dry_run {
                            this.helm_install_message = Some(format!(
                                "Helm server dry-run succeeded for release '{}'",
                                out.release_name
                            ));
                        } else {
                            this.helm_install_chart = None;
                            this.helm_install_message = None;
                            this.active_cluster_section = "helm-releases".into();
                            this.refresh_active_cluster_view(window, cx);
                        }
                    }
                    Err(err) => {
                        this.helm_install_message =
                            Some(format!("Error: {}", connection_error_message(err)));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_rbac_users_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let read_id = self.next_operation_id();
        let mgmt_id = self.next_operation_id();
        let ns = self.selected_namespace.clone();
        let mgmt_ns = ns.clone().unwrap_or_else(|| "default".into());
        let engine = self.connection.clone();
        cx.spawn_in(window, async move |this, cx| {
            let users_res = engine
                .rbac_users(RbacUsersRequest {
                    cluster_id: session.cluster_id.clone(),
                    session_id: session.session_id.clone(),
                    read_id,
                    namespace: ns,
                })
                .await;
            let mgmt_res = engine
                .rbac_management_snapshot(ManagementRequest {
                    cluster_id: session.cluster_id,
                    session_id: session.session_id,
                    read_id: mgmt_id,
                    namespace: mgmt_ns,
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.cluster_loading = false;
                if let Ok(u) = users_res {
                    this.rbac_users_response = Some(u);
                }
                if let Ok(m) = mgmt_res {
                    this.rbac_management_snapshot = Some(m);
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn plan_managed_rbac_user(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let name = self.rbac_new_user_input.read(cx).value().trim().to_string();
        if name.is_empty() {
            return;
        }
        let namespace = self
            .selected_namespace
            .clone()
            .unwrap_or_else(|| "default".into());
        let read_id = self.next_operation_id();
        let engine = self.connection.clone();
        cx.spawn_in(window, async move |this, cx| {
            let res = engine
                .rbac_user_plan(PlanRequest {
                    cluster_id: session.cluster_id,
                    session_id: session.session_id,
                    read_id,
                    operation: PlanOperation::Create,
                    user: ManagedUser {
                        name,
                        namespace,
                        uid: None,
                        resource_version: None,
                        assignments: vec![],
                        cluster_access: ClusterAccess::None,
                        cluster_template: None,
                    },
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                match res {
                    Ok(plan) => {
                        this.rbac_active_plan = Some(plan);
                        this.cluster_error = None;
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

    pub(crate) fn apply_managed_rbac_plan(
        &mut self,
        plan_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let read_id = self.next_operation_id();
        let engine = self.connection.clone();
        cx.spawn_in(window, async move |this, cx| {
            let res = engine
                .rbac_user_apply(ApplyRequest {
                    cluster_id: session.cluster_id,
                    session_id: session.session_id,
                    read_id,
                    plan_id,
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.rbac_active_plan = None;
                match res {
                    Ok(_) => {
                        this.load_rbac_users_view(window, cx);
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

    fn load_custom_definitions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let read_id = self.next_operation_id();
        let engine = self.connection.clone();
        cx.spawn_in(window, async move |this, cx| {
            let res = engine
                .custom_resource_definitions(CustomDefinitionRequest {
                    cluster_id: session.cluster_id,
                    session_id: session.session_id,
                    read_id,
                    continue_token: None,
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.cluster_loading = false;
                match res {
                    Ok(page) => {
                        this.custom_definitions_page = Some(page.clone());
                        this.resource_page = Some(page);
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

    fn load_custom_instances(
        &mut self,
        crd_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let uid = self
            .custom_definitions_page
            .as_ref()
            .and_then(|p| p.items.iter().find(|i| i.name == crd_name))
            .and_then(|i| i.uid.clone())
            .unwrap_or_default();
        if uid.is_empty() {
            self.cluster_loading = false;
            return;
        }
        let read_id = self.next_operation_id();
        let namespace = self.selected_namespace.clone();
        let engine = self.connection.clone();
        cx.spawn_in(window, async move |this, cx| {
            let res = engine
                .custom_resources(CustomResourceRequest {
                    cluster_id: session.cluster_id,
                    session_id: session.session_id,
                    read_id,
                    target: CustomResourceTarget {
                        definition_name: crd_name,
                        definition_uid: uid,
                    },
                    namespace,
                    continue_token: None,
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.cluster_loading = false;
                match res {
                    Ok(page) => {
                        this.custom_instances_page = Some(page);
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

    pub(crate) fn render_cluster_workspace(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let session = self.cluster_session.clone();
        let section = self.active_cluster_section.clone();
        let has_inspector = self.inspector.target.is_some();
        let has_dock = self.pod_dock_open;

        let main_body = v_flex()
            .size_full()
            .overflow_hidden()
            .child(self.render_cluster_header(session.as_ref(), window, cx))
            .when_some(self.cluster_error.clone(), |el, msg| {
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
                    .w_full()
                    .overflow_hidden()
                    .child(match section.as_str() {
                        "overview" => self.render_cluster_overview_body(cx).into_any_element(),
                        "workloads-overview" => {
                            self.render_workloads_overview_body(cx).into_any_element()
                        }
                        "helm-charts" | "helm-releases" => {
                            self.render_helm_body(window, cx).into_any_element()
                        }
                        "port-forwarding" => self.render_port_forwards_body(cx).into_any_element(),
                        "users" => self.render_rbac_users_body(window, cx).into_any_element(),
                        s if s.starts_with("crd:") => {
                            self.render_custom_instances_body(cx).into_any_element()
                        }
                        other => {
                            if let Some(view) = resource_view(other) {
                                self.render_resource_table_body(view, window, cx)
                                    .into_any_element()
                            } else {
                                self.render_cluster_overview_body(cx).into_any_element()
                            }
                        }
                    }),
            );

        let with_inspector = if has_inspector {
            h_resizable("cluster-inspector-split")
                .child(resizable_panel().child(main_body))
                .child(
                    resizable_panel()
                        .size(px(420.0))
                        .size_range(px(300.0)..px(760.0))
                        .child(self.render_inspector_panel(window, cx)),
                )
                .into_any_element()
        } else {
            main_body.into_any_element()
        };

        if has_dock {
            v_resizable("cluster-pod-dock-split")
                .child(resizable_panel().child(with_inspector))
                .child(
                    resizable_panel()
                        .size(px(260.0))
                        .size_range(px(160.0)..px(520.0))
                        .child(self.render_pod_dock_panel(window, cx)),
                )
                .into_any_element()
        } else {
            with_inspector
        }
    }

    fn render_cluster_header(
        &mut self,
        session: Option<&ConnectionSnapshot>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let context_name = session
            .map(|s| s.context_name.clone())
            .unwrap_or_else(|| "Cluster".into());
        let status = session.map(|s| s.status).unwrap_or("disconnected");
        let api_version = session.and_then(|s| s.api_version.clone());
        let section_title = destination_label(&self.active_cluster_section);
        let selected_ns = self
            .selected_namespace
            .clone()
            .unwrap_or_else(|| "All namespaces".into());

        v_flex()
            .w_full()
            .px_4()
            .py_2p5()
            .gap_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().secondary)
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(div().text_sm().font_semibold().child(context_name))
                            .child(match status {
                                "connected" => Badge::new().success().child("Connected"),
                                "connecting" => Badge::new().warning().child("Connecting"),
                                "error" => Badge::new().danger().child("Error"),
                                _ => Badge::new().secondary().child("Disconnected"),
                            })
                            .when_some(api_version, |el, ver| {
                                el.child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(ver),
                                )
                            })
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!("· {section_title}")),
                            ),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1p5()
                            .child(
                                Button::new("cluster-ns-all-btn")
                                    .xsmall()
                                    .when(self.selected_namespace.is_none(), |b| b.primary())
                                    .when(self.selected_namespace.is_some(), |b| b.outline())
                                    .label(format!("NS: {selected_ns}"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.selected_namespace = None;
                                        this.refresh_active_cluster_view(window, cx);
                                        cx.notify();
                                    })),
                            )
                            .children(self.namespaces.iter().take(6).cloned().map(|ns| {
                                let active =
                                    self.selected_namespace.as_deref() == Some(ns.as_str());
                                let ns_click = ns.clone();
                                Button::new(SharedString::from(format!("ns-chip-{ns}")))
                                    .xsmall()
                                    .when(active, |b| b.primary())
                                    .when(!active, |b| b.ghost())
                                    .label(ns)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.selected_namespace = Some(ns_click.clone());
                                        this.refresh_active_cluster_view(window, cx);
                                        cx.notify();
                                    }))
                            }))
                            .child(
                                Button::new("cluster-refresh-view-btn")
                                    .xsmall()
                                    .outline()
                                    .icon(IconName::Undo2)
                                    .label("Refresh")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.refresh_active_cluster_view(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("cluster-reconnect-btn")
                                    .xsmall()
                                    .outline()
                                    .label("Reconnect")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.reconnect_active_cluster(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("cluster-disconnect-btn")
                                    .xsmall()
                                    .ghost()
                                    .label("Disconnect")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.disconnect_active_cluster(cx);
                                    })),
                            ),
                    ),
            )
    }

    fn render_cluster_overview_body(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let snap = &self.overview_snapshot;
        let mut cpu_used = 0.0;
        let mut cpu_total = 0.0;
        let mut mem_used = 0.0;
        let mut mem_total = 0.0;
        for node_item in &snap.nodes {
            if let Some(node) = &node_item.node {
                if let Some(u) = node.cpu.used {
                    cpu_used += u;
                }
                if let Some(t) = node.cpu.allocatable.or(node.cpu.capacity) {
                    cpu_total += t;
                }
                if let Some(u) = node.memory.used {
                    mem_used += u;
                }
                if let Some(t) = node.memory.allocatable.or(node.memory.capacity) {
                    mem_total += t;
                }
            }
        }
        let cpu_pct = if cpu_total > 0.0 {
            ((cpu_used / cpu_total) * 100.0).round() as u64
        } else {
            0
        };
        let mem_pct = if mem_total > 0.0 {
            ((mem_used / mem_total) * 100.0).round() as u64
        } else {
            0
        };

        v_flex()
            .flex_1()
            .p_4()
            .gap_4()
            .overflow_y_scrollbar()
            .child(
                h_flex()
                    .w_full()
                    .gap_4()
                    .child(
                        v_flex()
                            .flex_1()
                            .p_4()
                            .gap_1()
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(cx.theme().border)
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Cluster CPU Usage"),
                            )
                            .child(div().text_lg().font_semibold().child(format!(
                                "{cpu_pct}% ({cpu_used:.2} / {cpu_total:.1} cores)"
                            ))),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .p_4()
                            .gap_1()
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(cx.theme().border)
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Cluster Memory Usage"),
                            )
                            .child(div().text_lg().font_semibold().child(format!(
                                "{mem_pct}% ({:.1} / {:.1} GiB)",
                                mem_used / (1024.0 * 1024.0 * 1024.0),
                                mem_total / (1024.0 * 1024.0 * 1024.0)
                            ))),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .p_4()
                            .gap_1()
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(cx.theme().border)
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Nodes & Pods"),
                            )
                            .child(div().text_lg().font_semibold().child(format!(
                                "{} Nodes · {} Pods",
                                snap.nodes.len(),
                                snap.pods.len()
                            ))),
                    ),
            )
            .child(self.render_workloads_cards_grid(cx))
            .child(self.render_latest_events_card(cx))
    }

    fn render_workloads_overview_body(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .flex_1()
            .p_4()
            .gap_4()
            .overflow_y_scrollbar()
            .child(self.render_workloads_cards_grid(cx))
            .child(self.render_latest_events_card(cx))
    }

    fn render_workloads_cards_grid(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let snap = &self.overview_snapshot;
        let cards = [
            ("pods", "Pods", snap.pods.len()),
            ("deployments", "Deployments", snap.deployments.len()),
            ("daemon-sets", "DaemonSets", snap.daemon_sets.len()),
            ("stateful-sets", "StatefulSets", snap.stateful_sets.len()),
            ("replica-sets", "ReplicaSets", snap.replica_sets.len()),
            ("jobs", "Jobs", snap.jobs.len()),
            ("cron-jobs", "CronJobs", snap.cron_jobs.len()),
        ];

        h_flex()
            .w_full()
            .gap_3()
            .flex_wrap()
            .children(cards.into_iter().map(|(dest, label, count)| {
                let target_dest = dest.to_string();
                v_flex()
                    .w(px(160.0))
                    .p_3()
                    .gap_1p5()
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().secondary)
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(label),
                    )
                    .child(div().text_xl().font_semibold().child(count.to_string()))
                    .child(
                        Button::new(SharedString::from(format!("workload-card-{dest}")))
                            .xsmall()
                            .ghost()
                            .label("View →")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.active_cluster_section = target_dest.clone();
                                this.refresh_active_cluster_view(window, cx);
                                cx.notify();
                            })),
                    )
            }))
    }

    fn render_latest_events_card(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let events = &self.overview_snapshot.events;
        v_flex()
            .w_full()
            .p_4()
            .gap_2()
            .rounded(px(8.0))
            .border_1()
            .border_color(cx.theme().border)
            .child(div().text_sm().font_semibold().child("Latest Cluster Events"))
            .children(events.iter().take(15).map(|ev| {
                let kind_type = ev
                    .columns
                    .get("type")
                    .cloned()
                    .unwrap_or_else(|| "Normal".into());
                let obj = ev
                    .columns
                    .get("involvedObject")
                    .cloned()
                    .unwrap_or_else(|| ev.name.clone());
                h_flex()
                    .w_full()
                    .py_1p5()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .items_center()
                    .justify_between()
                    .text_xs()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(if kind_type == "Warning" {
                                Badge::new().warning().child("Warning")
                            } else {
                                Badge::new().secondary().child(kind_type)
                            })
                            .child(div().font_medium().child(ev.status.clone()))
                            .child(
                                div()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(obj),
                            ),
                    )
                    .child(
                        div()
                            .text_color(cx.theme().muted_foreground)
                            .child(format_age(ev.created_at.as_deref())),
                    )
            }))
    }

    fn render_resource_table_body(
        &mut self,
        view: &'static ResourceView,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let items = self
            .resource_page
            .as_ref()
            .filter(|p| p.kind == view.kind)
            .map(|p| p.items.clone())
            .unwrap_or_default();
        let is_pods = view.kind == ResourceKind::Pods;
        let live_sync_on = self.active_pod_sync_id.is_some();

        v_flex()
            .flex_1()
            .p_4()
            .gap_3()
            .overflow_hidden()
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .w(px(240.0))
                                    .child(
                                        Input::new(&self.resource_filter_input)
                                            .small()
                                            .cleanable(true),
                                    ),
                            )
                            .child(
                                Button::new("apply-resource-filter-btn")
                                    .small()
                                    .outline()
                                    .label("Filter")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.load_resource_table(view, window, cx);
                                    })),
                            )
                            .children(view.statuses.iter().map(|status| {
                                let active =
                                    self.selected_status_filter.as_deref() == Some(*status);
                                let status_str = (*status).to_string();
                                Button::new(SharedString::from(format!("status-pill-{status}")))
                                    .xsmall()
                                    .when(active, |b| b.primary())
                                    .when(!active, |b| b.ghost())
                                    .label(*status)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if this.selected_status_filter.as_deref()
                                            == Some(status_str.as_str())
                                        {
                                            this.selected_status_filter = None;
                                        } else {
                                            this.selected_status_filter = Some(status_str.clone());
                                        }
                                        this.load_resource_table(view, window, cx);
                                        cx.notify();
                                    }))
                            })),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .when(is_pods, |el| {
                                el.child(
                                    Switch::new("pods-live-sync-switch")
                                        .label("Live Watch")
                                        .checked(live_sync_on)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.toggle_pod_live_sync(window, cx);
                                        })),
                                )
                            })
                            .when(view.creatable, |el| {
                                let kind = view.kind;
                                el.child(
                                    Button::new("create-resource-btn")
                                        .small()
                                        .primary()
                                        .icon(IconName::Plus)
                                        .label(format!("Create {}", view.label))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.open_create_resource_modal(kind, window, cx);
                                        })),
                                )
                            }),
                    ),
            )
            .when_some(self.creating_resource_kind, |el, kind| {
                el.child(
                    v_flex()
                        .w_full()
                        .h(px(260.0))
                        .p_3()
                        .gap_2()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().secondary)
                        .child(
                            h_flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_semibold()
                                        .child(format!("Create {:?} from YAML", kind)),
                                )
                                .child(
                                    h_flex()
                                        .gap_1p5()
                                        .child(
                                            Button::new("create-dry-run-btn")
                                                .xsmall()
                                                .outline()
                                                .label("Server Dry-Run")
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.submit_create_resource(true, window, cx);
                                                })),
                                        )
                                        .child(
                                            Button::new("create-submit-btn")
                                                .xsmall()
                                                .primary()
                                                .label("Create")
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.submit_create_resource(false, window, cx);
                                                })),
                                        )
                                        .child(
                                            Button::new("create-cancel-btn")
                                                .xsmall()
                                                .ghost()
                                                .icon(IconName::Close)
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.creating_resource_kind = None;
                                                    cx.notify();
                                                })),
                                        ),
                                ),
                        )
                        .when_some(self.create_modal_message.clone(), |el, msg| {
                            el.child(div().text_xs().child(msg))
                        })
                        .child(
                            div()
                                .flex_1()
                                .w_full()
                                .border_1()
                                .border_color(cx.theme().border)
                                .rounded(px(6.0))
                                .overflow_hidden()
                                .child(Editor::new(&self.create_yaml_editor).h_full()),
                        ),
                )
            })
            .child(
                v_flex()
                    .flex_1()
                    .w_full()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(px(8.0))
                    .overflow_hidden()
                    .child(
                        h_flex()
                            .w_full()
                            .px_3()
                            .py_2()
                            .bg(cx.theme().table_head)
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .text_xs()
                            .font_semibold()
                            .text_color(cx.theme().muted_foreground)
                            .children(view.columns.iter().map(|col| {
                                div().w(px(col.width)).truncate().child(col.label)
                            }))
                            .child(div().w(px(140.0)).child("Actions")),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .w_full()
                            .overflow_y_scrollbar()
                            .children(items.into_iter().enumerate().map(|(idx, item)| {
                                let reference = ResourceReference {
                                    kind: view.kind,
                                    namespace: item.namespace.clone(),
                                    name: item.name.clone(),
                                    uid: item.uid.clone(),
                                };
                                let del_item = item.clone();
                                let kind = view.kind;
                                let is_crd = kind == ResourceKind::CustomResourceDefinitions;
                                let crd_name = item.name.clone();

                                h_flex()
                                    .id(SharedString::from(format!("res-row-{idx}")))
                                    .w_full()
                                    .px_3()
                                    .py_2()
                                    .border_b_1()
                                    .border_color(cx.theme().border)
                                    .items_center()
                                    .text_xs()
                                    .hover(|s| s.bg(cx.theme().table_hover))
                                    .children(view.columns.iter().map(|col| {
                                        div()
                                            .w(px(col.width))
                                            .truncate()
                                            .child(cell_value(&item, col.key))
                                    }))
                                    .child(
                                        h_flex()
                                            .w(px(140.0))
                                            .gap_1()
                                            .items_center()
                                            .child(
                                                Button::new(SharedString::from(format!(
                                                    "inspect-row-{idx}"
                                                )))
                                                .xsmall()
                                                .outline()
                                                .label("Inspect")
                                                .on_click(cx.listener(move |this, _, window, cx| {
                                                    this.open_resource_inspector(
                                                        reference.clone(),
                                                        window,
                                                        cx,
                                                    );
                                                })),
                                            )
                                            .when(is_crd, |el| {
                                                el.child(
                                                    Button::new(SharedString::from(format!(
                                                        "crd-instances-{idx}"
                                                    )))
                                                    .xsmall()
                                                    .primary()
                                                    .label("Instances")
                                                    .on_click(cx.listener(
                                                        move |this, _, window, cx| {
                                                            this.active_cluster_section =
                                                                format!("crd:{crd_name}");
                                                            this.refresh_active_cluster_view(
                                                                window, cx,
                                                            );
                                                            cx.notify();
                                                        },
                                                    )),
                                                )
                                            })
                                            .child(
                                                Button::new(SharedString::from(format!(
                                                    "delete-row-{idx}"
                                                )))
                                                .xsmall()
                                                .ghost()
                                                .icon(IconName::Trash)
                                                .on_click(cx.listener(move |this, _, window, cx| {
                                                    this.delete_resource_row(
                                                        kind,
                                                        del_item.clone(),
                                                        window,
                                                        cx,
                                                    );
                                                })),
                                            ),
                                    )
                            })),
                    ),
            )
    }

    fn render_helm_body(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let page = self.helm_page.clone();
        let install_chart = self.helm_install_chart.clone();

        v_flex()
            .flex_1()
            .p_4()
            .gap_3()
            .overflow_hidden()
            .when_some(install_chart, |el, chart_name| {
                el.child(
                    v_flex()
                        .w_full()
                        .h(px(260.0))
                        .p_3()
                        .gap_2()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().secondary)
                        .child(
                            h_flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_semibold()
                                        .child(format!("Install Helm Chart: {chart_name}")),
                                )
                                .child(
                                    h_flex()
                                        .gap_1p5()
                                        .child(
                                            div().w(px(140.0)).child(
                                                Input::new(&self.helm_release_name_input).xsmall(),
                                            ),
                                        )
                                        .child(
                                            div().w(px(120.0)).child(
                                                Input::new(&self.helm_namespace_input).xsmall(),
                                            ),
                                        )
                                        .child(
                                            Button::new("helm-dry-run-btn")
                                                .xsmall()
                                                .outline()
                                                .label("Dry-Run")
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.submit_helm_install(true, window, cx);
                                                })),
                                        )
                                        .child(
                                            Button::new("helm-install-btn")
                                                .xsmall()
                                                .primary()
                                                .label("Install")
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.submit_helm_install(false, window, cx);
                                                })),
                                        )
                                        .child(
                                            Button::new("helm-cancel-install-btn")
                                                .xsmall()
                                                .ghost()
                                                .icon(IconName::Close)
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.helm_install_chart = None;
                                                    cx.notify();
                                                })),
                                        ),
                                ),
                        )
                        .when_some(self.helm_install_message.clone(), |el, msg| {
                            el.child(div().text_xs().child(msg))
                        })
                        .child(
                            div()
                                .flex_1()
                                .w_full()
                                .border_1()
                                .border_color(cx.theme().border)
                                .rounded(px(6.0))
                                .overflow_hidden()
                                .child(Editor::new(&self.helm_values_editor).h_full()),
                        ),
                )
            })
            .child(if let Some(page) = page {
                let view = page.view;
                v_flex()
                    .flex_1()
                    .w_full()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(px(8.0))
                    .overflow_y_scrollbar()
                    .children(page.items.into_iter().enumerate().map(|(idx, item)| {
                        let name = item.name.clone();
                        let ns = item.namespace.clone();
                        let ver = item.version.clone();
                        let inst_name = item.name.clone();
                        let inst_ver = item.version.clone();

                        h_flex()
                            .id(SharedString::from(format!("helm-row-{idx}")))
                            .w_full()
                            .px_4()
                            .py_2p5()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .items_center()
                            .justify_between()
                            .text_xs()
                            .child(
                                h_flex()
                                    .gap_3()
                                    .items_center()
                                    .child(div().w(px(220.0)).font_semibold().child(item.name))
                                    .child(div().w(px(120.0)).child(item.version))
                                    .child(
                                        div()
                                            .w(px(120.0))
                                            .text_color(cx.theme().muted_foreground)
                                            .child(item.namespace.unwrap_or_else(|| "repo".into())),
                                    )
                                    .when_some(item.status, |el, st| {
                                        el.child(Badge::new().secondary().child(st))
                                    }),
                            )
                            .child(
                                h_flex()
                                    .gap_1p5()
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "helm-inspect-{idx}"
                                        )))
                                        .xsmall()
                                        .outline()
                                        .label("Details")
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.open_helm_inspector(
                                                view,
                                                name.clone(),
                                                ns.clone(),
                                                (view == HelmView::Charts).then(|| ver.clone()),
                                                window,
                                                cx,
                                            );
                                        })),
                                    )
                                    .when(view == HelmView::Charts, |el| {
                                        el.child(
                                            Button::new(SharedString::from(format!(
                                                "helm-open-install-{idx}"
                                            )))
                                            .xsmall()
                                            .primary()
                                            .label("Install")
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.helm_install_chart = Some(inst_name.clone());
                                                this.helm_install_version = Some(inst_ver.clone());
                                                this.helm_values_editor.update(cx, |ed, cx| {
                                                    ed.set_value("replicaCount: 1\n", window, cx);
                                                });
                                                cx.notify();
                                            })),
                                        )
                                    }),
                            )
                    }))
                    .into_any_element()
            } else {
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("No Helm entries loaded.")
                    .into_any_element()
            })
    }

    fn render_port_forwards_body(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let forwards = self.port_forwards.clone();
        v_flex()
            .flex_1()
            .p_4()
            .gap_3()
            .overflow_y_scrollbar()
            .child(div().text_sm().font_semibold().child("Active Port Forwards"))
            .children(forwards.into_iter().map(|fwd| {
                let id = fwd.id.clone();
                h_flex()
                    .w_full()
                    .p_3()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(cx.theme().border)
                    .items_center()
                    .justify_between()
                    .text_xs()
                    .child(div().font_medium().child(format!(
                        "{}/{} · 127.0.0.1:{} -> :{}",
                        fwd.namespace, fwd.name, fwd.local_port, fwd.remote_port
                    )))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(Badge::new().success().child(fwd.status))
                            .child(
                                Button::new(SharedString::from(format!("stop-fwd-{id}")))
                                    .xsmall()
                                    .danger()
                                    .label("Stop")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.stop_port_forward(id.clone(), cx);
                                    })),
                            ),
                    )
            }))
    }

    fn render_rbac_users_body(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let users: Option<RbacUsersResponse> = self.rbac_users_response.clone();
        let mgmt: Option<ManagementSnapshot> = self.rbac_management_snapshot.clone();
        let plan: Option<UserPlan> = self.rbac_active_plan.clone();

        v_flex()
            .flex_1()
            .p_4()
            .gap_4()
            .overflow_y_scrollbar()
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .child("Managed RBAC ServiceAccounts & Subjects"),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .w(px(200.0))
                                    .child(Input::new(&self.rbac_new_user_input).small()),
                            )
                            .child(
                                Button::new("rbac-plan-user-btn")
                                    .small()
                                    .primary()
                                    .label("Dry-Run Create Plan")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.plan_managed_rbac_user(window, cx);
                                    })),
                            ),
                    ),
            )
            .when_some(plan, |el, p| {
                let plan_id = p.plan_id.clone();
                el.child(
                    v_flex()
                        .p_3()
                        .gap_2()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(cx.theme().primary)
                        .bg(cx.theme().secondary)
                        .text_xs()
                        .child(div().font_semibold().child(format!(
                            "Server Dry-Run Plan Ready for '{}': {} step(s)",
                            p.user.name,
                            p.steps.len()
                        )))
                        .child(
                            Button::new("rbac-apply-plan-btn")
                                .xsmall()
                                .primary()
                                .label("Apply Plan")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.apply_managed_rbac_plan(plan_id.clone(), window, cx);
                                })),
                        ),
                )
            })
            .when_some(mgmt, |el, m| {
                el.child(
                    v_flex()
                        .gap_2()
                        .child(
                            div().text_xs().font_semibold().child(format!(
                                "Managed ServiceAccounts in '{}' ({}) · Templates ({})",
                                m.namespace,
                                m.users.len(),
                                m.templates.len()
                            )),
                        )
                        .children(m.users.into_iter().map(|u| {
                            h_flex()
                                .justify_between()
                                .p_2()
                                .rounded(px(6.0))
                                .border_1()
                                .border_color(cx.theme().border)
                                .text_xs()
                                .child(div().font_medium().child(format!(
                                    "{}/{}",
                                    u.namespace, u.name
                                )))
                                .child(
                                    Badge::new()
                                        .secondary()
                                        .child(format!("{} assignment(s)", u.assignments.len())),
                                )
                        })),
                )
            })
            .when_some(users, |el, resp| {
                el.child(
                    v_flex()
                        .gap_1p5()
                        .child(
                            div()
                                .text_xs()
                                .font_semibold()
                                .child(format!("Cluster RBAC Subjects ({})", resp.items.len())),
                        )
                        .children(resp.items.into_iter().take(50).map(|subj| {
                            h_flex()
                                .justify_between()
                                .p_2()
                                .rounded(px(6.0))
                                .border_1()
                                .border_color(cx.theme().border)
                                .text_xs()
                                .child(div().font_medium().child(format!(
                                    "[{}] {}",
                                    subj.kind, subj.name
                                )))
                                .child(
                                    div()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!("{} binding(s)", subj.bindings.len())),
                                )
                        })),
                )
            })
    }

    fn render_custom_instances_body(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let page: Option<CustomResourcePage> = self.custom_instances_page.clone();
        v_flex()
            .flex_1()
            .p_4()
            .gap_3()
            .overflow_y_scrollbar()
            .child(if let Some(page) = page {
                v_flex()
                    .gap_2()
                    .child(div().text_sm().font_semibold().child(format!(
                        "{} ({}/{})",
                        page.kind, page.group, page.version
                    )))
                    .children(page.items.into_iter().map(|item| {
                        h_flex()
                            .justify_between()
                            .p_2p5()
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(cx.theme().border)
                            .text_xs()
                            .child(div().font_medium().child(item.name))
                            .child(
                                div()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(item.namespace.unwrap_or_else(|| "Cluster".into())),
                            )
                            .child(div().child(format_age(item.created_at.as_deref())))
                    }))
                    .into_any_element()
            } else {
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Select a CustomResourceDefinition to inspect its instances.")
                    .into_any_element()
            })
    }
}
