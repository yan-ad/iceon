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

/// Decode the backwards-compatible scalar wire field into an exact OR selection.
/// `|` is reserved for selections; condition summaries use commas, not pipes.
/// Invalid/oversized selections fail closed. Case and ordering are canonicalized
/// so equivalent UI selections have the same request and scroll identity.
pub fn decode_status_selection(filter: &str) -> Option<Vec<String>> {
    if filter.is_empty() || filter.len() > 256 || filter.chars().any(char::is_control) {
        return None;
    }
    let mut labels = Vec::new();
    for label in filter.split('|') {
        if labels.len() >= 32 || label.is_empty() || label.trim() != label {
            return None;
        }
        labels.push(label.to_ascii_lowercase());
    }
    labels.sort();
    labels.dedup();
    Some(labels)
}

/// Encode a nonempty selection, or return None for All Status (or invalid input).
pub fn encode_status_selection(labels: &[String]) -> Option<String> {
    if labels.is_empty() || labels.iter().any(|label| label.contains('|')) {
        return None;
    }
    decode_status_selection(&labels.join("|")).map(|labels| labels.join("|"))
}

/// Match complete labels with OR semantics, never substrings (Ready != Not ready).
pub fn matches_status(item: &ResourceItem, filter: &str) -> bool {
    decode_status_selection(filter).is_some_and(|labels| {
        labels.iter().any(|label| {
            item.status.eq_ignore_ascii_case(label)
                || item.columns.get("conditions").is_some_and(|conditions| {
                    conditions
                        .split(',')
                        .any(|token| token.trim().eq_ignore_ascii_case(label))
                })
        })
    })
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

#[cfg(test)]
mod selection_tests {
    use super::*;

    #[test]
    fn selections_are_canonical_and_scalar_compatible() {
        let selection = |labels: &[&str]| {
            encode_status_selection(&labels.iter().map(|s| (*s).into()).collect::<Vec<_>>())
        };
        assert_eq!(selection(&[]), None);
        assert_eq!(selection(&["Running"]), Some("running".into()));
        assert_eq!(
            selection(&["Running", "Pending", "RUNNING"]),
            Some("pending|running".into())
        );
        assert_eq!(
            selection(&["Pending", "Running"]),
            selection(&["Running", "Pending"])
        );
        assert_eq!(
            selection(&["Available: False", "Progressing: Unknown"]),
            Some("available: false|progressing: unknown".into())
        );
        assert_eq!(selection(&["Running|Pending"]), None);
        assert_eq!(decode_status_selection(&vec!["x"; 33].join("|")), None);
        assert_eq!(decode_status_selection(""), None);
    }
}
