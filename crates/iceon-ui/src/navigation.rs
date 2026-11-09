//! Pure-Rust navigation model, cluster destination hierarchy, resource column definitions,
//! YAML creation scaffolds, age formatters, and sanitized error messages.

use iceon_core::catalog::{CatalogError, ClusterSummary, PinTarget};
use iceon_core::connection::{ConnectionError, ResourceItem, ResourceKind};
use iceon_core::preferences::PreferencesError;
use iceon_core::route_history::{SavedPage, SavedRoute};
use iceon_core::secrets;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Catalog,
    Secrets,
    Cluster {
        cluster_id: String,
        section: String,
    },
}

impl Route {
    pub fn to_saved_route(&self) -> SavedRoute {
        match self {
            Self::Catalog => SavedRoute {
                page: SavedPage::Catalog,
                cluster_id: None,
                section: None,
            },
            Self::Secrets => SavedRoute {
                page: SavedPage::Secrets,
                cluster_id: None,
                section: None,
            },
            Self::Cluster {
                cluster_id,
                section,
            } => SavedRoute {
                page: SavedPage::Cluster,
                cluster_id: Some(cluster_id.clone()),
                section: Some(section.clone()),
            },
        }
    }

    pub fn from_saved_route(saved: SavedRoute) -> Self {
        match saved.page {
            SavedPage::Catalog => Self::Catalog,
            SavedPage::Secrets => Self::Secrets,
            SavedPage::Cluster => match saved.cluster_id {
                Some(cluster_id) => Self::Cluster {
                    cluster_id,
                    section: saved.section.unwrap_or_else(|| "overview".into()),
                },
                None => Self::Catalog,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct NavigationHistory {
    entries: Vec<Route>,
    index: usize,
}

impl Default for NavigationHistory {
    fn default() -> Self {
        Self {
            entries: vec![Route::Catalog],
            index: 0,
        }
    }
}

impl NavigationHistory {
    pub fn new(initial: Route) -> Self {
        Self {
            entries: vec![initial],
            index: 0,
        }
    }

    pub fn current(&self) -> &Route {
        &self.entries[self.index]
    }

    pub fn can_go_back(&self) -> bool {
        self.index > 0
    }

    pub fn can_go_forward(&self) -> bool {
        self.index + 1 < self.entries.len()
    }

    pub fn push(&mut self, route: Route) {
        if self.entries.get(self.index) == Some(&route) {
            return;
        }
        self.entries.truncate(self.index + 1);
        self.entries.push(route);
        if self.entries.len() > 64 {
            self.entries.remove(0);
        }
        self.index = self.entries.len() - 1;
    }

    pub fn go_back(&mut self) -> Option<Route> {
        if self.can_go_back() {
            self.index -= 1;
            Some(self.current().clone())
        } else {
            None
        }
    }

    pub fn go_forward(&mut self) -> Option<Route> {
        if self.can_go_forward() {
            self.index += 1;
            Some(self.current().clone())
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ColumnDef {
    pub key: &'static str,
    pub label: &'static str,
    pub width: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct ResourceView {
    pub id: &'static str,
    pub label: &'static str,
    pub kind: ResourceKind,
    pub namespaced: bool,
    pub columns: &'static [ColumnDef],
    pub statuses: &'static [&'static str],
    pub creatable: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct NavChild {
    pub id: &'static str,
    pub label: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct NavSection {
    pub id: &'static str,
    pub label: &'static str,
    pub default_child: &'static str,
    pub children: &'static [NavChild],
}

pub const CLUSTER_SECTIONS: &[NavSection] = &[
    NavSection {
        id: "overview",
        label: "Overview",
        default_child: "overview",
        children: &[],
    },
    NavSection {
        id: "nodes",
        label: "Nodes",
        default_child: "nodes",
        children: &[],
    },
    NavSection {
        id: "workloads",
        label: "Workloads",
        default_child: "workloads-overview",
        children: &[
            NavChild {
                id: "workloads-overview",
                label: "Overview",
            },
            NavChild {
                id: "pods",
                label: "Pods",
            },
            NavChild {
                id: "deployments",
                label: "Deployments",
            },
            NavChild {
                id: "daemon-sets",
                label: "DaemonSets",
            },
            NavChild {
                id: "stateful-sets",
                label: "StatefulSets",
            },
            NavChild {
                id: "replica-sets",
                label: "ReplicaSets",
            },
            NavChild {
                id: "replication-controllers",
                label: "ReplicationControllers",
            },
            NavChild {
                id: "jobs",
                label: "Jobs",
            },
            NavChild {
                id: "cron-jobs",
                label: "CronJobs",
            },
        ],
    },
    NavSection {
        id: "config",
        label: "Config",
        default_child: "config-maps",
        children: &[
            NavChild {
                id: "config-maps",
                label: "ConfigMaps",
            },
            NavChild {
                id: "secrets",
                label: "Secrets",
            },
            NavChild {
                id: "resource-quotas",
                label: "ResourceQuotas",
            },
            NavChild {
                id: "limit-ranges",
                label: "LimitRanges",
            },
            NavChild {
                id: "horizontal-pod-autoscalers",
                label: "HorizontalPodAutoscalers",
            },
            NavChild {
                id: "vertical-pod-autoscalers",
                label: "VerticalPodAutoscalers",
            },
            NavChild {
                id: "pod-disruption-budgets",
                label: "PodDisruptionBudgets",
            },
            NavChild {
                id: "priority-classes",
                label: "PriorityClasses",
            },
            NavChild {
                id: "runtime-classes",
                label: "RuntimeClasses",
            },
            NavChild {
                id: "leases",
                label: "Leases",
            },
            NavChild {
                id: "mutating-webhook-configs",
                label: "MutatingWebhookConfigs",
            },
            NavChild {
                id: "validating-webhook-configs",
                label: "ValidatingWebhookConfigs",
            },
            NavChild {
                id: "validating-admission-policies",
                label: "ValidatingAdmissionPolicies",
            },
            NavChild {
                id: "validating-admission-policy-bindings",
                label: "ValidatingAdmissionPolicyBindings",
            },
        ],
    },
    NavSection {
        id: "network",
        label: "Network",
        default_child: "services",
        children: &[
            NavChild {
                id: "services",
                label: "Services",
            },
            NavChild {
                id: "endpoint-slices",
                label: "EndpointSlices",
            },
            NavChild {
                id: "endpoints",
                label: "Endpoints",
            },
            NavChild {
                id: "ingresses",
                label: "Ingresses",
            },
            NavChild {
                id: "ingress-classes",
                label: "IngressClasses",
            },
            NavChild {
                id: "network-policies",
                label: "NetworkPolicies",
            },
            NavChild {
                id: "port-forwarding",
                label: "Port Forwarding",
            },
        ],
    },
    NavSection {
        id: "storage",
        label: "Storage",
        default_child: "persistent-volume-claims",
        children: &[
            NavChild {
                id: "persistent-volume-claims",
                label: "PersistentVolumeClaims",
            },
            NavChild {
                id: "persistent-volumes",
                label: "PersistentVolumes",
            },
            NavChild {
                id: "storage-classes",
                label: "StorageClasses",
            },
        ],
    },
    NavSection {
        id: "namespaces",
        label: "Namespaces",
        default_child: "namespaces",
        children: &[],
    },
    NavSection {
        id: "events",
        label: "Events",
        default_child: "events",
        children: &[],
    },
    NavSection {
        id: "helm",
        label: "Helm",
        default_child: "helm-charts",
        children: &[
            NavChild {
                id: "helm-charts",
                label: "Charts",
            },
            NavChild {
                id: "helm-releases",
                label: "Releases",
            },
        ],
    },
    NavSection {
        id: "access-control",
        label: "Access Control",
        default_child: "service-accounts",
        children: &[
            NavChild {
                id: "users",
                label: "Users",
            },
            NavChild {
                id: "service-accounts",
                label: "ServiceAccounts",
            },
            NavChild {
                id: "cluster-roles",
                label: "ClusterRoles",
            },
            NavChild {
                id: "roles",
                label: "Roles",
            },
            NavChild {
                id: "cluster-role-bindings",
                label: "ClusterRoleBindings",
            },
            NavChild {
                id: "role-bindings",
                label: "RoleBindings",
            },
            NavChild {
                id: "pod-security-policies",
                label: "PodSecurityPolicies",
            },
        ],
    },
    NavSection {
        id: "custom-resources",
        label: "Custom Resources",
        default_child: "custom-resource-definitions",
        children: &[NavChild {
            id: "custom-resource-definitions",
            label: "Definitions",
        }],
    },
];

pub fn normalize_destination(section: &str) -> String {
    if let Some(rest) = section.strip_prefix("crd:") {
        if !rest.is_empty() {
            return section.to_string();
        }
    }
    for group in CLUSTER_SECTIONS {
        if group.id == section {
            return group.default_child.to_string();
        }
        for child in group.children {
            if child.id == section {
                return child.id.to_string();
            }
        }
    }
    "overview".to_string()
}

pub fn destination_label(section: &str) -> String {
    if let Some(rest) = section.strip_prefix("crd:") {
        return format!("CRD · {rest}");
    }
    for group in CLUSTER_SECTIONS {
        if group.id == section {
            return group.label.to_string();
        }
        for child in group.children {
            if child.id == section {
                return format!("{} · {}", group.label, child.label);
            }
        }
    }
    "Overview".to_string()
}

pub fn active_group_id(destination: &str) -> &'static str {
    if destination.starts_with("crd:") {
        return "custom-resources";
    }
    for group in CLUSTER_SECTIONS {
        if group.id == destination || group.default_child == destination {
            return group.id;
        }
        if group.children.iter().any(|c| c.id == destination) {
            return group.id;
        }
    }
    "overview"
}

pub const RESOURCE_VIEWS: &[ResourceView] = &[
    ResourceView {
        id: "nodes",
        label: "Nodes",
        kind: ResourceKind::Nodes,
        namespaced: false,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 200.0 },
            ColumnDef { key: "cpu", label: "CPU", width: 110.0 },
            ColumnDef { key: "memory", label: "Memory", width: 120.0 },
            ColumnDef { key: "roles", label: "Roles", width: 130.0 },
            ColumnDef { key: "taints", label: "Taints", width: 80.0 },
            ColumnDef { key: "version", label: "Version", width: 120.0 },
            ColumnDef { key: "age", label: "Age", width: 80.0 },
            ColumnDef { key: "status", label: "Conditions", width: 140.0 },
        ],
        statuses: &["Ready", "NotReady", "SchedulingDisabled"],
        creatable: false,
    },
    ResourceView {
        id: "namespaces",
        label: "Namespaces",
        kind: ResourceKind::Namespaces,
        namespaced: false,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 240.0 },
            ColumnDef { key: "labels", label: "Labels", width: 220.0 },
            ColumnDef { key: "age", label: "Age", width: 90.0 },
            ColumnDef { key: "status", label: "Status", width: 120.0 },
        ],
        statuses: &["Active", "Terminating"],
        creatable: true,
    },
    ResourceView {
        id: "pods",
        label: "Pods",
        kind: ResourceKind::Pods,
        namespaced: true,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 220.0 },
            ColumnDef { key: "namespace", label: "Namespace", width: 130.0 },
            ColumnDef { key: "containers", label: "Ready", width: 90.0 },
            ColumnDef { key: "restarts", label: "Restarts", width: 90.0 },
            ColumnDef { key: "controlledBy", label: "Controlled By", width: 150.0 },
            ColumnDef { key: "node", label: "Node", width: 140.0 },
            ColumnDef { key: "qos", label: "QoS", width: 100.0 },
            ColumnDef { key: "age", label: "Age", width: 80.0 },
            ColumnDef { key: "status", label: "Status", width: 120.0 },
        ],
        statuses: &["Running", "Pending", "Succeeded", "Failed", "Terminating"],
        creatable: true,
    },
    ResourceView {
        id: "deployments",
        label: "Deployments",
        kind: ResourceKind::Deployments,
        namespaced: true,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 220.0 },
            ColumnDef { key: "namespace", label: "Namespace", width: 130.0 },
            ColumnDef { key: "pods", label: "Pods", width: 100.0 },
            ColumnDef { key: "replicas", label: "Replicas", width: 90.0 },
            ColumnDef { key: "age", label: "Age", width: 90.0 },
            ColumnDef { key: "status", label: "Conditions", width: 150.0 },
        ],
        statuses: &["Available", "Progressing", "ReplicaFailure", "Paused"],
        creatable: true,
    },
    ResourceView {
        id: "daemon-sets",
        label: "DaemonSets",
        kind: ResourceKind::DaemonSets,
        namespaced: true,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 220.0 },
            ColumnDef { key: "namespace", label: "Namespace", width: 130.0 },
            ColumnDef { key: "pods", label: "Pods", width: 100.0 },
            ColumnDef { key: "nodeSelector", label: "Node Selector", width: 180.0 },
            ColumnDef { key: "age", label: "Age", width: 90.0 },
            ColumnDef { key: "status", label: "Status", width: 120.0 },
        ],
        statuses: &["Ready", "Progressing", "Empty"],
        creatable: true,
    },
    ResourceView {
        id: "stateful-sets",
        label: "StatefulSets",
        kind: ResourceKind::StatefulSets,
        namespaced: true,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 220.0 },
            ColumnDef { key: "namespace", label: "Namespace", width: 130.0 },
            ColumnDef { key: "pods", label: "Pods", width: 100.0 },
            ColumnDef { key: "replicas", label: "Replicas", width: 100.0 },
            ColumnDef { key: "age", label: "Age", width: 90.0 },
            ColumnDef { key: "status", label: "Status", width: 120.0 },
        ],
        statuses: &["Ready", "Progressing", "ScaledDown"],
        creatable: true,
    },
    ResourceView {
        id: "replica-sets",
        label: "ReplicaSets",
        kind: ResourceKind::ReplicaSets,
        namespaced: true,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 220.0 },
            ColumnDef { key: "namespace", label: "Namespace", width: 130.0 },
            ColumnDef { key: "desired", label: "Desired", width: 90.0 },
            ColumnDef { key: "current", label: "Current", width: 90.0 },
            ColumnDef { key: "ready", label: "Ready", width: 90.0 },
            ColumnDef { key: "age", label: "Age", width: 90.0 },
            ColumnDef { key: "status", label: "Status", width: 120.0 },
        ],
        statuses: &["Ready", "Scaling", "ScaledDown"],
        creatable: true,
    },
    ResourceView {
        id: "replication-controllers",
        label: "ReplicationControllers",
        kind: ResourceKind::ReplicationControllers,
        namespaced: true,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 220.0 },
            ColumnDef { key: "namespace", label: "Namespace", width: 130.0 },
            ColumnDef { key: "replicas", label: "Replicas", width: 100.0 },
            ColumnDef { key: "desiredReplicas", label: "Desired", width: 100.0 },
            ColumnDef { key: "selector", label: "Selector", width: 180.0 },
            ColumnDef { key: "age", label: "Age", width: 90.0 },
            ColumnDef { key: "status", label: "Status", width: 120.0 },
        ],
        statuses: &["Ready", "Progressing", "ScaledDown"],
        creatable: true,
    },
    ResourceView {
        id: "jobs",
        label: "Jobs",
        kind: ResourceKind::Jobs,
        namespaced: true,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 220.0 },
            ColumnDef { key: "namespace", label: "Namespace", width: 130.0 },
            ColumnDef { key: "completions", label: "Completions", width: 110.0 },
            ColumnDef { key: "age", label: "Age", width: 90.0 },
            ColumnDef { key: "status", label: "Conditions", width: 130.0 },
        ],
        statuses: &["Running", "Complete", "Failed", "Suspended"],
        creatable: true,
    },
    ResourceView {
        id: "cron-jobs",
        label: "CronJobs",
        kind: ResourceKind::CronJobs,
        namespaced: true,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 220.0 },
            ColumnDef { key: "namespace", label: "Namespace", width: 130.0 },
            ColumnDef { key: "schedule", label: "Schedule", width: 140.0 },
            ColumnDef { key: "suspend", label: "Suspend", width: 90.0 },
            ColumnDef { key: "active", label: "Active", width: 90.0 },
            ColumnDef { key: "lastSchedule", label: "Last Schedule", width: 140.0 },
            ColumnDef { key: "age", label: "Age", width: 90.0 },
            ColumnDef { key: "status", label: "Status", width: 120.0 },
        ],
        statuses: &["Scheduled", "Active", "Suspended"],
        creatable: true,
    },
    ResourceView {
        id: "config-maps",
        label: "ConfigMaps",
        kind: ResourceKind::ConfigMaps,
        namespaced: true,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 240.0 },
            ColumnDef { key: "namespace", label: "Namespace", width: 140.0 },
            ColumnDef { key: "keys", label: "Keys", width: 200.0 },
            ColumnDef { key: "age", label: "Age", width: 90.0 },
            ColumnDef { key: "status", label: "Mode", width: 120.0 },
        ],
        statuses: &[],
        creatable: true,
    },
    ResourceView {
        id: "secrets",
        label: "Secrets",
        kind: ResourceKind::Secrets,
        namespaced: true,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 240.0 },
            ColumnDef { key: "namespace", label: "Namespace", width: 140.0 },
            ColumnDef { key: "labels", label: "Labels", width: 160.0 },
            ColumnDef { key: "keys", label: "Keys", width: 160.0 },
            ColumnDef { key: "type", label: "Type", width: 150.0 },
            ColumnDef { key: "age", label: "Age", width: 90.0 },
            ColumnDef { key: "status", label: "Mode", width: 120.0 },
        ],
        statuses: &[],
        creatable: true,
    },
    ResourceView {
        id: "resource-quotas",
        label: "ResourceQuotas",
        kind: ResourceKind::ResourceQuotas,
        namespaced: true,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 240.0 },
            ColumnDef { key: "namespace", label: "Namespace", width: 140.0 },
            ColumnDef { key: "age", label: "Age", width: 90.0 },
            ColumnDef { key: "status", label: "Scope", width: 160.0 },
        ],
        statuses: &[],
        creatable: true,
    },
    ResourceView {
        id: "limit-ranges",
        label: "LimitRanges",
        kind: ResourceKind::LimitRanges,
        namespaced: true,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 240.0 },
            ColumnDef { key: "namespace", label: "Namespace", width: 140.0 },
            ColumnDef { key: "age", label: "Age", width: 90.0 },
            ColumnDef { key: "status", label: "Limits", width: 160.0 },
        ],
        statuses: &[],
        creatable: true,
    },
    ResourceView {
        id: "horizontal-pod-autoscalers",
        label: "HorizontalPodAutoscalers",
        kind: ResourceKind::HorizontalPodAutoscalers,
        namespaced: true,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 200.0 },
            ColumnDef { key: "namespace", label: "Namespace", width: 130.0 },
            ColumnDef { key: "metrics", label: "Metrics", width: 150.0 },
            ColumnDef { key: "minPods", label: "Min Pods", width: 90.0 },
            ColumnDef { key: "maxPods", label: "Max Pods", width: 90.0 },
            ColumnDef { key: "replicas", label: "Replicas", width: 90.0 },
            ColumnDef { key: "age", label: "Age", width: 80.0 },
            ColumnDef { key: "status", label: "Status", width: 130.0 },
        ],
        statuses: &["AbleToScale", "ScalingActive", "ScalingLimited"],
        creatable: true,
    },
    ResourceView {
        id: "vertical-pod-autoscalers",
        label: "VerticalPodAutoscalers",
        kind: ResourceKind::VerticalPodAutoscalers,
        namespaced: true,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 220.0 },
            ColumnDef { key: "namespace", label: "Namespace", width: 130.0 },
            ColumnDef { key: "mode", label: "Mode", width: 120.0 },
            ColumnDef { key: "age", label: "Age", width: 90.0 },
            ColumnDef { key: "status", label: "Status", width: 140.0 },
        ],
        statuses: &[],
        creatable: true,
    },
    ResourceView {
        id: "pod-disruption-budgets",
        label: "PodDisruptionBudgets",
        kind: ResourceKind::PodDisruptionBudgets,
        namespaced: true,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 200.0 },
            ColumnDef { key: "namespace", label: "Namespace", width: 130.0 },
            ColumnDef { key: "minAvailable", label: "Min Available", width: 110.0 },
            ColumnDef { key: "maxUnavailable", label: "Max Unavailable", width: 120.0 },
            ColumnDef { key: "currentHealthy", label: "Current Healthy", width: 120.0 },
            ColumnDef { key: "desiredHealthy", label: "Desired Healthy", width: 120.0 },
            ColumnDef { key: "age", label: "Age", width: 80.0 },
            ColumnDef { key: "status", label: "Status", width: 120.0 },
        ],
        statuses: &["Healthy", "DisruptionBlocked"],
        creatable: true,
    },
    ResourceView {
        id: "priority-classes",
        label: "PriorityClasses",
        kind: ResourceKind::PriorityClasses,
        namespaced: false,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 220.0 },
            ColumnDef { key: "value", label: "Value", width: 110.0 },
            ColumnDef { key: "globalDefault", label: "Global Default", width: 120.0 },
            ColumnDef { key: "age", label: "Age", width: 90.0 },
            ColumnDef { key: "status", label: "Preemption", width: 150.0 },
        ],
        statuses: &[],
        creatable: true,
    },
    ResourceView {
        id: "runtime-classes",
        label: "RuntimeClasses",
        kind: ResourceKind::RuntimeClasses,
        namespaced: false,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 240.0 },
            ColumnDef { key: "handler", label: "Handler", width: 180.0 },
            ColumnDef { key: "age", label: "Age", width: 90.0 },
            ColumnDef { key: "status", label: "Status", width: 120.0 },
        ],
        statuses: &[],
        creatable: true,
    },
    ResourceView {
        id: "leases",
        label: "Leases",
        kind: ResourceKind::Leases,
        namespaced: true,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 220.0 },
            ColumnDef { key: "namespace", label: "Namespace", width: 140.0 },
            ColumnDef { key: "holder", label: "Holder", width: 220.0 },
            ColumnDef { key: "age", label: "Age", width: 90.0 },
            ColumnDef { key: "status", label: "Status", width: 120.0 },
        ],
        statuses: &[],
        creatable: true,
    },
    ResourceView {
        id: "mutating-webhook-configs",
        label: "MutatingWebhookConfigs",
        kind: ResourceKind::MutatingWebhookConfigurations,
        namespaced: false,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 280.0 },
            ColumnDef { key: "webhooks", label: "Webhooks", width: 120.0 },
            ColumnDef { key: "age", label: "Age", width: 90.0 },
            ColumnDef { key: "status", label: "Status", width: 120.0 },
        ],
        statuses: &[],
        creatable: true,
    },
    ResourceView {
        id: "validating-webhook-configs",
        label: "ValidatingWebhookConfigs",
        kind: ResourceKind::ValidatingWebhookConfigurations,
        namespaced: false,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 280.0 },
            ColumnDef { key: "webhooks", label: "Webhooks", width: 120.0 },
            ColumnDef { key: "age", label: "Age", width: 90.0 },
            ColumnDef { key: "status", label: "Status", width: 120.0 },
        ],
        statuses: &[],
        creatable: true,
    },
    ResourceView {
        id: "validating-admission-policies",
        label: "ValidatingAdmissionPolicies",
        kind: ResourceKind::ValidatingAdmissionPolicies,
        namespaced: false,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 260.0 },
            ColumnDef { key: "failurePolicy", label: "Failure Policy", width: 140.0 },
            ColumnDef { key: "validations", label: "Validations", width: 110.0 },
            ColumnDef { key: "paramKind", label: "Param Kind", width: 140.0 },
            ColumnDef { key: "age", label: "Age", width: 90.0 },
        ],
        statuses: &[],
        creatable: true,
    },
    ResourceView {
        id: "validating-admission-policy-bindings",
        label: "ValidatingAdmissionPolicyBindings",
        kind: ResourceKind::ValidatingAdmissionPolicyBindings,
        namespaced: false,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 260.0 },
            ColumnDef { key: "policyName", label: "Policy", width: 200.0 },
            ColumnDef { key: "paramRef", label: "Param Ref", width: 160.0 },
            ColumnDef { key: "validationActions", label: "Actions", width: 140.0 },
            ColumnDef { key: "age", label: "Age", width: 90.0 },
        ],
        statuses: &[],
        creatable: true,
    },
    ResourceView {
        id: "services",
        label: "Services",
        kind: ResourceKind::Services,
        namespaced: true,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 200.0 },
            ColumnDef { key: "namespace", label: "Namespace", width: 130.0 },
            ColumnDef { key: "type", label: "Type", width: 120.0 },
            ColumnDef { key: "clusterIp", label: "Cluster IP", width: 130.0 },
            ColumnDef { key: "ports", label: "Ports", width: 160.0 },
            ColumnDef { key: "externalIp", label: "External IP", width: 140.0 },
            ColumnDef { key: "selector", label: "Selector", width: 180.0 },
            ColumnDef { key: "age", label: "Age", width: 80.0 },
            ColumnDef { key: "status", label: "Status", width: 110.0 },
        ],
        statuses: &["Active", "Pending"],
        creatable: true,
    },
    ResourceView {
        id: "endpoint-slices",
        label: "EndpointSlices",
        kind: ResourceKind::EndpointSlices,
        namespaced: true,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 220.0 },
            ColumnDef { key: "namespace", label: "Namespace", width: 130.0 },
            ColumnDef { key: "addressType", label: "Address Type", width: 120.0 },
            ColumnDef { key: "ports", label: "Ports", width: 140.0 },
            ColumnDef { key: "endpoints", label: "Endpoints", width: 140.0 },
            ColumnDef { key: "service", label: "Service", width: 160.0 },
            ColumnDef { key: "age", label: "Age", width: 80.0 },
        ],
        statuses: &[],
        creatable: true,
    },
    ResourceView {
        id: "endpoints",
        label: "Endpoints",
        kind: ResourceKind::Endpoints,
        namespaced: true,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 220.0 },
            ColumnDef { key: "namespace", label: "Namespace", width: 130.0 },
            ColumnDef { key: "endpoints", label: "Endpoints", width: 240.0 },
            ColumnDef { key: "age", label: "Age", width: 80.0 },
            ColumnDef { key: "status", label: "Status", width: 120.0 },
        ],
        statuses: &["Ready", "Empty"],
        creatable: true,
    },
    ResourceView {
        id: "ingresses",
        label: "Ingresses",
        kind: ResourceKind::Ingresses,
        namespaced: true,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 200.0 },
            ColumnDef { key: "namespace", label: "Namespace", width: 130.0 },
            ColumnDef { key: "loadBalancers", label: "LoadBalancers", width: 160.0 },
            ColumnDef { key: "rules", label: "Rules", width: 220.0 },
            ColumnDef { key: "age", label: "Age", width: 80.0 },
            ColumnDef { key: "status", label: "Class", width: 130.0 },
        ],
        statuses: &[],
        creatable: true,
    },
    ResourceView {
        id: "ingress-classes",
        label: "IngressClasses",
        kind: ResourceKind::IngressClasses,
        namespaced: false,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 200.0 },
            ColumnDef { key: "controller", label: "Controller", width: 240.0 },
            ColumnDef { key: "apiGroup", label: "API Group", width: 150.0 },
            ColumnDef { key: "scope", label: "Scope", width: 110.0 },
            ColumnDef { key: "kind", label: "Kind", width: 130.0 },
            ColumnDef { key: "age", label: "Age", width: 80.0 },
        ],
        statuses: &[],
        creatable: true,
    },
    ResourceView {
        id: "network-policies",
        label: "NetworkPolicies",
        kind: ResourceKind::NetworkPolicies,
        namespaced: true,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 220.0 },
            ColumnDef { key: "namespace", label: "Namespace", width: 140.0 },
            ColumnDef { key: "policyTypes", label: "Policy Types", width: 180.0 },
            ColumnDef { key: "age", label: "Age", width: 90.0 },
            ColumnDef { key: "status", label: "Selector", width: 180.0 },
        ],
        statuses: &[],
        creatable: true,
    },
    ResourceView {
        id: "persistent-volume-claims",
        label: "PersistentVolumeClaims",
        kind: ResourceKind::PersistentVolumeClaims,
        namespaced: true,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 200.0 },
            ColumnDef { key: "namespace", label: "Namespace", width: 130.0 },
            ColumnDef { key: "storageClass", label: "StorageClass", width: 150.0 },
            ColumnDef { key: "size", label: "Size", width: 100.0 },
            ColumnDef { key: "volume", label: "Volume", width: 180.0 },
            ColumnDef { key: "age", label: "Age", width: 80.0 },
            ColumnDef { key: "status", label: "Status", width: 110.0 },
        ],
        statuses: &["Bound", "Pending", "Lost"],
        creatable: true,
    },
    ResourceView {
        id: "persistent-volumes",
        label: "PersistentVolumes",
        kind: ResourceKind::PersistentVolumes,
        namespaced: false,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 220.0 },
            ColumnDef { key: "storageClass", label: "StorageClass", width: 150.0 },
            ColumnDef { key: "capacity", label: "Capacity", width: 110.0 },
            ColumnDef { key: "claim", label: "Claim", width: 180.0 },
            ColumnDef { key: "age", label: "Age", width: 80.0 },
            ColumnDef { key: "status", label: "Status", width: 120.0 },
        ],
        statuses: &["Available", "Bound", "Released", "Failed"],
        creatable: true,
    },
    ResourceView {
        id: "storage-classes",
        label: "StorageClasses",
        kind: ResourceKind::StorageClasses,
        namespaced: false,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 200.0 },
            ColumnDef { key: "provisioner", label: "Provisioner", width: 220.0 },
            ColumnDef { key: "reclaimPolicy", label: "Reclaim Policy", width: 130.0 },
            ColumnDef { key: "default", label: "Default", width: 90.0 },
            ColumnDef { key: "age", label: "Age", width: 80.0 },
            ColumnDef { key: "status", label: "Binding Mode", width: 150.0 },
        ],
        statuses: &[],
        creatable: true,
    },
    ResourceView {
        id: "events",
        label: "Events",
        kind: ResourceKind::Events,
        namespaced: true,
        columns: &[
            ColumnDef { key: "type", label: "Type", width: 100.0 },
            ColumnDef { key: "name", label: "Name", width: 200.0 },
            ColumnDef { key: "namespace", label: "Namespace", width: 130.0 },
            ColumnDef { key: "involvedObject", label: "Involved Object", width: 180.0 },
            ColumnDef { key: "source", label: "Source", width: 150.0 },
            ColumnDef { key: "count", label: "Count", width: 80.0 },
            ColumnDef { key: "lastSeen", label: "Last Seen", width: 110.0 },
            ColumnDef { key: "age", label: "Age", width: 80.0 },
            ColumnDef { key: "status", label: "Reason", width: 140.0 },
        ],
        statuses: &["Normal", "Warning"],
        creatable: false,
    },
    ResourceView {
        id: "service-accounts",
        label: "ServiceAccounts",
        kind: ResourceKind::ServiceAccounts,
        namespaced: true,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 240.0 },
            ColumnDef { key: "namespace", label: "Namespace", width: 140.0 },
            ColumnDef { key: "age", label: "Age", width: 90.0 },
            ColumnDef { key: "status", label: "Automount", width: 140.0 },
        ],
        statuses: &[],
        creatable: true,
    },
    ResourceView {
        id: "cluster-roles",
        label: "ClusterRoles",
        kind: ResourceKind::ClusterRoles,
        namespaced: false,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 280.0 },
            ColumnDef { key: "age", label: "Age", width: 90.0 },
            ColumnDef { key: "status", label: "Rules", width: 140.0 },
        ],
        statuses: &[],
        creatable: true,
    },
    ResourceView {
        id: "roles",
        label: "Roles",
        kind: ResourceKind::Roles,
        namespaced: true,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 240.0 },
            ColumnDef { key: "namespace", label: "Namespace", width: 140.0 },
            ColumnDef { key: "age", label: "Age", width: 90.0 },
            ColumnDef { key: "status", label: "Rules", width: 140.0 },
        ],
        statuses: &[],
        creatable: true,
    },
    ResourceView {
        id: "cluster-role-bindings",
        label: "ClusterRoleBindings",
        kind: ResourceKind::ClusterRoleBindings,
        namespaced: false,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 240.0 },
            ColumnDef { key: "bindings", label: "Subjects", width: 240.0 },
            ColumnDef { key: "age", label: "Age", width: 90.0 },
            ColumnDef { key: "status", label: "Role Ref", width: 180.0 },
        ],
        statuses: &[],
        creatable: true,
    },
    ResourceView {
        id: "role-bindings",
        label: "RoleBindings",
        kind: ResourceKind::RoleBindings,
        namespaced: true,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 220.0 },
            ColumnDef { key: "namespace", label: "Namespace", width: 130.0 },
            ColumnDef { key: "bindings", label: "Subjects", width: 220.0 },
            ColumnDef { key: "age", label: "Age", width: 90.0 },
            ColumnDef { key: "status", label: "Role Ref", width: 160.0 },
        ],
        statuses: &[],
        creatable: true,
    },
    ResourceView {
        id: "pod-security-policies",
        label: "PodSecurityPolicies",
        kind: ResourceKind::PodSecurityPolicies,
        namespaced: false,
        columns: &[
            ColumnDef { key: "name", label: "Name", width: 240.0 },
            ColumnDef { key: "privileged", label: "Privileged", width: 110.0 },
            ColumnDef { key: "volumes", label: "Volumes", width: 220.0 },
            ColumnDef { key: "age", label: "Age", width: 90.0 },
        ],
        statuses: &[],
        creatable: true,
    },
    ResourceView {
        id: "custom-resource-definitions",
        label: "CustomResourceDefinitions",
        kind: ResourceKind::CustomResourceDefinitions,
        namespaced: false,
        columns: &[
            ColumnDef { key: "name", label: "Resource", width: 240.0 },
            ColumnDef { key: "group", label: "Group", width: 180.0 },
            ColumnDef { key: "version", label: "Version", width: 100.0 },
            ColumnDef { key: "scope", label: "Scope", width: 110.0 },
            ColumnDef { key: "age", label: "Age", width: 80.0 },
            ColumnDef { key: "status", label: "Kind", width: 150.0 },
        ],
        statuses: &[],
        creatable: true,
    },
];

