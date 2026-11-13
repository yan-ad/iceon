//! Shared workload detail presentation and guarded Deployment Scale dialog.
use super::*;
use gpui_kit::component::input::{InputEvent, InputState};
use iceon_core::connection::{detail_collections::CollectionItem, detail_metadata::MetadataEntry};

pub(super) fn fact<'a>(detail: &'a ResourceDetail, label: &str) -> Option<&'a str> {
    detail
        .sections
        .iter()
        .flat_map(|section| &section.facts)
        .find(|fact| fact.label == label)
        .map(|fact| fact.value.as_str())
}

fn conditions_message(conditions: Option<&str>) -> Option<&'static str> {
    match conditions {
        None => Some("Conditions unavailable"),
        Some("") => Some("No true conditions reported"),
        Some(_) => None,
    }
}
fn value(detail: &ResourceDetail, key: &str) -> String {
    detail
        .item
        .columns
        .get(key)
        .cloned()
        .unwrap_or_else(|| "—".into())
}
fn replicas(detail: &ResourceDetail) -> String {
    format!(
        "{} desired, {} updated, {} total, {} available, {} unavailable",
        value(detail, "desired"),
        value(detail, "updated"),
        value(detail, "replicas"),
        value(detail, "available"),
        fact(detail, "Unavailable replicas").unwrap_or("—")
    )
}
fn revision_order(items: &[CollectionItem]) -> Vec<CollectionItem> {
    let mut items = items.to_vec();
    items.sort_by(|a, b| {
        let revision = |item: &CollectionItem| {
            item.columns
                .get("revision")
                .and_then(|value| value.parse::<u64>().ok())
        };
        revision(b)
            .cmp(&revision(a))
            .then_with(|| a.reference.name.cmp(&b.reference.name))
    });
    items
}
fn state_message(
    kind: CollectionKind,
    collection: Option<&ResourceCollection>,
) -> Option<&'static str> {
    let revisions = kind == CollectionKind::ReplicaSets;
    match collection.map(|collection| collection.state) {
        None => Some("This collection has not been loaded."),
        Some(CollectionState::Denied) => {
            Some("Access denied. Your permissions do not allow this collection to be listed.")
        }
        Some(CollectionState::Unavailable) => {
            Some("This collection is unavailable. Refresh to try again.")
        }
        Some(CollectionState::Partial) => {
            Some("Partial snapshot: more records may exist than are shown here.")
        }
        Some(CollectionState::Complete) if collection.is_some_and(|c| c.items.is_empty()) => {
            Some(if revisions {
                "No owned ReplicaSets found."
            } else {
                "No recent events recorded for this Deployment."
            })
        }
        _ => None,
    }
}
pub(super) fn field(label: impl Into<SharedString>, content: impl IntoElement, cx: &App) -> Div {
    div()
        .w_full()
        .flex()
        .items_start()
        .py_3()
        .gap_3()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .w(px(112.))
                .flex_shrink_0()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(label.into()),
        )
        .child(div().flex_1().min_w_0().text_sm().child(content))
}
pub(super) fn text_field(label: &str, text: impl Into<SharedString>, cx: &App) -> Div {
    field(label.to_owned(), selectable_text(text.into()), cx)
}
pub(super) fn section(title: &str, cx: &App) -> Div {
    div()
        .w_full()
        .px_3()
        .py_3()
        .mt_4()
        .bg(cx.theme().secondary)
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .child(title.to_owned())
}
fn metadata(entries: &[MetadataEntry], cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
        .items_start()
        .gap_2()
        .w_full()
        .when(entries.is_empty(), |el| el.child("—"))
        .children(entries.iter().map(|entry| {
            div()
                .max_w_full()
                .px_2()
                .py_1()
                .rounded(px(3.))
                .bg(cx.theme().secondary)
                .child(selectable_text(format!("{}={}", entry.key, entry.value)))
        }))
}
fn valid_scale(reference: &ResourceReference, version: &str) -> bool {
    reference.kind == ResourceKind::Deployments
        && !reference.name.trim().is_empty()
        && reference
            .namespace
            .as_deref()
            .is_some_and(|v| !v.trim().is_empty())
        && reference
            .uid
            .as_deref()
            .is_some_and(|v| !v.trim().is_empty())
        && !version.trim().is_empty()
}
fn parse_replicas(value: &str) -> Option<u32> {
    let value = value.trim();
    (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| value.parse().ok())
        .flatten()
}

