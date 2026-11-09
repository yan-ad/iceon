use super::{ConnectionError, NodeInfo, ResourceItem, ResourceKind};
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) fn namespaced(kind: ResourceKind) -> bool {
    matches!(
        kind,
        ResourceKind::Pods
            | ResourceKind::Deployments
            | ResourceKind::DaemonSets
            | ResourceKind::StatefulSets
            | ResourceKind::ReplicaSets
            | ResourceKind::ReplicationControllers
            | ResourceKind::Jobs
            | ResourceKind::CronJobs
            | ResourceKind::ConfigMaps
            | ResourceKind::Secrets
            | ResourceKind::ResourceQuotas
            | ResourceKind::LimitRanges
            | ResourceKind::HorizontalPodAutoscalers
            | ResourceKind::VerticalPodAutoscalers
            | ResourceKind::Leases
            | ResourceKind::PodDisruptionBudgets
            | ResourceKind::Services
            | ResourceKind::Endpoints
            | ResourceKind::EndpointSlices
            | ResourceKind::Ingresses
            | ResourceKind::NetworkPolicies
            | ResourceKind::PersistentVolumeClaims
            | ResourceKind::Events
            | ResourceKind::ServiceAccounts
            | ResourceKind::Roles
            | ResourceKind::RoleBindings
    )
}
fn count(v: &Value, c: &mut BTreeMap<String, String>, key: &str, path: &str) {
    if let Some(len) = v.pointer(path).and_then(|v| match v {
        Value::Object(map) => Some(map.len()),
        Value::Array(array) => Some(array.len()),
        _ => None,
    }) {
        insert(c, key, len.to_string());
    }
}
fn scalar(v: &Value, c: &mut BTreeMap<String, String>, key: &str, path: &str) {
    if let Some(value) = v.pointer(path) {
        let text = match value {
            Value::String(s) => bounded(s),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            _ => return,
        };
        insert(c, key, text);
    }
}
fn strings(v: &Value, c: &mut BTreeMap<String, String>, key: &str, path: &str) {
    if let Some(values) = v.pointer(path).and_then(Value::as_array) {
        insert(
            c,
            key,
            bounded(
                &values
                    .iter()
                    .filter_map(Value::as_str)
                    .take(12)
                    .collect::<Vec<_>>()
                    .join(","),
            ),
        );
    }
}
fn quantities(v: &Value, c: &mut BTreeMap<String, String>, key: &str, path: &str) {
    if let Some(value) = v.pointer(path).and_then(Value::as_str)
        && parse_quantity(value).is_some()
    {
        insert(c, key, bounded(value));
    }
}
fn number(v: &Value, c: &mut BTreeMap<String, String>, key: &str, path: &str) {
    if let Some(value) = v.pointer(path).and_then(Value::as_u64) {
        insert(c, key, value.to_string());
    }
}
fn table_status(kind: ResourceKind, v: &Value, c: &mut BTreeMap<String, String>) -> String {
    use ResourceKind::*;
    match kind {
        ResourceQuotas => {
            for (key, resource) in [
                ("cpu", "requests.cpu"),
                ("memory", "requests.memory"),
                ("pods", "pods"),
                ("cpuLimit", "limits.cpu"),
                ("memoryLimit", "limits.memory"),
                ("storage", "requests.storage"),
            ] {
                quantities(
                    v,
                    c,
                    &format!("{key}Hard"),
                    &format!("/status/hard/{resource}"),
                );
                quantities(
                    v,
                    c,
                    &format!("{key}Used"),
                    &format!("/status/used/{resource}"),
                );
            }
        }
        LimitRanges => {
            count(v, c, "limits", "/spec/limits");
            if let Some(limits) = v.pointer("/spec/limits").and_then(Value::as_array) {
                let types = limits
                    .iter()
                    .take(12)
                    .filter_map(|l| l.get("type").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join(",");
                insert(c, "types", bounded(&types));
                // Only Container defaults are useful without exposing arbitrary resource keys.
                if let Some(limit) = limits
                    .iter()
                    .find(|l| l.get("type").and_then(Value::as_str) == Some("Container"))
                {
                    for (key, path) in [
                        ("minCpu", "/min/cpu"),
                        ("maxCpu", "/max/cpu"),
                        ("minMemory", "/min/memory"),
                        ("maxMemory", "/max/memory"),
                        ("defaultCpu", "/default/cpu"),
                        ("defaultMemory", "/default/memory"),
                        ("defaultRequestCpu", "/defaultRequest/cpu"),
                        ("defaultRequestMemory", "/defaultRequest/memory"),
                    ] {
                        quantities(limit, c, key, path);
                    }
                }
            }
        }
        HorizontalPodAutoscalers => {
            scalar(v, c, "target", "/spec/scaleTargetRef/name");
            scalar(v, c, "targetKind", "/spec/scaleTargetRef/kind");
            scalar(v, c, "min", "/spec/minReplicas");
            scalar(v, c, "max", "/spec/maxReplicas");
            number(v, c, "current", "/status/currentReplicas");
            number(v, c, "desired", "/status/desiredReplicas");
            count(v, c, "metrics", "/spec/metrics");
        }
        VerticalPodAutoscalers => {
            scalar(v, c, "target", "/spec/targetRef/name");
            scalar(v, c, "targetKind", "/spec/targetRef/kind");
            scalar(v, c, "updateMode", "/spec/updatePolicy/updateMode");
            count(
                v,
                c,
                "recommendations",
                "/status/recommendation/containerRecommendations",
            );
        }
        PodDisruptionBudgets => {
            for (key, path) in [
                ("minAvailable", "/spec/minAvailable"),
                ("maxUnavailable", "/spec/maxUnavailable"),
                ("allowedDisruptions", "/status/disruptionsAllowed"),
                ("currentHealthy", "/status/currentHealthy"),
                ("desiredHealthy", "/status/desiredHealthy"),
                ("expectedPods", "/status/expectedPods"),
            ] {
                scalar(v, c, key, path);
            }
        }
        PriorityClasses => {
            scalar(v, c, "value", "/value");
            scalar(v, c, "globalDefault", "/globalDefault");
            scalar(v, c, "preemptionPolicy", "/preemptionPolicy");
        }
        RuntimeClasses => {
            scalar(v, c, "handler", "/handler");
            quantities(v, c, "overheadCpu", "/overhead/podFixed/cpu");
            quantities(v, c, "overheadMemory", "/overhead/podFixed/memory");
            count(v, c, "nodeSelectors", "/scheduling/nodeSelector");
            count(v, c, "tolerations", "/scheduling/tolerations");
        }
        Leases => {
            scalar(v, c, "holder", "/spec/holderIdentity");
            if let Some(renewed) = v
                .pointer("/spec/renewTime")
                .and_then(Value::as_str)
                .and_then(timestamp)
            {
                insert(c, "renewed", renewed);
            }
            number(v, c, "durationSeconds", "/spec/leaseDurationSeconds");
            number(v, c, "transitions", "/spec/leaseTransitions");
        }
        Services => return service_status(v, c),
        Endpoints => {
            count(v, c, "subsets", "/subsets");
            if let Some(subsets) = v.pointer("/subsets").and_then(Value::as_array) {
                for (key, field) in [
                    ("readyAddresses", "addresses"),
                    ("notReadyAddresses", "notReadyAddresses"),
                    ("ports", "ports"),
                ] {
                    let total = subsets
                        .iter()
                        .filter_map(|s| s.get(field).and_then(Value::as_array))
                        .map(Vec::len)
                        .sum::<usize>();
                    insert(c, key, total.to_string());
                }
            }
        }
        EndpointSlices => {
            scalar(v, c, "addressType", "/addressType");
            count(v, c, "endpoints", "/endpoints");
            count(v, c, "ports", "/ports");
            if let Some(endpoints) = v.pointer("/endpoints").and_then(Value::as_array) {
                insert(
                    c,
                    "ready",
                    endpoints
                        .iter()
                        .filter(|e| {
                            e.pointer("/conditions/ready").and_then(Value::as_bool) == Some(true)
                        })
                        .count()
                        .to_string(),
                );
            }
        }
        Ingresses => {
            scalar(v, c, "class", "/spec/ingressClassName");
            count(v, c, "rules", "/spec/rules");
            count(v, c, "tls", "/spec/tls");
        }
        IngressClasses => {
            scalar(v, c, "controller", "/spec/controller");
            scalar(v, c, "parametersKind", "/spec/parameters/kind");
            scalar(v, c, "parametersName", "/spec/parameters/name");
        }
        NetworkPolicies => {
            strings(v, c, "policyTypes", "/spec/policyTypes");
            count(v, c, "ingressRules", "/spec/ingress");
            count(v, c, "egressRules", "/spec/egress");
            count(v, c, "matchLabels", "/spec/podSelector/matchLabels");
            count(
                v,
                c,
                "matchExpressions",
                "/spec/podSelector/matchExpressions",
            );
        }
        PodSecurityPolicies => {
            for (key, path) in [
                ("privileged", "/spec/privileged"),
                ("hostNetwork", "/spec/hostNetwork"),
                ("hostPID", "/spec/hostPID"),
                ("hostIPC", "/spec/hostIPC"),
                ("readOnlyRootFilesystem", "/spec/readOnlyRootFilesystem"),
                ("runAsUser", "/spec/runAsUser/rule"),
                ("seLinux", "/spec/seLinux/rule"),
            ] {
                scalar(v, c, key, path);
            }
        }
        PersistentVolumes => {
            scalar(v, c, "capacity", "/spec/capacity/storage");
            scalar(v, c, "reclaim", "/spec/persistentVolumeReclaimPolicy");
            scalar(v, c, "storageClass", "/spec/storageClassName");
            scalar(v, c, "volumeMode", "/spec/volumeMode");
            strings(v, c, "accessModes", "/spec/accessModes");
            scalar(v, c, "claim", "/spec/claimRef/name");
            scalar(v, c, "claimNamespace", "/spec/claimRef/namespace");
        }
        StorageClasses => {
            scalar(v, c, "provisioner", "/provisioner");
            scalar(v, c, "reclaim", "/reclaimPolicy");
            scalar(v, c, "bindingMode", "/volumeBindingMode");
            scalar(v, c, "allowExpansion", "/allowVolumeExpansion");
        }
        ServiceAccounts => {
            count(v, c, "secrets", "/secrets");
            count(v, c, "imagePullSecrets", "/imagePullSecrets");
            scalar(v, c, "automountToken", "/automountServiceAccountToken");
        }
        MutatingWebhookConfigurations | ValidatingWebhookConfigurations => {
            count(v, c, "webhooks", "/webhooks");
            if let Some(webhooks) = v.pointer("/webhooks").and_then(Value::as_array) {
                for (key, field) in [
                    ("failurePolicies", "failurePolicy"),
                    ("sideEffects", "sideEffects"),
                    ("matchPolicies", "matchPolicy"),
                ] {
                    let values = webhooks
                        .iter()
                        .take(12)
                        .filter_map(|w| w.get(field).and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join(",");
                    insert(c, key, bounded(&values));
                }
            }
        }
        ValidatingAdmissionPolicies => {
            scalar(v, c, "failurePolicy", "/spec/failurePolicy");
            count(v, c, "validations", "/spec/validations");
            count(v, c, "matchConditions", "/spec/matchConditions");
            scalar(v, c, "paramKind", "/spec/paramKind/kind");
        }
        ValidatingAdmissionPolicyBindings => {
            scalar(v, c, "policy", "/spec/policyName");
            strings(v, c, "validationActions", "/spec/validationActions");
            scalar(v, c, "paramName", "/spec/paramRef/name");
            scalar(v, c, "paramNamespace", "/spec/paramRef/namespace");
        }
        CustomResourceDefinitions => {
            scalar(v, c, "group", "/spec/group");
            scalar(v, c, "scope", "/spec/scope");
            scalar(v, c, "kind", "/spec/names/kind");
            scalar(v, c, "plural", "/spec/names/plural");
            if let Some(versions) = v.pointer("/spec/versions").and_then(Value::as_array) {
                let versions = versions
                    .iter()
                    .take(12)
                    .filter_map(|v| v.get("name").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join(",");
                insert(c, "versions", bounded(&versions));
            }
            strings(v, c, "shortNames", "/spec/names/shortNames");
        }
        _ => return generic_status(v, c),
    }
    v.pointer("/status/phase")
        .and_then(Value::as_str)
        .map_or_else(|| "Unknown".into(), bounded)
}

fn rbac_status(v: &Value, c: &mut BTreeMap<String, String>) -> String {
    for (key, path) in [("rules", "/rules"), ("subjects", "/subjects")] {
        if let Some(values) = v.pointer(path).and_then(Value::as_array) {
            insert(c, key, values.len().to_string());
        }
    }
    if let Some(name) = v.pointer("/roleRef/name").and_then(Value::as_str) {
        insert(c, "roleRef", bounded(name));
    }
    scalar(v, c, "roleKind", "/roleRef/kind");
    "Unknown".into()
}

fn identity(meta: &Value, field: &str) -> Result<Option<String>, ConnectionError> {
    match meta.get(field) {
        None => Ok(None),
        Some(Value::String(value))
            if !value.is_empty()
                && value.len() <= 256
                && value.bytes().all(|byte| byte.is_ascii_graphic()) =>
        {
            Ok(Some(value.clone()))
        }
        _ => Err(ConnectionError::InvalidResponse),
    }
}

pub(super) fn project(kind: ResourceKind, value: Value) -> Result<ResourceItem, ConnectionError> {
    let meta = value
        .get("metadata")
        .ok_or(ConnectionError::InvalidResponse)?;
    let name = meta
        .get("name")
        .and_then(Value::as_str)
        .ok_or(ConnectionError::InvalidResponse)?
        .to_owned();
    if !super::details::valid_resource_name(kind, &name) {
        return Err(ConnectionError::InvalidResponse);
    }
    let uid = identity(meta, "uid")?;
    let resource_version = identity(meta, "resourceVersion")?;
    let namespace = meta.get("namespace").and_then(Value::as_str).map(bounded);
    if namespace
        .as_deref()
        .is_some_and(|v| !super::transport::valid_namespace(v))
        || (namespaced(kind) && namespace.is_none())
    {
        return Err(ConnectionError::InvalidResponse);
    }
    let created_at = meta
        .get("creationTimestamp")
        .and_then(Value::as_str)
        .and_then(timestamp);
    let mut columns = BTreeMap::new();
    let status = match kind {
        ResourceKind::Nodes => project_node(&value, &mut columns)?,
        ResourceKind::Namespaces => {
            count(&value, &mut columns, "labels", "/metadata/labels");
            match value.pointer("/status/phase").and_then(Value::as_str) {
                Some("Active") => "Active",
                Some("Terminating") => "Terminating",
                _ => "Unknown",
            }
            .into()
        }
        ResourceKind::Pods => pod_status(&value, &mut columns),
        ResourceKind::Deployments
        | ResourceKind::DaemonSets
        | ResourceKind::StatefulSets
        | ResourceKind::ReplicaSets
        | ResourceKind::ReplicationControllers => workload_status(kind, &value, &mut columns),
        ResourceKind::Jobs => job_status(&value, &mut columns),
        ResourceKind::CronJobs => cron_job_status(&value, &mut columns),
        ResourceKind::Services => service_status(&value, &mut columns),
        ResourceKind::PersistentVolumeClaims => pvc_status(&value, &mut columns),
        ResourceKind::Events => event_status(&value, &mut columns),
        ResourceKind::Roles
        | ResourceKind::ClusterRoles
        | ResourceKind::RoleBindings
        | ResourceKind::ClusterRoleBindings => rbac_status(&value, &mut columns),
        ResourceKind::Secrets | ResourceKind::ConfigMaps => "Metadata only".into(),
        _ => table_status(kind, &value, &mut columns),
    };
    if kind == ResourceKind::Pods
        && let Some(owner) = super::details::controlled_owner(&value, namespace.as_deref())
    {
        insert(&mut columns, "controlledBy", owner.name);
    }
    Ok(ResourceItem {
        name,
        uid,
        resource_version,
        namespace,
        created_at,
        status: bounded(&status),
        columns,
        node: if kind == ResourceKind::Nodes {
            Some(node_info(&value)?)
        } else {
            None
        },
    })
}

fn project_node(
    v: &Value,
    columns: &mut BTreeMap<String, String>,
) -> Result<String, ConnectionError> {
    let info = node_info(v)?;
    let roles = bounded(&info.roles.join(","));
    insert(columns, "roles", roles);
    if let Some(version) = &info.version {
        insert(columns, "version", version.clone());
    }
    if v.pointer("/spec/unschedulable").and_then(Value::as_bool) == Some(true) {
        insert(columns, "schedulable", "false".into());
    }
    Ok(match v
        .pointer("/status/conditions")
        .and_then(Value::as_array)
        .and_then(|cs| {
            cs.iter()
                .find(|c| c.get("type").and_then(Value::as_str) == Some("Ready"))
        })
        .and_then(|c| c.get("status"))
        .and_then(Value::as_str)
    {
        Some("True") => "Ready",
        Some("False") => "Not ready",
        _ => "Unknown",
    }
    .into())
}
fn node_info(v: &Value) -> Result<NodeInfo, ConnectionError> {
    super::transport::nodes::project(v)
}
fn generic_status(v: &Value, c: &mut BTreeMap<String, String>) -> String {
    if let Some(p) = v.pointer("/status/phase").and_then(Value::as_str) {
        insert(c, "phase", bounded(p));
        return bounded(p);
    }
    "Unknown".into()
}
fn suspend(v: &Value, c: &mut BTreeMap<String, String>) {
    if let Some(value) = v.pointer("/spec/suspend").and_then(Value::as_bool) {
        insert(c, "suspend", value.to_string());
    }
}
fn cron_job_status(v: &Value, c: &mut BTreeMap<String, String>) -> String {
    for (key, path) in [
        ("schedule", "/spec/schedule"),
        ("timeZone", "/spec/timeZone"),
    ] {
        if let Some(value) = v.pointer(path).and_then(Value::as_str) {
            insert(c, key, bounded(value));
        }
    }
    suspend(v, c);
    if let Some(active) = v.pointer("/status/active").and_then(Value::as_array) {
        insert(c, "active", active.len().to_string());
    }
    if let Some(last_schedule) = v
        .pointer("/status/lastScheduleTime")
        .and_then(Value::as_str)
        .and_then(timestamp)
    {
        insert(c, "lastSchedule", last_schedule);
    }
    "Unknown".into()
}
fn pod_status(v: &Value, c: &mut BTreeMap<String, String>) -> String {
    pod_health(v, c);
    let cs = v
        .pointer("/status/containerStatuses")
        .and_then(Value::as_array);
    let (ready, restarts) = (
        cs.map(|x| {
            x.iter()
                .filter(|c| c.get("ready").and_then(Value::as_bool) == Some(true))
                .count()
        })
        .unwrap_or(0),
        cs.map(|x| {
            x.iter()
                .map(|c| c.get("restartCount").and_then(Value::as_u64).unwrap_or(0))
                .fold(0u64, u64::saturating_add)
        })
        .unwrap_or(0),
    );
    if let Some(total) = v
        .pointer("/spec/containers")
        .and_then(Value::as_array)
        .or(cs)
        .map(Vec::len)
    {
        insert(c, "ready", format!("{ready}/{total}"));
    }
    insert(c, "restarts", restarts.to_string());
    if let Some(n) = v.pointer("/spec/nodeName").and_then(Value::as_str) {
        insert(c, "node", bounded(n));
    }
    if let Some(q) = v.pointer("/status/qosClass").and_then(Value::as_str) {
        insert(c, "qos", bounded(q));
    }
    if v.pointer("/metadata/deletionTimestamp")
        .is_some_and(|value| !value.is_null())
    {
        return "Terminating".into();
    }
    v.pointer("/status/phase")
        .and_then(Value::as_str)
        .map_or_else(|| "Unknown".into(), bounded)
}
// Additive row facts shared by snapshot and LIST/WATCH projection. Warning codes
// are local classifications, not Event warnings or server messages (which may
// contain credentials). Missing readiness never becomes a healthy check mark.
fn pod_health(v: &Value, c: &mut BTreeMap<String, String>) {
    let conditions = v.pointer("/status/conditions").and_then(Value::as_array);
    let condition = |name: &str| {
        conditions
            .and_then(|items| {
                items
                    .iter()
                    .find(|item| item.get("type").and_then(Value::as_str) == Some(name))
            })
            .and_then(|item| item.get("status"))
            .and_then(Value::as_str)
    };
    let ready = match condition("Ready") {
        Some("True") => "true",
        Some("False") => "false",
        _ => "unknown",
    };
    insert(c, "podReady", ready.into());
    let terminating = v
        .pointer("/metadata/deletionTimestamp")
        .is_some_and(|value| !value.is_null());
    insert(c, "terminating", terminating.to_string());
    if let Some(time) = v
        .pointer("/metadata/deletionTimestamp")
        .and_then(Value::as_str)
        .and_then(timestamp)
    {
        insert(c, "deletionTimestamp", time);
    }

    let statuses = || {
        [
            "/status/containerStatuses",
            "/status/initContainerStatuses",
            "/status/ephemeralContainerStatuses",
        ]
        .into_iter()
        .filter_map(|path| v.pointer(path).and_then(Value::as_array))
        .flatten()
    };
    let waiting = statuses().any(|status| {
        status
            .pointer("/state/waiting")
            .is_some_and(Value::is_object)
    });
    let failed = statuses().any(|status| {
        status
            .pointer("/state/terminated/exitCode")
            .and_then(Value::as_i64)
            .is_some_and(|code| code != 0)
    });
    let gate_unready = v
        .pointer("/spec/readinessGates")
        .and_then(Value::as_array)
        .is_some_and(|gates| {
            gates.iter().any(|gate| {
                gate.get("conditionType")
                    .and_then(Value::as_str)
                    .is_none_or(|name| condition(name) != Some("True"))
            })
        });
    let unready = ["/spec/containers", "/spec/initContainers"]
        .into_iter()
        .filter_map(|path| {
            v.pointer(path)
                .and_then(Value::as_array)
                .map(|items| (path, items))
        })
        .any(|(path, items)| {
            items.iter().any(|container| {
                if path == "/spec/initContainers"
                    && container.get("restartPolicy").and_then(Value::as_str) != Some("Always")
                {
                    return false;
                }
                let name = container.get("name").and_then(Value::as_str);
                name.is_none()
                    || !statuses().any(|status| {
                        status.get("name").and_then(Value::as_str) == name
                            && (status.get("ready").and_then(Value::as_bool) == Some(true)
                                || status
                                    .pointer("/state/terminated/exitCode")
                                    .and_then(Value::as_i64)
                                    == Some(0))
                    })
            })
        });
    let phase = v.pointer("/status/phase").and_then(Value::as_str);
    let warning = match phase {
        Some("Succeeded") => None,
        Some("Failed") => Some("PodFailed"),
        _ if failed => Some("ContainerFailed"),
        _ if waiting => Some("ContainerWaiting"),
        Some("Pending") => Some("PodPending"),
        Some("Running") if unready => Some("ContainersNotReady"),
        Some("Running") if gate_unready => Some("ReadinessGateNotReady"),
        Some("Running") if ready == "false" => Some("PodNotReady"),
        Some("Unknown") => Some("PodUnknown"),
        _ => None,
    };
    let health = if terminating {
        "terminating"
    } else if warning.is_some() {
        "warning"
    } else if phase == Some("Succeeded") {
        "completed"
    } else if phase == Some("Running") && ready == "true" {
        "healthy"
    } else {
        "unknown"
    };
    insert(c, "health", health.into());
    if let Some(warning) = warning {
        insert(c, "warning", warning.into());
    }
}
fn workload_status(kind: ResourceKind, v: &Value, c: &mut BTreeMap<String, String>) -> String {
    let fields = if kind == ResourceKind::DaemonSets {
        [
            ("desired", "/status/desiredNumberScheduled"),
            ("scheduled", "/status/currentNumberScheduled"),
            ("ready", "/status/numberReady"),
            ("updated", "/status/updatedNumberScheduled"),
            ("available", "/status/numberAvailable"),
        ]
    } else {
        [
            ("desired", "/spec/replicas"),
            ("replicas", "/status/replicas"),
            ("ready", "/status/readyReplicas"),
            ("updated", "/status/updatedReplicas"),
            ("available", "/status/availableReplicas"),
        ]
    };
    for (key, path) in fields {
        number(v, c, key, path);
    }
    v.pointer("/status/conditions")
        .and_then(Value::as_array)
        .and_then(|a| {
            a.iter()
                .find(|x| x.get("type").and_then(Value::as_str) == Some("Available"))
        })
        .and_then(|x| x.get("status"))
        .and_then(Value::as_str)
        .unwrap_or("Unknown")
        .into()
}
fn job_status(v: &Value, c: &mut BTreeMap<String, String>) -> String {
    for (k, p) in [
        ("completions", "/status/succeeded"),
        ("failed", "/status/failed"),
        ("active", "/status/active"),
        ("desiredCompletions", "/spec/completions"),
        ("parallelism", "/spec/parallelism"),
    ] {
        if let Some(n) = v.pointer(p).and_then(Value::as_u64) {
            insert(c, k, n.to_string());
        }
    }
    suspend(v, c);
    if let Some(conditions) = v.pointer("/status/conditions").and_then(Value::as_array) {
        for (condition, status) in [("Failed", "Failed"), ("Complete", "Completed")] {
            if conditions.iter().any(|item| {
                item.get("type").and_then(Value::as_str) == Some(condition)
                    && item.get("status").and_then(Value::as_str) == Some("True")
            }) {
                return status.into();
            }
        }
    }
    "Unknown".into()
}
fn service_status(v: &Value, c: &mut BTreeMap<String, String>) -> String {
    scalar(v, c, "externalName", "/spec/externalName");
    scalar(v, c, "externalTrafficPolicy", "/spec/externalTrafficPolicy");
    scalar(v, c, "sessionAffinity", "/spec/sessionAffinity");
    count(v, c, "externalIPs", "/spec/externalIPs");
    count(v, c, "loadBalancers", "/status/loadBalancer/ingress");
    for (k, p) in [("type", "/spec/type"), ("clusterIP", "/spec/clusterIP")] {
        if let Some(x) = v.pointer(p).and_then(Value::as_str) {
            insert(c, k, bounded(x));
        }
    }
    if let Some(ports) = v.pointer("/spec/ports").and_then(Value::as_array) {
        insert(
            c,
            "ports",
            ports
                .iter()
                .filter_map(|x| x.get("port").and_then(Value::as_u64))
                .take(12)
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    "Unknown".into()
}
fn pvc_status(v: &Value, c: &mut BTreeMap<String, String>) -> String {
    scalar(v, c, "volume", "/spec/volumeName");
    scalar(v, c, "volumeMode", "/spec/volumeMode");
    quantities(v, c, "requested", "/spec/resources/requests/storage");
    if let Some(x) = v
        .pointer("/status/capacity/storage")
        .and_then(Value::as_str)
    {
        insert(c, "capacity", bounded(x));
    }
    if let Some(x) = v.pointer("/spec/storageClassName").and_then(Value::as_str) {
        insert(c, "storageClass", bounded(x));
    }
    if let Some(a) = v.pointer("/spec/accessModes").and_then(Value::as_array) {
        insert(
            c,
            "accessModes",
            a.iter()
                .filter_map(Value::as_str)
                .take(12)
                .map(bounded)
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    v.pointer("/status/phase")
        .and_then(Value::as_str)
        .unwrap_or("Unknown")
        .into()
}
fn event_status(v: &Value, c: &mut BTreeMap<String, String>) -> String {
    for (k, p) in [
        ("type", "/type"),
        ("reason", "/reason"),
        ("message", "/message"),
    ] {
        if let Some(x) = v.pointer(p).and_then(Value::as_str) {
            insert(c, k, bounded(x));
        }
    }
    if let Some(count) = v
        .pointer("/series/count")
        .and_then(Value::as_u64)
        .or_else(|| v.pointer("/count").and_then(Value::as_u64))
    {
        insert(c, "count", count.to_string());
    }
    for (key, paths) in [
        (
            "lastSeen",
            vec![
                "/series/lastObservedTime",
                "/lastTimestamp",
                "/eventTime",
                "/firstTimestamp",
            ],
        ),
        ("occurrenceTime", vec!["/eventTime", "/firstTimestamp"]),
    ] {
        if let Some(time) = paths
            .into_iter()
            .find_map(|path| v.pointer(path).and_then(Value::as_str).and_then(timestamp))
        {
            insert(c, key, time);
        }
    }
    if let Some(source) = v
        .pointer("/reportingComponent")
        .and_then(Value::as_str)
        .filter(|value| !bounded(value).is_empty())
        .or_else(|| v.pointer("/source/component").and_then(Value::as_str))
    {
        insert(c, "source", bounded(source));
    }
    if let (Some(kind), Some(name)) = (
        v.pointer("/involvedObject/kind").and_then(Value::as_str),
        v.pointer("/involvedObject/name").and_then(Value::as_str),
    ) && valid_name(name)
    {
        insert(c, "involvedObject", bounded(&format!("{kind}/{name}")));
    }
    v.pointer("/type")
        .and_then(Value::as_str)
        .unwrap_or("Unknown")
        .into()
}
fn insert(c: &mut BTreeMap<String, String>, k: &str, v: String) {
    if c.len() < 12 && v.len() <= 256 {
        c.insert(k.into(), v);
    }
}
fn bounded(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().filter(|c| !c.is_control() && !matches!(*c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')) {
        if out.len() + c.len_utf8() > 256 {
            break;
        }
        out.push(c);
    }
    out
}
pub(super) fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 253
        && s.split('.').all(|label| {
            !label.is_empty()
                && label.as_bytes()[0].is_ascii_alphanumeric()
                && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}
pub(super) fn timestamp(s: &str) -> Option<String> {
    (s.len() <= 64 && s.parse::<jiff::Timestamp>().is_ok()).then(|| s.to_owned())
}
pub(super) fn parse_quantity(s: &str) -> Option<f64> {
    if s.is_empty() || s.len() > 128 || s.starts_with('-') || s.chars().any(char::is_whitespace) {
        return None;
    }
    let mut number = s;
    let mut multiplier = 1.0;
    for (suffix, power, binary) in [
        ("Ki", 1, true),
        ("Mi", 2, true),
        ("Gi", 3, true),
        ("Ti", 4, true),
        ("Pi", 5, true),
        ("Ei", 6, true),
        ("n", -9, false),
        ("u", -6, false),
        ("m", -3, false),
        ("k", 3, false),
        ("M", 6, false),
        ("G", 9, false),
        ("T", 12, false),
        ("P", 15, false),
        ("E", 18, false),
    ] {
        if let Some(n) = s.strip_suffix(suffix) {
            number = n;
            multiplier = if binary {
                1024f64.powi(power)
            } else {
                10f64.powi(power)
            };
            break;
        }
    }
    // Kubernetes quantities permit decimal exponents, not NaN, infinity or mixed suffixes.
    let digits = number.strip_prefix('+').unwrap_or(number);
    let mut parts = digits.split(['e', 'E']);
    let mantissa = parts.next()?;
    let exponent = parts.next();
    if parts.next().is_some()
        || mantissa.is_empty()
        || !mantissa.bytes().any(|b| b.is_ascii_digit())
        || mantissa.bytes().any(|b| !b.is_ascii_digit() && b != b'.')
        || mantissa.bytes().filter(|b| *b == b'.').count() > 1
    {
        return None;
    }
    if let Some(e) = exponent {
        if multiplier != 1.0 {
            return None;
        }
        let e = e.strip_prefix(['+', '-']).unwrap_or(e);
        if e.is_empty() || !e.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
    }
    let value = number.parse::<f64>().ok()? * multiplier;
    (value.is_finite() && value >= 0.0 && value <= i64::MAX as f64).then_some(value)
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resource_identity_projection_retains_only_safe_metadata() {
        for kind in [
            ResourceKind::Pods,
            ResourceKind::Secrets,
            ResourceKind::ConfigMaps,
        ] {
            let row = project(kind, json!({
                "metadata": {"name":"example", "namespace":"team", "uid":"uid-123", "resourceVersion":"v1:42", "annotations":{"private":"PRIVATE"}, "managedFields":[{"private":"PRIVATE"}]},
                "data":{"password":"PRIVATE"}, "stringData":{"password":"PRIVATE"},
                "spec":{"private":"PRIVATE"}
            })).unwrap();
            let wire = serde_json::to_value(row).unwrap();
            assert_eq!(wire["uid"], "uid-123");
            assert_eq!(wire["resourceVersion"], "v1:42");
            assert!(!wire.to_string().contains("PRIVATE"));
            assert!(wire.get("metadata").is_none());
            assert!(wire.get("data").is_none());
            assert!(wire.get("spec").is_none());
        }
    }

    #[test]
    fn resource_identity_projection_absence_serializes_as_null() {
        let row = project(
            ResourceKind::Pods,
            json!({"metadata":{"name":"example","namespace":"team"}}),
        )
        .unwrap();
        let wire = serde_json::to_value(row).unwrap();
        assert_eq!(wire.get("uid"), Some(&Value::Null));
        assert_eq!(wire.get("resourceVersion"), Some(&Value::Null));
    }

    #[test]
    fn resource_identity_projection_rejects_malformed_present_fields() {
        for field in ["uid", "resourceVersion"] {
            for invalid in [
                json!(null),
                json!(42),
                json!(false),
                json!([]),
                json!({"private":"PRIVATE"}),
                json!(""),
                json!(" "),
                json!("a b"),
                json!("a\tb"),
                json!("a\nb"),
                json!("a\n"),
                json!("a\u{0000}b"),
                json!("a\u{007f}b"),
                json!("a\u{0085}b"),
                json!("a\u{00a0}b"),
                json!("é"),
                json!("x".repeat(257)),
            ] {
                let mut input = json!({"metadata":{"name":"example","namespace":"team","uid":"valid","resourceVersion":"valid"}});
                input["metadata"][field] = invalid;
                assert_eq!(
                    project(ResourceKind::Pods, input),
                    Err(ConnectionError::InvalidResponse),
                    "{field}"
                );
            }
            let mut input = json!({"metadata":{"name":"example","namespace":"team"}});
            input["metadata"][field] = json!("x".repeat(256));
            let wire = serde_json::to_value(project(ResourceKind::Pods, input).unwrap()).unwrap();
            assert_eq!(wire[field], "x".repeat(256));
        }
    }
}
