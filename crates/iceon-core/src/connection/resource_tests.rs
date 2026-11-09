use super::tests::{connected, fake, fixture, normal, resource};
use super::*;
use serde_json::json;

#[test]
fn fixed_endpoints_cover_every_resource_kind_and_wire_spelling() {
    let kinds = [
        (ResourceKind::Nodes, "nodes", "/api/v1/nodes"),
        (ResourceKind::Namespaces, "namespaces", "/api/v1/namespaces"),
        (ResourceKind::Pods, "pods", "/api/v1/pods"),
        (
            ResourceKind::ReplicationControllers,
            "replicationControllers",
            "/api/v1/replicationcontrollers",
        ),
        (
            ResourceKind::VerticalPodAutoscalers,
            "verticalPodAutoscalers",
            "/apis/autoscaling.k8s.io/v1/verticalpodautoscalers",
        ),
        (
            ResourceKind::Leases,
            "leases",
            "/apis/coordination.k8s.io/v1/leases",
        ),
        (
            ResourceKind::MutatingWebhookConfigurations,
            "mutatingWebhookConfigurations",
            "/apis/admissionregistration.k8s.io/v1/mutatingwebhookconfigurations",
        ),
        (
            ResourceKind::ValidatingWebhookConfigurations,
            "validatingWebhookConfigurations",
            "/apis/admissionregistration.k8s.io/v1/validatingwebhookconfigurations",
        ),
        (
            ResourceKind::ValidatingAdmissionPolicies,
            "validatingAdmissionPolicies",
            "/apis/admissionregistration.k8s.io/v1/validatingadmissionpolicies",
        ),
        (
            ResourceKind::ValidatingAdmissionPolicyBindings,
            "validatingAdmissionPolicyBindings",
            "/apis/admissionregistration.k8s.io/v1/validatingadmissionpolicybindings",
        ),
        (
            ResourceKind::EndpointSlices,
            "endpointSlices",
            "/apis/discovery.k8s.io/v1/endpointslices",
        ),
        (
            ResourceKind::PodSecurityPolicies,
            "podSecurityPolicies",
            "/apis/policy/v1beta1/podsecuritypolicies",
        ),
        (
            ResourceKind::Deployments,
            "deployments",
            "/apis/apps/v1/deployments",
        ),
        (
            ResourceKind::DaemonSets,
            "daemonSets",
            "/apis/apps/v1/daemonsets",
        ),
        (
            ResourceKind::StatefulSets,
            "statefulSets",
            "/apis/apps/v1/statefulsets",
        ),
        (
            ResourceKind::ReplicaSets,
            "replicaSets",
            "/apis/apps/v1/replicasets",
        ),
        (ResourceKind::Jobs, "jobs", "/apis/batch/v1/jobs"),
        (
            ResourceKind::CronJobs,
            "cronJobs",
            "/apis/batch/v1/cronjobs",
        ),
        (ResourceKind::ConfigMaps, "configMaps", "/api/v1/configmaps"),
        (ResourceKind::Secrets, "secrets", "/api/v1/secrets"),
        (
            ResourceKind::ResourceQuotas,
            "resourceQuotas",
            "/api/v1/resourcequotas",
        ),
        (
            ResourceKind::LimitRanges,
            "limitRanges",
            "/api/v1/limitranges",
        ),
        (
            ResourceKind::HorizontalPodAutoscalers,
            "horizontalPodAutoscalers",
            "/apis/autoscaling/v2/horizontalpodautoscalers",
        ),
        (
            ResourceKind::PodDisruptionBudgets,
            "podDisruptionBudgets",
            "/apis/policy/v1/poddisruptionbudgets",
        ),
        (
            ResourceKind::PriorityClasses,
            "priorityClasses",
            "/apis/scheduling.k8s.io/v1/priorityclasses",
        ),
        (
            ResourceKind::RuntimeClasses,
            "runtimeClasses",
            "/apis/node.k8s.io/v1/runtimeclasses",
        ),
        (ResourceKind::Services, "services", "/api/v1/services"),
        (ResourceKind::Endpoints, "endpoints", "/api/v1/endpoints"),
        (
            ResourceKind::Ingresses,
            "ingresses",
            "/apis/networking.k8s.io/v1/ingresses",
        ),
        (
            ResourceKind::IngressClasses,
            "ingressClasses",
            "/apis/networking.k8s.io/v1/ingressclasses",
        ),
        (
            ResourceKind::NetworkPolicies,
            "networkPolicies",
            "/apis/networking.k8s.io/v1/networkpolicies",
        ),
        (
            ResourceKind::PersistentVolumes,
            "persistentVolumes",
            "/api/v1/persistentvolumes",
        ),
        (
            ResourceKind::PersistentVolumeClaims,
            "persistentVolumeClaims",
            "/api/v1/persistentvolumeclaims",
        ),
        (
            ResourceKind::StorageClasses,
            "storageClasses",
            "/apis/storage.k8s.io/v1/storageclasses",
        ),
        (ResourceKind::Events, "events", "/api/v1/events"),
        (
            ResourceKind::ServiceAccounts,
            "serviceAccounts",
            "/api/v1/serviceaccounts",
        ),
        (
            ResourceKind::Roles,
            "roles",
            "/apis/rbac.authorization.k8s.io/v1/roles",
        ),
        (
            ResourceKind::RoleBindings,
            "roleBindings",
            "/apis/rbac.authorization.k8s.io/v1/rolebindings",
        ),
        (
            ResourceKind::ClusterRoles,
            "clusterRoles",
            "/apis/rbac.authorization.k8s.io/v1/clusterroles",
        ),
        (
            ResourceKind::ClusterRoleBindings,
            "clusterRoleBindings",
            "/apis/rbac.authorization.k8s.io/v1/clusterrolebindings",
        ),
        (
            ResourceKind::CustomResourceDefinitions,
            "customResourceDefinitions",
            "/apis/apiextensions.k8s.io/v1/customresourcedefinitions",
        ),
    ];
    for (kind, wire, path) in kinds {
        assert_eq!(serde_json::to_value(kind).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<ResourceKind>(json!(wire)).unwrap(),
            kind
        );
        let mut request = request(kind);
        assert_eq!(
            transport::resource_path(&request),
            format!(
                "{path}?limit={}&timeoutSeconds=10",
                if kind == ResourceKind::CustomResourceDefinitions {
                    10
                } else {
                    100
                }
            )
        );
        request.namespace = Some("team-a".into());
        assert_eq!(
            transport::validate_resource_request(&request).is_ok(),
            resources::namespaced(kind)
        );
    }
}

