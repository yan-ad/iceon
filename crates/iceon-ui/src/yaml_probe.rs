//! Native integration regression for production YAML tabs and bottom console.
//! Uses fresh stores, a fabricated UI lease, and a disconnected engine. No kubeconfig,
//! real cluster, PTY, or shell is opened. NotConnected reads are expected; editor
//! buffers are set through the production EditorState (no private tab injection).
use crate::{IceonApp, terminal_surface};
use gpui_kit::{
    component::{WindowExt as _, dock::DockPlacement},
    prelude::*,
    *,
};
use iceon_core::{
    catalog::CatalogEngine,
    connection::{
        ConnectionEngine, ConnectionSnapshot, ResourceKind, details::ResourceReference,
        pod_runtime::PodStreamMode,
    },
    preferences::PreferencesEngine,
    secrets,
};
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc, time::Duration};

type Outcome = Rc<RefCell<Option<Result<(), String>>>>;
const FIRST: &str =
    "apiVersion: v1\nkind: Pod\nmetadata:\n  name: offline-one\n# unsaved first buffer\n";
const SECOND: &str = "kind: Pod\n# independent second buffer\n";
struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn reference(name: &str) -> ResourceReference {
    ResourceReference {
        kind: ResourceKind::Pods,
        namespace: Some("fixture".into()),
        name: name.into(),
        uid: Some(format!("uid-{name}")),
    }
}
struct Probe {
    owner: Entity<IceonApp>,
    area: Entity<gpui_kit::component::dock::DockArea>,
    _directory: Rc<Directory>,
    outcome: Outcome,
    stage: usize,
    first: Option<(u64, Entity<gpui_kit::component::input::EditorState>)>,
    second: Option<u64>,
    armed: bool,
}
impl Probe {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        directory: Rc<Directory>,
        outcome: Outcome,
    ) -> Self {
        let catalog = Arc::new(CatalogEngine::new_with_home(directory.0.clone(), None));
        let connection = Arc::new(ConnectionEngine::new(catalog.clone()));
        let preferences = PreferencesEngine::new(directory.0.clone()).expect("offline preferences");
        let secrets = secrets::Engine::new(directory.0.clone());
        let owner =
            cx.new(|cx| IceonApp::new(catalog, connection, preferences, secrets, window, cx));
        let dock = crate::workspace_dock::create(owner.downgrade(), window, cx);
        let area = dock.dock_area.clone();
        owner.update(cx, |app, cx| {
            app.cluster_session = Some(ConnectionSnapshot {
                cluster_id: "yaml-probe".into(),
                session_id: "disconnected-fixture".into(),
                context_name: "offline".into(),
                status: "connected",
                api_version: None,
                error: None,
            });
            app.workspace_dock = Some(dock);
            app.open_yaml_editor(reference("one"), window, cx);
        });
        Self {
            owner,
            area,
            _directory: directory,
            outcome,
            stage: 0,
            first: None,
            second: None,
            armed: false,
        }
    }
    fn check(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<bool, String> {
        macro_rules! ensure {
            ($condition:expr, $message:literal) => {
                if !$condition {
                    return Err($message.into());
                }
            };
        }
        let stage = self.stage;
        let first = &mut self.first;
        let second = &mut self.second;
        self.owner.update(cx, |app, cx| {
            match stage {
                0 => {
                    ensure!(
                        app.pod_dock_open && app.yaml_dock_tabs.len() == 1,
                        "open_yaml_editor did not open one bottom YAML tab"
                    );
                    ensure!(
                        app.workspace_dock
                            .as_ref()
                            .unwrap()
                            .dock_area
                            .read(cx)
                            .is_dock_open(DockPlacement::Bottom),
                        "native bottom dock is closed"
                    );
                    let tab = &app.yaml_dock_tabs[0];
                    ensure!(
                        app.active_yaml_tab == Some(tab.key),
                        "new YAML tab is not selected"
                    );
                    ensure!(
                        tab.editor.focus_handle(cx).is_focused(window),
                        "new YAML editor is not focused"
                    );
                    *first = Some((tab.key, tab.editor.clone()));
                    tab.editor
                        .update(cx, |editor, cx| editor.set_value(FIRST, window, cx));
                    app.open_yaml_editor(reference("one"), window, cx);
                    ensure!(
                        app.yaml_dock_tabs.len() == 1,
                        "same resource duplicated a YAML tab"
                    );
                    ensure!(
                        app.yaml_dock_tabs[0].editor == first.as_ref().unwrap().1,
                        "same resource replaced editor identity"
                    );
                    ensure!(
                        app.yaml_dock_tabs[0].editor.read(cx).value().as_str() == FIRST,
                        "reopening resource discarded unsaved buffer"
                    );
                    app.open_yaml_editor(reference("two"), window, cx);
                }
                1 => {
                    ensure!(
                        app.yaml_dock_tabs.len() == 2,
                        "distinct resource did not get separate tab"
                    );
                    let tab = &app.yaml_dock_tabs[1];
                    ensure!(
                        tab.editor != first.as_ref().unwrap().1,
                        "distinct resources share an editor"
                    );
                    ensure!(
                        tab.editor.focus_handle(cx).is_focused(window),
                        "second YAML editor is not focused"
                    );
                    *second = Some(tab.key);
                    tab.editor
                        .update(cx, |editor, cx| editor.set_value(SECOND, window, cx));
                    app.select_yaml_tab(first.as_ref().unwrap().0, window, cx);
                }
                2 => {
                    ensure!(
                        app.active_yaml_tab == Some(first.as_ref().unwrap().0),
                        "YAML selection did not switch"
                    );
                    ensure!(
                        first
                            .as_ref()
                            .unwrap()
                            .1
                            .focus_handle(cx)
                            .is_focused(window),
                        "selection switch did not focus first editor"
                    );
                    ensure!(
                        first.as_ref().unwrap().1.read(cx).value().as_str() == FIRST,
                        "selection switch lost first buffer"
                    );
                    let counter = app.operation_counter;
                    app.request_close_yaml_tab(first.as_ref().unwrap().0, window, cx);
                    ensure!(
                        window.has_active_dialog(cx),
                        "dirty YAML close did not request confirmation"
                    );
                    ensure!(
                        app.yaml_dock_tabs.len() == 2,
                        "dirty close removed tab before confirmation"
                    );
                    ensure!(
                        app.operation_counter == counter,
                        "dirty close started a backend operation"
                    );
                    window.close_dialog(cx); // cancel: no discard/apply callback
                    app.open_pod_stream(
                        "fixture".into(),
                        "offline-terminal".into(),
                        "uid-terminal".into(),
                        "main".into(),
                        PodStreamMode::Shell,
                        window,
                        cx,
                    );
                }
                3 => {
                    ensure!(
                        app.active_yaml_tab.is_none() && !app.pod_dock_tabs.is_empty(),
                        "terminal did not become active"
                    );
                    ensure!(
                        app.yaml_dock_tabs.len() == 2,
                        "terminal switch removed YAML buffers"
                    );
                    ensure!(
                        first.as_ref().unwrap().1.read(cx).value().as_str() == FIRST,
                        "terminal switch lost first YAML buffer"
                    );
                    let tab = app
                        .yaml_dock_tabs
                        .iter()
                        .find(|t| Some(t.key) == *second)
                        .ok_or("second tab disappeared")?;
                    ensure!(
                        tab.editor.read(cx).value().as_str() == SECOND,
                        "terminal switch lost second YAML buffer"
                    );
                    app.select_yaml_tab(second.unwrap(), window, cx);
                }
                4 => {
                    let tab = app
                        .yaml_dock_tabs
                        .iter()
                        .find(|t| Some(t.key) == *second)
                        .ok_or("second tab disappeared")?;
                    ensure!(
                        app.active_yaml_tab == *second
                            && tab.editor.focus_handle(cx).is_focused(window),
                        "return from terminal did not select/focus YAML"
                    );
                    ensure!(
                        tab.editor.read(cx).value().as_str() == SECOND,
                        "return from terminal changed buffer"
                    );
                    ensure!(
                        !window.has_active_dialog(cx),
                        "cancelled confirmation remained open"
                    );
                    return Ok(true);
                }
                _ => return Err("unexpected native probe stage".into()),
            }
            Ok(false)
        })
    }
}
impl Render for Probe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.owner.update(cx, |app, cx| {
            if let Some(dock) = &mut app.workspace_dock {
                dock.sync(app.pod_dock_open, window, cx);
            }
        });
        if !self.armed {
            self.armed = true;
            let owner = cx.entity().downgrade();
            window.on_next_frame(move |window, cx| {
                let _ = owner.update(cx, |_, cx| {
                cx.spawn_in(window, async move |owner, cx| {
                    for _ in 0..5 {
                        cx.background_executor().timer(Duration::from_millis(250)).await;
                        let result = owner.update_in(cx, |probe, window, cx| {
                            let result = probe.check(window, cx);
                            probe.stage += 1;
                            cx.notify();
                            result
                        });
                        let result = result.unwrap_or_else(|e| Err(e.to_string()));
                        if !matches!(result, Ok(false)) {
                            let _ = owner.update_in(cx, |probe, _, cx| {
                                match &result { Ok(true) => println!("PASS yaml_probe: native bottom dock, editor focus, resource reuse/identity, independent buffers, selection, dirty-close confirmation/cancel without backend operation, terminal round-trip"), Err(e) => eprintln!("FAIL yaml_probe stage {}: {e}", probe.stage - 1), _ => {} }
                                *probe.outcome.borrow_mut() = Some(result.map(|_| ()));
                                cx.quit();
                            });
                            break;
                        }
                    }
                }).detach();
                });
            });
        }
        div().size_full().child(self.area.clone())
    }
}
pub fn run() {
    let path = std::env::temp_dir().join(format!(
        "iceon-yaml-probe-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&path).expect("fresh offline fixture directory");
    let directory = Rc::new(Directory(path));
    let outcome: Outcome = Rc::new(RefCell::new(None));
    let result = outcome.clone();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            crate::console_font::init(cx);
            terminal_surface::init(cx);
            let watchdog = result.clone();
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_secs(10))
                    .await;
                cx.update(|cx| {
                    if watchdog.borrow().is_none() {
                        eprintln!("FAIL yaml_probe: timed out");
                        *watchdog.borrow_mut() = Some(Err("timed out".into()));
                    }
                    cx.quit();
                });
            })
            .detach();
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1000.), px(700.)),
                        cx,
                    ))),
                    ..Default::default()
                },
                cx,
                move |window, cx| cx.new(|cx| Probe::new(window, cx, directory, result)),
            )
            .expect("native YAML probe window");
            cx.activate(true);
        });
    // GPUI can swallow panics in native callbacks. The authoritative result is
    // this marker (and a panic outside the event loop), never exit status alone.
    match outcome.borrow_mut().take() {
        Some(Ok(())) => {}
        Some(Err(e)) => panic!("FAIL yaml_probe: {e}"),
        None => panic!("FAIL yaml_probe: window closed before completion"),
    }
}
