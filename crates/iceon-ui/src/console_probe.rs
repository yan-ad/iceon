//! Isolated native rendering/input smoke probe. Deliberately does not construct
//! IceonApp, load preferences/kubeconfig, initialize TLS, or start any engine.
//!
//! The library exposes this module only under `cfg(debug_assertions)`. The probe
//! drives the real TerminalSurface and standard Dock, not source-inclusion stubs.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use gpui_kit::component::dock::{
    BasePanel, DockArea, DockLayout, DockPlacement, DockSkin, Panel, PanelControl, PanelEvent,
    PanelId, panel_handle,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::terminal_surface::{self, TerminalEvent, TerminalSurface};

const TITLE: &str = "Pod console preview (offline)";
const LOG_COUNT: usize = 100;
const FIXTURE: &str = concat!(
    "\x1b[2J\x1b[H",
    "\x1b[1;32moffline:/ $\x1b[0m\r\n",
    "progress 100% stale\r\x1b[2Kprogress 25%\x1b[K\r\n",
    "cursor: old\x1b[3D\x1b[K\x1b[38;2;80;170;255mready\x1b[0m\r\n",
    "$ "
);
const EXPECTED: &str = "offline:/ $\nprogress 25%\ncursor: ready\n$ ";
const KEYS: [&str; 7] = ["x", "ctrl-c", "enter", "up", "down", "left", "right"];
const INPUT: [&str; 7] = ["x", "\x03", "\r", "\x1b[A", "\x1b[B", "\x1b[D", "\x1b[C"];

type Requests = Rc<RefCell<Vec<std::ops::Range<usize>>>>;

struct ProbePanel {
    terminal: Option<Entity<TerminalSurface>>,
    focus: FocusHandle,
    logs: Rc<Vec<SharedString>>,
    requests: Requests,
    log_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    log_scroll: UniformListScrollHandle,
    renders: Rc<Cell<usize>>,
}

