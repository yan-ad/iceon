//! Cluster workspace views: Overview, 41 Kubernetes Resource tables, Port Forwarding, Helm, Custom Resources, and RBAC Users.

use std::rc::Rc;
use std::time::Duration;

use crate::inspector_view::editor_surface::EditorSurface;
use crate::navigation::resource_view_by_kind;
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::{
    ActiveTheme as _, ColorName, Disableable as _, IconName, Selectable as _, Sizable as _,
    WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    resizable::{HandleEdge, resize_handle},
    scroll::ScrollableElement as _,
    select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState},
    tag::Tag,
    v_flex,
};

// Kept independently of the route/drawer lifecycle, so toggling either panel
// never discards the last chosen size. The file contains geometry only.
pub(crate) const SIDEBAR_MIN_WIDTH: f32 = 224.0;
pub(crate) const INSPECTOR_MIN_WIDTH: f32 = 360.0;

pub(crate) struct NavigationWidths {
    sidebar: f32,
    inspector: f32,
    dirty: bool,
}

impl Default for NavigationWidths {
    fn default() -> Self {
        Self {
            sidebar: 252.0,
            inspector: 420.0,
            dirty: false,
        }
    }
}

fn bounded_panel_width(value: f32, minimum: f32, maximum: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(minimum, maximum)
    } else {
        fallback
    }
}

impl NavigationWidths {
    pub(crate) fn sidebar_width(&self) -> f32 {
        self.sidebar
    }
    fn path() -> std::path::PathBuf {
        iceon_core::runtime::app_data_dir().join("navigation-widths")
    }

    fn parse(text: &str) -> Option<Self> {
        let mut values = text.split_whitespace();
        let sidebar = values.next()?.parse().ok()?;
        let inspector = values.next()?.parse().ok()?;
        if values.next().is_some() {
            return None;
        }
        Some(Self {
            sidebar: bounded_panel_width(sidebar, SIDEBAR_MIN_WIDTH, 420.0, 252.0),
            inspector: bounded_panel_width(inspector, INSPECTOR_MIN_WIDTH, 720.0, 420.0),
            dirty: false,
        })
    }

    pub(crate) fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    fn save(&mut self) {
        if !self.dirty {
            return;
        }
        let path = Self::path();
        // Atomic replacement prevents a partially written width on next launch.
        let temporary = path.with_extension("tmp");
        let result = (|| -> std::io::Result<()> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&temporary, format!("{} {}\n", self.sidebar, self.inspector))?;
            std::fs::rename(&temporary, &path)
        })();
        if result.is_ok() {
            self.dirty = false;
        }
    }
}

#[derive(Clone)]
struct ResizeNavigation {
    sidebar: bool,
}

impl Render for ResizeNavigation {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

#[cfg(test)]
mod navigation_width_tests {
    use super::{NavigationWidths, SIDEBAR_MIN_WIDTH};

