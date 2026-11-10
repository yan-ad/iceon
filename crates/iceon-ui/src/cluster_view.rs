//! Cluster workspace views: Overview, 41 Kubernetes Resource tables, Port Forwarding, Helm, Custom Resources, and RBAC Users.

use std::rc::Rc;
use std::time::Duration;

use crate::navigation::{cell_value, format_age, resource_view_by_kind};
use gpui_kit::component::{
    ActiveTheme as _, IconName, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Editor, Input},
    scroll::ScrollableElement as _,
    select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState},
    tag::Tag,
    v_flex, v_virtual_list,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NamespaceChoice {
    pub(crate) name: String,
}

impl SelectItem for NamespaceChoice {
    type Value = String;

    fn title(&self) -> SharedString {
        self.name.clone().into()
    }
    fn value(&self) -> &Self::Value {
        &self.name
    }
    fn matches(&self, query: &str) -> bool {
        self.name
            .to_lowercase()
            .contains(&query.trim().to_lowercase())
    }
}

pub(crate) type NamespaceSelectState = SelectState<SearchableVec<NamespaceChoice>>;

pub(crate) fn create_namespace_select(
    window: &mut Window,
    cx: &mut Context<IceonApp>,
) -> (Entity<NamespaceSelectState>, Vec<NamespaceChoice>) {
    let items = vec![NamespaceChoice {
        name: "All Namespaces".into(),
    }];
    let select = cx.new(|cx| {
        SelectState::new(SearchableVec::new(items.clone()), None, window, cx).searchable(true)
    });
    cx.subscribe_in(
        &select,
        window,
        |this, _, event: &SelectEvent<SearchableVec<NamespaceChoice>>, window, cx| {
            let SelectEvent::Confirm(Some(namespace)) = event else {
                return;
            };
            this.selected_namespace = (namespace != "All Namespaces").then(|| namespace.clone());
            this.current_continue_token = None;
            this.continue_token_history.clear();
            this.reload_active_section(window, cx);
        },
    )
    .detach();
    (select, items)
}
use gpui_kit::prelude::*;
use gpui_kit::*;

use iceon_core::connection::{
    OpenRequest, ResourceItem, ResourceKind, ResourcePage, ResourceRequest, SessionRequest,
    create::{CreateRequest, CreateResult},
    custom_resources::{CustomResourcePage, CustomResourceRequest, CustomResourceTarget},
    details::ResourceReference,
    helm::{
        HelmRequest, HelmView,
        install::{HelmInstallRequest, HelmInstallResult},
    },
    namespace_cache::NamespaceRefreshRequest,
    node_actions::NodeAction,
    pod_runtime::PodStreamMode,
    pod_sync::PodSyncRequest,
    rbac::RbacUsersRequest,
    rbac_management::{
        ApplyRequest, Assignment, ClusterAccess, ManagedUser, ManagementRequest, PlanOperation,
        PlanRequest, TemplateIdentity,
    },
    workload_actions::WorkloadAction,
};

use crate::{
    IceonApp,
    inspector_view::DeleteTarget,
    navigation::{Route, destination_label, normalize_destination, resource_view},
};

#[derive(Clone, Debug, Default)]
pub struct OverviewSnapshot {
    pub nodes: Option<ResourcePage>,
    pub namespaces: Option<ResourcePage>,
    pub pods: Option<ResourcePage>,
    pub deployments: Option<ResourcePage>,
    pub daemonsets: Option<ResourcePage>,
    pub statefulsets: Option<ResourcePage>,
    pub replicasets: Option<ResourcePage>,
    pub jobs: Option<ResourcePage>,
    pub cronjobs: Option<ResourcePage>,
    pub events: Option<ResourcePage>,
}

#[derive(Clone, Copy)]
pub struct ColumnDef {
    pub header: &'static str,
    pub key: &'static str,
    pub width: f32,
}

pub fn section_to_resource_kind(section: &str) -> Option<ResourceKind> {
    if let Some(view) = resource_view(section) {
        return Some(view.kind);
    }
    match section {
        "workloads/pods" => Some(ResourceKind::Pods),
        "workloads/deployments" => Some(ResourceKind::Deployments),
        "workloads/daemonsets" => Some(ResourceKind::DaemonSets),
        "workloads/statefulsets" => Some(ResourceKind::StatefulSets),
        "workloads/replicasets" => Some(ResourceKind::ReplicaSets),
        "workloads/replicationcontrollers" => Some(ResourceKind::ReplicationControllers),
        "workloads/jobs" => Some(ResourceKind::Jobs),
        "workloads/cronjobs" => Some(ResourceKind::CronJobs),
        _ => None,
    }
}

pub fn parse_custom_resource_target(section: &str) -> Option<(Option<String>, Option<String>)> {
    if let Some(def) = section.strip_prefix("crd:")
        && !def.is_empty()
    {
        return Some((None, Some(def.to_owned())));
    }
    if let Some(rest) = section.strip_prefix("custom-resources/") {
        let mut parts = rest.splitn(2, '/');
        let group = parts.next().filter(|s| !s.is_empty()).map(str::to_owned);
        let name = parts.next().filter(|s| !s.is_empty()).map(str::to_owned);
        return Some((group, name));
    }
    None
}

