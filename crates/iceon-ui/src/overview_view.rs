//! Snapshot dashboard: no synthetic history, health, or capacity values.
use crate::{
    IceonApp,
    navigation::Route,
    resource_scroll::ResourceScrollScope,
    resource_table::{ResourceTable, TableData},
};
use gpui_kit::{
    component::{
        ActiveTheme as _, Icon, IconName, Sizable as _,
        button::{Button, ButtonVariants as _},
        chart::PieChart,
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
    MetricState, ResourceItem, ResourceKind, ResourcePage, Usage, details::ResourceReference,
    permissions::PermissionAction,
};

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
            "Complete" | "Completed" => Complete,
            "Suspended" => Idle,
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
/// One surface contract for loaded, loading, summary, and capacity cards.
pub(crate) fn card_surface(cx: &App) -> Div {
    card_surface_colors(cx.theme().background, cx.theme().border)
}

fn card_surface_colors(background: Hsla, border: Hsla) -> Div {
    v_flex()
        .border_1()
        .border_color(border)
        .bg(background)
        .rounded(px(12.))
}

fn workload_icon(section: &str) -> Icon {
    Icon::new(crate::sidebar_view::section_asset_icon(section))
}

fn loading_card(label: &str, cx: &App) -> AnyElement {
    card_surface(cx)
        .w_full()
        .min_w_0()
        .p_4()
        .gap_3()
        .rounded(px(12.))
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

/// A complete, finite node observation. Missing coverage is never an empty
/// (zero-used) cluster. Values above allocatable remain visible in the label;
/// only the geometry is clamped to the ring's finite domain.
#[derive(Clone, Copy, Debug, PartialEq)]
struct CapacityReading {
    used: f64,
    allocatable: f64,
}
impl CapacityReading {
    fn aggregate(values: impl IntoIterator<Item = Usage>) -> Option<Self> {
        let mut count = 0;
        let mut reading = Self {
            used: 0.,
            allocatable: 0.,
        };
        for usage in values {
            let (used, allocatable) = (usage.used?, usage.allocatable?);
            if !used.is_finite() || !allocatable.is_finite() || used < 0. || allocatable <= 0. {
                return None;
            }
            reading.used += used;
            reading.allocatable += allocatable;
            count += 1;
        }
        (count > 0
            && reading.used.is_finite()
            && reading.allocatable.is_finite()
            && reading.percent().is_finite())
        .then_some(reading)
    }
    fn percent(self) -> f64 {
        self.used / self.allocatable * 100.
    }
    fn fraction(self) -> f32 {
        (self.used / self.allocatable).clamp(0., 1.) as f32
    }
    fn over_capacity(self) -> bool {
        self.used > self.allocatable
    }
}

fn capacity_reading(
    page: Option<&ResourcePage>,
    memory: bool,
) -> Result<CapacityReading, &'static str> {
    let page = page.ok_or("Node metrics unavailable")?;
    if page.truncated || page.continue_token.is_some() || page.metrics == Some(MetricState::Partial)
    {
        return Err("Partial coverage · cluster usage unavailable");
    }
    match page.metrics {
        Some(MetricState::Available) => {}
        Some(MetricState::Stale) => return Err("Stale metrics · refresh to retry"),
        Some(MetricState::Denied) => return Err("Metrics access unavailable"),
        _ => return Err("Node metrics unavailable"),
    }
    let usages: Option<Vec<_>> = page
        .items
        .iter()
        .map(|item| {
            let node = item.node.as_ref()?;
            Some(if memory {
                node.memory.clone()
            } else {
                node.cpu.clone()
            })
        })
        .collect();
    CapacityReading::aggregate(usages.ok_or("Incomplete node data")?)
        .ok_or("Usage or positive allocatable unavailable")
}

const CAPACITY_KEYS: [&str; 2] = ["overview-cpu-capacity", "overview-memory-capacity"];
const CAPACITY_COLUMN: usize = 1;

pub(crate) fn capacity_pair(cpu: AnyElement, memory: AnyElement) -> AnyElement {
    h_flex()
        .id("overview-capacity-pair")
        .w_full()
        .gap_4()
        .items_start()
        .flex_wrap()
        .child(cpu)
        .child(memory)
        .into_any_element()
}

pub(crate) fn workload_grid(columns: [Vec<AnyElement>; 2]) -> AnyElement {
    h_flex()
        .gap_4()
        .items_start()
        .min_w_0()
        .flex_wrap()
        .children(
            columns
                .into_iter()
                .filter(|column| !column.is_empty())
                .map(|cards| v_flex().flex_1().min_w(px(330.)).gap_4().children(cards)),
        )
        .into_any_element()
}

pub(crate) fn capacity_card(
    page: Option<&ResourcePage>,
    memory: bool,
    loading: bool,
    cx: &App,
) -> AnyElement {
    let label = if memory { "Memory" } else { "CPU" };
    let key = CAPACITY_KEYS[usize::from(memory)];
    let reading = capacity_reading(page, memory);
    let used_color = if reading.is_ok_and(CapacityReading::over_capacity) {
        cx.theme().danger
    } else {
        cx.theme().primary
    };
    // A neutral remainder is not a claim of schedulable/available capacity.
    // An unavailable observation paints only a neutral ring, never success.
    let track = cx.theme().muted_foreground.opacity(0.2);
    let fraction = reading.map_or(0., CapacityReading::fraction);
    let slices = if reading.is_ok() {
        vec![(fraction, used_color), (1. - fraction, track)]
    } else {
        vec![(1., track)]
    };
    let value = reading
        .map(|value| format!("{:.0}%", value.percent()))
        .unwrap_or_else(|_| "—".into());
    let detail = reading
        .map(|value| {
            if memory {
                format!(
                    "{:.1} / {:.1} GiB",
                    value.used / 1073741824.,
                    value.allocatable / 1073741824.
                )
            } else {
                format!("{:.2} / {:.2} cores", value.used, value.allocatable)
            }
        })
        .unwrap_or_else(|reason| {
            if loading && page.is_none() {
                "Loading node metrics…".into()
            } else {
                reason.into()
            }
        });
    card_surface(cx)
        .id(key)
        .flex_1()
        .min_w(px(145.))
        .p_3()
        .gap_2()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(label),
        )
        .child(
            div()
                .relative()
                .w_full()
                .h(px(116.))
                .child(
                    PieChart::new(slices)
                        .id(key)
                        .value(|slice| slice.0)
                        .color(|slice| slice.1)
                        .inner_radius(38.)
                        .outer_radius(50.)
                        .interactive(false)
                        .appear(false),
                )
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(value),
                ),
        )
        .child(div().text_xs().child(detail))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Observed usage / allocatable"),
        )
        .when(reading.is_ok_and(CapacityReading::over_capacity), |card| {
            card.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child("Exceeds allocatable"),
            )
        })
        .into_any_element()
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
        card_surface(cx)
            .w_full()
            .min_w_0()
            .p_4()
            .gap_3()
            .rounded(px(12.))
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new(SharedString::from(format!("overview-card-{section}")))
                            .ghost()
                            .small()
                            .icon(workload_icon(&section))
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
                                        app.toggle_resource_inspector(
                                            reference.clone(),
                                            window,
                                            cx,
                                        );
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
        capacity_card(
            self.overview_snapshot.nodes.as_deref(),
            memory,
            self.cluster_loading,
            cx,
        )
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
                    cx.new(|cx| {
                        ResourceTable::new(owner, cx)
                            .without_selection()
                            .without_pagination()
                    })
                })
                .clone();
            let data = TableData {
                page: Some(page.clone()),
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
                    .map(|n| format!("{n} / {}", count(snap.nodes.as_deref())))
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
                    .map(|n| format!("{n} / {}", count(snap.pods.as_deref())))
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
            columns[column].push(self.overview_workload_card(
                label,
                section,
                kind,
                page.map(|page| page.as_ref()),
                cx,
            ));
        }
        // The final right-hand cell follows DaemonSets, opposite Jobs. Keep it
        // inside the workload layout even when only node access is granted.
        if self.can_list(ResourceKind::Nodes) {
            columns[CAPACITY_COLUMN].push(capacity_pair(
                self.overview_capacity(false, cx),
                self.overview_capacity(true, cx),
            ));
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
            .when(!stats.is_empty(), |el| el.child(card_surface(cx).flex_row().w_full().flex_wrap().children(stats)))
            .when(!workloads.is_empty() || self.can_list(ResourceKind::Nodes), |el| el
                .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Workloads"))
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Hover for details · permitted resources can be inspected · + means partial inventory"))
                .child(workload_grid(columns)))
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
                .child(card_surface(cx).overflow_hidden().child(events)))

            .into_any_element()
    }
}

