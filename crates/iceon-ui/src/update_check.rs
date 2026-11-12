//! Shared explicit update check for menu and title-bar entry points.
use crate::IceonApp;
use gpui_kit::{
    component::{WindowExt as _, button::Button, v_flex},
    prelude::*,
    *,
};
impl IceonApp {
    pub(crate) fn check_for_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.update_check_running {
            return;
        }
        self.update_check_running = true;
        let status = cx.new(|_| "Checking GitHub for updates…".to_owned());
        let view = status.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            dialog
                .title("Check for Updates")
                .child(update_dialog_content(view))
        });
        let task = iceon_core::runtime::spawn(iceon_core::updates::check());
        cx.notify();
        cx.spawn_in(window, async move |owner, cx| {
            let result = task.await;
            let _ = owner.update_in(cx, |app, _, cx| {
                app.update_check_running = false;
                app.update_status = match &result {
                    Ok(Ok(result)) if result.available => crate::shell_footer::UpdateStatus::Available(result.latest.clone()),
                    Ok(Ok(_)) => crate::shell_footer::UpdateStatus::Current,
                    _ => crate::shell_footer::UpdateStatus::Failed,
                };
                let message = match result {
                    Ok(Ok(result)) if result.available => format!("Iceon {} is available. You have {}. Open Releases to download and install it manually.", result.latest,result.current),
                    Ok(Ok(result)) => format!("You’re up to date. Iceon {} is installed (latest stable: {}).",result.current,result.latest),
                    Ok(Err(error)) => error,
                    Err(_) => "The update check stopped. Please try again.".into(),
                };
                status.update(cx, |text,cx| { *text=message; cx.notify(); });
                cx.notify();
                cx.refresh_windows();
            });
        }).detach();
    }
}
fn update_dialog_content(status: Entity<String>) -> impl IntoElement {
    v_flex().gap_3().child(UpdateStatus(status)).child(
        Button::new("open-official-releases")
            .outline()
            .label("Open Releases")
            .on_click(|_, _, cx| cx.open_url(iceon_core::updates::RELEASES_URL)),
    )
}
#[derive(IntoElement)]
struct UpdateStatus(Entity<String>);
impl RenderOnce for UpdateStatus {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div().text_sm().child(self.0.read(cx).clone())
    }
}
