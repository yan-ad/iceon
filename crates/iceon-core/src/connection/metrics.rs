use super::{
    ConnectionError, MetricState, ResourceItem, Result, resources::parse_quantity, transport,
};
use kube::Client;
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

#[derive(Deserialize)]
struct MetricsList {
    items: Vec<NodeMetric>,
    #[serde(default)]
    metadata: ListMetadata,
}

#[derive(Deserialize)]
struct PodMetricsList {
    items: Vec<PodMetric>,
    #[serde(default)]
    metadata: ListMetadata,
}
#[derive(Deserialize)]
struct PodMetric {
    metadata: PodMeta,
    #[serde(default)]
    timestamp: String,
    #[serde(default)]
    containers: Vec<PodContainerMetric>,
}
#[derive(Deserialize)]
struct PodMeta {
    name: String,
    namespace: String,
}
#[derive(Deserialize)]
struct PodContainerMetric {
    #[serde(default)]
    usage: PodUsage,
}
#[derive(Default, Deserialize)]
struct PodUsage {
    #[serde(default)]
    cpu: String,
    #[serde(default)]
    memory: String,
}

pub(super) async fn enrich_pods(
    client: &Client,
    items: &mut [ResourceItem],
    namespace: Option<&str>,
    cancel: &CancellationToken,
) -> Result<MetricState> {
    // Never retain old measurements if enrichment fails or a pod has no sample.
    for item in items.iter_mut() {
        item.columns.remove("cpu");
        item.columns.remove("memory");
    }
    let metrics_cancel = cancel.child_token();
    let _cancel_on_drop = metrics_cancel.clone().drop_guard();
    let fetch = async {
        if namespace.is_some_and(|ns| !transport::valid_namespace(ns)) {
            return Err(ConnectionError::InvalidInput);
        }
        let scope = namespace
            .map(|ns| format!("/namespaces/{ns}"))
            .unwrap_or_default();
        let base = format!("/apis/metrics.k8s.io/v1beta1{scope}/pods?limit=100&timeoutSeconds=3");
        let mut all = Vec::new();
        let mut continuation = String::new();
        let mut seen_tokens = HashSet::new();
        for _ in 0..100 {
            let mut path = base.clone();
            if !continuation.is_empty() {
                path.push('&');
                path.push_str(
                    &url::form_urlencoded::Serializer::new(String::new())
                        .append_pair("continue", &continuation)
                        .finish(),
                );
            }
            let page: PodMetricsList =
                transport::get(client, &path, 2 * 1024 * 1024, &metrics_cancel).await?;
            if page.items.len() > 10_000 - all.len() {
                return Err(ConnectionError::LimitExceeded);
            }
            all.extend(page.items);
            continuation = page.metadata.continue_token;
            if continuation.is_empty() {
                return Ok(all);
            }
            if continuation.len() > 4096
                || continuation.chars().any(char::is_control)
                || !seen_tokens.insert(continuation.clone())
                || all.len() == 10_000
            {
                return Err(ConnectionError::InvalidResponse);
            }
        }
        Err(ConnectionError::LimitExceeded)
    };
    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(ConnectionError::StaleSession),
        result = tokio::time::timeout(Duration::from_secs(3), fetch) => result,
    };
    metrics_cancel.cancel();
    if cancel.is_cancelled() {
        return Err(ConnectionError::StaleSession);
    }
    match result {
        Ok(Ok(metrics)) => Ok(project_pods(
            items,
            metrics,
            namespace,
            jiff::Timestamp::now(),
        )),
        Ok(Err(ConnectionError::ResourceUnavailable)) => Ok(MetricState::Unavailable),
        Ok(Err(ConnectionError::AuthFailed | ConnectionError::PermissionDenied)) => {
            Ok(MetricState::Denied)
        }
        _ => Ok(MetricState::Error),
    }
}

