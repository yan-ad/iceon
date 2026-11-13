//! DaemonSet scheduling presentation. All values come from bounded core facts.
use super::deployment_inspector::{fact, field, text_field};
use super::*;
use iceon_core::connection::detail_facts::DetailFact;

fn section_facts<'a>(detail: &'a ResourceDetail, title: &str) -> Vec<&'a DetailFact> {
    detail
        .sections
        .iter()
        .filter(|section| section.title == title)
        .flat_map(|section| &section.facts)
        .collect()
}
fn image_facts(detail: &ResourceDetail) -> Vec<&DetailFact> {
    image_section_facts(&detail.sections)
}
fn image_section_facts(
    sections: &[iceon_core::connection::detail_facts::DetailSection],
) -> Vec<&DetailFact> {
    ["Container images", "Init container images"]
        .into_iter()
        .flat_map(|title| {
            sections
                .iter()
                .filter(move |section| section.title == title)
                .flat_map(|section| &section.facts)
        })
        .filter(|fact| fact.label.ends_with(" · image"))
        .collect()
}
fn scheduling_summary(title: &str, facts: &[&DetailFact]) -> String {
    if facts.is_empty() {
        return "Not reported".into();
    }
    if facts
        .iter()
        .all(|fact| matches!(fact.value.as_str(), "(empty list)" | "(empty map)"))
    {
        "None configured".into()
    } else {
        let prefix = format!("{title} · ");
        let groups = facts
            .iter()
            .filter_map(|fact| fact.label.strip_prefix(&prefix))
            .filter_map(|label| label.split(" · ").next())
            .collect::<std::collections::BTreeSet<_>>();
        if groups.is_empty() {
            format!("{} reviewed fields", facts.len())
        } else {
            format!(
                "{} {}",
                groups.len(),
                if title == "Affinity" {
                    "affinity groups"
                } else {
                    "entries"
                }
            )
        }
    }
}
fn pod_status(collection: Option<&ResourceCollection>) -> String {
    match collection {
        None => "Not loaded".into(),
        Some(collection) => match collection.state {
            CollectionState::Denied => "Access denied".into(),
            CollectionState::Unavailable => "Unavailable".into(),
            state => {
                let mut counts = std::collections::BTreeMap::<&str, usize>::new();
                for item in &collection.items {
                    *counts.entry(item.status.as_str()).or_default() += 1;
                }
                let summary = if counts.is_empty() {
                    "No owned pods found".into()
                } else {
                    counts
                        .into_iter()
                        .map(|(status, count)| format!("{count} {status}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                if state == CollectionState::Partial {
                    format!("Partial snapshot · {summary}")
                } else {
                    summary
                }
            }
        },
    }
}

impl IceonApp {
    fn daemon_set_expandable(
        &self,
        detail: &ResourceDetail,
        title: &str,
        cx: &mut Context<Self>,
    ) -> Div {
        let facts = section_facts(detail, title);
        let key = format!("daemon-set-{title}");
        let expanded = self.inspector.pod_expanded.contains(&key);
        field(
            title.to_owned(),
            v_flex()
                .w_full()
                .gap_2()
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(scheduling_summary(title, &facts)),
                        )
                        .when(!facts.is_empty(), |row| {
                            row.child(
                                Button::new(SharedString::from(key.clone()))
                                    .xsmall()
                                    .ghost()
                                    .label(if expanded { "Hide" } else { "Show" })
                                    .icon(if expanded {
                                        IconName::ChevronUp
                                    } else {
                                        IconName::ChevronDown
                                    })
                                    .accessibility_label(format!(
                                        "{} DaemonSet {title}",
                                        if expanded { "Hide" } else { "Show" }
                                    ))
                                    .on_click(cx.listener(move |app, _, _, cx| {
                                        if !app.inspector.pod_expanded.remove(&key) {
                                            app.inspector.pod_expanded.insert(key.clone());
                                        }
                                        cx.notify();
                                    })),
                            )
                        }),
                )
                .when(expanded, |body| {
                    body.children(facts.iter().map(|fact| {
                        v_flex()
                            .w_full()
                            .gap_1()
                            .p_2()
                            .rounded(px(3.))
                            .bg(cx.theme().secondary)
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(selectable_text(fact.label.clone())),
                            )
                            .child(div().child(selectable_text(fact.value.clone())))
                    }))
                }),
            cx,
        )
    }

    fn daemon_set_selector(&self, detail: &ResourceDetail, title: &str, cx: &App) -> Div {
        let facts = section_facts(detail, title);
        field(
            title.to_owned(),
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .when(facts.is_empty(), |body| body.child("Not reported"))
                .children(facts.iter().map(|fact| {
                    let label = fact
                        .label
                        .strip_prefix(&format!("{title} · "))
                        .unwrap_or(&fact.label);
                    let text = if fact.value == "(empty map)" {
                        "None configured".to_owned()
                    } else {
                        format!("{label}={}", fact.value)
                    };
                    div()
                        .max_w_full()
                        .px_2()
                        .py_1()
                        .rounded(px(3.))
                        .bg(cx.theme().secondary)
                        .child(selectable_text(text))
                })),
            cx,
        )
    }

    pub(super) fn render_daemon_set_details(
        &self,
        detail: &ResourceDetail,
        body: Div,
        cx: &mut Context<Self>,
    ) -> Div {
        let images = image_facts(detail);
        let mut body = body
            .child(self.daemon_set_selector(detail, "Selector", cx))
            .child(self.daemon_set_selector(detail, "Node selector", cx))
            .child(field(
                "Images",
                v_flex()
                    .w_full()
                    .gap_3()
                    .when(images.is_empty(), |body| body.child("Not reported"))
                    .children(images.iter().map(|fact| {
                        v_flex()
                            .w_full()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(selectable_text(
                                        fact.label.trim_end_matches(" · image").to_owned(),
                                    )),
                            )
                            .child(div().w_full().child(selectable_text(fact.value.clone())))
                    })),
                cx,
            ))
            .child(text_field(
                "Strategy type",
                fact(detail, "Update strategy").unwrap_or("—").to_owned(),
                cx,
            ))
            .child(self.daemon_set_expandable(detail, "Tolerations", cx))
            .child(self.daemon_set_expandable(detail, "Affinity", cx));
        let selector_expressions = section_facts(detail, "Selector expressions");
        if !selector_expressions.is_empty() {
            body = body.child(self.daemon_set_expandable(detail, "Selector expressions", cx));
        }
        for (label, key) in [
            ("Desired", "desired"),
            ("Current", "scheduled"),
            ("Ready", "ready"),
            ("Updated", "updated"),
            ("Available", "available"),
        ] {
            body = body.child(text_field(
                label,
                detail
                    .item
                    .columns
                    .get(key)
                    .cloned()
                    .unwrap_or_else(|| "—".into()),
                cx,
            ));
        }
        for label in [
            "Unavailable",
            "Misscheduled",
            "Minimum ready seconds",
            "Observed generation",
        ] {
            if let Some(value) = fact(detail, label) {
                body = body.child(text_field(label, value.to_owned(), cx));
            }
        }
        for item in section_facts(detail, "DaemonSet rolling update") {
            body = body.child(text_field(&item.label, item.value.clone(), cx));
        }
        let pods = self.inspector.collections.iter().find(|collection| {
            collection.collection == CollectionKind::Pods
                && collection.reference == detail.reference
        });
        body = body.child(text_field("Pod status", pod_status(pods), cx));
        let partial = section_facts(detail, "Coverage");
        if !partial.is_empty() {
            body = body.child(
                div()
                    .py_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Some scheduling details were omitted for safety or size limits."),
            );
        }
        body
    }
}

