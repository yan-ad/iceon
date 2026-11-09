use super::ResourceKind;
use serde::Serialize;
use serde_json::Value;

const MAX_SECTIONS: usize = 12;
const MAX_FACTS: usize = 32;
const MAX_VALUE: usize = 1024;
const MAX_TOTAL: usize = 64 * 1024;
const MAX_ITEMS: usize = 12;

// These schemas describe reviewed API fields, not arbitrary object serialization.
enum Shape {
    Scalar,
    Map,
    List(&'static Shape),
    Object(&'static [(&'static str, &'static Shape)]),
}
const SCALAR: Shape = Shape::Scalar;
const MAP: Shape = Shape::Map;
const STRINGS: Shape = Shape::List(&SCALAR);
const EXPRESSION: Shape = Shape::Object(&[
    ("key", &SCALAR),
    ("operator", &SCALAR),
    ("values", &STRINGS),
]);
const SELECTOR: Shape = Shape::Object(&[
    ("matchLabels", &MAP),
    ("matchExpressions", &Shape::List(&EXPRESSION)),
]);
const TOLERATION: Shape = Shape::Object(&[
    ("key", &SCALAR),
    ("operator", &SCALAR),
    ("value", &SCALAR),
    ("effect", &SCALAR),
    ("tolerationSeconds", &SCALAR),
]);
const NODE_TERM: Shape = Shape::Object(&[
    ("matchExpressions", &Shape::List(&EXPRESSION)),
    ("matchFields", &Shape::List(&EXPRESSION)),
]);
const POD_TERM: Shape = Shape::Object(&[
    ("labelSelector", &SELECTOR),
    ("namespaceSelector", &SELECTOR),
    ("namespaces", &STRINGS),
    ("topologyKey", &SCALAR),
    ("matchLabelKeys", &STRINGS),
    ("mismatchLabelKeys", &STRINGS),
]);
const POD_AFFINITY: Shape = Shape::Object(&[
    (
        "requiredDuringSchedulingIgnoredDuringExecution",
        &Shape::List(&POD_TERM),
    ),
    (
        "preferredDuringSchedulingIgnoredDuringExecution",
        &Shape::List(&Shape::Object(&[
            ("weight", &SCALAR),
            ("podAffinityTerm", &POD_TERM),
        ])),
    ),
]);
const AFFINITY: Shape = Shape::Object(&[
    (
        "nodeAffinity",
        &Shape::Object(&[
            (
                "requiredDuringSchedulingIgnoredDuringExecution",
                &Shape::Object(&[("nodeSelectorTerms", &Shape::List(&NODE_TERM))]),
            ),
            (
                "preferredDuringSchedulingIgnoredDuringExecution",
                &Shape::List(&Shape::Object(&[
                    ("weight", &SCALAR),
                    ("preference", &NODE_TERM),
                ])),
            ),
        ]),
    ),
    ("podAffinity", &POD_AFFINITY),
    ("podAntiAffinity", &POD_AFFINITY),
]);
const METRIC_VALUE: Shape = Shape::Object(&[
    ("type", &SCALAR),
    ("value", &SCALAR),
    ("averageValue", &SCALAR),
    ("averageUtilization", &SCALAR),
]);
const METRIC_SOURCE: Shape = Shape::Object(&[
    ("name", &SCALAR),
    ("container", &SCALAR),
    (
        "metric",
        &Shape::Object(&[("name", &SCALAR), ("selector", &SELECTOR)]),
    ),
    (
        "describedObject",
        &Shape::Object(&[
            ("apiVersion", &SCALAR),
            ("kind", &SCALAR),
            ("name", &SCALAR),
        ]),
    ),
    ("target", &METRIC_VALUE),
    ("current", &METRIC_VALUE),
]);
const METRIC: Shape = Shape::Object(&[
    ("type", &SCALAR),
    ("resource", &METRIC_SOURCE),
    ("containerResource", &METRIC_SOURCE),
    ("pods", &METRIC_SOURCE),
    ("object", &METRIC_SOURCE),
    ("external", &METRIC_SOURCE),
]);
const ADMISSION_RULE: Shape = Shape::Object(&[
    ("operations", &STRINGS),
    ("apiGroups", &STRINGS),
    ("apiVersions", &STRINGS),
    ("resources", &STRINGS),
    ("resourceNames", &STRINGS),
    ("scope", &SCALAR),
]);
const MATCHING: Shape = Shape::Object(&[
    ("matchPolicy", &SCALAR),
    ("namespaceSelector", &SELECTOR),
    ("objectSelector", &SELECTOR),
    ("resourceRules", &Shape::List(&ADMISSION_RULE)),
    ("excludeResourceRules", &Shape::List(&ADMISSION_RULE)),
]);
const MATCH_CONDITION: Shape = Shape::Object(&[("name", &SCALAR), ("expression", &SCALAR)]);
const NETWORK_PEER: Shape = Shape::Object(&[
    ("podSelector", &SELECTOR),
    ("namespaceSelector", &SELECTOR),
    (
        "ipBlock",
        &Shape::Object(&[("cidr", &SCALAR), ("except", &STRINGS)]),
    ),
]);
const NETWORK_RULE: Shape = Shape::Object(&[
    (
        "ports",
        &Shape::List(&Shape::Object(&[
            ("protocol", &SCALAR),
            ("port", &SCALAR),
            ("endPort", &SCALAR),
        ])),
    ),
    ("from", &Shape::List(&NETWORK_PEER)),
    ("to", &Shape::List(&NETWORK_PEER)),
]);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DetailFact {
    pub label: String,
    pub value: String,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DetailSection {
    pub title: String,
    pub facts: Vec<DetailFact>,
}

fn safe(s: &str) -> Option<String> {
    if s.is_empty()
        || s.len() > MAX_VALUE
        || s.chars().any(|c| {
            c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
    {
        None
    } else {
        Some(s.to_owned())
    }
}
#[derive(Default)]
struct Facts {
    sections: Vec<DetailSection>,
    partial: bool,
}
impl Facts {
    fn reviewed(&mut self, title: &str, v: &Value, path: &str, shape: &Shape) {
        let Some(value) = v.pointer(path) else { return };
        let mut facts = Vec::new();
        self.walk(&mut facts, title, value, shape);
        self.push(title, facts);
    }

    fn walk(&mut self, facts: &mut Vec<DetailFact>, label: &str, v: &Value, shape: &Shape) {
        if v.is_null() {
            return;
        }
        if facts.len() >= MAX_FACTS {
            self.partial = true;
            return;
        }
        match shape {
            Shape::Scalar => {
                let value = match v {
                    Value::String(s) if s.is_empty() => Some(if label.ends_with("apiGroups") {
                        "(core)".into()
                    } else {
                        "(empty)".into()
                    }),
                    Value::String(s) => safe(s),
                    Value::Number(n) => Some(n.to_string()),
                    Value::Bool(b) => Some(b.to_string()),
                    _ => None,
                };
                if let Some(value) = value.filter(|_| safe(label).is_some()) {
                    facts.push(DetailFact {
                        label: label.into(),
                        value,
                    });
                } else {
                    self.partial = true;
                }
            }
            Shape::List(item) => {
                let Some(items) = v.as_array() else {
                    self.partial = true;
                    return;
                };
                self.partial |= items.len() > MAX_ITEMS;
                if items.is_empty() {
                    self.walk(facts, label, &Value::String("(empty list)".into()), &SCALAR);
                }
                for (i, value) in items.iter().take(MAX_ITEMS).enumerate() {
                    // Scalar list labels retain their field suffix (notably apiGroups).
                    let child = if matches!(item, Shape::Scalar) {
                        label.to_owned()
                    } else {
                        format!("{label} · {}", i + 1)
                    };
                    self.walk(facts, &child, value, item);
                }
            }
            Shape::Map => {
                let Some(map) = v.as_object() else {
                    self.partial = true;
                    return;
                };
                self.partial |= map.len() > MAX_ITEMS;
                if map.is_empty() {
                    self.walk(facts, label, &Value::String("(empty map)".into()), &SCALAR);
                }
                for (key, value) in map.iter().take(MAX_ITEMS) {
                    if safe(key).is_none() || key.len() > 128 {
                        self.partial = true;
                        continue;
                    }
                    self.walk(facts, &format!("{label} · {key}"), value, &SCALAR);
                }
            }
            Shape::Object(fields) => {
                let Some(map) = v.as_object() else {
                    self.partial = true;
                    return;
                };
                self.partial |= map
                    .keys()
                    .any(|key| !fields.iter().any(|(field, _)| key == field));
                if map.is_empty() {
                    self.walk(
                        facts,
                        label,
                        &Value::String("(empty object)".into()),
                        &SCALAR,
                    );
                }
                for (key, shape) in *fields {
                    if let Some(value) = map.get(*key) {
                        self.walk(facts, &format!("{label} · {key}"), value, shape);
                    }
                }
            }
        }
    }
    fn scalar(&mut self, v: &Value, path: &str) -> Option<String> {
        let value = v.pointer(path)?;
        let result = match value {
            Value::String(s)
                if s.is_empty()
                    && matches!(path.rsplit('/').next(), Some("apiGroup" | "apiGroups")) =>
            {
                Some("(core)".into())
            }
            Value::String(s) => safe(s),
            Value::Bool(b) => Some(b.to_string()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        };
        self.partial |= !value.is_null() && result.is_none();
        result
    }
    fn fields(&mut self, title: &str, v: &Value, fields: &[(&str, &str)]) {
        let facts = fields
            .iter()
            .map(|(label, path)| DetailFact {
                label: (*label).into(),
                value: self
                    .scalar(v, path)
                    .unwrap_or_else(|| "Not reported".into()),
            })
            .collect();
        self.push(title, facts);
    }
    fn push(&mut self, title: &str, mut facts: Vec<DetailFact>) {
        if facts.is_empty() {
            return;
        }
        if self.sections.len() >= MAX_SECTIONS {
            self.partial = true;
            return;
        }
        self.partial |= facts.len() > MAX_FACTS;
        facts.truncate(MAX_FACTS);
        self.sections.push(DetailSection {
            title: title.into(),
            facts,
        });
    }
    fn rows(&mut self, title: &str, v: &Value, path: &str, fields: &[(&str, &str)]) {
        let Some(rows) = v.pointer(path).and_then(Value::as_array) else {
            self.partial |= v.pointer(path).is_some_and(|v| !v.is_null());
            return;
        };
        self.partial |= rows.len() > MAX_ITEMS;
        let mut facts = Vec::new();
        for (i, row) in rows.iter().take(MAX_ITEMS).enumerate() {
            self.partial |= !row.is_object();
            for (label, path) in fields {
                if let Some(value) = self.scalar(row, path) {
                    facts.push(DetailFact {
                        label: format!("{} · {}", i + 1, label),
                        value,
                    });
                }
            }
        }
        self.push(title, facts);
    }
    fn strings(&mut self, title: &str, v: &Value, fields: &[(&str, &str)]) {
        let mut facts = Vec::new();
        for (label, path) in fields {
            let Some(items) = v.pointer(path).and_then(Value::as_array) else {
                self.partial |= v.pointer(path).is_some_and(|v| !v.is_null());
                continue;
            };
            self.partial |= items.len() > MAX_ITEMS;
            for item in items.iter().take(MAX_ITEMS) {
                let value = match item.as_str() {
                    Some("") if *path == "/apiGroups" => Some("(core)".into()),
                    Some(s) => safe(s),
                    None => None,
                };
                if let Some(value) = value {
                    facts.push(DetailFact {
                        label: (*label).into(),
                        value,
                    });
                } else {
                    self.partial = true;
                }
            }
        }
        self.push(title, facts);
    }
    fn count(&mut self, title: &str, v: &Value, fields: &[(&str, &str)]) {
        let facts = fields
            .iter()
            .map(|(label, path)| {
                let value = v.pointer(path);
                let count = value.and_then(|x| {
                    x.as_array()
                        .map(Vec::len)
                        .or_else(|| x.as_object().map(|m| m.len()))
                });
                self.partial |= value.is_some_and(|v| !v.is_null()) && count.is_none();
                DetailFact {
                    label: (*label).into(),
                    value: count
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "Not reported".into()),
                }
            })
            .collect();
        self.push(title, facts);
    }
    fn selector(&mut self, title: &str, v: &Value, path: &str) {
        // Only explicitly selected spec selectors, never metadata labels or annotations.
        let Some(map) = v.pointer(path).and_then(Value::as_object) else {
            self.partial |= v.pointer(path).is_some_and(|v| !v.is_null());
            return;
        };
        self.partial |= map.len() > MAX_ITEMS;
        let facts = map
            .iter()
            .take(MAX_ITEMS)
            .filter_map(|(key, value)| {
                let fact = safe(key).filter(|key| key.len() <= 128).and_then(|key| {
                    Some(DetailFact {
                        label: key,
                        value: value.as_str().and_then(safe)?,
                    })
                });
                self.partial |= fact.is_none();
                fact
            })
            .collect();
        self.push(title, facts);
    }
    fn finish(mut self) -> Vec<DetailSection> {
        // Bound worst-case JSON escaping, section envelopes, and the final coverage marker.
        let mut total = 1024;
        for section in &mut self.sections {
            total += section.title.len() * 6 + 64;
            section.facts.retain(|fact| {
                let bytes = (fact.label.len() + fact.value.len()) * 6 + 64;
                if total + bytes > MAX_TOTAL - 4096 {
                    self.partial = true;
                    false
                } else {
                    total += bytes;
                    true
                }
            });
        }
        self.sections.retain(|s| !s.facts.is_empty());
        if self.partial {
            // A full projection must still disclose omissions, even if a section is displaced.
            self.sections.truncate(MAX_SECTIONS - 1);
            self.sections.push(DetailSection {
                title: "Coverage".into(),
                facts: vec![DetailFact {
                    label: "Coverage".into(),
                    value:
                        "Partial: some facts omitted due to display limits or unsupported values"
                            .into(),
                }],
            });
        }
        self.sections
    }
}

pub(super) fn project(kind: ResourceKind, v: &Value) -> Vec<DetailSection> {
    use ResourceKind::*;
    let mut f = Facts::default();
    match kind {
        // PartialObjectMetadata only: no data, key names, type, labels or annotations.
        Secrets | ConfigMaps => return Vec::new(),
        Nodes => {
            f.fields(
                "Node system",
                v,
                &[
                    ("OS", "/status/nodeInfo/operatingSystem"),
                    ("OS image", "/status/nodeInfo/osImage"),
                    ("Architecture", "/status/nodeInfo/architecture"),
                    ("Kernel", "/status/nodeInfo/kernelVersion"),
                    ("Kubelet", "/status/nodeInfo/kubeletVersion"),
                    (
                        "Container runtime",
                        "/status/nodeInfo/containerRuntimeVersion",
                    ),
                    ("Unschedulable", "/spec/unschedulable"),
                    ("Provider ID", "/spec/providerID"),
                ],
            );
            f.reviewed("Capacity resources", v, "/status/capacity", &MAP);
            f.reviewed("Allocatable resources", v, "/status/allocatable", &MAP);
            f.fields(
                "Capacity",
                v,
                &[
                    ("CPU", "/status/capacity/cpu"),
                    ("Memory", "/status/capacity/memory"),
                    ("Pods", "/status/capacity/pods"),
                    ("Ephemeral storage", "/status/capacity/ephemeral-storage"),
                ],
            );
            f.fields(
                "Allocatable",
                v,
                &[
                    ("CPU", "/status/allocatable/cpu"),
                    ("Memory", "/status/allocatable/memory"),
                    ("Pods", "/status/allocatable/pods"),
                    ("Ephemeral storage", "/status/allocatable/ephemeral-storage"),
                ],
            );
            f.rows(
                "Addresses",
                v,
                "/status/addresses",
                &[("Type", "/type"), ("Address", "/address")],
            );
            f.rows(
                "Taints",
                v,
                "/spec/taints",
                &[("Key", "/key"), ("Value", "/value"), ("Effect", "/effect")],
            );
        }
        Pods => f.fields(
            "Pod scheduling",
            v,
            &[
                ("Node", "/spec/nodeName"),
                ("Service account", "/spec/serviceAccountName"),
                ("Scheduler", "/spec/schedulerName"),
                ("Priority class", "/spec/priorityClassName"),
                ("Restart policy", "/spec/restartPolicy"),
                ("Phase", "/status/phase"),
                ("Pod IP", "/status/podIP"),
                ("Host IP", "/status/hostIP"),
                ("QoS class", "/status/qosClass"),
            ],
        ),
        Deployments | DaemonSets | StatefulSets | ReplicaSets | ReplicationControllers => {
            let title = match kind {
                Deployments => "Deployment",
                DaemonSets => "DaemonSet",
                StatefulSets => "StatefulSet",
                ReplicaSets => "ReplicaSet",
                _ => "ReplicationController",
            };
            if kind == DaemonSets {
                f.fields(
                    title,
                    v,
                    &[
                        ("Desired scheduled", "/status/desiredNumberScheduled"),
                        ("Current scheduled", "/status/currentNumberScheduled"),
                        ("Ready", "/status/numberReady"),
                        ("Available", "/status/numberAvailable"),
                        ("Unavailable", "/status/numberUnavailable"),
                        ("Updated scheduled", "/status/updatedNumberScheduled"),
                        ("Misscheduled", "/status/numberMisscheduled"),
                        ("Update strategy", "/spec/updateStrategy/type"),
                    ],
                );
            } else {
                f.fields(
                    title,
                    v,
                    &[
                        ("Desired replicas", "/spec/replicas"),
                        ("Current replicas", "/status/replicas"),
                        ("Ready replicas", "/status/readyReplicas"),
                        ("Available replicas", "/status/availableReplicas"),
                        ("Observed generation", "/status/observedGeneration"),
                    ],
                );
                if kind == Deployments {
                    f.fields(
                        "Deployment rollout",
                        v,
                        &[
                            ("Updated replicas", "/status/updatedReplicas"),
                            ("Unavailable replicas", "/status/unavailableReplicas"),
                            ("Strategy", "/spec/strategy/type"),
                            ("Max surge", "/spec/strategy/rollingUpdate/maxSurge"),
                            (
                                "Max unavailable",
                                "/spec/strategy/rollingUpdate/maxUnavailable",
                            ),
                            ("Paused", "/spec/paused"),
                            ("Progress deadline seconds", "/spec/progressDeadlineSeconds"),
                        ],
                    );
                }
                if kind == StatefulSets {
                    f.fields(
                        "StatefulSet rollout",
                        v,
                        &[
                            ("Service", "/spec/serviceName"),
                            ("Pod management", "/spec/podManagementPolicy"),
                            ("Update strategy", "/spec/updateStrategy/type"),
                            ("Current revision", "/status/currentRevision"),
                            ("Update revision", "/status/updateRevision"),
                            ("Updated replicas", "/status/updatedReplicas"),
                        ],
                    );
                }
            }
            f.selector(
                "Selector",
                v,
                if kind == ReplicationControllers {
                    "/spec/selector"
                } else {
                    "/spec/selector/matchLabels"
                },
            );
            if kind != ReplicationControllers {
                f.reviewed(
                    "Selector expressions",
                    v,
                    "/spec/selector/matchExpressions",
                    &Shape::List(&EXPRESSION),
                );
            }
        }
        Jobs => f.fields(
            "Job execution",
            v,
            &[
                ("Parallelism", "/spec/parallelism"),
                ("Completions", "/spec/completions"),
                ("Completion mode", "/spec/completionMode"),
                ("Backoff limit", "/spec/backoffLimit"),
                ("Active deadline seconds", "/spec/activeDeadlineSeconds"),
                ("Suspended", "/spec/suspend"),
                ("Active", "/status/active"),
                ("Succeeded", "/status/succeeded"),
                ("Failed", "/status/failed"),
                ("Start time", "/status/startTime"),
                ("Completion time", "/status/completionTime"),
            ],
        ),
        CronJobs => {
            f.fields(
                "CronJob schedule",
                v,
                &[
                    ("Schedule", "/spec/schedule"),
                    ("Time zone", "/spec/timeZone"),
                    ("Concurrency policy", "/spec/concurrencyPolicy"),
                    ("Suspended", "/spec/suspend"),
                    ("Starting deadline seconds", "/spec/startingDeadlineSeconds"),
                    (
                        "Successful history limit",
                        "/spec/successfulJobsHistoryLimit",
                    ),
                    ("Failed history limit", "/spec/failedJobsHistoryLimit"),
                    ("Last schedule", "/status/lastScheduleTime"),
                    ("Last success", "/status/lastSuccessfulTime"),
                ],
            );
            f.rows(
                "Active jobs",
                v,
                "/status/active",
                &[("Name", "/name"), ("Namespace", "/namespace")],
            );
        }
        Services => {
            f.fields(
                "Service networking",
                v,
                &[
                    ("Type", "/spec/type"),
                    ("Cluster IP", "/spec/clusterIP"),
                    ("External name", "/spec/externalName"),
                    ("External traffic policy", "/spec/externalTrafficPolicy"),
                    ("Internal traffic policy", "/spec/internalTrafficPolicy"),
                    ("Session affinity", "/spec/sessionAffinity"),
                    ("IP family policy", "/spec/ipFamilyPolicy"),
                    (
                        "Allocate load balancer node ports",
                        "/spec/allocateLoadBalancerNodePorts",
                    ),
                    ("Load balancer class", "/spec/loadBalancerClass"),
                    ("Health check node port", "/spec/healthCheckNodePort"),
                ],
            );
            f.rows(
                "Service ports",
                v,
                "/spec/ports",
                &[
                    ("Name", "/name"),
                    ("Port", "/port"),
                    ("Target port", "/targetPort"),
                    ("Protocol", "/protocol"),
                    ("Node port", "/nodePort"),
                    ("Application protocol", "/appProtocol"),
                ],
            );
            f.strings(
                "Service addresses",
                v,
                &[
                    ("Cluster IP", "/spec/clusterIPs"),
                    ("External IP", "/spec/externalIPs"),
                    ("IP family", "/spec/ipFamilies"),
                ],
            );
            f.rows(
                "Load balancer",
                v,
                "/status/loadBalancer/ingress",
                &[
                    ("IP", "/ip"),
                    ("Hostname", "/hostname"),
                    ("IP mode", "/ipMode"),
                ],
            );
            f.selector("Service selector", v, "/spec/selector");
        }
        Endpoints => {
            f.count("Endpoints", v, &[("Subsets", "/subsets")]);
            if let Some(subsets) = v.get("subsets").and_then(Value::as_array) {
                f.partial |= subsets.len() > 3;
                for (i, subset) in subsets.iter().take(3).enumerate() {
                    f.rows(
                        &format!("Subset {} ready addresses", i + 1),
                        subset,
                        "/addresses",
                        &[
                            ("IP", "/ip"),
                            ("Hostname", "/hostname"),
                            ("Node", "/nodeName"),
                        ],
                    );
                    f.rows(
                        &format!("Subset {} not-ready addresses", i + 1),
                        subset,
                        "/notReadyAddresses",
                        &[("IP", "/ip"), ("Hostname", "/hostname")],
                    );
                    f.rows(
                        &format!("Subset {} ports", i + 1),
                        subset,
                        "/ports",
                        &[
                            ("Name", "/name"),
                            ("Port", "/port"),
                            ("Protocol", "/protocol"),
                        ],
                    );
                }
            }
        }
        EndpointSlices => {
            f.fields("Endpoint slice", v, &[("Address type", "/addressType")]);
            f.rows(
                "Endpoint ports",
                v,
                "/ports",
                &[
                    ("Name", "/name"),
                    ("Port", "/port"),
                    ("Protocol", "/protocol"),
                ],
            );
            f.rows(
                "Endpoints",
                v,
                "/endpoints",
                &[
                    ("Node", "/nodeName"),
                    ("Hostname", "/hostname"),
                    ("Ready", "/conditions/ready"),
                    ("Serving", "/conditions/serving"),
                    ("Terminating", "/conditions/terminating"),
                ],
            );
            if let Some(endpoints) = v.get("endpoints").and_then(Value::as_array) {
                f.partial |= endpoints.len() > 6;
                for (i, endpoint) in endpoints.iter().take(6).enumerate() {
                    f.strings(
                        &format!("Endpoint {} addresses", i + 1),
                        endpoint,
                        &[("Address", "/addresses")],
                    );
                }
            }
        }
        Ingresses => {
            f.fields(
                "Ingress",
                v,
                &[
                    ("Class", "/spec/ingressClassName"),
                    ("Default service", "/spec/defaultBackend/service/name"),
                    (
                        "Default service port",
                        "/spec/defaultBackend/service/port/number",
                    ),
                ],
            );
            f.rows("Ingress rules", v, "/spec/rules", &[("Host", "/host")]);
            if let Some(rules) = v.pointer("/spec/rules").and_then(Value::as_array) {
                f.partial |= rules.len() > 4;
                for (i, rule) in rules.iter().take(4).enumerate() {
                    f.rows(
                        &format!("Rule {} paths", i + 1),
                        rule,
                        "/http/paths",
                        &[
                            ("Path", "/path"),
                            ("Path type", "/pathType"),
                            ("Service", "/backend/service/name"),
                            ("Port", "/backend/service/port/number"),
                            ("Named port", "/backend/service/port/name"),
                        ],
                    );
                }
            }
            if let Some(tls) = v.pointer("/spec/tls").and_then(Value::as_array) {
                f.partial |= tls.len() > 4;
                for (i, tls) in tls.iter().take(4).enumerate() {
                    f.strings(&format!("TLS {} hosts", i + 1), tls, &[("Host", "/hosts")]);
                }
            }
        }
        IngressClasses => f.fields(
            "Ingress class",
            v,
            &[
                ("Controller", "/spec/controller"),
                ("Parameters API group", "/spec/parameters/apiGroup"),
                ("Parameters kind", "/spec/parameters/kind"),
                ("Parameters name", "/spec/parameters/name"),
                ("Parameters scope", "/spec/parameters/scope"),
            ],
        ),
        NetworkPolicies => {
            f.count(
                "Network policy",
                v,
                &[
                    ("Ingress rules", "/spec/ingress"),
                    ("Egress rules", "/spec/egress"),
                ],
            );
            f.strings("Policy types", v, &[("Type", "/spec/policyTypes")]);
            f.reviewed("Pod selector", v, "/spec/podSelector", &SELECTOR);
            f.reviewed(
                "Ingress rules",
                v,
                "/spec/ingress",
                &Shape::List(&NETWORK_RULE),
            );
            f.reviewed(
                "Egress rules",
                v,
                "/spec/egress",
                &Shape::List(&NETWORK_RULE),
            );
        }
        PersistentVolumeClaims | PersistentVolumes => {
            f.fields(
                if kind == PersistentVolumes {
                    "Persistent volume"
                } else {
                    "Persistent volume claim"
                },
                v,
                &[
                    ("Phase", "/status/phase"),
                    ("Storage class", "/spec/storageClassName"),
                    ("Volume mode", "/spec/volumeMode"),
                ],
            );
            f.strings(
                "Access modes",
                v,
                &[
                    ("Requested mode", "/spec/accessModes"),
                    ("Reported mode", "/status/accessModes"),
                ],
            );
            if kind == PersistentVolumes {
                f.fields(
                    "Volume capacity and lifecycle",
                    v,
                    &[
                        ("Capacity", "/spec/capacity/storage"),
                        ("Reclaim policy", "/spec/persistentVolumeReclaimPolicy"),
                        ("Claim", "/spec/claimRef/name"),
                        ("Claim namespace", "/spec/claimRef/namespace"),
                        ("CSI driver", "/spec/csi/driver"),
                    ],
                );
            } else {
                f.fields(
                    "Claim capacity and binding",
                    v,
                    &[
                        ("Requested storage", "/spec/resources/requests/storage"),
                        ("Reported capacity", "/status/capacity/storage"),
                        ("Volume", "/spec/volumeName"),
                    ],
                );
            }
        }
        StorageClasses => {
            f.fields(
                "Storage class",
                v,
                &[
                    ("Provisioner", "/provisioner"),
                    ("Reclaim policy", "/reclaimPolicy"),
                    ("Binding mode", "/volumeBindingMode"),
                    ("Allow expansion", "/allowVolumeExpansion"),
                ],
            );
            f.count(
                "Storage topology",
                v,
                &[("Allowed topologies", "/allowedTopologies")],
            );
        }
        Roles | ClusterRoles => {
            f.count("RBAC permissions", v, &[("Rules", "/rules")]);
            if let Some(rules) = v.get("rules").and_then(Value::as_array) {
                f.partial |= rules.len() > 8;
                for (i, rule) in rules.iter().take(8).enumerate() {
                    f.strings(
                        &format!("Rule {} permissions", i + 1),
                        rule,
                        &[
                            ("Verb", "/verbs"),
                            ("API group", "/apiGroups"),
                            ("Resource", "/resources"),
                            ("Resource name", "/resourceNames"),
                        ],
                    );
                }
            }
        }
        RoleBindings | ClusterRoleBindings => {
            f.fields(
                "Role reference",
                v,
                &[
                    ("API group", "/roleRef/apiGroup"),
                    ("Kind", "/roleRef/kind"),
                    ("Name", "/roleRef/name"),
                ],
            );
            f.rows(
                "Subjects",
                v,
                "/subjects",
                &[
                    ("Kind", "/kind"),
                    ("API group", "/apiGroup"),
                    ("Name", "/name"),
                    ("Namespace", "/namespace"),
                ],
            );
        }
        ServiceAccounts => {
            f.fields(
                "Service account",
                v,
                &[("Automount token", "/automountServiceAccountToken")],
            );
            f.count(
                "References",
                v,
                &[
                    ("Secret references (contents withheld)", "/secrets"),
                    (
                        "Image pull secret references (contents withheld)",
                        "/imagePullSecrets",
                    ),
                ],
            );
        }
        CustomResourceDefinitions => {
            f.fields(
                "Custom resource definition",
                v,
                &[
                    ("Group", "/spec/group"),
                    ("Scope", "/spec/scope"),
                    ("Kind", "/spec/names/kind"),
                    ("Plural", "/spec/names/plural"),
                    ("Conversion strategy", "/spec/conversion/strategy"),
                ],
            );
            f.rows(
                "Versions",
                v,
                "/spec/versions",
                &[
                    ("Name", "/name"),
                    ("Served", "/served"),
                    ("Storage", "/storage"),
                ],
            );
            f.strings(
                "Stored versions",
                v,
                &[("Version", "/status/storedVersions")],
            );
        }
        Namespaces => f.fields("Namespace lifecycle", v, &[("Phase", "/status/phase")]),
        Events => f.fields(
            "Event summary",
            v,
            &[
                ("Type", "/type"),
                ("Reason", "/reason"),
                ("Count", "/count"),
                ("First timestamp", "/firstTimestamp"),
                ("Last timestamp", "/lastTimestamp"),
                ("Reporting controller", "/reportingController"),
            ],
        ),
        ResourceQuotas => {
            f.reviewed("Hard resource limits", v, "/status/hard", &MAP);
            f.reviewed("Resource usage", v, "/status/used", &MAP);
            f.reviewed("Quota scopes", v, "/spec/scopes", &STRINGS);
            f.reviewed(
                "Scope selector",
                v,
                "/spec/scopeSelector/matchExpressions",
                &Shape::List(&Shape::Object(&[
                    ("scopeName", &SCALAR),
                    ("operator", &SCALAR),
                    ("values", &STRINGS),
                ])),
            );
            f.fields(
                "Quota hard limits",
                v,
                &[
                    ("CPU requests", "/status/hard/requests.cpu"),
                    ("Memory requests", "/status/hard/requests.memory"),
                    ("CPU limits", "/status/hard/limits.cpu"),
                    ("Memory limits", "/status/hard/limits.memory"),
                    ("Pods", "/status/hard/pods"),
                    ("Storage requests", "/status/hard/requests.storage"),
                ],
            );
            f.fields(
                "Quota usage",
                v,
                &[
                    ("CPU requests", "/status/used/requests.cpu"),
                    ("Memory requests", "/status/used/requests.memory"),
                    ("CPU limits", "/status/used/limits.cpu"),
                    ("Memory limits", "/status/used/limits.memory"),
                    ("Pods", "/status/used/pods"),
                    ("Storage requests", "/status/used/requests.storage"),
                ],
            );
        }
        LimitRanges => f.rows(
            "Limit ranges",
            v,
            "/spec/limits",
            &[
                ("Type", "/type"),
                ("Min CPU", "/min/cpu"),
                ("Max CPU", "/max/cpu"),
                ("Min memory", "/min/memory"),
                ("Max memory", "/max/memory"),
                ("Default CPU", "/default/cpu"),
                ("Default memory", "/default/memory"),
                ("Default request CPU", "/defaultRequest/cpu"),
                ("Default request memory", "/defaultRequest/memory"),
            ],
        ),
        HorizontalPodAutoscalers => f.fields(
            "Horizontal scaling",
            v,
            &[
                ("Target kind", "/spec/scaleTargetRef/kind"),
                ("Target", "/spec/scaleTargetRef/name"),
                ("Minimum replicas", "/spec/minReplicas"),
                ("Maximum replicas", "/spec/maxReplicas"),
                ("Current replicas", "/status/currentReplicas"),
                ("Desired replicas", "/status/desiredReplicas"),
                ("Last scale", "/status/lastScaleTime"),
            ],
        ),
        VerticalPodAutoscalers => {
            f.reviewed(
                "Recommendation resources",
                v,
                "/status/recommendation/containerRecommendations",
                &Shape::List(&Shape::Object(&[
                    ("containerName", &SCALAR),
                    ("target", &MAP),
                    ("lowerBound", &MAP),
                    ("upperBound", &MAP),
                    ("uncappedTarget", &MAP),
                ])),
            );
            f.reviewed(
                "Container policies",
                v,
                "/spec/resourcePolicy/containerPolicies",
                &Shape::List(&Shape::Object(&[
                    ("containerName", &SCALAR),
                    ("mode", &SCALAR),
                    ("minAllowed", &MAP),
                    ("maxAllowed", &MAP),
                    ("controlledResources", &STRINGS),
                    ("controlledValues", &SCALAR),
                ])),
            );
            f.rows(
                "Recommenders",
                v,
                "/spec/recommenders",
                &[("Name", "/name")],
            );
            f.fields(
                "Vertical scaling",
                v,
                &[
                    ("Target kind", "/spec/targetRef/kind"),
                    ("Target", "/spec/targetRef/name"),
                    ("Update mode", "/spec/updatePolicy/updateMode"),
                    ("Minimum replicas", "/spec/updatePolicy/minReplicas"),
                ],
            );
            f.rows(
                "Container recommendations",
                v,
                "/status/recommendation/containerRecommendations",
                &[
                    ("Container", "/containerName"),
                    ("Target CPU", "/target/cpu"),
                    ("Target memory", "/target/memory"),
                    ("Lower CPU", "/lowerBound/cpu"),
                    ("Upper CPU", "/upperBound/cpu"),
                    ("Lower memory", "/lowerBound/memory"),
                    ("Upper memory", "/upperBound/memory"),
                    ("Uncapped CPU", "/uncappedTarget/cpu"),
                    ("Uncapped memory", "/uncappedTarget/memory"),
                ],
            );
        }
        PodDisruptionBudgets => {
            f.fields(
                "Disruption budget",
                v,
                &[
                    ("Min available", "/spec/minAvailable"),
                    ("Max unavailable", "/spec/maxUnavailable"),
                    ("Allowed disruptions", "/status/disruptionsAllowed"),
                    ("Current healthy", "/status/currentHealthy"),
                    ("Desired healthy", "/status/desiredHealthy"),
                    ("Expected pods", "/status/expectedPods"),
                ],
            );
            f.selector("Pod selector", v, "/spec/selector/matchLabels");
            f.reviewed(
                "Pod selector expressions",
                v,
                "/spec/selector/matchExpressions",
                &Shape::List(&EXPRESSION),
            );
        }
        PriorityClasses => f.fields(
            "Scheduling priority",
            v,
            &[
                ("Value", "/value"),
                ("Global default", "/globalDefault"),
                ("Preemption policy", "/preemptionPolicy"),
                ("Description", "/description"),
            ],
        ),
        RuntimeClasses => f.fields(
            "Runtime class",
            v,
            &[
                ("Handler", "/handler"),
                ("CPU overhead", "/overhead/podFixed/cpu"),
                ("Memory overhead", "/overhead/podFixed/memory"),
            ],
        ),
        Leases => f.fields(
            "Lease",
            v,
            &[
                ("Holder", "/spec/holderIdentity"),
                ("Duration seconds", "/spec/leaseDurationSeconds"),
                ("Acquire time", "/spec/acquireTime"),
                ("Renew time", "/spec/renewTime"),
                ("Transitions", "/spec/leaseTransitions"),
            ],
        ),
        MutatingWebhookConfigurations | ValidatingWebhookConfigurations => {
            f.count("Admission webhooks", v, &[("Webhooks", "/webhooks")]);
            f.rows(
                "Webhook policies",
                v,
                "/webhooks",
                &[
                    ("Name", "/name"),
                    ("Failure policy", "/failurePolicy"),
                    ("Match policy", "/matchPolicy"),
                    ("Side effects", "/sideEffects"),
                    ("Timeout seconds", "/timeoutSeconds"),
                    ("Service", "/clientConfig/service/name"),
                    ("Service namespace", "/clientConfig/service/namespace"),
                ],
            );
        }
        ValidatingAdmissionPolicies => {
            f.fields(
                "Admission policy",
                v,
                &[
                    ("Failure policy", "/spec/failurePolicy"),
                    ("Parameter kind", "/spec/paramKind/kind"),
                    ("Parameter API version", "/spec/paramKind/apiVersion"),
                    ("Observed generation", "/status/observedGeneration"),
                ],
            );
            f.count(
                "Validation coverage",
                v,
                &[
                    ("Validations", "/spec/validations"),
                    ("Match conditions", "/spec/matchConditions"),
                    ("Resource rules", "/spec/matchConstraints/resourceRules"),
                ],
            );
        }
        ValidatingAdmissionPolicyBindings => {
            f.fields(
                "Admission policy binding",
                v,
                &[
                    ("Policy", "/spec/policyName"),
                    ("Parameter name", "/spec/paramRef/name"),
                    ("Parameter namespace", "/spec/paramRef/namespace"),
                    (
                        "Parameter not found action",
                        "/spec/paramRef/parameterNotFoundAction",
                    ),
                ],
            );
            f.strings(
                "Validation actions",
                v,
                &[("Action", "/spec/validationActions")],
            );
        }
        PodSecurityPolicies => f.fields(
            "Pod security policy",
            v,
            &[
                ("Privileged", "/spec/privileged"),
                ("Host network", "/spec/hostNetwork"),
                ("Host PID", "/spec/hostPID"),
                ("Host IPC", "/spec/hostIPC"),
                ("Read-only root filesystem", "/spec/readOnlyRootFilesystem"),
                ("Run as user rule", "/spec/runAsUser/rule"),
                ("SELinux rule", "/spec/seLinux/rule"),
                ("FS group rule", "/spec/fsGroup/rule"),
            ],
        ),
    }
    match kind {
        Pods
        | Deployments
        | DaemonSets
        | StatefulSets
        | ReplicaSets
        | ReplicationControllers
        | Jobs
        | CronJobs => {
            let base = match kind {
                Pods => "/spec",
                CronJobs => "/spec/jobTemplate/spec/template/spec",
                _ => "/spec/template/spec",
            };
            let image = Shape::List(&Shape::Object(&[("name", &SCALAR), ("image", &SCALAR)]));
            f.reviewed("Container images", v, &format!("{base}/containers"), &image);
            f.reviewed(
                "Init container images",
                v,
                &format!("{base}/initContainers"),
                &image,
            );
            f.reviewed("Node selector", v, &format!("{base}/nodeSelector"), &MAP);
            f.reviewed(
                "Tolerations",
                v,
                &format!("{base}/tolerations"),
                &Shape::List(&TOLERATION),
            );
            f.reviewed("Affinity", v, &format!("{base}/affinity"), &AFFINITY);
            if matches!(kind, Jobs | CronJobs | ReplicationControllers) {
                let base = if kind == CronJobs {
                    "/spec/jobTemplate/spec"
                } else {
                    "/spec"
                };
                f.fields(
                    "Workload readiness",
                    v,
                    &[
                        ("Minimum ready seconds", "/spec/minReadySeconds"),
                        ("Fully labeled replicas", "/status/fullyLabeledReplicas"),
                    ],
                );
                if kind != ReplicationControllers {
                    f.reviewed(
                        "Workload selector",
                        v,
                        &format!("{base}/selector"),
                        &SELECTOR,
                    );
                }
            }
        }
        HorizontalPodAutoscalers => {
            f.fields(
                "Legacy CPU utilization",
                v,
                &[
                    ("Target percent", "/spec/targetCPUUtilizationPercentage"),
                    ("Current percent", "/status/currentCPUUtilizationPercentage"),
                ],
            );
            // Keep spec/status lists independent: array order is not metric identity.
            f.reviewed("Metric targets", v, "/spec/metrics", &Shape::List(&METRIC));
            f.reviewed(
                "Current metrics",
                v,
                "/status/currentMetrics",
                &Shape::List(&METRIC),
            );
        }
        RuntimeClasses => {
            f.reviewed("Runtime node selector", v, "/scheduling/nodeSelector", &MAP);
            f.reviewed(
                "Runtime tolerations",
                v,
                "/scheduling/tolerations",
                &Shape::List(&TOLERATION),
            );
            f.reviewed("Runtime overhead resources", v, "/overhead/podFixed", &MAP);
        }
        PersistentVolumeClaims => f.reviewed("Claim selector", v, "/spec/selector", &SELECTOR),
        PersistentVolumes => {
            f.reviewed(
                "Volume node affinity",
                v,
                "/spec/nodeAffinity/required/nodeSelectorTerms",
                &Shape::List(&NODE_TERM),
            );
            f.reviewed("Mount options", v, "/spec/mountOptions", &STRINGS);
        }
        StorageClasses => {
            f.reviewed("Mount options", v, "/mountOptions", &STRINGS);
            f.reviewed(
                "Allowed topology terms",
                v,
                "/allowedTopologies",
                &Shape::List(&Shape::Object(&[(
                    "matchLabelExpressions",
                    &Shape::List(&Shape::Object(&[("key", &SCALAR), ("values", &STRINGS)])),
                )])),
            );
        }
        LimitRanges => {
            f.reviewed(
                "Resource constraints",
                v,
                "/spec/limits",
                &Shape::List(&Shape::Object(&[
                    ("type", &SCALAR),
                    ("min", &MAP),
                    ("max", &MAP),
                    ("default", &MAP),
                    ("defaultRequest", &MAP),
                    ("maxLimitRequestRatio", &MAP),
                ])),
            );
        }
        IngressClasses => f.fields(
            "Parameter namespace",
            v,
            &[("Namespace", "/spec/parameters/namespace")],
        ),
        CustomResourceDefinitions => {
            f.fields(
                "Resource names",
                v,
                &[
                    ("Singular", "/spec/names/singular"),
                    ("List kind", "/spec/names/listKind"),
                ],
            );
            f.reviewed("Short names", v, "/spec/names/shortNames", &STRINGS);
            if let Some(versions) = v.pointer("/spec/versions").and_then(Value::as_array) {
                f.partial |= versions.len() > MAX_ITEMS;
                for (i, version) in versions.iter().take(MAX_ITEMS).enumerate() {
                    f.reviewed(
                        &format!("Version {} printer columns", i + 1),
                        version,
                        "/additionalPrinterColumns",
                        &Shape::List(&Shape::Object(&[
                            ("name", &SCALAR),
                            ("type", &SCALAR),
                            ("jsonPath", &SCALAR),
                            ("priority", &SCALAR),
                            ("format", &SCALAR),
                            ("description", &SCALAR),
                        ])),
                    );
                    // Schema validation is intentionally not an arbitrary JSON viewer.
                    f.partial |= version
                        .get("schema")
                        .is_some_and(|schema| !schema.is_null());
                }
            }
        }
        Roles | ClusterRoles => {
            if let Some(rules) = v.get("rules").and_then(Value::as_array) {
                for (i, rule) in rules.iter().take(8).enumerate() {
                    f.reviewed(
                        &format!("Rule {} non-resource URLs", i + 1),
                        rule,
                        "/nonResourceURLs",
                        &STRINGS,
                    );
                }
            }
            f.reviewed(
                "Aggregation selectors",
                v,
                "/aggregationRule/clusterRoleSelectors",
                &Shape::List(&SELECTOR),
            );
        }
        MutatingWebhookConfigurations | ValidatingWebhookConfigurations => {
            let webhook = Shape::List(&Shape::Object(&[
                ("name", &SCALAR),
                ("matchPolicy", &SCALAR),
                ("namespaceSelector", &SELECTOR),
                ("objectSelector", &SELECTOR),
                ("rules", &Shape::List(&ADMISSION_RULE)),
                ("matchConditions", &Shape::List(&MATCH_CONDITION)),
                ("admissionReviewVersions", &STRINGS),
                ("reinvocationPolicy", &SCALAR),
                ("failurePolicy", &SCALAR),
                ("sideEffects", &SCALAR),
                ("timeoutSeconds", &SCALAR),
            ]));
            f.reviewed("Webhook matching", v, "/webhooks", &webhook);
        }
        ValidatingAdmissionPolicies => {
            f.reviewed("Match constraints", v, "/spec/matchConstraints", &MATCHING);
            f.reviewed(
                "CEL match conditions",
                v,
                "/spec/matchConditions",
                &Shape::List(&MATCH_CONDITION),
            );
            f.reviewed(
                "CEL validations",
                v,
                "/spec/validations",
                &Shape::List(&Shape::Object(&[
                    ("expression", &SCALAR),
                    ("message", &SCALAR),
                    ("messageExpression", &SCALAR),
                    ("reason", &SCALAR),
                ])),
            );
            f.reviewed(
                "CEL variables",
                v,
                "/spec/variables",
                &Shape::List(&MATCH_CONDITION),
            );
            f.reviewed(
                "CEL audit annotations",
                v,
                "/spec/auditAnnotations",
                &Shape::List(&Shape::Object(&[
                    ("key", &SCALAR),
                    ("valueExpression", &SCALAR),
                ])),
            );
            f.reviewed(
                "Type checking warnings",
                v,
                "/status/typeChecking/expressionWarnings",
                &Shape::List(&Shape::Object(&[
                    ("fieldRef", &SCALAR),
                    ("warning", &SCALAR),
                ])),
            );
        }
        ValidatingAdmissionPolicyBindings => {
            f.reviewed("Match resources", v, "/spec/matchResources", &MATCHING);
            f.reviewed(
                "Parameter selector",
                v,
                "/spec/paramRef/selector",
                &SELECTOR,
            );
        }
        _ => {}
    }
    f.rows(
        "Conditions",
        v,
        "/status/conditions",
        &[
            ("Type", "/type"),
            ("Status", "/status"),
            ("Reason", "/reason"),
            ("Last transition", "/lastTransitionTime"),
        ],
    );
    if f.sections.is_empty() {
        f.fields("Inspection", &Value::Null, &[("Facts", "/unavailable")]);
    }
    f.finish()
}

#[cfg(test)]
mod tests;