pub fn resource_view(id: &str) -> Option<&'static ResourceView> {
    RESOURCE_VIEWS.iter().find(|view| view.id == id)
}

pub fn resource_view_by_kind(kind: ResourceKind) -> Option<&'static ResourceView> {
    RESOURCE_VIEWS.iter().find(|view| view.kind == kind)
}

pub fn cell_value(item: &ResourceItem, column_key: &str) -> String {
    match column_key {
        "name" => item.name.clone(),
        "namespace" => item.namespace.clone().unwrap_or_else(|| "—".into()),
        "status" => item.status.clone(),
        "age" => format_age(item.created_at.as_deref()),
        "cpu" => {
            if let Some(node) = &item.node {
                format_node_usage(&node.cpu, true)
            } else {
                item.columns
                    .get("cpu")
                    .cloned()
                    .unwrap_or_else(|| "—".into())
            }
        }
        "memory" => {
            if let Some(node) = &item.node {
                format_node_usage(&node.memory, false)
            } else {
                item.columns
                    .get("memory")
                    .cloned()
                    .unwrap_or_else(|| "—".into())
            }
        }
        "roles" => {
            if let Some(node) = &item.node {
                if node.roles.is_empty() {
                    "worker".into()
                } else {
                    node.roles.join(", ")
                }
            } else {
                item.columns
                    .get("roles")
                    .cloned()
                    .unwrap_or_else(|| "—".into())
            }
        }
        "taints" => {
            if let Some(node) = &item.node {
                node.taints.to_string()
            } else {
                item.columns
                    .get("taints")
                    .cloned()
                    .unwrap_or_else(|| "0".into())
            }
        }
        "version" => {
            if let Some(node) = &item.node {
                node.version.clone().unwrap_or_else(|| "—".into())
            } else {
                item.columns
                    .get("version")
                    .cloned()
                    .unwrap_or_else(|| "—".into())
            }
        }
        other => item
            .columns
            .get(other)
            .cloned()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "—".into()),
    }
}

