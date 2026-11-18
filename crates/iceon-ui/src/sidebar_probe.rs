//! Offline native interaction audit of the production sidebar/NavigationGroup.
//! Run `cargo run --locked -p iceon-app -- --sidebar-probe` (debug only).
//! Uses synthetic extension destinations so no discovery lease, kubeconfig,
//! resource request, startup worker, or local shell is needed. Native pointer
//! input exercises the real submenu row and caret; geometry is measured from
//! the kit's actual virtual list, not inferred from an application boolean.
//! This is not an accessibility-tree or full-shell screenshot assertion.
use crate::{IceonApp, navigation::Route};
use gpui_kit::{
    component::{
        Sizable as _,
        button::{Button, ButtonVariants as _},
        theme::ThemeMode,
    },
    prelude::*,
    *,
};
use iceon_core::{
    catalog::CatalogEngine, connection::ConnectionEngine, preferences::PreferencesEngine, secrets,
};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::Duration,
};

const CLUSTER: &str = "offline-sidebar";
type Outcome = Rc<RefCell<Option<Result<(), String>>>>;
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        for nonce in 0u64.. {
            let path = std::env::temp_dir().join(format!(
                "iceon-sidebar-probe-{}-{nonce}",
                std::process::id()
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot create sidebar fixture: {error}"),
            }
        }
        unreachable!()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Probe {
    owner: Entity<IceonApp>,
    list: Rc<RefCell<Option<ListState>>>,
    sidebar_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    toggle_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    expanded_height: Option<Pixels>,
    collapsed_height: Option<Pixels>,
    expanded_width: Option<Pixels>,
    initial_operation: u64,
    _fixture: Rc<Fixture>,
    _subscription: Subscription,
}
impl Probe {
    fn new(fixture: Rc<Fixture>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let catalog = Arc::new(CatalogEngine::new_with_home(fixture.0.clone(), None));
        let connection = Arc::new(ConnectionEngine::new(catalog.clone()));
        let preferences = PreferencesEngine::new(fixture.0.clone()).expect("fixture preferences");
        let secrets = secrets::Engine::new(fixture.0.clone());
        let owner =
            cx.new(|cx| IceonApp::new(catalog, connection, preferences, secrets, window, cx));
        owner.update(cx, |app, _| {
            app.local_terminal_active = false;
            app.navigation_widths = Default::default();
            // No session is installed. Unknown standard-resource visibility is
            // intentionally not elevated; two synthetic extension rows exercise
            // the same production NavigationGroup renderer without permissions.
            app.history.push(Route::Cluster {
                cluster_id: CLUSTER.into(),
                section: "overview".into(),
            });
            app.preferences_draft.app.enabled_extensions = ["keda".into(), "cnpg".into()].into();
        });
        let initial_operation = owner.read(cx).operation_counter;
        let subscription = cx.observe(&owner, |_, _, cx| cx.notify());
        Self {
            owner,
            list: Rc::default(),
            sidebar_bounds: Rc::default(),
            toggle_bounds: Rc::default(),
            expanded_height: None,
            collapsed_height: None,
            expanded_width: None,
            initial_operation,
            _fixture: fixture,
            _subscription: subscription,
        }
    }
    fn check(
        &mut self,
        stage: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let require = |ok, message: &str| if ok { Ok(()) } else { Err(message.to_owned()) };
        let app = self.owner.read(cx);
        require(
            app.cluster_session.is_none(),
            "fixture unexpectedly acquired a session",
        )?;
        require(
            app.operation_counter == self.initial_operation,
            "interaction scheduled an operation",
        )?;
        require(
            app.device_terminals.is_empty(),
            "interaction started a terminal",
        )?;
        require(
            app.history.current()
                == &(Route::Cluster {
                    cluster_id: CLUSTER.into(),
                    section: "overview".into(),
                }),
            "parent or rail toggle unexpectedly navigated",
        )?;
        require(
            !app.route_visible("pods"),
            "fixture must not grant resource visibility",
        )?;
        let rail = app.sidebar_collapsed;
        let list = self
            .list
            .borrow()
            .clone()
            .ok_or("sidebar list did not render")?;
        require(
            list.item_count() == 1,
            "expected the real Extensions navigation group only",
        )?;
        let group = list
            .bounds_for_item(0)
            .ok_or("navigation group was not measured")?;
        let sidebar = self
            .sidebar_bounds
            .get()
            .ok_or("sidebar was not measured")?;
        require(sidebar.size.height > px(0.), "sidebar has no height")?;
        let toggle = self.toggle_bounds.get().ok_or("toggle was not measured")?;
        match stage {
            0 => {
                require(
                    !rail && group.size.height < px(70.),
                    "parent must start collapsed without exposing children",
                )?;
                self.collapsed_height = Some(group.size.height);
                self.expanded_width = Some(sidebar.size.width);
                click(
                    window,
                    cx,
                    point(group.left() + px(50.), group.top() + px(26.)),
                );
            }
            1 => {
                require(
                    !rail && group.size.height > self.collapsed_height.unwrap() + px(30.),
                    "parent row failed to expose both children",
                )?;
                self.expanded_height = Some(group.size.height);
                click(
                    window,
                    cx,
                    point(group.right() - px(18.), group.top() + px(26.)),
                );
            }
            2 => {
                require(
                    !rail && group.size.height == self.collapsed_height.unwrap(),
                    "caret failed to collapse children",
                )?;
                click(window, cx, toggle.center());
            }
            3 | 5 => {
                require(
                    rail && sidebar.size.width == px(crate::cluster_view::SIDEBAR_RAIL_WIDTH),
                    "toggle failed to produce the shared 48px icon rail",
                )?;
                require(
                    sidebar.size.width < self.expanded_width.unwrap(),
                    "rail did not shrink the actual layout",
                )?;
                require(
                    group.size.height < px(55.),
                    "rail must contain one parent, not flattened child destinations",
                )?;
                if stage == 3 {
                    // The visible parent icon must open the actual tree. The kit
                    // otherwise hides children in collapsed mode (no dropdown).
                    click(window, cx, group.center());
                } else {
                    click(window, cx, toggle.center());
                }
            }
            4 | 6 => {
                require(
                    !rail && sidebar.size.width == self.expanded_width.unwrap(),
                    "parent activation/toggle failed to restore expanded width",
                )?;
                require(
                    group.size.height == self.expanded_height.unwrap(),
                    "rail parent did not expose children or retain tree expansion",
                )?;
                if stage == 4 {
                    click(window, cx, toggle.center());
                }
            }
            _ => return Err("unknown probe stage".into()),
        }
        Ok(())
    }
}
impl Render for Probe {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = self
            .owner
            .read(cx)
            .navigation_widths
            .effective_sidebar_width(true, self.owner.read(cx).sidebar_collapsed);
        let owner = self.owner.clone();
        let toggle_bounds = self.toggle_bounds.clone();
        let sidebar_bounds = self.sidebar_bounds.clone();
        let list = self.list.clone();
        let (sidebar_type, sidebar) = self.owner.update(cx, |app, cx| {
            let sidebar = app.render_cluster_sidebar(CLUSTER, cx);
            (
                std::any::type_name_of_val(&sidebar),
                sidebar.into_any_element(),
            )
        });
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div().h(px(44.)).child(
                    Button::new("sidebar-probe-toggle")
                        .small()
                        .ghost()
                        .label("Toggle sidebar")
                        .accessibility_label("Toggle sidebar")
                        .on_click(move |_, _, cx| {
                            owner.update(cx, |app, cx| {
                                app.sidebar_collapsed = !app.sidebar_collapsed;
                                cx.notify();
                            })
                        })
                        .child(
                            canvas(
                                move |bounds, _, _| toggle_bounds.set(Some(bounds)),
                                |_, _, _, _| {},
                            )
                            .absolute()
                            .inset_0(),
                        ),
                ),
            )
            .child(
                div().flex_1().min_h_0().flex().child(
                    div().w(px(width)).h_full().relative().child(sidebar).child(
                        canvas(
                            move |bounds, window, cx| {
                                sidebar_bounds.set(Some(bounds));
                                // Same identity/scope used by Sidebar::render, read after
                                // it has laid out; do not substitute a test-only list.
                                let state = window.with_id(sidebar_type, |window| {
                                    window.use_keyed_state(
                                        SharedString::from(format!(
                                            "cluster-navigation-{CLUSTER}-list-state"
                                        )),
                                        cx,
                                        |_, _| ListState::new(0, ListAlignment::Top, px(0.)),
                                    )
                                });
                                *list.borrow_mut() = Some(state.read(cx).clone());
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0(),
                    ),
                ),
            )
    }
}
fn click(window: &mut Window, cx: &mut App, position: Point<Pixels>) {
    window.dispatch_event(
        PlatformInput::MouseMove(MouseMoveEvent {
            position,
            pressed_button: None,
            modifiers: Modifiers::default(),
        }),
        cx,
    );
    window.dispatch_event(
        PlatformInput::MouseDown(MouseDownEvent {
            button: MouseButton::Left,
            position,
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        }),
        cx,
    );
    window.dispatch_event(
        PlatformInput::MouseUp(MouseUpEvent {
            button: MouseButton::Left,
            position,
            modifiers: Modifiers::default(),
            click_count: 1,
        }),
        cx,
    );
}
pub fn run() {
    let runtime = iceon_core::runtime::handle();
    let _guard = runtime.enter();
    let fixture = Rc::new(Fixture::new());
    let outcome: Outcome = Rc::default();
    let result = outcome.clone();
    gpui_kit::application().with_assets(crate::app_assets::AppAssets).run(move |cx| {
        gpui_kit::init(cx);
        crate::console_font::init(cx);
        crate::shell_theme::apply(ThemeMode::Dark, None, cx);
        let (handle, view) = crate::shell_theme::open_window(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(640.), px(420.)), cx))),
            ..Default::default()
        }, cx, move |window, cx| {
            window.set_window_title("Iceon sidebar — offline interaction probe");
            cx.new(|cx| Probe::new(fixture, window, cx))
        }).expect("offline sidebar window");
        let watchdog = result.clone();
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_secs(10)).await;
            cx.update(|cx| { if watchdog.borrow().is_none() { *watchdog.borrow_mut() = Some(Err("native watchdog expired".into())); } cx.quit(); });
        }).detach();
        cx.on_window_closed(|cx, _| { if cx.windows().is_empty() { cx.quit(); } }).detach();
        cx.spawn(async move |cx| {
            for stage in 0..7 {
                cx.background_executor().timer(Duration::from_millis(350)).await;
                let checked = handle.update(cx, |_, window, cx| {
                    window.draw(cx).clear(cx);
                    let checked = view.update(cx, |probe, cx| probe.check(stage, window, cx));
                    if let Err(error) = checked {
                        eprintln!("FAIL sidebar_probe stage {stage}: {error}");
                        *result.borrow_mut() = Some(Err(error));
                        cx.quit();
                        return false;
                    }
                    if stage == 6 {
                        println!("PASS sidebar_probe: initially collapsed -> parent-row open -> caret collapse -> single-parent 48px rail -> parent activation opens tree; rail roundtrip retains measured child layout; no session, resource visibility, or shell");
                        *result.borrow_mut() = Some(Ok(()));
                        cx.quit();
                    }
                    true
                });
                if !matches!(checked, Ok(true)) { return; }
            }
        }).detach();
        cx.activate(true);
    });
    match outcome.borrow_mut().take() {
        Some(Ok(())) => {}
        Some(Err(error)) => panic!("FAIL sidebar_probe: {error}"),
        None => panic!("FAIL sidebar_probe: window closed before verification"),
    }
}