impl EventEmitter<PanelEvent> for ProbePanel {}
impl Focusable for ProbePanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl BasePanel for ProbePanel {
    fn panel_name(&self) -> &'static str {
        if self.terminal.is_some() {
            "iceon.probe.shell"
        } else {
            "iceon.probe.logs"
        }
    }
    fn closable(&self, _: &App) -> bool {
        false
    }
}
impl Panel for ProbePanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        if self.terminal.is_some() {
            "Shell"
        } else {
            "Logs (100 offline fixtures)"
        }
    }
    fn tab_name(&self, _: &App) -> Option<SharedString> {
        Some(
            if self.terminal.is_some() {
                "Shell"
            } else {
                "Logs"
            }
            .into(),
        )
    }
    fn inner_padding(&self, _: &App) -> bool {
        false
    }
    fn title_bar(&self, _: &App) -> bool {
        self.terminal.is_none()
    }
    fn zoom_control(&self, _: &App) -> Option<PanelControl> {
        Some(PanelControl::Toolbar)
    }
}
impl Render for ProbePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        if let Some(terminal) = &self.terminal {
            return div()
                .size_full()
                .min_w_0()
                .min_h_0()
                .overflow_hidden()
                .child(terminal.clone())
                .into_any_element();
        }
        let logs = self.logs.clone();
        let requests = self.requests.clone();
        let bounds = self.log_bounds.clone();
        div()
            .relative()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .bg(rgb(0x101820))
            .child(
                uniform_list(
                    "probe-log-output",
                    logs.len(),
                    cx.processor(move |_, range: std::ops::Range<usize>, _, _| {
                        requests.borrow_mut().push(range.clone());
                        range
                            .map(|index| {
                                div()
                                    .h(px(21.))
                                    .px_2()
                                    .font_family("monospace")
                                    .text_size(px(14.))
                                    .line_height(px(21.))
                                    .text_color(rgb(0xcbd5e1))
                                    .whitespace_nowrap()
                                    .child(gpui_kit::base::SelectableText::new(
                                        ("probe-log-row", index),
                                        logs[index].clone(),
                                    ))
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&self.log_scroll)
                .size_full(),
            )
            // Measure the actual log body, not guessed titlebar/tab coordinates.
            .child(
                canvas(move |rect, _, _| bounds.set(Some(rect)), |_, _, _, _| {})
                    .absolute()
                    .inset_0(),
            )
            .into_any_element()
    }
}

struct ConsoleProbe {
    area: Entity<DockArea>,
    _skin: Rc<DockSkin>,
    terminal: Entity<TerminalSurface>,
    _panels: [Entity<ProbePanel>; 2],
    _input_subscription: Subscription,
    inputs: Rc<RefCell<Vec<String>>>,
    resizes: Rc<RefCell<Vec<(u16, u16)>>>,
    requests: Requests,
    log_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    grids: Vec<(u16, u16)>,
    panel_renders: Rc<Cell<usize>>,
    idle_baseline: usize,
    armed: bool,
    outcome: Rc<RefCell<Option<Result<(), String>>>>,
}

impl ConsoleProbe {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        outcome: Rc<RefCell<Option<Result<(), String>>>>,
    ) -> Self {
        let terminal = cx.new(|cx| TerminalSurface::new(window, cx));
        // Feed BEFORE the first native layout: the subsequent measured resize
        // must preserve the parser's cells and cursor, not append escape text.
        terminal.update(cx, |surface, cx| {
            surface.reset(cx);
            surface.feed(FIXTURE, cx);
        });
        let inputs = Rc::new(RefCell::new(Vec::new()));
        let resizes = Rc::new(RefCell::new(Vec::new()));
        let captured = inputs.clone();
        let measured = resizes.clone();
        let subscription = cx.subscribe(&terminal, move |_, _, event: &TerminalEvent, _| {
            match event {
                TerminalEvent::Input(input) => captured.borrow_mut().push(input.clone()),
                TerminalEvent::Resize { cols, rows } => measured.borrow_mut().push((*cols, *rows)),
            }
            // No automatic echo! Only the timed probe's explicitly marked
            // fixture response below may feed output back into the surface.
        });
        let logs = Rc::new((0..LOG_COUNT).map(|index| {
            format!("2026-11-10T12:00:{:02}Z INFO offline fixture {index:03}: no cluster connected", index % 60).into()
        }).collect::<Vec<SharedString>>());
        let requests = Rc::new(RefCell::new(Vec::new()));
        let log_bounds = Rc::new(Cell::new(None));
        let panel_renders = Rc::new(Cell::new(0));
        let panels = [Some(terminal.clone()), None].map(|terminal| {
            cx.new(|cx| ProbePanel {
                terminal,
                focus: cx.focus_handle(),
                logs: logs.clone(),
                requests: requests.clone(),
                log_bounds: log_bounds.clone(),
                log_scroll: UniformListScrollHandle::new(),
                renders: panel_renders.clone(),
            })
        });
        let (area, skin) = DockSkin::dock_area("iceon-offline-console-probe", None, window, cx);
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
            area.set_dock_size(DockPlacement::Bottom, px(180.), window, cx);
        });
        Self {
            area,
            _skin: skin,
            terminal,
            _panels: panels,
            _input_subscription: subscription,
            inputs,
            resizes,
            requests,
            log_bounds,
            grids: Vec::new(),
            panel_renders,
            idle_baseline: 0,
            armed: false,
            outcome,
        }
    }

    fn check(
        &mut self,
        stage: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let grid = self.terminal.read(cx).size();
        let contents = self.terminal.read(cx).contents();
        let require = |ok: bool, message: &str| {
            if ok {
                Ok(())
            } else {
                Err(format!("stage {stage}: {message}"))
            }
        };
        require(grid.0 > 0 && grid.1 > 0, "terminal has an empty grid")?;
        match stage {
            0 => {
                require(
                    cx.text_system()
                        .all_font_names()
                        .iter()
                        .any(|name| name == crate::console_font::FAMILY),
                    "bundled JetBrains Mono was not registered",
                )?;
                require(
                    !self.resizes.borrow().is_empty(),
                    "native terminal measurement did not emit Resize",
                )?;
                require(
                    contents == EXPECTED,
                    &format!("ANSI cells after initial layout: {contents:?}"),
                )?;
                require(
                    !contents.contains('\x1b')
                        && !contents.contains("stale")
                        && !contents.contains("old"),
                    "ANSI was displayed literally or clear/cursor failed",
                )?;
                let requests = self.requests.borrow();
                require(
                    !requests.is_empty(),
                    "log virtual-list callback never rendered",
                )?;
                require(
                    requests.iter().all(|range| range.len() < LOG_COUNT),
                    "virtual list requested all 100 rows",
                )?;
                drop(requests);
                let rect = self.log_bounds.get().ok_or("log body was not measured")?;
                require(
                    rect.size.width > px(0.) && rect.size.height > px(0.),
                    "log body has no native bounds",
                )?;
                window.dispatch_event(
                    PlatformInput::Scroll(ScrollEvent {
                        position: rect.origin + point(rect.size.width / 2., rect.size.height / 2.),
                        delta: ScrollDelta::Lines(point(0., -4.)),
                        ..Default::default()
                    }),
                    cx,
                );
                // Focus before dispatch; the next stage runs after the focused
                // dispatch tree has been committed by a native frame.
                window.focus(&self.terminal.focus_handle(cx), cx);
                self.grids.push(grid);
            }
            1 => {
                require(
                    window.focused(cx) == Some(self.terminal.focus_handle(cx)),
                    "terminal did not retain focus",
                )?;
            }
            2 => {
                require(
                    self.inputs.borrow().iter().map(String::as_str).eq(INPUT),
                    &format!("raw synthetic Input events: {:?}", self.inputs.borrow()),
                )?;
                require(
                    contents == EXPECTED,
                    "input was locally echoed without a remote response",
                )?;
                // Explicit offline remote-response callback, not a local shell.
                self.terminal.update(cx, |terminal, cx| {
                    terminal.feed("x\r\nfixture remote echo: x^C\r\n$ ", cx)
                });
                require(
                    self.terminal
                        .read(cx)
                        .contents()
                        .contains("$ x\nfixture remote echo: x^C"),
                    "cursor was not preserved at the prompt",
                )?;
                self.area.update(cx, |area, cx| {
                    area.set_dock_size(DockPlacement::Bottom, px(280.), window, cx)
                });
            }
            3 => {
                require(
                    grid.1 < self.grids[0].1,
                    "resizing the bottom Dock did not shrink the terminal grid",
                )?;
                self.grids.push(grid);
                self.area.update(cx, |area, cx| {
                    area.toggle_dock(DockPlacement::Bottom, window, cx)
                });
                require(
                    !self.area.read(cx).is_dock_open(DockPlacement::Bottom),
                    "Logs dock did not collapse",
                )?;
            }
            4 => {
                let shell_id = PanelId::from(self._panels[0].entity_id());
                require(
                    grid.1 > self.grids[1].1,
                    "collapsing Logs did not enlarge the terminal",
                )?;
                self.grids.push(grid);
                self.area.update(cx, |area, cx| {
                    area.toggle_dock(DockPlacement::Bottom, window, cx);
                    let node = area
                        .layout(DockPlacement::Center)
                        .expect("center layout")
                        .find_panel_node(shell_id)
                        .expect("Shell tab group");
                    area.set_zoomed_in(node, window, cx);
                });
                require(
                    self.area.read(cx).is_zoomed(),
                    "Shell Dock maximize was not applied",
                )?;
            }
            5 => {
                require(
                    grid.1 > self.grids[1].1,
                    "maximized Shell did not fill the Dock",
                )?;
                self.grids.push(grid);
                self.area
                    .update(cx, |area, cx| area.set_zoomed_out(window, cx));
                require(
                    !self.area.read(cx).is_zoomed(),
                    "Dock restore did not clear zoom",
                )?;
            }
            6 => {
                require(
                    grid == self.grids[1],
                    "restoring Dock did not restore the resized terminal grid",
                )?;
                require(
                    contents.starts_with(
                        "offline:/ $\nprogress 25%\ncursor: ready\n$ x\nfixture remote echo: x^C",
                    ),
                    "Dock transitions lost terminal cells",
                )?;
                require(
                    self.inputs.borrow().len() == INPUT.len(),
                    "layout/scroll emitted unexpected terminal input",
                )?;
                self.idle_baseline = self.panel_renders.get();
                println!(
                    "PASS console_probe grid={:?} -> {:?}; synthetic input={:?}; cells={:?}; virtual log batches={} (each < {LOG_COUNT}); measured-body wheel dispatched; Dock resize/collapse/maximize/restore",
                    self.grids,
                    grid,
                    self.inputs.borrow(),
                    contents,
                    self.requests.borrow().len()
                );
            }
            7 => {
                let extra = self.panel_renders.get().saturating_sub(self.idle_baseline);
                require(
                    extra <= 2,
                    &format!("idle Dock triggered {extra} unnecessary panel renders"),
                )?;
                println!(
                    "PASS console_probe: bundled JetBrains Mono; titleless center; idle panel renders={extra}"
                );
            }
            _ => unreachable!(),
        }
        Ok(())
    }
}

