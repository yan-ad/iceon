use super::*;

async fn transport_fixture(
    deny: bool,
) -> (
    tempfile::TempDir,
    ConnectionEngine,
    PlanRequest,
    std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>,
) {
    use http::Response;
    use std::sync::{Arc, Mutex};
    let (root, _, engine, id, _) = super::super::tests::fixture();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let record = calls.clone();
    let client = Client::new(
        tower::service_fn(move |request: Request<Body>| {
            let calls = record.clone();
            async move {
                let path = request.uri().to_string();
                let method = request.method().to_string();
                calls.lock().unwrap().push((method.clone(), path.clone()));
                let body = request.into_body().collect().await.unwrap().to_bytes();
                let (status, value) = if path == "/version" {
                    (200, json!({"gitVersion":"v1.36"}))
                } else if path.contains("selfsubjectaccessreviews") {
                    let v: Value = serde_json::from_slice(&body).unwrap();
                    (
                        200,
                        json!({"apiVersion":"authorization.k8s.io/v1","kind":"SelfSubjectAccessReview","spec":v["spec"],"status":{"allowed":!deny}}),
                    )
                } else if path == "/api/v1/namespaces/default" {
                    (
                        200,
                        json!({"kind":"Namespace","metadata":{"name":"default"}}),
                    )
                } else if method == "GET" && path.contains("serviceaccounts/alice") {
                    (404, json!({}))
                } else if method == "POST" && path.contains("serviceaccounts") {
                    let mut v: Value = serde_json::from_slice(&body).unwrap();
                    v["metadata"]["uid"] = json!("new-uid");
                    v["metadata"]["resourceVersion"] = json!("1");
                    (201, v)
                } else {
                    panic!("unexpected fixture call {method} {path}")
                };
                Ok::<_, std::io::Error>(
                    Response::builder()
                        .status(status)
                        .body(Body::from(serde_json::to_vec(&value).unwrap()))
                        .unwrap(),
                )
            }
        }),
        "default",
    );
    let scope = super::super::tests::connected(&engine, id, client).await;
    let request = PlanRequest {
        cluster_id: scope.cluster_id,
        session_id: scope.session_id,
        read_id: 1,
        operation: PlanOperation::Create,
        user: user(),
    };
    (root, engine, request, calls)
}

#[tokio::test]
async fn another_engine_refresh_does_not_invalidate_an_approved_plan() {
    let (_root, engine, request, state, _) = stateful_fixture(user(), PlanOperation::Create).await;
    let plan = engine.rbac_user_plan(request.clone()).await.unwrap();
    let (_other_root, other, other_request, _, _) =
        stateful_fixture(user(), PlanOperation::Create).await;
    other
        .refresh_permissions(other_request.scope().session())
        .unwrap();
    engine
        .rbac_user_apply(apply_request(&request, &plan))
        .await
        .unwrap();
    assert!(state.lock().unwrap().real_mutations > 0);
}

#[tokio::test]
async fn approved_plan_rejects_manual_refresh_and_credential_renewal_without_mutations() {
    for manual_refresh in [true, false] {
        let (_root, engine, request, state, calls) =
            stateful_fixture(user(), PlanOperation::Create).await;
        let plan = engine.rbac_user_plan(request.clone()).await.unwrap();
        let scope = request.scope().session();
        let (approved_epoch, lease_cancel) = {
            let session = engine.session.lock().unwrap();
            let session = session.as_ref().unwrap();
            (
                session
                    .rbac_management
                    .plan
                    .as_ref()
                    .unwrap()
                    .authorization_epoch
                    .clone(),
                session.cancel.clone(),
            )
        };
        assert_eq!(approved_epoch.validate(), Ok(()));
        if manual_refresh {
            engine.refresh_permissions(scope.clone()).unwrap();
        } else {
            // Renewal advances this counter after replacing its inner client, not its outer lease.
            super::super::permissions::invalidate_authentication(&engine.authentication_generation);
        }
        assert_eq!(
            approved_epoch.validate(),
            Err(ConnectionError::StaleSession)
        );
        assert!(!lease_cancel.is_cancelled());
        {
            let session = engine.session.lock().unwrap();
            let session = session.as_ref().unwrap();
            assert_eq!(session.snapshot.cluster_id, scope.cluster_id);
            assert_eq!(session.snapshot.session_id, scope.session_id);
            assert!(session.client.is_some());
        }
        let calls_before_apply = calls.lock().unwrap().len();
        assert_eq!(
            engine.rbac_user_apply(apply_request(&request, &plan)).await,
            Err(ConnectionError::StaleSession)
        );
        assert_eq!(calls.lock().unwrap().len(), calls_before_apply);
        assert_eq!(state.lock().unwrap().real_mutations, 0);
    }
}

