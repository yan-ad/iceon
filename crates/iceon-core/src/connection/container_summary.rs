//! Compact, safe container runtime facts for inventory rows and WATCH updates.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ContainerSummary {
    pub name: String,
    pub init: bool,
    pub state: String,
    pub ready: bool,
    pub restarts: u64,
    pub reason: Option<String>,
    pub exit_code: Option<i64>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub container_id: Option<String>,
}
pub(super) fn project(pod: &Value) -> Vec<ContainerSummary> {
    let mut result = Vec::new();
    for (spec, statuses, init) in [
        ("containers", "containerStatuses", false),
        ("initContainers", "initContainerStatuses", true),
    ] {
        let Some(containers) = pod
            .pointer(&format!("/spec/{spec}"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for container in containers.iter().take(128) {
            let Some(name) = container.get("name").and_then(Value::as_str) else {
                continue;
            };
            if name.is_empty()
                || name.len() > 63
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            {
                continue;
            }
            let status = pod
                .pointer(&format!("/status/{statuses}"))
                .and_then(Value::as_array)
                .and_then(|items| {
                    items
                        .iter()
                        .find(|item| item.get("name").and_then(Value::as_str) == Some(name))
                });
            let state = status.and_then(|status| status.get("state"));
            let (label, details) = ["running", "waiting", "terminated"]
                .into_iter()
                .find_map(|key| {
                    state
                        .and_then(|state| state.get(key))
                        .filter(|value| value.is_object())
                        .map(|value| (key, value))
                })
                .unwrap_or(("unknown", &Value::Null));
            let text = |value: Option<&Value>| {
                value.and_then(Value::as_str).map(super::resources::bounded)
            };
            result.push(ContainerSummary {
                name: super::resources::bounded(name),
                init,
                state: label.into(),
                ready: status
                    .and_then(|status| status.get("ready"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                restarts: status
                    .and_then(|status| status.get("restartCount"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                reason: details
                    .get("reason")
                    .and_then(Value::as_str)
                    .filter(|reason| {
                        matches!(
                            *reason,
                            "Completed"
                                | "Error"
                                | "OOMKilled"
                                | "ContainerCreating"
                                | "PodInitializing"
                                | "CrashLoopBackOff"
                                | "ImagePullBackOff"
                                | "ErrImagePull"
                                | "CreateContainerConfigError"
                                | "CreateContainerError"
                                | "RunContainerError"
                                | "StartError"
                        )
                    })
                    .map(str::to_owned),
                exit_code: details.get("exitCode").and_then(Value::as_i64),
                started_at: details
                    .get("startedAt")
                    .and_then(Value::as_str)
                    .and_then(super::resources::timestamp),
                finished_at: details
                    .get("finishedAt")
                    .and_then(Value::as_str)
                    .and_then(super::resources::timestamp),
                container_id: text(status.and_then(|status| status.get("containerID"))).filter(
                    |id| {
                        id.split_once("://").is_some_and(|(runtime, id)| {
                            matches!(runtime, "containerd" | "docker" | "cri-o")
                                && !id.is_empty()
                                && id.bytes().all(|b| b.is_ascii_hexdigit())
                        })
                    },
                ),
            });
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::project;
    use serde_json::json;
    #[test]
    fn projects_runtime_facts_without_secret_bearing_spec_or_messages() {
        let pod = json!({"spec":{"containers":[{"name":"app","env":[{"value":"secret"}]},{"name":"missing"}],"initContainers":[{"name":"init"}]},"status":{"containerStatuses":[{"name":"app","ready":true,"restartCount":2,"state":{"running":{"startedAt":"2026-11-12T00:00:00Z"}}}],"initContainerStatuses":[{"name":"init","ready":false,"state":{"terminated":{"exitCode":0,"reason":"Completed","message":"secret"}}}]}});
        let summaries = project(&pod);
        assert_eq!(summaries.len(), 3);
        assert!(summaries[0].ready);
        assert_eq!(summaries[0].restarts, 2);
        assert_eq!(summaries[1].state, "unknown");
        assert!(summaries[2].init);
        assert_eq!(summaries[2].exit_code, Some(0));
        assert!(
            !serde_json::to_string(&summaries)
                .unwrap()
                .contains("secret")
        );
    }
}