impl Render for ConsoleProbe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.armed {
            self.armed = true;
            let weak = cx.entity().downgrade();
            // Timers use GPUI's platform executor, not Tokio or an app runtime.
            // Start only after an actual native frame has rendered the Dock.
            window.on_next_frame(move |window, cx| {
                let _ = weak.update(cx, |_, cx| {
                    cx.spawn_in(window, async move |this, cx| {
                        for stage in 0..=7 {
                            cx.background_executor()
                                .timer(Duration::from_millis(350))
                                .await;
                            let result = cx.update(|window, cx| {
                                let checked = this.update(cx, |this, cx| {
                                    let result = this.check(stage, window, cx);
                                    match &result {
                                        Ok(()) => println!("console_probe: stage {stage} passed"),
                                        Err(error) => eprintln!("FAIL console_probe: {error}"),
                                    }
                                    if result.is_err() || stage == 7 {
                                        *this.outcome.borrow_mut() = Some(result.clone());
                                        cx.quit();
                                    }
                                    result
                                });
                                // Input callbacks update the probe owner, so dispatch only
                                // after releasing its entity-map borrow.
                                if stage == 1 && matches!(&checked, Ok(Ok(()))) {
                                    for key in KEYS {
                                        window
                                            .dispatch_keystroke(Keystroke::parse(key).unwrap(), cx);
                                    }
                                }
                                checked
                            });
                            if !matches!(result, Ok(Ok(Ok(())))) {
                                break;
                            }
                        }
                    })
                    .detach();
                });
            });
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0()
            .child(div().h(px(30.)).flex_shrink_0().px_3().child(TITLE))
            .child(div().flex_1().min_h_0().min_w_0().child(self.area.clone()))
    }
}