#[tokio::test]
async fn approved_plan_expiry_uses_backend_monotonic_deadline() {
    let (_root, engine, request, state, calls) =
        stateful_fixture(user(), PlanOperation::Create).await;
    let plan = engine.rbac_user_plan(request.clone()).await.unwrap();
    {
        let mut session = engine.session.lock().unwrap();
        let stored = session
            .as_mut()
            .unwrap()
            .rbac_management
            .plan
            .as_mut()
            .unwrap();
        stored.plan.expires_at = u64::MAX;
        stored.deadline = tokio::time::Instant::now() - Duration::from_secs(1);
    }
    let calls_before_apply = calls.lock().unwrap().len();
    assert_eq!(
        engine.rbac_user_apply(apply_request(&request, &plan)).await,
        Err(ConnectionError::StaleSession)
    );
    assert_eq!(calls.lock().unwrap().len(), calls_before_apply);
    assert_eq!(state.lock().unwrap().real_mutations, 0);
}

#[test]
fn template_rules_require_exclusive_resource_or_non_resource_fields() {
    for rule in [
        json!({"verbs":["get"]}),
        json!({"apiGroups":[""],"resources":["pods"],"nonResourceURLs":["/healthz"],"verbs":["get"]}),
        json!({"apiGroups":[""],"nonResourceURLs":["/healthz"],"verbs":["get"]}),
        json!({"resourceNames":["pod"],"nonResourceURLs":["/healthz"],"verbs":["get"]}),
        json!({"resources":["pods"],"verbs":["get"]}),
        json!({"apiGroups":[""],"resources":["pods"],"verbs":[]}),
        json!({"nonResourceURLs":["healthz"],"verbs":["get"]}),
    ] {
        let value = json!({"kind":"ClusterRole","metadata":{"name":format!("{CLUSTER_PREFIX}reader"),"uid":"role-uid","resourceVersion":"1"},"rules":[rule]});
        assert_eq!(
            template(&value),
            Err(ConnectionError::InvalidResponse),
            "{value}"
        );
    }
    for rule in [
        json!({"apiGroups":[""],"resources":["pods/log"],"resourceNames":["pod"],"verbs":["get"]}),
        json!({"nonResourceURLs":["/healthz"],"verbs":["get"]}),
    ] {
        let value = json!({"kind":"ClusterRole","metadata":{"name":format!("{CLUSTER_PREFIX}reader"),"uid":"role-uid","resourceVersion":"1"},"rules":[rule]});
        assert!(template(&value).is_ok(), "{value}");
    }
}

#[tokio::test]
async fn plan_dry_run_precedes_real_create_and_token_is_one_shot() {
    let (_root, engine, request, calls) = transport_fixture(false).await;
    let plan = engine.rbac_user_plan(request.clone()).await.unwrap();
    assert!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| m == "POST")
            .all(|(_, p)| p.contains("selfsubjectaccessreviews") || p.ends_with("dryRun=All"))
    );
    let apply = ApplyRequest {
        cluster_id: request.cluster_id,
        session_id: request.session_id,
        read_id: 2,
        plan_id: plan.plan_id,
    };
    assert!(
        engine
            .rbac_user_apply(apply.clone())
            .await
            .unwrap()
            .complete
    );
    assert_eq!(
        engine.rbac_user_apply(apply).await,
        Err(ConnectionError::NotFound)
    );
    let calls = calls.lock().unwrap();
    let posts: Vec<_> = calls
        .iter()
        .filter(|(m, p)| m == "POST" && p.contains("serviceaccounts"))
        .collect();
    assert_eq!(posts.len(), 2);
    assert!(posts[0].1.contains("dryRun=All"));
    assert!(!posts[1].1.contains("dryRun"));
}

#[tokio::test]
async fn denied_plan_submits_zero_mutations() {
    let (_root, engine, request, calls) = transport_fixture(true).await;
    assert_eq!(
        engine.rbac_user_plan(request).await,
        Err(ConnectionError::PermissionDenied)
    );
    assert!(
        !calls
            .lock()
            .unwrap()
            .iter()
            .any(|(m, p)| m != "GET" && !p.contains("selfsubjectaccessreviews"))
    );
}

