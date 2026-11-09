use super::permissions::{AuthorizationEpoch, PermissionAction, PermissionCheck};
use super::{ConnectionEngine, ConnectionError, ResourceKind, Result, SessionRequest};
use http::{Method, Request};
use http_body_util::BodyExt;
use kube::{Client, client::Body};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio_util::sync::CancellationToken;

const MAX_RULES: usize = 32;
const MAX_TEMPLATES: usize = 64;
const MAX_BODY: usize = 2 * 1024 * 1024;
const MAX_USERS: usize = 100;
const MAX_STEPS: usize = 128;
const MANAGER: &str = "iceon-permission-manager";
const USER: &str = "iceon.yan.ad/managed-user";
const USER_UID: &str = "iceon.yan.ad/managed-user-uid";
const NS_PREFIX: &str = "template-namespaced-resources___";
const CLUSTER_PREFIX: &str = "template-cluster-resources___";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagementRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub read_id: u64,
    pub namespace: String,
}
impl ManagementSlot {
    pub(super) fn is_executing(&self) -> bool {
        self.executing
    }
}

fn validate_rule(rule: &Rule) -> Result<()> {
    let resource = !rule.resources.is_empty();
    let non_resource = !rule.non_resource_urls.is_empty();
    if resource == non_resource
        || rule.verbs.is_empty()
        || rule.verbs.iter().any(|v| !text(v, 253))
        || (resource
            && (rule.api_groups.is_empty() || rule.resources.iter().any(|r| !text(r, 253))))
        || (non_resource && (!rule.api_groups.is_empty() || !rule.resource_names.is_empty()))
        || rule
            .non_resource_urls
            .iter()
            .any(|url| !url.starts_with('/') || !text(url, 2048))
    {
        return Err(ConnectionError::InvalidResponse);
    }
    Ok(())
}

fn plan_warnings(templates: &[ManagedTemplate]) -> Vec<String> {
    let mut warnings = vec!["Existing grants outside this saved assignment set are not managed. Partial execution is not rolled back.".into()];
    if templates.iter().flat_map(|t| &t.rules).any(|r| {
        r.api_groups
            .iter()
            .chain(&r.resources)
            .chain(&r.verbs)
            .chain(&r.non_resource_urls)
            .any(|v| v.contains('*'))
    }) {
        warnings.push("Selected rules contain wildcards that can grant access to current and future API resources or operations.".into());
    }
    warnings
}

