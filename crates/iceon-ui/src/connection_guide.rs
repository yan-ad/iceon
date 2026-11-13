//! Native first-connection guide and typed, credential-free recovery surfaces.
use gpui_kit::base::Selectable as _;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use iceon_core::connection::{ConnectionError, diagnostics::ConnectionSetupDiagnostic};

use crate::{
    IceonApp,
    navigation::{Route, connection_error_message},
};

const CONFIG_DOCS: &str =
    "https://kubernetes.io/docs/concepts/configuration/organize-cluster-access-kubeconfig/";
const AUTH_DOCS: &str = "https://kubernetes.io/docs/reference/access-authn-authz/authentication/#client-go-credential-plugins";
const GKE_DOCS: &str = "https://cloud.google.com/kubernetes-engine/docs/how-to/cluster-access-for-kubectl#install_plugin";
const AWS_DOCS: &str = "https://docs.aws.amazon.com/eks/latest/userguide/create-kubeconfig.html";
// kubelogin is also the name of unrelated OIDC tools; never infer Azure from that name alone.
const AZURE_DOCS: &str = "https://learn.microsoft.com/en-us/azure/aks/kubelogin-authentication";

#[derive(Default)]
pub(crate) struct ConnectionGuide {
    pub error: Option<ConnectionError>,
    pub diagnostic: Option<ConnectionSetupDiagnostic>,
    step: usize,
    copied: bool,
}

pub(crate) fn retryable_connection_error(error: ConnectionError) -> bool {
    matches!(
        error,
        ConnectionError::Network
            | ConnectionError::Timeout
            | ConnectionError::ConnectionFailed
            | ConnectionError::Busy
    )
}

