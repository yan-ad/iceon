//! Snapshot dashboard: no synthetic history, health, or capacity values.
use crate::{
    IceonApp,
    navigation::Route,
    resource_scroll::ResourceScrollScope,
    resource_table::{ResourceTable, TableData},
};
use gpui_kit::{
    component::{
        ActiveTheme as _, IconName, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        scroll::ScrollableElement as _,
        skeleton::Skeleton,
        tooltip::Tooltip,
        v_flex,
    },
    prelude::*,
    *,
};
use iceon_core::connection::{
    ResourceItem, ResourceKind, ResourcePage, details::ResourceReference,
    permissions::PermissionAction,
};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Health {
    Attention,
    Starting,
    Running,
    Complete,
    Idle,
    Unknown,
}
fn number(item: &ResourceItem, key: &str) -> Option<u64> {
    item.columns.get(key)?.parse().ok()
}
fn health(kind: ResourceKind, item: &ResourceItem) -> Health {
    use Health::*;
    match kind {
        ResourceKind::Pods => match item.status.as_str() {
            "Failed" => Attention,
            "Succeeded" => Complete,
            "Pending" => Starting,
            "Running"
                if item
                    .columns
                    .get("health")
                    .is_some_and(|value| value == "healthy" || value == "running") =>
            {
                Running
            }
            "Running" if item.columns.contains_key("warning") => Attention,
            "Running" => Starting,
            _ => Unknown,
        },
        ResourceKind::Jobs => match item.status.as_str() {
            "Failed" => Attention,
            "Complete" => Complete,
            _ if number(item, "active").is_some_and(|n| n > 0) => Running,
            _ => Unknown,
        },
        ResourceKind::CronJobs => match item.columns.get("suspend").map(String::as_str) {
            Some("true") => Idle,
            Some("false") => Running,
            _ => Unknown,
        },
        _ => match (number(item, "desired"), number(item, "ready")) {
            (Some(0), _) => Idle,
            (Some(desired), Some(ready)) if ready >= desired => Running,
            (Some(_), Some(_)) => Attention,
            _ => Unknown,
        },
    }
}
fn color(state: Health, cx: &App) -> Hsla {
    match state {
        Health::Attention => cx.theme().danger,
        Health::Starting => cx.theme().warning,
        Health::Running => cx.theme().success,
        Health::Complete => cx.theme().success.opacity(0.45),
        Health::Idle | Health::Unknown => cx.theme().muted_foreground.opacity(0.4),
    }
}
fn count(page: Option<&ResourcePage>) -> String {
    page.map(|page| {
        format!(
            "{}{}",
            page.items.len(),
            if page.truncated || page.continue_token.is_some() {
                "+"
            } else {
                ""
            }
        )
    })
    .unwrap_or_else(|| "—".into())
}
fn stat(label: &str, value: String, note: &str, loading: bool, cx: &App) -> AnyElement {
    v_flex()
        .flex_1()
        .min_w(px(145.))
        .gap_1()
        .px_3()
        .py_2()
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(label.to_owned()),
        )
        .child(if loading && value == "—" {
            Skeleton::new().w(px(72.)).h(px(28.)).into_any_element()
        } else {
            div()
                .text_xl()
                .font_weight(FontWeight::SEMIBOLD)
                .child(value)
                .into_any_element()
        })
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(note.to_owned()),
        )
        .into_any_element()
}
fn loading_card(label: &str, cx: &App) -> AnyElement {
    v_flex()
        .w_full()
        .min_w_0()
        .p_4()
        .gap_3()
        .rounded(px(12.))
        .bg(cx.theme().secondary)
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child(label.to_owned()),
        )
        .child(Skeleton::new().h(px(44.)).rounded(px(4.)))
        .child(
            h_flex()
                .gap_4()
                .children((0..3).map(|_| Skeleton::new().w(px(72.)).h(px(32.)))),
        )
        .into_any_element()
}

/// Authorization denials are an expected absence, not a dashboard error.
fn overview_error(error: &str) -> Option<String> {
    let messages: Vec<_> = error
        .split("; ")
        .filter(|message| {
            let lower = message.to_ascii_lowercase();
            ![
                "forbidden",
                "unauthorized",
                "permission denied",
                "access denied",
            ]
            .iter()
            .any(|denial| lower.contains(denial))
        })
        .collect();
    (!messages.is_empty()).then(|| messages.join("; "))
}

