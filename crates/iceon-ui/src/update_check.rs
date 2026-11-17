//! Shared update check, background 15-minute polling, download staging, and restart lifecycle.
use crate::IceonApp;
use gpui_kit::{
    component::{
        WindowExt as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    prelude::*,
    *,
};

pub(crate) struct UpdateDialogState {
    pub(crate) message: String,
    pub(crate) can_restart: bool,
}

impl IceonApp {
    pub(crate) fn init_update_lifecycle(&mut self, cx: &mut Context<Self>) {
        if let Some(downloaded) = iceon_core::updates::has_ready_update(env!("CARGO_PKG_VERSION")) {
            self.update_status = crate::shell_footer::UpdateStatus::Downloaded(downloaded.version);
        }

        // Initial background check shortly after application startup.
        let initial_owner = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(15))
                .await;
            let _ = initial_owner.update(cx, |app, cx| {
                app.poll_background_update(cx);
            });
        })
        .detach();

        // Automatic background update check and download every 15 minutes.
        let poll_owner = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(15 * 60))
                    .await;
                let settled = poll_owner.update(cx, |app, cx| {
                    app.poll_background_update(cx);
                });
                if settled.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    pub(crate) fn poll_background_update(&mut self, cx: &mut Context<Self>) {
        if self.update_check_running {
            return;
        }
        if matches!(
            self.update_status,
            crate::shell_footer::UpdateStatus::Downloaded(_)
        ) {
            return;
        }

        self.update_check_running = true;
        let task = iceon_core::runtime::spawn(async move {
            let check_res = iceon_core::updates::check().await?;
            if !check_res.available {
                return Ok(None);
            }
            iceon_core::updates::check_and_download(env!("CARGO_PKG_VERSION")).await
        });

        cx.spawn(async move |owner, cx| {
            let result = task.await;
            let _ = owner.update(cx, |app, cx| {
                app.update_check_running = false;
                match result {
                    Ok(Ok(Some(downloaded))) => {
                        app.update_status =
                            crate::shell_footer::UpdateStatus::Downloaded(downloaded.version);
                    }
                    Ok(Ok(None)) => {
                        if !matches!(
                            app.update_status,
                            crate::shell_footer::UpdateStatus::Downloaded(_)
                        ) {
                            app.update_status = crate::shell_footer::UpdateStatus::Current;
                        }
                    }
                    _ => {
                        // Background polling does not overwrite an existing downloaded status or pop errors.
                    }
                }
                cx.notify();
                cx.refresh_windows();
            });
        })
        .detach();
    }

    pub(crate) fn check_for_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let crate::shell_footer::UpdateStatus::Downloaded(version) = &self.update_status {
            let version = version.clone();
            let owner = cx.entity().downgrade();
            window.open_dialog(cx, move |dialog, _, _| {
                let owner = owner.clone();
                dialog.title("Update Ready").child(
                    v_flex()
                        .gap_3()
                        .child(div().text_sm().child(format!(
                            "Iceon {version} has been downloaded and is ready to install."
                        )))
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Button::new("restart-to-update-dialog")
                                        .primary()
                                        .label("Restart to Update")
                                        .on_click(move |_, window, cx| {
                                            let _ = owner.update(cx, |app, cx| {
                                                app.restart_to_update(window, cx);
                                            });
                                        }),
                                )
                                .child(
                                    Button::new("open-official-releases")
                                        .outline()
                                        .label("Release Notes")
                                        .on_click(|_, _, cx| {
                                            cx.open_url(iceon_core::updates::RELEASES_URL)
                                        }),
                                ),
                        ),
                )
            });
            return;
        }

        if self.update_check_running {
            window.open_dialog(cx, |dialog, _, _| {
                dialog.title("Check for Updates").child(
                    v_flex()
                        .gap_3()
                        .child(
                            div()
                                .text_sm()
                                .child("An update check or download is currently in progress…"),
                        )
                        .child(
                            Button::new("open-official-releases")
                                .outline()
                                .label("Open Releases")
                                .on_click(|_, _, cx| {
                                    cx.open_url(iceon_core::updates::RELEASES_URL)
                                }),
                        ),
                )
            });
            return;
        }

        self.update_check_running = true;
        let state = cx.new(|_| UpdateDialogState {
            message: "Checking GitHub for updates…".to_owned(),
            can_restart: false,
        });

        let view_state = state.clone();
        let owner_ref = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, _| {
            let owner_ref = owner_ref.clone();
            let view_state = view_state.clone();
            dialog
                .title("Check for Updates")
                .child(update_dialog_content(view_state, owner_ref))
        });

        cx.notify();
        cx.spawn_in(window, async move |owner, cx| {
            let check_task = iceon_core::runtime::spawn(iceon_core::updates::check());
            let check_res = check_task.await;
            match check_res {
                Ok(Ok(check)) if check.available => {
                    let version = check.latest.clone();
                    let _ = owner.update_in(cx, |app, _, cx| {
                        app.update_status =
                            crate::shell_footer::UpdateStatus::Downloading(version.clone());
                        state.update(cx, |s, cx| {
                            s.message = format!(
                                "Iceon {version} is available. Downloading update in background…"
                            );
                            cx.notify();
                        });
                        cx.notify();
                        cx.refresh_windows();
                    });

                    let dl_task = iceon_core::runtime::spawn(async move {
                        iceon_core::updates::check_and_download(env!("CARGO_PKG_VERSION")).await
                    });
                    let dl_res = dl_task.await;
                    let _ = owner.update_in(cx, |app, _, cx| {
                        app.update_check_running = false;
                        match dl_res {
                            Ok(Ok(Some(downloaded))) => {
                                app.update_status =
                                    crate::shell_footer::UpdateStatus::Downloaded(
                                        downloaded.version.clone(),
                                    );
                                let msg = format!(
                                    "Iceon {} has been downloaded and is ready to install.",
                                    downloaded.version
                                );
                                state.update(cx, |s, cx| {
                                    s.message = msg;
                                    s.can_restart = true;
                                    cx.notify();
                                });
                            }
                            _ => {
                                app.update_status =
                                    crate::shell_footer::UpdateStatus::Available(version.clone());
                                let msg = format!(
                                    "Iceon {version} is available, but automatic download could not be completed. You can open Releases to download it manually."
                                );
                                state.update(cx, |s, cx| {
                                    s.message = msg;
                                    s.can_restart = false;
                                    cx.notify();
                                });
                            }
                        }
                        cx.notify();
                        cx.refresh_windows();
                    });
                }
                Ok(Ok(check)) => {
                    let _ = owner.update_in(cx, |app, _, cx| {
                        app.update_check_running = false;
                        app.update_status = crate::shell_footer::UpdateStatus::Current;
                        let msg = format!(
                            "You’re up to date. Iceon {} is installed (latest stable: {}).",
                            env!("CARGO_PKG_VERSION"),
                            check.latest
                        );
                        state.update(cx, |s, cx| {
                            s.message = msg;
                            s.can_restart = false;
                            cx.notify();
                        });
                        cx.notify();
                        cx.refresh_windows();
                    });
                }
                Ok(Err(err)) => {
                    let _ = owner.update_in(cx, |app, _, cx| {
                        app.update_check_running = false;
                        app.update_status = crate::shell_footer::UpdateStatus::Failed;
                        state.update(cx, |s, cx| {
                            s.message = err;
                            s.can_restart = false;
                            cx.notify();
                        });
                        cx.notify();
                        cx.refresh_windows();
                    });
                }
                Err(_) => {
                    let _ = owner.update_in(cx, |app, _, cx| {
                        app.update_check_running = false;
                        app.update_status = crate::shell_footer::UpdateStatus::Failed;
                        state.update(cx, |s, cx| {
                            s.message = "The update check stopped. Please try again.".into();
                            s.can_restart = false;
                            cx.notify();
                        });
                        cx.notify();
                        cx.refresh_windows();
                    });
                }
            }
        })
        .detach();
    }

    pub(crate) fn restart_to_update(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(downloaded) = iceon_core::updates::read_downloaded_update() {
            match iceon_core::updates::restart_with_update(&downloaded.path) {
                Ok(()) => {
                    cx.quit();
                }
                Err(err) => {
                    eprintln!("Failed to restart with update: {err}");
                    cx.open_url(iceon_core::updates::RELEASES_URL);
                }
            }
        } else if let Ok(current_exe) = std::env::current_exe() {
            let _ = iceon_core::updates::restart_with_update(&current_exe);
            cx.quit();
        }
    }
}

