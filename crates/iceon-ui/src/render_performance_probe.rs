//! Native, fixture-only audit of the production ResourceTable and TerminalSurface.
//! The real IceonApp owner is retained but never rendered or attached to startup.
//! No connection, secrets worker, terminal process, or kubeconfig is started.

use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use gpui_kit::component::dock::{
    BasePanel, DockArea, DockLayout, DockPlacement, DockSkin, Panel, PanelEvent, panel_handle,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use iceon_core::{
    catalog::CatalogEngine,
    connection::{ConnectionEngine, ResourceItem, ResourceKind, ResourcePage},
    preferences::PreferencesEngine,
    secrets,
};

use crate::{
    IceonApp,
    resource_scroll::ResourceScrollScope,
    resource_table::{ResourceTable, TableData, TableStats},
    terminal_surface::{self, TerminalSurface},
};

const ROWS: usize = 100;
const TITLE: &str = "Real resource table performance (offline)";
static DIRECTORY_NONCE: AtomicUsize = AtomicUsize::new(0);
type Outcome = Rc<RefCell<Option<Result<(), String>>>>;
type FrameTiming = Rc<Cell<Option<Duration>>>;

/// A fresh directory is essential: never restore a user's routes or discovery paths.
struct FixtureDirectory(PathBuf);
impl FixtureDirectory {
    fn new() -> Self {
        loop {
            let path = std::env::temp_dir().join(format!(
                "iceon-render-probe-{}-{}",
                std::process::id(),
                DIRECTORY_NONCE.fetch_add(1, Ordering::Relaxed),
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot create offline probe directory: {error}"),
            }
        }
    }
    fn cleanup(&self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("render_performance_probe: fixture cleanup: {error}");
        }
    }
}
impl Drop for FixtureDirectory {
    fn drop(&mut self) {
        self.cleanup();
    }
}

fn fixture_page() -> Rc<ResourcePage> {
    Rc::new(ResourcePage {
        cluster_id: "offline-render-probe".into(),
        session_id: "fixture-only".into(),
        kind: ResourceKind::Pods,
        read_id: 1,
        items: (0..ROWS)
            .map(|index| ResourceItem {
                name: format!("offline-pod-{index:03}"),
                namespace: Some("fixture-namespace".into()),
                status: "Running".into(),
                uid: Some(format!("fixture-uid-{index:03}")),
                resource_version: Some("1".into()),
                created_at: None,
                columns: [("image".into(), format!("offline/image-{index:03}:fixture"))].into(),
                node: None,
                containers: Vec::new(),
            })
            .collect(),
        truncated: false,
        fetched_at: 0,
        namespace: None,
        continue_token: None,
        metrics: None,
    })
}

struct ProbePanel {
    table: Option<Entity<ResourceTable>>,
    terminal: Entity<TerminalSurface>,
    focus: FocusHandle,
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
}
impl EventEmitter<PanelEvent> for ProbePanel {}
impl Focusable for ProbePanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl BasePanel for ProbePanel {
    fn panel_name(&self) -> &'static str {
        if self.table.is_some() {
            "iceon.render-probe.table"
        } else {
            "iceon.render-probe.terminal"
        }
    }
    fn closable(&self, _: &App) -> bool {
        false
    }
}
impl Panel for ProbePanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        if self.table.is_some() {
            "100 real resource rows"
        } else {
            "Fixture terminal"
        }
    }
    fn title_bar(&self, _: &App) -> bool {
        false
    }
    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}
impl Render for ProbePanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut body = div()
            .relative()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden();
        if let Some(table) = &self.table {
            let bounds = self.bounds.clone();
            body = body
                .child(table.clone().cached(StyleRefinement::default().size_full()))
                .child(
                    canvas(move |rect, _, _| bounds.set(Some(rect)), |_, _, _, _| {})
                        .absolute()
                        .inset_0(),
                );
        } else {
            body = body.child(self.terminal.clone());
        }
        body
    }
}

