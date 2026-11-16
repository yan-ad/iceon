//! Shared table status policy. Counts and configuration are not health observations.
use super::{ResourceItem, ResourceKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusPresentation {
    None,
    Status,
    Conditions,
    Severity,
}

pub fn presentation(kind: ResourceKind) -> StatusPresentation {
    use ResourceKind::*;
    match kind {
        Nodes
        | Namespaces
        | Pods
        | PersistentVolumes
        | PersistentVolumeClaims
        | Jobs
        | CronJobs => StatusPresentation::Status,
        Deployments
        | HorizontalPodAutoscalers
        | VerticalPodAutoscalers
        | PodDisruptionBudgets
        | CustomResourceDefinitions => StatusPresentation::Conditions,
        Events => StatusPresentation::Severity,
        DaemonSets
        | StatefulSets
        | ReplicaSets
        | ReplicationControllers
        | ConfigMaps
        | Secrets
        | ResourceQuotas
        | LimitRanges
        | PriorityClasses
        | RuntimeClasses
        | Leases
        | Services
        | Endpoints
        | Ingresses
        | IngressClasses
        | NetworkPolicies
        | EndpointSlices
        | MutatingWebhookConfigurations
        | ValidatingWebhookConfigurations
        // No supported condition-type vocabulary; type checking is not lifecycle status.
        | ValidatingAdmissionPolicies
        | ValidatingAdmissionPolicyBindings
        | PodSecurityPolicies
        | StorageClasses
        | ServiceAccounts
        | Roles
        | RoleBindings
        | ClusterRoles
        | ClusterRoleBindings => StatusPresentation::None,
    }
}

pub const fn filters(kind: ResourceKind) -> &'static [&'static str] {
    use ResourceKind::*;
    match kind {
        Nodes => &["Ready", "Not ready", "Unknown", "Not reported"],
        Namespaces => &["Active", "Terminating", "Unknown", "Not reported"],
        Pods => &[
            "Pending",
            "Running",
            "Succeeded",
            "Failed",
            "Terminating",
            "Unknown",
            "Not reported",
        ],
        PersistentVolumes => &[
            "Pending",
            "Available",
            "Bound",
            "Released",
            "Failed",
            "Unknown",
            "Not reported",
        ],
        PersistentVolumeClaims => &["Pending", "Bound", "Lost", "Unknown", "Not reported"],
        Jobs => &[
            "Failed",
            "Completed",
            "Suspended",
            "Running",
            "Not reported",
        ],
        CronJobs => &["Suspended", "Enabled", "Not reported"],
        Deployments => &[
            "Available",
            "Unavailable",
            "Progressing",
            "ReplicaFailure",
            "Paused",
            "Unknown",
            "Not reported",
        ],
        HorizontalPodAutoscalers => &[
            "AbleToScale",
            "ScalingActive",
            "ScalingLimited",
            "Not reported",
        ],
        VerticalPodAutoscalers => &[
            "RecommendationProvided",
            "LowConfidence",
            "NoPodsMatched",
            "FetchingHistory",
            "Not reported",
        ],
        PodDisruptionBudgets => &["DisruptionAllowed", "Not reported"],
        CustomResourceDefinitions => &[
            "Established",
            "NamesAccepted",
            "Terminating",
            "Not reported",
        ],
        Events => &["Normal", "Warning", "Unknown", "Not reported"],
        DaemonSets
        | StatefulSets
        | ReplicaSets
        | ReplicationControllers
        | ConfigMaps
        | Secrets
        | ResourceQuotas
        | LimitRanges
        | PriorityClasses
        | RuntimeClasses
        | Leases
        | Services
        | Endpoints
        | Ingresses
        | IngressClasses
        | NetworkPolicies
        | EndpointSlices
        | MutatingWebhookConfigurations
        | ValidatingWebhookConfigurations
        | ValidatingAdmissionPolicyBindings
        | PodSecurityPolicies
        | StorageClasses
        | ServiceAccounts
        | Roles
        | RoleBindings
        | ClusterRoles
        | ClusterRoleBindings
        | ValidatingAdmissionPolicies => &[],
    }
}

/// Match complete labels only, never substrings (e.g. Ready != Not ready).
pub fn matches_status(item: &ResourceItem, filter: &str) -> bool {
    !filter.is_empty()
        && (item.status.eq_ignore_ascii_case(filter)
            || item.columns.get("conditions").is_some_and(|conditions| {
                conditions
                    .split(',')
                    .any(|token| token.trim().eq_ignore_ascii_case(filter))
            }))
}

pub(super) fn condition_types(kind: ResourceKind) -> &'static [&'static str] {
    use ResourceKind::*;
    match kind {
        Deployments => &["Available", "Progressing", "ReplicaFailure"],
        HorizontalPodAutoscalers => &["AbleToScale", "ScalingActive", "ScalingLimited"],
        VerticalPodAutoscalers => &[
            "RecommendationProvided",
            "LowConfidence",
            "NoPodsMatched",
            "FetchingHistory",
        ],
        PodDisruptionBudgets => &["DisruptionAllowed"],
        CustomResourceDefinitions => &["Established", "NamesAccepted", "Terminating"],
        _ => &[],
    }
}
