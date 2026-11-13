//! Advisory, exact-scope capability projections. Kubernetes remains the authority.
use crate::{
    IceonApp,
    cluster_view::{namespaced_kind, section_to_resource_kind},
    navigation::RESOURCE_VIEWS,
};
use gpui_kit::{prelude::*, *};
use iceon_core::connection::{
    ResourceKind,
    permissions::{PermissionAction, PermissionCheck, PermissionDecision, ReviewRequest},
};
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct AccessScope {
    cluster: String,
    lease: String,
    namespace: Option<String>,
}
#[derive(Default)]
pub(crate) struct AccessState {
    initial_lease: Option<(String, String)>,
    scope: Option<AccessScope>,
    generation: u64,
    ready: bool,
    running: bool,
    epoch: Option<String>,
    entries: HashMap<PermissionCheck, (PermissionDecision, u64)>,
    // Display-only projection; action decisions always use expiring entries.
    published_lists: HashMap<PermissionCheck, PermissionDecision>,
    staging_lists: HashMap<PermissionCheck, PermissionDecision>,
    pending: HashSet<PermissionCheck>,
    active: Option<ReviewRequest>,
    revision: u64,
    scheduled: HashSet<PermissionCheck>,
    lists_changed: bool,
}
#[derive(Default)]
pub(crate) struct KnownClusterAccess {
    navigation: HashMap<String, HashMap<PermissionCheck, PermissionDecision>>,
}

fn resource_route_visible(
    kind: ResourceKind,
    cluster_decision: Option<PermissionDecision>,
) -> bool {
    namespaced_kind(kind) || cluster_decision == Some(PermissionDecision::Allowed)
}
fn needs_initial_access_page(
    completed: Option<&(String, String)>,
    cluster: &str,
    lease: &str,
    running: bool,
) -> bool {
    running
        && !completed.is_some_and(|(previous_cluster, previous_lease)| {
            previous_cluster == cluster && previous_lease == lease
        })
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn kinds() -> Vec<ResourceKind> {
    let mut seen = HashSet::new();
    RESOURCE_VIEWS
        .iter()
        .map(|v| v.kind)
        .filter(|k| seen.insert(*k))
        .collect()
}
fn check(
    kind: ResourceKind,
    action: PermissionAction,
    namespace: Option<&str>,
    name: Option<&str>,
) -> PermissionCheck {
    PermissionCheck {
        kind,
        action,
        namespace: namespace.map(str::to_owned),
        name: name.map(str::to_owned),
    }
}
fn list_checks(scope: &AccessScope) -> Vec<PermissionCheck> {
    kinds()
        .into_iter()
        .map(|kind| {
            check(
                kind,
                PermissionAction::List,
                if namespaced_kind(kind) {
                    scope.namespace.as_deref()
                } else {
                    None
                },
                None,
            )
        })
        .collect()
}
// Mirror the public review API's scope/action rules; invalid batches poison all checks.
fn valid(c: &PermissionCheck) -> bool {
    use PermissionAction::*;
    let scoped = namespaced_kind(c.kind);
    let dns_label = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && s.as_bytes()[0].is_ascii_alphanumeric()
            && s.as_bytes()[s.len() - 1].is_ascii_alphanumeric()
    };
    if c.namespace
        .as_deref()
        .is_some_and(|ns| ns.len() > 63 || !dns_label(ns))
    {
        return false;
    }
    if c.name.as_deref().is_some_and(|name| {
        if matches!(
            c.kind,
            ResourceKind::Roles
                | ResourceKind::ClusterRoles
                | ResourceKind::RoleBindings
                | ResourceKind::ClusterRoleBindings
        ) {
            name.is_empty()
                || name.len() > 253
                || matches!(name, "." | "..")
                || name.chars().any(|c| {
                    c.is_control() || c.is_whitespace() || matches!(c, '/' | '%' | '?' | '#')
                })
        } else {
            name.len() > 253 || !name.split('.').all(dns_label)
        }
    }) {
        return false;
    }
    if !scoped && c.namespace.is_some() {
        return false;
    }
    if scoped && c.namespace.is_none() && !matches!(c.action, List | Watch) {
        return false;
    }
    if matches!(c.action, List | Watch) && c.name.is_some() {
        return false;
    }
    if matches!(c.action, Logs | Exec | Attach | PortForward | Evict)
        && (c.kind != ResourceKind::Pods || c.name.is_none())
    {
        return false;
    }
    true
}
impl AccessState {
    fn list_decision(&self, key: &PermissionCheck) -> PermissionDecision {
        if !self.ready {
            return PermissionDecision::Unknown;
        }
        self.published_lists
            .get(key)
            .copied()
            .unwrap_or(PermissionDecision::Unknown)
    }

