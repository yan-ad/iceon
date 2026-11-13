//! Offline native checkbox interaction probe. No engines, cluster, or shell.
//! cargo run --locked -p iceon-ui --example checkbox_probe
#[path = "../src/app_checkbox.rs"]
#[allow(dead_code)] // The fixture intentionally exercises only the tri-state API.
mod app_checkbox;

use gpui_kit::base::CheckboxState;
use gpui_kit::component::{ActiveTheme as _, Theme, ThemeMode};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

type Outcome = Rc<RefCell<Option<Result<(), String>>>>;
struct Probe {
    state: CheckboxState,
    changes: usize,
    parent_clicks: usize,
    bounds: [Rc<Cell<Option<Bounds<Pixels>>>>; 2],
    armed: bool,
    outcome: Outcome,
}

impl Probe {
    fn verify(
        &mut self,
        stage: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        eprintln!("checkbox_probe: checking stage {stage}");
        let require = |condition, message: &str| {
            if condition {
                Ok(())
            } else {
                Err(format!("stage {stage}: {message}"))
            }
        };
        match stage {
            0 => require(
                self.bounds.iter().all(|bounds| bounds.get().is_some()),
                "checkboxes have no native bounds",
            )?,
            1 => {
                require(
                    self.state == CheckboxState::Checked && self.changes == 1,
                    "mixed checkbox did not become checked on click",
                )?;
                require(
                    self.parent_clicks == 0,
                    "checkbox click opened its parent row",
                )?;
            }
            2 => require(
                self.state == CheckboxState::Unchecked && self.changes == 2,
                "Space did not toggle the focused checkbox",
            )?,
            3 => {
                require(self.changes == 2, "disabled checkbox invoked its callback")?;
                Theme::change(ThemeMode::Light, Some(window), cx);
            }
            4 => {
                require(
                    self.state == CheckboxState::Checked && self.changes == 3,
                    "light-theme checkbox did not toggle",
                )?;
                println!(
                    "PASS checkbox_probe: mixed -> checked; Space toggle; disabled inert; row isolation; light/dark themes"
                );
            }
            _ => unreachable!(),
        }
        Ok(())
    }
}

fn click(bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    let position = bounds.center();
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
            let owner = cx.entity().downgrade();
            window.on_next_frame(move |window, cx| {
                let _ = owner.update(cx, |_, cx| {
                    cx.spawn_in(window, async move |this, cx| {
                        for stage in 0..=4 {
                            cx.background_executor()
                                .timer(Duration::from_millis(150))
                                .await;
                            let result = this.update_in(cx, |this, window, cx| {
                                let result = this.verify(stage, window, cx);
                                if let Err(error) = &result {
                                    eprintln!("FAIL checkbox_probe: {error}");
                                }
                                if result.is_err() || stage == 4 {
                                    *this.outcome.borrow_mut() = Some(result.clone());
                                    cx.quit();
                                }
                                (result, this.bounds.clone())
                            });
                            let Ok((Ok(()), bounds)) = result else { break };
                            cx.update(|window, cx| match stage {
                                0 | 3 => click(bounds[0].get().unwrap(), window, cx),
                                1 => {
                                    window
                                        .dispatch_keystroke(Keystroke::parse("space").unwrap(), cx);
                                    window.dispatch_event(
                                        PlatformInput::KeyUp(KeyUpEvent {
                                            keystroke: Keystroke::parse("space").unwrap(),
                                        }),
                                        cx,
                                    );
                                }
                                2 => click(bounds[1].get().unwrap(), window, cx),
                                _ => {}
                            })
                            .ok();
                        }
                    })
                    .detach();
                });
            });
        }
        let bounds = self.bounds.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .p_4()
            .gap_4()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child("Zed-style checkbox — offline native probe")
            .children(
                [false, true]
                    .into_iter()
                    .enumerate()
                    .map(|(index, disabled)| {
                        let measured = bounds[index].clone();
                        let owner = cx.entity().downgrade();
                        div()
                            .id(("checkbox-row", index))
                            .flex()
                            .items_center()
                            .h(px(36.))
                            .relative()
                            .on_click(cx.listener(|this, _, _, _| this.parent_clicks += 1))
                            .child(
                                div()
                                    .relative()
                                    .child(
                                        app_checkbox::Checkbox::new(("checkbox-probe", index))
                                            .state(self.state)
                                            .disabled(disabled)
                                            .accessibility_label(if disabled {
                                                "Disabled selection"
                                            } else {
                                                "Select resource"
                                            })
                                            .on_change(move |state, _, _, cx| {
                                                cx.stop_propagation();
                                                let _ = owner.update(cx, |this, cx| {
                                                    this.state = state;
                                                    this.changes += 1;
                                                    cx.notify();
                                                });
                                            }),
                                    )
                                    .child(
                                        canvas(
                                            move |bounds, _, _| measured.set(Some(bounds)),
                                            |_, _, _, _| {},
                                        )
                                        .absolute()
                                        .inset_0(),
                                    ),
                            )
                            .child(div().ml_2().child(if disabled {
                                "Disabled"
                            } else {
                                "Mixed / checked / unchecked"
                            }))
                    }),
            )
    }
}

fn main() {
    let outcome: Outcome = Rc::new(RefCell::new(None));
    let result = outcome.clone();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            Theme::change(ThemeMode::Dark, None, cx);
            let watchdog = result.clone();
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_secs(10))
                    .await;
                cx.update(|cx| {
                    if watchdog.borrow().is_none() {
                        *watchdog.borrow_mut() = Some(Err("checkbox probe timed out".into()));
                    }
                    cx.quit();
                });
            })
            .detach();
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(100.), px(100.)),
                        size(px(500.), px(240.)),
                    ))),
                    ..Default::default()
                },
                cx,
                |_, cx| {
                    cx.new(|_| Probe {
                        state: CheckboxState::Indeterminate,
                        changes: 0,
                        parent_clicks: 0,
                        bounds: std::array::from_fn(|_| Rc::new(Cell::new(None))),
                        armed: false,
                        outcome: result,
                    })
                },
            )
            .unwrap();
            cx.activate(true);
        });
    let result = outcome.borrow_mut().take();
    match result {
        Some(Ok(())) => {}
        Some(Err(error)) => {
            eprintln!("FAIL checkbox_probe: {error}");
            std::process::exit(1);
        }
        None => {
            eprintln!("FAIL checkbox_probe: closed before verification");
            std::process::exit(1);
        }
    }
}
