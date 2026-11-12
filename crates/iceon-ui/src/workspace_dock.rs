//! Standard Dock behavior belongs only to the bottom console.
//! Resources and inspector are ordinary workspace views, not dockable panels.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gpui_kit::component::{
    Icon, IconName,
    dock::{
        BasePanel, DockArea, DockLayout, DockPlacement, DockSkin, DockToggleEvent, Panel,
        PanelControl, PanelEvent, PanelId, panel_handle,
    },
    h_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::{IceonApp, navigation::Route};

const CONSOLE_HEIGHT: f32 = 320.;

#[derive(PartialEq, Eq)]
struct ResourceProjection {
    route: Route,
    section: String,
    page: usize,
    namespace: Option<String>,
    namespaces: Vec<String>,
    query: String,
    status: Option<String>,
    loading: bool,
    error: Option<String>,
    sync_id: Option<String>,
    preferences_revision: u64,
    selected: usize,
    deleting: Option<u64>,
    delete_message: Option<String>,
    access_revision: u64,
}

fn resource_projection(app: &IceonApp, cx: &App) -> Option<ResourceProjection> {
    if !matches!(app.history.current(), Route::Cluster { .. })
        || crate::cluster_view::section_to_resource_kind(&app.active_cluster_section).is_none()
    {
        return None;
    }
    Some(ResourceProjection {
        route: app.history.current().clone(),
        section: app.active_cluster_section.clone(),
        page: app
            .resource_page
            .as_ref()
            .map(|page| Rc::as_ptr(page) as usize)
            .unwrap_or_default(),
        namespace: app.selected_namespace.clone(),
        namespaces: app.namespaces.clone(),
        query: app.resource_filter_input.read(cx).value().to_string(),
        status: app.selected_status_filter.clone(),
        loading: app.cluster_loading,
        error: app.cluster_error.clone(),
        sync_id: app.active_pod_sync_id.clone(),
        preferences_revision: app.preferences_revision,
        selected: app
            .resource_table
            .as_ref()
            .map(|table| table.read(cx).selected_targets().len())
            .unwrap_or_default(),
        deleting: app.bulk_delete.active,
        delete_message: app.bulk_delete.message.clone(),
        access_revision: app.access_revision(),
    })
}

pub(crate) struct WorkspaceDock {
    pub(crate) dock_area: Entity<DockArea>,
    _dock_skin: Rc<DockSkin>,
    console: Entity<WorkspacePanel>,
    desired: Rc<Cell<Option<bool>>>,
    console_height: Pixels,
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
    dock_skin.set_toggle_button_visible(false, cx);
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
    let main_projection = RefCell::new(resource_projection(owner.upgrade().unwrap().read(cx), cx));
    let main = main.downgrade();
    let console_projection = console.downgrade();
    let owner_subscription = cx.observe(
        &owner.upgrade().expect("workspace owner"),
        move |app, cx| {
            let next = resource_projection(app.read(cx), cx);
            if next.is_none() || *main_projection.borrow() != next {
                let _ = main.update(cx, |_, cx| cx.notify());
            }
            *main_projection.borrow_mut() = next;
            let _ = console_projection.update(cx, |_, cx| cx.notify());
        },
    );
    WorkspaceDock {
        dock_area,
        _dock_skin: dock_skin,
        console,
        desired,
        console_height: px(CONSOLE_HEIGHT),
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
        let console = self.console.clone();
        let height = &mut self.console_height;
        self.dock_area.update(cx, |area, cx| {
            if area.is_zoomed() {
                area.set_zoomed_out(window, cx);
            }
            if console_open {
                if !area.has_dock(DockPlacement::Bottom) {
                    area.set_dock(
                        DockPlacement::Bottom,
                        DockLayout::tabs().panel_view(panel_handle(console), cx),
                        window,
                        cx,
                    );
                    area.set_dock_size(DockPlacement::Bottom, *height, window, cx);
                }
                if !area.is_dock_open(DockPlacement::Bottom) {
                    area.toggle_dock(DockPlacement::Bottom, window, cx);
                }
            } else if area.has_dock(DockPlacement::Bottom) {
                if let Some(size) = area.dock_size(DockPlacement::Bottom) {
                    *height = size;
                }
                area.remove_dock(DockPlacement::Bottom, window, cx);
            }
        });
    }

    /// Log chunks/status changes do not need to invalidate the resource table.
    pub(crate) fn notify_console(&self, cx: &mut App) {
        self.console.update(cx, |_, cx| cx.notify());
    }

    pub(crate) fn toggle_console_zoom(&self, window: &mut Window, cx: &mut App) {
        let panel = PanelId::from(self.console.entity_id());
        self.dock_area.update(cx, |area, cx| {
            if area.is_zoomed() {
                area.set_zoomed_out(window, cx);
            } else if let Some(node) = area
                .layout(DockPlacement::Bottom)
                .and_then(|tree| tree.find_panel_node(panel))
            {
                area.set_zoomed_in(node, window, cx);
            }
        });
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

    fn title_bar(self) -> bool {
        false // All session navigation is owned by the single persistent TabBar.
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
            .child("Console")
    }
    fn tab_name(&self, _: &App) -> Option<SharedString> {
        self.kind.chrome().then(|| "Console".into())
    }
    fn title_bar(&self, _: &App) -> bool {
        self.kind.title_bar()
    }
    fn inner_padding(&self, _: &App) -> bool {
        false
    }
    fn zoom_control(&self, _: &App) -> Option<PanelControl> {
        None // Standard Button controls in the session row invoke the Dock API.
    }
}
impl Render for WorkspacePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let kind = self.kind;
        let content = self
            .owner
            .update(cx, |app, cx| match kind {
                PanelKind::Cluster => match app.history.current() {
                    Route::Setup => app.render_cluster_setup(cx).into_any_element(),
                    Route::Secrets => app.render_secrets_view(window, cx).into_any_element(),
                    Route::Cluster { .. } => app.render_cluster_main(window, cx),
                },
                PanelKind::Console => gpui_kit::component::v_flex()
                    .size_full()
                    .min_h_0()
                    .min_w_0()
                    .child(app.render_console_tabs(window, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .min_w_0()
                            .child(app.render_pod_dock_panel(window, cx)),
                    )
                    .into_any_element(),
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
        assert!(!PanelKind::Cluster.title_bar());
        assert!(!PanelKind::Console.title_bar());
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