/// Open one fixture-only native window, exercise it, print PASS and quit.
/// The normal application's startup path is intentionally never called.
pub fn run() {
    let outcome = Rc::new(RefCell::new(None));
    let result = outcome.clone();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            crate::console_font::init(cx);
            terminal_surface::init(cx);
            // Also bound failures before the first frame (or a lost view/window
            // during an async update). Never leave a probe app running silently.
            let watchdog = result.clone();
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_secs(10))
                    .await;
                cx.update(|cx| {
                    if watchdog.borrow().is_none() {
                        eprintln!("FAIL console_probe: native render probe timed out");
                        *watchdog.borrow_mut() = Some(Err("native render probe timed out".into()));
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
                        size(px(1000.), px(600.)),
                        cx,
                    ))),
                    ..Default::default()
                },
                cx,
                move |window, cx| {
                    window.set_window_title(TITLE);
                    cx.new(|cx| ConsoleProbe::new(window, cx, result))
                },
            )
            .expect("failed to open offline console probe");
            cx.activate(true);
        });
    match outcome.borrow_mut().take() {
        Some(Ok(())) => {}
        Some(Err(error)) => panic!("FAIL console_probe: {error}"),
        None => panic!("FAIL console_probe: window closed before checks completed"),
    }
}

#[cfg(test)]
mod tests {
    use super::{EXPECTED, FIXTURE};
    use std::num::NonZeroU16;

    #[::core::prelude::v1::test]
    fn fixture_cells_cursor_and_resize() {
        let mut parser = vt100::Parser::new(
            NonZeroU16::new(24).unwrap(),
            NonZeroU16::new(80).unwrap(),
            100,
        );
        parser.process(FIXTURE.as_bytes());
        assert_eq!(parser.screen().contents(), EXPECTED);
        assert_eq!(parser.screen().cursor_position(), (3, 2));
        assert_eq!(
            parser.screen().cell(0, 0).unwrap().fgcolor(),
            vt100::Color::Idx(2)
        );
        assert!(parser.screen().cell(0, 0).unwrap().bold());
        assert_eq!(
            parser.screen().cell(2, 8).unwrap().fgcolor(),
            vt100::Color::Rgb(80, 170, 255)
        );
        parser.set_size(NonZeroU16::new(12).unwrap(), NonZeroU16::new(100).unwrap());
        assert_eq!(parser.screen().contents(), EXPECTED);
        assert_eq!(parser.screen().cursor_position(), (3, 2));
        assert!(!parser.screen().contents().contains('\x1b'));
    }
}
