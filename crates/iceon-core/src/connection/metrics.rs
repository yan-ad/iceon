use super::{
    ConnectionError, MetricState, ResourceItem, Result, resources::parse_quantity, transport,
};
use kube::Client;
use serde::Deserialize;
use std::{collections::HashMap, time::Duration};
use tokio_util::sync::CancellationToken;

#[derive(Deserialize)]
struct MetricsList {
    items: Vec<NodeMetric>,
    #[serde(default)]
    metadata: ListMetadata,
}
#[derive(Default, Deserialize)]
struct ListMetadata {
    #[serde(default, rename = "continue")]
    continue_token: String,
}
#[derive(Deserialize)]
struct NodeMetric {
    metadata: Meta,
    timestamp: String,
    usage: UsageRaw,
}
#[derive(Deserialize)]
struct Meta {
    name: String,
}
#[derive(Deserialize)]
struct UsageRaw {
    cpu: String,
    memory: String,
}

pub(super) async fn enrich(
    client: &Client,
    items: &mut [ResourceItem],
    cancel: &CancellationToken,
) -> Result<MetricState> {
    // The child cancels queued transport work on the shorter metrics deadline without
    // cancelling the successful inventory request that owns it.
    let metrics_cancel = cancel.child_token();
    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(ConnectionError::StaleSession),
        result = tokio::time::timeout(Duration::from_secs(3), transport::get::<MetricsList>(
            client, "/apis/metrics.k8s.io/v1beta1/nodes?limit=100&timeoutSeconds=3",
            2 * 1024 * 1024, &metrics_cancel,
        )) => result,
    };
    metrics_cancel.cancel();
    let list = match result {
        Ok(Ok(list)) => list,
        Ok(Err(ConnectionError::ResourceUnavailable)) => return Ok(MetricState::Unavailable),
        Ok(Err(ConnectionError::AuthFailed | ConnectionError::PermissionDenied)) => {
            return Ok(MetricState::Denied);
        }
        _ => return Ok(MetricState::Error),
    };
    if list.items.len() > 100 {
        return Ok(MetricState::Error);
    }
    let mut by_name = HashMap::new();
    for metric in list.items {
        if !super::resources::valid_name(&metric.metadata.name)
            || by_name
                .insert(metric.metadata.name.clone(), metric)
                .is_some()
        {
            return Ok(MetricState::Error);
        }
    }
    let now = jiff::Timestamp::now();
    let mut missing = !list.metadata.continue_token.is_empty();
    let mut stale = false;
    let mut invalid = false;
    for item in items {
        let Some(metric) = by_name.get(&item.name) else {
            missing = true;
            continue;
        };
        let timestamp = super::resources::timestamp(&metric.timestamp);
        let Some(timestamp) = timestamp else {
            invalid = true;
            continue;
        };
        let ts = timestamp
            .parse::<jiff::Timestamp>()
            .expect("validated timestamp");
        let age = now.as_second().saturating_sub(ts.as_second());
        if !(0..=120).contains(&age) {
            stale = true;
            continue;
        }
        let (Some(cpu), Some(memory)) = (
            parse_quantity(&metric.usage.cpu),
            parse_quantity(&metric.usage.memory),
        ) else {
            invalid = true;
            continue;
        };
        if let Some(node) = item.node.as_mut() {
            node.metrics_timestamp = Some(timestamp);
            node.cpu.used = Some(cpu);
            node.memory.used = Some(memory);
        }
    }
    Ok(if missing || invalid {
        MetricState::Partial
    } else if stale {
        MetricState::Stale
    } else {
        MetricState::Available
    })
}
