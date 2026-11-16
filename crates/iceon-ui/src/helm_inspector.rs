//! Metadata-only Helm drawer. Never requests values, notes, or manifests.
use super::*;
use iceon_core::connection::{
    ConnectionEngine,
    helm::{HelmCancelRequest, HelmHistory},
};
use std::sync::Arc;

/// Own cancellation for the drawer's request, including when app state is dropped.
/// Core's monotonic read identity prevents an old guard canceling a newer read.
pub(super) struct HelmInspectorRead {
    connection: Arc<ConnectionEngine>,
    request: HelmDetailRequest,
}
impl HelmInspectorRead {
    pub(super) fn new(connection: Arc<ConnectionEngine>, request: HelmDetailRequest) -> Self {
        Self {
            connection,
            request,
        }
    }
}
impl Drop for HelmInspectorRead {
    fn drop(&mut self) {
        let request = &self.request;
        let _ = self.connection.helm_detail_cancel(HelmCancelRequest {
            cluster_id: request.cluster_id.clone(),
            session_id: request.session_id.clone(),
            view: request.view,
            read_id: request.read_id,
        });
    }
}
fn same_request(left: &HelmDetailRequest, right: &HelmDetailRequest) -> bool {
    left.cluster_id == right.cluster_id
        && left.session_id == right.session_id
        && left.read_id == right.read_id
        && left.view == right.view
        && left.name == right.name
        && left.namespace == right.namespace
        && left.version == right.version
}
pub(super) fn accepts_read(state: &InspectorState, request: &HelmDetailRequest) -> bool {
    accepts_request(
        state.read_id,
        state.target.as_ref(),
        state.helm_read.as_ref().map(|read| &read.request),
        request,
    )
}
fn accepts_request(
    read_id: u64,
    target: Option<&InspectorTarget>,
    active: Option<&HelmDetailRequest>,
    request: &HelmDetailRequest,
) -> bool {
    read_id == request.read_id
        && active.is_some_and(|active| same_request(active, request))
        && target
            == Some(&InspectorTarget::Helm {
                view: request.view,
                name: request.name.clone(),
                namespace: request.namespace.clone(),
                version: request.version.clone(),
            })
}

