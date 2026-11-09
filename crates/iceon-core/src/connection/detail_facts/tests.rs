use super::*;
use serde_json::json;

fn encoded(kind: ResourceKind, value: Value) -> String {
    let sections = project(kind, &value);
    assert!(sections.len() <= MAX_SECTIONS);
    assert!(sections.iter().all(|s| s.facts.len() <= MAX_FACTS));
    let text = serde_json::to_string(&sections).unwrap();
    assert!(text.len() <= MAX_TOTAL);
    assert!(!text.contains("PRIVATE"));
    text
}

#[test]
fn storage_rbac_and_crd_extensions_never_project_unreviewed_payloads() {
    for (kind, value, expected) in [
        (
            ResourceKind::PersistentVolumeClaims,
            json!({"spec":{"selector":{"matchExpressions":[{"key":"disk","operator":"In","values":["ssd"]}]}}}),
            "ssd",
        ),
        (
            ResourceKind::PersistentVolumes,
            json!({"spec":{"mountOptions":["noatime"],"csi":{"volumeAttributes":{"key":"PRIVATE"},"nodePublishSecretRef":{"name":"PRIVATE"}}}}),
            "noatime",
        ),
        (
            ResourceKind::StorageClasses,
            json!({"parameters":{"token":"PRIVATE"},"allowedTopologies":[{"matchLabelExpressions":[{"key":"zone","values":["west"]}]}]}),
            "west",
        ),
        (
            ResourceKind::ClusterRoles,
            json!({"rules":[{"nonResourceURLs":["/healthz"]}],"aggregationRule":{"clusterRoleSelectors":[{"matchExpressions":[{"key":"aggregate","operator":"Exists"}]}]}}),
            "/healthz",
        ),
        (
            ResourceKind::IngressClasses,
            json!({"spec":{"parameters":{"namespace":"routing"}}}),
            "routing",
        ),
        (
            ResourceKind::CustomResourceDefinitions,
            json!({"spec":{"names":{"singular":"widget","listKind":"WidgetList","shortNames":["wd"]},"versions":[{"name":"v1","schema":{"openAPIV3Schema":{"description":"PRIVATE"}},"additionalPrinterColumns":[{"name":"Queue","type":"string","jsonPath":".status.queue","priority":0}]}]}}),
            ".status.queue",
        ),
    ] {
        let text = encoded(kind, value);
        assert!(text.contains(expected), "{kind:?}: {expected}");
    }
}

#[test]
fn reviewed_structured_projection_discloses_unsupported_and_fact_overflow() {
    let sections = project(
        ResourceKind::RuntimeClasses,
        &json!({"scheduling":{"tolerations":[{"key":"safe","unreviewed":"PRIVATE"}]}}),
    );
    assert_partial(&sections);
    assert!(
        !serde_json::to_string(&sections)
            .unwrap()
            .contains("PRIVATE")
    );
    let expr = json!({"key":"zone","operator":"In","values":vec!["west";12]});
    let sections = project(
        ResourceKind::NetworkPolicies,
        &json!({"spec":{"ingress":[{"from":[{"podSelector":{"matchExpressions":vec![expr;12]}}]}]}}),
    );
    assert_partial(&sections);
    assert!(sections.iter().all(|s| s.facts.len() <= MAX_FACTS));
}

#[test]
fn service_lb_protocol_and_false_allocation_are_reported() {
    let text = encoded(
        ResourceKind::Services,
        json!({"spec":{
        "allocateLoadBalancerNodePorts":false,"healthCheckNodePort":0,
        "ports":[{"appProtocol":"kubernetes.io/h2c"}]},
        "status":{"loadBalancer":{"ingress":[{"ipMode":"Proxy"}]}}}),
    );
    for value in ["false", "Proxy", "kubernetes.io/h2c", "\"0\""] {
        assert!(text.contains(value));
    }
}