fn format_node_usage(usage: &iceon_core::connection::Usage, is_cpu: bool) -> String {
    let limit = usage.allocatable.or(usage.capacity);
    match (usage.used, limit) {
        (Some(used), Some(total)) if total > 0.0 => {
            let pct = ((used / total) * 100.0).round() as u64;
            if is_cpu {
                format!("{pct}% ({used:.2}/{total:.1}c)")
            } else {
                let used_gib = used / (1024.0 * 1024.0 * 1024.0);
                let total_gib = total / (1024.0 * 1024.0 * 1024.0);
                format!("{pct}% ({used_gib:.1}/{total_gib:.1}Gi)")
            }
        }
        (None, Some(total)) => {
            if is_cpu {
                format!("— / {total:.1}c")
            } else {
                let total_gib = total / (1024.0 * 1024.0 * 1024.0);
                format!("— / {total_gib:.1}Gi")
            }
        }
        _ => "—".into(),
    }
}

pub fn format_age(created_at: Option<&str>) -> String {
    let Some(raw) = created_at else {
        return "—".into();
    };
    let Ok(ts) = raw.parse::<jiff::Timestamp>() else {
        return raw.to_string();
    };
    let now = jiff::Timestamp::now();
    let secs = now.as_second().saturating_sub(ts.as_second()).max(0);
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

pub fn yaml_scaffold(kind: ResourceKind, namespace: Option<&str>) -> String {
    let (api_version, kind_name) = iceon_core::connection::create::creation_info(kind);
    let ns = namespace.unwrap_or("default");
    let namespaced = RESOURCE_VIEWS
        .iter()
        .find(|v| v.kind == kind)
        .is_none_or(|v| v.namespaced);
    let ns_line = if namespaced {
        format!("\n  namespace: {ns}")
    } else {
        String::new()
    };
    let body = match kind {
        ResourceKind::Pods => {
            "spec:\n  containers:\n    - name: app\n      image: nginx:stable\n      ports:\n        - containerPort: 80\n"
        }
        ResourceKind::Deployments | ResourceKind::ReplicaSets => {
            "spec:\n  replicas: 1\n  selector:\n    matchLabels:\n      app: sample\n  template:\n    metadata:\n      labels:\n        app: sample\n    spec:\n      containers:\n        - name: app\n          image: nginx:stable\n"
        }
        ResourceKind::StatefulSets => {
            "spec:\n  serviceName: sample\n  replicas: 1\n  selector:\n    matchLabels:\n      app: sample\n  template:\n    metadata:\n      labels:\n        app: sample\n    spec:\n      containers:\n        - name: app\n          image: nginx:stable\n"
        }
        ResourceKind::DaemonSets => {
            "spec:\n  selector:\n    matchLabels:\n      app: sample\n  template:\n    metadata:\n      labels:\n        app: sample\n    spec:\n      containers:\n        - name: app\n          image: nginx:stable\n"
        }
        ResourceKind::Jobs => {
            "spec:\n  template:\n    spec:\n      restartPolicy: Never\n      containers:\n        - name: job\n          image: busybox:stable\n          command: [\"sh\", \"-c\", \"echo hello\"]\n"
        }
        ResourceKind::CronJobs => {
            "spec:\n  schedule: \"*/5 * * * *\"\n  jobTemplate:\n    spec:\n      template:\n        spec:\n          restartPolicy: OnFailure\n          containers:\n            - name: job\n              image: busybox:stable\n              command: [\"sh\", \"-c\", \"date\"]\n"
        }
        ResourceKind::Services => {
            "spec:\n  selector:\n    app: sample\n  ports:\n    - name: http\n      port: 80\n      targetPort: 80\n      protocol: TCP\n"
        }
        ResourceKind::ConfigMaps => "data:\n  example.key: \"example-value\"\n",
        ResourceKind::Secrets => "type: Opaque\nstringData:\n  example.key: \"replace-me\"\n",
        ResourceKind::Namespaces => "",
        _ => "spec: {}\n",
    };
    format!(
        "apiVersion: {api_version}\nkind: {kind_name}\nmetadata:\n  name: sample{ns_line}\n{body}"
    )
}

pub fn catalog_error_message(error: CatalogError) -> &'static str {
    match error {
        CatalogError::InvalidInput => "Invalid catalog input or file path.",
        CatalogError::InvalidKubeconfig => "File is not a valid Kubernetes v1 Config.",
        CatalogError::InputUnavailable => "Kubeconfig file could not be read or accessed.",
        CatalogError::LimitExceeded => "Catalog size or entry limit exceeded.",
        CatalogError::NotFound => "Requested cluster entry was not found.",
        CatalogError::StoreUnavailable => "Local catalog database is unavailable.",
        CatalogError::StoreCorrupt => "Local catalog database failed integrity validation.",
        CatalogError::OpenFailed => "Failed to open kubeconfig selection.",
    }
}