    #[test]
    fn persisted_widths_round_trip_and_clamp_to_accessible_ranges() {
        let widths = NavigationWidths::parse("280 480\n").unwrap();
        assert_eq!((widths.sidebar, widths.inspector), (280.0, 480.0));
        let widths = NavigationWidths::parse("12 9999").unwrap();
        assert_eq!(
            (widths.sidebar, widths.inspector),
            (SIDEBAR_MIN_WIDTH, 720.0)
        );
        let widths = NavigationWidths::parse("NaN inf").unwrap();
        assert_eq!((widths.sidebar, widths.inspector), (252.0, 420.0));
        for bad in ["", "280", "wrong 480", "280 480 secret"] {
            assert!(NavigationWidths::parse(bad).is_none());
        }
    }
}

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

pub(crate) fn create_status_select(
    window: &mut Window,
    cx: &mut Context<IceonApp>,
) -> Entity<NamespaceSelectState> {
    let select = cx.new(|cx| {
        SelectState::new(
            SearchableVec::new(Vec::<NamespaceChoice>::new()),
            None,
            window,
            cx,
        )
    });
    cx.subscribe_in(
        &select,
        window,
        |this, _, event: &SelectEvent<SearchableVec<NamespaceChoice>>, window, cx| {
            let SelectEvent::Confirm(Some(status)) = event else {
                return;
            };
            this.selected_status_filter = (status != "All").then(|| status.clone());
            this.current_continue_token = None;
            this.continue_token_history.clear();
            this.reload_active_section(window, cx);
        },
    )
    .detach();
    select
}

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
use gpui_kit::component::InteractiveElementExt as _;
use gpui_kit::prelude::*;
use gpui_kit::*;

use iceon_core::connection::{
    OpenRequest, ResourceItem, ResourceKind, ResourcePage, ResourceRequest, SessionRequest,
    custom_resources::{CustomResourcePage, CustomResourceRequest, CustomResourceTarget},
    details::ResourceReference,
    helm::{
        HelmRequest, HelmView,
        install::{HelmInstallRequest, HelmInstallResult},
    },
    pod_sync::PodSyncRequest,
    rbac::RbacUsersRequest,
    rbac_management::{
        ApplyRequest, Assignment, ClusterAccess, ManagedUser, ManagementRequest, PlanOperation,
        PlanRequest, TemplateIdentity,
    },
};

use crate::{
    IceonApp,
    navigation::{Route, normalize_destination, resource_view},
};

#[derive(Clone, Debug, Default)]
pub struct OverviewSnapshot {
    pub nodes: Option<Rc<ResourcePage>>,
    pub namespaces: Option<Rc<ResourcePage>>,
    pub pods: Option<Rc<ResourcePage>>,
    pub deployments: Option<Rc<ResourcePage>>,
    pub daemonsets: Option<Rc<ResourcePage>>,
    pub statefulsets: Option<Rc<ResourcePage>>,
    pub replicasets: Option<Rc<ResourcePage>>,
    pub jobs: Option<Rc<ResourcePage>>,
    pub cronjobs: Option<Rc<ResourcePage>>,
    pub events: Option<Rc<ResourcePage>>,
}

#[derive(Clone, Copy)]
pub struct ColumnDef {
    pub header: &'static str,
    pub key: &'static str,
    pub width: f32,
}

// Shared by the resource table header and rows; fits four compact icon actions.
pub(crate) const RESOURCE_ACTIONS_WIDTH: f32 = 124.0;
pub(crate) const RESOURCE_NAME_WIDTH: f32 = 260.0;
pub(crate) const RESOURCE_NAMESPACE_WIDTH: f32 = 144.0;
pub(crate) const RESOURCE_AGE_WIDTH: f32 = 164.0;
pub(crate) const RESOURCE_STATUS_WIDTH: f32 = 148.0;

// Shared cell geometry: padding is inside each column; only internal vertical rules.
pub(crate) fn table_cell(width: f32, flexible: bool, _separator: bool, _border: Hsla) -> Div {
    h_flex()
        .h_full()
        .items_center()
        .px_3()
        .flex_shrink_0()
        .overflow_hidden()
        .when(flexible, |cell| cell.flex_1().min_w(px(width)))
        .when(!flexible, |cell| cell.w(px(width)))
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

pub(crate) fn parse_custom_resource_target(
    section: &str,
) -> Option<(Option<String>, Option<String>)> {
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
                header: "Containers",
                key: "containers",
                width: 140.0,
            },
            ColumnDef {
                header: "CPU",
                key: "cpu",
                width: 96.0,
            },
            ColumnDef {
                header: "Memory",
                key: "memory",
                width: 110.0,
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
                width: 220.0,
            },
            ColumnDef {
                header: "Versions",
                key: "versions",
                width: 140.0,
            },
            ColumnDef {
                header: "Short Names",
                key: "shortNames",
                width: 140.0,
            },
            ColumnDef {
                header: "Scope",
                key: "scope",
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

fn zero_resource_title(filtered: bool, pending: bool) -> &'static str {
    match (filtered, pending) {
        (true, true) => "Searching resources…",
        (false, true) => "Loading resources…",
        (true, false) => "No matching resources",
        (false, false) => "No resources found",
    }
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
        if normalized != self.active_cluster_section
            && !self.preferences_draft.app.persistent_search
        {
            self.resource_filter_input
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        if normalized != self.active_cluster_section {
            self.selected_status_filter = None;
        }
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

        self.selected_namespace = None;
        self.sync_namespace_select(window, cx);
        self.stop_active_pod_sync();
        self.access = crate::rbac_access::AccessState::default();
        self.cluster_session = None;
        self.cluster_loading = true;
        self.namespaces.clear();
        self.resource_page = None;
        self.overview_snapshot = OverviewSnapshot::default();
        self.helm_page = None;
        self.custom_discovery_generation = self.next_operation_id();
        self.custom_discovery_loading = false;
        self.custom_discovery_error = None;
        self.expanded_custom_groups.clear();
        self.custom_definition_group = None;
        self.custom_definitions_page = None;
        self.custom_instances_page = None;
        self.rbac_users_response = None;
        self.rbac_management_snapshot = None;
        self.rbac_active_plan = None;
        self.rbac_apply_response = None;
        cx.notify();

        let connection = self.connection.clone();
        let connect_generation = self.next_operation_id();
        self.cluster_connect_generation = connect_generation;
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
                        if this.cluster_connect_generation != connect_generation {
                            return;
                        }
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
            // A workspace owns one lease, but transient kubeconfig/network failures
            // should not require the user to leave and re-enter the cluster. Retry
            // only the connect phase; opening a different cluster still replaces
            // and cancels this lease through ConnectionEngine::open.
            let mut connected = connection.connect(scope.clone()).await;
            if connected.is_err() {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                connected = connection.connect(scope.clone()).await;
            }

            let _ = this.update_in(cx, |this, window, cx| {
                if this.cluster_connect_generation != connect_generation {
                    return;
                }
                this.cluster_loading = false;
                match connected {
                    Ok(snap) => {
                        let status = snap.status;
                        let err = snap.error;
                        if snap.status == "connected"
                            && let Some(version) = snap.api_version.as_ref()
                        {
                            this.cluster_versions
                                .insert(snap.cluster_id.clone(), version.clone());
                        }
                        this.cluster_session = Some(snap);
                        this.refresh_catalog(cx);
                        if status == "connected" {
                            this.refresh_access(window, cx);
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
        self.cancel_resource_pagination();
        let section = self.active_cluster_section.clone();
        if !self.ensure_access(window, cx) {
            self.cluster_loading = true;
            cx.notify();
            return;
        }
        if !self.route_allowed(&section) {
            self.cluster_loading = false;
            self.resource_page = None;
            self.cluster_error = Some(self.access_message().into());
            cx.notify();
            return;
        }

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
        }
    }

    fn load_overview_snapshot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.overview_generation = self.next_operation_id();
        let generation = self.overview_generation;
        self.overview_snapshot = OverviewSnapshot::default();
        self.cluster_error = None;
        self.cluster_loading = false;
        self.overview_pending = 0;
        cx.notify();

        let Some(session) = self
            .cluster_session
            .clone()
            .filter(|s| s.status == "connected")
        else {
            return;
        };
        let namespace = self.selected_namespace.clone();
        let section = self.active_cluster_section.clone();
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
        ]
        .into_iter()
        .filter(|kind| self.can_list(*kind))
        .collect::<Vec<_>>();
        let key = crate::overview_cache::OverviewCacheKey {
            cluster_id: session.cluster_id.clone(),
            session_id: session.session_id.clone(),
            namespace: namespace.clone(),
        };
        let cached = self.overview_cache.get(&key);
        let has_cached = cached.is_some();
        if let Some(mut snapshot) = cached {
            crate::overview_cache::retain_allowed(&mut snapshot, &kinds);
            self.overview_snapshot = snapshot;
        }
        let events_allowed = kinds.contains(&ResourceKind::Events);
        let workload_kinds = kinds
            .into_iter()
            .filter(|kind| *kind != ResourceKind::Events)
            .collect::<Vec<_>>();
        let mut batches = Vec::new();
        if !workload_kinds.is_empty() {
            batches.push((workload_kinds, false));
        }
        if events_allowed {
            batches.push((vec![ResourceKind::Events], true));
        }
        self.overview_pending = batches.len();
        let request_session = session.clone();
        let request_namespace = namespace.clone();
        let is_current = move |app: &Self| {
            app.overview_generation == generation
                && app.active_cluster_section == section
                && matches!(
                    section.as_str(),
                    "overview" | "workloads-overview" | "workloads/overview"
                )
                && app.selected_namespace == namespace
                && app.cluster_session.as_ref().is_some_and(|s| {
                    s.status == "connected"
                        && s.cluster_id == session.cluster_id
                        && s.session_id == session.session_id
                })
                && matches!(app.history.current(), Route::Cluster { cluster_id, .. } if cluster_id == &session.cluster_id)
        };
        let session = request_session;
        let namespace = request_namespace;
        self.cluster_loading = !has_cached && self.overview_pending > 0;
        cx.notify();

        // Events run as an independent future, never behind the workload pagination loop.
        for (kinds, events_preview) in batches {
            let connection = connection.clone();
            let session = session.clone();
            let namespace = namespace.clone();
            let key = key.clone();
            let is_current = is_current.clone();
            cx.spawn_in(window, async move |this, cx| {
                let mut errors = Vec::new();
                for kind in kinds {
                    let mut combined: Option<ResourcePage> = None;
                    let mut token = None;
                    let mut seen_tokens = std::collections::HashSet::new();
                    let mut incomplete = false;
                    let mut failed = false;
                    for page_index in 0..100 {
                        // Allocate an operation only while this exact overview and lease are active.
                        let Ok(Some(req)) = this.update_in(cx, |app, _, _| {
                            is_current(app).then(|| ResourceRequest {
                                cluster_id: session.cluster_id.clone(),
                                session_id: session.session_id.clone(),
                                kind,
                                read_id: app.next_operation_id(),
                                namespace: if namespaced_kind(kind) {
                                    namespace.clone()
                                } else {
                                    None
                                },
                                continue_token: token.clone(),
                                query: None,
                                status: None,
                                group: None,
                            })
                        }) else {
                            return;
                        };
                        let result = connection.resources(req.clone()).await;
                        if !this
                            .update_in(cx, |app, _, _| is_current(app))
                            .unwrap_or(false)
                        {
                            return;
                        }
                        let mut page = match result {
                            Ok(page)
                                if page.cluster_id == req.cluster_id
                                    && page.session_id == req.session_id
                                    && page.kind == req.kind
                                    && page.read_id == req.read_id
                                    && page.namespace == req.namespace =>
                            {
                                page
                            }
                            result => {
                                if let Some(page) = combined.as_mut() {
                                    page.truncated = true;
                                }
                                let reason = match result {
                                    Err(error) => format!("{error:?}"),
                                    Ok(_) => "response identity mismatch".to_string(),
                                };
                                failed = true;
                                errors.push(format!("{kind:?}: overview page failed: {reason}"));
                                break;
                            }
                        };
                        if events_preview {
                            // One API page only. Retain continuation, and explicitly mark
                            // any locally omitted rows (which cannot be resumed) incomplete.
                            page.truncated |=
                                page.continue_token.is_some() || page.items.len() > 100;
                            page.items.truncate(100);
                            combined = Some(page);
                            break;
                        }
                        token = page.continue_token.clone();
                        // A continuation alone is not permanent truncation once all pages arrive.
                        incomplete |= page.truncated && token.is_none();
                        let remaining = 10_000 - combined.as_ref().map_or(0, |p| p.items.len());
                        incomplete |= page.items.len() > remaining;
                        page.items.truncate(remaining);
                        if let Some(accumulated) = combined.as_mut() {
                            accumulated.items.append(&mut page.items);
                            accumulated.read_id = page.read_id;
                            accumulated.fetched_at = page.fetched_at;
                            accumulated.continue_token = token.clone();
                            accumulated.truncated = incomplete || token.is_some();
                        } else {
                            page.truncated = incomplete || token.is_some();
                            combined = Some(page);
                        }
                        let Some(next_token) = token.as_ref() else {
                            break;
                        };
                        if page_index == 99
                            || combined.as_ref().is_some_and(|p| p.items.len() >= 10_000)
                            || !seen_tokens.insert(next_token.clone())
                        {
                            // Retain the observed prefix, never a fabricated empty page.
                            if let Some(page) = combined.as_mut() {
                                page.truncated = true;
                            }
                            break;
                        }
                    }
                    // Merge only this successful kind into the currently published
                    // snapshot; never overwrite an independent events publication.
                    if !failed && let Some(page) = combined {
                        let _ = this.update_in(cx, |app, _, cx| {
                            if is_current(app) && app.can_list(kind) {
                                crate::overview_cache::apply_page(&mut app.overview_snapshot, page);
                                app.overview_cache
                                    .insert(key.clone(), app.overview_snapshot.clone());
                                cx.notify();
                            }
                        });
                    }
                }
                let _ = this.update_in(cx, |app, _, cx| {
                    if !is_current(app) {
                        return;
                    }
                    app.overview_pending = app.overview_pending.saturating_sub(1);
                    app.cluster_loading = !has_cached && app.overview_pending > 0;
                    if !errors.is_empty() {
                        let message = errors.join("; ");
                        if let Some(prior) = app.cluster_error.as_mut() {
                            prior.push_str("; ");
                            prior.push_str(&message);
                        } else {
                            app.cluster_error = Some(message);
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }

    pub(crate) fn cancel_resource_pagination(&mut self) {
        if let Some(request) = self.resource_pagination.request.take() {
            let _ = self.connection.cancel_resources(request);
        }
        self.resource_pagination = Default::default();
    }

    pub(crate) fn load_resource_table(
        &mut self,
        kind: ResourceKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.ensure_access(window, cx) || !self.can_list(kind) {
            return;
        }
        if let Some(table) = &self.resource_table {
            table.update(cx, |table, cx| table.clear_selection(cx));
        }
        self.cancel_resource_pagination();
        self.stop_active_pod_sync();
        self.current_continue_token = None;
        self.continue_token_history.clear();
        self.resource_page = None;
        self.cluster_error = None;
        self.fetch_resource_page(kind, None, window, cx);
    }

    pub(crate) fn load_more_resources(
        &mut self,
        kind: ResourceKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self
            .resource_pagination
            .request
            .as_ref()
            .is_some_and(|r| self.resource_request_is_current(r, cx))
        {
            return;
        }
        if self.resource_pagination.loading
            || self.resource_pagination.stopped
            || self.cluster_error.is_some()
        {
            return;
        }
        let Some(token) = self
            .resource_page
            .as_ref()
            .filter(|p| p.kind == kind)
            .and_then(|p| p.continue_token.clone())
        else {
            return;
        };
        if let Err(error) = self.resource_pagination.begin_next(&token) {
            window.push_notification(
                gpui_kit::component::notification::Notification::warning(error),
                cx,
            );
            cx.notify();
            return;
        }
        self.fetch_resource_page(kind, Some(token), window, cx);
    }

    fn notify_resource_page_failure(
        &mut self,
        req: &ResourceRequest,
        message: String,
        retry: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.resource_pagination
            .failed(req.continue_token.as_deref());
        self.cluster_loading = false;
        let owner = cx.entity().downgrade();
        let request = req.clone();
        let note = gpui_kit::component::notification::Notification::warning(message)
            .title("Could not load more resources")
            .on_click(move |_, window, cx| {
                if !retry {
                    return;
                }
                let _ = owner.update(cx, |app, cx| {
                    if !app.resource_request_is_current(&request, cx) {
                        return;
                    }
                    app.resource_pagination.retry();
                    if request.continue_token.is_some() {
                        app.load_more_resources(request.kind, window, cx);
                    } else {
                        app.fetch_resource_page(request.kind, None, window, cx);
                    }
                });
            });
        window.push_notification(note, cx);
    }

    fn resource_request_is_current(&self, req: &ResourceRequest, cx: &Context<Self>) -> bool {
        self.resource_pagination.request.as_ref() == Some(req)
            && self.cluster_session.as_ref().is_some_and(|s| {
                s.status == "connected"
                    && s.cluster_id == req.cluster_id
                    && s.session_id == req.session_id
            })
            && section_to_resource_kind(&self.active_cluster_section) == Some(req.kind)
            && self
                .selected_namespace
                .as_ref()
                .filter(|_| namespaced_kind(req.kind))
                == req.namespace.as_ref()
            && (req.kind != ResourceKind::CustomResourceDefinitions
                || self.custom_definition_group == req.group)
            && self.selected_status_filter == req.status
            && self.resource_filter_input.read(cx).value().trim()
                == req.query.as_deref().unwrap_or_default()
            && matches!(self.history.current(), Route::Cluster { cluster_id, .. } if cluster_id == &req.cluster_id)
    }

    fn fetch_resource_page(
        &mut self,
        kind: ResourceKind,
        token: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.ensure_access(window, cx) || !self.can_list(kind) {
            return;
        }
        let Some(session) = self
            .cluster_session
            .clone()
            .filter(|s| s.status == "connected")
        else {
            return;
        };
        let query = self
            .resource_filter_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let req = ResourceRequest {
            cluster_id: session.cluster_id,
            session_id: session.session_id,
            kind,
            read_id: self.next_operation_id(),
            namespace: if namespaced_kind(kind) {
                self.selected_namespace.clone()
            } else {
                None
            },
            continue_token: token,
            query: (!query.is_empty()).then_some(query),
            status: self.selected_status_filter.clone(),
            group: (kind == ResourceKind::CustomResourceDefinitions)
                .then(|| self.custom_definition_group.clone())
                .flatten(),
        };
        self.resource_pagination.request = Some(req.clone());
        self.resource_pagination.loading = true;
        self.cluster_loading = true;
        let connection = self.connection.clone();
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = connection.resources(req.clone()).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.resource_request_is_current(&req, cx) { return; }
                this.resource_pagination.loading = false;
                this.cluster_loading = false;
                match result {
                    Ok(page) if this.resource_pagination.accepts(&req, &page) => {
                        this.resource_pagination.completed();
                        let (page, limited) = crate::resource_pagination::merge_page(
                            this.resource_page.as_deref().filter(|_| req.continue_token.is_some()), page);
                        let incomplete = page.truncated && page.continue_token.is_none();
                        if req.kind == ResourceKind::Namespaces {
                            this.namespaces = page.items.iter().map(|item| item.name.clone()).collect();
                            this.sync_namespace_select(window,cx);
                        }
                        this.resource_page = Some(Rc::new(page));
                        if limited || incomplete {
                            this.resource_pagination.stopped = true;
                            window.push_notification(gpui_kit::component::notification::Notification::warning(if limited {
                                "Resource scan reached the 50,000 item limit. Narrow the namespace or search. Loaded rows are preserved."
                            } else { "Resource response was incomplete without a continuation. Loaded rows are preserved; refresh to retry." }), cx);
                        } else if req.query.is_some() || req.status.is_some()
                            || this.resource_page.as_ref().is_some_and(|p| p.items.is_empty()) {
                            this.load_more_resources(kind, window, cx);
                        } else if kind == ResourceKind::Pods && req.continue_token.is_none()
                            && this.resource_page.as_ref().is_some_and(|p| p.continue_token.is_none()) {
                            this.start_pod_sync(window, cx);
                        }
                    }
                    Ok(_) => this.notify_resource_page_failure(&req, "The response did not match the requested page. Loaded rows are preserved; refresh to retry.".into(), false, window, cx),
                    Err(err) => this.notify_resource_page_failure(&req, format!("Loaded rows are preserved. Click this notification to retry: {err:?}"), true, window, cx),
                }
                cx.notify();
            });
        }).detach();
    }

    fn start_pod_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.resource_allowed(
            ResourceKind::Pods,
            iceon_core::connection::permissions::PermissionAction::Watch,
            self.selected_namespace.as_deref(),
            None,
        ) {
            return;
        }
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        if session.status != "connected" {
            return;
        }
        let Some(req) = self.resource_pagination.request.clone() else {
            return;
        };
        let connection = self.connection.clone();

        cx.spawn_in(window, async move |this, cx| {
            let opened = match connection.pod_sync_open(req.clone()).await {
                Ok(snap) => snap,
                Err(_) => return,
            };
            let sync_id = opened.sync_id.clone();
            let mut last_seq = opened.sequence;
            let initial_page = opened.page;

            let mut accepted = false;
            let _ = this.update_in(cx, |this, _, cx| {
                if !this.resource_request_is_current(&req, cx) {
                    return;
                }
                if initial_page
                    .as_ref()
                    .is_some_and(|p| !this.resource_pagination.accepts(&req, p)
                        || p.truncated || p.continue_token.is_some())
                {
                    return;
                }
                accepted = true;
                this.active_pod_sync_id = Some(sync_id.clone());
                if let Some(page) = initial_page {
                    this.resource_page = Some(Rc::new(page));
                }
                cx.notify();
            });

            if !accepted {
                let _ = connection.pod_sync_close(PodSyncRequest {
                    cluster_id: session.cluster_id.clone(),
                    session_id: session.session_id.clone(),
                    sync_id,
                });
                return;
            }
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
                        // A filter/route/session can change before its reload
                        // callback runs. Never publish a snapshot for that old scope.
                        if !this.resource_request_is_current(&req, cx) {
                            this.stop_active_pod_sync();
                            return;
                        }
                        if snap.page.as_ref().is_some_and(|p| !this.resource_pagination.accepts(&req, p)) {
                            this.stop_active_pod_sync();
                            this.cluster_error = Some("Live pod response identity mismatch; keeping loaded rows. Refresh to retry.".into());
                            cx.notify();
                            return;
                        }
                        if snap.page.as_ref().is_some_and(|p| p.truncated || p.continue_token.is_some()) {
                            this.stop_active_pod_sync();
                            this.cluster_error = Some("Live pod inventory exceeded its bounded snapshot; keeping loaded rows. Refresh to fetch all pages.".into());
                            cx.notify();
                            return;
                        }
                        still_active = true;
                        if snap.sequence > last_seq {
                            last_seq = snap.sequence;
                            if let Some(page) = snap
                                .page
                                .filter(|p| !p.truncated && p.continue_token.is_none())
                            {
                                this.resource_page = Some(Rc::new(page));
                                cx.notify();
                            }
                        }
                    }
                });
                if !still_active || is_stopped {
                    break;
                }
            }
            // Errors and stopped snapshots also retire the UI's live marker;
            // do not clear a newer stream that replaced this one meanwhile.
            let _ = this.update_in(cx, |this, _, cx| {
                if this.active_pod_sync_id.as_deref() == Some(sync_id.as_str()) {
                    this.stop_active_pod_sync();
                    cx.notify();
                }
            });
            let _ = connection.pod_sync_close(PodSyncRequest {
                cluster_id: session.cluster_id,
                session_id: session.session_id,
                sync_id,
            });
        })
        .detach();
    }

    fn load_helm_section(&mut self, view: HelmView, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let read_id = self.next_operation_id();
        self.helm_operation = read_id;
        let section = self.active_cluster_section.clone();
        let namespace = self.selected_namespace.clone();
        let req = HelmRequest {
            cluster_id: session.cluster_id.clone(),
            session_id: session.session_id.clone(),
            view,
            namespace: if view == HelmView::Releases {
                self.selected_namespace.clone()
            } else {
                None
            },
            read_id,
        };
        let helm_preferences =
            iceon_core::connection::helm::HelmPreferences::from(&self.preferences_draft);
        let connection = self.connection.clone();
        self.helm_page = None;
        self.cluster_error = None;
        self.cluster_loading = true;
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let result = connection
                .helm_table_with_preferences(req, helm_preferences)
                .await;
            let _ = this.update_in(cx, |this, _, cx| {
                if this.helm_operation != read_id
                    || this.active_cluster_section != section
                    || this.selected_namespace != namespace
                    || !this.cluster_session.as_ref().is_some_and(|current| {
                        current.cluster_id == session.cluster_id
                            && current.session_id == session.session_id
                    })
                {
                    return;
                }
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
        self.custom_instances_page = None;
        self.cluster_error = None;
        let selected = self
            .custom_definitions_page
            .as_ref()
            .and_then(|page| {
                page.items.iter().find(|item| {
                    crd_name.as_ref().is_some_and(|name| name == &item.name)
                        && group
                            .as_ref()
                            .is_none_or(|group| item.columns.get("group") == Some(group))
                })
            })
            .cloned();
        if let Some(definition) = selected {
            if let Some(group) = definition.columns.get("group")
                && !self.expanded_custom_groups.contains(group)
            {
                self.expanded_custom_groups.push(group.clone());
            }
            if let Some(uid) = definition.uid {
                let namespace = if definition
                    .columns
                    .get("scope")
                    .is_some_and(|scope| scope == "Namespaced")
                {
                    self.selected_namespace.clone()
                } else {
                    None
                };
                self.load_crd_instances_only(definition.name, uid, namespace, window, cx);
            } else {
                self.cluster_loading = false;
                self.cluster_error = Some("Definition has no UID; reload discovery.".into());
            }
        } else {
            self.cluster_loading = self.custom_discovery_loading;
            if !self.custom_discovery_loading {
                self.cluster_error =
                    Some(self.custom_discovery_error.clone().unwrap_or_else(|| {
                        "Selected custom resource definition is unavailable. Refresh discovery."
                            .into()
                    }));
            }
        }
        cx.notify();
    }

    fn load_crd_instances_only(
        &mut self,
        definition_name: String,
        definition_uid: String,
        namespace: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let operation = self.next_operation_id();
        self.custom_instance_operation = operation;
        self.cluster_loading = true;
        let section = self.active_cluster_section.clone();
        let selected_namespace = self.selected_namespace.clone();
        let req = CustomResourceRequest {
            cluster_id: session.cluster_id.clone(),
            session_id: session.session_id.clone(),
            read_id: operation,
            target: CustomResourceTarget {
                definition_name,
                definition_uid,
            },
            namespace,
            continue_token: None,
        };
        let connection = self.connection.clone();
        cx.spawn_in(window, async move |owner, cx| {
            let result = connection.custom_resources(req).await;
            let _ = owner.update_in(cx, |app, _, cx| {
                if app.custom_instance_operation != operation
                    || app.active_cluster_section != section
                    || app.selected_namespace != selected_namespace
                    || !app.cluster_session.as_ref().is_some_and(|s| {
                        s.cluster_id == session.cluster_id && s.session_id == session.session_id
                    })
                {
                    return;
                }
                app.cluster_loading = false;
                match result {
                    Ok(page) => app.custom_instances_page = Some(page),
                    Err(error) => {
                        app.cluster_error =
                            Some(format!("Custom resources query failed: {error:?}"))
                    }
                }
                cx.notify();
            });
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
        let helm_preferences =
            iceon_core::connection::helm::HelmPreferences::from(&self.preferences_draft);
        let connection = self.connection.clone();
        cx.spawn_in(window, async move |this, cx| {
            let res: std::result::Result<HelmInstallResult, _> = connection
                .helm_install_with_preferences(req, helm_preferences, false)
                .await;
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
        let cluster_route = matches!(self.history.current(), Route::Cluster { .. });
        if cluster_route && self.initial_access_loading() {
            let cluster_name = self
                .cluster_session
                .as_ref()
                .and_then(|session| {
                    self.catalog_snapshot
                        .clusters
                        .iter()
                        .find(|cluster| cluster.id == session.cluster_id)
                })
                .map(|cluster| cluster.name.clone())
                .unwrap_or_else(|| "Cluster".into());
            return v_flex()
                .size_full()
                .min_h_0()
                .min_w_0()
                .items_center()
                .justify_center()
                .bg(cx.theme().background)
                .p_6()
                .child(
                    v_flex()
                        .w_full()
                        .max_w(px(640.))
                        .items_center()
                        .gap_4()
                        .child(
                            div()
                                .w_full()
                                .text_center()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .truncate()
                                .child(cluster_name),
                        )
                        .child(
                            gpui_kit::component::spinner::Spinner::new()
                                .large()
                                .color(cx.theme().primary),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().foreground)
                                .child("Checking cluster access…"),
                        )
                        .child(
                            v_flex()
                                .items_center()
                                .gap_2()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("Connection established")
                                .child("Reviewing resource and namespace permissions")
                                .child("Preparing available navigation and actions"),
                        ),
                )
                .into_any_element();
        }
        let sidebar_open =
            cluster_route && !self.sidebar_collapsed && !self.cluster_sidebar_collapsed;
        let console_open = self.pod_dock_open;
        let inspector_open =
            cluster_route && self.inspector_open && self.inspector.target.is_some();
        let area = self.workspace_dock.as_mut().map(|dock| {
            dock.sync(console_open, window, cx);
            dock.dock_area.clone()
        });
        h_flex()
            .id("resizable-cluster-workspace")
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .on_drag_move(cx.listener(
                move |this, event: &DragMoveEvent<ResizeNavigation>, _, cx| {
                    let sidebar = event.drag(cx).sidebar;
                    let requested: f32 = if sidebar {
                        (event.event.position.x - event.bounds.left()).into()
                    } else {
                        (event.bounds.right() - event.event.position.x).into()
                    };
                    // Keep the table reachable while respecting each panel's minimum.
                    let total: f32 = event.bounds.size.width.into();
                    let other = if sidebar && inspector_open {
                        this.navigation_widths.inspector
                    } else if !sidebar && sidebar_open {
                        this.navigation_widths.sidebar
                    } else {
                        0.0
                    };
                    let (minimum, maximum, fallback) = if sidebar {
                        (SIDEBAR_MIN_WIDTH, 420.0_f32, 252.0)
                    } else {
                        (INSPECTOR_MIN_WIDTH, 720.0_f32, 420.0)
                    };
                    let width = bounded_panel_width(
                        requested,
                        minimum,
                        maximum.min((total - other - 320.0).max(minimum)),
                        fallback,
                    );
                    if sidebar {
                        this.navigation_widths.sidebar = width;
                    } else {
                        this.navigation_widths.inspector = width;
                    }
                    this.navigation_widths.dirty = true;
                    cx.notify();
                },
            ))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.navigation_widths.save()),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.navigation_widths.save()),
            )
            .when(sidebar_open, |row| {
                let cluster_id = self
                    .cluster_session
                    .as_ref()
                    .map(|session| session.cluster_id.clone())
                    .unwrap_or_default();
                row.child(
                    div()
                        .relative()
                        .w(px(self.navigation_widths.sidebar))
                        .min_w(px(SIDEBAR_MIN_WIDTH))
                        .h_full()
                        .flex_shrink_0()
                        .child(self.render_cluster_sidebar(&cluster_id, cx))
                        .child(
                            resize_handle("resource-sidebar-resize", Axis::Horizontal)
                                .inside(HandleEdge::Trailing)
                                .with_appearance(Rc::new(|_, _, _| Some(div().into_any_element())))
                                .on_drag(ResizeNavigation { sidebar: true }, |drag, _, _, cx| {
                                    cx.new(|_| drag.clone())
                                })
                                .on_double_click(cx.listener(|this, _, _, cx| {
                                    this.navigation_widths.sidebar = 252.0;
                                    this.navigation_widths.dirty = true;
                                    this.navigation_widths.save();
                                    cx.notify();
                                })),
                        ),
                )
            })
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(div().flex_1().min_h_0().min_w_0().children(area))
                    .when(
                        !console_open && !matches!(self.history.current(), Route::Setup),
                        |column| column.child(self.render_console_tabs(window, cx)),
                    ),
            )
            .when(inspector_open, |row| {
                row.child(
                    div()
                        .relative()
                        .w(px(self.navigation_widths.inspector))
                        .min_w(px(INSPECTOR_MIN_WIDTH))
                        .h_full()
                        .flex_shrink_0()
                        .child(self.render_inspector_drawer(window, cx))
                        .child(
                            resize_handle("inspector-resize", Axis::Horizontal)
                                .inside(HandleEdge::Leading)
                                .with_appearance(Rc::new(|_, _, _| Some(div().into_any_element())))
                                .on_drag(ResizeNavigation { sidebar: false }, |drag, _, _, cx| {
                                    cx.new(|_| drag.clone())
                                })
                                .on_double_click(cx.listener(|this, _, _, cx| {
                                    this.navigation_widths.inspector = 420.0;
                                    this.navigation_widths.dirty = true;
                                    this.navigation_widths.save();
                                    cx.notify();
                                })),
                        ),
                )
            })
            .into_any_element()
    }

    pub(crate) fn render_cluster_main(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let section = self.active_cluster_section.clone();
        let selected = self
            .resource_table
            .as_ref()
            .map(|table| table.read(cx).selected_targets().len())
            .unwrap_or_default();
        let deleting = self.bulk_delete.active.is_some();
        let active_kind = section_to_resource_kind(&section);
        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .child(
                v_flex()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .w_full()
                    .child(self.render_cluster_section_body(&section, active_kind, window, cx))
                    .when_some(
                        self.cluster_error.clone().filter(|_| active_kind.is_none() && parse_custom_resource_target(&section).is_none() && !matches!(section.as_str(), "helm-charts" | "helm-releases" | "helm/charts" | "helm/releases") && !matches!(section.as_str(), "overview" | "workloads-overview" | "workloads/overview")),
                        |el, error| el.child(crate::empty_state::error_state(error)),
                    )
                    .when_some(active_kind.filter(|kind| self.can_list(*kind)), |el, kind| {
                        let create_namespace = if namespaced_kind(kind) { self.selected_namespace.as_deref() } else { None };
                        let can_create = self.can_action(kind, iceon_core::connection::permissions::PermissionAction::Create, create_namespace, None);
                        el.child(
                            v_flex().absolute().right(px(20.0)).bottom(px(20.0)).occlude().gap_2()
                                .when_some(self.bulk_delete.message.clone(), |group, message| group.child(div().max_w(px(620.0)).text_xs().child(message)))
                                .child(h_flex().gap_2().items_center()
                                    .child(Button::new("delete-checked-resources").outline().icon(gpui_kit_assets::IconName::Trash)
                                        .accessibility_label(format!("Delete {selected} selected resources"))
                                        .disabled(selected == 0 || deleting || kind == ResourceKind::Nodes)
                                        .tooltip(format!("Delete {selected} selected resources · normal grace periods"))
                                        .on_click(cx.listener(|app, _, window, cx| { cx.stop_propagation(); app.confirm_bulk_delete(false, window, cx); })))
                                    .child(Button::new("force-delete-checked-resources").danger().icon(IconName::TriangleAlert)
                                        .accessibility_label(format!("Force delete {selected} selected pods"))
                                        .disabled(selected == 0 || deleting || kind != ResourceKind::Pods)
                                        .tooltip(format!("Force delete {selected} selected pods · zero grace period; finalizers remain"))
                                        .on_click(cx.listener(|app, _, window, cx| { cx.stop_propagation(); app.confirm_bulk_delete(true, window, cx); })))
                                    .when(can_create, |actions| actions.child(
                                    Button::new("open-create-yaml-btn")
                                        .primary()
                                        .rounded_full()
                                        .shadow_lg()
                                        .icon(IconName::Plus)
                                        .tooltip("Create a resource from YAML")
                                        .accessibility_label("Create a resource from YAML")
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            cx.stop_propagation();
                                            this.open_create_yaml_tab(kind, window, cx);
                                        })),
                                ))),
                        )
                    }),
            )
            .into_any_element()
    }