#[test]
fn all_workload_templates_project_reviewed_scheduling_not_container_payloads() {
    let spec = json!({"containers":[{"name":"app","image":"image:v2","env":[{"value":"PRIVATE"}],"command":["PRIVATE"]}],
        "initContainers":[{"name":"init","image":"init:v3"}],
        "nodeSelector":{"zone":"west"},"tolerations":[{"key":"dedicated","tolerationSeconds":0}],
        "affinity":{"nodeAffinity":{"requiredDuringSchedulingIgnoredDuringExecution":{"nodeSelectorTerms":[{"matchExpressions":[{"key":"disk","operator":"In","values":["nvme"]}]}]}}}});
    for kind in [
        ResourceKind::Pods,
        ResourceKind::Deployments,
        ResourceKind::DaemonSets,
        ResourceKind::StatefulSets,
        ResourceKind::ReplicaSets,
        ResourceKind::ReplicationControllers,
        ResourceKind::Jobs,
        ResourceKind::CronJobs,
    ] {
        let value = match kind {
            ResourceKind::Pods => json!({"spec":spec}),
            ResourceKind::CronJobs => {
                json!({"spec":{"jobTemplate":{"spec":{"template":{"spec":spec}}}}})
            }
            _ => json!({"spec":{"template":{"spec":spec}}}),
        };
        let text = encoded(kind, value);
        for value in ["image:v2", "init:v3", "west", "dedicated", "nvme"] {
            assert!(text.contains(value), "{kind:?}: {value}");
        }
    }
}

#[test]
fn selectors_include_values_and_disclose_nested_overflow() {
    let expr = json!({"key":"zone","operator":"In","values":["east", "west", ""]});
    let text = encoded(
        ResourceKind::Deployments,
        json!({"spec":{"selector":{"matchExpressions":[expr]}}}),
    );
    assert!(text.contains("east") && text.contains("west") && text.contains("(empty)"));
    let sections = project(
        ResourceKind::Deployments,
        &json!({"spec":{"selector":{"matchExpressions":[{"key":"zone","operator":"In","values":vec!["east";13]}]}}}),
    );
    assert_partial(&sections);
}

#[test]
fn arbitrary_reviewed_resource_keys_and_provider_id_are_retained() {
    let text = encoded(
        ResourceKind::Nodes,
        json!({"spec":{"providerID":"cloud:///node-1"},"status":{"capacity":{"vendor.test/gpu":"8"},"allocatable":{"hugepages-2Mi":"0"}}}),
    );
    assert!(
        text.contains("cloud:///node-1")
            && text.contains("vendor.test/gpu")
            && text.contains("hugepages-2Mi")
    );
    let text = encoded(
        ResourceKind::ResourceQuotas,
        json!({"status":{"hard":{"count/jobs.batch":"9"},"used":{"count/jobs.batch":"0"}},"spec":{"scopeSelector":{"matchExpressions":[{"scopeName":"PriorityClass","operator":"In","values":["batch"]}]}}}),
    );
    for value in ["count/jobs.batch", "PriorityClass", "batch"] {
        assert!(text.contains(value));
    }
}

#[test]
fn hpa_metric_types_keep_current_and_targets_independent_with_zero_values() {
    for (kind, field) in [
        ("Resource", "resource"),
        ("ContainerResource", "containerResource"),
        ("Pods", "pods"),
        ("Object", "object"),
        ("External", "external"),
    ] {
        let mut target = json!({"type":kind});
        target[field] = json!({"name":"cpu","container":"app","metric":{"name":"queue","selector":{"matchExpressions":[{"key":"queue","operator":"In","values":["primary"]}]}},"target":{"type":"AverageValue","averageValue":"0"}});
        let mut current = json!({"type":kind});
        current[field] = json!({"name":"cpu","current":{"averageUtilization":0}});
        let text = encoded(
            ResourceKind::HorizontalPodAutoscalers,
            json!({"spec":{"metrics":[target]},"status":{"currentMetrics":[current]}}),
        );
        for value in [
            "Metric targets",
            "Current metrics",
            "primary",
            "averageUtilization",
            "\"0\"",
        ] {
            assert!(text.contains(value), "{kind}: {value}");
        }
    }
}