fn diagnostic_summary(
    error: ConnectionError,
    diagnostic: Option<&ConnectionSetupDiagnostic>,
) -> String {
    let helper = diagnostic
        .and_then(|diagnostic| diagnostic.executable_basename.as_deref())
        .unwrap_or("not identified");
    format!(
        "Iceon connection setup\n{}\nAuthentication helper: {}\nkubectl is not required for browsing.\nNo credentials or kubeconfig contents included.",
        connection_error_message(error),
        helper
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Config,
    Authentication,
    Network,
}

struct Recovery {
    title: &'static str,
    detail: &'static str,
    action: &'static str,
    stage: Stage,
    docs: &'static str,
}

fn recovery(error: ConnectionError) -> Recovery {
    use ConnectionError::*;
    let (title, detail, action, stage, docs) = match error {
        KubeconfigUnavailable | NotFound => (
            "Let's find your kubeconfig",
            "The saved configuration is no longer available. Import the original file again, or choose an updated kubeconfig from your cluster administrator.",
            "Import kubeconfig",
            Stage::Config,
            CONFIG_DOCS,
        ),
        KubeconfigInvalid | SourceChanged | InvalidInput => (
            "Your configuration needs a review",
            "The selected context could not be validated, or its source changed. Import an updated kubeconfig from a trusted source, then reconnect.",
            "Import kubeconfig",
            Stage::Config,
            CONFIG_DOCS,
        ),
        InsecureTransport => (
            "A secure connection is required",
            "Use an HTTPS API endpoint with a valid certificate authority. Iceon does not bypass TLS verification or accept insecure kubeconfigs.",
            "Import kubeconfig",
            Stage::Config,
            CONFIG_DOCS,
        ),
        AuthHelperMissing => (
            "One sign-in tool is missing",
            "Your kubeconfig uses an external authentication helper. Install the helper configured in your kubeconfig, or ask your administrator for the correct setup, then retry the connection.",
            "Open auth setup",
            Stage::Authentication,
            AUTH_DOCS,
        ),
        AuthInteractiveRequired => (
            "Finish signing in outside Iceon",
            "This context requires interactive terminal input. Complete your provider's sign-in in a trusted terminal and use a context that supports non-interactive credential refresh.",
            "Open auth setup",
            Stage::Authentication,
            AUTH_DOCS,
        ),
        AuthHelperFailed | AuthHelperInvalidResponse => (
            "Your sign-in tool needs attention",
            "The authentication helper could not return usable credentials. Check its installation and provider login in a trusted terminal, then retry. Installing kubectl alone will not fix this.",
            "Open auth setup",
            Stage::Authentication,
            AUTH_DOCS,
        ),
        UnsupportedAuth => (
            "This sign-in method needs updating",
            "Ask your administrator for a supported exec-credential kubeconfig, or a context using supported credentials. Re-import the updated file when it is ready.",
            "Import kubeconfig",
            Stage::Authentication,
            AUTH_DOCS,
        ),
        AuthExpired => (
            "It's time to sign in again",
            "Your credentials expired or were rejected. Renew your provider login or obtain an updated kubeconfig, then retry the connection.",
            "Retry connection",
            Stage::Authentication,
            AUTH_DOCS,
        ),
        AuthFailed => (
            "Your provider rejected the sign-in",
            "Renew your provider login or obtain an updated kubeconfig. Once the provider account is ready, retry the connection.",
            "Open auth setup",
            Stage::Authentication,
            AUTH_DOCS,
        ),
        TlsFailed => (
            "Check the cluster certificate",
            "The secure handshake could not be verified. Check the cluster CA, device clock, and any network inspection software. Keep TLS verification enabled.",
            "Retry connection",
            Stage::Network,
            CONFIG_DOCS,
        ),
        PermissionDenied | PermissionReviewUnavailable => (
            "Check access with your administrator",
            "The connection attempt could not verify the access it needs. Check the permissions for this context; this is not evidence that every namespace is denied.",
            "Retry connection",
            Stage::Network,
            CONFIG_DOCS,
        ),
        Network | Timeout | ConnectionFailed => (
            "Your cluster is out of reach",
            "Check your internet connection, required VPN, and cluster API endpoint. Private clusters may only be reachable from your organization's network.",
            "Retry connection",
            Stage::Network,
            CONFIG_DOCS,
        ),
        _ => (
            "Let's get this connection working",
            "The connection did not complete. Retry, check Kubernetes preferences, or import an updated configuration. No cluster resources were changed by these setup checks.",
            "Retry connection",
            Stage::Network,
            CONFIG_DOCS,
        ),
    };
    Recovery {
        title,
        detail,
        action,
        stage,
        docs,
    }
}

fn provider_docs(helper: Option<&str>) -> Option<(&'static str, &'static str)> {
    match helper.map(|name| name.strip_suffix(".exe").unwrap_or(name)) {
        Some("gke-gcloud-auth-plugin" | "gcloud") => Some(("Google Cloud setup", GKE_DOCS)),
        Some("aws" | "aws-iam-authenticator") => Some(("Amazon EKS setup", AWS_DOCS)),
        Some("az") => Some(("Azure sign-in setup", AZURE_DOCS)),
        _ => None,
    }
}

const STEPS: [(&str, &str, IconName); 4] = [
    (
        "Bring your config",
        "Import a file or synced folder. Get a kubeconfig from your cloud provider or cluster administrator.",
        IconName::FolderOpen,
    ),
    (
        "Pick your context",
        "Choose a cluster in the header. A context links a cluster with the identity you use to access it.",
        IconName::Folder,
    ),
    (
        "Complete sign-in",
        "Some contexts need a provider tool. Iceon uses the authentication method in your kubeconfig; kubectl is not a prerequisite.",
        IconName::Network,
    ),
    (
        "Explore your cluster",
        "Iceon reviews your resource permissions, then opens your workspace. Browse workloads and choose the namespaces you need.",
        IconName::LayoutDashboard,
    ),
];

impl IceonApp {
    pub(crate) fn record_connection_failure(
        &mut self,
        error: ConnectionError,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Route::Cluster { cluster_id, .. } = self.history.current().clone() else {
            return;
        };
        self.connection_guide.error = Some(error);
        self.cluster_error = Some(connection_error_message(error).into());
        self.cluster_loading = false;
        let connection = self.connection.clone();
        let generation = self.cluster_connect_generation;
        cx.spawn_in(window, async move |owner, cx| {
            let diagnostic = connection.connection_setup_diagnostic(cluster_id.clone()).await.ok();
            let _ = owner.update_in(cx, |app, _, cx| {
                if app.cluster_connect_generation == generation
                    && app.connection_guide.error == Some(error)
                    && matches!(app.history.current(), Route::Cluster { cluster_id: current, .. } if current == &cluster_id)
                {
                    app.connection_guide.diagnostic = diagnostic;
                    cx.notify();
                }
            });
        }).detach();
    }

    pub(crate) fn retry_cluster_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Route::Cluster {
            cluster_id,
            section,
        } = self.history.current().clone()
        {
            self.open_cluster_workspace(cluster_id, section, window, cx);
        }
    }

    pub(crate) fn render_connection_guide(
        &self,
        connecting: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let error = self.connection_guide.error;
        let issue = error.map(recovery);
        let helper = self
            .connection_guide
            .diagnostic
            .as_ref()
            .and_then(|d| d.executable_basename.as_deref());
        let blocked_step = issue.as_ref().map(|r| match r.stage {
            Stage::Config => 0,
            Stage::Authentication => 2,
            Stage::Network => 3,
        });
        let selected_step = blocked_step.unwrap_or(self.connection_guide.step.min(3));
        let theme = cx.theme();
        let foreground = theme.foreground;
        let muted = theme.muted_foreground;
        let primary = theme.primary;
        let border = theme.border;
        let secondary = theme.secondary;
        let warning = theme.warning;
        let mut tint = primary;
        tint.a = 0.10;
        let mut warning_tint = warning;
        warning_tint.a = 0.10;
        let title = issue.as_ref().map_or(
            if connecting {
                "Connecting to your cluster"
            } else {
                "Your cluster. A clearer view."
            },
            |issue| issue.title,
        );
        let detail = issue.as_ref().map_or(if connecting {
            "Reading your selected context, authenticating, and contacting the Kubernetes API. Provider helpers may run during this connection attempt."
        } else {
            "Bring a kubeconfig and we'll take it from there. Iceon connects directly to Kubernetes, with no kubectl installation required."
        }, |issue| issue.detail);
        let provider = provider_docs(helper);
        let docs = provider
            .map(|(_, url)| url)
            .unwrap_or(issue.as_ref().map_or(CONFIG_DOCS, |issue| issue.docs));
        let import = issue
            .as_ref()
            .is_none_or(|issue| issue.action == "Import kubeconfig");
        let retry = issue
            .as_ref()
            .is_some_and(|issue| issue.action == "Retry connection");
        let guide_actions = h_flex()
            .flex_wrap()
            .gap_2()
            .child(
                Button::new("guide-primary")
                    .primary()
                    .disabled(connecting)
                    .icon(
                        if issue
                            .as_ref()
                            .is_some_and(|i| i.action == "Retry connection")
                        {
                            IconName::RefreshCw
                        } else {
                            IconName::FolderOpen
                        },
                    )
                    .label(
                        issue
                            .as_ref()
                            .map_or("Import kubeconfig", |issue| issue.action),
                    )
                    .on_click(cx.listener(move |app, _, window, cx| {
                        if retry {
                            app.retry_cluster_connection(window, cx);
                        } else if import {
                            app.open_add_config(window, cx);
                        } else {
                            cx.open_url(docs);
                        }
                    })),
            )
            .child(
                Button::new("guide-preferences")
                    .outline()
                    .label("Kubernetes preferences")
                    .on_click(cx.listener(|app, _, window, cx| {
                        app.open_kubernetes_preferences(window, cx)
                    })),
            )
            .when(error.is_some() && !retry, |row| {
                row.child(
                    Button::new("guide-retry")
                        .outline()
                        .icon(IconName::RefreshCw)
                        .label("Retry connection")
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.retry_cluster_connection(window, cx)
                        })),
                )
            })
            .when(error.is_some(), |row| {
                row.child(
                    Button::new("guide-replace-config")
                        .ghost()
                        .label("Import another config")
                        .on_click(
                            cx.listener(|app, _, window, cx| app.open_add_config(window, cx)),
                        ),
                )
            });

        v_flex().id("connection-guide").size_full().min_w_0().min_h_0()
            .overflow_y_scroll().bg(theme.tokens.sidebar).text_color(foreground).p_6()
            .child(v_flex().w_full().max_w(px(920.)).mx_auto().gap_6()
                .child(h_flex().items_center().gap_3()
                    .child(div().size(px(48.)).rounded(px(16.)).bg(tint).flex().items_center().justify_center()
                        .child(Icon::new(if error.is_some() { IconName::CircleAlert } else { IconName::Network }).size(px(24.)).text_color(primary)))
                    .child(v_flex().gap_1()
                        .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(if error.is_some() { "Connection guide" } else { "Welcome to Iceon" }))
                        .child(div().text_xs().text_color(muted).child("A direct connection, with you in control"))))
                .child(v_flex().max_w(px(760.)).gap_3()
                    .child(div().text_size(px(32.)).font_weight(FontWeight::BOLD).child(title))
                    .child(div().text_sm().text_color(muted).child(detail)))
                .when(connecting, |view| view.child(h_flex().items_center().gap_2()
                    .child(gpui_kit::component::spinner::Spinner::new().small())
                    .child(div().text_sm().child("Establishing a secure connection…"))))
                .child(guide_actions)
                .when(!connecting && error.is_none() && !self.catalog_snapshot.clusters.is_empty(), |view| view.child(
                    v_flex().gap_2()
                        .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Choose an available context"))
                        .child(h_flex().flex_wrap().gap_2().children(self.catalog_snapshot.clusters.iter().map(|cluster| {
                            let id = cluster.id.clone();
                            Button::new(SharedString::from(format!("guide-cluster-{id}"))).outline()
                                .max_w(px(360.)).icon(IconName::Network).label(cluster.name.clone())
                                .tooltip(format!("Connect using {}", cluster.context_name))
                                .on_click(cx.listener(move |app, _, window, cx| app.navigate(
                                    Route::Cluster { cluster_id: id.clone(), section: "overview".into() }, true, window, cx)))
                        })))))
                .child(h_flex().w_full().flex_wrap().gap_2().children(STEPS.iter().enumerate().map(|(index, (title, _, icon))| {
                    let selected = index == selected_step;
                    let blocked = blocked_step == Some(index);
                    let color = if blocked { warning } else { primary };
                    v_flex().flex_1().min_w(px(176.)).gap_3()
                        .child(div().h(px(3.)).rounded_full().bg(if selected { color } else { border }))
                        .child(Button::new(("guide-step", index)).ghost().w_full().justify_start()
                            .icon(icon.clone()).label(format!("{}. {title}", index + 1))
                            .accessibility_label(format!("Setup step {}: {title}", index + 1))
                            .selected(selected).disabled(error.is_some() || connecting)
                            .when(selected, |button| button.bg(if blocked { warning_tint } else { tint }))
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.connection_guide.step = index;
                                cx.notify();
                            })))
                })))
                .child(v_flex().w_full().p_5().gap_4().rounded(px(16.)).border_1().border_color(border)
                    .child(h_flex().items_center().gap_2()
                        .child(Icon::new(STEPS[selected_step].2.clone()).text_color(if error.is_some() { warning } else { primary }))
                        .child(div().text_lg().font_weight(FontWeight::SEMIBOLD)
                            .child(if error.is_some() { "Your next step" } else { STEPS[selected_step].0 })))
                    .child(div().text_sm().text_color(muted).child(if error.is_some() {
                        "Resolve the highlighted stage, then retry. Your imported contexts stay available in the cluster selector."
                    } else { STEPS[selected_step].1 }))
                    .when_some(helper.map(str::to_owned).filter(|_| blocked_step == Some(2)), |view, helper| view.child(
                        h_flex().flex_wrap().items_center().gap_2().p_3().rounded(px(8.)).bg(secondary)
                            .child(div().text_sm().text_color(muted).child("Configured sign-in helper"))
                            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(helper))))
                    .when(cfg!(target_os = "macos") && blocked_step == Some(2), |view| view.child(div().text_sm().text_color(muted)
                        .child("Works in Terminal but not here? Finder apps can use a different PATH. Make the helper available to the app, or use an absolute executable path in your kubeconfig, then retry.")))
                    .child(h_flex().flex_wrap().gap_2()
                        .child(Button::new("guide-docs").outline().small().icon(IconName::ExternalLink)
                            .label(provider.map_or("Read setup instructions", |(label, _)| label))
                            .on_click(move |_, _, cx| cx.open_url(docs)))
                        .when(error.is_none() && !connecting && selected_step < 3, |row| row.child(Button::new("guide-next").ghost().small().label("Next step")
                            .on_click(cx.listener(|app, _, _, cx| { app.connection_guide.step = (app.connection_guide.step + 1).min(3); cx.notify(); }))))
                        .when_some(error, |row, error| row.child(Button::new("guide-copy-diagnostic").ghost().small()
                            .label(if self.connection_guide.copied { "Summary copied" } else { "Copy safe summary" })
                            .on_click(cx.listener(move |app, _, _, cx| {
                                // Explicit clipboard action: category and safe basename only, never raw errors/config.
                                let summary = diagnostic_summary(error, app.connection_guide.diagnostic.as_ref());
                                cx.write_to_clipboard(ClipboardItem::new_string(summary));
                                app.connection_guide.copied = true;
                                cx.notify();
                            }))))))
                .child(h_flex().items_start().gap_3().p_4().rounded(px(12.)).bg(tint)
                    .child(Icon::new(IconName::Info).text_color(primary))
                    .child(v_flex().gap_1().flex_1().min_w_0()
                        .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("No kubectl? You're still ready to start."))
                        .child(div().text_sm().text_color(muted).child("Cluster browsing uses the Kubernetes API directly. Provider sign-in helpers may be required by your context. kubectl is optional; Helm is needed only for Helm workflows."))))
                .when_some(self.catalog_error.clone().or_else(|| self.catalog_snapshot.discovery_error.map(|error| crate::navigation::catalog_error_message(error).into())), |view, message| view.child(div().text_sm().text_color(warning).child(message)))
                .child(div().text_xs().text_color(muted).child("Import only kubeconfigs you trust. Import reads catalog metadata; connecting may run the configured sign-in helper. Setup never installs tools automatically.")))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn setup_failures_have_distinct_stages_and_real_recovery_actions() {
        assert_eq!(
            recovery(ConnectionError::AuthHelperMissing).stage,
            Stage::Authentication
        );
        assert_eq!(
            recovery(ConnectionError::KubeconfigUnavailable).stage,
            Stage::Config
        );
        assert_eq!(recovery(ConnectionError::Network).stage, Stage::Network);
        assert_eq!(recovery(ConnectionError::TlsFailed).stage, Stage::Network);
        assert_eq!(
            recovery(ConnectionError::AuthExpired).action,
            "Retry connection"
        );
        assert_eq!(
            recovery(ConnectionError::InsecureTransport).action,
            "Import kubeconfig"
        );
        assert!(
            !recovery(ConnectionError::TlsFailed)
                .detail
                .contains("disable")
        );
    }

    #[::core::prelude::v1::test]
    fn provider_links_are_static_and_never_guess_ambiguous_kubelogin() {
        assert_eq!(
            provider_docs(Some("gke-gcloud-auth-plugin")),
            Some(("Google Cloud setup", GKE_DOCS))
        );
        assert!(provider_docs(Some("aws.exe")).is_some());
        assert!(provider_docs(Some("az")).is_some());
        for name in ["kubelogin", "oidc-login", "sh", "curl", "unknown"] {
            assert_eq!(provider_docs(Some(name)), None);
        }
    }

    #[::core::prelude::v1::test]
    fn retry_policy_only_retries_transient_failures() {
        assert!(retryable_connection_error(ConnectionError::Network));
        assert!(retryable_connection_error(ConnectionError::Timeout));
        assert!(!retryable_connection_error(
            ConnectionError::AuthHelperMissing
        ));
        assert!(!retryable_connection_error(
            ConnectionError::KubeconfigInvalid
        ));
    }

    #[::core::prelude::v1::test]
    fn diagnostic_summary_contains_no_raw_helper_arguments_or_credentials() {
        let summary = diagnostic_summary(
            ConnectionError::AuthHelperMissing,
            Some(&ConnectionSetupDiagnostic {
                context_name: Some("prod".into()),
                executable_basename: Some("gcloud".into()),
            }),
        );
        assert!(summary.contains("gcloud"));
        assert!(!summary.contains("--token"));
        assert!(!summary.contains("prod"));
        assert!(!summary.contains("password"));
    }
}