fn user() -> ManagedUser {
    ManagedUser {
        name: "alice".into(),
        namespace: "default".into(),
        uid: None,
        resource_version: None,
        assignments: vec![],
        cluster_access: ClusterAccess::None,
        cluster_template: None,
    }
}
#[test]
fn validates_explicit_namespace() {
    let mut u = user();
    u.namespace.clear();
    assert_eq!(validate_user(&u), Err(ConnectionError::InvalidInput));
}
#[test]
fn rejects_injected_name() {
    let mut u = user();
    u.name = "alice/../secrets".into();
    assert_eq!(validate_user(&u), Err(ConnectionError::InvalidInput));
}
#[test]
fn accepts_dns_subdomain() {
    let mut u = user();
    u.name = "alice.example".into();
    assert!(validate_user(&u).is_ok());
}
#[test]
fn rejects_empty_dns_label() {
    let mut u = user();
    u.name = "alice..example".into();
    assert!(validate_user(&u).is_err());
}
#[test]
fn deterministic_binding_names() {
    let u = user();
    let a = binding_name(&u, "role", "default");
    assert_eq!(a, binding_name(&u, "role", "default"));
    assert_ne!(a, binding_name(&u, "role", "other"));
    assert!(a.len() <= 63);
}
#[test]
fn service_account_disables_automount() {
    let op = account_operation(&user(), &PlanOperation::Create, None).unwrap();
    assert_eq!(op.body["automountServiceAccountToken"], false);
    assert!(op.body.get("secrets").is_none());
    assert!(op.body.get("tokens").is_none());
}
#[test]
fn exact_policy_fields_preserved() {
    let r = json!({"kind":"ClusterRole","metadata":{"name":format!("{NS_PREFIX}reader"),"uid":"u","resourceVersion":"1"},"rules":[{"apiGroups":[""],"resources":["pods/log"],"verbs":["get"],"resourceNames":["p"]},{"verbs":["get"],"nonResourceURLs":["/healthz"]}]});
    let t = template(&r).unwrap();
    assert_eq!(t.rules[1].non_resource_urls, vec!["/healthz"]);
    assert_eq!(t.rules[0].api_groups, vec![""]);
}
#[test]
fn refuses_truncated_rules() {
    let r = json!({"kind":"ClusterRole","metadata":{"name":format!("{NS_PREFIX}reader"),"uid":"u","resourceVersion":"1"},"rules":vec![json!({});33]});
    assert_eq!(template(&r), Err(ConnectionError::LimitExceeded));
}
#[test]
fn refuses_extra_subject() {
    let mut u = user();
    u.uid = Some("sa".into());
    let b = binding(&u, "role", Some("default".into()));
    let mut value = b.body.clone();
    value["metadata"]["uid"] = json!("binding");
    value["metadata"]["resourceVersion"] = json!("1");
    value["subjects"]
        .as_array_mut()
        .unwrap()
        .push(json!({"kind":"User","name":"unrelated"}));
    assert_eq!(
        verify_binding(&value, &b, &u),
        Err(ConnectionError::Conflict)
    );
}
#[test]
fn namespaced_assignments_emit_each_selected_namespace() {
    let mut u = user();
    u.assignments.push(Assignment {
        template_name: format!("{NS_PREFIX}reader"),
        template_uid: "u".into(),
        template_resource_version: "1".into(),
        namespaces: vec!["one".into(), "two".into()],
        all_namespaces: false,
    });
    let bs = bindings(&u).unwrap();
    assert_eq!(bs.len(), 2);
    assert_eq!(bs[1].step.namespace.as_deref(), Some("two"));
}
#[test]
fn cluster_access_requires_exact_template() {
    let mut u = user();
    u.cluster_access = ClusterAccess::Read;
    assert!(validate_user(&u).is_err());
}
#[test]
fn encodes_role_names() {
    assert_eq!(encode("system:reader"), "system%3Areader");
}
#[test]
fn saved_annotation_is_json_string() {
    let u = user();
    let mut value = account_operation(&u, &PlanOperation::Create, None)
        .unwrap()
        .body;
    value["metadata"]["uid"] = json!("u");
    value["metadata"]["resourceVersion"] = json!("1");
    assert_eq!(saved_user(&value).unwrap().uid.as_deref(), Some("u"));
}

