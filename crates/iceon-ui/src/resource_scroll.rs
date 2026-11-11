//! View-owned scroll state; rebuilding a virtual list must not reset its offset.

use gpui_kit::component::VirtualListScrollHandle;
use iceon_core::connection::ResourceKind;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResourceScrollScope {
    pub cluster_id: String,
    pub kind: ResourceKind,
    pub namespace: Option<String>,
    pub query: String,
    pub status: Option<String>,
    pub page_token: Option<String>,
}

pub(crate) struct ResourceScrollState {
    handle: VirtualListScrollHandle,
    scope: Option<ResourceScrollScope>,
}

impl ResourceScrollState {
    pub fn for_scope(&mut self, scope: ResourceScrollScope) -> VirtualListScrollHandle {
        if self.scope.as_ref() != Some(&scope) {
            self.handle = VirtualListScrollHandle::new();
            self.scope = Some(scope);
        }
        self.handle.clone()
    }
}

impl Default for ResourceScrollState {
    fn default() -> Self {
        Self {
            handle: VirtualListScrollHandle::new(),
            scope: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ResourceScrollScope, ResourceScrollState};
    use gpui_kit::{point, px};
    use iceon_core::connection::ResourceKind;

    fn pods_scope() -> ResourceScrollScope {
        ResourceScrollScope {
            cluster_id: "cluster-a".into(),
            kind: ResourceKind::Pods,
            namespace: None,
            query: String::new(),
            status: None,
            page_token: None,
        }
    }

    #[test]
    fn redraws_and_live_updates_keep_the_same_scroll_offset() {
        let mut state = ResourceScrollState::default();
        let original = state.for_scope(pods_scope());
        original.set_offset(point(px(0.), px(-420.)));
        let redrawn = state.for_scope(pods_scope());
        assert_eq!(redrawn.offset(), original.offset());
        redrawn.set_offset(point(px(0.), px(-840.)));
        assert_eq!(original.offset(), redrawn.offset());
    }

    #[test]
    fn changing_any_resource_scope_resets_the_offset() {
        for field in 0..6 {
            let mut changed = pods_scope();
            match field {
                0 => changed.cluster_id = "cluster-b".into(),
                1 => changed.kind = ResourceKind::Deployments,
                2 => changed.namespace = Some("default".into()),
                3 => changed.query = "nginx".into(),
                4 => changed.status = Some("Running".into()),
                _ => changed.page_token = Some("opaque-token".into()),
            }
            let mut state = ResourceScrollState::default();
            state
                .for_scope(pods_scope())
                .set_offset(point(px(0.), px(-420.)));
            assert_eq!(state.for_scope(changed).offset(), point(px(0.), px(0.)));
        }
    }
}
