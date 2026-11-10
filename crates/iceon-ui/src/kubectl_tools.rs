//! Explicit settings action: prepare kubectl and copy its path, never execute it.

use crate::*;
use iceon_core::connection::SessionRequest;

impl IceonApp {
    /// Hook for the Kubernetes preferences "Prepare kubectl for connected
    /// cluster" button. Uses persisted settings, not an unsaved form draft.
    pub(crate) fn prepare_kubectl(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self
            .cluster_session
            .as_ref()
            .filter(|s| s.status == "connected")
        else {
            self.preferences_error = Some("Connect to a cluster before preparing kubectl.".into());
            cx.notify();
            return;
        };
        let request = SessionRequest {
            cluster_id: session.cluster_id.clone(),
            session_id: session.session_id.clone(),
        };
        let preferences = match self.preferences_engine.snapshot() {
            Ok(snapshot) => snapshot.preferences,
            Err(error) => {
                self.preferences_error =
                    Some(format!("Unable to read saved kubectl settings: {error:?}"));
                cx.notify();
                return;
            }
        };
        self.preferences_error = Some("Preparing kubectl using saved settings…".into());
        cx.notify();
        let connection = self.connection.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = connection.prepare_kubectl(
                &request,
                &preferences.kubernetes,
                &preferences.proxy,
                &iceon_core::runtime::app_data_dir(),
            ).await;
            let _ = this.update_in(cx, |this, _, cx| {
                // A late result must not overwrite settings or clipboard after
                // the user switches/disconnects the workspace.
                if !this.cluster_session.as_ref().is_some_and(|session| {
                    session.cluster_id == request.cluster_id
                        && session.session_id == request.session_id
                        && session.status == "connected"
                }) {
                    this.preferences_error = Some("Kubectl preparation cancelled: cluster session changed.".into());
                    cx.notify();
                    return;
                }
                this.preferences_error = Some(match result {
                    Ok(resolved) => {
                        let path = resolved.path.display().to_string();
                        cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
                        format!("Kubectl ready: {path}. Path copied to clipboard; binary was not executed.")
                    }
                    Err(error) => format!("Unable to prepare kubectl: {error}. Enable automatic download or configure an existing executable, then save settings and retry."),
                });
                cx.notify();
            });
        }).detach();
    }
}