fn current_scale_lease(expected: &(String, String), current: Option<(&str, &str, &str)>) -> bool {
    current.is_some_and(|(cluster, session, status)| {
        status == "connected" && cluster == expected.0 && session == expected.1
    })
}

impl IceonApp {
    pub(crate) fn open_deployment_scale(
        &mut self,
        reference: ResourceReference,
        resource_version: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !valid_scale(&reference, &resource_version)
            || !self.can_workload_action(&reference, WorkloadAction::Scale)
        {
            return;
        }
        let Some(session) = self
            .cluster_session
            .as_ref()
            .filter(|session| session.status == "connected")
        else {
            return;
        };
        let lease = (session.cluster_id.clone(), session.session_id.clone());
        let initial = self
            .inspector
            .detail
            .as_ref()
            .filter(|d| d.reference == reference)
            .and_then(|detail| detail.item.columns.get("desired"))
            .cloned()
            .or_else(|| {
                self.resource_page
                    .as_ref()
                    .and_then(|page| {
                        page.items.iter().find(|item| {
                            item.name == reference.name
                                && item.namespace == reference.namespace
                                && item.uid == reference.uid
                        })
                    })
                    .and_then(|item| item.columns.get("desired"))
                    .cloned()
            })
            .unwrap_or_default();
        let owner = cx.entity().downgrade();
        let dialog = cx.new(|cx| {
            let input = cx
                .new(|cx| InputState::new(window, cx).placeholder("Desired replicas (0 or more)"));
            input.update(cx, |input, cx| input.set_value(initial, window, cx));
            cx.subscribe(&input, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            })
            .detach();
            DeploymentScale {
                owner,
                reference,
                resource_version,
                lease,
                input,
                closed: false,
            }
        });
        let weak = dialog.downgrade();
        window.open_dialog(cx, move |modal, _, _| {
            let weak = weak.clone();
            modal
                .title("Scale Deployment")
                .width(px(420.))
                .child(dialog.clone())
                .on_close(move |_, _, cx| {
                    let weak = weak.clone();
                    cx.defer(move |cx| {
                        let _ = weak.update(cx, |dialog, _| dialog.closed = true);
                    });
                })
        });
    }

    pub(super) fn render_workload_actions(
        &self,
        detail: &ResourceDetail,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let reference = detail.reference.clone();
        let version = detail.item.resource_version.clone().unwrap_or_default();
        let deployment = reference.kind == ResourceKind::Deployments;
        let kind_name = if deployment {
            "Deployment"
        } else {
            "DaemonSet"
        };
        let identity = matches!(
            reference.kind,
            ResourceKind::Deployments | ResourceKind::DaemonSets
        ) && !reference.name.trim().is_empty()
            && reference
                .namespace
                .as_deref()
                .is_some_and(|v| !v.trim().is_empty())
            && reference
                .uid
                .as_deref()
                .is_some_and(|v| !v.trim().is_empty())
            && !version.trim().is_empty();
        let scale_reference = reference.clone();
        let scale_version = version.clone();
        let ink = cx.theme().foreground;
        h_flex()
            .gap_1()
            .items_center()
            .when(deployment, |bar| {
                bar.child(
                    Button::new("deployment-scale")
                        .xsmall()
                        .ghost()
                        .icon(
                            gpui_kit::component::Icon::new(gpui_kit_assets::IconName::ArrowUpDown)
                                .text_color(ink),
                        )
                        .tooltip("Scale Deployment")
                        .accessibility_label("Scale Deployment")
                        .disabled(
                            !identity
                                || !self.can_workload_action(&reference, WorkloadAction::Scale),
                        )
                        .on_click(cx.listener(move |app, _, window, cx| {
                            app.open_deployment_scale(
                                scale_reference.clone(),
                                scale_version.clone(),
                                window,
                                cx,
                            )
                        })),
                )
            })
            .child(
                Button::new("deployment-restart")
                    .xsmall()
                    .ghost()
                    .icon(crate::action_icons::refresh().text_color(ink))
                    .tooltip(format!("Restart {kind_name}"))
                    .accessibility_label(format!("Restart {kind_name}"))
                    .disabled(
                        !identity || !self.can_workload_action(&reference, WorkloadAction::Restart),
                    )
                    .on_click(cx.listener(|app, _, window, cx| {
                        app.run_workload_action(WorkloadAction::Restart, window, cx)
                    })),
            )
            .child(
                Button::new("deployment-edit")
                    .xsmall()
                    .ghost()
                    .icon(
                        gpui_kit::component::Icon::new(gpui_kit_assets::IconName::Pencil)
                            .text_color(ink),
                    )
                    .tooltip(format!("Edit {kind_name} YAML"))
                    .accessibility_label(format!("Edit {kind_name} YAML"))
                    .disabled(
                        !identity
                            || !self.can_reference_action(&reference, PermissionAction::Patch),
                    )
                    .on_click(
                        cx.listener(|app, _, window, cx| app.edit_inspected_yaml(window, cx)),
                    ),
            )
            .child(
                Button::new("deployment-delete")
                    .xsmall()
                    .ghost()
                    .icon(crate::action_icons::delete().text_color(ink))
                    .tooltip(format!("Delete {kind_name}"))
                    .accessibility_label(format!("Delete {kind_name}"))
                    .disabled(
                        !identity
                            || !self.can_reference_action(&reference, PermissionAction::Delete),
                    )
                    .on_click(
                        cx.listener(|app, _, window, cx| app.delete_inspected_resource(window, cx)),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_workload_inspector(
        &self,
        detail: &ResourceDetail,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let kind_name = if detail.reference.kind == ResourceKind::DaemonSets {
            "DaemonSets"
        } else {
            "Deployments"
        };
        let timezone = &self.preferences_draft.app.timezone;
        let namespace = detail.reference.namespace.clone();
        let created = detail
            .item
            .created_at
            .as_deref()
            .map(|timestamp| {
                format!(
                    "{} ago ({})",
                    crate::navigation::format_age_at(Some(timestamp), jiff::Timestamp::now()),
                    crate::preference_runtime::timestamp(timestamp, timezone)
                )
            })
            .unwrap_or_else(|| "—".into());
        let mut body = v_flex().w_full().min_w_0()
            .child(div().w_full().p_3().mb_3().rounded(px(4.)).bg(cx.theme().secondary).text_sm()
                .text_color(cx.theme().muted_foreground).child(format!("Historical CPU, memory, network, and filesystem metrics are not configured for {kind_name}.")))
            .child(text_field("Created", created, cx))
            .child(text_field("Name", detail.reference.name.clone(), cx))
            .child(field("Namespace", div().when_some(namespace, |el, namespace| {
                let target = ResourceReference { kind: ResourceKind::Namespaces, name: namespace.clone(), namespace: None, uid: None };
                el.child(Button::new("deployment-namespace").xsmall().ghost().label(namespace)
                    .text_color(cx.theme().link).disabled(!self.can_reference_action(&target, PermissionAction::Get))
                    .on_click(cx.listener(move |app, _, window, cx| app.open_resource_inspector(target.clone(), window, cx))))
            }), cx));
        if let Some(meta) = &detail.metadata {
            body = body
                .child(field("Labels", metadata(&meta.labels, cx), cx))
                .child(field("Annotations", metadata(&meta.annotations, cx), cx))
                .child(field(
                    "Managed fields",
                    v_flex()
                        .w_full()
                        .gap_2()
                        .when(meta.managed_fields.is_empty(), |el| el.child("—"))
                        .children(meta.managed_fields.iter().enumerate().map(
                            |(index, managed)| {
                                let key = format!("deployment-managed-{index}");
                                let expanded = self.inspector.pod_expanded.contains(&key);
                                let summary = format!(
                                    "{}: {}{}",
                                    managed.manager,
                                    managed.operation,
                                    managed
                                        .subresource
                                        .as_ref()
                                        .map(|s| format!(" ({s})"))
                                        .unwrap_or_default()
                                );
                                v_flex()
                                    .w_full()
                                    .gap_2()
                                    .child(
                                        h_flex()
                                            .w_full()
                                            .gap_2()
                                            .justify_between()
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .child(selectable_text(summary)),
                                            )
                                            .child(
                                                Button::new(SharedString::from(key.clone()))
                                                    .xsmall()
                                                    .ghost()
                                                    .label(if expanded { "Hide" } else { "Show" })
                                                    .accessibility_label(format!(
                                                        "{} managed fields for {}",
                                                        if expanded { "Hide" } else { "Show" },
                                                        managed.manager
                                                    ))
                                                    .on_click(cx.listener(move |app, _, _, cx| {
                                                        if !app.inspector.pod_expanded.remove(&key)
                                                        {
                                                            app.inspector
                                                                .pod_expanded
                                                                .insert(key.clone());
                                                        }
                                                        cx.notify();
                                                    })),
                                            ),
                                    )
                                    .when(expanded, |el| {
                                        el.child(
                                            div()
                                                .text_xs()
                                                .p_2()
                                                .bg(cx.theme().secondary)
                                                .child(selectable_text(managed.fields.clone())),
                                        )
                                    })
                            },
                        )),
                    cx,
                ));
            if meta.partial {
                body = body.child(
                    div()
                        .py_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Some metadata was omitted for safety or size limits."),
                );
            }
        } else {
            body = body.child(text_field("Metadata", "Unavailable", cx));
        }
        if detail.reference.kind == ResourceKind::DaemonSets {
            let body = self.render_daemon_set_details(detail, body, cx);
            return self.render_workload_events(detail, body, cx);
        }
        body = body
            .child(text_field("Replicas", replicas(detail), cx))
            .child(field(
                "Selector",
                v_flex().gap_1().children(
                    detail
                        .sections
                        .iter()
                        .filter(|section| section.title.starts_with("Selector"))
                        .flat_map(|section| &section.facts)
                        .map(|fact| {
                            div().child(selectable_text(format!("{}: {}", fact.label, fact.value)))
                        }),
                ),
                cx,
            ))
            .child(text_field(
                "Strategy type",
                fact(detail, "Strategy").unwrap_or("—").to_owned(),
                cx,
            ))
            .child(field(
                "Conditions",
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .when_some(
                        conditions_message(
                            detail.item.columns.get("conditions").map(String::as_str),
                        ),
                        |el, message| el.child(message),
                    )
                    .children(
                        detail
                            .item
                            .columns
                            .get("conditions")
                            .into_iter()
                            .flat_map(|conditions| conditions.split(','))
                            .filter(|s| !s.is_empty())
                            .map(|condition| match condition {
                                "Available" => Tag::success().small().child(condition.to_owned()),
                                "Progressing" => Tag::info().small().child(condition.to_owned()),
                                "ReplicaFailure" => {
                                    Tag::danger().small().child(condition.to_owned())
                                }
                                _ => Tag::secondary().small().child(condition.to_owned()),
                            }),
                    ),
                cx,
            ));
        for condition in detail
            .sections
            .iter()
            .filter(|s| s.title.starts_with("Conditions"))
            .flat_map(|s| &s.facts)
        {
            body = body.child(text_field(&condition.label, condition.value.clone(), cx));
        }
        body = body.child(section("Deploy revisions", cx));
        let collection = self.inspector.collections.iter().find(|collection| {
            collection.collection == CollectionKind::ReplicaSets
                && collection.reference == detail.reference
        });
        if let Some(message) = state_message(CollectionKind::ReplicaSets, collection) {
            body = body.child(
                div()
                    .p_3()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(message),
            );
        }
        if let Some(collection) = collection {
            body = body.child(
                h_flex()
                    .w_full()
                    .py_3()
                    .gap_2()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(div().flex_1().min_w_0().child("Name / Namespace"))
                    .child(div().w(px(46.)).child("Rev."))
                    .child(div().w(px(42.)).child("Pods"))
                    .child(div().w(px(60.)).child("Age")),
            );
            for (index, item) in revision_order(&collection.items).into_iter().enumerate() {
                let target = item.reference.clone();
                body = body.child(
                    h_flex()
                        .w_full()
                        .py_2()
                        .gap_2()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .items_center()
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    Button::new(SharedString::from(format!(
                                        "deployment-revision-{index}"
                                    )))
                                    .xsmall()
                                    .ghost()
                                    .label(item.reference.name.clone())
                                    .text_color(cx.theme().link)
                                    .disabled(
                                        !self.can_reference_action(&target, PermissionAction::Get),
                                    )
                                    .on_click(cx.listener(move |app, _, window, cx| {
                                        app.open_resource_inspector(target.clone(), window, cx)
                                    })),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(
                                            item.reference.namespace.unwrap_or_else(|| "—".into()),
                                        ),
                                ),
                        )
                        .child(
                            div().w(px(46.)).text_xs().child(
                                item.columns
                                    .get("revision")
                                    .cloned()
                                    .unwrap_or_else(|| "—".into()),
                            ),
                        )
                        .child(
                            div().w(px(42.)).text_xs().child(
                                item.columns
                                    .get("replicas")
                                    .cloned()
                                    .unwrap_or_else(|| "—".into()),
                            ),
                        )
                        .child(
                            div()
                                .w(px(60.))
                                .text_xs()
                                .child(crate::age_label::age_label(
                                    SharedString::from(format!("deployment-revision-age-{index}")),
                                    item.created_at.as_deref(),
                                    timezone,
                                )),
                        ),
                );
            }
        }
        self.render_workload_events(detail, body, cx)
    }

    pub(super) fn render_workload_events(
        &self,
        detail: &ResourceDetail,
        mut body: Div,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let timezone = &self.preferences_draft.app.timezone;
        body = body.child(section("Events", cx));
        let collection = self.inspector.collections.iter().find(|collection| {
            collection.collection == CollectionKind::Events
                && collection.reference == detail.reference
        });
        if let Some(message) = state_message(CollectionKind::Events, collection) {
            let message = if detail.reference.kind == ResourceKind::DaemonSets {
                message.replace("Deployment", "DaemonSet")
            } else {
                message.to_owned()
            };
            body = body.child(
                div()
                    .p_3()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(message),
            );
        }
        if let Some(collection) = collection {
            for (index, item) in collection.items.iter().enumerate() {
                let last_seen = item
                    .columns
                    .get("lastSeen")
                    .map(|timestamp| {
                        format!(
                            "{} ago ({})",
                            crate::navigation::format_age_at(
                                Some(timestamp),
                                jiff::Timestamp::now()
                            ),
                            crate::preference_runtime::timestamp(timestamp, timezone)
                        )
                    })
                    .unwrap_or_else(|| "—".into());
                body = body.child(
                    v_flex()
                        .w_full()
                        .pt_4()
                        .child(
                            div().text_sm().font_weight(FontWeight::SEMIBOLD).child(
                                selectable_text(
                                    item.columns
                                        .get("message")
                                        .or_else(|| item.columns.get("reason"))
                                        .cloned()
                                        .unwrap_or_else(|| "Event".into()),
                                ),
                            ),
                        )
                        .child(text_field(
                            "Source",
                            item.columns
                                .get("source")
                                .cloned()
                                .unwrap_or_else(|| "—".into()),
                            cx,
                        ))
                        .child(text_field(
                            "Count",
                            item.columns
                                .get("count")
                                .cloned()
                                .unwrap_or_else(|| "—".into()),
                            cx,
                        ))
                        .child(text_field("Last seen", last_seen, cx))
                        .child(
                            Button::new(SharedString::from(format!("deployment-event-{index}")))
                                .xsmall()
                                .ghost()
                                .label("Inspect event")
                                .disabled(
                                    !self.can_reference_action(
                                        &item.reference,
                                        PermissionAction::Get,
                                    ),
                                )
                                .on_click(cx.listener({
                                    let target = item.reference.clone();
                                    move |app, _, window, cx| {
                                        app.open_resource_inspector(target.clone(), window, cx)
                                    }
                                })),
                        ),
                );
            }
        }
        body.into_any_element()
    }
}