#[cfg(any(test, debug_assertions))]
#[allow(dead_code, unused_imports)]
pub(crate) mod tests {
    use super::{CapacityReading, Health, capacity_reading, count, health};
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
    use iceon_core::connection::{
        MetricState, NodeInfo, ResourceItem, ResourceKind, ResourcePage, Usage,
    };

    pub(crate) fn usage(used: Option<f64>, allocatable: Option<f64>) -> Usage {
        Usage {
            used,
            allocatable,
            capacity: None,
        }
    }

    pub(crate) fn node_page(metrics: Option<MetricState>, usages: Vec<Usage>) -> ResourcePage {
        ResourcePage {
            cluster_id: "fixture".into(),
            session_id: "fixture".into(),
            kind: ResourceKind::Nodes,
            read_id: 1,
            items: usages
                .into_iter()
                .enumerate()
                .map(|(index, node_usage)| ResourceItem {
                    name: format!("node-{index}"),
                    namespace: None,
                    uid: None,
                    resource_version: None,
                    status: "Ready".into(),
                    created_at: None,
                    node: Some(NodeInfo {
                        roles: Vec::new(),
                        taints: 0,
                        version: None,
                        internal_ips: Vec::new(),
                        schedulable: true,
                        conditions: Vec::new(),
                        disk_allocatable: None,
                        cpu: node_usage.clone(),
                        memory: usage(
                            node_usage.used.map(|v| v * 2.),
                            node_usage.allocatable.map(|v| v * 2.),
                        ),
                        metrics_timestamp: None,
                    }),
                    containers: Vec::new(),
                    columns: Default::default(),
                })
                .collect(),
            truncated: false,
            fetched_at: 1,
            namespace: None,
            continue_token: None,
            metrics,
        }
    }

