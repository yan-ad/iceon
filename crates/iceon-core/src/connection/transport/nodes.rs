use super::*;
use crate::connection::{NodeCondition, NodeInfo as ProjectedNodeInfo, Usage};
use std::{collections::BTreeSet, net::IpAddr};

#[derive(Deserialize)]
struct Item {
    metadata: Metadata,
    #[serde(default)]
    status: Status,
    #[serde(default)]
    spec: Spec,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    #[serde(default)]
    labels: std::collections::BTreeMap<String, String>,
    creation_timestamp: Option<String>,
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    #[serde(default)]
    conditions: Vec<Condition>,
    #[serde(default)]
    addresses: Vec<Address>,
    #[serde(default)]
    node_info: NodeInfo,
    #[serde(default)]
    allocatable: Quantities,
    #[serde(default)]
    capacity: Quantities,
}
#[derive(Deserialize)]
struct Condition {
    #[serde(rename = "type")]
    kind: String,
    status: String,
}
#[derive(Default, Deserialize)]
pub(super) struct Spec {
    #[serde(default)]
    unschedulable: bool,
    #[serde(default)]
    taints: Vec<serde::de::IgnoredAny>,
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NodeInfo {
    kubelet_version: Option<String>,
}
#[derive(Deserialize)]
pub(super) struct Address {
    #[serde(rename = "type")]
    kind: String,
    address: String,
}
#[derive(Default, Deserialize)]
pub(super) struct Quantities {
    cpu: Option<String>,
    memory: Option<String>,
    #[serde(rename = "ephemeral-storage")]
    disk: Option<String>,
}
fn text(value: &str, limit: usize) -> Result<()> {
    if value.is_empty() || value.len() > limit || value.chars().any(char::is_control) {
        Err(ConnectionError::InvalidResponse)
    } else {
        Ok(())
    }
}
fn array(length: usize) -> Result<()> {
    if length > ITEM_LIMIT {
        Err(ConnectionError::LimitExceeded)
    } else {
        Ok(())
    }
}
pub(in crate::connection) fn project(value: &serde_json::Value) -> Result<ProjectedNodeInfo> {
    let item: Item =
        serde_json::from_value(value.clone()).map_err(|_| ConnectionError::InvalidResponse)?;
    array(item.metadata.labels.len())?;
    array(item.spec.taints.len())?;
    array(item.status.conditions.len())?;
    array(item.status.addresses.len())?;
    let mut roles = BTreeSet::new();
    for (key, value) in &item.metadata.labels {
        let role = key
            .strip_prefix("node-role.kubernetes.io/")
            .or_else(|| (key == "kubernetes.io/role").then_some(value.as_str()));
        if let Some(role) = role {
            if role.is_empty() {
                continue;
            }
            text(role, 63)?;
            if !role
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            {
                return Err(ConnectionError::InvalidResponse);
            }
            roles.insert(role.to_owned());
        }
    }
    if let Some(version) = &item.status.node_info.kubelet_version {
        text(version, 256)?;
    }
    if let Some(timestamp) = &item.metadata.creation_timestamp {
        text(timestamp, 64)?;
        timestamp
            .parse::<k8s_openapi::jiff::Timestamp>()
            .map_err(|_| ConnectionError::InvalidResponse)?;
    }
    let mut internal_ips = BTreeSet::new();
    for address in &item.status.addresses {
        if address.kind == "InternalIP" {
            text(&address.address, 45)?;
            let ip = address
                .address
                .parse::<IpAddr>()
                .map_err(|_| ConnectionError::InvalidResponse)?;
            internal_ips.insert(ip.to_string());
        }
    }
    let mut types = HashSet::new();
    let mut conditions = Vec::new();
    for condition in &item.status.conditions {
        text(&condition.kind, 256)?;
        if !matches!(condition.status.as_str(), "True" | "False" | "Unknown")
            || !types.insert(&condition.kind)
        {
            return Err(ConnectionError::InvalidResponse);
        }
        conditions.push(NodeCondition {
            kind: condition.kind.clone(),
            status: condition.status.clone(),
        });
    }
    validate_quantities(&item.status.allocatable)?;
    validate_quantities(&item.status.capacity)?;
    Ok(ProjectedNodeInfo {
        roles: roles.into_iter().collect(),
        taints: item.spec.taints.len() as u32,
        version: item.status.node_info.kubelet_version.clone(),
        internal_ips: internal_ips.into_iter().collect(),
        schedulable: !item.spec.unschedulable,
        conditions,
        cpu: Usage {
            used: None,
            allocatable: item
                .status
                .allocatable
                .cpu
                .as_deref()
                .and_then(super::super::resources::parse_quantity),
            capacity: item
                .status
                .capacity
                .cpu
                .as_deref()
                .and_then(super::super::resources::parse_quantity),
        },
        memory: Usage {
            used: None,
            allocatable: item
                .status
                .allocatable
                .memory
                .as_deref()
                .and_then(super::super::resources::parse_quantity),
            capacity: item
                .status
                .capacity
                .memory
                .as_deref()
                .and_then(super::super::resources::parse_quantity),
        },
        disk_allocatable: item.status.allocatable.disk.clone(),
        metrics_timestamp: None,
    })
}
fn validate_quantities(quantities: &Quantities) -> Result<()> {
    for value in [&quantities.cpu, &quantities.memory, &quantities.disk]
        .into_iter()
        .flatten()
    {
        if !valid_quantity(value) || super::super::resources::parse_quantity(value).is_none() {
            return Err(ConnectionError::InvalidResponse);
        }
    }
    Ok(())
}
// Quantities stay fixed-point strings, including metrics-server nano/micro units.
fn valid_quantity(value: &str) -> bool {
    if value.is_empty() || value.len() > 64 {
        return false;
    }
    let bytes = value.as_bytes();
    let mut index = usize::from(matches!(bytes[0], b'+' | b'-'));
    let mut digits = 0;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
        digits += 1;
    }
    if index < bytes.len() && bytes[index] == b'.' {
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return false;
    }
    let suffix = &value[index..];
    if matches!(
        suffix,
        "" | "n"
            | "u"
            | "m"
            | "k"
            | "M"
            | "G"
            | "T"
            | "P"
            | "E"
            | "Ki"
            | "Mi"
            | "Gi"
            | "Ti"
            | "Pi"
            | "Ei"
    ) {
        return true;
    }
    if let Some(exponent) = suffix.strip_prefix(['e', 'E']) {
        let exponent = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        return !exponent.is_empty() && exponent.bytes().all(|b| b.is_ascii_digit());
    }
    false
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quantity_syntax_is_bounded_and_preserves_all_kubernetes_suffix_forms() {
        for value in [
            "0", "+1", "-1", ".5", "1.", "1.5Gi", "123n", "10u", "2m", "3k", "4M", "5G", "6T",
            "7P", "8E", "1Ki", "1Mi", "1Ti", "1Pi", "1Ei", "1e3", "1E-3", "1e+3",
        ] {
            assert!(valid_quantity(value), "{value}");
        }
        for value in [
            "",
            "+",
            "-",
            ".",
            "NaN",
            "inf",
            "1K",
            "1ki",
            "1GB",
            "1e",
            "1e+",
            "1e1.2",
            "1 2",
            "1\n",
            "1..2",
            "1µ",
            "1mPRIVATE",
        ] {
            assert!(!valid_quantity(value), "{value}");
        }
        assert!(!valid_quantity(&"1".repeat(65)));
    }
}