    fn stage_list(&mut self, key: &PermissionCheck, decision: PermissionDecision) {
        if key.action == PermissionAction::List {
            self.staging_lists.insert(key.clone(), decision);
        }
    }

    fn commit_lists(&mut self, checks: &[PermissionCheck]) -> bool {
        if !checks
            .iter()
            .all(|key| self.staging_lists.contains_key(key))
        {
            return false;
        }
        let published = checks
            .iter()
            .map(|key| {
                let decision = self.staging_lists[key];
                // Unavailable reviews do not revoke navigation or grant fresh action access.
                let decision = if decision == PermissionDecision::Unknown {
                    self.published_lists.get(key).copied().unwrap_or(decision)
                } else {
                    decision
                };
                (key.clone(), decision)
            })
            .collect::<HashMap<_, _>>();
        self.lists_changed |= self.published_lists != published;
        self.published_lists = published;
        self.staging_lists.clear();
        self.ready = true;
        true
    }

    fn invalidate(&mut self) {
        self.entries.clear();
        self.published_lists.clear();
        self.staging_lists.clear();
        self.pending.clear();
        self.scheduled.clear();
        self.epoch = None;
        self.ready = false;
        self.running = false;
        self.lists_changed = false;
    }

    fn needs_renewal(&self, key: &PermissionCheck, at: u64) -> bool {
        self.entries
            .get(key)
            .is_none_or(|(_, expiry)| *expiry <= at.saturating_add(10_000))
    }

    fn decision(&self, key: &PermissionCheck) -> PermissionDecision {
        self.entries
            .get(key)
            .filter(|(_, expiry)| *expiry > now())
            .map_or(PermissionDecision::Unknown, |(d, _)| *d)
    }
    fn current(&self, scope: &AccessScope, generation: u64) -> bool {
        self.scope.as_ref() == Some(scope) && self.generation == generation
    }
}
impl IceonApp {
    fn access_scope(&self) -> Option<AccessScope> {
        self.cluster_session
            .as_ref()
            .filter(|s| s.status == "connected")
            .map(|s| AccessScope {
                cluster: s.cluster_id.clone(),
                lease: s.session_id.clone(),
                namespace: self
                    .selected_namespace
                    .clone()
                    .filter(|_| self.selected_namespaces.len() <= 1),
            })
    }
    pub(crate) fn initial_access_loading(&self) -> bool {
        let Some(session) = self.cluster_session.as_ref() else {
            return false;
        };
        !self
            .known_cluster_access
            .navigation
            .contains_key(&session.cluster_id)
            && session.status == "connected"
            && needs_initial_access_page(
                self.access.initial_lease.as_ref(),
                &session.cluster_id,
                &session.session_id,
                self.access.running,
            )
    }