fn project_pods(
    items: &mut [ResourceItem],
    metrics: Vec<PodMetric>,
    namespace: Option<&str>,
    now: jiff::Timestamp,
) -> MetricState {
    let mut by_name = HashMap::new();
    for metric in metrics {
        if !super::resources::valid_name(&metric.metadata.name)
            || !transport::valid_namespace(&metric.metadata.namespace)
            || namespace.is_some_and(|ns| ns != metric.metadata.namespace)
        {
            return MetricState::Error;
        }
        let key = (
            metric.metadata.namespace.clone(),
            metric.metadata.name.clone(),
        );
        if by_name.insert(key, metric).is_some() {
            return MetricState::Error;
        }
    }
    let mut partial = false;
    let mut stale = false;
    for item in items {
        item.columns.remove("cpu");
        item.columns.remove("memory");
        let metric = item
            .namespace
            .as_ref()
            .and_then(|ns| by_name.get(&(ns.clone(), item.name.clone())));
        let Some(metric) = metric else {
            partial = true;
            continue;
        };
        let Some(ts) = super::resources::timestamp(&metric.timestamp)
            .and_then(|ts| ts.parse::<jiff::Timestamp>().ok())
        else {
            partial = true;
            continue;
        };
        // Compare nanoseconds as well: even a subsecond future sample is invalid.
        let age = now.as_nanosecond() - ts.as_nanosecond();
        if !(0..=120_000_000_000).contains(&age) {
            stale = true;
            continue;
        }
        let usage = (!metric.containers.is_empty()).then_some(()).and_then(|_| {
            metric
                .containers
                .iter()
                .try_fold((0.0, 0.0), |(cpu, memory), container| {
                    let cpu = cpu + parse_quantity(&container.usage.cpu)?;
                    let memory = memory + parse_quantity(&container.usage.memory)?;
                    (cpu.is_finite() && cpu >= 0.0 && memory.is_finite() && memory >= 0.0)
                        .then_some((cpu, memory))
                })
        });
        let Some((cpu, memory)) = usage else {
            partial = true;
            continue;
        };
        item.columns.insert("cpu".into(), format!("{cpu:.3}"));
        item.columns.insert(
            "memory".into(),
            if memory >= 1024f64.powi(3) {
                format!("{:.1}Gi", memory / 1024f64.powi(3))
            } else {
                format!("{:.1}Mi", memory / 1024f64.powi(2))
            },
        );
    }
    if partial {
        MetricState::Partial
    } else if stale {
        MetricState::Stale
    } else {
        MetricState::Available
    }
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

#[cfg(test)]
mod pod_tests {
    use super::*;
    use serde_json::{Value, json};

    fn now() -> jiff::Timestamp {
        "2026-11-12T12:00:00Z".parse().unwrap()
    }

    fn pod(namespace: &str, name: &str) -> ResourceItem {
        super::super::resources::project(
            super::super::ResourceKind::Pods,
            json!({
                "metadata": {"name": name, "namespace": namespace}
            }),
        )
        .unwrap()
    }

    fn metric(namespace: &str, name: &str, containers: Value) -> Value {
        json!({"metadata": {"name": name, "namespace": namespace},
            "timestamp": now().to_string(), "containers": containers})
    }

    fn project(items: &mut [ResourceItem], metrics: Vec<Value>) -> MetricState {
        project_pods(
            items,
            metrics
                .into_iter()
                .map(|v| serde_json::from_value(v).unwrap())
                .collect(),
            None,
            now(),
        )
    }

    #[test]
    fn namespace_collision_and_aggregate_units() {
        let mut items = vec![pod("one", "same"), pod("two", "same")];
        assert_eq!(
            project(
                &mut items,
                vec![
                    metric(
                        "two",
                        "same",
                        json!([{"usage":{"cpu":"2", "memory":"1Gi"}}])
                    ),
                    metric(
                        "one",
                        "same",
                        json!([
                            {"usage":{"cpu":"100000000n", "memory":"32768Ki"}},
                            {"usage":{"cpu":"250000u", "memory":"32Mi"}},
                            {"usage":{"cpu":"150m", "memory":"67108864"}}
                        ])
                    ),
                ]
            ),
            MetricState::Available
        );
        assert_eq!(items[0].columns["cpu"], "0.500");
        assert_eq!(items[0].columns["memory"], "128.0Mi");
        assert_eq!(items[1].columns["cpu"], "2.000");
        assert_eq!(items[1].columns["memory"], "1.0Gi");
    }

    #[test]
    fn missing_and_invalid_samples_have_no_values() {
        for containers in [
            json!([]),
            json!([{"usage":{"cpu":"1"}}]),
            json!([{"usage":{"cpu":"bad", "memory":"1Gi"}}]),
            json!([{"usage":{"cpu":"-1", "memory":"1Gi"}}]),
            json!([{"usage":{"cpu":"NaN", "memory":"1Gi"}}]),
            json!([{"usage":{"cpu":"1", "memory":"Inf"}}]),
            json!([{"usage":{"cpu":"1e999", "memory":"1Gi"}}]),
            json!([{"usage":{"cpu":"1", "memory":"-1"}}]),
            json!([{"usage":{"cpu":"1", "memory":"1Gi"}}, {"usage":{}}]),
        ] {
            let mut items = vec![pod("one", "same")];
            items[0].columns.insert("cpu".into(), "old".into());
            items[0].columns.insert("memory".into(), "old".into());
            assert_eq!(
                project(&mut items, vec![metric("one", "same", containers)]),
                MetricState::Partial
            );
            assert!(!items[0].columns.contains_key("cpu"));
            assert!(!items[0].columns.contains_key("memory"));
        }
        let mut items = vec![pod("one", "missing")];
        assert_eq!(project(&mut items, vec![]), MetricState::Partial);
        assert!(!items[0].columns.contains_key("cpu"));
    }

    #[test]
    fn stale_future_and_invalid_timestamps_have_no_values() {
        for (timestamp, state) in [
            ("2026-11-12T11:57:59Z", MetricState::Stale),
            ("2026-11-12T11:57:59.999999999Z", MetricState::Stale),
            ("2026-11-12T12:00:00.000000001Z", MetricState::Stale),
            ("invalid", MetricState::Partial),
        ] {
            let mut items = vec![pod("one", "same")];
            let mut sample = metric(
                "one",
                "same",
                json!([{"usage":{"cpu":"1", "memory":"1Mi"}}]),
            );
            sample["timestamp"] = json!(timestamp);
            assert_eq!(project(&mut items, vec![sample]), state);
            assert!(!items[0].columns.contains_key("cpu"));
            assert!(!items[0].columns.contains_key("memory"));
        }
        let mut items = vec![pod("one", "same")];
        let mut sample = metric("one", "same", json!([{"usage":{"cpu":"0", "memory":"0"}}]));
        sample["timestamp"] = json!("2026-11-12T11:58:00Z");
        assert_eq!(project(&mut items, vec![sample]), MetricState::Available);
        assert_eq!(items[0].columns["cpu"], "0.000");
        assert_eq!(items[0].columns["memory"], "0.0Mi");
    }
}
