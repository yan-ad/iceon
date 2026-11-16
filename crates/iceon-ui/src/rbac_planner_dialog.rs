//! A weak-owner projection of the planner, bound to the opening Users scope.
use std::{cell::Cell, rc::Rc};

use crate::{IceonApp, navigation::Route};
use gpui_kit::component::{WindowExt as _, scroll::ScrollableElement as _};
use gpui_kit::prelude::*;
use gpui_kit::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PlannerScope {
    route: Route,
    section: String,
    cluster: String,
    session: String,
    namespace: Option<String>,
    namespaces: Vec<String>,
    // Once invalidated, returning to the opening route cannot revive old controls.
    active: Rc<Cell<bool>>,
}

pub(crate) fn users_header_route(route: &Route, section: &str) -> bool {
    matches!(route, Route::Cluster { section: current, .. }
        if current == section && matches!(section, "users" | "access-control/users"))
}

impl PlannerScope {
    fn capture(app: &IceonApp) -> Option<Self> {
        if !users_header_route(app.history.current(), &app.active_cluster_section) {
            return None;
        }
        let session = app.cluster_session.as_ref()?;
        let Route::Cluster { cluster_id, .. } = app.history.current() else {
            return None;
        };
        if cluster_id != &session.cluster_id {
            return None;
        }
        Some(Self {
            route: app.history.current().clone(),
            section: app.active_cluster_section.clone(),
            cluster: session.cluster_id.clone(),
            session: session.session_id.clone(),
            namespace: app.selected_namespace.clone(),
            namespaces: app.selected_namespaces.clone(),
            active: Rc::new(Cell::new(true)),
        })
    }

    pub(crate) fn is_current(&self, app: &IceonApp) -> bool {
        self.active.get()
            && Self::capture(app).is_some_and(|current| current == *self)
            && app.rbac_planner_scope.as_ref() == Some(self)
    }
}

struct PlannerDialog {
    owner: WeakEntity<IceonApp>,
    scope: PlannerScope,
}

impl IceonApp {
    pub(crate) fn open_rbac_planner(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.rbac_planner_scope.is_some() {
            return;
        }
        let Some(scope) = PlannerScope::capture(self) else {
            return;
        };
        self.rbac_planner_scope = Some(scope.clone());
        let owner = cx.entity().downgrade();
        let close_owner = owner.clone();
        let state = cx.new(|cx| {
            if let Some(app) = owner.upgrade() {
                let observed_scope = scope.clone();
                cx.observe(&app, move |_, app, cx| {
                    if !observed_scope.is_current(app.read(cx)) {
                        observed_scope.active.set(false);
                    }
                    cx.notify();
                })
                .detach();
            }
            PlannerDialog { owner, scope }
        });
        window.open_dialog(cx, move |dialog, _, _| {
            let owner = close_owner.clone();
            dialog
                .title("ServiceAccount planner")
                .width(px(760.0))
                .child(state.clone())
                .on_close(move |_, _, cx| {
                    let _ = owner.update(cx, |app, cx| {
                        if let Some(scope) = app.rbac_planner_scope.take() {
                            scope.active.set(false);
                        }
                        cx.notify();
                    });
                })
        });
        cx.notify();
    }

    // Keep all original control callbacks, but check their opening identity again
    // at dispatch time: an already-built element may outlive a navigation event.
    pub(crate) fn rbac_planner_listener<E: ?Sized>(
        &self,
        cx: &Context<Self>,
        listener: impl Fn(&mut Self, &E, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl Fn(&E, &mut Window, &mut App) + 'static {
        let scope = self.rbac_planner_scope.clone();
        cx.listener(move |app, event, window, cx| {
            if scope.as_ref().is_some_and(|scope| scope.is_current(app)) {
                listener(app, event, window, cx);
            }
        })
    }
}

impl Render for PlannerDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let scope = self.scope.clone();
        // Use an explicit update, never read-borrow the owner while constructing
        // its listeners. The dialog holds no strong reference to the app.
        let content = self
            .owner
            .update(cx, |app, cx| {
                if scope.is_current(app) {
                    app.render_rbac_planner_content(cx)
                } else {
                    scope.active.set(false);
                    div()
                        .p_4()
                        .child("This planner is no longer active. Close it and reopen from Users.")
                        .into_any_element()
                }
            })
            .unwrap_or_else(|_| div().child("Workspace closed.").into_any_element());
        div()
            .id("rbac-planner-dialog-scroll")
            .w_full()
            .max_h((window.viewport_size().height * 0.7).max(px(0.0)))
            .overflow_y_scrollbar()
            .child(content)
    }
}

#[cfg(test)]
mod tests {
    use super::{PlannerScope, users_header_route};
    use crate::navigation::Route;
    use std::{cell::Cell, rc::Rc};

    fn scope() -> PlannerScope {
        PlannerScope {
            route: Route::Cluster {
                cluster_id: "fixture".into(),
                section: "users".into(),
            },
            section: "users".into(),
            cluster: "fixture".into(),
            session: "lease".into(),
            namespace: Some("dev".into()),
            namespaces: vec!["dev".into()],
            active: Rc::new(Cell::new(true)),
        }
    }

    #[test]
    fn header_is_only_available_on_matching_users_routes() {
        for section in ["users", "access-control/users"] {
            let route = Route::Cluster {
                cluster_id: "fixture".into(),
                section: section.into(),
            };
            assert!(users_header_route(&route, section));
            assert!(!users_header_route(&route, "pods"));
        }
        assert!(!users_header_route(&Route::Setup, "users"));
        assert!(!users_header_route(&Route::Secrets, "users"));
        let route = Route::Cluster {
            cluster_id: "fixture".into(),
            section: "pods".into(),
        };
        assert!(!users_header_route(&route, "pods"));
    }

    #[test]
    fn dialog_identity_changes_with_namespace_route_cluster_and_session() {
        let opened = scope();
        assert_eq!(opened, scope());
        let mut changed = scope();
        changed.namespace = None;
        assert_ne!(opened, changed);
        let mut changed = scope();
        changed.namespaces.push("other".into());
        assert_ne!(opened, changed);
        let mut changed = scope();
        changed.route = Route::Setup;
        assert_ne!(opened, changed);
        let mut changed = scope();
        changed.section = "pods".into();
        assert_ne!(opened, changed);
        let mut changed = scope();
        changed.cluster = "other".into();
        assert_ne!(opened, changed);
        let mut changed = scope();
        changed.session = "replacement".into();
        assert_ne!(opened, changed);
    }

    #[test]
    fn closing_or_invalidating_scope_disables_all_captured_callbacks() {
        let opened = scope();
        let callback_scope = opened.clone();
        opened.active.set(false);
        assert!(!callback_scope.active.get());
        assert_ne!(opened, scope());
    }
}