#[cfg(test)]
mod tests {
    use super::{pod_status, scheduling_summary};
    use iceon_core::connection::{
        ResourceKind,
        detail_collections::{CollectionItem, CollectionKind, CollectionState, ResourceCollection},
        detail_facts::DetailFact,
        details::ResourceReference,
    };

    fn collection(state: CollectionState) -> ResourceCollection {
        ResourceCollection {
            cluster_id: "fixture".into(),
            session_id: "fixture".into(),
            read_id: 1,
            reference: ResourceReference {
                kind: ResourceKind::DaemonSets,
                name: "agent".into(),
                namespace: Some("offline".into()),
                uid: Some("fixture-agent".into()),
            },
            collection: CollectionKind::Pods,
            state,
            items: vec![],
        }
    }
    #[test]
    fn pod_status_preserves_denied_partial_missing_and_empty_states() {
        assert_eq!(pod_status(None), "Not loaded");
        assert_eq!(
            pod_status(Some(&collection(CollectionState::Denied))),
            "Access denied"
        );
        assert_eq!(
            pod_status(Some(&collection(CollectionState::Unavailable))),
            "Unavailable"
        );
        assert_eq!(
            pod_status(Some(&collection(CollectionState::Complete))),
            "No owned pods found"
        );
        let mut c = collection(CollectionState::Partial);
        c.items.push(CollectionItem {
            reference: ResourceReference {
                kind: ResourceKind::Pods,
                name: "agent-pod".into(),
                namespace: Some("offline".into()),
                uid: Some("pod".into()),
            },
            created_at: None,
            status: "Running".into(),
            columns: Default::default(),
        });
        assert_eq!(pod_status(Some(&c)), "Partial snapshot · 1 Running");
    }
    #[test]
    fn scheduling_summaries_distinguish_absent_from_explicit_empty() {
        assert_eq!(scheduling_summary("Tolerations", &[]), "Not reported");
        let empty = DetailFact {
            label: "Tolerations".into(),
            value: "(empty list)".into(),
        };
        assert_eq!(
            scheduling_summary("Tolerations", &[&empty]),
            "None configured"
        );
        let field = DetailFact {
            label: "Tolerations · 1 · key".into(),
            value: "dedicated".into(),
        };
        let effect = DetailFact {
            label: "Tolerations · 1 · effect".into(),
            value: "NoSchedule".into(),
        };
        let second = DetailFact {
            label: "Tolerations · 2 · operator".into(),
            value: "Exists".into(),
        };
        assert_eq!(
            scheduling_summary("Tolerations", &[&field, &effect, &second]),
            "2 entries"
        );
        let affinity = DetailFact { label: "Affinity · nodeAffinity · requiredDuringSchedulingIgnoredDuringExecution · nodeSelectorTerms · 1 · matchExpressions · 1 · key".into(), value: "disk".into() };
        assert_eq!(
            scheduling_summary("Affinity", &[&affinity]),
            "1 affinity groups"
        );
    }

    #[test]
    fn image_rows_include_regular_and_init_images_but_not_container_names() {
        use iceon_core::connection::detail_facts::DetailSection;
        let sections = ["Container images", "Init container images"].map(|title| DetailSection {
            title: title.into(),
            facts: vec![
                DetailFact {
                    label: format!("{title} · 1 · name"),
                    value: "agent".into(),
                },
                DetailFact {
                    label: format!("{title} · 1 · image"),
                    value: "agent:v1".into(),
                },
            ],
        });
        let images = super::image_section_facts(&sections);
        assert_eq!(images.len(), 2);
        assert!(
            images
                .iter()
                .any(|fact| fact.label.starts_with("Container images"))
        );
        assert!(
            images
                .iter()
                .any(|fact| fact.label.starts_with("Init container images"))
        );
        assert!(images.iter().all(|fact| fact.label.ends_with(" · image")));
    }
}
