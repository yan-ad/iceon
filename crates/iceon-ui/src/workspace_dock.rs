//! Standard Dock behavior belongs only to the bottom console.
//! Resources and inspector are ordinary workspace views, not dockable panels.

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

use crate::IceonApp;

const CONSOLE_HEIGHT: f32 = 320.;

pub(crate) struct WorkspaceDock {
    pub(crate) dock_area: Entity<DockArea>,
    _dock_skin: Rc<DockSkin>,
    console: Entity<WorkspacePanel>,
    desired: Rc<Cell<Option<bool>>>,
    _toggle_subscription: Subscription,
    _owner_subscription: Subscription,
}

pub(crate) fn create(
    owner: WeakEntity<IceonApp>,
    window: &mut Window,
    cx: &mut App,
) -> WorkspaceDock {
    let main = cx.new(|cx| WorkspacePanel {
        owner: owner.clone(),
        kind: PanelKind::Cluster,
        focus_handle: cx.focus_handle(),
    });
    let console = cx.new(|cx| WorkspacePanel {
        owner: owner.clone(),
        kind: PanelKind::Console,
        focus_handle: cx.focus_handle(),
    });
    let (dock_area, dock_skin) = DockSkin::dock_area("iceon-workspace", Some(2), window, cx);
    dock_area.update(cx, |area, cx| {
        area.set_center(
            DockLayout::tabs().panel_view(panel_handle(main.clone()), cx),
            window,
            cx,
        );
        area.set_dock(
            DockPlacement::Bottom,
            DockLayout::tabs().panel_view(panel_handle(console.clone()), cx),
            window,
            cx,
        );
        area.set_dock_size(DockPlacement::Bottom, px(CONSOLE_HEIGHT), window, cx);
    });
    let desired = Rc::new(Cell::new(None::<bool>));
    let user_desired = desired.clone();
    let toggle_owner = owner.clone();
    let toggle_subscription = cx.subscribe(&dock_area, move |_, event: &DockToggleEvent, cx| {
        if event.placement() != DockPlacement::Bottom {
            return;
        }
        user_desired.set(Some(event.is_open()));
        let _ = toggle_owner.update(cx, |app, cx| {
            if app.pod_dock_open != event.is_open() {
                app.pod_dock_open = event.is_open();
                cx.notify();
            }
        });
    });
    // Invalidate projections only after actual owner notifications. Doing this in
    // render/sync invalidates the area during its own render and causes frame churn.
    let panes = [main.downgrade(), console.downgrade()];
    let owner_subscription =
        cx.observe(&owner.upgrade().expect("workspace owner"), move |_, cx| {
            for pane in &panes {
                let _ = pane.update(cx, |_, cx| cx.notify());
            }
        });
    WorkspaceDock {
        dock_area,
        _dock_skin: dock_skin,
        console,
        desired,
        _toggle_subscription: toggle_subscription,
        _owner_subscription: owner_subscription,
    }
}

impl WorkspaceDock {
    pub(crate) fn sync(&mut self, console_open: bool, window: &mut Window, cx: &mut App) {
        if !visibility_changed(self.desired.get(), console_open) {
            return;
        }
        self.desired.set(Some(console_open));
        self.dock_area.update(cx, |area, cx| {
            if area.is_zoomed() {
                area.set_zoomed_out(window, cx);
            }
            if area.is_dock_open(DockPlacement::Bottom) != console_open {
                area.toggle_dock(DockPlacement::Bottom, window, cx);
            }
        });
    }

    /// Log chunks/status changes do not need to invalidate the resource table.
    pub(crate) fn notify_console(&self, cx: &mut App) {
        self.console.update(cx, |_, cx| cx.notify());
    }
}

fn visibility_changed(previous: Option<bool>, next: bool) -> bool {
    previous != Some(next)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PanelKind {
    Cluster,
    Console,
}
impl PanelKind {
    fn name(self) -> &'static str {
        match self {
            Self::Cluster => "iceon.workspace.cluster",
            Self::Console => "iceon.workspace.console",
        }
    }
    fn chrome(self) -> bool {
        self == Self::Console
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
        false
    }
    fn zoomable(&self, _: &App) -> bool {
        self.kind.chrome()
    }
}
impl Panel for WorkspacePanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_2()
            .items_center()
            .child(Icon::new(IconName::SquareTerminal).size(px(14.)))
            .child("Terminal")
    }
    fn tab_name(&self, _: &App) -> Option<SharedString> {
        self.kind.chrome().then(|| "Terminal".into())
    }
    fn title_bar(&self, _: &App) -> bool {
        self.kind.chrome()
    }
    fn inner_padding(&self, _: &App) -> bool {
        false
    }
    fn zoom_control(&self, _: &App) -> Option<PanelControl> {
        self.kind.chrome().then_some(PanelControl::Toolbar)
    }
}
impl Render for WorkspacePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let kind = self.kind;
        let content = self
            .owner
            .update(cx, |app, cx| match kind {
                PanelKind::Cluster => app.render_cluster_main(window, cx),
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
    use super::{PanelKind, visibility_changed};
    #[test]
    fn only_bottom_console_has_dock_chrome() {
        assert!(!PanelKind::Cluster.chrome());
        assert!(PanelKind::Console.chrome());
        assert_ne!(PanelKind::Cluster.name(), PanelKind::Console.name());
    }
    #[test]
    fn unchanged_visibility_never_invalidates_the_dock() {
        for open in [false, true] {
            assert!(visibility_changed(None, open));
            assert!(!visibility_changed(Some(open), open));
            assert!(visibility_changed(Some(!open), open));
        }
    }
}