pub fn columns_for_kind(kind: ResourceKind) -> &'static [ColumnDef] {
    use ResourceKind::*;
    match kind {
        Namespaces => &[ColumnDef {
            header: "Labels",
            key: "labels",
            width: 260.0,
        }],
        Nodes => &[
            ColumnDef {
                header: "CPU",
                key: "cpu",
                width: 120.0,
            },
            ColumnDef {
                header: "Memory",
                key: "memory",
                width: 125.0,
            },
            ColumnDef {
                header: "Roles",
                key: "roles",
                width: 130.0,
            },
            ColumnDef {
                header: "Taints",
                key: "taints",
                width: 70.0,
            },
            ColumnDef {
                header: "Version",
                key: "version",
                width: 115.0,
            },
            ColumnDef {
                header: "Schedulable",
                key: "schedulable",
                width: 95.0,
            },
        ],
        Pods => &[
            ColumnDef {
                header: "Ready",
                key: "ready",
                width: 80.0,
            },
            ColumnDef {
                header: "Restarts",
                key: "restarts",
                width: 80.0,
            },
            ColumnDef {
                header: "Controlled By",
                key: "controlledBy",
                width: 150.0,
            },
            ColumnDef {
                header: "Node",
                key: "node",
                width: 140.0,
            },
            ColumnDef {
                header: "QoS",
                key: "qos",
                width: 100.0,
            },
        ],
        Deployments | StatefulSets | ReplicaSets | ReplicationControllers => &[
            ColumnDef {
                header: "Desired",
                key: "desired",
                width: 80.0,
            },
            ColumnDef {
                header: "Replicas",
                key: "replicas",
                width: 80.0,
            },
            ColumnDef {
                header: "Ready",
                key: "ready",
                width: 80.0,
            },
            ColumnDef {
                header: "Available",
                key: "available",
                width: 90.0,
            },
        ],
        DaemonSets => &[
            ColumnDef {
                header: "Desired",
                key: "desired",
                width: 80.0,
            },
            ColumnDef {
                header: "Scheduled",
                key: "scheduled",
                width: 90.0,
            },
            ColumnDef {
                header: "Ready",
                key: "ready",
                width: 80.0,
            },
            ColumnDef {
                header: "Available",
                key: "available",
                width: 90.0,
            },
        ],
        Jobs => &[
            ColumnDef {
                header: "Completions",
                key: "completions",
                width: 100.0,
            },
            ColumnDef {
                header: "Active",
                key: "active",
                width: 80.0,
            },
            ColumnDef {
                header: "Failed",
                key: "failed",
                width: 80.0,
            },
        ],
        CronJobs => &[
            ColumnDef {
                header: "Schedule",
                key: "schedule",
                width: 130.0,
            },
            ColumnDef {
                header: "Suspend",
                key: "suspend",
                width: 80.0,
            },
            ColumnDef {
                header: "Active",
                key: "active",
                width: 80.0,
            },
            ColumnDef {
                header: "Last Schedule",
                key: "lastSchedule",
                width: 160.0,
            },
        ],
        ConfigMaps => &[ColumnDef {
            header: "Keys",
            key: "keys",
            width: 240.0,
        }],
        Secrets => &[
            ColumnDef {
                header: "Type",
                key: "type",
                width: 180.0,
            },
            ColumnDef {
                header: "Keys",
                key: "keys",
                width: 160.0,
            },
            ColumnDef {
                header: "Labels",
                key: "labels",
                width: 120.0,
            },
        ],
        Services => &[
            ColumnDef {
                header: "Type",
                key: "type",
                width: 110.0,
            },
            ColumnDef {
                header: "Cluster IP",
                key: "clusterIP",
                width: 130.0,
            },
            ColumnDef {
                header: "Ports",
                key: "ports",
                width: 150.0,
            },
        ],
        Endpoints => &[
            ColumnDef {
                header: "Subsets",
                key: "subsets",
                width: 85.0,
            },
            ColumnDef {
                header: "Ready",
                key: "readyAddresses",
                width: 85.0,
            },
            ColumnDef {
                header: "Not Ready",
                key: "notReadyAddresses",
                width: 95.0,
            },
            ColumnDef {
                header: "Ports",
                key: "ports",
                width: 85.0,
            },
        ],
        EndpointSlices => &[
            ColumnDef {
                header: "Address Type",
                key: "addressType",
                width: 110.0,
            },
            ColumnDef {
                header: "Endpoints",
                key: "endpoints",
                width: 95.0,
            },
            ColumnDef {
                header: "Ready",
                key: "ready",
                width: 80.0,
            },
            ColumnDef {
                header: "Ports",
                key: "ports",
                width: 80.0,
            },
        ],
        Ingresses => &[
            ColumnDef {
                header: "Class",
                key: "class",
                width: 120.0,
            },
            ColumnDef {
                header: "Rules",
                key: "rules",
                width: 80.0,
            },
            ColumnDef {
                header: "TLS",
                key: "tls",
                width: 80.0,
            },
        ],
        IngressClasses => &[
            ColumnDef {
                header: "Controller",
                key: "controller",
                width: 220.0,
            },
            ColumnDef {
                header: "Params Kind",
                key: "parametersKind",
                width: 120.0,
            },
        ],
        NetworkPolicies => &[
            ColumnDef {
                header: "Policy Types",
                key: "policyTypes",
                width: 140.0,
            },
            ColumnDef {
                header: "Ingress",
                key: "ingressRules",
                width: 80.0,
            },
            ColumnDef {
                header: "Egress",
                key: "egressRules",
                width: 80.0,
            },
        ],
        PersistentVolumes => &[
            ColumnDef {
                header: "Capacity",
                key: "capacity",
                width: 100.0,
            },
            ColumnDef {
                header: "Access Modes",
                key: "accessModes",
                width: 130.0,
            },
            ColumnDef {
                header: "Storage Class",
                key: "storageClass",
                width: 130.0,
            },
            ColumnDef {
                header: "Claim",
                key: "claim",
                width: 140.0,
            },
        ],
        PersistentVolumeClaims => &[
            ColumnDef {
                header: "Storage Class",
                key: "storageClass",
                width: 130.0,
            },
            ColumnDef {
                header: "Capacity",
                key: "capacity",
                width: 100.0,
            },
            ColumnDef {
                header: "Requested",
                key: "requested",
                width: 100.0,
            },
            ColumnDef {
                header: "Access Modes",
                key: "accessModes",
                width: 130.0,
            },
        ],
        StorageClasses => &[
            ColumnDef {
                header: "Provisioner",
                key: "provisioner",
                width: 200.0,
            },
            ColumnDef {
                header: "Reclaim",
                key: "reclaim",
                width: 100.0,
            },
            ColumnDef {
                header: "Binding Mode",
                key: "bindingMode",
                width: 140.0,
            },
        ],
        HorizontalPodAutoscalers => &[
            ColumnDef {
                header: "Target",
                key: "target",
                width: 140.0,
            },
            ColumnDef {
                header: "Min",
                key: "min",
                width: 65.0,
            },
            ColumnDef {
                header: "Max",
                key: "max",
                width: 65.0,
            },
            ColumnDef {
                header: "Current",
                key: "current",
                width: 75.0,
            },
            ColumnDef {
                header: "Desired",
                key: "desired",
                width: 75.0,
            },
        ],
        VerticalPodAutoscalers => &[
            ColumnDef {
                header: "Target",
                key: "target",
                width: 150.0,
            },
            ColumnDef {
                header: "Kind",
                key: "targetKind",
                width: 110.0,
            },
            ColumnDef {
                header: "Update Mode",
                key: "updateMode",
                width: 110.0,
            },
        ],
        PodDisruptionBudgets => &[
            ColumnDef {
                header: "Min Avail",
                key: "minAvailable",
                width: 90.0,
            },
            ColumnDef {
                header: "Max Unavail",
                key: "maxUnavailable",
                width: 95.0,
            },
            ColumnDef {
                header: "Allowed",
                key: "allowedDisruptions",
                width: 80.0,
            },
            ColumnDef {
                header: "Healthy",
                key: "currentHealthy",
                width: 80.0,
            },
        ],
        ResourceQuotas => &[
            ColumnDef {
                header: "CPU Used",
                key: "cpuUsed",
                width: 90.0,
            },
            ColumnDef {
                header: "CPU Hard",
                key: "cpuHard",
                width: 90.0,
            },
            ColumnDef {
                header: "Mem Used",
                key: "memoryUsed",
                width: 95.0,
            },
            ColumnDef {
                header: "Mem Hard",
                key: "memoryHard",
                width: 95.0,
            },
        ],
        LimitRanges => &[
            ColumnDef {
                header: "Limits",
                key: "limits",
                width: 75.0,
            },
            ColumnDef {
                header: "Types",
                key: "types",
                width: 150.0,
            },
            ColumnDef {
                header: "Default CPU",
                key: "defaultCpu",
                width: 100.0,
            },
            ColumnDef {
                header: "Default Mem",
                key: "defaultMemory",
                width: 100.0,
            },
        ],
        PriorityClasses => &[
            ColumnDef {
                header: "Value",
                key: "value",
                width: 95.0,
            },
            ColumnDef {
                header: "Global Default",
                key: "globalDefault",
                width: 110.0,
            },
            ColumnDef {
                header: "Preemption",
                key: "preemptionPolicy",
                width: 140.0,
            },
        ],
        RuntimeClasses => &[
            ColumnDef {
                header: "Handler",
                key: "handler",
                width: 150.0,
            },
            ColumnDef {
                header: "Overhead CPU",
                key: "overheadCpu",
                width: 110.0,
            },
            ColumnDef {
                header: "Overhead Mem",
                key: "overheadMemory",
                width: 110.0,
            },
        ],
        Leases => &[
            ColumnDef {
                header: "Holder",
                key: "holder",
                width: 200.0,
            },
            ColumnDef {
                header: "Duration (s)",
                key: "durationSeconds",
                width: 95.0,
            },
            ColumnDef {
                header: "Renewed",
                key: "renewed",
                width: 165.0,
            },
        ],
        Events => &[
            ColumnDef {
                header: "Reason",
                key: "reason",
                width: 130.0,
            },
            ColumnDef {
                header: "Object",
                key: "involvedObject",
                width: 170.0,
            },
            ColumnDef {
                header: "Count",
                key: "count",
                width: 65.0,
            },
            ColumnDef {
                header: "Message",
                key: "message",
                width: 280.0,
            },
        ],
        Roles | ClusterRoles => &[ColumnDef {
            header: "Rules",
            key: "rules",
            width: 90.0,
        }],
        RoleBindings | ClusterRoleBindings => &[
            ColumnDef {
                header: "Role Ref",
                key: "roleRef",
                width: 170.0,
            },
            ColumnDef {
                header: "Subjects",
                key: "subjects",
                width: 90.0,
            },
        ],
        ServiceAccounts => &[
            ColumnDef {
                header: "Secrets",
                key: "secrets",
                width: 85.0,
            },
            ColumnDef {
                header: "Automount",
                key: "automountToken",
                width: 95.0,
            },
        ],
        MutatingWebhookConfigurations | ValidatingWebhookConfigurations => &[ColumnDef {
            header: "Webhooks",
            key: "webhooks",
            width: 120.0,
        }],
        ValidatingAdmissionPolicies => &[
            ColumnDef {
                header: "Failure Policy",
                key: "failurePolicy",
                width: 130.0,
            },
            ColumnDef {
                header: "Validations",
                key: "validations",
                width: 100.0,
            },
            ColumnDef {
                header: "Param Kind",
                key: "paramKind",
                width: 140.0,
            },
        ],
        ValidatingAdmissionPolicyBindings => &[
            ColumnDef {
                header: "Policy",
                key: "policyName",
                width: 190.0,
            },
            ColumnDef {
                header: "Param Ref",
                key: "paramRef",
                width: 150.0,
            },
            ColumnDef {
                header: "Actions",
                key: "validationActions",
                width: 130.0,
            },
        ],
        PodSecurityPolicies => &[
            ColumnDef {
                header: "Privileged",
                key: "privileged",
                width: 110.0,
            },
            ColumnDef {
                header: "Volumes",
                key: "volumes",
                width: 220.0,
            },
        ],
        CustomResourceDefinitions => &[
            ColumnDef {
                header: "Group",
                key: "group",
                width: 180.0,
            },
            ColumnDef {
                header: "Kind",
                key: "kind",
                width: 140.0,
            },
            ColumnDef {
                header: "Scope",
                key: "scope",
                width: 100.0,
            },
            ColumnDef {
                header: "Versions",
                key: "versions",
                width: 120.0,
            },
        ],
    }
}