type RecordedCalls = std::sync::Arc<std::sync::Mutex<Vec<(String, String, Value)>>>;

#[derive(Default)]
struct ApiState {
    objects: std::collections::HashMap<String, Value>,
    fail_real_mutation: Option<usize>,
    real_mutations: usize,
    cancel_after_mutation: bool,
}

async fn stateful_fixture(
    mut request_user: ManagedUser,
    operation: PlanOperation,
) -> (
    tempfile::TempDir,
    ConnectionEngine,
    PlanRequest,
    std::sync::Arc<std::sync::Mutex<ApiState>>,
    RecordedCalls,
) {
    use std::sync::{Arc, Mutex};
    let (root, _, engine, id, _) = super::super::tests::fixture();
    let state = Arc::new(Mutex::new(ApiState::default()));
    if operation != PlanOperation::Create {
        request_user.uid = Some("account-uid".into());
        request_user.resource_version = Some("1".into());
        let op = account_operation(&request_user, &PlanOperation::Create, None).unwrap();
        let mut account = op.body.clone();
        account["metadata"]["uid"] = json!("account-uid");
        account["metadata"]["resourceVersion"] = json!("1");
        state
            .lock()
            .unwrap()
            .objects
            .insert(op.path(false), account);
        for binding in bindings(&request_user).unwrap() {
            let mut value = binding.body.clone();
            value["metadata"]["uid"] = json!("binding-uid");
            value["metadata"]["resourceVersion"] = json!("1");
            state
                .lock()
                .unwrap()
                .objects
                .insert(binding.path(false), value);
        }
    }
    for assignment in &request_user.assignments {
        let name = &assignment.template_name;
        state.lock().unwrap().objects.insert(
            format!("/apis/rbac.authorization.k8s.io/v1/clusterroles/{}", encode(name)),
            json!({"kind":"ClusterRole","metadata":{"name":name,"uid":assignment.template_uid,"resourceVersion":assignment.template_resource_version},"rules":[{"apiGroups":[""],"resources":["pods"],"verbs":["get","list","watch"]}]}),
        );
    }
    let calls: RecordedCalls = Arc::new(Mutex::new(Vec::new()));
    let recorded = calls.clone();
    let server = state.clone();
    let client = Client::new(
        tower::service_fn(move |request: Request<Body>| {
            let state = server.clone();
            let calls = recorded.clone();
            async move {
                let path = request.uri().to_string();
                let method = request.method().to_string();
                let cancellation = request.extensions().get::<CancellationToken>().cloned();
                let bytes = request.into_body().collect().await.unwrap().to_bytes();
                let body: Value = if bytes.is_empty() {
                    Value::Null
                } else {
                    serde_json::from_slice(&bytes).unwrap()
                };
                calls
                    .lock()
                    .unwrap()
                    .push((method.clone(), path.clone(), body.clone()));
                let mut state = state.lock().unwrap();
                let (status, value) = if path == "/version" {
                    (200, json!({"gitVersion":"v1.36"}))
                } else if path.contains("selfsubjectaccessreviews") {
                    (
                        200,
                        json!({"apiVersion":"authorization.k8s.io/v1","kind":"SelfSubjectAccessReview","spec":body["spec"],"status":{"allowed":true}}),
                    )
                } else if path == "/api/v1/namespaces/default" {
                    (
                        200,
                        json!({"kind":"Namespace","metadata":{"name":"default"}}),
                    )
                } else if method == "GET" {
                    state
                        .objects
                        .get(&path)
                        .cloned()
                        .map(|v| (200, v))
                        .unwrap_or((404, json!({})))
                } else {
                    let dry = path.ends_with("?dryRun=All");
                    if !dry {
                        state.real_mutations += 1;
                    }
                    if !dry && state.fail_real_mutation == Some(state.real_mutations) {
                        (403, json!({}))
                    } else if method == "DELETE" {
                        if !dry {
                            state.objects.remove(path.split('?').next().unwrap());
                        }
                        (200, json!({"kind":"Status","status":"Success"}))
                    } else {
                        let mut result = body;
                        result["metadata"]["uid"] = json!(if result["kind"] == "ServiceAccount" {
                            "account-uid"
                        } else {
                            "binding-uid"
                        });
                        result["metadata"]["resourceVersion"] = json!("1");
                        if !dry {
                            let object_path = if method == "POST" {
                                format!("{}/{}", path, result["metadata"]["name"].as_str().unwrap())
                            } else {
                                path.clone()
                            };
                            state.objects.insert(object_path, result.clone());
                        }
                        (200, result)
                    }
                };
                if state.cancel_after_mutation && state.real_mutations > 0 {
                    cancellation.unwrap().cancel();
                }
                Ok::<_, std::io::Error>(
                    http::Response::builder()
                        .status(status)
                        .body(Body::from(serde_json::to_vec(&value).unwrap()))
                        .unwrap(),
                )
            }
        }),
        "default",
    );
    let scope = super::super::tests::connected(&engine, id, client).await;
    (
        root,
        engine,
        PlanRequest {
            cluster_id: scope.cluster_id,
            session_id: scope.session_id,
            read_id: 1,
            operation,
            user: request_user,
        },
        state,
        calls,
    )
}