pub fn connection_error_message(error: ConnectionError) -> &'static str {
    match error {
        ConnectionError::InvalidInput => "Invalid request parameters or YAML manifest.",
        ConnectionError::ProcessUnavailable => "Target container is not currently running.",
        ConnectionError::NotFound => "Target resource was not found on the cluster.",
        ConnectionError::KubeconfigUnavailable => "Referenced kubeconfig file is unavailable.",
        ConnectionError::KubeconfigInvalid => "Kubeconfig failed validation.",
        ConnectionError::SourceChanged => "Cluster or resource identity changed since last read.",
        ConnectionError::UnsupportedAuth => "Kubeconfig uses an unsupported authentication plugin.",
        ConnectionError::AuthHelperMissing => "Authentication exec helper binary was not found.",
        ConnectionError::AuthHelperFailed => "Authentication exec helper exited with an error.",
        ConnectionError::AuthHelperInvalidResponse => {
            "Authentication exec helper returned an invalid ExecCredential."
        }
        ConnectionError::AuthInteractiveRequired => {
            "Interactive authentication is required outside non-interactive mode."
        }
        ConnectionError::AuthExpired => "Cluster credentials have expired; reconnect to refresh.",
        ConnectionError::InsecureTransport => {
            "Insecure cluster transport (HTTP, proxy, or skip-TLS-verify) is rejected."
        }
        ConnectionError::AuthFailed => "Cluster authentication failed (HTTP 401).",
        ConnectionError::TlsFailed => "TLS handshake or certificate verification failed.",
        ConnectionError::PermissionDenied => "Kubernetes RBAC permission denied (HTTP 403).",
        ConnectionError::PermissionReviewUnavailable => {
            "SelfSubjectAccessReview could not verify permissions."
        }
        ConnectionError::ResourceUnavailable => {
            "Requested API resource is not available on this cluster."
        }
        ConnectionError::Timeout => "Cluster operation timed out after 15 seconds.",
        ConnectionError::Network => "Network error while communicating with the cluster.",
        ConnectionError::ConnectionFailed => "Could not establish connection to the cluster.",
        ConnectionError::NotConnected => "Cluster session is not connected.",
        ConnectionError::StaleSession => "Operation belonged to a superseded session or request.",
        ConnectionError::Busy => "Another cluster operation is currently in progress.",
        ConnectionError::LimitExceeded => "Response or payload exceeded safety limits.",
        ConnectionError::InvalidResponse => "Cluster returned an unexpected API payload.",
        ConnectionError::StoreUnavailable => "Local session state is unavailable.",
        ConnectionError::HelmUnavailable => "Helm CLI (v3.17+ / v4) was not found in PATH.",
        ConnectionError::Conflict => "Resource version conflict or Helm command error (HTTP 409).",
    }
}