struct Probe {
    area: Entity<DockArea>,
    _skin: Rc<DockSkin>,
    _owner: Entity<IceonApp>,
    _panels: [Entity<ProbePanel>; 2],
    table: Entity<ResourceTable>,
    terminal: Entity<TerminalSurface>,
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    baseline: TableStats,
    terminal_baseline: (usize, usize),
    idle_baseline: TableStats,
    vertical_timing: FrameTiming,
    horizontal_timing: FrameTiming,
    outcome: Outcome,
    directory: Rc<FixtureDirectory>,
    armed: bool,
}
impl Probe {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        outcome: Outcome,
        directory: Rc<FixtureDirectory>,
    ) -> Self {
        let catalog = Arc::new(CatalogEngine::new_with_home(directory.0.clone(), None));
        let connection = Arc::new(ConnectionEngine::new(catalog.clone()));
        let preferences =
            PreferencesEngine::new(directory.0.clone()).expect("fresh offline preferences store");
        let secrets = secrets::Engine::new(directory.0.clone());
        // Fresh defaults have no restored route or configured discovery path.
        // IceonApp's harmless catalog timer may run, but has no home to discover.
        let owner =
            cx.new(|cx| IceonApp::new(catalog, connection, preferences, secrets, window, cx));
        let table = cx.new(|cx| ResourceTable::new(owner.downgrade(), cx));
        table.update(cx, |table, cx| {
            table.sync(
                TableData {
                    page: Some(fixture_page()),
                    scope: ResourceScrollScope {
                        cluster_id: "offline-render-probe".into(),
                        kind: ResourceKind::Pods,
                        namespace: None,
                        query: String::new(),
                        status: None,
                        page_token: None,
                    },
                },
                cx,
            )
        });
        let terminal = cx.new(|cx| TerminalSurface::new(window, cx));
        terminal.update(cx, |terminal, cx| {
            terminal.feed("offline fixture terminal\r\n$ ", cx)
        });
        let bounds = Rc::new(Cell::new(None));
        let panels = [Some(table.clone()), None].map(|table| {
            cx.new(|cx| ProbePanel {
                table,
                terminal: terminal.clone(),
                focus: cx.focus_handle(),
                bounds: bounds.clone(),
            })
        });
        let (area, skin) = DockSkin::dock_area("iceon-real-render-performance", None, window, cx);
        area.update(cx, |area, cx| {
            area.set_center(
                DockLayout::tabs().panel_view(panel_handle(panels[0].clone()), cx),
                window,
                cx,
            );
            area.set_dock(
                DockPlacement::Bottom,
                DockLayout::tabs().panel_view(panel_handle(panels[1].clone()), cx),
                window,
                cx,
            );
            area.set_dock_size(DockPlacement::Bottom, px(160.), window, cx);
        });
        Self {
            area,
            _skin: skin,
            _owner: owner,
            _panels: panels,
            table,
            terminal,
            bounds,
            baseline: TableStats::default(),
            terminal_baseline: (0, 0),
            idle_baseline: TableStats::default(),
            vertical_timing: Rc::new(Cell::new(None)),
            horizontal_timing: Rc::new(Cell::new(None)),
            outcome,
            directory,
            armed: false,
        }
    }

    fn check(&mut self, stage: usize, cx: &mut Context<Self>) -> Result<(), String> {
        let stats = self.table.read(cx).stats();
        let terminal = self.terminal.read(cx).performance_snapshot();
        let require = |ok: bool, message: &str| {
            if ok {
                Ok(())
            } else {
                Err(format!(
                    "stage {stage}: {message}; table={stats:?}, terminal={terminal:?}"
                ))
            }
        };
        require(stats.filter_builds == 1, "resource filter was rebuilt")?;
        require(stats.row_size_builds == 1, "virtual row sizes were rebuilt")?;
        match stage {
            0 => {
                require(
                    stats.renders > 0 && stats.row_renders > 0,
                    "real table did not render",
                )?;
                require(
                    stats.row_renders < ROWS,
                    "initial virtual layout rendered all 100 rows",
                )?;
                let rect = self
                    .bounds
                    .get()
                    .ok_or("table bounds were never measured")?;
                require(
                    rect.size.width > px(0.)
                        && rect.size.width <= px(600.)
                        && rect.size.height > px(70.),
                    "invalid real table viewport",
                )?;
                require(
                    terminal.0 > 0 && terminal.1 > 0,
                    "terminal grid never rendered",
                )?;
                self.baseline = stats;
                println!(
                    "render_performance_probe initial: viewport={rect:?}, table={stats:?}, terminal={terminal:?}"
                );
            }
            16 => {
                self.table.update(cx, |table, cx| {
                    table.sort_by_key("name", cx);
                    let ascending = table.visible_order();
                    assert_eq!(
                        ascending.first().map(String::as_str),
                        Some("offline-pod-000")
                    );
                    table.sort_by_key("name", cx);
                    assert_eq!(
                        table.visible_order().first().map(String::as_str),
                        Some("offline-pod-099")
                    );
                    table.sort_by_key("name", cx);
                    assert_eq!(table.visible_order(), ascending);
                    table.resize_column("namespace", 1., cx);
                    let widths = table.column_widths();
                    let namespace = widths
                        .iter()
                        .find(|column| column.0 == "namespace")
                        .unwrap();
                    assert_eq!(namespace.1, namespace.2);
                    table.resize_column("namespace", 256., cx);
                    assert_eq!(
                        table
                            .column_widths()
                            .iter()
                            .find(|column| column.0 == "namespace")
                            .unwrap()
                            .1,
                        256.
                    );
                    assert!(
                        table
                            .column_widths()
                            .iter()
                            .all(|column| column.1 >= column.2)
                    );
                    assert_eq!(table.stats().filter_builds, 1);
                    assert_eq!(table.stats().row_size_builds, 1);
                });
            }
            17 => {
                let selected = self._owner.read(cx).selected_namespace.clone();
                require(
                    selected.as_deref() == Some("fixture-namespace"),
                    "namespace cell did not set filter scope",
                )?;
                require(
                    self._owner.read(cx).current_continue_token.is_none(),
                    "namespace filter retained stale pagination",
                )?;
                println!(
                    "PASS table interactions: sort cycle, minimum widths, resize, namespace filtering; caches retained"
                );
            }
            20 => {}
            21 => {
                let owner = self._owner.read(cx);
                require(
                    owner.inspector_open,
                    "native row click did not open inspector",
                )?;
                require(
                    matches!(&owner.inspector.target,
                        Some(crate::inspector_view::InspectorTarget::Resource(reference))
                            if reference.name.starts_with("offline-pod-")
                                && reference.namespace.as_deref() == Some("fixture-namespace")
                                && reference.uid.as_deref().is_some_and(|uid| uid.starts_with("fixture-uid-"))),
                    "native row click did not retain fixture identity",
                )?;
            }
            22 => {
                let owner = self._owner.read(cx);
                require(
                    owner.selected_namespace.as_deref() == Some("fixture-namespace"),
                    "native namespace click did not set scope",
                )?;
                require(
                    owner.current_continue_token.is_none(),
                    "native namespace click retained stale pagination",
                )?;
                require(
                    !owner.inspector_open && owner.inspector.target.is_none(),
                    "namespace click bubbled into row inspector",
                )?;
                println!(
                    "PASS native row and namespace clicks: fixture identity, scope, pagination reset, no row propagation"
                );
            }
            18 => {}
            19 => {
                let widths = self.table.read(cx).column_widths();
                let width = widths
                    .iter()
                    .find(|column| column.0 == "namespace")
                    .unwrap()
                    .1;
                require(
                    width > 280.,
                    &format!("native separator drag did not resize Namespace: {width}"),
                )?;
                require(
                    stats.filter_builds == 1 && stats.row_size_builds == 1,
                    "native drag rebuilt filter or row sizes",
                )?;
                println!("PASS native Namespace separator drag: width={width}; caches retained");
            }
            1 => {
                let (vertical, _) = self.table.read(cx).scroll_offsets();
                require(
                    vertical.y < px(0.),
                    "vertical wheel did not move the actual table body",
                )?;
                require(
                    stats.row_renders > self.baseline.row_renders,
                    "vertical scroll did not render new virtual rows",
                )?;
                println!(
                    "render_performance_probe vertical: offset={vertical:?}, table={stats:?}, event-to-draw={:?}",
                    self.vertical_timing.get()
                );
            }
            2 => {
                let (vertical, horizontal) = self.table.read(cx).scroll_offsets();
                require(
                    vertical.y < px(0.),
                    "horizontal scroll reset the vertical offset",
                )?;
                require(
                    horizontal.x < px(0.),
                    "horizontal wheel did not move overflow columns",
                )?;
                println!(
                    "render_performance_probe horizontal: offset={horizontal:?}, table={stats:?}, event-to-draw={:?}",
                    self.horizontal_timing.get()
                );
                self.baseline = stats;
                self.terminal_baseline = terminal;
            }
            3 => {
                require(
                    stats.renders == self.baseline.renders,
                    "terminal-only output rerendered ResourceTable",
                )?;
                require(
                    stats.row_renders == self.baseline.row_renders,
                    "terminal-only output reran resource row callbacks",
                )?;
                require(
                    terminal.1 > self.terminal_baseline.1,
                    "terminal burst did not rebuild the real grid",
                )?;
                require(
                    terminal.1 - self.terminal_baseline.1 <= 2,
                    "40 coalesced chunks rebuilt the grid repeatedly",
                )?;
                self.terminal_baseline = terminal;
                println!(
                    "render_performance_probe burst: chunks=40, table={stats:?}, terminal={terminal:?}"
                );
            }
            4..=13 => {
                require(
                    terminal.0 > self.terminal_baseline.0,
                    "requested refresh did not render the terminal",
                )?;
                require(
                    terminal.1 == self.terminal_baseline.1,
                    "redraw extracted the full unchanged terminal grid",
                )?;
                self.terminal_baseline = terminal;
            }
            14 => {
                require(
                    terminal.1 == self.terminal_baseline.1,
                    "final refresh rebuilt unchanged terminal cells",
                )?;
                self.idle_baseline = stats;
                self.terminal_baseline = terminal;
            }
            15 => {
                require(stats == self.idle_baseline, "idle table generated work")?;
                require(
                    terminal == self.terminal_baseline,
                    "idle terminal generated work",
                )?;
                require(
                    self.vertical_timing.get().is_some() && self.horizontal_timing.get().is_some(),
                    "scroll draws were not observed",
                )?;
                println!(
                    "PASS render_performance_probe: actual ResourceTable, 100 fixtures; vertical/horizontal wheel; filter_builds=1 row_size_builds=1; terminal-only burst leaves table cached; 10 refreshes reuse terminal grid; idle stable. final table={stats:?}, terminal={terminal:?}. Timings are native CPU event-to-draw latency, not FPS or GPU completion."
                );
            }
            _ => unreachable!(),
        }
        Ok(())
    }
}