    #[test]
    fn capacity_aggregates_finite_usage_and_clamps_only_chart_geometry() {
        let reading =
            CapacityReading::aggregate([usage(Some(2.), Some(4.)), usage(Some(1.), Some(2.))])
                .unwrap();
        assert_eq!(
            reading,
            CapacityReading {
                used: 3.,
                allocatable: 6.
            }
        );
        assert_eq!(reading.percent(), 50.);
        assert_eq!(reading.fraction(), 0.5);

        let over = CapacityReading::aggregate([usage(Some(8.), Some(4.))]).unwrap();
        assert!(over.over_capacity());
        assert_eq!(over.percent(), 200.);
        assert_eq!(over.fraction(), 1.);
    }

    #[test]
    fn capacity_rejects_nonfinite_negative_empty_and_overflowing_values() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.] {
            assert!(CapacityReading::aggregate([usage(Some(bad), Some(4.))]).is_none());
            assert!(CapacityReading::aggregate([usage(Some(1.), Some(bad))]).is_none());
        }
        assert!(CapacityReading::aggregate([]).is_none());
        assert!(CapacityReading::aggregate([usage(Some(f64::MAX), Some(1.))]).is_none());
        assert!(
            CapacityReading::aggregate([
                usage(Some(1.), Some(f64::MAX)),
                usage(Some(1.), Some(f64::MAX))
            ])
            .is_none()
        );
        let zero = CapacityReading::aggregate([usage(Some(0.), Some(4.))]).unwrap();
        assert_eq!(zero.percent(), 0.);
        assert!(!zero.over_capacity());
    }

    #[test]
    fn capacity_requires_complete_current_node_observations() {
        let mut page = node_page(
            Some(MetricState::Available),
            vec![usage(Some(1.), Some(4.))],
        );
        assert_eq!(capacity_reading(Some(&page), false).unwrap().percent(), 25.);
        assert_eq!(capacity_reading(Some(&page), true).unwrap().used, 2.);
        for state in [
            None,
            Some(MetricState::Stale),
            Some(MetricState::Error),
            Some(MetricState::Denied),
            Some(MetricState::Unavailable),
            Some(MetricState::Partial),
        ] {
            page.metrics = state;
            assert!(capacity_reading(Some(&page), false).is_err());
        }
        page.metrics = Some(MetricState::Available);
        page.truncated = true;
        assert!(capacity_reading(Some(&page), false).is_err());
        page.truncated = false;
        page.continue_token = Some("more".into());
        assert!(capacity_reading(Some(&page), false).is_err());
        page.continue_token = None;
        page.items[0].node = None;
        assert!(capacity_reading(Some(&page), false).is_err());
    }

    #[test]
    fn capacity_keys_are_distinct_and_placement_is_the_daemonsets_column() {
        assert_ne!(super::CAPACITY_KEYS[0], super::CAPACITY_KEYS[1]);
        // Alternating workload insertion places DaemonSets at index five,
        // Jobs at six, and the capacity pair in the final right-hand cell.
        assert_eq!(super::CAPACITY_COLUMN, 5 % 2);
        assert_ne!(super::CAPACITY_COLUMN, 6 % 2);
    }

    #[test]
    fn cards_use_one_pixel_border_and_exact_background_surface() {
        use gpui_kit::{Styled as _, px, rgb};
        let background = rgb(0x123456).into();
        let border = rgb(0x654321).into();
        let mut surface = super::card_surface_colors(background, border);
        assert_eq!(surface.style().background, Some(background.into()));
        assert_eq!(surface.style().border_color, Some(border));
        for width in [
            surface.style().border_widths.top,
            surface.style().border_widths.bottom,
            surface.style().border_widths.left,
            surface.style().border_widths.right,
        ] {
            assert_eq!(width, Some(px(1.).into()));
        }
    }

    #[test]
    fn overview_icons_resolve_through_sidebar_assets() {
        use gpui_kit_assets::IconName::*;
        for (section, expected) in [
            ("pods", Box),
            ("deployments", Layers),
            ("cron-jobs", CalendarClock),
            ("stateful-sets", Database),
            ("replica-sets", Copy),
            ("daemon-sets", Network),
            ("jobs", ListChecks),
        ] {
            assert_eq!(crate::sidebar_view::section_asset_icon(section), expected);
            let _ = super::workload_icon(section);
        }
    }

    #[test]
    fn capacity_rejects_zero_missing_and_partial_metrics_without_fake_success() {
        assert!(CapacityReading::aggregate([usage(Some(0.), Some(0.))]).is_none());
        assert!(CapacityReading::aggregate([usage(None, Some(1.))]).is_none());
        assert_eq!(
            capacity_reading(None, false),
            Err("Node metrics unavailable")
        );
        assert_eq!(
            capacity_reading(
                Some(&node_page(
                    Some(MetricState::Partial),
                    vec![usage(Some(1.), Some(2.))]
                )),
                false
            ),
            Err("Partial coverage · cluster usage unavailable")
        );
        assert_eq!(
            capacity_reading(
                Some(&node_page(
                    Some(MetricState::Available),
                    vec![usage(Some(1.), None)]
                )),
                false,
            ),
            Err("Usage or positive allocatable unavailable")
        );
    }
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
    fn jobs_use_observed_completion_suspension_and_active_counts() {
        for status in ["Complete", "Completed"] {
            for columns in [vec![], vec![("active", "0")], vec![("active", "1")]] {
                assert_eq!(
                    health(ResourceKind::Jobs, &item(status, &columns)),
                    Health::Complete
                );
            }
        }
        for status in ["Suspended", "Failed"] {
            let expected = if status == "Suspended" {
                Health::Idle
            } else {
                Health::Attention
            };
            for columns in [vec![], vec![("active", "0")], vec![("active", "1")]] {
                assert_eq!(
                    health(ResourceKind::Jobs, &item(status, &columns)),
                    expected
                );
            }
        }
        for status in ["Running", "Not reported", "Unknown", ""] {
            assert_eq!(
                health(ResourceKind::Jobs, &item(status, &[("active", "1")])),
                Health::Running
            );
            for columns in [vec![], vec![("active", "0")], vec![("active", "invalid")]] {
                assert_eq!(
                    health(ResourceKind::Jobs, &item(status, &columns)),
                    Health::Unknown
                );
            }
        }
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