impl IceonApp {
    fn overview_workload_card(
        &self,
        label: &str,
        section: &str,
        kind: ResourceKind,
        page: Option<&ResourcePage>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if page.is_none() && self.cluster_loading {
            return loading_card(label, cx);
        }
        let mut items = page.map(|page| page.items.clone()).unwrap_or_default();
        items.sort_by(|a, b| (&a.namespace, &a.name).cmp(&(&b.namespace, &b.name)));
        let mut totals = [0usize; 6];
        for item in &items {
            totals[health(kind, item) as usize] += 1;
        }
        let cid = self
            .cluster_session
            .as_ref()
            .map(|session| session.cluster_id.clone())
            .unwrap_or_default();
        let section = section.to_owned();
        v_flex()
            .w_full()
            .min_w_0()
            .p_4()
            .gap_3()
            .rounded(px(12.))
            .bg(cx.theme().secondary)
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new(SharedString::from(format!("overview-card-{section}")))
                            .ghost()
                            .small()
                            .icon(IconName::GalleryVerticalEnd)
                            .label(label.to_owned())
                            .on_click(cx.listener(move |app, _, window, cx| {
                                app.navigate(
                                    Route::Cluster {
                                        cluster_id: cid.clone(),
                                        section: section.clone(),
                                    },
                                    true,
                                    window,
                                    cx,
                                )
                            })),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(count(page)),
                    ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .flex_wrap()
                    .children(items.iter().take(1000).enumerate().map(|(index, item)| {
                        let state = health(kind, item);
                        let details = format!(
                            "{}/{}\n{}",
                            item.namespace.as_deref().unwrap_or("cluster"),
                            item.name,
                            item.status
                        );
                        let reference = ResourceReference {
                            kind,
                            name: item.name.clone(),
                            namespace: item.namespace.clone(),
                            uid: item.uid.clone(),
                        };
                        let inspectable = self.can_action(
                            kind,
                            PermissionAction::Get,
                            item.namespace.as_deref(),
                            Some(&item.name),
                        );
                        div()
                            .id((
                                SharedString::from(format!("overview-square-{kind:?}")),
                                index,
                            ))
                            .size(px(9.))
                            .rounded(px(2.))
                            .bg(color(state, cx))
                            .when(inspectable, |square| square.cursor_pointer())
                            .tooltip(move |window, cx| {
                                Tooltip::new(details.clone()).build(window, cx)
                            })
                            .when(inspectable, |square| {
                                square.on_click(cx.listener(move |app, _, window, cx| {
                                    if app.can_action(
                                        kind,
                                        PermissionAction::Get,
                                        reference.namespace.as_deref(),
                                        Some(&reference.name),
                                    ) {
                                        app.open_resource_inspector(reference.clone(), window, cx);
                                    }
                                }))
                            })
                    })),
            )
            .when(page.is_none(), |card| {
                card.child(crate::empty_state::compact_empty_state(
                    IconName::CircleAlert,
                    "Data unavailable",
                    "Refresh to retry loading this resource.",
                ))
            })
            .when(page.is_some_and(|page| page.items.is_empty()), |card| {
                card.child(crate::empty_state::compact_empty_state(
                    IconName::Inbox,
                    "No resources found",
                    "There are no resources in this namespace scope.",
                ))
            })
            .when(items.len() > 1000, |card| {
                card.child(
                    div().text_xs().child(
                        "First 1,000 indicators shown; open the list for all loaded resources.",
                    ),
                )
            })
            .child(
                h_flex().gap_3().flex_wrap().children(
                    [
                        (Health::Attention, "Needs attention"),
                        (Health::Starting, "Starting"),
                        (Health::Running, "Running"),
                        (Health::Complete, "Completed"),
                        (Health::Idle, "Idle"),
                        (Health::Unknown, "Unknown"),
                    ]
                    .into_iter()
                    .filter(|(state, _)| totals[*state as usize] > 0)
                    .map(|(state, label)| {
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(totals[state as usize].to_string()),
                            )
                            .child(
                                h_flex()
                                    .gap_1()
                                    .items_center()
                                    .child(div().size(px(6.)).rounded(px(1.)).bg(color(state, cx)))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(label),
                                    ),
                            )
                    }),
                ),
            )
            .into_any_element()
    }
    fn overview_capacity(&self, memory: bool, cx: &App) -> AnyElement {
        let page = self.overview_snapshot.nodes.as_ref();
        if page.is_none() && self.cluster_loading {
            return div()
                .flex_1()
                .min_w_0()
                .child(loading_card(if memory { "Memory" } else { "CPU" }, cx))
                .into_any_element();
        }
        let complete = page.is_some_and(|page| {
            !page.items.is_empty() && !page.truncated && page.continue_token.is_none()
        });
        let values: Option<Vec<(f64, f64)>> = page.and_then(|page| {
            page.items
                .iter()
                .map(|item| {
                    let node = item.node.as_ref()?;
                    let usage = if memory { &node.memory } else { &node.cpu };
                    Some((usage.used?, usage.allocatable?))
                })
                .collect()
        });
        let values = values.filter(|_| complete);
        let sums = values.map(|values| {
            values
                .into_iter()
                .fold((0., 0.), |sum, value| (sum.0 + value.0, sum.1 + value.1))
        });
        let percent = sums
            .filter(|(_, total)| *total > 0.)
            .map(|(used, total)| used / total * 100.);
        let label = if memory { "Memory" } else { "CPU" };
        let text = sums
            .map(|(used, total)| {
                if memory {
                    format!(
                        "{:.1} / {:.1} GiB used",
                        used / 1073741824.,
                        total / 1073741824.
                    )
                } else {
                    format!("{used:.2} / {total:.2} cores used")
                }
            })
            .unwrap_or_else(|| "Metrics or complete node capacity unavailable".into());
        v_flex()
            .flex_1()
            .min_w_0()
            .p_4()
            .gap_3()
            .rounded(px(12.))
            .bg(cx.theme().secondary)
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(label),
            )
            .child(h_flex().gap_1().flex_wrap().children((0..100).map(|index| {
                div().size(px(8.)).rounded(px(2.)).bg(
                    if percent.is_some_and(|p| f64::from(index) < p.min(100.)) {
                        cx.theme().primary
                    } else {
                        cx.theme().muted_foreground.opacity(0.2)
                    },
                )
            })))
            .child(
                div().text_lg().child(
                    percent
                        .map(|p| format!("{p:.0}%"))
                        .unwrap_or_else(|| "—".into()),
                ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(text),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Observed node usage / allocatable · requests not collected"),
            )
            .into_any_element()
    }
    pub(crate) fn render_overview_dashboard(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if !self.access_ready() {
            return v_flex()
                .size_full()
                .p_4()
                .gap_4()
                .child(div().text_sm().child(self.access_message()))
                .child(Skeleton::new().h(px(44.)))
                .child(
                    Button::new("overview-refresh-access")
                        .ghost()
                        .small()
                        .label("Retry access check")
                        .on_click(cx.listener(|app, _, window, cx| app.refresh_access(window, cx))),
                )
                .into_any_element();
        }
        // Never project cached data from a previously authorized scope.
        let mut snap = self.overview_snapshot.clone();
        if !self.can_list(ResourceKind::Nodes) {
            snap.nodes = None;
        }
        if !self.can_list(ResourceKind::Pods) {
            snap.pods = None;
        }
        if !self.can_list(ResourceKind::Events) {
            snap.events = None;
        }
        let workloads: Vec<_> = [
            (&snap.pods, ResourceKind::Pods),
            (&snap.deployments, ResourceKind::Deployments),
            (&snap.daemonsets, ResourceKind::DaemonSets),
            (&snap.statefulsets, ResourceKind::StatefulSets),
            (&snap.replicasets, ResourceKind::ReplicaSets),
            (&snap.jobs, ResourceKind::Jobs),
            (&snap.cronjobs, ResourceKind::CronJobs),
        ]
        .into_iter()
        .filter(|(_, kind)| self.can_list(*kind))
        .collect();
        let attention = workloads
            .iter()
            .filter_map(|(page, kind)| {
                page.as_ref().map(|page| {
                    page.items
                        .iter()
                        .filter(|item| health(*kind, item) == Health::Attention)
                        .count()
                })
            })
            .sum::<usize>();
        let ready = snap.nodes.as_ref().map(|page| {
            page.items
                .iter()
                .filter(|item| item.status == "Ready")
                .count()
        });
        let running = snap.pods.as_ref().map(|page| {
            page.items
                .iter()
                .filter(|item| item.status == "Running")
                .count()
        });
        let warning = snap.events.as_ref().map(|page| {
            page.items
                .iter()
                .filter(|item| item.status == "Warning")
                .count()
        });
        let owner = cx.entity().downgrade();
        let events = if !self.can_list(ResourceKind::Events) {
            div().into_any_element()
        } else if let Some(page) = snap.events.as_ref() {
            let table = self
                .overview_events_table
                .get_or_insert_with(|| {
                    cx.new(|cx| ResourceTable::new(owner, cx).without_selection())
                })
                .clone();
            let data = TableData {
                page: Some(Rc::new(page.clone())),
                scope: ResourceScrollScope {
                    cluster_id: page.cluster_id.clone(),
                    kind: ResourceKind::Events,
                    namespace: page.namespace.clone(),
                    query: String::new(),
                    status: None,
                    page_token: None,
                },
            };
            table.update(cx, |table, cx| table.sync(data, cx));
            div().h(px(330.)).w_full().child(table).into_any_element()
        } else if self.cluster_loading {
            v_flex()
                .w_full()
                .h(px(330.))
                .gap_2()
                .p_3()
                .child(Skeleton::new().h(px(32.)))
                .children((0..6).map(|_| Skeleton::new().h(px(36.)).secondary()))
                .into_any_element()
        } else {
            crate::empty_state::compact_empty_state(
                IconName::Bell,
                "Events unavailable",
                "Refresh to retry loading events for this scope.",
            )
            .into_any_element()
        };
        let mut stats = Vec::new();
        if !workloads.is_empty() {
            stats.push(stat(
                "Needs attention",
                if workloads.iter().all(|(page, _)| page.is_some()) {
                    attention.to_string()
                } else {
                    "—".into()
                },
                "Authorized workload issues",
                self.cluster_loading,
                cx,
            ));
        }
        if self.can_list(ResourceKind::Nodes) {
            stats.push(stat(
                "Nodes ready",
                ready
                    .map(|n| format!("{n} / {}", count(snap.nodes.as_ref())))
                    .unwrap_or_else(|| "—".into()),
                "Loaded nodes",
                self.cluster_loading,
                cx,
            ));
        }
        if self.can_list(ResourceKind::Pods) {
            stats.push(stat(
                "Pods running",
                running
                    .map(|n| format!("{n} / {}", count(snap.pods.as_ref())))
                    .unwrap_or_else(|| "—".into()),
                "Selected namespace scope",
                self.cluster_loading,
                cx,
            ));
        }
        if self.can_list(ResourceKind::Events) {
            stats.push(stat(
                "Warning events",
                warning.map(|n| n.to_string()).unwrap_or_else(|| "—".into()),
                "Loaded event records, not an hourly rate",
                self.cluster_loading,
                cx,
            ));
        }
        let mut columns = [Vec::new(), Vec::new()];
        for (label, section, kind, page) in [
            ("Pods", "pods", ResourceKind::Pods, snap.pods.as_ref()),
            (
                "Deployments",
                "deployments",
                ResourceKind::Deployments,
                snap.deployments.as_ref(),
            ),
            (
                "CronJobs",
                "cron-jobs",
                ResourceKind::CronJobs,
                snap.cronjobs.as_ref(),
            ),
            (
                "StatefulSets",
                "stateful-sets",
                ResourceKind::StatefulSets,
                snap.statefulsets.as_ref(),
            ),
            (
                "ReplicaSets",
                "replica-sets",
                ResourceKind::ReplicaSets,
                snap.replicasets.as_ref(),
            ),
            (
                "DaemonSets",
                "daemon-sets",
                ResourceKind::DaemonSets,
                snap.daemonsets.as_ref(),
            ),
            ("Jobs", "jobs", ResourceKind::Jobs, snap.jobs.as_ref()),
        ]
        .into_iter()
        .filter(|(_, _, kind, _)| self.can_list(*kind))
        {
            let column = if columns[0].len() <= columns[1].len() {
                0
            } else {
                1
            };
            columns[column].push(self.overview_workload_card(label, section, kind, page, cx));
        }
        if workloads.is_empty()
            && !self.can_list(ResourceKind::Nodes)
            && !self.can_list(ResourceKind::Events)
        {
            return crate::empty_state::page_empty_state(IconName::LayoutDashboard,
                "No overview resources available", "Choose another namespace or refresh access to view resources available to your account.").into_any_element();
        }
        v_flex()
            .size_full().min_h_0().p_4().gap_4().overflow_y_scrollbar()
            .when_some(self.cluster_error.as_deref().and_then(overview_error), |el, error| el.child(crate::empty_state::error_state(error)))
            .when(!stats.is_empty(), |el| el.child(h_flex().w_full().flex_wrap().rounded(px(12.)).bg(cx.theme().secondary).children(stats)))
            .when(!workloads.is_empty(), |el| el
                .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Workloads"))
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Hover for details · permitted resources can be inspected · + means partial inventory"))
                .child(h_flex().gap_4().items_start().min_w_0().children(columns.into_iter().filter(|column| !column.is_empty()).map(|cards| v_flex().flex_1().min_w_0().gap_4().children(cards)))))
            .when(self.can_list(ResourceKind::Nodes), |el| el
                .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Capacity"))
                .child(h_flex().gap_4().child(self.overview_capacity(false, cx)).child(self.overview_capacity(true, cx))))
            .when(self.can_list(ResourceKind::Events), |el| el
                .child(h_flex().justify_between().items_center()
                    .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Events"))
                    .child(Button::new("overview-all-events").ghost().small().label("Show all")
                        .on_click(cx.listener(|app, _, window, cx| {
                            if app.route_allowed("events")
                                && let Some(session) = &app.cluster_session {
                                    app.navigate(Route::Cluster { cluster_id: session.cluster_id.clone(), section: "events".into() }, true, window, cx);
                                }
                        }))))
                .child(events))

            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{Health, count, health};
    #[test]
    fn authorization_failures_are_not_dashboard_errors() {
        assert_eq!(super::overview_error("Pods: Forbidden"), None);
        assert_eq!(
            super::overview_error("Events: Unauthorized; Pods: access denied"),
            None
        );
        assert_eq!(
            super::overview_error("Events: Forbidden; Nodes: timed out"),
            Some("Nodes: timed out".into())
        );
    }
    use iceon_core::connection::{ResourceItem, ResourceKind};
    fn item(status: &str, columns: &[(&str, &str)]) -> ResourceItem {
        ResourceItem {
            name: "fixture".into(),
            namespace: Some("default".into()),
            uid: Some("uid".into()),
            resource_version: Some("1".into()),
            status: status.into(),
            created_at: None,
            node: None,
            containers: Vec::new(),
            columns: columns
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }
    #[test]
    fn health_never_treats_not_ready_as_running() {
        assert_eq!(
            health(
                ResourceKind::Deployments,
                &item("NotReady", &[("desired", "3"), ("ready", "1")])
            ),
            Health::Attention
        );
        assert_eq!(
            health(ResourceKind::Deployments, &item("Unknown", &[])),
            Health::Unknown
        );
        assert_eq!(
            health(
                ResourceKind::Deployments,
                &item("Unknown", &[("desired", "0"), ("ready", "0")])
            ),
            Health::Idle
        );
        assert_eq!(
            health(
                ResourceKind::Pods,
                &item("Running", &[("health", "healthy")])
            ),
            Health::Running
        );
        assert_eq!(
            health(
                ResourceKind::Pods,
                &item("Running", &[("warning", "ContainerWaiting")])
            ),
            Health::Attention
        );
        assert_eq!(
            health(ResourceKind::Pods, &item("Succeeded", &[])),
            Health::Complete
        );
    }
    #[test]
    fn cronjobs_use_observed_suspend_field_and_missing_data_is_unknown() {
        assert_eq!(
            health(
                ResourceKind::CronJobs,
                &item("Unknown", &[("suspend", "true")])
            ),
            Health::Idle
        );
        assert_eq!(
            health(
                ResourceKind::CronJobs,
                &item("Unknown", &[("suspend", "false")])
            ),
            Health::Running
        );
        assert_eq!(
            health(ResourceKind::CronJobs, &item("Unknown", &[])),
            Health::Unknown
        );
        assert_eq!(count(None), "—");
    }
}