/// Dispatch outside Probe's entity-map borrow: native handlers can update entities.
fn scroll(
    window: &mut Window,
    cx: &mut App,
    rect: Bounds<Pixels>,
    delta: Point<Pixels>,
    timing: FrameTiming,
) {
    let start = Instant::now();
    // The table header is ~42 px tall; target the measured body below it.
    let position = rect.origin
        + point(
            rect.size.width / 2.,
            px(55.) + (rect.size.height - px(55.)) / 2.,
        );
    window.dispatch_event(
        PlatformInput::Scroll(ScrollEvent {
            position,
            delta: ScrollDelta::Pixels(delta),
            ..Default::default()
        }),
        cx,
    );
    // The wheel hit-test can arm row/column tooltips and scrollbar hover
    // transitions. Move out of the measured table before taking its next
    // baseline, so their delayed appearance is not mistaken for terminal work.
    window.dispatch_event(
        PlatformInput::MouseMove(MouseMoveEvent {
            position: point(px(-10.), px(-10.)),
            ..Default::default()
        }),
        cx,
    );
    println!(
        "render_performance_probe wheel dispatch={:?}",
        start.elapsed()
    );
    // A hidden/occluded native window has no display-link frames. Draw after
    // dispatch notifications flush, not inside the event's entity update.
    window.defer(cx, move |window, cx| {
        window.draw(cx).clear(cx);
        timing.set(Some(start.elapsed()));
    });
}

