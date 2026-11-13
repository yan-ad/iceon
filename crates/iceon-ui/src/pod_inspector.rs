//! Pod-only continuous inspector. Only reviewed core projections are rendered;
//! raw YAML, container environment, commands and arguments never enter this view.
use super::*;
use gpui_kit::component::collapsible::Collapsible;

/// A separate immutable buffer: ownership paths must never share the editable
/// YAML dock's state or acquire mutation controls.
#[derive(IntoElement)]
struct OwnershipPaths {
    id: SharedString,
    paths: String,
}
impl RenderOnce for OwnershipPaths {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        use gpui_kit::component::input::{Editor, EditorState};
        let paths = self.paths;
        let editor = window.use_keyed_state(self.id, cx, |window, cx| {
            EditorState::new(window, cx)
                .language("json")
                .line_number(true)
                .default_value(paths)
        });
        Editor::new(&editor)
            .readonly(true)
            .h(px(160.))
            .aria_label("Managed field ownership paths, read only")
    }
}

const LABEL_FRACTION: f32 = 0.30;
const VALUE_FRACTION: f32 = 1.0 - LABEL_FRACTION;

fn row(label: impl Into<SharedString>, value: impl IntoElement, cx: &App) -> AnyElement {
    h_flex()
        .w_full()
        .items_start()
        .py_2()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .w(relative(LABEL_FRACTION))
                .flex_shrink_0()
                .pr_3()
                .text_color(cx.theme().muted_foreground)
                .child(selectable_text(label)),
        )
        .child(div().w(relative(VALUE_FRACTION)).min_w_0().child(value))
        .into_any_element()
}

fn text_row(
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    cx: &App,
) -> AnyElement {
    row(label, selectable_text(value), cx)
}

fn chips(values: impl IntoIterator<Item = String>) -> AnyElement {
    let values: Vec<_> = values.into_iter().collect();
    h_flex()
        .gap_1()
        .flex_wrap()
        .children(
            values
                .iter()
                .map(|v| Tag::secondary().small().child(selectable_text(v.clone()))),
        )
        .when(values.is_empty(), |row| row.child(selectable_text("—")))
        .into_any_element()
}

fn heading(title: impl Into<SharedString>) -> AnyElement {
    div()
        .pt_4()
        .pb_1()
        .font_weight(FontWeight::SEMIBOLD)
        .child(selectable_text(title))
        .into_any_element()
}

fn absent(value: Option<&str>) -> String {
    value.filter(|v| !v.is_empty()).unwrap_or("—").to_owned()
}

fn event_notice(state: PodEventsState, empty: bool) -> Option<&'static str> {
    match state {
        PodEventsState::Denied => Some("Permission to list pod events was denied."),
        PodEventsState::Unavailable => Some("Pod events are unavailable. Refresh to retry."),
        PodEventsState::Partial => Some("Only a partial event history is available."),
        PodEventsState::Complete if empty => Some("No events recorded for this pod."),
        PodEventsState::Complete => None,
    }
}

