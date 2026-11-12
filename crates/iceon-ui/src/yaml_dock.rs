//! Session-bound create and reviewed-field YAML buffers. Rendering never starts network work.
use crate::{
    IceonApp, cluster_view::namespaced_kind, inspector_view::editor_surface::EditorSurface,
};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::EditorState,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use iceon_core::connection::permissions::PermissionAction;
use iceon_core::connection::{
    ResourceKind,
    create::{CreateCancelRequest, CreateRequest},
    details::ResourceReference,
    resource_edit::{EditCancelRequest, EditMode, EditRequest, EditResult},
};

#[derive(Debug, PartialEq, Eq)]
enum YamlTarget {
    Edit(ResourceReference),
    Create {
        kind: ResourceKind,
        namespace: Option<String>,
    },
}

pub(crate) struct YamlDockTab {
    pub key: u64,
    target: YamlTarget,
    cluster_id: String,
    session_id: String,
    pub editor: Entity<EditorState>,
    baseline: String,
    loaded: Option<EditResult>,
    operation: Option<u64>,
    pub message: String,
    _observer: Subscription,
}
impl YamlDockTab {
    pub fn kind(&self) -> ResourceKind {
        match &self.target {
            YamlTarget::Edit(r) => r.kind,
            YamlTarget::Create { kind, .. } => *kind,
        }
    }
    pub fn namespace(&self) -> Option<&str> {
        match &self.target {
            YamlTarget::Edit(r) => r.namespace.as_deref(),
            YamlTarget::Create { namespace, .. } => namespace.as_deref(),
        }
    }
    pub fn name(&self) -> String {
        match &self.target {
            YamlTarget::Edit(r) => r.name.clone(),
            YamlTarget::Create { kind, .. } => format!("Create {kind:?}"),
        }
    }