struct DeploymentScale {
    owner: WeakEntity<IceonApp>,
    reference: ResourceReference,
    resource_version: String,
    lease: (String, String),
    input: Entity<InputState>,
    closed: bool,
}
impl Render for DeploymentScale {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let replicas = parse_replicas(self.input.read(cx).value().as_str());
        let available = !self.closed
            && self.owner.upgrade().is_some_and(|owner| {
                let app = owner.read(cx);
                current_scale_lease(
                    &self.lease,
                    app.cluster_session
                        .as_ref()
                        .map(|s| (s.cluster_id.as_str(), s.session_id.as_str(), s.status)),
                ) && app.can_workload_action(&self.reference, WorkloadAction::Scale)
            });
        v_flex()
            .w_full()
            .gap_3()
            .child(selectable_text(format!(
                "Desired replicas for {}",
                self.reference.name
            )))
            .child(Input::new(&self.input))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(if replicas.is_some() {
                        "Scaling changes the number of running replicas. Zero stops all replicas."
                    } else {
                        "Enter a whole number from 0 to 4294967295."
                    }),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("cancel-deployment-scale")
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("apply-deployment-scale")
                            .primary()
                            .label("Apply scale")
                            .disabled(!available || replicas.is_none())
                            .on_click(cx.listener(move |dialog, _, window, cx| {
                                if dialog.closed {
                                    return;
                                }
                                let Some(replicas) =
                                    parse_replicas(dialog.input.read(cx).value().as_str())
                                else {
                                    return;
                                };
                                let _ = dialog.owner.update(cx, |app, cx| {
                                    if !current_scale_lease(
                                        &dialog.lease,
                                        app.cluster_session.as_ref().map(|s| {
                                            (s.cluster_id.as_str(), s.session_id.as_str(), s.status)
                                        }),
                                    ) {
                                        return;
                                    }
                                    app.run_workload_action_for(
                                        dialog.reference.clone(),
                                        dialog.resource_version.clone(),
                                        WorkloadAction::Scale,
                                        Some(replicas),
                                        window,
                                        cx,
                                    );
                                });
                                dialog.closed = true;
                                window.close_dialog(cx);
                            })),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_replicas, revision_order, state_message, valid_scale};
    use iceon_core::connection::{
        ResourceKind,
        detail_collections::{CollectionItem, CollectionKind, CollectionState, ResourceCollection},
        details::ResourceReference,
    };
    #[test]
    fn scale_dialog_rejects_replaced_disconnected_or_missing_lease() {
        let lease = ("cluster".to_owned(), "session".to_owned());
        assert!(super::current_scale_lease(
            &lease,
            Some(("cluster", "session", "connected"))
        ));
        for current in [
            None,
            Some(("cluster", "new-session", "connected")),
            Some(("other", "session", "connected")),
            Some(("cluster", "session", "error")),
        ] {
            assert!(!super::current_scale_lease(&lease, current));
        }
        assert_eq!(
            super::conditions_message(None),
            Some("Conditions unavailable")
        );
        assert_eq!(
            super::conditions_message(Some("")),
            Some("No true conditions reported")
        );
        assert_eq!(super::conditions_message(Some("Available")), None);
    }
    #[test]
    fn scale_validation_preserves_zero_and_rejects_invalid_input_and_identity() {
        assert_eq!(parse_replicas("0"), Some(0));
        assert_eq!(parse_replicas(" 8 "), Some(8));
        for value in ["", "-1", "+1", "1.5", "4294967296", "ten"] {
            assert_eq!(parse_replicas(value), None);
        }
        let mut reference = ResourceReference {
            kind: ResourceKind::Deployments,
            name: "offline".into(),
            namespace: Some("fixture".into()),
            uid: Some("uid-1".into()),
        };
        assert!(valid_scale(&reference, "17"));
        assert!(!valid_scale(&reference, ""));
        reference.uid = None;
        assert!(!valid_scale(&reference, "17"));
    }
    #[test]
    fn missing_and_denied_collections_never_claim_empty_complete_history() {
        assert!(
            state_message(CollectionKind::ReplicaSets, None)
                .unwrap()
                .contains("not been loaded")
        );
        let mut collection = ResourceCollection {
            cluster_id: "fixture".into(),
            session_id: "session".into(),
            read_id: 1,
            reference: ResourceReference {
                kind: ResourceKind::Deployments,
                name: "offline".into(),
                namespace: Some("fixture".into()),
                uid: Some("uid".into()),
            },
            collection: CollectionKind::ReplicaSets,
            state: CollectionState::Denied,
            items: vec![],
        };
        assert!(
            state_message(CollectionKind::ReplicaSets, Some(&collection))
                .unwrap()
                .contains("Access denied")
        );
        collection.state = CollectionState::Partial;
        assert!(
            state_message(CollectionKind::ReplicaSets, Some(&collection))
                .unwrap()
                .contains("Partial")
        );
        collection.state = CollectionState::Complete;
        assert_eq!(
            state_message(CollectionKind::ReplicaSets, Some(&collection)),
            Some("No owned ReplicaSets found.")
        );
    }
    #[test]
    fn revision_history_sorts_numerically_and_missing_revisions_last() {
        let item = |revision: Option<&str>| CollectionItem {
            reference: ResourceReference {
                kind: ResourceKind::ReplicaSets,
                name: format!("rs-{}", revision.unwrap_or("unknown")),
                namespace: Some("fixture".into()),
                uid: Some("uid".into()),
            },
            created_at: None,
            status: "Ready".into(),
            columns: revision
                .map(|value| [("revision".into(), value.into())].into_iter().collect())
                .unwrap_or_default(),
        };
        let sorted = revision_order(&[item(Some("2")), item(None), item(Some("10"))]);
        assert_eq!(
            sorted[0].columns.get("revision").map(String::as_str),
            Some("10")
        );
        assert_eq!(
            sorted[1].columns.get("revision").map(String::as_str),
            Some("2")
        );
        assert!(!sorted[2].columns.contains_key("revision"));
    }
}