impl IceonApp {
    fn pod_disclosure(
        &self,
        key: String,
        title: String,
        content: impl IntoElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.inspector.pod_expanded.contains(&key);
        Collapsible::new()
            .open(open)
            .w_full()
            .child(
                Button::new(SharedString::from(format!("pod-expand-{key}")))
                    .small()
                    .ghost()
                    .icon(if open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .label(title)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.inspector.pod_expanded.remove(&key) {
                            this.inspector.pod_expanded.insert(key.clone());
                        }
                        cx.notify();
                    })),
            )
            .content(content)
            .into_any_element()
    }

    fn pod_reference(
        &self,
        id: String,
        target: ResourceReference,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        Button::new(SharedString::from(id))
            .xsmall()
            .ghost()
            .label(target.name.clone())
            .disabled(!self.can_reference_action(&target, PermissionAction::Get))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.open_resource_inspector(target.clone(), window, cx)
            }))
            .into_any_element()
    }

    pub(super) fn render_pod_inspector(
        &self,
        detail: &ResourceDetail,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut body = v_flex()
            .w_full()
            .text_xs()
            .gap_0()
            .child(
                div()
                    .py_4()
                    .text_color(cx.theme().muted_foreground)
                    .child(selectable_text(
                        "Metrics history is not available. No usage history has been collected.",
                    )),
            )
            .child(text_row("Name", detail.reference.name.clone(), cx));
        if let Some(namespace) = &detail.reference.namespace {
            let target = ResourceReference {
                kind: ResourceKind::Namespaces,
                name: namespace.clone(),
                namespace: None,
                uid: None,
            };
            let link = self.pod_reference("pod-namespace".into(), target, cx);
            body = body.child(row("Namespace", link, cx));
        }
        body = body.child(text_row(
            "Created",
            crate::preference_runtime::timestamp(
                &detail.item.created_at.clone().unwrap_or_default(),
                &self.preferences_draft.app.timezone,
            ),
            cx,
        ));
        body = body.child(text_row(
            "UID",
            absent(
                detail
                    .reference
                    .uid
                    .as_deref()
                    .or(detail.item.uid.as_deref()),
            ),
            cx,
        ));
        if let Some(meta) = &detail.metadata {
            for (label, entries) in [("Labels", &meta.labels), ("Annotations", &meta.annotations)] {
                let chips = h_flex()
                    .w_full()
                    .gap_1()
                    .flex_wrap()
                    .children(entries.iter().map(|entry| {
                        Tag::secondary()
                            .small()
                            .child(selectable_text(format!("{}: {}", entry.key, entry.value)))
                    }))
                    .when(entries.is_empty(), |el| el.child(selectable_text("—")));
                body = body.child(row(label, chips, cx));
            }
            if meta.partial {
                body = body.child(div().py_2().text_color(cx.theme().muted_foreground).child(
                    selectable_text(
                        "Metadata is partial; unsafe or unsupported entries are withheld.",
                    ),
                ));
            }
            if !meta.finalizers.is_empty() {
                body = body.child(text_row("Finalizers", meta.finalizers.join(", "), cx));
            }
            let mut fields = v_flex().w_full();
            for (index, field) in meta.managed_fields.iter().enumerate() {
                // fields is a core-produced JSON array of ownership paths, not raw fieldsV1.
                let content = v_flex()
                    .w_full()
                    .child(text_row("Operation", field.operation.clone(), cx))
                    .child(text_row(
                        "API version",
                        absent(field.api_version.as_deref()),
                        cx,
                    ))
                    .child(text_row(
                        "Updated",
                        field
                            .time
                            .as_ref()
                            .map(|time| {
                                crate::preference_runtime::timestamp(
                                    time,
                                    &self.preferences_draft.app.timezone,
                                )
                            })
                            .unwrap_or_else(|| "—".into()),
                        cx,
                    ))
                    .child(text_row(
                        "Subresource",
                        absent(field.subresource.as_deref()),
                        cx,
                    ))
                    .child(OwnershipPaths {
                        id: format!(
                            "pod-owned-paths-{}-{}-{index}",
                            detail.reference.name,
                            detail.item.resource_version.as_deref().unwrap_or_default()
                        )
                        .into(),
                        paths: field.fields.clone(),
                    });
                fields = fields.child(self.pod_disclosure(
                    format!("managed-{index}"),
                    field.manager.clone(),
                    content,
                    cx,
                ));
            }
            body = body.child(self.pod_disclosure(
                "managed".into(),
                format!("Managed fields ({})", meta.managed_fields.len()),
                fields,
                cx,
            ));
        }

        if let Some(pod) = &self.inspector.pod_detail {
            body = body
                .child(row("Status", status_tag(&pod.phase), cx))
                .child(text_row("QoS class", absent(pod.qos.as_deref()), cx))
                .child(text_row(
                    "Pod IPs",
                    if pod.pod_ips.is_empty() {
                        "—".into()
                    } else {
                        pod.pod_ips.join(", ")
                    },
                    cx,
                ))
                .child(text_row(
                    "Host IPs",
                    if pod.host_ips.is_empty() {
                        "—".into()
                    } else {
                        pod.host_ips.join(", ")
                    },
                    cx,
                ));
            if let Some(timestamp) = &pod.deletion_timestamp {
                body = body.child(text_row(
                    "Terminating since",
                    crate::preference_runtime::timestamp(
                        timestamp,
                        &self.preferences_draft.app.timezone,
                    ),
                    cx,
                ));
            }
            for (label, target) in [
                (
                    "Node",
                    pod.node.as_ref().map(|name| ResourceReference {
                        kind: ResourceKind::Nodes,
                        name: name.clone(),
                        namespace: None,
                        uid: None,
                    }),
                ),
                (
                    "Service account",
                    pod.service_account.as_ref().map(|name| ResourceReference {
                        kind: ResourceKind::ServiceAccounts,
                        name: name.clone(),
                        namespace: Some(pod.namespace.clone()),
                        uid: None,
                    }),
                ),
            ] {
                if let Some(target) = target {
                    let link = self.pod_reference(format!("pod-ref-{label}"), target, cx);
                    body = body.child(row(label, link, cx));
                } else {
                    body = body.child(text_row(label, "—", cx));
                }
            }
            body = body.child(text_row(
                "Termination grace period",
                pod.inspection
                    .termination_grace_period_seconds
                    .map(|v| format!("{v}s"))
                    .unwrap_or_else(|| "—".into()),
                cx,
            ));
            body = body.child(row(
                "Node selector",
                chips(
                    pod.inspection
                        .node_selectors
                        .iter()
                        .map(|v| format!("{}={}", v.key, v.value)),
                ),
                cx,
            ));
            let tolerations = v_flex()
                .w_full()
                .children(pod.inspection.tolerations.iter().map(|v| {
                    text_row(
                        absent(v.key.as_deref()),
                        format!(
                            "{} · {} · {} · {}",
                            v.operator,
                            absent(v.value.as_deref()),
                            absent(v.effect),
                            v.toleration_seconds
                                .map(|n| format!("{n}s"))
                                .unwrap_or_else(|| "—".into())
                        ),
                        cx,
                    )
                }));
            body = body.child(self.pod_disclosure(
                "tolerations".into(),
                format!("Tolerations ({})", pod.inspection.tolerations.len()),
                tolerations,
                cx,
            ));
            body = body.child(row(
                "Conditions",
                h_flex()
                    .gap_1()
                    .flex_wrap()
                    .children(pod.conditions.iter().map(|condition| {
                        let tag = if condition.status == "True" {
                            Tag::success()
                        } else {
                            Tag::secondary()
                        };
                        tag.small().child(selectable_text(format!(
                            "{}: {}",
                            condition.kind, condition.status
                        )))
                    })),
                cx,
            ));
        } else {
            body = body.child(div().py_2().text_color(cx.theme().muted_foreground).child(
                selectable_text("Pod runtime details are unavailable. Refresh to retry."),
            ));
        }

        // These sections are reviewed core facts, never arbitrary API object fields.
        for (index, section) in detail.sections.iter().enumerate() {
            let has_pod = self.inspector.pod_detail.is_some();
            if has_pod
                && matches!(
                    section.title.as_str(),
                    "Container images" | "Init container images"
                )
            {
                continue;
            }
            let facts = v_flex().w_full().children(
                section
                    .facts
                    .iter()
                    .filter(|fact| {
                        !has_pod
                            || !matches!(
                                fact.label.as_str(),
                                "Node"
                                    | "Service account"
                                    | "Phase"
                                    | "Pod IP"
                                    | "Host IP"
                                    | "QoS class"
                            )
                    })
                    .map(|fact| text_row(fact.label.clone(), fact.value.clone(), cx)),
            );
            if section.title.to_ascii_lowercase().contains("toleration") {
                body = body.child(self.pod_disclosure(
                    format!("facts-{index}"),
                    section.title.clone(),
                    facts,
                    cx,
                ));
            } else {
                body = body.child(heading(section.title.clone())).child(facts);
            }
        }
        if !detail.related.is_empty() {
            body = body.child(heading("Related resources"));
            for (index, (relation, target)) in detail.related.iter().enumerate() {
                let link = self.pod_reference(format!("pod-related-{index}"), target.clone(), cx);
                body = body.child(row(relation.clone(), link, cx));
            }
        }
        if let Some(pod) = &self.inspector.pod_detail {
            body = body.child(heading(format!("Containers ({})", pod.containers.len())));
            for (index, container) in pod.containers.iter().enumerate() {
                let mut actions = h_flex().gap_1();
                for (label, permission, mode) in [
                    ("Logs", PermissionAction::Logs, PodStreamMode::Logs),
                    ("Shell", PermissionAction::Exec, PodStreamMode::Shell),
                    ("Attach", PermissionAction::Attach, PodStreamMode::Attach),
                ] {
                    let target = PodTarget {
                        namespace: pod.namespace.clone(),
                        name: pod.name.clone(),
                        uid: pod.uid.clone(),
                    };
                    let name = container.name.clone();
                    actions = actions.child(
                        Button::new(SharedString::from(format!("pod-container-{index}-{label}")))
                            .xsmall()
                            .ghost()
                            .icon(match mode {
                                PodStreamMode::Logs => crate::action_icons::logs(),
                                PodStreamMode::Attach => crate::action_icons::attach(),
                                PodStreamMode::Shell => {
                                    gpui_kit::component::Icon::new(IconName::SquareTerminal)
                                }
                            })
                            .tooltip(label)
                            .accessibility_label(label)
                            .disabled(!self.can_reference_action(&detail.reference, permission))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_pod_stream(target.clone(), name.clone(), mode, window, cx)
                            })),
                    );
                }
                body = body
                    .child(
                        h_flex()
                            .pt_3()
                            .pb_1()
                            .justify_between()
                            .flex_wrap()
                            .child(selectable_text(format!(
                                "{} ({})",
                                container.name, container.kind
                            )))
                            .child(actions),
                    )
                    .child(text_row("Image", container.image.clone(), cx))
                    .child(text_row("State", container.state.clone(), cx))
                    .child(text_row(
                        "Ready",
                        if container.ready { "Yes" } else { "No" },
                        cx,
                    ))
                    .child(text_row("Restarts", container.restarts.to_string(), cx));
                let mut ports = h_flex().gap_1().flex_wrap();
                for (port_index, port) in container.ports.iter().enumerate() {
                    let namespace = pod.namespace.clone();
                    let name = pod.name.clone();
                    let uid = pod.uid.clone();
                    let number = port.port;
                    ports = ports.child(
                        Button::new(SharedString::from(format!("pod-port-{index}-{port_index}")))
                            .xsmall()
                            .ghost()
                            .label(format!("{} / {}", number, port.protocol))
                            .tooltip("Forward port")
                            .disabled(
                                port.protocol != "TCP"
                                    || !self.can_reference_action(
                                        &detail.reference,
                                        PermissionAction::PortForward,
                                    ),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.start_pod_forward(
                                    namespace.clone(),
                                    name.clone(),
                                    uid.clone(),
                                    number,
                                    window,
                                    cx,
                                )
                            })),
                    );
                }
                body = body.child(row(
                    "Ports",
                    ports.when(container.ports.is_empty(), |el| {
                        el.child(selectable_text("—"))
                    }),
                    cx,
                ));
                if let Some(facts) = pod
                    .inspection
                    .containers
                    .iter()
                    .find(|v| v.name == container.name && v.kind == container.kind)
                {
                    body = body
                        .child(row(
                            "Requests",
                            chips(
                                facts
                                    .requests
                                    .iter()
                                    .map(|v| format!("{}={}", v.name, v.quantity)),
                            ),
                            cx,
                        ))
                        .child(row(
                            "Limits",
                            chips(
                                facts
                                    .limits
                                    .iter()
                                    .map(|v| format!("{}={}", v.name, v.quantity)),
                            ),
                            cx,
                        ));
                    let mut env = v_flex().gap_1();
                    for entry in &facts.environment {
                        use iceon_core::connection::pods::PodEnvironmentSource as Source;
                        let source = match &entry.source {
                            Source::Redacted => "value hidden".into(),
                            Source::SecretKeyRef { name, key, .. } => {
                                format!(
                                    "Secret {} / {}",
                                    absent(name.as_deref()),
                                    absent(key.as_deref())
                                )
                            }
                            Source::ConfigMapKeyRef { name, key, .. } => {
                                format!(
                                    "ConfigMap {} / {}",
                                    absent(name.as_deref()),
                                    absent(key.as_deref())
                                )
                            }
                            Source::FieldRef { field } => format!("Field {}", absent(*field)),
                            Source::ResourceFieldRef { resource, .. } => {
                                format!("Resource {}", absent(resource.as_deref()))
                            }
                            Source::Unknown => "Source unavailable".into(),
                        };
                        env = env.child(selectable_text(format!("{}: {source}", entry.name)));
                    }
                    for source in &facts.environment_from {
                        env = env.child(selectable_text(format!(
                            "{}: {} (values hidden)",
                            source.kind,
                            absent(source.name.as_deref())
                        )));
                    }
                    body = body.child(row("Environment", env, cx));
                    body = body.child(row(
                        "Mounts",
                        v_flex().gap_1().children(facts.mounts.iter().map(|v| {
                            selectable_text(format!(
                                "{} · from {} ({})",
                                absent(v.mount_path.as_deref()),
                                absent(v.name.as_deref()),
                                if v.read_only {
                                    "read only"
                                } else {
                                    "read/write"
                                }
                            ))
                        })),
                        cx,
                    ));
                    for probe in &facts.probes {
                        body = body.child(row(
                            probe.purpose,
                            chips([
                                format!("{:?}", probe.kind),
                                format!("delay={}s", probe.initial_delay_seconds.unwrap_or(0)),
                                format!("timeout={}s", probe.timeout_seconds.unwrap_or(1)),
                                format!("period={}s", probe.period_seconds.unwrap_or(10)),
                                format!("success={}", probe.success_threshold.unwrap_or(1)),
                                format!("failure={}", probe.failure_threshold.unwrap_or(3)),
                            ]),
                            cx,
                        ));
                    }
                }
            }
            body = body.child(heading(format!(
                "Volumes ({})",
                pod.inspection.volumes.len()
            )));
            for (index, volume) in pod.inspection.volumes.iter().enumerate() {
                body = body.child(heading(absent(volume.name.as_deref())));
                for (source_index, source) in volume.sources.iter().enumerate() {
                    body = body.child(text_row("Kind", source.kind, cx));
                    if let Some(name) = &source.name {
                        let kind = match source.kind {
                            "secret" => Some(ResourceKind::Secrets),
                            "configMap" => Some(ResourceKind::ConfigMaps),
                            "persistentVolumeClaim" => Some(ResourceKind::PersistentVolumeClaims),
                            _ => None,
                        };
                        if let Some(kind) = kind {
                            let link = self.pod_reference(
                                format!("pod-volume-{index}-{source_index}"),
                                ResourceReference {
                                    kind,
                                    name: name.clone(),
                                    namespace: Some(pod.namespace.clone()),
                                    uid: None,
                                },
                                cx,
                            );
                            body = body.child(row("Name", link, cx));
                        } else {
                            body = body.child(text_row("Name", name.clone(), cx));
                        }
                    }
                    if let Some(mode) = source.default_mode {
                        body = body.child(text_row("Default mode", format!("{mode:04o}"), cx));
                    }
                    if let Some(optional) = source.optional {
                        body = body.child(text_row("Optional", optional.to_string(), cx));
                    }
                    if let Some(seconds) = source.expiration_seconds {
                        body = body.child(text_row("Expiration", format!("{seconds}s"), cx));
                    }
                    if let Some(path) = &source.path {
                        body = body.child(text_row("Path", path.clone(), cx));
                    }
                }
            }
            body = body.child(heading(format!("Events ({})", pod.events.len())));
            if let Some(notice) = event_notice(pod.events_state, pod.events.is_empty()) {
                body = body.child(
                    div()
                        .py_2()
                        .text_color(cx.theme().muted_foreground)
                        .child(selectable_text(notice)),
                );
            }
            for event in &pod.events {
                body = body.child(row(
                    event.reason.clone(),
                    v_flex()
                        .gap_1()
                        .child(status_tag(&event.kind))
                        .child(selectable_text(format!("{} occurrences", event.count)))
                        .child(selectable_text(
                            event
                                .last_seen
                                .as_ref()
                                .map(|time| {
                                    crate::preference_runtime::timestamp(
                                        time,
                                        &self.preferences_draft.app.timezone,
                                    )
                                })
                                .unwrap_or_else(|| "Time unavailable".into()),
                        )),
                    cx,
                ));
            }
        }
        body.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{LABEL_FRACTION, PodEventsState, VALUE_FRACTION, absent, event_notice};
    #[test]
    fn facts_reserve_thirty_percent_for_labels() {
        assert_eq!(LABEL_FRACTION, 0.3);
        assert_eq!(LABEL_FRACTION + VALUE_FRACTION, 1.0);
    }
    #[test]
    fn unavailable_events_never_claim_empty_history() {
        for state in [
            PodEventsState::Denied,
            PodEventsState::Unavailable,
            PodEventsState::Partial,
        ] {
            assert_ne!(
                event_notice(state, true),
                Some("No events recorded for this pod.")
            );
            assert!(event_notice(state, false).is_some());
        }
        assert_eq!(event_notice(PodEventsState::Complete, false), None);
    }
    #[test]
    fn absent_values_have_explicit_placeholder() {
        assert_eq!(absent(None), "—");
        assert_eq!(absent(Some("")), "—");
        assert_eq!(absent(Some("node-a")), "node-a");
    }
}