fn assigned_user() -> ManagedUser {
    let mut value = user();
    value.assignments.push(Assignment {
        template_name: format!("{NS_PREFIX}reader"),
        template_uid: "role-uid".into(),
        template_resource_version: "1".into(),
        namespaces: vec!["default".into(), "other".into()],
        all_namespaces: false,
    });
    value
}

fn apply_request(request: &PlanRequest, plan: &UserPlan) -> ApplyRequest {
    ApplyRequest {
        cluster_id: request.cluster_id.clone(),
        session_id: request.session_id.clone(),
        read_id: 2,
        plan_id: plan.plan_id.clone(),
    }
}

#[tokio::test]
async fn baseline_identity_drift_prevents_every_real_mutation() {
    for target in ["account", "template", "binding"] {
        for field in ["uid", "resourceVersion"] {
            let (_root, engine, request, state, _calls) =
                stateful_fixture(assigned_user(), PlanOperation::Update).await;
            let plan = engine.rbac_user_plan(request.clone()).await.unwrap();
            let path = match target {
                "account" => account_operation(&request.user, &PlanOperation::Update, None)
                    .unwrap()
                    .path(false),
                "template" => format!(
                    "/apis/rbac.authorization.k8s.io/v1/clusterroles/{}",
                    encode(&request.user.assignments[0].template_name)
                ),
                _ => bindings(&request.user).unwrap()[0].path(false),
            };
            state.lock().unwrap().objects.get_mut(&path).unwrap()["metadata"][field] =
                json!("changed");
            let response = engine
                .rbac_user_apply(apply_request(&request, &plan))
                .await
                .unwrap();
            assert!(!response.complete, "{target} {field}");
            assert_eq!(
                response.results[0].message_code, "Conflict",
                "{target} {field}"
            );
            assert_eq!(state.lock().unwrap().real_mutations, 0, "{target} {field}");
        }
    }
}

#[tokio::test]
async fn unrelated_binding_subject_blocks_plan_without_dry_run_or_mutation() {
    let (_root, engine, request, state, calls) =
        stateful_fixture(assigned_user(), PlanOperation::Update).await;
    let path = bindings(&request.user).unwrap()[0].path(false);
    state.lock().unwrap().objects.get_mut(&path).unwrap()["subjects"]
        .as_array_mut()
        .unwrap()
        .push(json!({"kind":"User","name":"unrelated"}));
    assert_eq!(
        engine.rbac_user_plan(request).await,
        Err(ConnectionError::Conflict)
    );
    assert_eq!(state.lock().unwrap().real_mutations, 0);
    assert!(
        calls
            .lock()
            .unwrap()
            .iter()
            .all(|(method, path, _)| method == "GET" || path.contains("selfsubjectaccessreviews"))
    );
}

#[tokio::test]
async fn second_step_failure_stops_without_rolling_back_created_account() {
    let (_root, engine, request, state, calls) =
        stateful_fixture(assigned_user(), PlanOperation::Create).await;
    let plan = engine.rbac_user_plan(request.clone()).await.unwrap();
    assert_eq!(plan.steps.len(), 3);
    assert_eq!(state.lock().unwrap().real_mutations, 0);
    let dry_calls: Vec<_> = calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, p, _)| p.ends_with("?dryRun=All"))
        .cloned()
        .collect();
    assert_eq!(dry_calls.len(), 3);
    state.lock().unwrap().fail_real_mutation = Some(2);
    let apply = apply_request(&request, &plan);
    let response = engine.rbac_user_apply(apply.clone()).await.unwrap();
    assert!(!response.complete);
    assert_eq!(
        response
            .results
            .iter()
            .map(|r| r.state.as_str())
            .collect::<Vec<_>>(),
        vec!["applied", "failed", "notSubmitted"]
    );
    assert_eq!(response.results[1].message_code, "PermissionDenied");
    assert_eq!(state.lock().unwrap().real_mutations, 2);
    assert!(
        state
            .lock()
            .unwrap()
            .objects
            .contains_key("/api/v1/namespaces/default/serviceaccounts/alice")
    );
    assert!(!calls.lock().unwrap().iter().any(|(m, _, _)| m == "DELETE"));
    assert_eq!(
        engine.rbac_user_apply(apply).await,
        Err(ConnectionError::NotFound)
    );
}