#[test]
fn vpa_policies_and_all_recommendation_bounds_preserve_resource_keys() {
    let text = encoded(
        ResourceKind::VerticalPodAutoscalers,
        json!({"spec":{"updatePolicy":{"minReplicas":0},"recommenders":[{"name":"custom"}],"resourcePolicy":{"containerPolicies":[{"containerName":"*","mode":"Off","minAllowed":{"cpu":"0"},"maxAllowed":{"vendor/gpu":"2"},"controlledResources":["cpu"],"controlledValues":"RequestsOnly"}]}},"status":{"recommendation":{"containerRecommendations":[{"containerName":"app","target":{"cpu":"1"},"lowerBound":{"memory":"1Mi"},"upperBound":{"memory":"8Mi"},"uncappedTarget":{"vendor/gpu":"3"}}]}}}),
    );
    for value in [
        "Off",
        "RequestsOnly",
        "custom",
        "1Mi",
        "8Mi",
        "uncappedTarget",
        "vendor/gpu",
    ] {
        assert!(text.contains(value));
    }
}

#[test]
fn network_policy_structure_preserves_rules_peers_and_empty_vs_absent() {
    let text = encoded(
        ResourceKind::NetworkPolicies,
        json!({"spec":{"podSelector":{},"ingress":[{"ports":[{"port":"http","endPort":8081}],"from":[{"namespaceSelector":{},"podSelector":{"matchExpressions":[{"key":"app","operator":"In","values":["frontend"]}]}},{"ipBlock":{"cidr":"10.0.0.0/8","except":["10.1.0.0/16"]}}]}],"egress":[]}}),
    );
    for value in [
        "frontend",
        "10.1.0.0/16",
        "endPort",
        "(empty object)",
        "(empty list)",
        "from · 2",
    ] {
        assert!(text.contains(value), "{value}");
    }
    let absent = encoded(ResourceKind::NetworkPolicies, json!({"spec":{}}));
    assert!(!absent.contains("(empty list)"));
}

#[test]
fn runtime_class_scheduling_and_extended_limits_are_bounded() {
    let text = encoded(
        ResourceKind::RuntimeClasses,
        json!({"scheduling":{"nodeSelector":{"runtime":"kata"},"tolerations":[{"key":"sandbox","operator":"Exists","tolerationSeconds":0}]},"overhead":{"podFixed":{"vendor/resource":"0"}}}),
    );
    for value in ["kata", "sandbox", "vendor/resource", "\"0\""] {
        assert!(text.contains(value));
    }
    let text = encoded(
        ResourceKind::LimitRanges,
        json!({"spec":{"limits":[{"type":"Container","maxLimitRequestRatio":{"vendor/resource":"2"}}]}}),
    );
    assert!(text.contains("vendor/resource"));
}

#[test]
fn admission_cel_matching_and_warnings_are_display_only() {
    let text = encoded(
        ResourceKind::ValidatingAdmissionPolicies,
        json!({"spec":{"matchConstraints":{"matchPolicy":"Exact","resourceRules":[{"apiGroups":[""],"resources":["pods"],"operations":["CREATE"]}],"excludeResourceRules":[]},"validations":[{"expression":"object.spec.replicas >= 0","messageExpression":"'invalid'","reason":"Invalid"}],"variables":[{"name":"replicas","expression":"object.spec.replicas"}],"auditAnnotations":[{"key":"audit","valueExpression":"'checked'"}]},"status":{"typeChecking":{"expressionWarnings":[{"fieldRef":"spec.validations[0]","warning":"type mismatch"}]}}}),
    );
    for value in [
        "Exact",
        "(core)",
        "CREATE",
        "object.spec.replicas >= 0",
        "'checked'",
        "type mismatch",
    ] {
        assert!(text.contains(value));
    }
    let text = encoded(
        ResourceKind::ValidatingWebhookConfigurations,
        json!({"webhooks":[{"name":"safe","clientConfig":{"caBundle":"PRIVATE","url":"PRIVATE"},"namespaceSelector":{"matchLabels":{"team":"platform"}},"matchConditions":[{"name":"guard","expression":"true"}]}]}),
    );
    assert!(text.contains("platform") && text.contains("guard"));
    let text = encoded(
        ResourceKind::ValidatingAdmissionPolicyBindings,
        json!({"spec":{"paramRef":{"selector":{"matchLabels":{"tier":"prod"}}},"matchResources":{"objectSelector":{}}}}),
    );
    assert!(text.contains("prod"));
}