pub fn secrets_error_message(error: secrets::Error) -> &'static str {
    match error {
        secrets::Error::InvalidInput => "Invalid profile or secret path input.",
        secrets::Error::NotFound => "Secret path or profile was not found.",
        secrets::Error::StoreUnavailable => "Local secrets profile store is unavailable.",
        secrets::Error::CredentialStoreUnavailable => "OS keyring credential store is unavailable.",
        secrets::Error::AuthFailed => "Vault authentication failed.",
        secrets::Error::PermissionDenied => "Vault permission denied for this path.",
        secrets::Error::SessionExpired => "Vault session expired; sign in again.",
        secrets::Error::StaleSession => "Vault session was superseded.",
        secrets::Error::Cancelled => "Operation was cancelled.",
        secrets::Error::Busy => "Another Vault operation is in progress.",
        secrets::Error::Network => "Network error communicating with Vault.",
        secrets::Error::Tls => "TLS verification failed communicating with Vault.",
        secrets::Error::InvalidResponse => "Vault returned an invalid response.",
        secrets::Error::TooLarge => "Vault response exceeded size limits.",
        secrets::Error::UnsupportedAuth => "Unsupported Vault auth method.",
    }
}

pub fn preferences_error_message(error: PreferencesError) -> &'static str {
    match error {
        PreferencesError::InvalidInput => {
            "Invalid preferences input (check font sizes, proxy URL, or paths)."
        }
        PreferencesError::Conflict => "Preferences were updated elsewhere; reload and retry.",
        PreferencesError::StoreUnavailable => "Preferences SQLite database is unavailable.",
        PreferencesError::StoreCorrupt => "Preferences database failed validation.",
    }
}

pub fn pin_label(target: &PinTarget, clusters: &[ClusterSummary]) -> (String, String) {
    let cluster_name = clusters
        .iter()
        .find(|c| c.id == target.id)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| target.id.clone());
    match &target.section {
        Some(section) => (
            format!("{cluster_name} · {}", destination_label(section)),
            section.clone(),
        ),
        None => (cluster_name, "overview".into()),
    }
}