    pub fn dirty(&self, cx: &App) -> bool {
        dirty(&self.baseline, self.editor.read(cx).value().as_ref())
    }
}
fn dirty(baseline: &str, buffer: &str) -> bool {
    baseline != buffer
}
fn response_is_current(
    key: u64,
    expected_key: u64,
    operation: Option<u64>,
    request: u64,
    refresh: u64,
    lease_valid: bool,
) -> bool {
    key == expected_key && lease_valid && operation.is_some_and(|id| id == request || id == refresh)
}
fn same_lease(cluster: &str, session: &str, current: Option<(&str, &str)>) -> bool {
    current == Some((cluster, session))
}
impl IceonApp {
    fn yaml_lease_valid(&self, tab: &YamlDockTab) -> bool {
        same_lease(
            &tab.cluster_id,
            &tab.session_id,
            self.cluster_session
                .as_ref()
                .map(|s| (s.cluster_id.as_str(), s.session_id.as_str())),
        )
    }
    pub(crate) fn open_yaml_editor(
        &mut self,
        reference: ResourceReference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_reference_action(&reference, PermissionAction::Get) {
            return;
        }
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        if let Some(key) = self
            .yaml_dock_tabs
            .iter()
            .find(|t| {
                matches!(&t.target, YamlTarget::Edit(r) if *r == reference)
                    && t.cluster_id == session.cluster_id
                    && t.session_id == session.session_id
            })
            .map(|t| t.key)
        {
            self.select_yaml_tab(key, window, cx);
            return;
        }
        if self.yaml_dock_tabs.len() + self.pod_dock_tabs.len() + self.device_terminals.len() >= 12
        {
            self.cluster_error = Some("Console tab limit reached (12). Close a tab first.".into());
            cx.notify();
            return;
        }
        let key = self.next_operation_id();
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("yaml")
                .line_number(true)
        });
        let observer = cx.observe(&editor, |_, _, cx| cx.notify());
        self.yaml_dock_tabs.push(YamlDockTab {
            key,
            target: YamlTarget::Edit(reference),
            cluster_id: session.cluster_id,
            session_id: session.session_id,
            editor,
            baseline: String::new(),
            loaded: None,
            operation: None,
            message: "Loading reviewed fields…".into(),
            _observer: observer,
        });
        self.select_yaml_tab(key, window, cx);
        self.run_yaml_operation(key, EditMode::Read, false, window, cx);
    }
    pub(crate) fn open_create_yaml_tab(
        &mut self,
        kind: ResourceKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let namespace = namespaced_kind(kind).then(|| {
            self.selected_namespace
                .clone()
                .unwrap_or_else(|| "default".into())
        });
        if !self.can_action(kind, PermissionAction::Create, namespace.as_deref(), None) {
            return;
        }
        let target = YamlTarget::Create { kind, namespace };
        if let Some(key) = self
            .yaml_dock_tabs
            .iter()
            .find(|tab| {
                tab.cluster_id == session.cluster_id
                    && tab.session_id == session.session_id
                    && tab.target == target
            })
            .map(|tab| tab.key)
        {
            self.select_yaml_tab(key, window, cx);
            return;
        }
        if self.yaml_dock_tabs.len() + self.pod_dock_tabs.len() + self.device_terminals.len() >= 12
        {
            self.cluster_error = Some("Console tab limit reached (12). Close a tab first.".into());
            cx.notify();
            return;
        }
        let key = self.next_operation_id();
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("yaml")
                .line_number(true)
        });
        let observer = cx.observe(&editor, |_, _, cx| cx.notify());
        let template = crate::cluster_view::default_yaml_template(
            kind,
            match &target {
                YamlTarget::Create { namespace, .. } => namespace.as_deref().unwrap_or("default"),
                YamlTarget::Edit(_) => unreachable!(),
            },
        );
        editor.update(cx, |editor, cx| editor.set_value(template, window, cx));
        self.yaml_dock_tabs.push(YamlDockTab {
            key,
            target,
            cluster_id: session.cluster_id,
            session_id: session.session_id,
            editor,
            baseline: String::new(),
            loaded: None,
            operation: None,
            message: "New resource YAML. Validate with server dry-run before creating.".into(),
            _observer: observer,
        });
        self.select_yaml_tab(key, window, cx);
    }
    pub(crate) fn select_yaml_tab(
        &mut self,
        key: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab) = self.yaml_dock_tabs.iter().find(|t| t.key == key) {
            self.active_yaml_tab = Some(key);
            self.pod_dock_open = true;
            window.focus(&tab.editor.focus_handle(cx), cx);
            cx.notify();
        }
    }
    pub(crate) fn request_close_yaml_tab(
        &mut self,
        key: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_close_console_tabs(
            crate::console_tabs::ConsoleTab::Yaml(key),
            crate::console_tabs::CloseTabs::One,
            window,
            cx,
        );
    }
    pub(crate) fn close_yaml_tab(&mut self, key: u64, cx: &mut Context<Self>) {
        if let Some(index) = self.yaml_dock_tabs.iter().position(|t| t.key == key) {
            let tab = self.yaml_dock_tabs.remove(index);
            if let Some(operation_id) = tab.operation {
                if matches!(tab.target, YamlTarget::Create { .. }) {
                    let _ = self.connection.cancel_create(CreateCancelRequest {
                        cluster_id: tab.cluster_id.clone(),
                        session_id: tab.session_id.clone(),
                        kind: tab.kind(),
                        read_id: operation_id,
                    });
                } else {
                    let _ = self.connection.cancel_resource_edit(EditCancelRequest {
                        cluster_id: tab.cluster_id,
                        session_id: tab.session_id,
                        operation_id,
                    });
                }
            }
            if self.active_yaml_tab == Some(key) {
                self.active_yaml_tab = self
                    .yaml_dock_tabs
                    .get(index.min(self.yaml_dock_tabs.len().saturating_sub(1)))
                    .map(|t| t.key);
            }
            cx.notify();
        }
    }
    fn run_yaml_operation(
        &mut self,
        key: u64,
        mode: EditMode,
        close: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.yaml_dock_tabs.iter().position(|t| t.key == key) else {
            return;
        };
        if !self.yaml_lease_valid(&self.yaml_dock_tabs[index]) {
            return;
        }
        let target = &self.yaml_dock_tabs[index].target;
        let allowed = match target {
            YamlTarget::Edit(reference) => self.can_reference_action(
                reference,
                if mode == EditMode::Read {
                    PermissionAction::Get
                } else {
                    PermissionAction::Patch
                },
            ),
            YamlTarget::Create { kind, namespace } => {
                self.can_action(*kind, PermissionAction::Create, namespace.as_deref(), None)
            }
        };
        if !allowed {
            return;
        }
        if self.yaml_dock_tabs[index].operation.is_some() {
            return;
        }
        if matches!(self.yaml_dock_tabs[index].target, YamlTarget::Create { .. }) {
            if mode != EditMode::Read {
                self.run_create_yaml_operation(key, mode == EditMode::DryRun, close, window, cx);
            }
            return;
        }
        let tab = &self.yaml_dock_tabs[index];
        let YamlTarget::Edit(reference) = &tab.target else {
            return;
        };
        if reference.uid.as_deref().is_none_or(str::is_empty) {
            self.yaml_dock_tabs[index].message =
                "Missing resource UID. Refresh the resource and reopen YAML.".into();
            cx.notify();
            return;
        }
        if mode != EditMode::Read
            && tab
                .loaded
                .as_ref()
                .is_none_or(|r| r.resource_version.is_empty())
        {
            return;
        }
        let submitted = tab.editor.read(cx).value().to_string();
        let request_reference = reference.clone();
        let cluster_id = tab.cluster_id.clone();
        let session_id = tab.session_id.clone();
        let resource_version = if mode == EditMode::Read {
            None
        } else {
            tab.loaded.as_ref().map(|r| r.resource_version.clone())
        };
        let operation_id = self.next_operation_id();
        // Reserve a distinct monotonic operation for the authoritative post-apply read.
        let refresh_id = self.next_operation_id();
        self.yaml_dock_tabs[index].operation = Some(operation_id);
        self.yaml_dock_tabs[index].message = match mode {
            EditMode::Read => "Loading…",
            EditMode::DryRun => "Validating…",
            EditMode::Apply => "Applying…",
        }
        .into();
        cx.notify();
        let connection = self.connection.clone();
        let req = EditRequest {
            cluster_id,
            session_id,
            operation_id,
            reference: request_reference,
            resource_version,
            yaml: (mode != EditMode::Read).then(|| submitted.clone()),
            mode,
        };
        cx.spawn_in(window, async move |owner, cx| {
            let result = connection.resource_edit(req.clone()).await;
            // A successful apply needs a fresh reviewed projection (including metadata RV).
            let mut refreshed = None;
            if mode == EditMode::Apply && result.as_ref().is_ok_and(|r| r.accepted) {
                let proceed = owner.update_in(cx, |app, _, _| {
                    let valid = app.yaml_dock_tabs.iter().position(|t| t.key == key && t.operation == Some(operation_id) && app.yaml_lease_valid(t));
                    if let Some(index) = valid { app.yaml_dock_tabs[index].operation = Some(refresh_id); true } else { false }
                }).unwrap_or(false);
                if !proceed { return; }
                refreshed = Some(connection.resource_edit(EditRequest { operation_id: refresh_id, mode: EditMode::Read, yaml: None, resource_version: None, ..req }).await);
            }
            let _ = owner.update_in(cx, |app, window, cx| {
                let Some(index) = app.yaml_dock_tabs.iter().position(|t| response_is_current(t.key, key, t.operation, operation_id, refresh_id, app.yaml_lease_valid(t))) else { return; };
                let tab = &mut app.yaml_dock_tabs[index]; tab.operation = None;
                let unchanged = tab.editor.read(cx).value() == submitted;
                let mut close_now = false;
                match result {
                    Ok(result) if mode == EditMode::Read || result.accepted => match mode {
                        EditMode::Read => { if unchanged { let yaml = result.yaml.clone().unwrap_or_default(); tab.baseline = yaml.clone(); tab.editor.update(cx, |e, cx| e.set_value(yaml, window, cx)); tab.loaded = Some(result); tab.message = "Reviewed fields only; server UID/resourceVersion preconditions enforced.".into(); } else { tab.message = "Read completed; buffer changed meanwhile. Reload to replace it.".into(); } },
                        EditMode::DryRun => tab.message = "Server dry-run validated. No changes applied.".into(),
                        EditMode::Apply => {
                            tab.message = "Applied successfully.".into();
                            if unchanged {
                                match refreshed {
                                    Some(Ok(fresh)) if fresh.yaml.is_some() && !fresh.resource_version.is_empty() => { let yaml = fresh.yaml.clone().unwrap_or_default(); tab.baseline = yaml.clone(); tab.editor.update(cx, |e, cx| e.set_value(yaml, window, cx)); tab.loaded = Some(fresh); close_now = close; },
                                    _ => { tab.loaded = None; tab.message = "Applied, but refresh failed. Buffer retained; reload before saving again.".into(); }
                                }
                            } else { tab.loaded = None; tab.message = "Applied submitted version; newer edits preserved. Reload required before another save.".into(); }
                        }
                    },
                    Ok(_) => tab.message = "Server did not accept this operation.".into(),
                    Err(error) => tab.message = format!("YAML operation failed: {error:?}. Buffer preserved."),
                }
                if close_now { app.close_yaml_tab(key, cx); }
                if mode == EditMode::Apply { app.reload_active_section(window, cx); }
                cx.notify();
            });
        }).detach();
    }
    fn run_create_yaml_operation(
        &mut self,
        key: u64,
        dry_run: bool,
        close: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.yaml_dock_tabs.iter().position(|tab| tab.key == key) else {
            return;
        };
        let tab = &self.yaml_dock_tabs[index];
        if !self.can_action(tab.kind(), PermissionAction::Create, tab.namespace(), None) {
            return;
        }
        if !self.yaml_lease_valid(&self.yaml_dock_tabs[index])
            || self
                .yaml_dock_tabs
                .iter()
                .any(|tab| tab.operation.is_some() && self.yaml_lease_valid(tab))
        {
            return;
        }
        let operation_id = self.next_operation_id();
        let tab = &self.yaml_dock_tabs[index];
        let YamlTarget::Create { kind, namespace } = &tab.target else {
            return;
        };
        let submitted = tab.editor.read(cx).value().to_string();
        let request = CreateRequest {
            cluster_id: tab.cluster_id.clone(),
            session_id: tab.session_id.clone(),
            kind: *kind,
            namespace: namespace.clone(),
            yaml: submitted.clone(),
            dry_run,
            read_id: operation_id,
        };
        self.yaml_dock_tabs[index].operation = Some(operation_id);
        self.yaml_dock_tabs[index].message = if dry_run {
            "Validating…"
        } else {
            "Creating…"
        }
        .into();
        let connection = self.connection.clone();
        cx.notify();
        cx.spawn_in(window, async move |owner, cx| {
            let result = connection.create(request).await;
            let _ = owner.update_in(cx, |app, window, cx| {
                let Some(index) = app.yaml_dock_tabs.iter().position(|tab| {
                    response_is_current(
                        tab.key,
                        key,
                        tab.operation,
                        operation_id,
                        operation_id,
                        app.yaml_lease_valid(tab),
                    )
                }) else {
                    return;
                };
                let tab = &mut app.yaml_dock_tabs[index];
                tab.operation = None;
                let unchanged = tab.editor.read(cx).value() == submitted;
                let mut created = false;
                match result {
                    Ok(result) if result.created => {
                        created = true;
                        tab.baseline = submitted;
                        tab.message = format!(
                            "Created {} successfully. Newer edits, if any, are preserved.",
                            result.name
                        );
                    }
                    Ok(result) => {
                        tab.message = format!(
                            "Server dry-run validated for {}. No resource created.",
                            result.name
                        )
                    }
                    Err(error) => {
                        tab.message = format!("Create failed: {error:?}. Buffer preserved.")
                    }
                }
                if created {
                    if close && unchanged {
                        app.close_yaml_tab(key, cx);
                    }
                    app.reload_active_section(window, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(crate) fn render_yaml_dock(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(tab) = self
            .yaml_dock_tabs
            .iter()
            .find(|t| Some(t.key) == self.active_yaml_tab)
        else {
            return Empty.into_any_element();
        };
        let key = tab.key;
        let valid = self.yaml_lease_valid(tab);
        let is_dirty = tab.dirty(cx);
        let busy = self
            .yaml_dock_tabs
            .iter()
            .any(|t| t.operation.is_some() && self.yaml_lease_valid(t));
        let creating = matches!(tab.target, YamlTarget::Create { .. });
        let readable = match &tab.target {
            YamlTarget::Edit(reference) => {
                self.can_reference_action(reference, PermissionAction::Get)
            }
            _ => false,
        };
        let writable = match &tab.target {
            YamlTarget::Edit(reference) => {
                self.can_reference_action(reference, PermissionAction::Patch)
            }
            YamlTarget::Create { kind, namespace } => {
                self.can_action(*kind, PermissionAction::Create, namespace.as_deref(), None)
            }
        };
        let save_disabled =
            !writable || !valid || busy || (!creating && tab.loaded.is_none()) || !is_dirty;
        let close_disabled = (is_dirty && !writable)
            || !valid
            || busy
            || (is_dirty && !creating && tab.loaded.is_none());
        let editor = tab.editor.clone();
        let message = if valid {
            tab.message.clone()
        } else {
            "Session ended or changed. Buffer retained; saving is disabled.".into()
        };
        v_flex().size_full().min_h_0().min_w_0()
            .child(div().px_2().py_1().text_xs().child(format!("{:?} · {} · {}", tab.kind(), tab.name(), tab.namespace().unwrap_or("cluster-scoped"))))
            .child(h_flex().gap_2().px_2().py_1().flex_wrap()
                .when(!creating, |toolbar| toolbar.child(Button::new("yaml-reload").small().icon(crate::action_icons::refresh()).accessibility_label("Reload YAML").ghost().label("Reload").disabled(!readable || !valid || busy).on_click(cx.listener(move |app, _, window, cx| {
                    if is_dirty { app.confirm_destructive_action("Reload YAML?", "Discard this buffer and read current reviewed fields from the server?", "Reload", window, cx, move |app, window, cx| app.run_yaml_operation(key, EditMode::Read, false, window, cx)); }
                    else { app.run_yaml_operation(key, EditMode::Read, false, window, cx); }
                }))))
                .child(Button::new("yaml-dry-run").small().icon(IconName::Check).accessibility_label("Server dry-run").ghost().label("Dry-run").disabled(save_disabled).on_click(cx.listener(move |app, _, window, cx| app.run_yaml_operation(key, EditMode::DryRun, false, window, cx))))
                .child(Button::new("yaml-apply").small().icon(IconName::Check).accessibility_label(if creating { "Create resource from YAML" } else { "Apply reviewed YAML changes" }).label(if creating { "Create" } else { "Apply" }).disabled(save_disabled).on_click(cx.listener(move |app, _, window, cx| app.run_yaml_operation(key, EditMode::Apply, false, window, cx))))
                .child(Button::new("yaml-save-close").small().icon(IconName::Check).accessibility_label("Save YAML and close tab").label("Save & Close").disabled(close_disabled).on_click(cx.listener(move |app, _, window, cx| {
                    let dirty = app.yaml_dock_tabs.iter().find(|t| t.key == key).is_some_and(|t| t.dirty(cx));
                    if dirty { app.run_yaml_operation(key, EditMode::Apply, true, window, cx); } else { app.close_yaml_tab(key, cx); }
                }))))
            .child(div().text_xs().px_2().text_color(cx.theme().muted_foreground).child(message))
            .child(div().flex_1().min_h_0().min_w_0().child(EditorSurface::new(&editor, &self.preferences_draft.editor)))
            .into_any_element()
    }
}
#[cfg(test)]
mod tests {
    use super::{YamlTarget, dirty, response_is_current, same_lease};
    use iceon_core::connection::{ResourceKind, details::ResourceReference};
    #[test]
    fn creation_targets_are_distinct_from_edits_and_other_namespaces() {
        let create = |namespace: &str| YamlTarget::Create {
            kind: ResourceKind::Pods,
            namespace: Some(namespace.into()),
        };
        assert_eq!(create("default"), create("default"));
        assert_ne!(create("default"), create("production"));
        assert_ne!(
            create("default"),
            YamlTarget::Create {
                kind: ResourceKind::Deployments,
                namespace: Some("default".into()),
            }
        );
        assert_ne!(
            create("default"),
            YamlTarget::Edit(ResourceReference {
                kind: ResourceKind::Pods,
                namespace: Some("default".into()),
                name: "existing".into(),
                uid: Some("uid".into()),
            })
        );
    }
    #[test]
    fn new_templates_remain_unsaved_and_created_baseline_preserves_newer_edits() {
        let template = crate::cluster_view::default_yaml_template(ResourceKind::Pods, "fixture");
        assert!(dirty("", &template));
        assert!(template.contains("namespace: fixture"));
        assert!(!dirty(&template, &template));
        assert!(dirty(&template, &format!("{template}\n# newer changes")));
    }
    #[test]
    fn late_responses_require_tab_operation_and_lease_identity() {
        assert!(response_is_current(4, 4, Some(8), 8, 9, true));
        assert!(response_is_current(4, 4, Some(9), 8, 9, true));
        assert!(!response_is_current(5, 4, Some(8), 8, 9, true));
        assert!(!response_is_current(4, 4, Some(7), 8, 9, true));
        assert!(!response_is_current(4, 4, None, 8, 9, true));
        assert!(!response_is_current(4, 4, Some(8), 8, 9, false));
    }
    #[test]
    fn buffers_are_dirty_until_identical_to_baseline() {
        assert!(!dirty("a", "a"));
        assert!(dirty("a", "b"));
        assert!(dirty("a", ""));
    }
    #[test]
    fn session_identity_requires_cluster_and_lease() {
        assert!(same_lease("a", "1", Some(("a", "1"))));
        assert!(!same_lease("a", "1", Some(("a", "2"))));
        assert!(!same_lease("a", "1", Some(("b", "1"))));
        assert!(!same_lease("a", "1", None));
    }
}