#[test]
fn reviewed_shapes_reject_nested_payloads_and_disclose_limits() {
    for value in [
        json!({"bad":"PRIVATE"}),
        json!("\u{202e}PRIVATE"),
        json!("x".repeat(1025)),
    ] {
        let sections = project(
            ResourceKind::RuntimeClasses,
            &json!({"scheduling":{"nodeSelector":{"safe":value}}}),
        );
        assert_partial(&sections);
        assert!(
            !serde_json::to_string(&sections)
                .unwrap()
                .contains("PRIVATE")
        );
    }
    assert_partial(&project(
        ResourceKind::NetworkPolicies,
        &json!({"spec":{"ingress":vec![json!({});13]}}),
    ));
    let text = encoded(
        ResourceKind::RuntimeClasses,
        json!({"scheduling":{"nodeSelector":{},"tolerations":[]}}),
    );
    assert!(text.contains("(empty map)") && text.contains("(empty list)"));
    assert!(!encoded(ResourceKind::RuntimeClasses, json!({})).contains("(empty"));
}

#[test]
fn every_kind_has_explicit_safe_facts_or_sensitive_empty() {
    let source = include_str!("../../connection.rs");
    let variants = source
        .split("pub enum ResourceKind {")
        .nth(1)
        .unwrap()
        .split('}')
        .next()
        .unwrap();
    for variant in variants
        .lines()
        .map(str::trim)
        .map(|s| s.trim_end_matches(','))
        .filter(|s| !s.is_empty())
    {
        let wire = format!("{}{}", variant[..1].to_lowercase(), &variant[1..]);
        let kind: ResourceKind = serde_json::from_value(json!(wire)).unwrap();
        let value = json!({"metadata":{"labels":{"password":"PRIVATE"},"annotations":{"token":"PRIVATE"}},"data":{"PRIVATE":"PRIVATE"},"stringData":{"PRIVATE":"PRIVATE"},"status":{"message":"PRIVATE"},"spec":{"password":"PRIVATE"}});
        let sections = project(kind, &value);
        assert_eq!(
            sections.is_empty(),
            matches!(kind, ResourceKind::Secrets | ResourceKind::ConfigMaps),
            "{kind:?}"
        );
        let serialized = serde_json::to_string(&sections).unwrap();
        assert!(!serialized.contains("PRIVATE"), "{kind:?}");
        assert!(serialized.len() <= MAX_TOTAL);
    }
}

#[test]
fn kind_specific_values_are_not_generic_inventory_columns() {
    for (kind, object, expected) in [
        (
            ResourceKind::Nodes,
            json!({"status":{"capacity":{"cpu":"8"},"nodeInfo":{"kernelVersion":"6.6"}}}),
            "6.6",
        ),
        (
            ResourceKind::DaemonSets,
            json!({"status":{"desiredNumberScheduled":7}}),
            "7",
        ),
        (
            ResourceKind::StatefulSets,
            json!({"spec":{"serviceName":"stable-dns"}}),
            "stable-dns",
        ),
        (
            ResourceKind::Deployments,
            json!({"spec":{"strategy":{"type":"RollingUpdate"}}}),
            "RollingUpdate",
        ),
        (
            ResourceKind::Services,
            json!({"spec":{"ports":[{"port":8080,"targetPort":"http"}]}}),
            "http",
        ),
        (
            ResourceKind::Ingresses,
            json!({"spec":{"tls":[{"hosts":["tls.example.test"],"secretName":"PRIVATE"}]}}),
            "tls.example.test",
        ),
        (
            ResourceKind::Roles,
            json!({"rules":[{"verbs":["get"],"resources":["pods"]}]}),
            "pods",
        ),
        (
            ResourceKind::CustomResourceDefinitions,
            json!({"spec":{"versions":[{"name":"v1","served":false,"storage":true}]}}),
            "false",
        ),
        (
            ResourceKind::PersistentVolumes,
            json!({"spec":{"capacity":{"storage":"50Gi"},"csi":{"nodePublishSecretRef":{"name":"PRIVATE"}}}}),
            "50Gi",
        ),
    ] {
        let encoded = serde_json::to_string(&project(kind, &object)).unwrap();
        assert!(encoded.contains(expected), "{kind:?}");
        assert!(!encoded.contains("PRIVATE"));
    }
}