fn update_dialog_content(
    state: Entity<UpdateDialogState>,
    owner: WeakEntity<IceonApp>,
) -> impl IntoElement {
    v_flex()
        .gap_3()
        .child(UpdateDialogBody(state.clone()))
        .child(UpdateDialogActions { state, owner })
}

#[derive(IntoElement)]
struct UpdateDialogBody(Entity<UpdateDialogState>);

impl RenderOnce for UpdateDialogBody {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div().text_sm().child(self.0.read(cx).message.clone())
    }
}

#[derive(IntoElement)]
struct UpdateDialogActions {
    state: Entity<UpdateDialogState>,
    owner: WeakEntity<IceonApp>,
}

impl RenderOnce for UpdateDialogActions {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let can_restart = self.state.read(cx).can_restart;
        let owner = self.owner;
        if can_restart {
            h_flex()
                .gap_2()
                .child(
                    Button::new("restart-to-update-dialog")
                        .primary()
                        .label("Restart to Update")
                        .on_click(move |_, window, cx| {
                            let _ = owner.update(cx, |app, cx| {
                                app.restart_to_update(window, cx);
                            });
                        }),
                )
                .child(
                    Button::new("open-official-releases")
                        .outline()
                        .label("Release Notes")
                        .on_click(|_, _, cx| cx.open_url(iceon_core::updates::RELEASES_URL)),
                )
                .into_any_element()
        } else {
            Button::new("open-official-releases")
                .outline()
                .label("Open Releases")
                .on_click(|_, _, cx| cx.open_url(iceon_core::updates::RELEASES_URL))
                .into_any_element()
        }
    }
}