fn read_only_rule(rule: &Rule) -> bool {
    if !rule.non_resource_urls.is_empty() {
        return rule.verbs.iter().all(|verb| verb == "get");
    }
    rule.verbs
        .iter()
        .all(|verb| matches!(verb.as_str(), "get" | "list" | "watch"))
        && rule.api_groups.iter().all(|group| !group.contains('*'))
        && rule.resources.iter().all(|resource| {
            !resource.contains('*')
                && !resource
                    .split('/')
                    .any(|segment| matches!(segment, "exec" | "attach" | "portforward" | "proxy"))
        })
}
impl ManagementRequest {
    fn session(&self) -> SessionRequest {
        SessionRequest {
            cluster_id: self.cluster_id.clone(),
            session_id: self.session_id.clone(),
        }
    }
    fn validate(&self) -> Result<()> {
        super::validate_request(&self.session())?;
        super::validate_read_id(self.read_id)?;
        if !super::transport::valid_namespace(&self.namespace) {
            return Err(ConnectionError::InvalidInput);
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CancelRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub read_id: u64,
}
impl CancelRequest {
    fn session(&self) -> SessionRequest {
        SessionRequest {
            cluster_id: self.cluster_id.clone(),
            session_id: self.session_id.clone(),
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedUser {
    pub name: String,
    pub namespace: String,
    pub uid: Option<String>,
    pub resource_version: Option<String>,
    pub assignments: Vec<Assignment>,
    pub cluster_access: ClusterAccess,
    pub cluster_template: Option<TemplateIdentity>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Assignment {
    pub template_name: String,
    pub template_uid: String,
    pub template_resource_version: String,
    pub namespaces: Vec<String>,
    pub all_namespaces: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ClusterAccess {
    None,
    Read,
    Write,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TemplateIdentity {
    pub name: String,
    pub uid: String,
    pub resource_version: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Rule {
    #[serde(default)]
    pub api_groups: Vec<String>,
    #[serde(default)]
    pub resources: Vec<String>,
    #[serde(default)]
    pub verbs: Vec<String>,
    #[serde(default)]
    pub resource_names: Vec<String>,
    #[serde(default, rename = "nonResourceURLs")]
    pub non_resource_urls: Vec<String>,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ManagedTemplate {
    pub name: String,
    pub uid: String,
    pub resource_version: String,
    pub scope: String,
    pub rules: Vec<Rule>,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ManagementSnapshot {
    pub cluster_id: String,
    pub session_id: String,
    pub read_id: u64,
    pub namespace: String,
    pub users: Vec<ManagedUser>,
    pub templates: Vec<ManagedTemplate>,
    pub truncated: bool,
    pub fetched_at: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PlanOperation {
    Create,
    Update,
    Delete,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub read_id: u64,
    pub operation: PlanOperation,
    pub user: ManagedUser,
}
impl PlanRequest {
    fn scope(&self) -> ManagementRequest {
        ManagementRequest {
            cluster_id: self.cluster_id.clone(),
            session_id: self.session_id.clone(),
            read_id: self.read_id,
            namespace: self.user.namespace.clone(),
        }
    }
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlanSummary {
    pub api_groups: Vec<String>,
    pub resources: Vec<String>,
    pub verbs: Vec<String>,
    pub resource_names: Vec<String>,
    #[serde(rename = "nonResourceURLs")]
    pub non_resource_urls: Vec<String>,
    pub namespaces: Option<Vec<String>>,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlanStep {
    pub operation: PlanOperation,
    pub kind: String,
    pub name: String,
    pub namespace: Option<String>,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UserPlan {
    pub cluster_id: String,
    pub session_id: String,
    pub read_id: u64,
    pub plan_id: String,
    pub expires_at: u64,
    pub operation: PlanOperation,
    pub user: ManagedUser,
    pub summary: Vec<PlanSummary>,
    pub steps: Vec<PlanStep>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyRequest {
    pub cluster_id: String,
    pub session_id: String,
    pub read_id: u64,
    pub plan_id: String,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub operation: PlanOperation,
    pub kind: String,
    pub name: String,
    pub namespace: Option<String>,
    pub state: String,
    pub message_code: String,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResponse {
    pub cluster_id: String,
    pub session_id: String,
    pub read_id: u64,
    pub plan_id: String,
    pub operation: PlanOperation,
    pub results: Vec<ApplyResult>,
    pub complete: bool,
}

#[derive(Default)]
pub(super) struct ManagementSlot {
    latest: u64,
    cancel: Option<CancellationToken>,
    plan: Option<StoredPlan>,
    executing: bool,
}
struct StoredPlan {
    authorization_epoch: AuthorizationEpoch,
    plan: UserPlan,
    operations: Vec<Intended>,
    templates: Vec<ManagedTemplate>,
    deadline: tokio::time::Instant,
    retained_bindings: Vec<Intended>,
}
#[derive(Clone)]
struct Intended {
    step: PlanStep,
    body: Value,
    expected: Option<(String, String)>,
}
impl Intended {
    fn kind(&self) -> ResourceKind {
        match self.step.kind.as_str() {
            "ServiceAccount" => ResourceKind::ServiceAccounts,
            "RoleBinding" => ResourceKind::RoleBindings,
            _ => ResourceKind::ClusterRoleBindings,
        }
    }
    fn check(&self) -> PermissionCheck {
        PermissionCheck {
            kind: self.kind(),
            action: match self.step.operation {
                PlanOperation::Create => PermissionAction::Create,
                PlanOperation::Update => PermissionAction::Update,
                PlanOperation::Delete => PermissionAction::Delete,
            },
            namespace: self.step.namespace.clone(),
            name: if self.step.operation == PlanOperation::Create {
                None
            } else {
                Some(self.step.name.clone())
            },
        }
    }
    fn path(&self, collection: bool) -> String {
        let base = if self.kind() == ResourceKind::ServiceAccounts {
            "/api/v1"
        } else {
            "/apis/rbac.authorization.k8s.io/v1"
        };
        let ns = self
            .step
            .namespace
            .as_ref()
            .map(|n| format!("/namespaces/{n}"))
            .unwrap_or_default();
        let name = if collection {
            String::new()
        } else {
            format!("/{}", encode(&self.step.name))
        };
        format!("{base}{ns}/{}{name}", super::create::plural(self.kind()))
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}
fn text(s: &str, max: usize) -> bool {
    !s.is_empty()&&s.len()<=max&&s.trim()==s&&!s.chars().any(|c|c.is_control()||matches!(c,'\u{061c}'|'\u{200e}'|'\u{200f}'|'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}'))
}
fn field(v: &Value, p: &str) -> Result<String> {
    v.pointer(p)
        .and_then(Value::as_str)
        .filter(|s| text(s, 253))
        .map(str::to_owned)
        .ok_or(ConnectionError::InvalidResponse)
}
fn identity(v: &Value) -> Result<(String, String)> {
    Ok((
        field(v, "/metadata/uid")?,
        field(v, "/metadata/resourceVersion")?,
    ))
}
fn dns(s: &str, max: usize) -> bool {
    !s.is_empty()
        && s.len() <= max
        && s.split('.').all(|p| {
            !p.is_empty()
                && p.len() <= 63
                && p.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                && p.as_bytes()[0].is_ascii_alphanumeric()
                && p.as_bytes()[p.len() - 1].is_ascii_alphanumeric()
        })
}
fn role_name(s: &str) -> bool {
    text(s, 253)
        && !matches!(s, "." | "..")
        && !s
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '/' | '%' | '?' | '#'))
}
fn validate_user(u: &ManagedUser) -> Result<()> {
    if !dns(&u.name, 253)
        || !super::transport::valid_namespace(&u.namespace)
        || u.assignments.len() > 32
    {
        return Err(ConnectionError::InvalidInput);
    }
    if u.uid.as_ref().is_some_and(|s| !text(s, 253))
        || u.resource_version.as_ref().is_some_and(|s| !text(s, 253))
    {
        return Err(ConnectionError::InvalidInput);
    }
    let mut names = HashSet::new();
    for a in &u.assignments {
        if !a.template_name.starts_with(NS_PREFIX)
            || !role_name(&a.template_name)
            || !text(&a.template_uid, 253)
            || !text(&a.template_resource_version, 253)
            || !names.insert(&a.template_name)
            || a.namespaces.len() > 32
            || a.all_namespaces == !a.namespaces.is_empty()
        {
            return Err(ConnectionError::InvalidInput);
        }
        let mut namespaces = HashSet::new();
        for ns in &a.namespaces {
            if !super::transport::valid_namespace(ns) || !namespaces.insert(ns) {
                return Err(ConnectionError::InvalidInput);
            }
        }
    }
    match (&u.cluster_access, &u.cluster_template) {
        (ClusterAccess::None, None) => {}
        (ClusterAccess::Read | ClusterAccess::Write, Some(t))
            if t.name.starts_with(CLUSTER_PREFIX)
                && role_name(&t.name)
                && text(&t.uid, 253)
                && text(&t.resource_version, 253) => {}
        _ => return Err(ConnectionError::InvalidInput),
    }
    if serde_json::to_vec(u)
        .map_err(|_| ConnectionError::InvalidInput)?
        .len()
        > 64 * 1024
    {
        return Err(ConnectionError::LimitExceeded);
    }
    Ok(())
}
fn managed(v: &Value) -> bool {
    v.pointer("/metadata/labels/app.kubernetes.io~1managed-by")
        .and_then(Value::as_str)
        == Some(MANAGER)
}
fn saved_user(v: &Value) -> Result<ManagedUser> {
    if !managed(v) || v.get("kind").and_then(Value::as_str) != Some("ServiceAccount") {
        return Err(ConnectionError::Conflict);
    }
    let raw = v
        .pointer("/metadata/annotations/iceon.yan.ad~1managed-user")
        .and_then(Value::as_str)
        .filter(|s| s.len() <= 64 * 1024)
        .ok_or(ConnectionError::InvalidResponse)?;
    let mut u: ManagedUser =
        serde_json::from_str(raw).map_err(|_| ConnectionError::InvalidResponse)?;
    u.name = field(v, "/metadata/name")?;
    u.namespace = field(v, "/metadata/namespace")?;
    let (uid, rv) = identity(v)?;
    u.uid = Some(uid);
    u.resource_version = Some(rv);
    validate_user(&u).map_err(|_| ConnectionError::InvalidResponse)?;
    Ok(u)
}
fn template(v: &Value) -> Result<ManagedTemplate> {
    let name = field(v, "/metadata/name")?;
    let scope = if name.starts_with(NS_PREFIX) {
        "namespaced"
    } else if name.starts_with(CLUSTER_PREFIX) {
        "cluster"
    } else {
        return Err(ConnectionError::InvalidResponse);
    };
    if !role_name(&name) || v.get("kind").and_then(Value::as_str) != Some("ClusterRole") {
        return Err(ConnectionError::InvalidResponse);
    }
    let (uid, rv) = identity(v)?;
    let raw = v.get("rules").cloned().unwrap_or_else(|| json!([]));
    let rules: Vec<Rule> =
        serde_json::from_value(raw).map_err(|_| ConnectionError::InvalidResponse)?;
    if rules.len() > MAX_RULES {
        return Err(ConnectionError::LimitExceeded);
    }
    for r in &rules {
        for a in [
            &r.api_groups,
            &r.resources,
            &r.verbs,
            &r.resource_names,
            &r.non_resource_urls,
        ] {
            if a.len() > 128
                || a.iter()
                    .any(|s| s.len() > 2048 || s.chars().any(char::is_control))
            {
                return Err(ConnectionError::LimitExceeded);
            }
        }
        validate_rule(r).map_err(|_| ConnectionError::InvalidResponse)?;
    }
    Ok(ManagedTemplate {
        name,
        uid,
        resource_version: rv,
        scope: scope.into(),
        rules,
    })
}

impl ConnectionEngine {
    fn begin_management(
        &self,
        identity: &SessionRequest,
        id: u64,
        executing: bool,
    ) -> Result<(Client, CancellationToken)> {
        super::validate_request(identity)?;
        super::validate_read_id(id)?;
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        let s = super::current_mut(&mut state, identity)?;
        let client = s.client.clone().ok_or(ConnectionError::NotConnected)?;
        super::validate_credentials(&client)?;
        let slot = &mut s.rbac_management;
        if slot.executing {
            return Err(ConnectionError::Busy);
        }
        if id <= slot.latest {
            return Err(ConnectionError::StaleSession);
        }
        if let Some(c) = slot.cancel.take() {
            c.cancel();
        }
        slot.latest = id;
        slot.plan = None;
        slot.executing = executing;
        let cancel = s.cancel.child_token();
        slot.cancel = Some(cancel.clone());
        Ok((client, cancel))
    }
    fn finish_management(&self, identity: &SessionRequest, id: u64) -> Result<()> {
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        let s = super::current_mut(&mut state, identity)?;
        if s.rbac_management.latest != id {
            return Err(ConnectionError::StaleSession);
        }
        s.rbac_management.executing = false;
        s.rbac_management.cancel = None;
        Ok(())
    }
    pub fn cancel_rbac_management(&self, r: CancelRequest) -> Result<()> {
        super::validate_request(&r.session())?;
        super::validate_read_id(r.read_id)?;
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        if let Some(s) = state
            .as_mut()
            .filter(|s| super::matches_session(s, &r.session()))
        {
            let slot = &mut s.rbac_management;
            if slot.latest == r.read_id {
                if let Some(c) = &slot.cancel {
                    c.cancel();
                }
                slot.plan = None;
            } else if r.read_id > slot.latest && !slot.executing {
                if let Some(c) = slot.cancel.take() {
                    c.cancel();
                }
                slot.latest = r.read_id;
                slot.plan = None;
            }
        }
        Ok(())
    }
    pub async fn rbac_management_snapshot(
        &self,
        r: ManagementRequest,
    ) -> Result<ManagementSnapshot> {
        r.validate()?;
        let _permit = self
            .workers
            .try_acquire()
            .map_err(|_| ConnectionError::Busy)?;
        let (client, cancel) = self.begin_management(&r.session(), r.read_id, false)?;
        let result = super::bounded(&cancel, fetch_snapshot(&client, &r, &cancel)).await;
        self.finish_management(&r.session(), r.read_id)?;
        if cancel.is_cancelled() {
            return Err(ConnectionError::StaleSession);
        }
        result
    }
    pub async fn rbac_user_plan(&self, r: PlanRequest) -> Result<UserPlan> {
        let scope = r.scope();
        scope.validate()?;
        validate_user(&r.user)?;
        if (r.operation == PlanOperation::Create)
            != (r.user.uid.is_none() && r.user.resource_version.is_none())
            || r.user.uid.is_some() != r.user.resource_version.is_some()
        {
            return Err(ConnectionError::InvalidInput);
        }
        let (client, cancel) = self.begin_management(&scope.session(), r.read_id, false)?;
        let result = super::bounded(&cancel, self.prepare_plan(&client, &r, &cancel)).await;
        let mut state = self
            .session
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        let s = super::current_mut(&mut state, &scope.session())?;
        if s.rbac_management.latest != r.read_id || cancel.is_cancelled() {
            return Err(ConnectionError::StaleSession);
        }
        s.rbac_management.cancel = None;
        let stored = result?;
        let out = stored.plan.clone();
        s.rbac_management.plan = Some(stored);
        Ok(out)
    }
    async fn prepare_plan(
        &self,
        client: &Client,
        r: &PlanRequest,
        cancel: &CancellationToken,
    ) -> Result<StoredPlan> {
        let _permit = self
            .workers
            .try_acquire()
            .map_err(|_| ConnectionError::Busy)?;
        // Only exact GETs drive plans. A truncated inventory is never a source of grants.
        get_required(
            client,
            &format!("/api/v1/namespaces/{}", r.user.namespace),
            cancel,
        )
        .await?;
        let old = read_account(client, &r.user, cancel).await?;
        match (&r.operation, &old) {
            (PlanOperation::Create, None) => {}
            (PlanOperation::Create, Some(_)) => return Err(ConnectionError::Conflict),
            (_, Some(v))
                if identity(v)?
                    == (
                        r.user.uid.clone().unwrap_or_default(),
                        r.user.resource_version.clone().unwrap_or_default(),
                    ) => {}
            _ => return Err(ConnectionError::Conflict),
        }
        let previous = old.as_ref().map(saved_user).transpose()?;
        let templates = load_templates(client, &r.user, cancel).await?;
        let mut operations = Vec::new();
        let mut retained_bindings = Vec::new();
        let desired = if r.operation == PlanOperation::Delete {
            Vec::new()
        } else {
            bindings(&r.user)?
        };
        let previous_bindings = previous
            .as_ref()
            .map(bindings)
            .transpose()?
            .unwrap_or_default();
        for mut b in previous_bindings {
            let current = wire(client, Method::GET, &b.path(false), None, cancel)
                .await?
                .ok_or(ConnectionError::Conflict)?;
            verify_binding(&current, &b, &r.user)?;
            b.expected = Some(identity(&current)?);
            if !desired
                .iter()
                .any(|d| d.step.name == b.step.name && d.step.namespace == b.step.namespace)
            {
                b.step.operation = PlanOperation::Delete;
                operations.push(b);
            }
        }
        if r.operation != PlanOperation::Delete {
            operations.push(account_operation(&r.user, &r.operation, old.as_ref())?);
            for mut b in desired {
                if let Some(current) =
                    wire(client, Method::GET, &b.path(false), None, cancel).await?
                {
                    verify_binding(&current, &b, &r.user)?;
                    b.expected = Some(identity(&current)?);
                    b.step.operation = PlanOperation::Update;
                    retained_bindings.push(b);
                } else {
                    operations.push(b);
                }
            }
        } else {
            operations.push(account_operation(
                &r.user,
                &PlanOperation::Delete,
                old.as_ref(),
            )?);
        }
        if operations.len() > MAX_STEPS {
            return Err(ConnectionError::LimitExceeded);
        }
        drop(_permit);
        let scope = r.scope();
        let mut authorization_epoch = None;
        for op in &operations {
            if cancel.is_cancelled() {
                return Err(ConnectionError::StaleSession);
            }
            let epoch = self
                .require_permissions(&scope.session(), &[op.check()])
                .await?;
            let approved_epoch = authorization_epoch.get_or_insert(epoch.clone());
            approved_epoch.validate()?;
            let guarded = super::permissions::authorized_client(client.clone(), epoch);
            preflight(&guarded, op, &r.user, cancel).await?;
            submit(&guarded, op, &r.user, true, cancel).await?;
        }
        let authorization_epoch = authorization_epoch.ok_or(ConnectionError::InvalidInput)?;
        authorization_epoch.validate()?;
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(|_| ConnectionError::ConnectionFailed)?;
        let plan = UserPlan {
            cluster_id: r.cluster_id.clone(),
            session_id: r.session_id.clone(),
            read_id: r.read_id,
            plan_id: random.iter().map(|b| format!("{b:02x}")).collect(),
            expires_at: now() + 120_000,
            operation: r.operation.clone(),
            user: r.user.clone(),
            summary: summarize(&r.user, &templates),
            steps: operations.iter().map(|x| x.step.clone()).collect(),
            warnings: plan_warnings(&templates),
        };
        Ok(StoredPlan {
            authorization_epoch,
            plan,
            operations,
            templates,
            deadline: tokio::time::Instant::now() + Duration::from_secs(120),
            retained_bindings,
        })
    }
    pub async fn rbac_user_apply(&self, r: ApplyRequest) -> Result<ApplyResponse> {
        let scope = SessionRequest {
            cluster_id: r.cluster_id.clone(),
            session_id: r.session_id.clone(),
        };
        super::validate_request(&scope)?;
        super::validate_read_id(r.read_id)?;
        let (stored, client, cancel) = {
            let mut state = self
                .session
                .lock()
                .map_err(|_| ConnectionError::StoreUnavailable)?;
            let s = super::current_mut(&mut state, &scope)?;
            let client = s.client.clone().ok_or(ConnectionError::NotConnected)?;
            super::validate_credentials(&client)?;
            let slot = &mut s.rbac_management;
            if slot.executing {
                return Err(ConnectionError::Busy);
            }
            let p = slot.plan.as_ref().ok_or(ConnectionError::NotFound)?;
            p.authorization_epoch.validate()?;
            if p.plan.plan_id != r.plan_id
                || r.read_id <= slot.latest
                || p.deadline <= tokio::time::Instant::now()
            {
                return Err(ConnectionError::StaleSession);
            }
            let stored = slot.plan.take().ok_or(ConnectionError::NotFound)?;
            slot.latest = r.read_id;
            slot.executing = true;
            let cancel = s.cancel.child_token();
            slot.cancel = Some(cancel.clone());
            (stored, client, cancel)
        };
        let response = self
            .execute_plan(&r, &scope, &stored, &client, &cancel)
            .await;
        // Results remain useful even after a disconnect: a submitted mutation may have reached the server.
        let _ = self.finish_management(&scope, r.read_id);
        Ok(response)
    }
    async fn execute_plan(
        &self,
        r: &ApplyRequest,
        scope: &SessionRequest,
        p: &StoredPlan,
        client: &Client,
        cancel: &CancellationToken,
    ) -> ApplyResponse {
        let mut user = p.plan.user.clone();
        let mut failed = false;
        let mut results = Vec::new();
        let initial = super::bounded(cancel, async {
            p.authorization_epoch.validate()?;
            if load_templates(client, &user, cancel).await? != p.templates {
                return Err(ConnectionError::Conflict);
            }
            for retained in &p.retained_bindings {
                preflight(client, retained, &user, cancel).await?;
            }
            for op in &p.operations {
                let epoch = self.require_permissions(scope, &[op.check()]).await?;
                let guarded = super::permissions::authorized_client(client.clone(), epoch);
                preflight(&guarded, op, &user, cancel).await?;
            }
            p.authorization_epoch.validate()?;
            Ok(())
        })
        .await;
        let mut initial_error = initial.err();
        for op in &p.operations {
            let outcome = if failed {
                None
            } else if let Some(error) = initial_error.take() {
                Some(Err(error))
            } else {
                Some(
                    super::bounded(cancel, async {
                        p.authorization_epoch.validate()?;
                        if load_templates(client, &user, cancel).await? != p.templates {
                            return Err(ConnectionError::Conflict);
                        }
                        let epoch = self.require_permissions(scope, &[op.check()]).await?;
                        let guarded = super::permissions::authorized_client(client.clone(), epoch);
                        preflight(&guarded, op, &user, cancel).await?;
                        p.authorization_epoch.validate()?;
                        let v = submit(&guarded, op, &user, false, cancel).await?;
                        if op.kind() == ResourceKind::ServiceAccounts
                            && op.step.operation != PlanOperation::Delete
                        {
                            let (uid, rv) = identity(&v)?;
                            user.uid = Some(uid);
                            user.resource_version = Some(rv);
                        }
                        Ok(())
                    })
                    .await,
                )
            };
            let (state, code) = match outcome {
                None => ("notSubmitted", "previousStepFailed".into()),
                Some(Ok(())) => ("applied", "applied".into()),
                Some(Err(e)) => {
                    failed = true;
                    ("failed", format!("{e:?}"))
                }
            };
            results.push(ApplyResult {
                operation: op.step.operation.clone(),
                kind: op.step.kind.clone(),
                name: op.step.name.clone(),
                namespace: op.step.namespace.clone(),
                state: state.into(),
                message_code: code,
            });
        }
        ApplyResponse {
            cluster_id: r.cluster_id.clone(),
            session_id: r.session_id.clone(),
            read_id: r.read_id,
            plan_id: r.plan_id.clone(),
            operation: p.plan.operation.clone(),
            results,
            complete: !failed,
        }
    }
}
async fn fetch_snapshot(
    client: &Client,
    r: &ManagementRequest,
    cancel: &CancellationToken,
) -> Result<ManagementSnapshot> {
    let roles = get_required(
        client,
        "/apis/rbac.authorization.k8s.io/v1/clusterroles?limit=500",
        cancel,
    )
    .await?;
    let accounts=get_required(client,&format!("/api/v1/namespaces/{}/serviceaccounts?limit=100&labelSelector=app.kubernetes.io%2Fmanaged-by%3Diceon-permission-manager",r.namespace),cancel).await?;
    let mut truncated = continued(&roles) || continued(&accounts);
    let mut templates = Vec::new();
    let mut users = Vec::new();
    if roles.get("kind").and_then(Value::as_str) != Some("ClusterRoleList")
        || accounts.get("kind").and_then(Value::as_str) != Some("ServiceAccountList")
    {
        return Err(ConnectionError::InvalidResponse);
    }
    let rows = roles
        .get("items")
        .and_then(Value::as_array)
        .ok_or(ConnectionError::InvalidResponse)?;
    for v in rows {
        let name = field(v, "/metadata/name")?;
        if !name.starts_with(NS_PREFIX) && !name.starts_with(CLUSTER_PREFIX) {
            continue;
        }
        if templates.len() >= MAX_TEMPLATES {
            truncated = true;
            continue;
        }
        match template(v) {
            Ok(t) => templates.push(t),
            Err(ConnectionError::LimitExceeded) => truncated = true,
            Err(e) => return Err(e),
        }
    }
    for v in accounts
        .get("items")
        .and_then(Value::as_array)
        .ok_or(ConnectionError::InvalidResponse)?
    {
        if users.len() >= MAX_USERS {
            truncated = true;
            break;
        }
        let u = saved_user(v)?;
        if u.namespace != r.namespace {
            return Err(ConnectionError::InvalidResponse);
        }
        users.push(u);
    }
    Ok(ManagementSnapshot {
        cluster_id: r.cluster_id.clone(),
        session_id: r.session_id.clone(),
        read_id: r.read_id,
        namespace: r.namespace.clone(),
        users,
        templates,
        truncated,
        fetched_at: now(),
    })
}
fn continued(v: &Value) -> bool {
    v.pointer("/metadata/continue")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty())
}
async fn read_account(
    client: &Client,
    u: &ManagedUser,
    cancel: &CancellationToken,
) -> Result<Option<Value>> {
    wire(
        client,
        Method::GET,
        &format!(
            "/api/v1/namespaces/{}/serviceaccounts/{}",
            u.namespace, u.name
        ),
        None,
        cancel,
    )
    .await
}
async fn load_templates(
    client: &Client,
    u: &ManagedUser,
    cancel: &CancellationToken,
) -> Result<Vec<ManagedTemplate>> {
    let mut out = Vec::new();
    let mut selected: Vec<_> = u
        .assignments
        .iter()
        .map(|a| TemplateIdentity {
            name: a.template_name.clone(),
            uid: a.template_uid.clone(),
            resource_version: a.template_resource_version.clone(),
        })
        .collect();
    if let Some(t) = &u.cluster_template {
        selected.push(t.clone());
    }
    for t in selected {
        let value = get_required(
            client,
            &format!(
                "/apis/rbac.authorization.k8s.io/v1/clusterroles/{}",
                encode(&t.name)
            ),
            cancel,
        )
        .await?;
        let actual = template(&value)?;
        if actual.name != t.name
            || actual.uid != t.uid
            || actual.resource_version != t.resource_version
        {
            return Err(ConnectionError::Conflict);
        }
        if u.cluster_access == ClusterAccess::Read
            && u.cluster_template
                .as_ref()
                .is_some_and(|c| c.name == actual.name)
            && actual.rules.iter().any(|rule| !read_only_rule(rule))
        {
            return Err(ConnectionError::InvalidInput);
        }
        out.push(actual);
    }
    Ok(out)
}
fn binding_name(u: &ManagedUser, role: &str, scope: &str) -> String {
    let mut hash = Sha256::new();
    for s in [&u.namespace, &u.name, role, scope] {
        hash.update(s.as_bytes());
        hash.update([0]);
    }
    let digest = hash.finalize();
    let suffix: String = digest[..20].iter().map(|b| format!("{b:02x}")).collect();
    format!("iceon-rbac-{suffix}")
}
fn bindings(u: &ManagedUser) -> Result<Vec<Intended>> {
    let mut out = Vec::new();
    for a in &u.assignments {
        if a.all_namespaces {
            out.push(binding(u, &a.template_name, None));
        } else {
            for ns in &a.namespaces {
                out.push(binding(u, &a.template_name, Some(ns.clone())));
            }
        }
        if out.len() > MAX_STEPS {
            return Err(ConnectionError::LimitExceeded);
        }
    }
    if let Some(t) = &u.cluster_template {
        out.push(binding(u, &t.name, None));
    }
    Ok(out)
}
fn binding(u: &ManagedUser, role: &str, ns: Option<String>) -> Intended {
    let kind = if ns.is_some() {
        "RoleBinding"
    } else {
        "ClusterRoleBinding"
    };
    let name = binding_name(u, role, ns.as_deref().unwrap_or("*"));
    let mut metadata = json!({"name":name,"labels":{"app.kubernetes.io/managed-by":MANAGER},"annotations":{USER_UID:u.uid.clone().unwrap_or_else(||"pending-dry-run".into())}});
    if let Some(ns) = &ns {
        metadata["namespace"] = json!(ns);
    }
    Intended {
        step: PlanStep {
            operation: PlanOperation::Create,
            kind: kind.into(),
            name,
            namespace: ns,
        },
        body: json!({"apiVersion":"rbac.authorization.k8s.io/v1","kind":kind,"metadata":metadata,"roleRef":{"apiGroup":"rbac.authorization.k8s.io","kind":"ClusterRole","name":role},"subjects":[{"kind":"ServiceAccount","name":u.name,"namespace":u.namespace}]}),
        expected: None,
    }
}
fn verify_binding(v: &Value, op: &Intended, u: &ManagedUser) -> Result<()> {
    let subjects = v.get("subjects").and_then(Value::as_array);
    let exact_subject = subjects.is_some_and(|subjects| {
        subjects.len() == 1
            && subjects[0].get("kind").and_then(Value::as_str) == Some("ServiceAccount")
            && subjects[0].get("name").and_then(Value::as_str) == Some(u.name.as_str())
            && subjects[0].get("namespace").and_then(Value::as_str) == Some(u.namespace.as_str())
            && subjects[0]
                .get("apiGroup")
                .and_then(Value::as_str)
                .unwrap_or("")
                .is_empty()
    });
    if !managed(v)
        || v.pointer("/metadata/annotations/iceon.yan.ad~1managed-user-uid")
            .and_then(Value::as_str)
            != u.uid.as_deref()
        || u.uid.is_none()
        || v.get("roleRef") != op.body.get("roleRef")
        || !exact_subject
        || v.get("kind").and_then(Value::as_str) != Some(&op.step.kind)
        || v.pointer("/metadata/name").and_then(Value::as_str) != Some(&op.step.name)
        || v.pointer("/metadata/namespace").and_then(Value::as_str) != op.step.namespace.as_deref()
    {
        return Err(ConnectionError::Conflict);
    }
    identity(v)?;
    Ok(())
}
fn account_operation(
    u: &ManagedUser,
    operation: &PlanOperation,
    old: Option<&Value>,
) -> Result<Intended> {
    let mut body=old.cloned().unwrap_or_else(||json!({"apiVersion":"v1","kind":"ServiceAccount","metadata":{"name":u.name,"namespace":u.namespace}}));
    let mut saved = u.clone();
    saved.uid = None;
    saved.resource_version = None;
    let metadata = body
        .get_mut("metadata")
        .and_then(Value::as_object_mut)
        .ok_or(ConnectionError::InvalidResponse)?;
    metadata.remove("managedFields");
    metadata.entry("labels").or_insert_with(|| json!({}))["app.kubernetes.io/managed-by"] =
        json!(MANAGER);
    metadata.entry("annotations").or_insert_with(|| json!({}))[USER] =
        json!(serde_json::to_string(&saved).map_err(|_| ConnectionError::InvalidInput)?);
    body["automountServiceAccountToken"] = json!(false);
    // Never send credential-bearing server fields back in an update.
    if let Some(map) = body.as_object_mut() {
        map.remove("secrets");
        map.remove("imagePullSecrets");
    }
    Ok(Intended {
        step: PlanStep {
            operation: operation.clone(),
            kind: "ServiceAccount".into(),
            name: u.name.clone(),
            namespace: Some(u.namespace.clone()),
        },
        body,
        expected: old.map(identity).transpose()?,
    })
}
fn summarize(u: &ManagedUser, templates: &[ManagedTemplate]) -> Vec<PlanSummary> {
    let mut result = Vec::new();
    for t in templates {
        let namespaces = u
            .assignments
            .iter()
            .find(|a| a.template_name == t.name)
            .and_then(|a| {
                if a.all_namespaces {
                    None
                } else {
                    Some(a.namespaces.clone())
                }
            });
        for r in &t.rules {
            result.push(PlanSummary {
                api_groups: r.api_groups.clone(),
                resources: r.resources.clone(),
                verbs: r.verbs.clone(),
                resource_names: r.resource_names.clone(),
                non_resource_urls: r.non_resource_urls.clone(),
                namespaces: namespaces.clone(),
            });
        }
    }
    result
}
async fn preflight(
    client: &Client,
    op: &Intended,
    u: &ManagedUser,
    cancel: &CancellationToken,
) -> Result<()> {
    let current = wire(client, Method::GET, &op.path(false), None, cancel).await?;
    match (&op.step.operation, current) {
        (PlanOperation::Create, None) => {}
        (PlanOperation::Create, Some(_)) => return Err(ConnectionError::Conflict),
        (_, Some(v)) => {
            if Some(identity(&v)?) != op.expected {
                return Err(ConnectionError::Conflict);
            }
            if op.kind() == ResourceKind::ServiceAccounts {
                let actual = saved_user(&v)?;
                if actual.uid != u.uid {
                    return Err(ConnectionError::Conflict);
                }
            } else {
                verify_binding(&v, op, u)?;
            }
        }
        _ => return Err(ConnectionError::Conflict),
    }
    if op.kind() != ResourceKind::ServiceAccounts && u.uid.is_some() {
        let actual = read_account(client, u, cancel)
            .await?
            .ok_or(ConnectionError::Conflict)?;
        if identity(&actual)?.0 != u.uid.clone().unwrap_or_default() {
            return Err(ConnectionError::Conflict);
        }
        saved_user(&actual)?;
    }
    Ok(())
}
async fn submit(
    client: &Client,
    op: &Intended,
    u: &ManagedUser,
    dry: bool,
    cancel: &CancellationToken,
) -> Result<Value> {
    let (method, mut body, collection) = match op.step.operation {
        PlanOperation::Create => (Method::POST, op.body.clone(), true),
        PlanOperation::Update => (Method::PUT, op.body.clone(), false),
        PlanOperation::Delete => {
            let (uid, rv) = op.expected.as_ref().ok_or(ConnectionError::Conflict)?;
            (
                Method::DELETE,
                json!({"apiVersion":"v1","kind":"DeleteOptions","preconditions":{"uid":uid,"resourceVersion":rv}}),
                false,
            )
        }
    };
    if op.kind() != ResourceKind::ServiceAccounts && op.step.operation != PlanOperation::Delete {
        body["metadata"]["annotations"][USER_UID] =
            json!(u.uid.as_deref().unwrap_or("pending-dry-run"));
        if !dry && u.uid.is_none() {
            return Err(ConnectionError::Conflict);
        }
    }
    let path = format!(
        "{}{}",
        op.path(collection),
        if dry { "?dryRun=All" } else { "" }
    );
    let result = wire(client, method, &path, Some(body), cancel)
        .await?
        .ok_or(ConnectionError::Conflict)?;
    if op.step.operation != PlanOperation::Delete {
        for p in ["/kind", "/metadata/name", "/metadata/namespace"] {
            if result.pointer(p) != op.body.pointer(p) {
                return Err(ConnectionError::InvalidResponse);
            }
        }
        identity(&result)?;
    }
    Ok(result)
}
async fn get_required(client: &Client, path: &str, cancel: &CancellationToken) -> Result<Value> {
    wire(client, Method::GET, path, None, cancel)
        .await?
        .ok_or(ConnectionError::ResourceUnavailable)
}
async fn wire(
    client: &Client,
    method: Method,
    path: &str,
    body: Option<Value>,
    cancel: &CancellationToken,
) -> Result<Option<Value>> {
    if cancel.is_cancelled() {
        return Err(ConnectionError::StaleSession);
    }
    let bytes = body
        .map(|v| serde_json::to_vec(&v))
        .transpose()
        .map_err(|_| ConnectionError::InvalidInput)?
        .unwrap_or_default();
    if bytes.len() > MAX_BODY {
        return Err(ConnectionError::LimitExceeded);
    }
    let get = method == Method::GET;
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .body(Body::from(bytes))
        .map_err(|_| ConnectionError::InvalidInput)?;
    req.extensions_mut().insert(cancel.clone());
    let mut response = client
        .send(req)
        .await
        .map_err(|_| ConnectionError::Network)?;
    match response.status().as_u16() {
        200..=299 => {}
        404 if get => return Ok(None),
        401 => return Err(ConnectionError::AuthFailed),
        403 => return Err(ConnectionError::PermissionDenied),
        409 => return Err(ConnectionError::Conflict),
        400 | 422 => return Err(ConnectionError::InvalidInput),
        408 | 504 => return Err(ConnectionError::Timeout),
        _ => return Err(ConnectionError::ConnectionFailed),
    }
    if response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .is_some_and(|n| n > MAX_BODY)
    {
        return Err(ConnectionError::LimitExceeded);
    }
    let mut bytes = Vec::new();
    while let Some(frame) = response.body_mut().frame().await {
        if let Ok(data) = frame.map_err(|_| ConnectionError::Network)?.into_data() {
            if data.len() > MAX_BODY.saturating_sub(bytes.len()) {
                return Err(ConnectionError::LimitExceeded);
            }
            bytes.extend_from_slice(&data);
        }
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| ConnectionError::InvalidResponse)
}

#[cfg(test)]
mod tests;