#[test]
fn values_and_collections_are_bounded_without_raw_serialization() {
    let invalid = [
        "x".repeat(1025),
        "token\nPRIVATE".into(),
        "\u{202e}PRIVATE".into(),
    ];
    for text in invalid {
        let value = json!({"status":{"nodeInfo":{"kernelVersion":text}}});
        let encoded = serde_json::to_string(&project(ResourceKind::Nodes, &value)).unwrap();
        assert!(!encoded.contains("PRIVATE"));
        assert!(!encoded.contains(&"x".repeat(1025)));
    }
    let value = json!({"rules":(0..100).map(|_| json!({"verbs":vec!["\\".repeat(1024);100],"resources":vec!["r".repeat(1024);100]})).collect::<Vec<_>>()});
    let sections = project(ResourceKind::Roles, &value);
    assert!(sections.len() <= MAX_SECTIONS);
    assert!(
        sections
            .iter()
            .all(|s| s.facts.len() <= MAX_FACTS
                && s.facts.iter().all(|f| f.value.len() <= MAX_VALUE))
    );
    assert!(serde_json::to_vec(&sections).unwrap().len() <= MAX_TOTAL);
    let malformed =
        json!({"spec":{"parallelism":{"password":"PRIVATE"},"completions":["PRIVATE"]}});
    assert!(
        !serde_json::to_string(&project(ResourceKind::Jobs, &malformed))
            .unwrap()
            .contains("PRIVATE")
    );
}

#[test]
fn node_condition_facts_never_contain_messages() {
    let value = json!({"status":{"conditions":[{"type":"Ready","status":"False","reason":"KubeletNotReady","message":"PRIVATE"}]}});
    let encoded = serde_json::to_string(&project(ResourceKind::Nodes, &value)).unwrap();
    assert!(encoded.contains("KubeletNotReady"));
    assert!(!encoded.contains("PRIVATE"));
}

fn assert_partial(sections: &[DetailSection]) {
    let coverage = sections.last().expect("coverage section");
    assert_eq!(coverage.title, "Coverage");
    assert_eq!(coverage.facts.len(), 1);
    assert_eq!(coverage.facts[0].label, "Coverage");
    assert!(coverage.facts[0].value.starts_with("Partial:"));
    assert!(sections.len() <= MAX_SECTIONS);
    assert!(sections.iter().all(|s| s.facts.len() <= MAX_FACTS));
    assert!(serde_json::to_vec(sections).unwrap().len() <= MAX_TOTAL);
}

#[test]
fn rbac_combined_arrays_disclose_displaced_resource_names() {
    let rule = json!({"verbs":vec!["get";12],"apiGroups":vec!["apps";12],
        "resources":vec!["deployments";12],"resourceNames":vec!["named-target";12]});
    let sections = project(ResourceKind::Roles, &json!({"rules":[rule]}));
    assert_partial(&sections);
    assert!(
        !serde_json::to_string(&sections)
            .unwrap()
            .contains("named-target")
    );
}

#[test]
fn empty_api_groups_are_core_not_missing_or_arbitrary_empty_values() {
    let sections = project(
        ResourceKind::Roles,
        &json!({"rules":[{"apiGroups":[""],"verbs":["get"]}]}),
    );
    assert!(
        sections
            .iter()
            .flat_map(|s| &s.facts)
            .any(|f| f.label == "API group" && f.value == "(core)")
    );
    assert!(!sections.iter().any(|s| s.title == "Coverage"));
    let sections = project(
        ResourceKind::RoleBindings,
        &json!({"subjects":[{"apiGroup":"","name":"user"}]}),
    );
    assert!(
        sections
            .iter()
            .flat_map(|s| &s.facts)
            .any(|f| f.label.ends_with("API group") && f.value == "(core)")
    );
    let sections = project(ResourceKind::Roles, &json!({"rules":[{"resources":[""]}]}));
    assert_partial(&sections);
    assert!(!serde_json::to_string(&sections).unwrap().contains("(core)"));
}