#[path = "crd_fetch_tests.rs"]
mod crd_fetch_tests;

#[test]
fn workload_facts_use_observed_current_and_spec_desired_values() {
    let metadata = json!({"name":"example","namespace":"team"});
    let deployment = resources::project(
        ResourceKind::Deployments,
        json!({"metadata":metadata,"spec":{"replicas":3},"status":{"replicas":2}}),
    )
    .unwrap();
    assert_eq!(deployment.columns.get("desired"), Some(&"3".into()));
    assert_eq!(deployment.columns.get("replicas"), Some(&"2".into()));

    let daemon = resources::project(
        ResourceKind::DaemonSets,
        json!({"metadata":{"name":"example","namespace":"team"},"spec":{"replicas":99},"status":{"desiredNumberScheduled":4,"currentNumberScheduled":3,"updatedNumberScheduled":2,"numberAvailable":1}}),
    )
    .unwrap();
    assert_eq!(daemon.columns.get("desired"), Some(&"4".into()));
    assert_eq!(daemon.columns.get("scheduled"), Some(&"3".into()));
    assert_eq!(daemon.columns.get("updated"), Some(&"2".into()));
    assert_eq!(daemon.columns.get("available"), Some(&"1".into()));
    assert!(!daemon.columns.contains_key("replicas"));
}

#[test]
fn cron_jobs_project_safe_schedule_and_active_length_only() {
    let row = resources::project(
        ResourceKind::CronJobs,
        json!({"metadata":{"name":"nightly","namespace":"team"},"spec":{"schedule":"0 3 * * *","timeZone":"Etc/UTC","suspend":false,"jobTemplate":{"PRIVATE":"PRIVATE"}},"status":{"active":[{"name":"job-a"},{"name":"job-b"}],"lastScheduleTime":"2026-01-01T03:00:00Z"}}),
    )
    .unwrap();
    assert_eq!(row.columns.get("schedule"), Some(&"0 3 * * *".into()));
    assert_eq!(row.columns.get("timeZone"), Some(&"Etc/UTC".into()));
    assert_eq!(row.columns.get("active"), Some(&"2".into()));
    assert_eq!(
        row.columns.get("lastSchedule"),
        Some(&"2026-01-01T03:00:00Z".into())
    );
    assert_eq!(row.columns.get("suspend"), Some(&"false".into()));
    assert!(!serde_json::to_string(&row).unwrap().contains("PRIVATE"));

    let invalid = resources::project(
        ResourceKind::CronJobs,
        json!({"metadata":{"name":"nightly","namespace":"team"},"spec":{"schedule":"x"},"status":{"active":[],"lastScheduleTime":"not-a-time"}}),
    )
    .unwrap();
    assert_eq!(invalid.columns.get("active"), Some(&"0".into()));
    assert!(!invalid.columns.contains_key("lastSchedule"));
    let bounded = resources::project(
        ResourceKind::CronJobs,
        json!({"metadata":{"name":"nightly","namespace":"team"},"spec":{"schedule":"é\n".repeat(300),"timeZone":"Etc/UTC\u{0000}","suspend":"PRIVATE"},"status":{"active":[{"name":"PRIVATE","namespace":"PRIVATE"}],"message":"PRIVATE"}}),
    )
    .unwrap();
    assert_eq!(bounded.columns.get("schedule"), Some(&"é".repeat(128)));
    assert_eq!(bounded.columns.get("timeZone"), Some(&"Etc/UTC".into()));
    assert_eq!(bounded.columns.get("active"), Some(&"1".into()));
    assert!(!bounded.columns.contains_key("suspend"));
    assert!(!serde_json::to_string(&bounded).unwrap().contains("PRIVATE"));
}

