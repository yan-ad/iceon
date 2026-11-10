//! Standard GPUI Kit dock chrome for the cluster workspace.
//!
//! Construct once, after the `IceonApp` entity exists. The owner retains this
//! value and calls `sync` before rendering `dock_area.clone()`. Do not observe
//! pane notifications back into the owner: `sync` already invalidates their
//! projections whenever the owner's stream-backed state is rendered.
//!
//! The owner supplies `render_cluster_main` (toolbar and body, never this dock),
//! the sidebar, inspector and console renderers, and an `inspector_open` flag.
//! Those renderers must fill their allocated slot rather than fix its extent.

use std::{cell::Cell, rc::Rc};

use gpui_kit::component::{
    Icon, IconName,
    dock::{
        BasePanel, DockArea, DockLayout, DockPlacement, DockSkin, DockToggleEvent, Panel,
        PanelControl, PanelEvent, panel_handle,
    },
    h_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::{IceonApp, navigation::Route};

const SIDEBAR_WIDTH: f32 = 252.;
const INSPECTOR_WIDTH: f32 = 420.;
const CONSOLE_HEIGHT: f32 = 320.;

/// Retains the layout and its pane entities across parent renders and routes.
pub(crate) struct WorkspaceDock {
    pub(crate) dock_area: Entity<DockArea>,
    // Kept alongside the area so appearance can be changed without rebuilding it.
    pub(crate) dock_skin: Rc<DockSkin>,
    panels: [Entity<WorkspacePanel>; 4],
    desired: Rc<Cell<Option<Visibility>>>,
    _toggle_subscription: Subscription,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Visibility {
    sidebar: bool,
    console: bool,
    inspector: bool,
}

impl Visibility {
    fn at(self, placement: DockPlacement) -> bool {
        match placement {
            DockPlacement::Left => self.sidebar,
            DockPlacement::Bottom => self.console,
            DockPlacement::Right => self.inspector,
            DockPlacement::Center => true,
        }
    }

    fn with_toggle(mut self, placement: DockPlacement, open: bool) -> Self {
        match placement {
            DockPlacement::Left => self.sidebar = open,
            DockPlacement::Bottom => self.console = open,
            DockPlacement::Right => self.inspector = open,
            DockPlacement::Center => {}
        }
        self
    }
}

pub(crate) fn create(
    owner: WeakEntity<IceonApp>,
    window: &mut Window,
    cx: &mut App,
) -> WorkspaceDock {
    let panels = [
        PanelKind::Resources,
        PanelKind::Cluster,
        PanelKind::Inspector,
        PanelKind::Console,
    ]
    .map(|kind| {
        cx.new(|cx| WorkspacePanel {
            owner: owner.clone(),
            kind,
            focus_handle: cx.focus_handle(),
        })
    });
    let (dock_area, dock_skin) = DockSkin::dock_area("iceon-workspace", Some(1), window, cx);
    dock_area.update(cx, |area, cx| {
        area.set_center(
            DockLayout::tabs().panel_view(panel_handle(panels[1].clone()), cx),
            window,
            cx,
        );
        for (placement, index, size) in [
            (DockPlacement::Left, 0, SIDEBAR_WIDTH),
            (DockPlacement::Right, 2, INSPECTOR_WIDTH),
            (DockPlacement::Bottom, 3, CONSOLE_HEIGHT),
        ] {
            area.set_dock(
                placement,
                DockLayout::tabs().panel_view(panel_handle(panels[index].clone()), cx),
                window,
                cx,
            );
            area.set_dock_size(placement, px(size), window, cx);
        }
    });

    let desired = Rc::new(Cell::new(None::<Visibility>));
    let user_desired = desired.clone();
    let toggle_subscription = cx.subscribe(&dock_area, move |_, event: &DockToggleEvent, cx| {
        // Record the user's choice before notifying the owner. It is not
        // an application request to unzoom or restore a previous layout.
        if let Some(previous) = user_desired.get() {
            user_desired.set(Some(
                previous.with_toggle(event.placement(), event.is_open()),
            ));
        }
        let _ = owner.update(cx, |app, cx| {
            match event.placement() {
                DockPlacement::Left => {
                    app.sidebar_collapsed = !event.is_open();
                    if event.is_open() {
                        app.cluster_sidebar_collapsed = false;
                    }
                }
                DockPlacement::Bottom => app.pod_dock_open = event.is_open(),
                // Collapsing chrome must not discard an inspected target,
                // its loaded YAML, or an in-flight detail request.
                DockPlacement::Right => app.inspector_open = event.is_open(),
                DockPlacement::Center => return,
            }
            cx.notify();
        });
    });

    WorkspaceDock {
        dock_area,
        dock_skin,
        panels,
        desired,
        _toggle_subscription: toggle_subscription,
    }
}

impl WorkspaceDock {
    /// Apply only changes in application intent, preserving user resizing,
    /// tab placement and zoom on ordinary stream updates / parent renders.
    pub(crate) fn sync(
        &mut self,
        sidebar_open: bool,
        console_open: bool,
        inspector_open: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let next = Visibility {
            sidebar: sidebar_open,
            console: console_open,
            inspector: inspector_open,
        };
        let previous = self.desired.replace(Some(next));
        self.dock_area.update(cx, |area, cx| {
            // An explicit app open/close must be visible even when another
            // pane was maximized. Repaints alone must never cancel user zoom.
            if previous.is_some_and(|previous| previous != next) && area.is_zoomed() {
                area.set_zoomed_out(window, cx);
            }
            for placement in [
                DockPlacement::Left,
                DockPlacement::Bottom,
                DockPlacement::Right,
            ] {
                if previous.is_none_or(|previous| previous.at(placement) != next.at(placement))
                    && area.is_dock_open(placement) != next.at(placement)
                {
                    area.toggle_dock(placement, window, cx);
                }
            }
        });

        // These panes project the parent's state rather than keeping their
        // own copies. Invalidation is one-way: panes -> DockArea, not owner.
        // It emits no PanelEvent::LayoutChanged and never replaces the tree.
        for panel in &self.panels {
            panel.update(cx, |_, cx| cx.notify());
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PanelKind {
    Resources,
    Cluster,
    Inspector,
    Console,
}

impl PanelKind {
    fn name(self) -> &'static str {
        match self {
            Self::Resources => "iceon.workspace.resources",
            Self::Cluster => "iceon.workspace.cluster",
            Self::Inspector => "iceon.workspace.inspector",
            Self::Console => "iceon.workspace.console",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Resources => "Resources",
            Self::Cluster => "Cluster",
            Self::Inspector => "Inspector",
            Self::Console => "Pod console",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Resources => IconName::PanelLeft,
            Self::Cluster => IconName::Network,
            Self::Inspector => IconName::Info,
            Self::Console => IconName::SquareTerminal,
        }
    }
}

struct WorkspacePanel {
    owner: WeakEntity<IceonApp>,
    kind: PanelKind,
    focus_handle: FocusHandle,
}

impl EventEmitter<PanelEvent> for WorkspacePanel {}

impl Focusable for WorkspacePanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for WorkspacePanel {
    fn panel_name(&self) -> &'static str {
        self.kind.name()
    }

    fn closable(&self, _: &App) -> bool {
        // Collapse is reversible; closing would remove a permanent workspace
        // pane and require rebuilding the layout to recover it.
        false
    }
}

impl Panel for WorkspacePanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_2()
            .items_center()
            .child(Icon::new(self.kind.icon()).size(px(14.)))
            .child(self.kind.title())
    }

    fn tab_name(&self, _: &App) -> Option<SharedString> {
        Some(self.kind.title().into())
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }

    fn zoom_control(&self, _: &App) -> Option<PanelControl> {
        Some(PanelControl::Toolbar)
    }
}

impl Render for WorkspacePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let kind = self.kind;
        let content = self
            .owner
            .update(cx, |app, cx| match kind {
                PanelKind::Resources => {
                    let Route::Cluster { cluster_id, .. } = app.history.current().clone() else {
                        return Empty.into_any_element();
                    };
                    app.render_cluster_sidebar(&cluster_id, cx)
                        .into_any_element()
                }
                // This entry point MUST NOT render the enclosing dock again.
                PanelKind::Cluster => app.render_cluster_main(window, cx).into_any_element(),
                PanelKind::Inspector => app.render_inspector_drawer(window, cx).into_any_element(),
                PanelKind::Console => app.render_pod_dock_panel(window, cx).into_any_element(),
            })
            .unwrap_or_else(|_| Empty.into_any_element());

        div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .track_focus(&self.focus_handle)
            .child(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn panel_ids_are_stable_and_distinct() {
        let kinds = [
            PanelKind::Resources,
            PanelKind::Cluster,
            PanelKind::Inspector,
            PanelKind::Console,
        ];
        assert_eq!(
            kinds.map(PanelKind::name),
            [
                "iceon.workspace.resources",
                "iceon.workspace.cluster",
                "iceon.workspace.inspector",
                "iceon.workspace.console",
            ]
        );
        for (index, kind) in kinds.iter().enumerate() {
            assert!(
                kinds[index + 1..]
                    .iter()
                    .all(|other| kind.name() != other.name())
            );
        }
    }

    #[::core::prelude::v1::test]
    fn user_toggle_changes_only_its_edge() {
        let all_open = Visibility {
            sidebar: true,
            console: true,
            inspector: true,
        };
        for placement in [
            DockPlacement::Left,
            DockPlacement::Bottom,
            DockPlacement::Right,
        ] {
            let changed = all_open.with_toggle(placement, false);
            assert!(!changed.at(placement));
            assert!(changed.at(DockPlacement::Center));
            assert_eq!(changed.with_toggle(placement, true), all_open);
        }
        assert_eq!(all_open.with_toggle(DockPlacement::Center, false), all_open);
    }
}