fn available(value: Option<&str>) -> &str {
    value
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("—")
}
// Match the inventory's conservative plain x.y.z suffix rule; prereleases and
// unfamiliar labels stay intact rather than claiming an inferred chart version.
fn chart_parts(label: &str) -> (&str, &str) {
    if let Some((name, version)) = label.rsplit_once('-') {
        let parts: Vec<_> = version.split('.').collect();
        if !name.is_empty()
            && parts.len() == 3
            && parts.iter().all(|part| {
                !part.is_empty()
                    && part.bytes().all(|byte| byte.is_ascii_digit())
                    && (part.len() == 1 || !part.starts_with('0'))
            })
        {
            return (name, version);
        }
    }
    (available(Some(label)), "—")
}
fn latest_returned(detail: &HelmDetail) -> Option<&HelmHistory> {
    detail
        .history
        .iter()
        .max_by_key(|revision| revision.revision)
}
fn history_label(detail: &HelmDetail) -> String {
    format!(
        "Revision history ({}{})",
        detail.history.len(),
        if detail.truncated {
            " shown · partial"
        } else {
            ""
        }
    )
}
fn install_version(version: Option<&str>) -> Option<&str> {
    version.filter(|value| {
        !value.is_empty()
            && value.len() <= 128
            && value.as_bytes()[0].is_ascii_alphanumeric()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b".+_-".contains(&byte))
    })
}
fn normalized_updated(value: &str) -> Option<String> {
    if value.trim().is_empty() {
        return None;
    }
    if let Ok(time) = value.parse::<jiff::Timestamp>() {
        return Some(time.to_string());
    }
    let fields: Vec<_> = value.split_whitespace().collect();
    if fields.len() == 4 {
        let input = format!("{} {} {}", fields[0], fields[1], fields[2]);
        if let Ok(time) = jiff::Timestamp::strptime("%Y-%m-%d %H:%M:%S%.f %z", input) {
            return Some(time.to_string());
        }
    }
    None
}
fn surface(cx: &App) -> Div {
    div()
        .w_full()
        .px_3()
        .border_1()
        .border_color(cx.theme().border)
        .rounded(px(6.))
}
fn status(value: &str) -> crate::badge::Badge {
    let badge = if value.starts_with("pending-") || value == "uninstalling" {
        crate::badge::Badge::warning().child(value.to_owned())
    } else {
        crate::badge::Badge::for_resource_status(available(Some(value)))
    };
    badge.small()
}
fn cell(value: impl Into<SharedString>, width: f32) -> Div {
    div()
        .w(px(width))
        .flex_shrink_0()
        .text_xs()
        .child(selectable_text(value.into()))
}
impl IceonApp {
    pub(super) fn render_helm_inspector(
        &self,
        helm: &HelmDetail,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        use super::deployment_inspector::{field, section, text_field};
        let timezone = &self.preferences_draft.app.timezone;
        let mut content = v_flex().w_full().gap_3();
        if helm.view == HelmView::Charts {
            let name = helm.name.clone();
            let version = install_version(helm.version.as_deref()).map(str::to_owned);
            let enabled = version.is_some();
            content = content
                .child(
                    surface(cx)
                        .child(text_field("Chart", helm.name.clone(), cx))
                        .child(text_field(
                            "Version",
                            available(helm.version.as_deref()).to_owned(),
                            cx,
                        ))
                        .child(
                            div().py_3().child(
                                Button::new("insp-helm-install-chart-btn")
                                    .small()
                                    .primary()
                                    .label("Configure Install")
                                    .disabled(!enabled)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let Some(version) = version.clone() else {
                                            return;
                                        };
                                        this.helm_install_chart = Some(name.clone());
                                        this.helm_install_version = Some(version);
                                        this.helm_install_message = None;
                                        cx.notify();
                                    })),
                            ),
                        ),
                )
                .child(
                    surface(cx).children(
                        helm.facts
                            .iter()
                            .map(|fact| text_field(&fact.label, fact.value.clone(), cx)),
                    ),
                );
        } else {
            let revision = latest_returned(helm);
            let (chart, version) =
                chart_parts(revision.map_or("", |revision| revision.chart.as_str()));
            let updated = revision.and_then(|revision| normalized_updated(&revision.updated));
            content = content.child(
                surface(cx)
                    .child(text_field("Release", helm.name.clone(), cx))
                    .child(text_field(
                        "Namespace",
                        available(helm.namespace.as_deref()).to_owned(),
                        cx,
                    ))
                    .child(text_field("Chart", chart.to_owned(), cx))
                    .child(text_field("Version", version.to_owned(), cx))
                    .child(text_field(
                        "App version",
                        available(revision.map(|revision| revision.app_version.as_str()))
                            .to_owned(),
                        cx,
                    ))
                    .child(field(
                        "Status",
                        status(revision.map_or("—", |revision| revision.status.as_str())),
                        cx,
                    ))
                    .child(text_field(
                        "Revision",
                        revision
                            .map_or_else(|| "—".into(), |revision| revision.revision.to_string()),
                        cx,
                    ))
                    .child(field(
                        "Updated",
                        crate::age_label::age_label(
                            "helm-inspector-updated",
                            updated.as_deref(),
                            timezone,
                        ),
                        cx,
                    )),
            );
            let mut revisions = helm.history.iter().collect::<Vec<_>>();
            revisions.sort_by_key(|revision| std::cmp::Reverse(revision.revision));
            content = content
                .child(section(&history_label(helm), cx))
                .when(revisions.is_empty(), |el| {
                    el.child(selectable_text("No revision metadata returned."))
                })
                .child(
                    div()
                        .id("helm-revision-history")
                        .overflow_x_scrollbar()
                        .child(
                            v_flex()
                                .min_w(px(670.))
                                .child(
                                    h_flex()
                                        .py_2()
                                        .border_b_1()
                                        .border_color(cx.theme().border)
                                        .bg(cx.theme().table_head)
                                        .child(cell("Revision", 65.))
                                        .child(cell("Chart", 200.))
                                        .child(cell("App version", 110.))
                                        .child(cell("Status", 155.))
                                        .child(cell("Updated", 130.)),
                                )
                                .children(revisions.iter().map(|revision| {
                                    let updated = normalized_updated(&revision.updated);
                                    h_flex()
                                        .py_2()
                                        .items_center()
                                        .border_b_1()
                                        .border_color(cx.theme().border)
                                        .child(cell(revision.revision.to_string(), 65.))
                                        .child(cell(revision.chart.clone(), 200.))
                                        .child(cell(
                                            available(Some(&revision.app_version)).to_owned(),
                                            110.,
                                        ))
                                        .child(
                                            div()
                                                .w(px(155.))
                                                .flex_shrink_0()
                                                .child(status(&revision.status)),
                                        )
                                        .child(div().w(px(130.)).flex_shrink_0().text_xs().child(
                                            crate::age_label::age_label(
                                                SharedString::from(format!(
                                                    "helm-revision-{}",
                                                    revision.revision
                                                )),
                                                updated.as_deref(),
                                                timezone,
                                            ),
                                        ))
                                })),
                        ),
                );
        }
        content.when(helm.truncated, |el| el.child(div().text_xs().text_color(cx.theme().muted_foreground)
            .child(selectable_text("Partial metadata snapshot. Some fields or revisions may be omitted; summary uses the highest returned revision."))))
            .children([
                ("Values", "Values can contain credentials and are not fetched or editable here."),
                ("Notes", "Chart-generated notes can contain sensitive information and are not fetched."),
                ("Resources", "Rendered manifests can contain secret data and are not fetched or listed here."),
            ].into_iter().map(|(title, explanation)| surface(cx).py_3()
                .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(title))
                .child(div().mt_2().text_xs().text_color(cx.theme().muted_foreground)
                    .child(selectable_text(format!("Not exposed by safe Helm projection. {explanation}"))))))
    }
}

#[cfg(test)]
#[path = "helm_inspector_probe.rs"]
mod probe;