#[test]
fn jobs_and_pods_keep_missing_and_zero_facts_honest() {
    let job = resources::project(
        ResourceKind::Jobs,
        json!({"metadata":{"name":"job","namespace":"team"},"spec":{"completions":0,"parallelism":0,"suspend":true},"status":{"succeeded":0,"failed":0,"active":0}}),
    )
    .unwrap();
    assert_eq!(job.columns.get("desiredCompletions"), Some(&"0".into()));
    assert_eq!(job.columns.get("parallelism"), Some(&"0".into()));
    assert_eq!(job.columns.get("suspend"), Some(&"true".into()));
    assert_eq!(job.columns.get("active"), Some(&"0".into()));

    let pod = resources::project(
        ResourceKind::Pods,
        json!({"metadata":{"name":"pod","namespace":"team"},"spec":{"containers":[{"name":"a"},{"name":"b"}]},"status":{"phase":"Pending","containerStatuses":[]}}),
    )
    .unwrap();
    assert_eq!(pod.columns.get("ready"), Some(&"0/2".into()));

    let missing = resources::project(
        ResourceKind::Pods,
        json!({"metadata":{"name":"pod","namespace":"team"},"status":{"phase":"Pending"}}),
    )
    .unwrap();
    assert!(!missing.columns.contains_key("ready"));
    for kind in [
        ResourceKind::Deployments,
        ResourceKind::DaemonSets,
        ResourceKind::Jobs,
        ResourceKind::CronJobs,
    ] {
        let missing = resources::project(
            kind,
            json!({"metadata":{"name":"example","namespace":"team"}}),
        )
        .unwrap();
        assert!(missing.columns.is_empty(), "{kind:?}");
        assert_eq!(missing.status, "Unknown");
    }
    for kind in [
        ResourceKind::Deployments,
        ResourceKind::StatefulSets,
        ResourceKind::ReplicaSets,
        ResourceKind::ReplicationControllers,
    ] {
        let zero = resources::project(kind, json!({"metadata":{"name":"example","namespace":"team"},"spec":{"replicas":0},"status":{"replicas":0}})).unwrap();
        assert_eq!(zero.columns.get("replicas"), Some(&"0".into()));
        assert_eq!(zero.columns.get("desired"), Some(&"0".into()));
    }
}
fn request(kind: ResourceKind) -> ResourceRequest {
    ResourceRequest {
        cluster_id: "fixture".into(),
        session_id: "lease".into(),
        kind,
        read_id: 1,
        namespace: None,
        continue_token: None,
        query: None,
        status: None,
        group: None,
    }
}
#[test]
fn namespace_and_continuation_are_validated_and_encoded() {
    let mut r = request(ResourceKind::Pods);
    for ns in [
        "",
        "../secrets",
        "a/b",
        "a?x=y",
        "A",
        "-a",
        "a-",
        "a.b",
        "a\n",
    ] {
        r.namespace = Some(ns.into());
        assert_eq!(
            transport::validate_resource_request(&r),
            Err(ConnectionError::InvalidInput)
        );
    }
    r.namespace = Some("team-a".into());
    r.continue_token = Some("a+/=?&x#é".into());
    assert!(transport::validate_resource_request(&r).is_ok());
    assert_eq!(
        transport::resource_path(&r),
        "/api/v1/namespaces/team-a/pods?limit=100&timeoutSeconds=10&continue=a%2B%2F%3D%3F%26x%23%C3%A9"
    );
    for token in ["x".repeat(4097), "x\n".into()] {
        r.continue_token = Some(token);
        assert!(transport::validate_resource_request(&r).is_err());
    }
    assert!(
        serde_json::from_value::<ResourceRequest>(
            json!({"clusterId":"x","sessionId":"y","readId":1,"kind":"pods","url":"https://bad"})
        )
        .is_err()
    );
}
#[test]
fn quantities_are_cores_and_bytes_with_binary_decimal_and_exponents() {
    for (s, n) in [
        ("250m", 0.25),
        ("123456789n", 0.123456789),
        ("10u", 0.00001),
        ("2Gi", 2147483648.0),
        ("1.5Mi", 1572864.0),
        ("2G", 2e9),
        ("1e3", 1000.0),
        ("1E-3", 0.001),
        ("1Ei", 1152921504606846976.0),
    ] {
        assert!((resources::parse_quantity(s).unwrap() - n).abs() <= n * 1e-12);
    }
    for s in [
        "-1", "NaN", "inf", "1e400", "1e3m", "1.2.3", "1K", "1 Mi", "1e", "1e-", "1GiB", "100Ei",
    ] {
        assert_eq!(resources::parse_quantity(s), None, "{s}");
    }
}
#[tokio::test]
async fn metadata_only_negotiation_and_namespace_identity() {
    use http::{Request, Response};
    use kube::client::Body;
    let client = Client::new(
        transport::CancelService(tower::service_fn(|r: Request<Body>| async move {
            assert_eq!(
                r.headers()["accept"],
                "application/json;as=PartialObjectMetadataList;g=meta.k8s.io;v=v1"
            );
            Ok::<_,std::io::Error>(Response::new(Body::from(json!({"kind":"PartialObjectMetadataList","items":[
            {"metadata":{"name":"same","namespace":"a","annotations":{"key":"PRIVATE"}},"data":{"password":"PRIVATE"}},
            {"metadata":{"name":"same","namespace":"b"}}
        ]}).to_string().into_bytes())))
        })),
        "default",
    );
    for kind in [ResourceKind::Secrets, ResourceKind::ConfigMaps] {
        let page = transport::resources(&client, &request(kind), &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(page.items.len(), 2);
        assert!(!serde_json::to_string(&page).unwrap().contains("PRIVATE"));
    }
    let raw = fake(|_| (200, json!({"items":[]}).to_string().into_bytes()));
    assert_eq!(
        transport::resources(
            &raw,
            &request(ResourceKind::Secrets),
            &CancellationToken::new()
        )
        .await,
        Err(ConnectionError::InvalidResponse)
    );
}
fn node() -> serde_json::Value {
    json!({"metadata":{"name":"node-a","labels":{"node-role.kubernetes.io/control-plane":"","arbitrary":"PRIVATE"}},"status":{"nodeInfo":{"kubeletVersion":"v1.36.0"},"capacity":{"cpu":"2","memory":"2Gi"},"allocatable":{"cpu":"1500m","memory":"1Gi"},"conditions":[{"type":"Ready","status":"True"}]},"spec":{"unschedulable":true}})
}
#[tokio::test]
async fn node_metrics_states_preserve_inventory_and_never_invent_disk() {
    for (status, body, expected) in [
        (404, json!({}), MetricState::Unavailable),
        (403, json!({}), MetricState::Denied),
        (401, json!({}), MetricState::Denied),
        (500, json!({}), MetricState::Error),
        (200, json!({"items":[]}), MetricState::Partial),
        (
            200,
            json!({"items":[{"metadata":{"name":"node-a"},"timestamp":"2000-01-01T00:00:00Z","usage":{"cpu":"10m","memory":"1Ki"}}]}),
            MetricState::Stale,
        ),
        (
            200,
            json!({"items":[{"metadata":{"name":"node-a"},"timestamp":"invalid","usage":{"cpu":"10m","memory":"1Ki"}}]}),
            MetricState::Partial,
        ),
        (
            200,
            json!({"items":[{"metadata":{"name":"node-a"},"timestamp":jiff::Timestamp::now().to_string(),"usage":{"cpu":"3000000000n","memory":"3Gi"}}]}),
            MetricState::Available,
        ),
    ] {
        let client = fake(move |path| {
            if path.starts_with("/apis/metrics") {
                (status, body.to_string().into_bytes())
            } else {
                (200, json!({"items":[node()]}).to_string().into_bytes())
            }
        });
        let page = transport::resources(
            &client,
            &request(ResourceKind::Nodes),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(page.metrics, Some(expected));
        assert_eq!(page.items[0].status, "Ready");
        let n = page.items[0].node.as_ref().unwrap();
        assert_eq!(n.cpu.allocatable, Some(1.5));
        assert_eq!(n.memory.capacity, Some(2147483648.0));
        assert_eq!(n.roles, ["control-plane"]);
        assert_eq!(
            n.cpu.used,
            if expected == MetricState::Available {
                Some(3.0)
            } else {
                None
            }
        );
        assert_eq!(n.disk_allocatable, None);
        let wire = serde_json::to_value(n).unwrap();
        assert!(wire["diskAllocatable"].is_null());
        assert!(wire.get("disk").is_none());
    }
}
// The engine and buffered client can run on different runtimes; keep their deadlines on real time.
#[tokio::test]
async fn metrics_timeout_is_optional_but_cancellation_discards_late_metrics() {
    use http::{Request, Response};
    use kube::client::Body;
    let started = Arc::new(tokio::sync::Notify::new());
    let marker = started.clone();
    let client = Client::new(
        transport::CancelService(tower::service_fn(move |r: Request<Body>| {
            let marker = marker.clone();
            async move {
                if r.uri().path().starts_with("/apis/metrics") {
                    marker.notify_one();
                    return std::future::pending().await;
                }
                let bytes = if r.uri().path() == "/version" {
                    normal("/version").1
                } else {
                    json!({"items":[node()]}).to_string().into_bytes()
                };
                Ok::<_, std::io::Error>(Response::new(Body::from(bytes)))
            }
        })),
        "default",
    );
    let (_root, _catalog, engine, id, _) = fixture();
    let s = connected(&engine, id, client).await;
    let r = resource(&s, ResourceKind::Nodes);
    let (result, ()) = tokio::join!(engine.resources(r.clone()), async {
        started.notified().await;
        engine.cancel_resources(r.clone()).unwrap();
    });
    assert_eq!(result, Err(ConnectionError::StaleSession));
    let mut r = r;
    r.read_id = 2;
    let page = engine.resources(r).await.unwrap();
    assert_eq!(page.metrics, Some(MetricState::Error));
    assert_eq!(page.items.len(), 1);
}
#[test]
fn representative_projection_has_only_bounded_allowed_scalars() {
    let metadata = json!({"name":"example","namespace":"team","creationTimestamp":"2026-01-01T00:00:00Z","annotations":{"password":"PRIVATE"}});
    for (kind, extra, key, expected) in [
        (
            ResourceKind::Pods,
            json!({"status":{"phase":"Running","qosClass":"Guaranteed","containerStatuses":[{"ready":true,"restartCount":2}]},"spec":{"nodeName":"worker","containers":[{"env":["PRIVATE"]}]}}),
            "ready",
            "1/1",
        ),
        (
            ResourceKind::Deployments,
            json!({"status":{"readyReplicas":3,"updatedReplicas":2,"availableReplicas":3}}),
            "updated",
            "2",
        ),
        (
            ResourceKind::Services,
            json!({"spec":{"type":"ClusterIP","clusterIP":"10.0.0.1","ports":[{"port":443}]}}),
            "ports",
            "443",
        ),
        (
            ResourceKind::PersistentVolumeClaims,
            json!({"spec":{"storageClassName":"fast","accessModes":["ReadWriteOnce"]},"status":{"phase":"Bound","capacity":{"storage":"5Gi"}}}),
            "capacity",
            "5Gi",
        ),
        (
            ResourceKind::RoleBindings,
            json!({"roleRef":{"name":"reader"},"subjects":[{"name":"PRIVATE"}]}),
            "subjects",
            "1",
        ),
        (
            ResourceKind::Events,
            json!({"type":"Warning","reason":"Failed","count":3,"message":"Container could not start","involvedObject":{"kind":"Pod","name":"example"}}),
            "count",
            "3",
        ),
        (
            ResourceKind::CustomResourceDefinitions,
            json!({"spec":{"group":"apps.example.io","scope":"Namespaced","names":{"kind":"Widget","plural":"widgets","shortNames":["wdg"]},"versions":[{"name":"v1"}]}}),
            "group",
            "apps.example.io",
        ),
        (
            ResourceKind::Leases,
            json!({"spec":{"holderIdentity":"controller-a","leaseDurationSeconds":15,"leaseTransitions":2,"renewTime":"2026-01-01T00:00:00Z"}}),
            "holder",
            "controller-a",
        ),
    ] {
        let mut v = extra;
        v["metadata"] = metadata.clone();
        let row = resources::project(kind, v).unwrap();
        assert_eq!(row.columns[key], expected);
        assert!(row.columns.len() <= 12);
        assert!(row.columns.values().all(|v| v.len() <= 256));
        assert!(!serde_json::to_string(&row).unwrap().contains("PRIVATE"));
    }
}
#[tokio::test]
async fn bad_continuation_missing_namespace_and_resource_404_fail_closed() {
    for (status, body, error) in [
        (404, json!({}), ConnectionError::ResourceUnavailable),
        (
            200,
            json!({"items":[{"metadata":{"name":"example"}}]}),
            ConnectionError::InvalidResponse,
        ),
        (
            200,
            json!({"items":[],"metadata":{"continue":"x\n"}}),
            ConnectionError::InvalidResponse,
        ),
    ] {
        let client = fake(move |_| (status, body.to_string().into_bytes()));
        assert_eq!(
            transport::resources(
                &client,
                &request(ResourceKind::Pods),
                &CancellationToken::new()
            )
            .await,
            Err(error)
        );
    }
}

#[test]
fn enriched_table_columns_are_explicit_bounded_and_do_not_serialize_nested_data() {
    use ResourceKind::*;
    for (kind, extra, expected) in [
        (
            Namespaces,
            json!({"metadata":{"labels":{"private":"PRIVATE"}},"status":{"phase":"Active"}}),
            vec![("labels", "1")],
        ),
        (
            ResourceQuotas,
            json!({"status":{"hard":{"requests.cpu":"2","requests.memory":"1Gi","PRIVATE":"PRIVATE"},"used":{"requests.cpu":"500m"}}}),
            vec![("cpuHard", "2"), ("memoryHard", "1Gi"), ("cpuUsed", "500m")],
        ),
        (
            LimitRanges,
            json!({"spec":{"limits":[{"type":"Container","min":{"cpu":"10m"},"default":{"memory":"1Gi"},"PRIVATE":"PRIVATE"}]}}),
            vec![
                ("limits", "1"),
                ("types", "Container"),
                ("minCpu", "10m"),
                ("defaultMemory", "1Gi"),
            ],
        ),
        (
            HorizontalPodAutoscalers,
            json!({"spec":{"scaleTargetRef":{"name":"app","kind":"Deployment"},"minReplicas":1,"maxReplicas":5},"status":{"currentReplicas":2,"desiredReplicas":3}}),
            vec![
                ("target", "app"),
                ("min", "1"),
                ("max", "5"),
                ("current", "2"),
                ("desired", "3"),
            ],
        ),
        (
            VerticalPodAutoscalers,
            json!({"spec":{"targetRef":{"name":"app","kind":"Deployment"},"updatePolicy":{"updateMode":"Auto"}},"status":{"recommendation":{"containerRecommendations":[{"containerName":"PRIVATE"}]}}}),
            vec![
                ("target", "app"),
                ("updateMode", "Auto"),
                ("recommendations", "1"),
            ],
        ),
        (
            PodDisruptionBudgets,
            json!({"spec":{"minAvailable":"50%"},"status":{"disruptionsAllowed":2}}),
            vec![("minAvailable", "50%"), ("allowedDisruptions", "2")],
        ),
        (
            PriorityClasses,
            json!({"value":-10,"globalDefault":false,"description":"PRIVATE"}),
            vec![("value", "-10"), ("globalDefault", "false")],
        ),
        (
            RuntimeClasses,
            json!({"handler":"runc","overhead":{"podFixed":{"cpu":"100m"}},"scheduling":{"nodeSelector":{"PRIVATE":"PRIVATE"},"tolerations":[{"value":"PRIVATE"}]}}),
            vec![
                ("handler", "runc"),
                ("overheadCpu", "100m"),
                ("nodeSelectors", "1"),
                ("tolerations", "1"),
            ],
        ),
        (
            Endpoints,
            json!({"subsets":[{"addresses":[{"ip":"PRIVATE"}],"notReadyAddresses":[{"ip":"PRIVATE"}],"ports":[{"port":443}]}]}),
            vec![
                ("subsets", "1"),
                ("readyAddresses", "1"),
                ("notReadyAddresses", "1"),
                ("ports", "1"),
            ],
        ),
        (
            EndpointSlices,
            json!({"addressType":"IPv4","endpoints":[{"addresses":["PRIVATE"],"conditions":{"ready":true}}],"ports":[{"port":443}]}),
            vec![
                ("addressType", "IPv4"),
                ("endpoints", "1"),
                ("ports", "1"),
                ("ready", "1"),
            ],
        ),
        (
            Ingresses,
            json!({"spec":{"ingressClassName":"nginx","rules":[{"host":"PRIVATE"}],"tls":[{"secretName":"PRIVATE"}]}}),
            vec![("class", "nginx"), ("rules", "1"), ("tls", "1")],
        ),
        (
            IngressClasses,
            json!({"spec":{"controller":"example.io/controller","parameters":{"kind":"Config","name":"settings"}}}),
            vec![
                ("controller", "example.io/controller"),
                ("parametersKind", "Config"),
                ("parametersName", "settings"),
            ],
        ),
        (
            NetworkPolicies,
            json!({"spec":{"policyTypes":["Ingress","Egress"],"ingress":[{"PRIVATE":"PRIVATE"}],"podSelector":{"matchLabels":{"PRIVATE":"PRIVATE"}}}}),
            vec![
                ("policyTypes", "Ingress,Egress"),
                ("ingressRules", "1"),
                ("matchLabels", "1"),
            ],
        ),
        (
            PodSecurityPolicies,
            json!({"spec":{"privileged":false,"runAsUser":{"rule":"MustRunAsNonRoot"},"seLinux":{"rule":"RunAsAny","seLinuxOptions":{"user":"PRIVATE"}}}}),
            vec![
                ("privileged", "false"),
                ("runAsUser", "MustRunAsNonRoot"),
                ("seLinux", "RunAsAny"),
            ],
        ),
        (
            PersistentVolumes,
            json!({"spec":{"capacity":{"storage":"5Gi"},"storageClassName":"fast","persistentVolumeReclaimPolicy":"Retain","csi":{"volumeHandle":"PRIVATE"}}}),
            vec![
                ("capacity", "5Gi"),
                ("storageClass", "fast"),
                ("reclaim", "Retain"),
            ],
        ),
        (
            StorageClasses,
            json!({"provisioner":"example.io/storage","volumeBindingMode":"WaitForFirstConsumer","allowVolumeExpansion":true,"parameters":{"PRIVATE":"PRIVATE"}}),
            vec![
                ("provisioner", "example.io/storage"),
                ("bindingMode", "WaitForFirstConsumer"),
                ("allowExpansion", "true"),
            ],
        ),
        (
            ServiceAccounts,
            json!({"secrets":[{"name":"PRIVATE"}],"imagePullSecrets":[{"name":"PRIVATE"}],"automountServiceAccountToken":false}),
            vec![
                ("secrets", "1"),
                ("imagePullSecrets", "1"),
                ("automountToken", "false"),
            ],
        ),
        (
            MutatingWebhookConfigurations,
            json!({"webhooks":[{"failurePolicy":"Fail","sideEffects":"None","clientConfig":{"url":"PRIVATE","caBundle":"PRIVATE"}}]}),
            vec![
                ("webhooks", "1"),
                ("failurePolicies", "Fail"),
                ("sideEffects", "None"),
            ],
        ),
        (
            ValidatingWebhookConfigurations,
            json!({"webhooks":[{"matchPolicy":"Equivalent","clientConfig":{"url":"PRIVATE"}}]}),
            vec![("webhooks", "1"), ("matchPolicies", "Equivalent")],
        ),
        (
            ValidatingAdmissionPolicies,
            json!({"spec":{"failurePolicy":"Fail","validations":[{"expression":"PRIVATE","message":"PRIVATE"}],"matchConditions":[{"expression":"PRIVATE"}]}}),
            vec![
                ("failurePolicy", "Fail"),
                ("validations", "1"),
                ("matchConditions", "1"),
            ],
        ),
        (
            ValidatingAdmissionPolicyBindings,
            json!({"spec":{"policyName":"test-policy","validationActions":["Deny","Audit"]}}),
            vec![
                ("policy", "test-policy"),
                ("validationActions", "Deny,Audit"),
            ],
        ),
        (
            ReplicationControllers,
            json!({"spec":{"replicas":3,"template":{"PRIVATE":"PRIVATE"}},"status":{"replicas":2,"readyReplicas":2,"availableReplicas":2}}),
            vec![
                ("replicas", "2"),
                ("desired", "3"),
                ("ready", "2"),
                ("available", "2"),
            ],
        ),
    ] {
        let mut v = extra;
        if v.get("metadata").is_none() {
            v["metadata"] = json!({});
        }
        v["metadata"]["name"] = json!("example");
        v["metadata"]["annotations"] = json!({"PRIVATE":"PRIVATE"});
        if resources::namespaced(kind) {
            v["metadata"]["namespace"] = json!("team");
        }
        let row = resources::project(kind, v).unwrap();
        for (key, value) in expected {
            assert_eq!(
                row.columns.get(key).map(String::as_str),
                Some(value),
                "{kind:?}: {key}"
            );
        }
        assert!(row.columns.len() <= 12);
        assert!(
            row.columns
                .values()
                .all(|s| s.len() <= 256 && !s.chars().any(char::is_control))
        );
        assert!(
            !serde_json::to_string(&row).unwrap().contains("PRIVATE"),
            "{kind:?}"
        );
    }
    let row = resources::project(CustomResourceDefinitions,json!({"metadata":{"name":"widgets.example.io"},"spec":{"group":"example.io","scope":"Namespaced","names":{"kind":"Widget","plural":"widgets","shortNames":["w","wdg"]},"versions":[{"name":"v1","schema":{"PRIVATE":"PRIVATE"}},{"name":"v2"}]}})).unwrap();
    assert_eq!(row.columns["versions"], "v1,v2");
    assert_eq!(row.columns["shortNames"], "w,wdg");
    let row = resources::project(
        RuntimeClasses,
        json!({"metadata":{"name":"test"},"handler":"é\n".repeat(500)}),
    )
    .unwrap();
    assert!(row.columns["handler"].len() <= 256);
    assert!(!row.columns["handler"].contains('\n'));
}

#[tokio::test]
async fn removed_pod_security_policy_api_is_unavailable_and_never_falls_back() {
    let client = fake(|path| {
        assert!(path.starts_with("/apis/policy/v1beta1/podsecuritypolicies?"));
        (404, b"PRIVATE".to_vec())
    });
    assert_eq!(
        transport::resources(
            &client,
            &request(ResourceKind::PodSecurityPolicies),
            &CancellationToken::new()
        )
        .await,
        Err(ConnectionError::ResourceUnavailable)
    );
}

#[tokio::test]
async fn filtered_scan_finds_second_page_and_combines_status_group() {
    let client = fake(|path| {
        let second = path.contains("continue=next");
        (200, serde_json::to_vec(&json!({"metadata":{"continue":if second { "" } else { "next" }},"items":[{
            "metadata":{"name":if second { "widgets.example.io" } else { "other.example.io" }},
            "spec":{"group":"example.io","scope":"Namespaced","versions":[{"name":"v1"}]},
            "status":{"conditions":[{"type":"Established","status":"True"}]}
        }]})).unwrap())
    });
    let mut r = request(ResourceKind::CustomResourceDefinitions);
    r.query = Some("WIDGET".into());
    r.group = Some("example.io".into());
    let baseline = resources::project(ResourceKind::CustomResourceDefinitions, json!({"metadata":{"name":"x"},"spec":{"group":"example.io"},"status":{"conditions":[{"type":"Established","status":"True"}]}})).unwrap();
    r.status = Some(baseline.status);
    let page = transport::resources(&client, &r, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].name, "widgets.example.io");
    assert!(!page.truncated);
    r.group = Some("missing.io".into());
    assert!(
        transport::resources(&client, &r, &CancellationToken::new())
            .await
            .unwrap()
            .items
            .is_empty()
    );
    r.group = None;
    r.status = Some("not-an-observed-status".into());
    assert!(
        transport::resources(&client, &r, &CancellationToken::new())
            .await
            .unwrap()
            .items
            .is_empty()
    );
}

#[tokio::test]
async fn filtered_scan_bounds_pages_and_can_resume_without_matches() {
    let client = fake(|path| {
        let index = path
            .split("continue=")
            .nth(1)
            .unwrap_or("0")
            .parse::<usize>()
            .unwrap();
        (200, serde_json::to_vec(&json!({"metadata":{"continue":(index + 1).to_string()},"items":[{"metadata":{"name":format!("ns-{index}")}}]})).unwrap())
    });
    let mut r = request(ResourceKind::Namespaces);
    r.query = Some("ns-10".into());
    let page = transport::resources(&client, &r, &CancellationToken::new())
        .await
        .unwrap();
    assert!(page.items.is_empty());
    assert!(page.truncated);
    assert_eq!(page.continue_token.as_deref(), Some("10"));
    r.continue_token = page.continue_token;
    let resumed = transport::resources(&client, &r, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(resumed.items[0].name, "ns-10");
}

#[tokio::test]
async fn filtered_scan_reduces_page_size_instead_of_dropping_matches() {
    let client = fake(|path| {
        let second = path.contains("continue=next");
        let count = if second {
            assert!(path.contains("limit=1&"));
            1
        } else {
            99
        };
        let items = (0..count)
            .map(|i| json!({"metadata":{"name":format!("hit-{}-{i}", usize::from(second))}}))
            .collect::<Vec<_>>();
        (
            200,
            serde_json::to_vec(
                &json!({"metadata":{"continue":if second {"last"} else {"next"}},"items":items}),
            )
            .unwrap(),
        )
    });
    let mut r = request(ResourceKind::Namespaces);
    r.query = Some("hit".into());
    let page = transport::resources(&client, &r, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(page.items.len(), 100);
    assert_eq!(page.continue_token.as_deref(), Some("last"));
}

#[tokio::test]
async fn filtered_scan_cancellation_and_invalid_filters_fail_closed() {
    let mut r = request(ResourceKind::Pods);
    for value in ["".to_string(), "\n".to_string(), "x".repeat(257)] {
        r.query = Some(value);
        assert_eq!(
            transport::validate_resource_request(&r),
            Err(ConnectionError::InvalidInput)
        );
    }
    r.query = None;
    r.group = Some("example.io".into());
    assert_eq!(
        transport::validate_resource_request(&r),
        Err(ConnectionError::InvalidInput)
    );
    r.group = None;
    r.query = Some("hit".into());
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert_eq!(
        transport::resources(
            &fake(|_| panic!("cancelled scan must not fetch")),
            &r,
            &cancel
        )
        .await,
        Err(ConnectionError::StaleSession)
    );
    assert!(serde_json::from_value::<ResourceRequest>(
        json!({"clusterId":"x","sessionId":"y","kind":"pods","readId":1,"filters":{"query":"hit"}})
    )
    .is_err());
}

#[tokio::test]
async fn filtered_scan_time_bound_retains_next_unscanned_page() {
    let client = Client::new(
        transport::CancelService(tower::service_fn(
            |request: http::Request<kube::client::Body>| async move {
                if request.uri().to_string().contains("continue=next") {
                    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
                }
                Ok::<_, std::io::Error>(http::Response::new(kube::client::Body::from(
                    serde_json::to_vec(&json!({"metadata":{"continue":"next"},"items":[]}))
                        .unwrap(),
                )))
            },
        )),
        "default",
    );
    let mut r = request(ResourceKind::Namespaces);
    r.query = Some("hit".into());
    let page = transport::resources(&client, &r, &CancellationToken::new())
        .await
        .unwrap();
    assert!(page.items.is_empty());
    assert!(page.truncated);
    assert_eq!(page.continue_token.as_deref(), Some("next"));
}

#[test]
fn pod_health_distinguishes_ready_completed_warning_and_shutdown() {
    let base = json!({"metadata":{"name":"pod","namespace":"team"},"spec":{"containers":[{"name":"app"}]},"status":{"phase":"Running","conditions":[{"type":"Ready","status":"True"}],"containerStatuses":[{"name":"app","ready":true,"state":{"running":{}}}]}});
    let project = |value| resources::project(ResourceKind::Pods, value).unwrap();
    let healthy = project(base.clone());
    assert_eq!(healthy.columns["health"], "healthy");
    assert_eq!(healthy.columns["podReady"], "true");
    assert!(!healthy.columns.contains_key("warning"));

    for (phase, state, code) in [
        (
            "Running",
            json!({"waiting":{"reason":"CrashLoopBackOff"}}),
            "ContainerWaiting",
        ),
        (
            "Running",
            json!({"terminated":{"exitCode":1}}),
            "ContainerFailed",
        ),
        ("Running", json!({"running":{}}), "ContainersNotReady"),
        ("Failed", json!({"terminated":{"exitCode":1}}), "PodFailed"),
        ("Pending", json!({}), "PodPending"),
    ] {
        let mut value = base.clone();
        value["status"]["phase"] = json!(phase);
        value["status"]["conditions"][0]["status"] = json!("False");
        value["status"]["containerStatuses"][0]["ready"] = json!(false);
        value["status"]["containerStatuses"][0]["state"] = state;
        let row = project(value);
        assert_eq!(row.columns["health"], "warning");
        assert_eq!(row.columns["warning"], code);
    }
    let mut completed = base.clone();
    completed["status"]["phase"] = json!("Succeeded");
    completed["status"]["conditions"][0]["status"] = json!("False");
    completed["status"]["containerStatuses"][0]["ready"] = json!(false);
    completed["status"]["containerStatuses"][0]["state"] = json!({"terminated":{"exitCode":0}});
    let completed = project(completed);
    assert_eq!(completed.columns["health"], "completed");
    assert!(!completed.columns.contains_key("warning"));

    let mut deleting = base.clone();
    deleting["metadata"]["deletionTimestamp"] = json!("2026-10-26T12:00:00Z");
    let row = project(deleting);
    assert_eq!(row.status, "Terminating");
    assert_eq!(row.columns["health"], "terminating");
    assert_eq!(row.columns["terminating"], "true");
    assert_eq!(row.columns["deletionTimestamp"], "2026-10-26T12:00:00Z");

    let missing =
        project(json!({"metadata":{"name":"pod","namespace":"team"},"status":{"phase":"Running"}}));
    assert_eq!(missing.columns["health"], "unknown");
    assert_eq!(missing.columns["podReady"], "unknown");

    let mut gate = base.clone();
    gate["status"]["conditions"][0]["status"] = json!("False");
    gate["spec"]["readinessGates"] = json!([{"conditionType":"example.test/ready"}]);
    assert_eq!(project(gate).columns["warning"], "ReadinessGateNotReady");

    let mut sidecar = base;
    sidecar["spec"]["initContainers"] =
        json!([{"name":"init"},{"name":"sidecar","restartPolicy":"Always"}]);
    sidecar["status"]["initContainerStatuses"] =
        json!([{"name":"init","ready":false,"state":{"terminated":{"exitCode":0}}}]);
    assert_eq!(project(sidecar).columns["warning"], "ContainersNotReady");
}

#[test]
fn pod_health_is_bounded_and_never_copies_server_messages_or_container_secrets() {
    let marker = "PRIVATE-CREDENTIAL";
    let row = resources::project(ResourceKind::Pods, json!({
        "metadata":{"name":"pod","namespace":"team","deletionTimestamp":marker,"annotations":{"secret":marker}},
        "spec":{"containers":[{"name":marker,"env":[{"name":"TOKEN","value":marker}]}],"nodeName":"é\n".repeat(300)},
        "status":{"phase":"Running","reason":marker,"message":marker,"conditions":[{"type":"Ready","status":"False","reason":marker,"message":marker}],"containerStatuses":[{"name":marker,"ready":false,"state":{"waiting":{"reason":marker,"message":marker}}}]}
    })).unwrap();
    assert_eq!(row.status, "Terminating");
    assert_eq!(row.columns["warning"], "ContainerWaiting");
    assert!(!row.columns.contains_key("deletionTimestamp"));
    assert!(row.columns.len() <= 12);
    assert!(
        row.columns
            .values()
            .all(|value| value.len() <= 256 && !value.chars().any(char::is_control))
    );
    assert!(!serde_json::to_string(&row).unwrap().contains(marker));
}

#[test]
fn event_projection_bounds_messages_and_prefers_reported_occurrences() {
    let row = resources::project(
        ResourceKind::Events,
        json!({
            "metadata":{"name":"event","namespace":"team","annotations":{"secret":"PRIVATE"}},
            "type":"Warning\u{202e}", "reason":"BackOff\n", "message":"é\u{2066}\n".repeat(400),
            "source":{"component":"legacy"}, "reportingComponent":"kubelet\u{200f}",
            "count":1, "series":{"count":9,"lastObservedTime":"2026-10-25T00:00:00Z"},
            "lastTimestamp":"2026-10-24T00:00:00Z", "eventTime":"2026-10-01T00:00:00Z",
            "involvedObject":{"kind":"Pod\u{202e}","name":"example"},
            "data":{"token":"PRIVATE"}, "credentials":{"token":"PRIVATE"}
        }),
    )
    .unwrap();
    assert_eq!(row.status, "Warning");
    assert_eq!(row.columns["reason"], "BackOff");
    assert_eq!(row.columns["message"], "é".repeat(128));
    assert_eq!(row.columns["source"], "kubelet");
    assert_eq!(row.columns["count"], "9");
    assert_eq!(row.columns["lastSeen"], "2026-10-25T00:00:00Z");
    assert_eq!(row.columns["occurrenceTime"], "2026-10-01T00:00:00Z");
    assert_eq!(row.columns["involvedObject"], "Pod/example");
    assert!(row.columns.len() <= 12);
    assert!(!serde_json::to_string(&row).unwrap().contains("PRIVATE"));
}

#[test]
fn event_projection_validates_timestamps_and_preserves_missing_facts() {
    let row = resources::project(
        ResourceKind::Events,
        json!({
            "metadata":{"name":"event","namespace":"team"},
            "source":{"component":"scheduler"}, "count":3,
            "series":{"count":-1,"lastObservedTime":"invalid"},
            "lastTimestamp":"2026-10-24T00:00:00Z", "eventTime":"invalid"
        }),
    )
    .unwrap();
    assert_eq!(row.columns["count"], "3");
    assert_eq!(row.columns["source"], "scheduler");
    assert_eq!(row.columns["lastSeen"], "2026-10-24T00:00:00Z");
    assert!(!row.columns.contains_key("occurrenceTime"));
    assert!(!row.columns.contains_key("message"));
}

#[test]
fn workload_overview_projects_observed_readiness_and_terminal_job_conditions() {
    for kind in [
        ResourceKind::Deployments,
        ResourceKind::ReplicaSets,
        ResourceKind::StatefulSets,
    ] {
        let row = resources::project(kind, json!({"metadata":{"name":"app","namespace":"team"},"spec":{"replicas":3},"status":{"readyReplicas":3}})).unwrap();
        assert_eq!(row.columns["desired"], "3");
        assert_eq!(row.columns["ready"], "3");
    }
    for (condition, expected) in [("Complete", "Completed"), ("Failed", "Failed")] {
        let row = resources::project(ResourceKind::Jobs, json!({"metadata":{"name":"job","namespace":"team"},"status":{"conditions":[{"type":condition,"status":"True"}],"active":0}})).unwrap();
        assert_eq!(row.status, expected);
        assert_eq!(row.columns["active"], "0");
    }
}