#[tokio::test]
async fn delete_dry_runs_every_step_and_uses_exact_preconditions() {
    let (_root, engine, request, state, calls) =
        stateful_fixture(assigned_user(), PlanOperation::Delete).await;
    let plan = engine.rbac_user_plan(request.clone()).await.unwrap();
    assert_eq!(plan.steps.len(), 3);
    assert_eq!(state.lock().unwrap().real_mutations, 0);
    let dry: Vec<_> = calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(m, p, _)| m == "DELETE" && p.ends_with("?dryRun=All"))
        .cloned()
        .collect();
    assert_eq!(dry.len(), 3);
    for (_, _, body) in &dry {
        assert_eq!(body["preconditions"]["resourceVersion"], "1");
        assert!(body["preconditions"]["uid"].as_str().is_some());
    }
    assert!(
        engine
            .rbac_user_apply(apply_request(&request, &plan))
            .await
            .unwrap()
            .complete
    );
    let real: Vec<_> = calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(m, p, _)| m == "DELETE" && !p.contains('?'))
        .cloned()
        .collect();
    assert_eq!(real.len(), 3);
    assert!(real[2].1.contains("serviceaccounts/alice"));
    for (dry, real) in dry.iter().zip(&real) {
        assert_eq!(dry.2, real.2);
    }
}

#[test]
fn account_update_drops_credential_references_and_disables_automount() {
    let u = user();
    let mut old = account_operation(&u, &PlanOperation::Create, None)
        .unwrap()
        .body;
    old["metadata"]["uid"] = json!("uid");
    old["metadata"]["resourceVersion"] = json!("1");
    old["metadata"]["managedFields"] = json!([{"manager":"foreign"}]);
    old["secrets"] = json!([{"name":"private-token"}]);
    old["imagePullSecrets"] = json!([{"name":"registry-secret"}]);
    old["automountServiceAccountToken"] = json!(true);
    let updated = account_operation(&u, &PlanOperation::Update, Some(&old)).unwrap();
    assert_eq!(updated.body["automountServiceAccountToken"], false);
    assert!(updated.body.get("secrets").is_none());
    assert!(updated.body.get("imagePullSecrets").is_none());
    assert!(updated.body["metadata"].get("managedFields").is_none());
    assert_eq!(old["automountServiceAccountToken"], true);
}

#[tokio::test]
async fn cancellation_consumes_approved_plan_without_mutations() {
    let (_root, engine, request, state, _calls) =
        stateful_fixture(user(), PlanOperation::Create).await;
    let plan = engine.rbac_user_plan(request.clone()).await.unwrap();
    engine
        .cancel_rbac_management(CancelRequest {
            cluster_id: request.cluster_id.clone(),
            session_id: request.session_id.clone(),
            read_id: request.read_id,
        })
        .unwrap();
    assert_eq!(
        engine.rbac_user_apply(apply_request(&request, &plan)).await,
        Err(ConnectionError::NotFound)
    );
    assert_eq!(state.lock().unwrap().real_mutations, 0);
}