fn click(window: &mut Window, cx: &mut App, position: Point<Pixels>) {
    window.dispatch_event(
        PlatformInput::MouseMove(MouseMoveEvent {
            position,
            ..Default::default()
        }),
        cx,
    );
    window.dispatch_event(
        PlatformInput::MouseDown(MouseDownEvent {
            position,
            button: MouseButton::Left,
            click_count: 1,
            ..Default::default()
        }),
        cx,
    );
    window.dispatch_event(
        PlatformInput::MouseUp(MouseUpEvent {
            position,
            button: MouseButton::Left,
            click_count: 1,
            ..Default::default()
        }),
        cx,
    );
}

impl Render for Probe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.armed {
            self.armed = true;
            let weak = cx.entity().downgrade();
            window.on_next_frame(move |window, cx| {
                let _ = weak.update(cx, |_, cx| {
                    cx.spawn_in(window, async move |this, cx| {
                        for stage in 0..=22 {
                            cx.background_executor().timer(Duration::from_millis(220)).await;
                            let result = cx.update(|window, cx| {
                                let checked = this.update(cx, |probe, cx| {
                                    probe.check(stage, cx)?;
                                    Ok::<_, String>((probe.bounds.get(), probe.terminal.clone(), probe.vertical_timing.clone(), probe.horizontal_timing.clone()))
                                });
                                match checked {
                                    Ok(Ok((bounds, terminal, vertical, horizontal))) => {
                                        match stage {
                                            0 => scroll(window, cx, bounds.unwrap(), point(px(0.), px(-320.)), vertical),
                                            1 => scroll(window, cx, bounds.unwrap(), point(px(-200.), px(0.)), horizontal),
                                            2 => terminal.update(cx, |terminal, cx| {
                                                let start = Instant::now();
                                                for index in 0..40 {
                                                    terminal.feed(&format!("\r\nfixture chunk {index:02}: output only"), cx);
                                                }
                                                terminal.feed("\r\n$ ", cx);
                                                println!("render_performance_probe 40-chunk parser feed={:?}", start.elapsed());
                                            }),
                                            3..=12 => window.refresh(),
                                            16 => {
                                                let _ = this.update(cx, |probe, cx| {
                                                    probe._owner.update(cx, |app, cx| {
                                                        app.current_continue_token = Some("stale-fixture-token".into());
                                                        ResourceTable::filter_namespace(app, "fixture-namespace", window, cx);
                                                    });
                                                });
                                            }
                                            17 => scroll(window, cx, bounds.unwrap(), point(px(1000.), px(0.)), horizontal),
                                            18 => {
                                                let table = this.read_with(cx, |probe, _| probe.table.clone()).unwrap();
                                                let rect = table.read(cx).column_header_bounds("namespace").expect("Namespace header rendered");
                                                let origin = point(rect.right() - px(2.), rect.top() + px(20.));
                                                window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent {
                                                    position: origin, ..Default::default()
                                                }), cx);
                                                window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent {
                                                    position: origin, button: MouseButton::Left,
                                                    click_count: 1, ..Default::default()
                                                }), cx);
                                                for delta in [8., 24., 48., 64.] {
                                                    window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent {
                                                        position: origin + point(px(delta), px(0.)),
                                                        pressed_button: Some(MouseButton::Left), ..Default::default()
                                                    }), cx);
                                                    window.draw(cx).clear(cx);
                                                }
                                                window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent {
                                                    position: origin + point(px(64.), px(0.)),
                                                    button: MouseButton::Left, click_count: 1, ..Default::default()
                                                }), cx);
                                            }
                                            20 => {
                                                click(window, cx, bounds.unwrap().origin + point(px(30.), px(70.)));
                                            }
                                            21 => {
                                                let _ = this.update(cx, |probe, cx| {
                                                    probe._owner.update(cx, |owner, _| {
                                                        owner.inspector_open = false;
                                                        owner.inspector.target = None;
                                                        owner.selected_namespace = None;
                                                        owner.current_continue_token = Some("stale-native-click-token".into());
                                                    });
                                                });
                                                let table = this.read_with(cx, |probe, _| probe.table.clone()).unwrap();
                                                let header = table.read(cx).column_header_bounds("namespace").expect("Namespace header rendered");
                                                // Vertical scroll leaves the first row partially clipped;
                                                // target the next full row, inside its namespace button.
                                                click(window, cx, point(header.left() + px(65.), header.bottom() + px(37.)));
                                            }
                                            _ => {}
                                        }
                                        // Drive one native CPU draw even when macOS reports Hidden
                                        // (e.g. no screen/occluded automation). Defer until cx.notify
                                        // effects invalidate cached views; never force refresh here:
                                        // stage 2 must prove terminal-only invalidation preserves table.
                                        // Stages 13..15 deliberately schedule no work for idle checks.
                                        if !matches!(stage, 0 | 1 | 13..=15 | 22) {
                                            window.defer(cx, |window, cx| window.draw(cx).clear(cx));
                                        }
                                        if stage == 22 {
                                            let _ = this.update(cx, |probe, _| {
                                                *probe.outcome.borrow_mut() = Some(Ok(()));
                                                probe.directory.cleanup();
                                            });
                                            cx.quit();
                                        }
                                        true
                                    }
                                    other => {
                                        let error = match other {
                                            Ok(Err(error)) => error,
                                            Err(error) => format!("probe entity unavailable: {error}"),
                                            _ => unreachable!(),
                                        };
                                        eprintln!("FAIL render_performance_probe: {error}");
                                        let _ = this.update(cx, |probe, _| {
                                            *probe.outcome.borrow_mut() = Some(Err(error));
                                            probe.directory.cleanup();
                                        });
                                        cx.quit();
                                        false
                                    }
                                }
                            });
                            if !matches!(result, Ok(true)) { break; }
                        }
                    }).detach();
                });
            });
        }
        div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(self.area.clone())
    }
}