pub fn namespaced_kind(kind: ResourceKind) -> bool {
    !matches!(
        kind,
        ResourceKind::Nodes
            | ResourceKind::Namespaces
            | ResourceKind::PersistentVolumes
            | ResourceKind::StorageClasses
            | ResourceKind::PriorityClasses
            | ResourceKind::RuntimeClasses
            | ResourceKind::IngressClasses
            | ResourceKind::MutatingWebhookConfigurations
            | ResourceKind::ValidatingWebhookConfigurations
            | ResourceKind::ValidatingAdmissionPolicies
            | ResourceKind::ValidatingAdmissionPolicyBindings
            | ResourceKind::PodSecurityPolicies
            | ResourceKind::ClusterRoles
            | ResourceKind::ClusterRoleBindings
            | ResourceKind::CustomResourceDefinitions
    )
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
        self.active_cluster_section = normalized.clone();
        self.cluster_error = None;
        self.current_continue_token = None;
        self.continue_token_history.clear();

        let same_cluster = self
            .cluster_session
            .as_ref()
            .is_some_and(|s| s.cluster_id == cluster_id && s.status == "connected");

        if same_cluster {
            self.load_active_cluster_section(window, cx);
            return;
        }

        self.stop_active_pod_sync();
        self.cluster_loading = true;
        self.namespaces.clear();
        self.resource_page = None;
        self.helm_page = None;
        self.custom_definitions_page = None;
        self.custom_instances_page = None;
        self.rbac_users_response = None;
        self.rbac_management_snapshot = None;
        self.rbac_active_plan = None;
        self.rbac_apply_response = None;
        cx.notify();

        let connection = self.connection.clone();
        let ns_read_id = self.next_operation_id();
        cx.spawn_in(window, async move |this, cx| {
            let opened = match connection
                .open(OpenRequest {
                    cluster_id: cluster_id.clone(),
                })
                .await
            {
                Ok(snap) => snap,
                Err(err) => {
                    let _ = this.update_in(cx, |this, _, cx| {
                        this.cluster_loading = false;
                        this.cluster_error = Some(format!("Failed to open cluster: {err:?}"));
                        cx.notify();
                    });
                    return;
                }
            };

            let scope = SessionRequest {
                cluster_id: opened.cluster_id.clone(),
                session_id: opened.session_id.clone(),
            };
            let connected = connection.connect(scope.clone()).await;

            let ns_list = match connection.namespace_cache_get(scope).await {
                Ok(Some(cached)) => cached.names,
                _ => connection
                    .namespace_cache_refresh(NamespaceRefreshRequest {
                        cluster_id: opened.cluster_id.clone(),
                        session_id: opened.session_id.clone(),
                        read_id: ns_read_id,
                    })
                    .await
                    .map(|snap| snap.names)
                    .unwrap_or_default(),
            };

            let _ = this.update_in(cx, |this, window, cx| {
                this.cluster_loading = false;
                match connected {
                    Ok(snap) => {
                        let status = snap.status;
                        let err = snap.error;
                        this.cluster_session = Some(snap);
                        this.namespaces = ns_list;
                        this.refresh_catalog(cx);
                        if status == "connected" {
                            this.load_active_cluster_section(window, cx);
                        } else if let Some(e) = err {
                            this.cluster_error = Some(format!("Connection error: {e:?}"));
                        }
                    }
                    Err(err) => {
                        this.cluster_error = Some(format!("Connect failed: {err:?}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn stop_active_pod_sync(&mut self) {
        if let (Some(sync_id), Some(session)) = (
            self.active_pod_sync_id.take(),
            self.cluster_session.as_ref(),
        ) {
            let _ = self.connection.pod_sync_close(PodSyncRequest {
                cluster_id: session.cluster_id.clone(),
                session_id: session.session_id.clone(),
                sync_id,
            });
        }
    }

    pub(crate) fn reload_active_section(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.load_active_cluster_section(window, cx);
    }

    pub(crate) fn load_active_cluster_section(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stop_active_pod_sync();
        let section = self.active_cluster_section.clone();

        if section == "overview"
            || section == "workloads-overview"
            || section == "workloads/overview"
        {
            self.load_overview_snapshot(window, cx);
            return;
        }
        if section == "helm-charts" || section == "helm/charts" {
            self.load_helm_section(HelmView::Charts, window, cx);
            return;
        }
        if section == "helm-releases" || section == "helm/releases" {
            self.load_helm_section(HelmView::Releases, window, cx);
            return;
        }
        if section == "users" || section == "access-control/users" {
            self.load_rbac_users_section(window, cx);
            return;
        }
        if section == "port-forwarding" {
            self.pod_dock_open = true;
            cx.notify();
            return;
        }
        if let Some((group, crd_name)) = parse_custom_resource_target(&section) {
            self.load_custom_resources_section(group, crd_name, window, cx);
            return;
        }
        if let Some(kind) = section_to_resource_kind(&section) {
            self.load_resource_table(kind, window, cx);
            if kind == ResourceKind::Pods
                && self.current_continue_token.is_none()
                && self
                    .resource_filter_input
                    .read(cx)
                    .value()
                    .trim()
                    .is_empty()
                && self.selected_status_filter.is_none()
            {
                self.start_pod_sync(window, cx);
            }
        }
    }

    fn load_overview_snapshot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        if session.status != "connected" {
            return;
        }
        let namespace = self.selected_namespace.clone();
        let connection = self.connection.clone();
        let kinds = [
            ResourceKind::Nodes,
            ResourceKind::Namespaces,
            ResourceKind::Pods,
            ResourceKind::Deployments,
            ResourceKind::DaemonSets,
            ResourceKind::StatefulSets,
            ResourceKind::ReplicaSets,
            ResourceKind::Jobs,
            ResourceKind::CronJobs,
            ResourceKind::Events,
        ];
        let requests: Vec<(ResourceKind, ResourceRequest)> = kinds
            .into_iter()
            .map(|kind| {
                let read_id = self.next_operation_id();
                (
                    kind,
                    ResourceRequest {
                        cluster_id: session.cluster_id.clone(),
                        session_id: session.session_id.clone(),
                        kind,
                        read_id,
                        namespace: if namespaced_kind(kind) {
                            namespace.clone()
                        } else {
                            None
                        },
                        continue_token: None,
                        query: None,
                        status: None,
                        group: None,
                    },
                )
            })
            .collect();

        self.cluster_loading = true;
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let mut snap = OverviewSnapshot::default();
            for (kind, req) in requests {
                if let Ok(page) = connection.resources(req).await {
                    match kind {
                        ResourceKind::Nodes => snap.nodes = Some(page),
                        ResourceKind::Namespaces => snap.namespaces = Some(page),
                        ResourceKind::Pods => snap.pods = Some(page),
                        ResourceKind::Deployments => snap.deployments = Some(page),
                        ResourceKind::DaemonSets => snap.daemonsets = Some(page),
                        ResourceKind::StatefulSets => snap.statefulsets = Some(page),
                        ResourceKind::ReplicaSets => snap.replicasets = Some(page),
                        ResourceKind::Jobs => snap.jobs = Some(page),
                        ResourceKind::CronJobs => snap.cronjobs = Some(page),
                        ResourceKind::Events => snap.events = Some(page),
                        _ => {}
                    }
                }
            }
            let _ = this.update_in(cx, |this, _, cx| {
                this.cluster_loading = false;
                this.overview_snapshot = snap;
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn load_resource_table(
        &mut self,
        kind: ResourceKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        if session.status != "connected" {
            return;
        }
        let read_id = self.next_operation_id();
        let query_str = {
            let q = self
                .resource_filter_input
                .read(cx)
                .value()
                .trim()
                .to_string();
            (!q.is_empty()).then_some(q)
        };
        let req = ResourceRequest {
            cluster_id: session.cluster_id,
            session_id: session.session_id,
            kind,
            read_id,
            namespace: if namespaced_kind(kind) {
                self.selected_namespace.clone()
            } else {
                None
            },
            continue_token: self.current_continue_token.clone(),
            query: query_str,
            status: self.selected_status_filter.clone(),
            group: None,
        };
        let connection = self.connection.clone();
        self.cluster_loading = true;
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let result = connection.resources(req).await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.cluster_loading = false;
                match result {
                    Ok(page) => {
                        this.resource_page = Some(page);
                        this.cluster_error = None;
                    }
                    Err(err) => {
                        this.cluster_error = Some(format!("Resource load failed: {err:?}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn start_pod_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        if session.status != "connected" {
            return;
        }
        let read_id = self.next_operation_id();
        let connection = self.connection.clone();
        let req = ResourceRequest {
            cluster_id: session.cluster_id.clone(),
            session_id: session.session_id.clone(),
            kind: ResourceKind::Pods,
            read_id,
            namespace: self.selected_namespace.clone(),
            continue_token: None,
            query: None,
            status: None,
            group: None,
        };

        cx.spawn_in(window, async move |this, cx| {
            let opened = match connection.pod_sync_open(req).await {
                Ok(snap) => snap,
                Err(_) => return,
            };
            let sync_id = opened.sync_id.clone();
            let mut last_seq = opened.sequence;
            let initial_page = opened.page;

            let _ = this.update_in(cx, |this, _, cx| {
                this.active_pod_sync_id = Some(sync_id.clone());
                if let Some(page) = initial_page {
                    this.resource_page = Some(page);
                }
                cx.notify();
            });

            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let snap = match connection.pod_sync_read(PodSyncRequest {
                    cluster_id: session.cluster_id.clone(),
                    session_id: session.session_id.clone(),
                    sync_id: sync_id.clone(),
                }) {
                    Ok(s) => s,
                    Err(_) => break,
                };

                let is_stopped = matches!(snap.status, "stopped" | "error");
                if snap.sequence <= last_seq && !is_stopped {
                    continue;
                }
                let mut still_active = false;
                let _ = this.update_in(cx, |this, _, cx| {
                    if this.active_pod_sync_id.as_deref() == Some(sync_id.as_str()) {
                        still_active = true;
                        if snap.sequence > last_seq {
                            last_seq = snap.sequence;
                            if let Some(page) = snap.page {
                                this.resource_page = Some(page);
                                cx.notify();
                            }
                        }
                    }
                });
                if !still_active || is_stopped {
                    break;
                }
            }
        })
        .detach();
    }

    fn load_helm_section(&mut self, view: HelmView, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let read_id = self.next_operation_id();
        let req = HelmRequest {
            cluster_id: session.cluster_id,
            session_id: session.session_id,
            view,
            namespace: if view == HelmView::Releases {
                self.selected_namespace.clone()
            } else {
                None
            },
            read_id,
        };
        let connection = self.connection.clone();
        self.cluster_loading = true;
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let result = connection.helm_table(req).await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.cluster_loading = false;
                match result {
                    Ok(page) => {
                        this.helm_page = Some(page);
                        this.cluster_error = None;
                    }
                    Err(err) => {
                        this.cluster_error = Some(format!("Helm query error: {err:?}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_custom_resources_section(
        &mut self,
        group: Option<String>,
        crd_name: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let read_id = self.next_operation_id();
        let def_req = ResourceRequest {
            cluster_id: session.cluster_id.clone(),
            session_id: session.session_id.clone(),
            kind: ResourceKind::CustomResourceDefinitions,
            read_id,
            namespace: None,
            continue_token: None,
            query: None,
            status: None,
            group,
        };
        let connection = self.connection.clone();
        self.cluster_loading = true;
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let defs = connection.resources(def_req).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.cluster_loading = false;
                if let Ok(page) = defs {
                    let selected_crd = if let Some(ref target_name) = crd_name {
                        page.items
                            .iter()
                            .find(|item| &item.name == target_name)
                            .or_else(|| page.items.first())
                            .and_then(|item| item.uid.clone().map(|uid| (item.name.clone(), uid)))
                    } else {
                        page.items
                            .first()
                            .and_then(|item| item.uid.clone().map(|uid| (item.name.clone(), uid)))
                    };
                    this.custom_definitions_page = Some(page);
                    if let Some((def_name, def_uid)) = selected_crd {
                        this.load_crd_instances_only(def_name, def_uid, window, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_crd_instances_only(
        &mut self,
        definition_name: String,
        definition_uid: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let req = CustomResourceRequest {
            cluster_id: session.cluster_id,
            session_id: session.session_id,
            read_id: self.next_operation_id(),
            target: CustomResourceTarget {
                definition_name,
                definition_uid,
            },
            namespace: self.selected_namespace.clone(),
            continue_token: None,
        };
        let connection = self.connection.clone();
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(page) = connection.custom_resources(req).await {
                let _ = this.update_in(cx, |this, _, cx| {
                    this.custom_instances_page = Some(page);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn load_rbac_users_section(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let ns = self
            .selected_namespace
            .clone()
            .unwrap_or_else(|| "default".into());
        let users_req = RbacUsersRequest {
            cluster_id: session.cluster_id.clone(),
            session_id: session.session_id.clone(),
            read_id: self.next_operation_id(),
            namespace: self.selected_namespace.clone(),
        };
        let mgmt_req = ManagementRequest {
            cluster_id: session.cluster_id,
            session_id: session.session_id,
            read_id: self.next_operation_id(),
            namespace: ns,
        };
        let connection = self.connection.clone();
        self.cluster_loading = true;
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let users_res = connection.rbac_users(users_req).await;
            let mgmt_res = connection.rbac_management_snapshot(mgmt_req).await;
            let _ = this.update_in(cx, |this, _, cx| {
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

    pub(crate) fn open_create_resource_modal(
        &mut self,
        kind: ResourceKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.creating_resource_kind = Some(kind);
        self.create_modal_message = None;
        let ns = self.selected_namespace.as_deref().unwrap_or("default");
        let template = default_yaml_template(kind, ns);
        self.create_yaml_editor.update(cx, |editor, cx| {
            editor.set_value(template, window, cx);
        });
        cx.notify();
    }

    fn submit_create_resource(
        &mut self,
        dry_run: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(kind) = self.creating_resource_kind else {
            return;
        };
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let yaml = self.create_yaml_editor.read(cx).value().to_string();
        let req = CreateRequest {
            cluster_id: session.cluster_id,
            session_id: session.session_id,
            read_id: self.next_operation_id(),
            kind,
            namespace: if namespaced_kind(kind) {
                Some(
                    self.selected_namespace
                        .clone()
                        .unwrap_or_else(|| "default".into()),
                )
            } else {
                None
            },
            yaml,
            dry_run,
        };
        let connection = self.connection.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result: std::result::Result<CreateResult, _> = connection.create(req).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(res) => {
                        if !res.created {
                            this.create_modal_message =
                                Some(format!("Dry-run valid for {}", res.name));
                        } else {
                            this.creating_resource_kind = None;
                            this.create_modal_message = None;
                            this.reload_active_section(window, cx);
                        }
                    }
                    Err(err) => {
                        this.create_modal_message = Some(format!("Create failed: {err:?}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn submit_helm_install(&mut self, dry_run: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(chart), Some(version)) = (
            self.helm_install_chart.clone(),
            self.helm_install_version.clone(),
        ) else {
            return;
        };
        let Some(session) = self.cluster_session.clone() else {
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
        let req = HelmInstallRequest {
            cluster_id: session.cluster_id,
            session_id: session.session_id,
            read_id: self.next_operation_id(),
            chart,
            version: (!version.is_empty() && version != "latest").then_some(version),
            release_name,
            namespace,
            dry_run,
            values_yaml,
        };
        let connection = self.connection.clone();
        cx.spawn_in(window, async move |this, cx| {
            let res: std::result::Result<HelmInstallResult, _> = connection.helm_install(req).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match res {
                    Ok(installed) => {
                        this.helm_install_message = Some(format!(
                            "{} release {} ({})",
                            if !installed.created {
                                "Dry-run validated"
                            } else {
                                "Installed"
                            },
                            installed.release_name,
                            installed.chart
                        ));
                        if installed.created {
                            this.helm_install_chart = None;
                            this.load_helm_section(HelmView::Releases, window, cx);
                        }
                    }
                    Err(err) => {
                        this.helm_install_message = Some(format!("Helm install error: {err:?}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn create_rbac_user_plan(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

        let templates = self
            .rbac_management_snapshot
            .as_ref()
            .map(|m| m.templates.clone())
            .unwrap_or_default();

        let assignments: Vec<Assignment> = self
            .rbac_selected_templates
            .iter()
            .filter_map(|tname| {
                let tpl = templates
                    .iter()
                    .find(|t| &t.name == tname && t.scope == "namespaced")?;
                Some(Assignment {
                    template_name: tpl.name.clone(),
                    template_uid: tpl.uid.clone(),
                    template_resource_version: tpl.resource_version.clone(),
                    namespaces: if self.rbac_all_namespaces {
                        vec![]
                    } else {
                        vec![namespace.clone()]
                    },
                    all_namespaces: self.rbac_all_namespaces,
                })
            })
            .collect();

        let cluster_template = if self.rbac_selected_cluster_access != ClusterAccess::None {
            templates
                .iter()
                .find(|t| t.scope == "cluster")
                .map(|t| TemplateIdentity {
                    name: t.name.clone(),
                    uid: t.uid.clone(),
                    resource_version: t.resource_version.clone(),
                })
        } else {
            None
        };
        let cluster_access = if cluster_template.is_some() {
            self.rbac_selected_cluster_access.clone()
        } else {
            ClusterAccess::None
        };

        let req = PlanRequest {
            cluster_id: session.cluster_id,
            session_id: session.session_id,
            read_id: self.next_operation_id(),
            operation: PlanOperation::Create,
            user: ManagedUser {
                name,
                namespace,
                uid: None,
                resource_version: None,
                assignments,
                cluster_access,
                cluster_template,
            },
        };
        self.submit_rbac_plan_request(req, window, cx);
    }

    fn plan_existing_rbac_user(
        &mut self,
        user: ManagedUser,
        operation: PlanOperation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let req = PlanRequest {
            cluster_id: session.cluster_id,
            session_id: session.session_id,
            read_id: self.next_operation_id(),
            operation,
            user,
        };
        self.submit_rbac_plan_request(req, window, cx);
    }

    fn submit_rbac_plan_request(
        &mut self,
        req: PlanRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let connection = self.connection.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = connection.rbac_user_plan(req).await;
            let _ = this.update_in(cx, |this, _, cx| {
                match result {
                    Ok(plan) => {
                        this.rbac_apply_response = None;
                        this.rbac_active_plan = Some(plan);
                        this.cluster_error = None;
                    }
                    Err(err) => {
                        this.cluster_error = Some(format!("RBAC plan failed: {err:?}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_rbac_user_plan(
        &mut self,
        plan_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let req = ApplyRequest {
            cluster_id: session.cluster_id,
            session_id: session.session_id,
            read_id: self.next_operation_id(),
            plan_id,
        };
        let connection = self.connection.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = connection.rbac_user_apply(req).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Ok(applied) = result {
                    this.rbac_apply_response = Some(applied);
                }
                this.rbac_active_plan = None;
                this.load_rbac_users_section(window, cx);
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
        let section = self.active_cluster_section.clone();
        let active_kind = section_to_resource_kind(&section);
        let has_inspector = self.inspector.target.is_some();
        let has_pod_dock = self.pod_dock_open
            && (!self.pod_dock_tabs.is_empty() || !self.port_forwards.is_empty());

        v_flex()
            .size_full()
            .overflow_hidden()
            .child(self.render_cluster_toolbar(&section, active_kind, window, cx))
            .when_some(self.cluster_error.clone(), |el, err| {
                el.child(
                    div()
                        .w_full()
                        .px_4()
                        .py_2()
                        .bg(cx.theme().secondary)
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(err),
                )
            })
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_hidden()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .overflow_hidden()
                            .child(v_flex().flex_1().min_h_0().w_full().child(
                                self.render_cluster_section_body(&section, active_kind, window, cx),
                            ))
                            .when(has_pod_dock, |el| {
                                el.child(self.render_pod_dock_panel(window, cx))
                            }),
                    )
                    .when(has_inspector, |el| {
                        el.child(self.render_inspector_drawer(window, cx))
                    }),
            )
    }

    fn render_cluster_toolbar(
        &mut self,
        section: &str,
        active_kind: Option<ResourceKind>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let session = self.cluster_session.clone();
        let namespaces = self.namespaces.clone();
        let selected_ns = self.selected_namespace.clone();
        let show_ns_selector = active_kind.is_none_or(namespaced_kind);
        let dock_count = self.pod_dock_tabs.len() + self.port_forwards.len();
        let namespace_items = std::iter::once(NamespaceChoice {
            name: "All Namespaces".into(),
        })
        .chain(
            namespaces
                .iter()
                .cloned()
                .map(|name| NamespaceChoice { name }),
        )
        .collect::<Vec<_>>();
        if self.namespace_select_items != namespace_items {
            self.namespace_select.update(cx, |state, cx| {
                state.set_items(SearchableVec::new(namespace_items.clone()), window, cx);
            });
            self.namespace_select_items = namespace_items.clone();
        }
        let namespace_value = selected_ns
            .clone()
            .unwrap_or_else(|| "All Namespaces".into());
        if self.namespace_select.read(cx).selected_value() != Some(&namespace_value) {
            self.namespace_select.update(cx, |state, cx| {
                state.set_selected_value(&namespace_value, window, cx);
            });
        }

        v_flex()
            .w_full()
            .px_4()
            .py_2p5()
            .gap_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().table_head)
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .items_center()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(destination_label(section)),
                            )
                            .when_some(session.clone(), |el, s| {
                                el.child(
                                    match s.status {
                                        "connected" => Tag::success().small(),
                                        "connecting" => Tag::warning().small(),
                                        _ => Tag::secondary().small(),
                                    }
                                    .child(format!(
                                        "{} {}",
                                        s.status,
                                        s.api_version.unwrap_or_default()
                                    )),
                                )
                            })
                            .when(self.active_pod_sync_id.is_some(), |el| {
                                el.child(Tag::success().small().child("LIVE WATCH"))
                            }),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .when(active_kind.is_some(), |el| {
                                el.child(
                                    div().w(px(210.0)).child(
                                        Input::new(&self.resource_filter_input)
                                            .small()
                                            .cleanable(true),
                                    ),
                                )
                            })
                            .when(show_ns_selector && !namespaces.is_empty(), |el| {
                                el.child(
                                    div().w(px(190.0)).child(
                                        Select::new(&self.namespace_select)
                                            .id("namespace-select")
                                            .small()
                                            .w_full()
                                            .placeholder("Namespace")
                                            .search_placeholder("Search namespaces…")
                                            .menu_width(px(240.0)),
                                    ),
                                )
                            })
                            .when_some(active_kind, |el, kind| {
                                el.child(
                                    Button::new("open-create-yaml-btn")
                                        .xsmall()
                                        .primary()
                                        .icon(IconName::Plus)
                                        .label("Create YAML")
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.open_create_resource_modal(kind, window, cx);
                                        })),
                                )
                            })
                            .when(dock_count > 0, |el| {
                                el.child(
                                    Button::new("toggle-pod-dock-btn")
                                        .xsmall()
                                        .outline()
                                        .selected(self.pod_dock_open)
                                        .label(format!("Dock ({dock_count})"))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.pod_dock_open = !this.pod_dock_open;
                                            cx.notify();
                                        })),
                                )
                            })
                            .child(
                                Button::new("reload-cluster-section-btn")
                                    .xsmall()
                                    .ghost()
                                    .icon(IconName::Undo2)
                                    .label("Refresh")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.reload_active_section(window, cx);
                                    })),
                            ),
                    ),
            )
    }

    fn render_cluster_section_body(
        &mut self,
        section: &str,
        active_kind: Option<ResourceKind>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(create_kind) = self.creating_resource_kind {
            return self
                .render_create_resource_panel(create_kind, window, cx)
                .into_any_element();
        }
        if section == "overview"
            || section == "workloads-overview"
            || section == "workloads/overview"
        {
            return self.render_overview_body(cx).into_any_element();
        }
        if section == "port-forwarding" {
            return self.render_port_forwarding_body(cx).into_any_element();
        }
        if section == "helm-charts"
            || section == "helm-releases"
            || section == "helm/charts"
            || section == "helm/releases"
        {
            return self.render_helm_body(window, cx).into_any_element();
        }
        if section == "users" || section == "access-control/users" {
            return self.render_rbac_users_body(window, cx).into_any_element();
        }
        if parse_custom_resource_target(section).is_some() {
            return self.render_custom_resources_body(cx).into_any_element();
        }
        if let Some(kind) = active_kind {
            return self.render_resource_table_body(kind, cx).into_any_element();
        }
        div()
            .p_6()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child("Select a resource section from the sidebar.")
            .into_any_element()
    }

    fn render_create_resource_panel(
        &self,
        kind: ResourceKind,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .w_full()
            .h(px(520.0))
            .p_4()
            .gap_3()
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("Create {:?} from YAML", kind)),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("create-dry-run-btn")
                                    .xsmall()
                                    .outline()
                                    .label("Dry-Run")
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
                                    .label("Cancel")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.creating_resource_kind = None;
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .when_some(self.create_modal_message.clone(), |el, msg| {
                el.child(
                    div()
                        .px_3()
                        .py_1p5()
                        .rounded(px(6.0))
                        .bg(cx.theme().secondary)
                        .text_xs()
                        .child(msg),
                )
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
            )
    }

    fn render_overview_body(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let snap = &self.overview_snapshot;
        let current_cid = self
            .cluster_session
            .as_ref()
            .map(|s| s.cluster_id.clone())
            .unwrap_or_default();

        let count_breakdown = |page: &Option<ResourcePage>| -> (usize, usize, usize) {
            let Some(p) = page.as_ref() else {
                return (0, 0, 0);
            };
            let total = p.items.len();
            let mut healthy = 0;
            let mut warning = 0;
            for item in &p.items {
                let s = item.status.to_lowercase();
                if s.contains("running")
                    || s.contains("ready")
                    || s.contains("active")
                    || s.contains("complete")
                    || s.contains("succeeded")
                    || s == "true"
                {
                    healthy += 1;
                } else if s.contains("fail")
                    || s.contains("error")
                    || s.contains("pending")
                    || s.contains("warning")
                    || s == "false"
                {
                    warning += 1;
                }
            }
            (total, healthy, warning)
        };

        let cards = [
            ("Nodes", "nodes", count_breakdown(&snap.nodes)),
            (
                "Namespaces",
                "namespaces",
                count_breakdown(&snap.namespaces),
            ),
            ("Pods", "pods", count_breakdown(&snap.pods)),
            (
                "Deployments",
                "deployments",
                count_breakdown(&snap.deployments),
            ),
            (
                "DaemonSets",
                "daemon-sets",
                count_breakdown(&snap.daemonsets),
            ),
            (
                "StatefulSets",
                "stateful-sets",
                count_breakdown(&snap.statefulsets),
            ),
            (
                "ReplicaSets",
                "replica-sets",
                count_breakdown(&snap.replicasets),
            ),
            ("Jobs", "jobs", count_breakdown(&snap.jobs)),
            ("CronJobs", "cron-jobs", count_breakdown(&snap.cronjobs)),
        ];

        let events = snap
            .events
            .as_ref()
            .map(|p| p.items.clone())
            .unwrap_or_default();

        v_flex()
            .flex_1()
            .w_full()
            .h_full()
            .min_h_0()
            .p_4()
            .gap_4()
            .overflow_y_scrollbar()
            .child(
                h_flex()
                    .w_full()
                    .gap_3()
                    .flex_wrap()
                    .children(cards.into_iter().map(|(label, sec, (total, ok, warn))| {
                        let cid = current_cid.clone();
                        let target_sec = sec.to_owned();
                        v_flex()
                            .w(px(176.0))
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
                                            .text_color(cx.theme().muted_foreground)
                                            .child(label),
                                    )
                                    .child(
                                        Button::new(SharedString::from(format!("ov-nav-{sec}")))
                                            .xsmall()
                                            .ghost()
                                            .label("View")
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                if !cid.is_empty() {
                                                    this.navigate(
                                                        Route::Cluster {
                                                            cluster_id: cid.clone(),
                                                            section: target_sec.clone(),
                                                        },
                                                        true,
                                                        window,
                                                        cx,
                                                    );
                                                }
                                            })),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(FontWeight::BOLD)
                                    .child(total.to_string()),
                            )
                            .child(
                                h_flex()
                                    .gap_1()
                                    .items_center()
                                    .child(Tag::success().small().child(format!("{ok} ok")))
                                    .when(warn > 0, |r| {
                                        r.child(
                                            Tag::warning().small().child(format!("{warn} attn")),
                                        )
                                    }),
                            )
                    })),
            )
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
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("Recent Cluster Events ({})", events.len())),
                    )
                    .children(events.into_iter().take(25).enumerate().map(|(idx, ev)| {
                        let ev_ref = ResourceReference {
                            kind: ResourceKind::Events,
                            name: ev.name.clone(),
                            namespace: ev.namespace.clone(),
                            uid: ev.uid.clone(),
                        };
                        h_flex()
                            .w_full()
                            .py_1()
                            .gap_2()
                            .justify_between()
                            .items_center()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_2()
                                    .items_center()
                                    .child(if ev.status == "Warning" {
                                        Tag::warning().small().child(ev.status.clone())
                                    } else {
                                        Tag::secondary().small().child(ev.status.clone())
                                    })
                                    .child(
                                        div()
                                            .w(px(130.0))
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .truncate()
                                            .child(
                                                ev.columns
                                                    .get("reason")
                                                    .cloned()
                                                    .unwrap_or_default(),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .w(px(160.0))
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .truncate()
                                            .child(
                                                ev.columns
                                                    .get("involvedObject")
                                                    .cloned()
                                                    .unwrap_or_default(),
                                            ),
                                    )
                                    .child(div().flex_1().text_xs().truncate().child(
                                        ev.columns.get("message").cloned().unwrap_or_default(),
                                    )),
                            )
                            .child(
                                Button::new(SharedString::from(format!("ov-ev-insp-{idx}")))
                                    .xsmall()
                                    .ghost()
                                    .label("Inspect")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_resource_inspector(ev_ref.clone(), window, cx);
                                    })),
                            )
                    })),
            )
    }

    fn render_port_forwarding_body(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let forwards = self.port_forwards.clone();
        v_flex()
            .flex_1()
            .w_full()
            .h_full()
            .min_h_0()
            .p_4()
            .gap_2()
            .overflow_y_scrollbar()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("Active Local Port Forwards ({})", forwards.len())),
            )
            .when(forwards.is_empty(), |el| {
                el.child(
                    div()
                        .p_4()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().secondary)
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("No active port-forwards. Start one from a Service row or Pod container port."),
                )
            })
            .children(forwards.into_iter().enumerate().map(|(idx, fwd)| {
                let fid = fwd.id.clone();
                let copy_addr = format!("127.0.0.1:{}", fwd.local_port);
                h_flex()
                    .w_full()
                    .h(px(38.0))
                    .px_3()
                    .justify_between()
                    .items_center()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().secondary)
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(format!("127.0.0.1:{} → {}:{}", fwd.local_port, fwd.name, fwd.remote_port)),
                            )
                            .child(Tag::secondary().small().child(fwd.namespace))
                            .child(row_status_tag(fwd.status)),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(
                                Button::new(SharedString::from(format!("pf-copy-{idx}")))
                                    .xsmall()
                                    .outline()
                                    .label("Copy")
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            copy_addr.clone(),
                                        ));
                                    })),
                            )
                            .child(
                                Button::new(SharedString::from(format!("pf-stop-{idx}")))
                                    .xsmall()
                                    .danger()
                                    .label("Stop")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.stop_pod_forward(fid.clone(), cx);
                                    })),
                            ),
                    )
            }))
    }

    fn render_resource_table_body(
        &mut self,
        kind: ResourceKind,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let items: Vec<ResourceItem> = self
            .resource_page
            .as_ref()
            .filter(|p| p.kind == kind)
            .map(|p| p.items.clone())
            .unwrap_or_default();
        let query = self
            .resource_filter_input
            .read(cx)
            .value()
            .trim()
            .to_lowercase();
        let items: Vec<ResourceItem> = if query.is_empty() {
            items
        } else {
            items
                .into_iter()
                .filter(|item| {
                    item.name.to_lowercase().contains(&query)
                        || item
                            .namespace
                            .as_deref()
                            .unwrap_or_default()
                            .to_lowercase()
                            .contains(&query)
                        || item
                            .columns
                            .values()
                            .any(|value| value.to_lowercase().contains(&query))
                })
                .collect()
        };
        let next_token = self
            .resource_page
            .as_ref()
            .filter(|p| p.kind == kind)
            .and_then(|p| p.continue_token.clone());
        let has_prev = !self.continue_token_history.is_empty();
        let page_num = self.continue_token_history.len() + 1;

        let cols = columns_for_kind(kind);
        let show_ns = namespaced_kind(kind);
        let status_options: &'static [&'static str] = resource_view_by_kind(kind)
            .map(|v| v.statuses)
            .unwrap_or(&[]);
        let active_status = self.selected_status_filter.clone();
        let items: Vec<ResourceItem> = if let Some(ref st) = active_status {
            items
                .into_iter()
                .filter(|item| item.status.eq_ignore_ascii_case(st))
                .collect()
        } else {
            items
        };
        let item_count = items.len();
        let shared_items = Rc::new(items);
        let item_sizes = Rc::new(vec![size(px(42.0), px(42.0)); item_count]);
        let entity = cx.entity();

        v_flex()
            .flex_1()
            .h_full()
            .min_h_0()
            .w_full()
            .p_4()
            .gap_2()
            .overflow_hidden()
            // Table Pagination, Status Filter Pills & Count Header
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .child(
                        h_flex()
                            .gap_1p5()
                            .items_center()
                            .flex_wrap()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(
                                        "Showing {} {:?} items (Page {page_num})",
                                        item_count, kind
                                    )),
                            )
                            .when(!status_options.is_empty(), |row| {
                                row.child(
                                    Button::new("tbl-status-all")
                                        .xsmall()
                                        .ghost()
                                        .selected(active_status.is_none())
                                        .label("All")
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.selected_status_filter = None;
                                            this.current_continue_token = None;
                                            this.continue_token_history.clear();
                                            this.load_resource_table(kind, window, cx);
                                        })),
                                )
                                .children(
                                    status_options.iter().copied().map(|st| {
                                        let is_sel = active_status.as_deref() == Some(st);
                                        let st_str = st.to_owned();
                                        Button::new(SharedString::from(format!("tbl-st-{st}")))
                                            .xsmall()
                                            .ghost()
                                            .selected(is_sel)
                                            .label(st)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.selected_status_filter = if is_sel {
                                                    None
                                                } else {
                                                    Some(st_str.clone())
                                                };
                                                this.current_continue_token = None;
                                                this.continue_token_history.clear();
                                                this.load_resource_table(kind, window, cx);
                                            }))
                                    }),
                                )
                            }),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .when(has_prev, |row| {
                                row.child(
                                    Button::new("tbl-first-page-btn")
                                        .xsmall()
                                        .outline()
                                        .label("First Page")
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.continue_token_history.clear();
                                            this.current_continue_token = None;
                                            this.load_resource_table(kind, window, cx);
                                        })),
                                )
                                .child(
                                    Button::new("tbl-prev-page-btn")
                                        .xsmall()
                                        .outline()
                                        .label("Previous Page")
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.current_continue_token =
                                                this.continue_token_history.pop().flatten();
                                            this.load_resource_table(kind, window, cx);
                                        })),
                                )
                            })
                            .when_some(next_token, |row, token| {
                                row.child(
                                    Button::new("tbl-next-page-btn")
                                        .xsmall()
                                        .primary()
                                        .label("Next Page")
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            let prev = this.current_continue_token.take();
                                            this.continue_token_history.push(prev);
                                            this.current_continue_token = Some(token.clone());
                                            this.load_resource_table(kind, window, cx);
                                        })),
                                )
                            }),
                    ),
            )
            // Table Column Header
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .rounded(px(6.0))
                    .bg(cx.theme().table_head)
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .w(px(220.0))
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Name"),
                    )
                    .when(show_ns, |el| {
                        el.child(
                            div()
                                .w(px(120.0))
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Namespace"),
                        )
                    })
                    .children(cols.iter().map(|c| {
                        div()
                            .w(px(c.width))
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(c.header)
                    }))
                    .child(
                        div()
                            .w(px(76.0))
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Age"),
                    )
                    .child(
                        div()
                            .w(px(124.0))
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Status"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Actions"),
                    ),
            )
            // Virtualized Table Rows (strictly bounded viewport height; only builds visible rows)
            .child(
                v_virtual_list(
                    entity,
                    SharedString::from(format!("virtual-resource-table-{kind:?}")),
                    item_sizes,
                    move |this, visible_range, _window, cx| {
                        if std::env::var_os("ICEON_TRACE_RESOURCE_LAYOUT").is_some() {
                            eprintln!(
                                "resource-layout {kind:?}: count={} range={visible_range:?}",
                                shared_items.len()
                            );
                        }
                        visible_range
                            .filter_map(|idx| {
                                let item = shared_items.get(idx)?.clone();
                                Some(
                                    div()
                                        .w_full()
                                        .h(px(42.0))
                                        .pb(px(4.0))
                                        .child(this.render_resource_row(
                                            kind, item, idx, show_ns, cols, cx,
                                        ))
                                        .into_any_element(),
                                )
                            })
                            .collect()
                    },
                )
                .flex_1()
                .min_h_0()
                .w_full(),
            )
    }

    fn render_resource_row(
        &mut self,
        kind: ResourceKind,
        item: ResourceItem,
        idx: usize,
        show_ns: bool,
        cols: &'static [ColumnDef],
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let reference = ResourceReference {
            kind,
            name: item.name.clone(),
            namespace: item.namespace.clone(),
            uid: item.uid.clone(),
        };
        let row_actions = self
            .render_row_actions(kind, &item, idx, cx)
            .into_any_element();
        let age_str = format_age(item.created_at.as_deref());

        h_flex()
            .w_full()
            .h(px(38.0))
            .px_3()
            .gap_2()
            .items_center()
            .rounded(px(6.0))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().secondary)
            .hover(|s| s.bg(cx.theme().table_hover))
            .child(
                div()
                    .w(px(220.0))
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .truncate()
                    .child(item.name.clone()),
            )
            .when(show_ns, |el| {
                el.child(
                    div()
                        .w(px(120.0))
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .truncate()
                        .child(item.namespace.clone().unwrap_or_default()),
                )
            })
            .children(cols.iter().map(|c| {
                if kind == ResourceKind::Namespaces && c.key == "labels" {
                    let extra_count = item
                        .columns
                        .get("labels")
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(0)
                        .saturating_sub(1);
                    h_flex()
                        .w(px(c.width))
                        .gap_1()
                        .items_center()
                        .overflow_hidden()
                        .child(
                            Tag::secondary()
                                .small()
                                .child(format!("kubernetes.io/metadata.name={}", item.name)),
                        )
                        .when(extra_count > 0, |row| {
                            row.child(
                                Tag::secondary()
                                    .outline()
                                    .small()
                                    .child(format!("+{extra_count}")),
                            )
                        })
                        .into_any_element()
                } else {
                    div()
                        .w(px(c.width))
                        .text_xs()
                        .truncate()
                        .child(cell_value(&item, c.key))
                        .into_any_element()
                }
            }))
            .child(
                div()
                    .w(px(76.0))
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .truncate()
                    .child(age_str),
            )
            .child(
                h_flex()
                    .w(px(124.0))
                    .items_center()
                    .child(row_status_tag(&item.status)),
            )
            .child(
                h_flex()
                    .flex_1()
                    .gap_1()
                    .items_center()
                    .child(row_actions)
                    .child(
                        Button::new(SharedString::from(format!("inspect-row-{idx}")))
                            .xsmall()
                            .outline()
                            .label("Inspect")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_resource_inspector(reference.clone(), window, cx);
                            })),
                    ),
            )
    }

    fn render_row_actions(
        &self,
        kind: ResourceKind,
        item: &ResourceItem,
        idx: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_pod = kind == ResourceKind::Pods;
        let can_restart = matches!(
            kind,
            ResourceKind::Deployments | ResourceKind::DaemonSets | ResourceKind::StatefulSets
        );
        let is_cronjob = kind == ResourceKind::CronJobs;
        let is_service = kind == ResourceKind::Services;
        let is_node = kind == ResourceKind::Nodes;
        let is_crd = kind == ResourceKind::CustomResourceDefinitions;

        let ns = item.namespace.clone().unwrap_or_default();
        let name = item.name.clone();
        let uid = item.uid.clone();
        let rv = item.resource_version.clone();

        let first_svc_port: Option<u16> = is_service
            .then(|| {
                item.columns
                    .get("ports")
                    .and_then(|s| s.split(',').next())
                    .and_then(|p| p.trim().parse::<u16>().ok())
            })
            .flatten();

        let node_schedulable = item
            .node
            .as_ref()
            .map(|n| n.schedulable)
            .unwrap_or(item.columns.get("schedulable").map(String::as_str) != Some("false"));

        let current_cid = self
            .cluster_session
            .as_ref()
            .map(|s| s.cluster_id.clone())
            .unwrap_or_default();

        let ns_logs = ns.clone();
        let name_logs = name.clone();
        let ns_shell = ns;
        let name_shell = name.clone();
        let del_target = uid.clone().zip(rv.clone()).map(|(u, r)| DeleteTarget {
            kind,
            name: name.clone(),
            namespace: item.namespace.clone(),
            uid: u,
            resource_version: r,
        });

        h_flex()
            .gap_1()
            .items_center()
            // Pod row quick actions: Logs & Shell
            .when(is_pod, |row| {
                row.child(
                    Button::new(SharedString::from(format!("row-pod-logs-{idx}")))
                        .xsmall()
                        .primary()
                        .label("Logs")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_pod_stream_for_pod(
                                ns_logs.clone(),
                                name_logs.clone(),
                                PodStreamMode::Logs,
                                window,
                                cx,
                            );
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("row-pod-shell-{idx}")))
                        .xsmall()
                        .outline()
                        .label("Shell")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_pod_stream_for_pod(
                                ns_shell.clone(),
                                name_shell.clone(),
                                PodStreamMode::Shell,
                                window,
                                cx,
                            );
                        })),
                )
            })
            // Workload row quick action: Restart
            .when_some(
                can_restart.then(|| uid.clone().zip(rv.clone())).flatten(),
                |row, (u, r)| {
                    let wref = ResourceReference {
                        kind,
                        name: name.clone(),
                        namespace: item.namespace.clone(),
                        uid: Some(u),
                    };
                    row.child(
                        Button::new(SharedString::from(format!("row-wl-restart-{idx}")))
                            .xsmall()
                            .ghost()
                            .label("Restart")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.run_workload_action_for(
                                    wref.clone(),
                                    r.clone(),
                                    WorkloadAction::Restart,
                                    None,
                                    window,
                                    cx,
                                );
                            })),
                    )
                },
            )
            // CronJob row quick action: Trigger
            .when_some(
                is_cronjob.then(|| uid.clone().zip(rv.clone())).flatten(),
                |row, (u, r)| {
                    let cref = ResourceReference {
                        kind,
                        name: name.clone(),
                        namespace: item.namespace.clone(),
                        uid: Some(u),
                    };
                    row.child(
                        Button::new(SharedString::from(format!("row-cj-trigger-{idx}")))
                            .xsmall()
                            .primary()
                            .label("Trigger")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.run_workload_action_for(
                                    cref.clone(),
                                    r.clone(),
                                    WorkloadAction::Trigger,
                                    None,
                                    window,
                                    cx,
                                );
                            })),
                    )
                },
            )
            // Service row quick action: Port-Forward first port
            .when_some(
                first_svc_port.zip(uid.clone().zip(rv.clone())),
                |row, (port, (u, r))| {
                    let sref = ResourceReference {
                        kind: ResourceKind::Services,
                        name: name.clone(),
                        namespace: item.namespace.clone(),
                        uid: Some(u),
                    };
                    row.child(
                        Button::new(SharedString::from(format!("row-svc-fwd-{idx}")))
                            .xsmall()
                            .primary()
                            .label(format!("Forward :{port}"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.start_service_forward_for(
                                    sref.clone(),
                                    r.clone(),
                                    port,
                                    window,
                                    cx,
                                );
                            })),
                    )
                },
            )
            // Node row quick actions: Cordon/Uncordon + Drain
            .when_some(
                is_node.then(|| uid.clone().zip(rv.clone())).flatten(),
                |row, (u, r)| {
                    let nref = ResourceReference {
                        kind: ResourceKind::Nodes,
                        name: name.clone(),
                        namespace: None,
                        uid: Some(u),
                    };
                    let nref_drain = nref.clone();
                    let r_drain = r.clone();
                    let toggle_action = if node_schedulable {
                        NodeAction::Cordon
                    } else {
                        NodeAction::Uncordon
                    };
                    let toggle_label = if node_schedulable {
                        "Cordon"
                    } else {
                        "Uncordon"
                    };
                    row.child(
                        Button::new(SharedString::from(format!("row-node-cordon-{idx}")))
                            .xsmall()
                            .ghost()
                            .label(toggle_label)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.run_node_action_for(
                                    nref.clone(),
                                    r.clone(),
                                    toggle_action,
                                    window,
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("row-node-drain-{idx}")))
                            .xsmall()
                            .danger()
                            .label("Drain")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.review_node_drain_for(
                                    nref_drain.clone(),
                                    r_drain.clone(),
                                    window,
                                    cx,
                                );
                            })),
                    )
                },
            )
            // CRD row quick action: View Instances
            .when(is_crd, |row| {
                let target_sec = format!("crd:{}", item.name);
                let cid = current_cid;
                row.child(
                    Button::new(SharedString::from(format!("row-crd-inst-{idx}")))
                        .xsmall()
                        .primary()
                        .label("Instances")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if !cid.is_empty() {
                                this.navigate(
                                    Route::Cluster {
                                        cluster_id: cid.clone(),
                                        section: target_sec.clone(),
                                    },
                                    true,
                                    window,
                                    cx,
                                );
                            }
                        })),
                )
            })
            // Delete row button (for resources with UID + resourceVersion)
            .when_some(
                del_target.filter(|_| kind != ResourceKind::Nodes),
                |row, target| {
                    row.child(
                        Button::new(SharedString::from(format!("row-del-{idx}")))
                            .xsmall()
                            .ghost()
                            .icon(IconName::Delete)
                            .tooltip("Delete resource")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.delete_resource_by_target(target.clone(), window, cx);
                            })),
                    )
                },
            )
    }

    fn render_helm_body(&self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let items = self
            .helm_page
            .as_ref()
            .map(|p| p.items.clone())
            .unwrap_or_default();
        let view = self
            .helm_page
            .as_ref()
            .map(|p| p.view)
            .unwrap_or(HelmView::Charts);

        v_flex()
            .flex_1()
            .w_full()
            .h_full()
            .min_h_0()
            .p_4()
            .gap_3()
            .overflow_y_scrollbar()
            .when_some(self.helm_install_chart.clone(), |el, chart| {
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
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(format!("Install Helm Chart · {chart}")),
                                )
                                .child(
                                    h_flex()
                                        .gap_2()
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
                                            Button::new("helm-install-submit-btn")
                                                .xsmall()
                                                .primary()
                                                .label("Install Release")
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.submit_helm_install(false, window, cx);
                                                })),
                                        )
                                        .child(
                                            Button::new("helm-install-close-btn")
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
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    div()
                                        .w(px(220.0))
                                        .child(Input::new(&self.helm_release_name_input).small()),
                                )
                                .child(
                                    div()
                                        .w(px(180.0))
                                        .child(Input::new(&self.helm_namespace_input).small()),
                                ),
                        )
                        .when_some(self.helm_install_message.clone(), |el, msg| {
                            el.child(div().text_xs().child(msg))
                        })
                        .child(
                            div()
                                .h(px(180.0))
                                .w_full()
                                .border_1()
                                .border_color(cx.theme().border)
                                .rounded(px(6.0))
                                .overflow_hidden()
                                .child(Editor::new(&self.helm_values_editor).h_full()),
                        ),
                )
            })
            .children(items.into_iter().enumerate().map(|(idx, item)| {
                let name = item.name.clone();
                let ns = item.namespace.clone();
                let ver = item.version.clone();
                let name_install = name.clone();
                let ver_install = ver.clone();

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
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(item.name.clone()),
                                    )
                                    .child(Tag::secondary().small().child(item.version.clone()))
                                    .when_some(item.status.clone(), |el, st| {
                                        el.child(row_status_tag(&st))
                                    }),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(
                                        item.description
                                            .or(item.namespace)
                                            .unwrap_or_else(|| "-".into()),
                                    ),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1p5()
                            .when(view == HelmView::Charts, |el| {
                                el.child(
                                    Button::new(SharedString::from(format!("helm-inst-{idx}")))
                                        .xsmall()
                                        .primary()
                                        .label("Install")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.helm_install_chart = Some(name_install.clone());
                                            this.helm_install_version = Some(ver_install.clone());
                                            this.helm_install_message = None;
                                            cx.notify();
                                        })),
                                )
                            })
                            .child(
                                Button::new(SharedString::from(format!("helm-detail-{idx}")))
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
                            ),
                    )
            }))
    }

    fn render_custom_resources_body(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let defs = self
            .custom_definitions_page
            .as_ref()
            .map(|p| p.items.clone())
            .unwrap_or_default();
        let instances = self.custom_instances_page.clone();

        v_flex()
            .flex_1()
            .w_full()
            .h_full()
            .min_h_0()
            .p_4()
            .gap_4()
            .overflow_y_scrollbar()
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
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("Custom Resource Definitions ({})", defs.len())),
                    )
                    .child(h_flex().gap_1p5().flex_wrap().children(
                        defs.into_iter().enumerate().map(|(idx, def)| {
                            let crd_name = def.name.clone();
                            let crd_uid = def.uid.clone();
                            let inspect_ref = ResourceReference {
                                kind: ResourceKind::CustomResourceDefinitions,
                                name: def.name.clone(),
                                namespace: None,
                                uid: def.uid.clone(),
                            };
                            h_flex()
                                .gap_0p5()
                                .items_center()
                                .child(
                                    Button::new(SharedString::from(format!("crd-select-{idx}")))
                                        .xsmall()
                                        .outline()
                                        .label(def.name)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            if let Some(uid) = crd_uid.clone() {
                                                this.load_crd_instances_only(
                                                    crd_name.clone(),
                                                    uid,
                                                    window,
                                                    cx,
                                                );
                                            }
                                        })),
                                )
                                .child(
                                    Button::new(SharedString::from(format!("crd-inspect-{idx}")))
                                        .xsmall()
                                        .ghost()
                                        .label("Spec")
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.open_resource_inspector(
                                                inspect_ref.clone(),
                                                window,
                                                cx,
                                            );
                                        })),
                                )
                        }),
                    )),
            )
            .when_some(instances, |el, page: CustomResourcePage| {
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
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(format!(
                                    "{} ({}/{}) · {} instances",
                                    page.kind,
                                    page.group,
                                    page.version,
                                    page.items.len()
                                )),
                        )
                        .children(page.items.into_iter().map(|inst| {
                            h_flex()
                                .w_full()
                                .py_1p5()
                                .justify_between()
                                .items_center()
                                .border_b_1()
                                .border_color(cx.theme().border)
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(inst.name),
                                )
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .items_center()
                                        .when_some(inst.namespace, |el, ns| {
                                            el.child(Tag::secondary().small().child(ns))
                                        })
                                        .child(
                                            Tag::secondary()
                                                .small()
                                                .child(format!("{} labels", inst.label_count)),
                                        ),
                                )
                        })),
                )
            })
    }

    fn render_rbac_users_body(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let subjects = self
            .rbac_users_response
            .as_ref()
            .map(|r| r.items.clone())
            .unwrap_or_default();
        let coverage = self
            .rbac_users_response
            .as_ref()
            .map(|r| r.coverage.clone())
            .unwrap_or_default();
        let mgmt = self.rbac_management_snapshot.clone();
        let selected_templates = self.rbac_selected_templates.clone();
        let cluster_access = self.rbac_selected_cluster_access.clone();
        let all_ns = self.rbac_all_namespaces;

        v_flex()
            .flex_1()
            .w_full()
            .h_full()
            .min_h_0()
            .p_4()
            .gap_4()
            .overflow_y_scrollbar()
            // Managed ServiceAccount User Builder & Template Selector
            .child(
                v_flex()
                    .w_full()
                    .p_3()
                    .gap_2p5()
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
                                    .child("Managed ServiceAccount Users & Permission Planner"),
                            )
                            .child(
                                h_flex().gap_1().children(coverage.into_iter().map(|cov| {
                                    let badge = if cov.state == "complete" {
                                        Tag::success().small()
                                    } else {
                                        Tag::warning().small()
                                    };
                                    badge.child(format!("{}: {}", cov.resource, cov.state))
                                })),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .w(px(240.0))
                                    .child(Input::new(&self.rbac_new_user_input).small()),
                            )
                            .child(
                                Button::new("rbac-scope-toggle")
                                    .xsmall()
                                    .ghost()
                                    .selected(all_ns)
                                    .label(if all_ns {
                                        "Scope: All Namespaces"
                                    } else {
                                        "Scope: Current Namespace"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.rbac_all_namespaces = !this.rbac_all_namespaces;
                                        cx.notify();
                                    })),
                            )
                            .children(
                                [
                                    (ClusterAccess::None, "Cluster: None"),
                                    (ClusterAccess::Read, "Cluster: Read"),
                                    (ClusterAccess::Write, "Cluster: Write"),
                                ]
                                .into_iter()
                                .map(|(lvl, label)| {
                                    let active = cluster_access == lvl;
                                    Button::new(SharedString::from(format!(
                                        "rbac-cluster-lvl-{label}"
                                    )))
                                    .xsmall()
                                    .ghost()
                                    .selected(active)
                                    .label(label)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.rbac_selected_cluster_access = lvl.clone();
                                        cx.notify();
                                    }))
                                }),
                            )
                            .child(
                                Button::new("rbac-plan-create-btn")
                                    .xsmall()
                                    .primary()
                                    .label("Preview Create Plan")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.create_rbac_user_plan(window, cx);
                                    })),
                            ),
                    )
                    .when_some(mgmt.clone(), |el, snap| {
                        let ns_templates: Vec<_> = snap
                            .templates
                            .iter()
                            .filter(|t| t.scope == "namespaced")
                            .cloned()
                            .collect();
                        el.when(!ns_templates.is_empty(), |card| {
                            card.child(
                                h_flex()
                                    .gap_1p5()
                                    .flex_wrap()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child("Templates:"),
                                    )
                                    .children(ns_templates.into_iter().map(|tpl| {
                                        let tname = tpl.name.clone();
                                        let selected = selected_templates.contains(&tname);
                                        Button::new(SharedString::from(format!(
                                            "rbac-tpl-toggle-{tname}"
                                        )))
                                        .xsmall()
                                        .outline()
                                        .selected(selected)
                                        .label(format!("{} ({} rules)", tpl.name, tpl.rules.len()))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if let Some(pos) = this
                                                .rbac_selected_templates
                                                .iter()
                                                .position(|x| x == &tname)
                                            {
                                                this.rbac_selected_templates.remove(pos);
                                            } else {
                                                this.rbac_selected_templates.push(tname.clone());
                                            }
                                            cx.notify();
                                        }))
                                    })),
                            )
                        })
                        .when(!snap.users.is_empty(), |card| {
                            card.child(
                                v_flex()
                                    .w_full()
                                    .gap_1p5()
                                    .pt_2()
                                    .border_t_1()
                                    .border_color(cx.theme().border)
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(format!(
                                                "Managed ServiceAccount Users ({})",
                                                snap.users.len()
                                            )),
                                    )
                                    .children(snap.users.into_iter().enumerate().map(
                                        |(uidx, u)| {
                                            let u_del = u.clone();
                                            let sa_ref = ResourceReference {
                                                kind: ResourceKind::ServiceAccounts,
                                                name: u.name.clone(),
                                                namespace: Some(u.namespace.clone()),
                                                uid: u.uid.clone(),
                                            };
                                            h_flex()
                                                .justify_between()
                                                .items_center()
                                                .p_2()
                                                .rounded(px(6.0))
                                                .border_1()
                                                .border_color(cx.theme().border)
                                                .bg(cx.theme().background)
                                                .child(
                                                    h_flex()
                                                        .gap_2()
                                                        .items_center()
                                                        .child(
                                                            div()
                                                                .text_xs()
                                                                .font_weight(FontWeight::SEMIBOLD)
                                                                .child(format!(
                                                                    "{}/{}",
                                                                    u.namespace, u.name
                                                                )),
                                                        )
                                                        .child(
                                                            Tag::secondary().small().child(
                                                                format!(
                                                                    "{} templates · cluster {:?}",
                                                                    u.assignments.len(),
                                                                    u.cluster_access
                                                                ),
                                                            ),
                                                        ),
                                                )
                                                .child(
                                                    h_flex()
                                                        .gap_1()
                                                        .child(
                                                            Button::new(SharedString::from(
                                                                format!("managed-sa-insp-{uidx}"),
                                                            ))
                                                            .xsmall()
                                                            .ghost()
                                                            .label("Inspect SA")
                                                            .on_click(cx.listener(
                                                                move |this, _, window, cx| {
                                                                    this.open_resource_inspector(
                                                                        sa_ref.clone(),
                                                                        window,
                                                                        cx,
                                                                    );
                                                                },
                                                            )),
                                                        )
                                                        .child(
                                                            Button::new(SharedString::from(
                                                                format!("managed-sa-del-{uidx}"),
                                                            ))
                                                            .xsmall()
                                                            .danger()
                                                            .label("Plan Delete")
                                                            .on_click(cx.listener(
                                                                move |this, _, window, cx| {
                                                                    this.plan_existing_rbac_user(
                                                                        u_del.clone(),
                                                                        PlanOperation::Delete,
                                                                        window,
                                                                        cx,
                                                                    );
                                                                },
                                                            )),
                                                        ),
                                                )
                                        },
                                    )),
                            )
                        })
                    })
                    .when_some(self.rbac_active_plan.clone(), |el, plan| {
                        let pid = plan.plan_id.clone();
                        el.child(
                            v_flex()
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
                                            div()
                                                .text_xs()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child(format!(
                                                    "Dry-Run Verified Plan {:?} for {}/{} ({} steps)",
                                                    plan.operation,
                                                    plan.user.namespace,
                                                    plan.user.name,
                                                    plan.steps.len()
                                                )),
                                        )
                                        .child(
                                            h_flex()
                                                .gap_1p5()
                                                .child(
                                                    Button::new("rbac-apply-plan-btn")
                                                        .xsmall()
                                                        .primary()
                                                        .label("Apply Plan")
                                                        .on_click(cx.listener(
                                                            move |this, _, window, cx| {
                                                                this.apply_rbac_user_plan(
                                                                    pid.clone(),
                                                                    window,
                                                                    cx,
                                                                );
                                                            },
                                                        )),
                                                )
                                                .child(
                                                    Button::new("rbac-discard-plan-btn")
                                                        .xsmall()
                                                        .ghost()
                                                        .label("Discard")
                                                        .on_click(cx.listener(
                                                            |this, _, _, cx| {
                                                                this.rbac_active_plan = None;
                                                                cx.notify();
                                                            },
                                                        )),
                                                ),
                                        ),
                                )
                                .children(plan.steps.iter().map(|step| {
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!(
                                            "• {:?} {} {}",
                                            step.operation, step.kind, step.name
                                        ))
                                }))
                                .children(plan.warnings.iter().map(|w| {
                                    div().text_xs().text_color(cx.theme().warning).child(w.clone())
                                })),
                        )
                    })
                    .when_some(self.rbac_apply_response.clone(), |el, applied| {
                        el.child(
                            v_flex()
                                .p_2p5()
                                .gap_1()
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
                                                    "Plan Execution {:?} · Complete: {}",
                                                    applied.operation, applied.complete
                                                )),
                                        )
                                        .child(
                                            Button::new("dismiss-rbac-apply-res")
                                                .xsmall()
                                                .ghost()
                                                .icon(IconName::Close)
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.rbac_apply_response = None;
                                                    cx.notify();
                                                })),
                                        ),
                                )
                                .children(applied.results.iter().map(|r| {
                                    h_flex()
                                        .justify_between()
                                        .items_center()
                                        .child(
                                            div().text_xs().child(format!(
                                                "{:?} {} {}",
                                                r.operation, r.kind, r.name
                                            )),
                                        )
                                        .child(row_status_tag(&r.state))
                                })),
                        )
                    }),
            )
            // Discovered RBAC Subjects & Effective RoleBindings
            .child(
                v_flex()
                    .w_full()
                    .gap_2()
                    .children(subjects.into_iter().map(|subj| {
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
                                        h_flex()
                                            .gap_2()
                                            .items_center()
                                            .child(Tag::secondary().small().child(subj.kind))
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .child(subj.name),
                                            )
                                            .when_some(subj.namespace, |el, ns| {
                                                el.child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(cx.theme().muted_foreground)
                                                        .child(format!("ns: {ns}")),
                                                )
                                            }),
                                    )
                                    .child(
                                        Tag::secondary()
                                            .small()
                                            .child(format!("{} bindings", subj.bindings.len())),
                                    ),
                            )
                            .children(subj.bindings.into_iter().take(8).map(|b| {
                                h_flex()
                                    .justify_between()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!("{} · {}", b.kind, b.name))
                                    .child(format!("→ {}/{}", b.role_ref.kind, b.role_ref.name))
                            }))
                    })),
            )
    }
}