#[tokio::test]
async fn snapshot_uses_managed_selector_filters_roles_and_never_exports_raw_account() {
    let mut account = account_operation(&user(), &PlanOperation::Create, None)
        .unwrap()
        .body;
    account["metadata"]["uid"] = json!("server-uid");
    account["metadata"]["resourceVersion"] = json!("9");
    account["secrets"] = json!([{"name":"private-token-reference"}]);
    account["token"] = json!("fixture-token-never-export");
    account["metadata"]["annotations"]["private"] = json!("fixture-sensitive-annotation");
    let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = calls.clone();
    let client = Client::new(
        tower::service_fn(move |request: Request<Body>| {
            let calls = recorded.clone();
            let account = account.clone();
            async move {
                let path = request.uri().to_string();
                assert_eq!(request.method(), Method::GET);
                calls.lock().unwrap().push(path.clone());
                let value = if path.starts_with("/apis/rbac.authorization.k8s.io/v1/clusterroles?")
                {
                    json!({"kind":"ClusterRoleList","metadata":{},"items":[
                        {"kind":"ClusterRole","metadata":{"name":"cluster-admin","uid":"foreign","resourceVersion":"1"},"rules":[{"verbs":["*"]}]},
                        {"kind":"ClusterRole","metadata":{"name":format!("{NS_PREFIX}reader"),"uid":"reader","resourceVersion":"2"},"rules":[{"apiGroups":[""],"resources":["pods/log"],"verbs":["get"]}]}
                    ]})
                } else {
                    assert_eq!(
                        path,
                        "/api/v1/namespaces/default/serviceaccounts?limit=100&labelSelector=app.kubernetes.io%2Fmanaged-by%3Diceon-permission-manager"
                    );
                    json!({"kind":"ServiceAccountList","metadata":{},"items":[account]})
                };
                Ok::<_, std::io::Error>(
                    http::Response::builder()
                        .status(200)
                        .body(Body::from(serde_json::to_vec(&value).unwrap()))
                        .unwrap(),
                )
            }
        }),
        "default",
    );
    let snapshot = fetch_snapshot(
        &client,
        &ManagementRequest {
            cluster_id: "fixture".into(),
            session_id: "session".into(),
            read_id: 1,
            namespace: "default".into(),
        },
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(snapshot.templates.len(), 1);
    assert_eq!(snapshot.templates[0].rules[0].resources, vec!["pods/log"]);
    assert_eq!(snapshot.users.len(), 1);
    assert_eq!(snapshot.users[0].uid.as_deref(), Some("server-uid"));
    assert!(!snapshot.truncated);
    assert_eq!(calls.lock().unwrap().len(), 2);
    let serialized = serde_json::to_string(&snapshot).unwrap();
    for sensitive in [
        "private-token-reference",
        "fixture-token-never-export",
        "fixture-sensitive-annotation",
        "cluster-admin",
    ] {
        assert!(!serialized.contains(sensitive));
    }
}

#[test]
fn saved_identity_comes_from_server_not_annotation() {
    let mut value = account_operation(&user(), &PlanOperation::Create, None)
        .unwrap()
        .body;
    value["metadata"]["name"] = json!("server-name");
    value["metadata"]["namespace"] = json!("server-namespace");
    value["metadata"]["uid"] = json!("server-uid");
    value["metadata"]["resourceVersion"] = json!("7");
    let saved = saved_user(&value).unwrap();
    assert_eq!(saved.name, "server-name");
    assert_eq!(saved.namespace, "server-namespace");
    assert_eq!(saved.uid.as_deref(), Some("server-uid"));
    assert_eq!(saved.resource_version.as_deref(), Some("7"));
}

#[tokio::test]
async fn cluster_read_checks_server_verbs_but_write_preserves_actual_rules() {
    for (access, rule, accepted) in [
        (
            ClusterAccess::Read,
            json!({"apiGroups":[""],"resources":["nodes","namespaces"],"verbs":["get","list","watch"]}),
            true,
        ),
        (
            ClusterAccess::Read,
            json!({"apiGroups":[""],"resources":["pods/log"],"verbs":["get"]}),
            true,
        ),
        (
            ClusterAccess::Read,
            json!({"nonResourceURLs":["/healthz"],"verbs":["get"]}),
            true,
        ),
        (
            ClusterAccess::Read,
            json!({"apiGroups":[""],"resources":["pods"],"verbs":["get","create"]}),
            false,
        ),
        (
            ClusterAccess::Read,
            json!({"apiGroups":[""],"resources":["pods/exec"],"verbs":["get"]}),
            false,
        ),
        (
            ClusterAccess::Read,
            json!({"apiGroups":[""],"resources":["pods/attach"],"verbs":["get"]}),
            false,
        ),
        (
            ClusterAccess::Read,
            json!({"apiGroups":[""],"resources":["pods/portforward"],"verbs":["get"]}),
            false,
        ),
        (
            ClusterAccess::Read,
            json!({"apiGroups":[""],"resources":["nodes/proxy"],"verbs":["get"]}),
            false,
        ),
        (
            ClusterAccess::Read,
            json!({"apiGroups":["*"],"resources":["*"],"verbs":["*"]}),
            false,
        ),
        (
            ClusterAccess::Read,
            json!({"apiGroups":["*"],"resources":["pods"],"verbs":["get"]}),
            false,
        ),
        (
            ClusterAccess::Read,
            json!({"apiGroups":[""],"resources":["*"],"verbs":["get"]}),
            false,
        ),
        (
            ClusterAccess::Read,
            json!({"apiGroups":[""],"resources":["pods/*"],"verbs":["get"]}),
            false,
        ),
        (
            ClusterAccess::Read,
            json!({"apiGroups":[""],"resources":["pods"],"verbs":["*"]}),
            false,
        ),
        (
            ClusterAccess::Read,
            json!({"nonResourceURLs":["/healthz"],"verbs":["list"]}),
            false,
        ),
        (
            ClusterAccess::Write,
            json!({"apiGroups":[""],"resources":["pods/exec"],"verbs":["get"]}),
            true,
        ),
        (
            ClusterAccess::Write,
            json!({"apiGroups":["*"],"resources":["*"],"verbs":["*"]}),
            true,
        ),
    ] {
        let mut u = user();
        u.cluster_access = access;
        u.cluster_template = Some(TemplateIdentity {
            name: format!("{CLUSTER_PREFIX}chosen"),
            uid: "role-uid".into(),
            resource_version: "1".into(),
        });
        let name = u.cluster_template.as_ref().unwrap().name.clone();
        assert_eq!(
            validate_rule(&serde_json::from_value::<Rule>(rule.clone()).unwrap()),
            Ok(()),
            "{rule}"
        );
        let rules = json!([rule]);
        let served_rules = rules.clone();
        let client = Client::new(
            tower::service_fn(move |request: Request<Body>| {
                let name = name.clone();
                let rules = served_rules.clone();
                async move {
                    assert_eq!(request.method(), Method::GET);
                    assert_eq!(
                        request.uri().path(),
                        format!(
                            "/apis/rbac.authorization.k8s.io/v1/clusterroles/{}",
                            encode(&name)
                        )
                    );
                    let body = json!({"kind":"ClusterRole","metadata":{"name":name,"uid":"role-uid","resourceVersion":"1"},"rules":rules});
                    Ok::<_, std::io::Error>(
                        http::Response::builder()
                            .status(200)
                            .body(Body::from(serde_json::to_vec(&body).unwrap()))
                            .unwrap(),
                    )
                }
            }),
            "default",
        );
        let result = load_templates(&client, &u, &CancellationToken::new()).await;
        if accepted {
            let templates = result.unwrap();
            assert_eq!(
                templates[0].rules[0],
                serde_json::from_value::<Rule>(rules[0].clone()).unwrap()
            );
            let summary = summarize(&u, &templates);
            assert_eq!(summary[0].verbs, templates[0].rules[0].verbs);
        } else {
            assert_eq!(result, Err(ConnectionError::InvalidInput), "{rules}");
        }
    }
}

#[tokio::test]
async fn cancellation_after_server_accepts_post_keeps_partial_results_and_consumes_plan() {
    let (_root, engine, request, state, calls) =
        stateful_fixture(assigned_user(), PlanOperation::Create).await;
    let plan = engine.rbac_user_plan(request.clone()).await.unwrap();
    state.lock().unwrap().cancel_after_mutation = true;
    let apply = apply_request(&request, &plan);
    let response = engine.rbac_user_apply(apply.clone()).await.unwrap();
    assert!(!response.complete);
    assert_eq!(response.cluster_id, request.cluster_id);
    assert_eq!(response.session_id, request.session_id);
    assert_eq!(response.plan_id, plan.plan_id);
    assert_eq!(state.lock().unwrap().real_mutations, 1);
    assert!(
        state
            .lock()
            .unwrap()
            .objects
            .contains_key("/api/v1/namespaces/default/serviceaccounts/alice")
    );
    assert!(
        response
            .results
            .iter()
            .any(|result| result.state == "failed" && result.message_code == "StaleSession")
    );
    assert_eq!(response.results.last().unwrap().state, "notSubmitted");
    assert!(
        !calls
            .lock()
            .unwrap()
            .iter()
            .any(|(method, _, _)| method == "DELETE")
    );
    assert_eq!(
        engine.rbac_user_apply(apply).await,
        Err(ConnectionError::NotFound)
    );
}