pub fn run() {
    let directory = Rc::new(FixtureDirectory::new());
    let cleanup = directory.clone();
    let outcome: Outcome = Rc::new(RefCell::new(None));
    let result = outcome.clone();
    gpui_kit::application().with_assets(crate::app_assets::AppAssets).run(move |cx| {
        gpui_kit::init(cx);
        // This audit measures table/terminal invalidation, not scrollbar fades.
        // Scrolling mode schedules parent-view notifications (enter/exit and
        // the two-second idle hold), which can land in stage 3 or stage 15
        // depending on native display-link timing. Keep the real scrollbars
        // and wheel paths, but remove that independent animation producer.
        gpui_kit::component::Theme::set_scrollbar_mode(
            gpui_kit::component::scroll::ScrollbarMode::Always,
            cx,
        );
        crate::console_font::init(cx);
        terminal_surface::init(cx);
        let watchdog = result.clone();
        let watchdog_directory = directory.clone();
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_secs(10)).await;
            cx.update(|cx| {
                if watchdog.borrow().is_none() {
                    eprintln!("FAIL render_performance_probe: timed out before native checks completed");
                    *watchdog.borrow_mut() = Some(Err("native probe timed out".into()));
                }
                watchdog_directory.cleanup();
                cx.quit();
            });
        }).detach();
        cx.on_window_closed(|cx, _| { if cx.windows().is_empty() { cx.quit(); } }).detach();
        gpui_kit::open_window(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(600.), px(500.)), cx))),
            ..Default::default()
        }, cx, move |window, cx| {
            window.set_window_title(TITLE);
            cx.new(|cx| Probe::new(window, cx, result, directory))
        }).expect("failed to open native resource table performance probe");
        cx.activate(true);
    });
    cleanup.cleanup();
    match outcome.borrow_mut().take() {
        Some(Ok(())) => {}
        Some(Err(error)) => panic!("FAIL render_performance_probe: {error}"),
        None => panic!("FAIL render_performance_probe: window closed before checks completed"),
    }
}