    pub(crate) fn sync_namespace_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let namespace_items = std::iter::once(NamespaceChoice {
            name: "All Namespaces".into(),
        })
        .chain(
            self.namespaces
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
        let namespace_value = self
            .selected_namespace
            .clone()
            .unwrap_or_else(|| "All Namespaces".into());
        if self.namespace_select.read(cx).selected_value() != Some(&namespace_value) {
            self.namespace_select.update(cx, |state, cx| {
                state.set_selected_value(&namespace_value, window, cx);
            });
        }
    }

    pub(crate) fn render_header_resource_controls(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let section = self.active_cluster_section.clone();
        let active_kind = section_to_resource_kind(&section);
        let dock_count = self.pod_dock_tabs.len() + self.port_forwards.len();
        self.sync_namespace_select(window, cx);

        let statuses = active_kind
            .and_then(|kind| resource_view_by_kind(kind).map(|view| view.statuses))
            .unwrap_or(&[]);
        let show_status_select = !statuses.is_empty();
        let status_options: Vec<NamespaceChoice> = std::iter::once("All")
            .chain(statuses.iter().copied())
            .map(|name| NamespaceChoice { name: name.into() })
            .collect();
        if self.status_select_items != status_options {
            self.status_select.update(cx, |state, cx| {
                state.set_items(SearchableVec::new(status_options.clone()), window, cx);
            });
            self.status_select_items = status_options.clone();
        }
        let status_value = self
            .selected_status_filter
            .clone()
            .unwrap_or_else(|| "All".into());
        if self.status_select.read(cx).selected_value() != Some(&status_value) {
            self.status_select.update(cx, |state, cx| {
                state.set_selected_value(&status_value, window, cx);
            });
        }

        h_flex()
            .gap_2()
            .items_center()
            .flex_shrink_0()
            .when(active_kind.is_some(), |el| {
                el.child(
                    div().w(px(200.0)).child(
                        Input::new(&self.resource_filter_input)
                            .xsmall()
                            .cleanable(true),
                    ),
                )
            })
            .when(
                active_kind == Some(ResourceKind::CustomResourceDefinitions),
                |el| {
                    let groups = crate::custom_resource_navigation::grouped_definitions(
                        self.custom_definitions_page.as_ref(),
                    );
                    let owner = cx.entity().downgrade();
                    el.child(
                        Button::new("crd-group-filter")
                            .xsmall()
                            .outline()
                            .label(
                                self.custom_definition_group
                                    .clone()
                                    .unwrap_or_else(|| "All groups".into()),
                            )
                            .dropdown_menu(move |mut menu, _, cx| {
                                let current = owner
                                    .upgrade()
                                    .and_then(|app| app.read(cx).custom_definition_group.clone());
                                for group in
                                    std::iter::once(None).chain(groups.keys().cloned().map(Some))
                                {
                                    let owner = owner.clone();
                                    menu = menu.item(
                                        gpui_kit::component::menu::PopupMenuItem::new(
                                            group.clone().unwrap_or_else(|| "All groups".into()),
                                        )
                                        .checked(current == group)
                                        .on_click(
                                            move |_, window, cx| {
                                                let _ = owner.update(cx, |app, cx| {
                                                    app.custom_definition_group = group.clone();
                                                    app.reload_active_section(window, cx);
                                                });
                                            },
                                        ),
                                    );
                                }
                                menu
                            }),
                    )
                },
            )
            .when(show_status_select, |el| {
                el.child(
                    div().w(px(120.0)).child(
                        Select::new(&self.status_select)
                            .id("status-select")
                            .xsmall()
                            .w_full()
                            .placeholder("Status")
                            .search_placeholder("Filter statuses…")
                            .menu_width(px(190.0)),
                    ),
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
                    .icon(IconName::RefreshCw)
                    .tooltip("Refresh")
                    .accessibility_label("Refresh")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.reload_active_section(window, cx);
                    })),
            )
    }

    fn render_cluster_section_body(
        &mut self,
        section: &str,
        active_kind: Option<ResourceKind>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if matches!(
            section,
            "overview" | "workloads-overview" | "workloads/overview"
        ) {
            return self.render_overview_dashboard(cx);
        }
        if !self.access_ready() || !self.route_allowed(section) {
            if self.route_visible(section) {
                let reviewing = !self.access_ready();
                return v_flex().size_full().min_h_0().items_center().justify_center().gap_3().p_6()
                    .child(crate::empty_state::empty_state(IconName::Folder,
                        if reviewing { "Checking namespace access" } else { "Choose a namespace you can access" },
                        if reviewing { "The navigation stays available while access is reviewed." } else { "This resource is not accessible in the current scope. Choose a provided namespace to continue." }))
                    .child(self.namespace_dropdown("resource-choose-namespace", "Choose namespace…".into(), cx))
                    .into_any_element();
            }
            return crate::empty_state::empty_state(
                IconName::LayoutDashboard,
                "Access not available",
                self.access_message(),
            )
            .into_any_element();
        }
        if section == "overview"
            || section == "workloads-overview"
            || section == "workloads/overview"
        {
            return self.render_overview_dashboard(cx).into_any_element();
        }
        if section == "port-forwarding" {
            return self.render_port_forwarding_body(cx).into_any_element();
        }
        if section == "helm-charts"
            || section == "helm-releases"
            || section == "helm/charts"
            || section == "helm/releases"
        {
            if let Some(error) = self.cluster_error.clone() {
                return div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(crate::empty_state::error_state(error))
                    .into_any_element();
            }
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
        crate::empty_state::empty_state(
            IconName::LayoutDashboard,
            "No resource section selected",
            "Select a resource section from the sidebar.",
        )
        .into_any_element()
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
                el.child(crate::empty_state::empty_state(
                    IconName::Network,
                    "No active port-forwards",
                    "Start one from a Service row or Pod container port.",
                ))
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
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .items_center()
                            .child(div().text_xs().font_weight(FontWeight::SEMIBOLD).child(
                                format!(
                                    "127.0.0.1:{} → {}:{}",
                                    fwd.local_port, fwd.name, fwd.remote_port
                                ),
                            ))
                            .child(Tag::secondary().small().child(fwd.namespace))
                            .child(row_status_tag(fwd.status)),
                    )
                    .child(
                        h_flex()
                            .ml_auto()
                            .flex_shrink_0()
                            .justify_end()
                            .gap_1()
                            .items_center()
                            .child(
                                Button::new(SharedString::from(format!("pf-copy-{idx}")))
                                    .xsmall()
                                    .ghost()
                                    .icon(IconName::Copy)
                                    .tooltip("Copy local forwarding address")
                                    .accessibility_label("Copy local forwarding address")
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            copy_addr.clone(),
                                        ));
                                    })),
                            )
                            .child(
                                Button::new(SharedString::from(format!("pf-stop-{idx}")))
                                    .xsmall()
                                    .ghost()
                                    .icon(IconName::Square)
                                    .tooltip("Stop port forwarding")
                                    .accessibility_label("Stop port forwarding")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.stop_pod_forward(fid.clone(), cx);
                                    })),
                            ),
                    )
            }))
    }

    fn filtered_resource_items<'a>(
        &'a self,
        kind: ResourceKind,
        cx: &Context<Self>,
    ) -> Vec<&'a ResourceItem> {
        let query = self
            .resource_filter_input
            .read(cx)
            .value()
            .trim()
            .to_lowercase();
        self.resource_page
            .as_ref()
            .filter(|page| page.kind == kind)
            .map(|page| {
                page.items
                    .iter()
                    .filter(|item| {
                        crate::resource_table::matches_filter(
                            item,
                            &query,
                            self.selected_status_filter.as_deref(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn render_resource_table_body(
        &mut self,
        kind: ResourceKind,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(table) = &self.resource_table {
            let lease = self
                .cluster_session
                .as_ref()
                .map(|session| session.session_id.clone())
                .unwrap_or_default();
            table.update(cx, |table, cx| table.sync_selection_lease(lease, cx));
        }
        let has_rows = !self.filtered_resource_items(kind, cx).is_empty();
        if !has_rows {
            if let Some(table) = &self.resource_table {
                table.update(cx, |table, cx| table.clear_selection(cx));
            }
            let query = self
                .resource_filter_input
                .read(cx)
                .value()
                .trim()
                .to_owned();
            let filtered = !query.is_empty() || self.selected_status_filter.is_some();
            let pending = self.cluster_loading || self.resource_pagination.loading;
            let more = self
                .resource_page
                .as_ref()
                .filter(|page| page.kind == kind)
                .is_some_and(|page| page.continue_token.is_some() || page.truncated);
            let state = if let Some(error) = self.cluster_error.clone() {
                crate::empty_state::error_state(error).into_any_element()
            } else if self.resource_pagination.stopped {
                crate::empty_state::empty_state(
                    IconName::Search,
                    "No loaded results",
                    "Loading paused. Retry from the notification or use Refresh.",
                )
                .into_any_element()
            } else if pending || more {
                crate::empty_state::empty_state(
                    IconName::Search,
                    zero_resource_title(filtered, true),
                    "Results will appear here as they become available.",
                )
                .into_any_element()
            } else if filtered {
                crate::empty_state::empty_state(
                    IconName::Search,
                    zero_resource_title(true, false),
                    "Try a different search, change the status filter, or select another namespace.",
                ).into_any_element()
            } else {
                crate::empty_state::empty_state(
                    IconName::LayoutDashboard,
                    zero_resource_title(false, false),
                    "There are no resources in the selected namespace scope.",
                )
                .into_any_element()
            };
            return v_flex()
                .size_full()
                .min_h_0()
                .min_w_0()
                .items_center()
                .justify_center()
                .child(state)
                .into_any_element();
        }
        let owner = cx.entity().downgrade();
        let table = self
            .resource_table
            .get_or_insert_with(|| {
                cx.new(|cx| crate::resource_table::ResourceTable::new(owner, cx))
            })
            .clone();
        let data = crate::resource_table::TableData {
            page: self.resource_page.clone(),
            scope: crate::resource_scroll::ResourceScrollScope {
                cluster_id: self
                    .cluster_session
                    .as_ref()
                    .map(|session| session.cluster_id.clone())
                    .unwrap_or_default(),
                kind,
                namespace: self.selected_namespace.clone(),
                query: self
                    .resource_filter_input
                    .read(cx)
                    .value()
                    .trim()
                    .to_lowercase(),
                status: self.selected_status_filter.clone(),
                page_token: None,
            },
        };
        table.update(cx, |table, cx| table.sync(data, cx));
        table
            .cached(StyleRefinement::default().size_full())
            .into_any_element()
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
                                .child(EditorSurface::new(
                                    &self.helm_values_editor,
                                    &self.preferences_draft.editor,
                                )),
                        ),
                )
            })
            .when(
                self.helm_page.is_some()
                    && items.is_empty()
                    && !self.cluster_loading
                    && self.cluster_error.is_none(),
                |el| {
                    el.child(crate::empty_state::empty_state(
                        IconName::Inbox,
                        "No Helm items found",
                        "No charts or releases are available in the current scope.",
                    ))
                },
            )
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
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_sm()
                                            .truncate()
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
                                    .truncate()
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
                            .ml_auto()
                            .flex_shrink_0()
                            .justify_end()
                            .gap_1p5()
                            .when(view == HelmView::Charts, |el| {
                                el.child(
                                    Button::new(SharedString::from(format!("helm-inst-{idx}")))
                                        .xsmall()
                                        .ghost()
                                        .icon(IconName::Plus)
                                        .tooltip("Install Helm chart")
                                        .accessibility_label("Install Helm chart")
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
                                    .ghost()
                                    .icon(IconName::Eye)
                                    .tooltip("Inspect Helm details")
                                    .accessibility_label("Inspect Helm details")
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
        let instances = self.custom_instances_page.clone();

        v_flex()
            .flex_1()
            .w_full()
            .h_full()
            .min_h_0()
            .p_4()
            .gap_4()
            .overflow_y_scrollbar()
            .when(self.cluster_loading, |el| {
                el.child(div().text_sm().child("Loading custom resources…"))
            })
            .when_some(self.cluster_error.clone(), |el, error| {
                el.child(crate::empty_state::error_state(error))
            })
            .when_some(instances, |el, page: CustomResourcePage| {
                let el = el
                    .when(page.items.is_empty(), |el| {
                        el.child(
                            crate::empty_state::empty_state(IconName::FileText, "No custom resources found", "There are no instances of this resource in the selected namespace scope."),
                        )
                    })
                    .when(page.continue_token.is_some() || page.truncated, |el| {
                        el.child(
                            div()
                                .text_xs()
                                .child("Showing a partial page of custom resources."),
                        )
                    });
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
                                                        .flex_1()
                                                        .min_w_0()
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
                                                        .ml_auto()
                                                        .flex_shrink_0()
                                                        .justify_end()
                                                        .gap_1()
                                                        .child(
                                                            Button::new(SharedString::from(
                                                                format!("managed-sa-insp-{uidx}"),
                                                            ))
                                                            .xsmall()
                                                            .ghost()
                                                            .icon(IconName::Eye)
                                                            .tooltip("Inspect ServiceAccount")
                                                            .accessibility_label("Inspect ServiceAccount")
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
                                                            .ghost()
                                                            .icon(IconName::Delete)
                                                            .tooltip("Preview ServiceAccount deletion plan")
                                                            .accessibility_label("Preview ServiceAccount deletion plan")
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

pub(crate) fn row_status_tag(status: &str) -> Tag {
    let normalized = status.to_ascii_lowercase();
    let tag = match normalized.as_str() {
        "running" => Tag::color(ColorName::Green),
        "pending" | "waiting" | "schedulingdisabled" => Tag::color(ColorName::Amber),
        "succeeded" | "success" | "complete" | "completed" | "ready" => {
            Tag::color(ColorName::Emerald)
        }
        "failed" | "error" | "crash" | "replicafailure" => Tag::color(ColorName::Red),
        "terminating" | "warning" | "notready" | "not ready" => Tag::color(ColorName::Orange),
        "active" | "available" | "progressing" => Tag::color(ColorName::Blue),
        "suspended" | "paused" => Tag::color(ColorName::Violet),
        "lost" => Tag::color(ColorName::Rose),
        _ if normalized.contains("fail") || normalized.contains("error") => {
            Tag::color(ColorName::Red)
        }
        _ if normalized.contains("pending") || normalized.contains("wait") => {
            Tag::color(ColorName::Amber)
        }
        _ => Tag::secondary(),
    };
    tag.small().child(status.to_owned())
}

pub(crate) fn default_yaml_template(kind: ResourceKind, namespace: &str) -> String {
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
    fn pod_columns_show_containers_and_usage_before_restarts() {
        let keys: Vec<_> = columns_for_kind(ResourceKind::Pods)
            .iter()
            .map(|column| column.key)
            .collect();
        let containers = keys.iter().position(|key| *key == "containers").unwrap();
        let cpu = keys.iter().position(|key| *key == "cpu").unwrap();
        let memory = keys.iter().position(|key| *key == "memory").unwrap();
        let restarts = keys.iter().position(|key| *key == "restarts").unwrap();
        assert!(containers < cpu && cpu < memory && memory < restarts);
        assert!(!keys.contains(&"ready"));
    }

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

#[cfg(test)]
mod empty_result_tests {
    #[test]
    fn zero_results_distinguish_incomplete_scans_from_empty_inventory() {
        assert_eq!(
            super::zero_resource_title(true, true),
            "Searching resources…"
        );
        assert_eq!(
            super::zero_resource_title(false, true),
            "Loading resources…"
        );
        assert_eq!(
            super::zero_resource_title(true, false),
            "No matching resources"
        );
        assert_eq!(
            super::zero_resource_title(false, false),
            "No resources found"
        );
    }
}