    pub fn access_ready(&self) -> bool {
        self.access.ready && self.access.scope == self.access_scope() && self.access.scope.is_some()
    }
    /// Changes whenever the advisory projection is reset, reviewed, or ages out.
    pub fn access_revision(&self) -> u64 {
        self.access.revision
    }
    pub fn access_message(&self) -> &'static str {
        if self.access_ready() && !kinds().into_iter().any(|kind| self.can_list(kind)) {
            "No resource list access is currently allowed. Reviews may be denied or unavailable; access is retried automatically."
        } else if self.access_ready() {
            "Access is advisory. Unavailable or unreviewed capabilities are hidden; refresh access to retry."
        } else if self.access.running {
            "Checking access for this cluster and namespace…"
        } else {
            "Access has not been reviewed for this session and namespace. Refresh access to retry; this is not a denial."
        }
    }
    /// Exact cached review, with no inference across namespaces or resource names.
    pub fn resource_allowed(
        &self,
        kind: ResourceKind,
        action: PermissionAction,
        namespace: Option<&str>,
        name: Option<&str>,
    ) -> bool {
        self.access_ready()
            && valid(&check(kind, action, namespace, name))
            && self.access.decision(&check(kind, action, namespace, name))
                == PermissionDecision::Allowed
    }
    pub fn can_list(&self, kind: ResourceKind) -> bool {
        self.access_ready()
            && self.access.list_decision(&check(
                kind,
                PermissionAction::List,
                if namespaced_kind(kind) && self.selected_namespaces.len() <= 1 {
                    self.selected_namespace.as_deref()
                } else {
                    None
                },
                None,
            )) == PermissionDecision::Allowed
    }
    /// Display-only fallback: a generic allow can expose an ordinary row action.
    /// Generic denied/unknown never overrides an exact named allow, nor establishes a named denial.
    pub fn can_action(
        &self,
        kind: ResourceKind,
        action: PermissionAction,
        namespace: Option<&str>,
        name: Option<&str>,
    ) -> bool {
        if !self.access_ready() {
            return false;
        }
        let key = check(kind, action, namespace, name);
        if !valid(&key) {
            return false;
        }
        match self.access.decision(&key) {
            PermissionDecision::Allowed => true,
            PermissionDecision::Denied => false,
            PermissionDecision::Unknown => {
                !self.access.entries.contains_key(&key)
                    && name.is_some()
                    && !matches!(
                        action,
                        PermissionAction::Logs
                            | PermissionAction::Exec
                            | PermissionAction::Attach
                            | PermissionAction::PortForward
                            | PermissionAction::Evict
                    )
                    && self.resource_allowed(kind, action, namespace, None)
            }
        }
    }
    /// Navigation discovery is separate from permission to query the active scope.
    pub(crate) fn route_visible(&self, section: &str) -> bool {
        let cluster_id = match self.history.current() {
            crate::navigation::Route::Cluster { cluster_id, .. } => cluster_id,
            _ => return false,
        };
        let same_lease = self.cluster_session.as_ref().is_some_and(|session| {
            self.access.initial_lease.as_ref()
                == Some(&(session.cluster_id.clone(), session.session_id.clone()))
        });
        let cached = self.known_cluster_access.navigation.get(cluster_id);
        if !same_lease && cached.is_none() {
            return false;
        }
        if let Some(kind) = section_to_resource_kind(section) {
            let lists = if same_lease {
                &self.access.published_lists
            } else {
                cached.unwrap()
            };
            return resource_route_visible(
                kind,
                lists
                    .get(&check(kind, PermissionAction::List, None, None))
                    .copied(),
            );
        }
        match section {
            "overview" | "port-forwarding" | "helm/charts" | "helm-charts" | "helm/releases"
            | "helm-releases" => true,
            _ => self.route_allowed(section),
        }
    }
    pub fn route_allowed(&self, section: &str) -> bool {
        if !self.access_ready() {
            return false;
        }
        match section {
            "overview" | "workloads-overview" | "workloads/overview" | "port-forwarding" => true,
            "helm/charts" | "helm-charts" => true, // local chart catalog, no cluster read
            "helm/releases" | "helm-releases" => self.can_list(ResourceKind::Secrets),
            "users" | "access-control/users" => [
                ResourceKind::ServiceAccounts,
                ResourceKind::Roles,
                ResourceKind::RoleBindings,
                ResourceKind::ClusterRoles,
                ResourceKind::ClusterRoleBindings,
            ]
            .into_iter()
            .all(|k| self.can_list(k)),
            _ if crate::cluster_view::parse_custom_resource_target(section).is_some() => false, // arbitrary GVR is not covered by ResourceKind reviews
            _ => section_to_resource_kind(section).is_some_and(|k| self.can_list(k)),
        }
    }
    /// Queue exact lazy checks (notably named Pod subresources). One serialized worker
    /// shares the core's single review slot, including with initial action preflight.
    pub fn review_resource_access(
        &mut self,
        kind: ResourceKind,
        action: PermissionAction,
        namespace: Option<&str>,
        name: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.access.scope != self.access_scope() || self.access.scope.is_none() {
            self.refresh_access(window, cx);
            return;
        }
        let key = check(kind, action, namespace, name);
        if !valid(&key)
            || self.access.scheduled.contains(&key)
            || self
                .access
                .entries
                .get(&key)
                .is_some_and(|(_, expiry)| *expiry > now())
        {
            return;
        }
        if action == PermissionAction::List {
            if !self.access.running {
                let scope = self.access.scope.clone().unwrap();
                self.run_access_reviews(list_checks(&scope), window, cx);
            }
            return;
        }
        self.access.scheduled.insert(key.clone());
        self.access.pending.insert(key);
        if !self.access.running && self.access_ready() {
            self.run_access_reviews(Vec::new(), window, cx);
        }
    }
    pub fn refresh_access(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Same-scope refresh preserves navigation, namespaces and loaded data.
        if self.access.scope.is_some() && self.access.scope == self.access_scope() {
            if !self.access.running {
                let scope = self.access.scope.clone().unwrap();
                self.run_access_reviews(list_checks(&scope), window, cx);
            }
            return;
        }
        if let Some(request) = self.access.active.take() {
            let _ = self.connection.cancel_permissions(request);
        }
        let generation = self.next_operation_id();
        let scope = self.access_scope();
        let revision = self.access.revision.wrapping_add(1);
        let initial_lease = self.access.initial_lease.clone();
        let cluster_lists = if self
            .access
            .scope
            .as_ref()
            .zip(scope.as_ref())
            .is_some_and(|(old, new)| old.cluster == new.cluster && old.lease == new.lease)
        {
            self.access
                .published_lists
                .iter()
                .filter(|(key, _)| !namespaced_kind(key.kind))
                .map(|(key, value)| (key.clone(), *value))
                .collect()
        } else {
            HashMap::new()
        };
        self.access = AccessState {
            published_lists: cluster_lists,
            initial_lease,
            scope: scope.clone(),
            generation,
            revision,
            ..Default::default()
        };
        self.stop_active_pod_sync();
        self.cancel_resource_pagination();
        self.overview_generation = self.next_operation_id();
        self.custom_discovery_generation = self.next_operation_id();
        self.custom_definitions_page = None;
        self.sync_namespace_select(window, cx);
        if let Some(scope) = scope {
            let checks = list_checks(&scope);
            self.start_access_maintenance(scope, generation, window, cx);
            self.run_access_reviews(checks, window, cx);
        }
        cx.notify();
    }
    pub(crate) fn ensure_access(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.access.scope != self.access_scope()
            || (!self.access_ready() && !self.access.running)
        {
            self.refresh_access(window, cx);
        }
        self.access_ready()
    }
    fn run_access_reviews(
        &mut self,
        initial: Vec<PermissionCheck>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.access.running {
            return;
        }
        let Some(scope) = self.access.scope.clone() else {
            return;
        };
        let generation = self.access.generation;
        let initial_phase = !self.access.ready;
        let list_phase = !initial.is_empty();
        if list_phase {
            self.access.staging_lists.clear();
        }
        self.access.scheduled.extend(initial.iter().cloned());
        self.access.running = true;
        let connection = self.connection.clone();
        cx.spawn_in(window, async move |owner, cx| {
            let mut queue = initial;
            let mut lists = list_phase;
            loop {
                if queue.is_empty() {
                    if lists {
                        lists = false;
                        let Ok(Some(actions)) = owner.update_in(cx, |app, window, cx| {
                            if !app.access.current(&scope, generation)
                                || app.access_scope().as_ref() != Some(&scope)
                            {
                                return None;
                            }
                            if !app.access.commit_lists(&list_checks(&scope)) {
                                app.access.running = false;
                                return None;
                            }
                            app.access.initial_lease =
                                Some((scope.cluster.clone(), scope.lease.clone()));
                            app.known_cluster_access
                                .navigation
                                .insert(scope.cluster.clone(), app.access.published_lists.clone());
                            app.access.revision = app.access.revision.wrapping_add(1);
                            let changed = std::mem::take(&mut app.access.lists_changed);
                            if initial_phase || changed {
                                app.cluster_loading = false;
                                app.load_access_namespaces(window, cx);
                                if app.can_list(ResourceKind::CustomResourceDefinitions) {
                                    app.discover_custom_resources(window, cx);
                                }
                                app.load_active_cluster_section(window, cx);
                            }
                            cx.notify();
                            let actions = app.relevant_access_checks();
                            app.access.scheduled.extend(actions.iter().cloned());
                            Some(actions)
                        }) else {
                            return;
                        };
                        queue = actions;
                    } else {
                        let Ok(Some(pending)) = owner.update_in(cx, |app, window, cx| {
                            if !app.access.current(&scope, generation)
                                || app.access_scope().as_ref() != Some(&scope)
                            {
                                return None;
                            }
                            let pending = app.access.pending.drain().collect::<Vec<_>>();
                            if pending.is_empty() {
                                app.access.running = false;
                                app.access.active = None;
                                if std::mem::take(&mut app.access.lists_changed) {
                                    app.load_access_namespaces(window, cx);
                                    if app.can_list(ResourceKind::CustomResourceDefinitions) {
                                        app.discover_custom_resources(window, cx);
                                    }
                                    // Only permission changes, not every renewal, reload data.
                                    app.load_active_cluster_section(window, cx);
                                }
                            }
                            cx.notify();
                            Some(pending)
                        }) else {
                            return;
                        };
                        queue = pending;
                        if queue.is_empty() {
                            return;
                        }
                    }
                }
                if queue.is_empty() {
                    continue;
                }
                // Named inspector checks should not wait behind all generic actions.
                let Ok(Some(mut pending)) = owner.update_in(cx, |app, _, _| {
                    if !app.access.current(&scope, generation)
                        || app.access_scope().as_ref() != Some(&scope)
                    {
                        return None;
                    }
                    Some(app.access.pending.drain().collect::<Vec<_>>())
                }) else {
                    return;
                };
                pending.append(&mut queue);
                let mut seen = HashSet::new();
                queue = pending
                    .into_iter()
                    .filter(|key| seen.insert(key.clone()))
                    .collect();
                let batch = queue.drain(..queue.len().min(32)).collect::<Vec<_>>();
                let Ok(Some(request)) = owner.update_in(cx, |app, _, _| {
                    if !app.access.current(&scope, generation)
                        || app.access_scope().as_ref() != Some(&scope)
                    {
                        return None;
                    }
                    let request = ReviewRequest {
                        cluster_id: scope.cluster.clone(),
                        session_id: scope.lease.clone(),
                        read_id: app.next_operation_id(),
                        checks: batch,
                    };
                    app.access.active = Some(request.clone());
                    Some(request)
                }) else {
                    return;
                };
                let response = connection.review_permissions(request.clone()).await;
                let keep = owner
                    .update_in(cx, |app, _, cx| {
                        if !app.access.current(&scope, generation)
                            || app.access_scope().as_ref() != Some(&scope)
                        {
                            return false;
                        }
                        app.access.active = None;
                        for key in &request.checks {
                            app.access.scheduled.remove(key);
                        }
                        app.access.revision = app.access.revision.wrapping_add(1);
                        if let Ok(response) = response {
                            if response.cluster_id != request.cluster_id
                                || response.session_id != request.session_id
                                || response.read_id != request.read_id
                                || response.items.len() != request.checks.len()
                                || !response
                                    .items
                                    .iter()
                                    .zip(&request.checks)
                                    .all(|(item, check)| &item.check == check)
                            {
                                app.access.invalidate();
                                cx.notify();
                                return false;
                            }
                            if app
                                .access
                                .epoch
                                .as_ref()
                                .is_some_and(|epoch| epoch != &response.authorization_epoch)
                            {
                                app.access.invalidate();
                                cx.notify();
                                return false;
                            }
                            app.access.epoch = Some(response.authorization_epoch);
                            for item in response.items {
                                app.access.stage_list(&item.check, item.decision);
                                app.access
                                    .entries
                                    .insert(item.check, (item.decision, response.expires_at));
                            }
                        } else {
                            // A failed renewal must not extend an old allowance.
                            for key in &request.checks {
                                app.access.stage_list(key, PermissionDecision::Unknown);
                                app.access
                                    .entries
                                    .insert(key.clone(), (PermissionDecision::Unknown, now()));
                            }
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !keep {
                    return;
                }
            }
        })
        .detach();
    }
    /// Only the visible resource's generic actions, plus the open inspector's exact
    /// reference, are maintained. All Namespaces is never replaced with a guessed
    /// namespace; namespaced row checks must come from the actual row reference.
    fn relevant_access_checks(&self) -> Vec<PermissionCheck> {
        let Some(scope) = self.access.scope.as_ref() else {
            return Vec::new();
        };
        let mut checks = HashSet::new();
        if let Some(kind) = section_to_resource_kind(&self.active_cluster_section)
            && self.can_list(kind)
        {
            for action in [
                PermissionAction::Get,
                PermissionAction::Create,
                PermissionAction::Patch,
                PermissionAction::Update,
                PermissionAction::Delete,
                PermissionAction::Watch,
            ] {
                checks.insert(check(
                    kind,
                    action,
                    if namespaced_kind(kind) {
                        scope.namespace.as_deref()
                    } else {
                        None
                    },
                    None,
                ));
            }
        }
        if self.inspector_open
            && let Some(crate::inspector_view::InspectorTarget::Resource(reference)) =
                &self.inspector.target
        {
            for action in [
                PermissionAction::Get,
                PermissionAction::Patch,
                PermissionAction::Update,
                PermissionAction::Delete,
            ] {
                checks.insert(check(
                    reference.kind,
                    action,
                    reference.namespace.as_deref(),
                    Some(&reference.name),
                ));
            }
            if reference.kind == ResourceKind::Pods {
                for action in [
                    PermissionAction::Logs,
                    PermissionAction::Exec,
                    PermissionAction::Attach,
                    PermissionAction::PortForward,
                ] {
                    checks.insert(check(
                        reference.kind,
                        action,
                        reference.namespace.as_deref(),
                        Some(&reference.name),
                    ));
                }
            }
            if reference.kind == ResourceKind::CronJobs {
                checks.insert(check(
                    ResourceKind::Jobs,
                    PermissionAction::Create,
                    reference.namespace.as_deref(),
                    None,
                ));
            }
        }
        checks.into_iter().filter(valid).collect()
    }
    fn start_access_maintenance(
        &mut self,
        scope: AccessScope,
        generation: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Exactly one timer per explicit refresh generation. It never refreshes
        // core authorization epochs or cancels action/preflight work.
        cx.spawn_in(window, async move |owner, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(10))
                    .await;
                let keep = owner
                    .update_in(cx, |app, window, cx| {
                        if !app.access.current(&scope, generation)
                            || app.access_scope().as_ref() != Some(&scope)
                        {
                            return false;
                        }
                        // Action expiry stays fail-closed; navigation does not age out.
                        app.access.revision = app.access.revision.wrapping_add(1);
                        cx.notify();
                        if app.access.running {
                            return true;
                        }
                        if !app.access.ready {
                            app.run_access_reviews(list_checks(&scope), window, cx);
                            return true;
                        }
                        let at = now();
                        let lists = list_checks(&scope);
                        if lists.iter().any(|key| app.access.needs_renewal(key, at)) {
                            // Full review, atomically published: never renew a partial TTL batch.
                            app.run_access_reviews(lists, window, cx);
                            return true;
                        }
                        for key in app.relevant_access_checks() {
                            if app.access.needs_renewal(&key, at)
                                && app.access.scheduled.insert(key.clone())
                            {
                                app.access.pending.insert(key);
                            }
                        }
                        if !app.access.pending.is_empty() {
                            app.run_access_reviews(Vec::new(), window, cx);
                        }
                        true
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
    }
    fn load_access_namespaces(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_list(ResourceKind::Namespaces) {
            return;
        }
        let Some(scope) = self.access.scope.clone() else {
            return;
        };
        let connection = self.connection.clone();
        cx.spawn_in(window, async move |owner, cx| {
            let result = connection
                .discover_namespaces(iceon_core::connection::SessionRequest {
                    cluster_id: scope.cluster.clone(),
                    session_id: scope.lease.clone(),
                })
                .await;
            let _ = owner.update_in(cx, |app, window, cx| {
                if !app.cluster_session.as_ref().is_some_and(|session| {
                    session.cluster_id == scope.cluster && session.session_id == scope.lease
                }) {
                    return;
                }
                match result {
                    Ok(names) => {
                        app.namespaces = names;
                        app.sync_namespace_select(window, cx);
                    }
                    Err(iceon_core::connection::ConnectionError::StaleSession) => return,
                    Err(_) => { /* Keep a previously discovered list on transient failures. */ }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn revisited_navigation_cache_does_not_restore_action_permissions() {
        let mut known = super::KnownClusterAccess::default();
        let list = super::check(
            iceon_core::connection::ResourceKind::Nodes,
            iceon_core::connection::permissions::PermissionAction::List,
            None,
            None,
        );
        known.navigation.insert(
            "visited".into(),
            [(
                list.clone(),
                iceon_core::connection::permissions::PermissionDecision::Allowed,
            )]
            .into(),
        );
        assert!(known.navigation.contains_key("visited"));
        assert!(!known.navigation.contains_key("new-cluster"));
        let new_session = super::AccessState::default();
        assert_eq!(
            new_session.decision(&list),
            iceon_core::connection::permissions::PermissionDecision::Unknown
        );
        assert!(!new_session.ready);
    }
    #[test]
    fn discoverable_namespace_menu_does_not_authorize_all_namespace_reads() {
        use super::*;
        assert!(resource_route_visible(
            ResourceKind::Pods,
            Some(PermissionDecision::Denied)
        ));
        assert!(resource_route_visible(ResourceKind::Pods, None));
        assert!(!resource_route_visible(
            ResourceKind::Nodes,
            Some(PermissionDecision::Denied)
        ));
        let mut state = AccessState::default();
        state.entries.insert(
            check(
                ResourceKind::Pods,
                PermissionAction::List,
                Some("dev"),
                None,
            ),
            (PermissionDecision::Allowed, now() + 10000),
        );
        assert_eq!(
            state.decision(&check(
                ResourceKind::Pods,
                PermissionAction::List,
                None,
                None
            )),
            PermissionDecision::Unknown
        );
        assert_eq!(
            state.decision(&check(
                ResourceKind::Pods,
                PermissionAction::List,
                Some("dev"),
                None
            )),
            PermissionDecision::Allowed
        );
    }
    #[test]
    fn full_page_loading_is_only_for_initial_session_review() {
        let completed = ("cluster".into(), "lease".into());
        assert!(super::needs_initial_access_page(
            None, "cluster", "lease", true
        ));
        assert!(!super::needs_initial_access_page(
            Some(&completed),
            "cluster",
            "lease",
            true
        ));
        assert!(super::needs_initial_access_page(
            Some(&completed),
            "cluster",
            "new-lease",
            true
        ));
        assert!(!super::needs_initial_access_page(
            None, "cluster", "lease", false
        ));
    }
    use super::{AccessScope, AccessState, check, now, valid};
    use iceon_core::connection::{
        ResourceKind,
        permissions::{PermissionAction, PermissionDecision},
    };
    fn scope() -> AccessScope {
        AccessScope {
            cluster: "c".into(),
            lease: "s".into(),
            namespace: Some("a".into()),
        }
    }

    fn published_state() -> AccessState {
        let scope = scope();
        let mut state = AccessState {
            scope: Some(scope.clone()),
            ..Default::default()
        };
        for key in super::list_checks(&scope) {
            state.stage_list(&key, PermissionDecision::Allowed);
        }
        assert!(state.commit_lists(&super::list_checks(&scope)));
        state.lists_changed = false;
        state
    }

    #[test]
    fn expired_entries_keep_navigation_but_never_allow_actions() {
        let mut state = published_state();
        let key = check(ResourceKind::Pods, PermissionAction::List, Some("a"), None);
        state
            .entries
            .insert(key.clone(), (PermissionDecision::Allowed, 0));
        let action = check(
            ResourceKind::Pods,
            PermissionAction::Delete,
            Some("a"),
            Some("p"),
        );
        state
            .entries
            .insert(action.clone(), (PermissionDecision::Allowed, 0));
        assert_eq!(state.list_decision(&key), PermissionDecision::Allowed);
        assert_eq!(state.decision(&key), PermissionDecision::Unknown);
        assert_eq!(state.decision(&action), PermissionDecision::Unknown);
        assert!(!state.lists_changed);
    }

    #[test]
    fn initial_lists_are_not_published_until_every_batch_is_reviewed() {
        let checks = super::list_checks(&scope());
        assert!(checks.len() > 32, "exercise multiple review batches");
        let mut state = AccessState::default();
        for key in &checks[..32] {
            state.stage_list(key, PermissionDecision::Allowed);
        }
        assert!(!state.commit_lists(&checks));
        assert!(!state.ready);
        assert!(state.published_lists.is_empty());
        for key in &checks[32..] {
            state.stage_list(key, PermissionDecision::Denied);
        }
        assert!(state.commit_lists(&checks));
        assert!(state.ready);
        assert_eq!(state.list_decision(&checks[0]), PermissionDecision::Allowed);
        assert_eq!(
            state.list_decision(checks.last().unwrap()),
            PermissionDecision::Denied
        );
    }

    #[test]
    fn renewal_publishes_denials_atomically_and_only_flags_real_changes() {
        let mut state = published_state();
        let checks = super::list_checks(&scope());
        for key in &checks[..32] {
            state.stage_list(key, PermissionDecision::Denied);
        }
        assert!(!state.commit_lists(&checks));
        assert!(
            checks
                .iter()
                .all(|key| state.list_decision(key) == PermissionDecision::Allowed)
        );
        assert!(!state.lists_changed);
        for key in &checks[32..] {
            state.stage_list(key, PermissionDecision::Denied);
        }
        assert!(state.commit_lists(&checks));
        assert!(state.lists_changed);
        assert!(
            checks
                .iter()
                .all(|key| state.list_decision(key) == PermissionDecision::Denied)
        );
        state.lists_changed = false;
        for key in &checks {
            state.stage_list(key, PermissionDecision::Denied);
        }
        assert!(state.commit_lists(&checks));
        assert!(
            !state.lists_changed,
            "unchanged renewal must not reload a page"
        );
    }

    #[test]
    fn unavailable_refresh_retains_navigation_not_fresh_action_access() {
        let mut state = published_state();
        let checks = super::list_checks(&scope());
        for key in &checks {
            state.stage_list(key, PermissionDecision::Unknown);
            state
                .entries
                .insert(key.clone(), (PermissionDecision::Unknown, now() + 30_000));
        }
        assert!(state.ready, "background refresh must not reset readiness");
        assert!(state.commit_lists(&checks));
        assert!(!state.lists_changed);
        for key in &checks {
            assert_eq!(state.list_decision(key), PermissionDecision::Allowed);
            assert_eq!(state.decision(key), PermissionDecision::Unknown);
        }
        let mut initial = AccessState::default();
        for key in &checks {
            initial.stage_list(key, PermissionDecision::Unknown);
        }
        assert!(initial.commit_lists(&checks));
        assert!(
            checks
                .iter()
                .all(|key| initial.list_decision(key) == PermissionDecision::Unknown)
        );
    }

    #[test]
    fn new_scope_and_epoch_invalidation_clear_published_and_staged_decisions() {
        let mut state = published_state();
        let key = super::list_checks(&scope()).remove(0);
        state.stage_list(&key, PermissionDecision::Allowed);
        state.invalidate();
        assert!(!state.ready);
        assert!(!state.running);
        assert!(state.entries.is_empty());
        assert!(state.published_lists.is_empty());
        assert!(state.staging_lists.is_empty());
        assert_eq!(state.list_decision(&key), PermissionDecision::Unknown);
        let new_scope = AccessScope {
            namespace: Some("b".into()),
            ..scope()
        };
        let state = AccessState {
            scope: Some(new_scope.clone()),
            generation: 2,
            ..Default::default()
        };
        assert!(!state.current(&scope(), 1));
        assert!(state.current(&new_scope, 2));
        assert!(!state.ready);
        assert!(state.published_lists.is_empty());
    }

    #[test]
    fn exact_scope_and_unknown_are_not_allowance() {
        let mut state = AccessState::default();
        let generic = check(ResourceKind::Pods, PermissionAction::Get, Some("a"), None);
        state
            .entries
            .insert(generic.clone(), (PermissionDecision::Denied, now() + 30000));
        assert_eq!(state.decision(&generic), PermissionDecision::Denied);
        assert_eq!(
            state.decision(&check(
                ResourceKind::Pods,
                PermissionAction::Get,
                Some("a"),
                Some("pod")
            )),
            PermissionDecision::Unknown
        );
        assert_eq!(
            state.decision(&check(
                ResourceKind::Pods,
                PermissionAction::Get,
                Some("b"),
                None
            )),
            PermissionDecision::Unknown
        );
        state
            .entries
            .insert(generic.clone(), (PermissionDecision::Allowed, 0));
        assert_eq!(state.decision(&generic), PermissionDecision::Unknown);
    }
    #[test]
    fn renewal_window_does_not_extend_allowance() {
        let mut state = AccessState::default();
        let key = check(ResourceKind::Pods, PermissionAction::List, Some("a"), None);
        assert!(state.needs_renewal(&key, 100));
        state
            .entries
            .insert(key.clone(), (PermissionDecision::Allowed, 20_000));
        assert!(!state.needs_renewal(&key, 9_999));
        assert!(state.needs_renewal(&key, 10_000));
        state
            .entries
            .insert(key.clone(), (PermissionDecision::Allowed, now()));
        state.ready = true;
        assert_eq!(state.decision(&key), PermissionDecision::Unknown);
        assert!(state.ready, "entry expiry must not reset scope readiness");
        assert!(state.needs_renewal(&key, u64::MAX));
    }
    #[test]
    fn all_namespace_lists_keep_their_exact_scope() {
        let scope = AccessScope {
            cluster: "c".into(),
            lease: "s".into(),
            namespace: None,
        };
        let checks = super::list_checks(&scope);
        assert!(!checks.is_empty());
        assert!(checks.iter().all(|key| key.namespace.is_none()
            && key.name.is_none()
            && key.action == PermissionAction::List
            && valid(key)));
    }
    #[test]
    fn lease_namespace_and_generation_must_match() {
        let scope = AccessScope {
            cluster: "c".into(),
            lease: "one".into(),
            namespace: Some("a".into()),
        };
        let state = AccessState {
            scope: Some(scope.clone()),
            generation: 4,
            ..Default::default()
        };
        assert!(state.current(&scope, 4));
        assert!(!state.current(&scope, 5));
        assert!(!state.current(
            &AccessScope {
                lease: "two".into(),
                ..scope.clone()
            },
            4
        ));
        assert!(!state.current(
            &AccessScope {
                namespace: None,
                ..scope
            },
            4
        ));
    }
    #[test]
    fn preflight_obeys_core_action_scope_rules() {
        assert!(valid(&check(
            ResourceKind::Pods,
            PermissionAction::List,
            None,
            None
        )));
        assert!(!valid(&check(
            ResourceKind::Pods,
            PermissionAction::Patch,
            None,
            None
        )));
        assert!(!valid(&check(
            ResourceKind::Pods,
            PermissionAction::Logs,
            Some("a"),
            None
        )));
        assert!(valid(&check(
            ResourceKind::Pods,
            PermissionAction::Exec,
            Some("a"),
            Some("p")
        )));
        assert!(!valid(&check(
            ResourceKind::Nodes,
            PermissionAction::Get,
            Some("a"),
            None
        )));
        assert!(!valid(&check(
            ResourceKind::Pods,
            PermissionAction::Watch,
            Some("a"),
            Some("p")
        )));
    }
}