fn row_status_tag(status: &str) -> Tag {
    let normalized = status.to_ascii_lowercase();
    let tag = if normalized.contains("fail")
        || normalized.contains("error")
        || normalized.contains("crash")
    {
        Tag::danger()
    } else if normalized.contains("pending")
        || normalized.contains("wait")
        || normalized.contains("warn")
        || normalized.contains("notready")
        || normalized.contains("not ready")
    {
        Tag::warning()
    } else if normalized.contains("running")
        || normalized.contains("ready")
        || normalized.contains("complete")
        || normalized.contains("succeed")
    {
        Tag::success()
    } else {
        Tag::secondary()
    };
    tag.small().child(status.to_owned())
}

pub fn default_yaml_template(kind: ResourceKind, namespace: &str) -> String {
    let (api_version, kind_name) = iceon_core::connection::create::creation_info(kind);
    if namespaced_kind(kind) {
        format!(
            "apiVersion: {api_version}\nkind: {kind_name}\nmetadata:\n  name: example-{}\n  namespace: {namespace}\n",
            kind_name.to_lowercase()
        )
    } else {
        format!(
            "apiVersion: {api_version}\nkind: {kind_name}\nmetadata:\n  name: example-{}\n",
            kind_name.to_lowercase()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{columns_for_kind, default_yaml_template};
    use iceon_core::connection::ResourceKind;

    #[test]
    fn columns_and_templates_cover_core_resource_kinds() {
        assert!(!columns_for_kind(ResourceKind::Pods).is_empty());
        assert!(!columns_for_kind(ResourceKind::Nodes).is_empty());
        assert!(!columns_for_kind(ResourceKind::Deployments).is_empty());
        assert!(!columns_for_kind(ResourceKind::Services).is_empty());
        assert!(!columns_for_kind(ResourceKind::CustomResourceDefinitions).is_empty());

        let pod_yaml = default_yaml_template(ResourceKind::Pods, "team-a");
        assert!(pod_yaml.contains("kind: Pod"));
        assert!(pod_yaml.contains("namespace: team-a"));

        let node_yaml = default_yaml_template(ResourceKind::Nodes, "team-a");
        assert!(node_yaml.contains("kind: Node"));
        assert!(!node_yaml.contains("namespace:"));
    }
}