#[test]
fn outer_collection_limits_always_report_partial() {
    for (kind, value) in [
        (
            ResourceKind::Endpoints,
            json!({"subsets":vec![json!({"addresses":[{"ip":"127.0.0.1"}]});4]}),
        ),
        (
            ResourceKind::EndpointSlices,
            json!({"endpoints":vec![json!({"addresses":["127.0.0.1"]});7]}),
        ),
        (
            ResourceKind::Ingresses,
            json!({"spec":{"rules":vec![json!({"http":{"paths":[{"path":"/"}]}});5]}}),
        ),
        (
            ResourceKind::Ingresses,
            json!({"spec":{"tls":vec![json!({"hosts":["example.test"]});5]}}),
        ),
        (
            ResourceKind::Roles,
            json!({"rules":vec![json!({"verbs":["get"]});9]}),
        ),
    ] {
        assert_partial(&project(kind, &value));
    }
}

#[test]
fn byte_budget_and_section_budget_reserve_final_coverage() {
    let mut facts = Facts::default();
    for i in 0..MAX_SECTIONS + 1 {
        facts.push(
            &format!("Section {i}"),
            vec![
                DetailFact {
                    label: "Value".into(),
                    value: "\\".repeat(MAX_VALUE)
                };
                MAX_FACTS
            ],
        );
    }
    assert_partial(&facts.finish());
    let mut facts = Facts::default();
    for _ in 0..2 {
        facts.push(
            "Large section",
            vec![
                DetailFact {
                    label: "Value".into(),
                    value: "é".repeat(MAX_VALUE / 2)
                };
                MAX_FACTS
            ],
        );
    }
    let sections = facts.finish();
    assert_partial(&sections);
    assert!(
        sections
            .iter()
            .filter(|s| s.title != "Coverage")
            .map(|s| s.facts.len())
            .sum::<usize>()
            < 2 * MAX_FACTS
    );
    let mut facts = Facts::default();
    for _ in 0..MAX_SECTIONS + 1 {
        facts.push(
            "Section",
            vec![DetailFact {
                label: "Value".into(),
                value: "present".into(),
            }],
        );
    }
    assert_partial(&facts.finish());
    let mut facts = Facts::default();
    for _ in 0..MAX_SECTIONS {
        facts.push(
            "Section",
            vec![DetailFact {
                label: "Value".into(),
                value: "present".into(),
            }],
        );
    }
    // Invalid input after filling all section slots must still get a coverage slot.
    facts.fields(
        "Rejected",
        &json!({"value":"\u{2067}hidden"}),
        &[("Value", "/value")],
    );
    assert_partial(&facts.finish());
}

#[test]
fn invalid_values_are_partial_but_absent_optional_values_are_not() {
    for value in [
        json!("\u{202e}hidden"),
        json!("\u{2069}hidden"),
        json!("é".repeat(513)),
        json!({"unsupported":true}),
    ] {
        assert_partial(&project(
            ResourceKind::Jobs,
            &json!({"spec":{"parallelism":value}}),
        ));
    }
    assert!(
        !project(ResourceKind::Jobs, &json!({}))
            .iter()
            .any(|s| s.title == "Coverage")
    );
}

#[test]
fn sensitive_secret_env_header_and_webhook_ca_sentinels_never_escape() {
    let value = json!({
        "data":{"password":"SECRET_SENTINEL"},"stringData":{"password":"SECRET_SENTINEL"},
        "spec":{"containers":[{"env":[{"value":"ENV_SENTINEL"}],"livenessProbe":{"httpGet":{"httpHeaders":[{"value":"HEADER_SENTINEL"}]}}}]},
        "webhooks":[{"name":"safe.example.test","clientConfig":{"caBundle":"CA_SENTINEL","url":"HEADER_SENTINEL"}}]
    });
    for kind in [
        ResourceKind::Secrets,
        ResourceKind::ConfigMaps,
        ResourceKind::Pods,
        ResourceKind::MutatingWebhookConfigurations,
        ResourceKind::ValidatingWebhookConfigurations,
    ] {
        let encoded = serde_json::to_string(&project(kind, &value)).unwrap();
        for sentinel in [
            "SECRET_SENTINEL",
            "ENV_SENTINEL",
            "HEADER_SENTINEL",
            "CA_SENTINEL",
        ] {
            assert!(!encoded.contains(sentinel), "{kind:?}: {sentinel}");
        }
    }
}
