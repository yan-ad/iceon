//! Sequential, identity-preconditioned deletion of explicitly checked table rows.
use crate::{IceonApp, inspector_view::DeleteTarget};
use gpui_kit::{prelude::*, *};
use iceon_core::connection::{
    ResourceKind,
    delete::DeleteRequest,
    pods::{PodAction, PodMutationRequest, PodRequest},
};

#[derive(Default)]
pub(crate) struct BulkDeleteState {
    pub active: Option<u64>,
    pub message: Option<String>,
}
impl IceonApp {
    pub(crate) fn confirm_bulk_delete(
        &mut self,
        force: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.bulk_delete.active.is_some() {
            return;
        }
        let targets = self
            .resource_table
            .as_ref()
            .map(|table| table.read(cx).selected_targets())
            .unwrap_or_default();
        if targets.iter().any(|target| !self.can_delete_target(target))
            || targets.is_empty()
            || (force
                && targets
                    .iter()
                    .any(|target| target.kind != ResourceKind::Pods))
        {
            return;
        }
        let names = targets
            .iter()
            .take(8)
            .map(|target| {
                format!(
                    "{}/{}",
                    target.namespace.as_deref().unwrap_or("cluster"),
                    target.name
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        self.confirm_destructive_action(
            format!("{} {} selected resources?", if force { "Force delete" } else { "Delete" }, targets.len()),
            format!("Only the checked, loaded rows will be affected. UID/resourceVersion preconditions are enforced.\n{names}{}\n{}", if targets.len() > 8 { "\n…" } else { "" }, if force { "Zero grace period: workloads may remain running. Finalizers are NOT removed." } else { "Normal deletion respects grace periods and finalizers." }),
            if force { "Force Delete" } else { "Delete" }, window, cx,
            move |app, window, cx| app.start_bulk_delete(targets.clone(), force, window, cx),
        );
    }
    fn start_bulk_delete(
        &mut self,
        targets: Vec<DeleteTarget>,
        force: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.bulk_delete.active.is_some() {
            return;
        }
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let current = self
            .resource_table
            .as_ref()
            .map(|table| table.read(cx).selected_targets())
            .unwrap_or_default();
        if targets
            .iter()
            .any(|target| !current.contains(target) || !self.can_delete_target(target))
        {
            return;
        }

        let batch = self.next_operation_id();
        self.bulk_delete.active = Some(batch);
        self.bulk_delete.message = Some(format!("Deleting {} selected resources…", targets.len()));
        let connection = self.connection.clone();
        cx.notify();
        cx.spawn_in(window, async move |owner, cx| {
            let mut completed = 0;
            let mut failures = Vec::new();
            for target in targets {
                let read_id = owner.update_in(cx, |app, _, _| {
                    (app.can_delete_target(&target) && app.bulk_delete.active == Some(batch) && app.cluster_session.as_ref().is_some_and(|current| current.cluster_id == session.cluster_id && current.session_id == session.session_id)).then(|| app.next_operation_id())
                }).ok().flatten();
                let Some(read_id) = read_id else { break; };
                let result = if force {
                    connection.pod_mutate(PodMutationRequest {
                        pod: PodRequest { cluster_id: session.cluster_id.clone(), session_id: session.session_id.clone(), namespace: target.namespace.clone().unwrap_or_default(), name: target.name.clone(), uid: Some(target.uid.clone()) },
                        action: PodAction::ForceDelete, resource_version: target.resource_version.clone(), yaml: None,
                    }).await.map(|_| ())
                } else {
                    connection.delete(DeleteRequest { cluster_id: session.cluster_id.clone(), session_id: session.session_id.clone(), kind: target.kind, namespace: target.namespace.clone(), name: target.name.clone(), uid: target.uid.clone(), resource_version: target.resource_version.clone(), read_id }).await.and_then(|result| {
                        if result.accepted || result.deleted { Ok(()) } else { Err(iceon_core::connection::ConnectionError::InvalidInput) }
                    })
                };
                match result { Ok(()) => completed += 1, Err(error) => failures.push(format!("{}: {error:?}", target.name)) }
            }
            let _ = owner.update_in(cx, |app, window, cx| {
                if app.bulk_delete.active != Some(batch) { return; }
                app.bulk_delete.active = None;
                if !app.cluster_session.as_ref().is_some_and(|current| current.cluster_id == session.cluster_id && current.session_id == session.session_id) { app.bulk_delete.message = None; cx.notify(); return; }
                app.reload_active_section(window, cx);
                app.bulk_delete.message = Some(format!("{completed} deletion requests accepted (grace periods/finalizers may still be pending); {} failed. {}", failures.len(), failures.join("; ")));
                cx.notify();
            });
        }).detach();
    }
}
