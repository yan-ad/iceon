//! Cached resource-table entity. Scrolling never borrows or renders the application shell.
use crate::{
    IceonApp,
    cluster_view::{
        ColumnDef, RESOURCE_ACTIONS_WIDTH, RESOURCE_AGE_WIDTH, RESOURCE_NAME_WIDTH,
        RESOURCE_NAMESPACE_WIDTH, RESOURCE_STATUS_WIDTH, columns_for_kind, namespaced_kind,
        resource_table_min_width, row_status_tag, table_cell,
    },
    inspector_view::DeleteTarget,
    navigation::{Route, cell_value, format_age},
    resource_scroll::{ResourceScrollScope, ResourceScrollState},
};
use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _, VirtualListScrollHandle,
    button::{Button, ButtonVariants as _},
    h_flex,
    scroll::{ScrollableElement as _, ScrollbarAxis},
    tag::Tag,
    v_flex, v_virtual_list,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use iceon_core::connection::{
    ResourceItem, ResourceKind, ResourcePage, details::ResourceReference, node_actions::NodeAction,
    pod_runtime::PodStreamMode, workload_actions::WorkloadAction,
};
use std::rc::Rc;

pub(crate) struct TableData {
    pub page: Option<Rc<ResourcePage>>,
    pub scope: ResourceScrollScope,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TableStats {
    pub filter_builds: usize,
    pub row_size_builds: usize,
    pub renders: usize,
    pub row_renders: usize,
}

#[derive(Default)]
struct TableCache {
    data: Option<TableData>,
    indices: Rc<Vec<usize>>,
    row_sizes: Rc<Vec<Size<Pixels>>>,
    stats: TableStats,
}

pub(crate) fn matches_filter(item: &ResourceItem, query: &str, status: Option<&str>) -> bool {
    status.is_none_or(|status| item.status.eq_ignore_ascii_case(status))
        && (query.is_empty()
            || item.name.to_lowercase().contains(query)
            || item
                .namespace
                .as_deref()
                .unwrap_or_default()
                .to_lowercase()
                .contains(query)
            || item
                .columns
                .values()
                .any(|value| value.to_lowercase().contains(query)))
}

impl TableCache {
    fn update(&mut self, data: TableData) -> bool {
        if self.data.as_ref().is_some_and(|old| {
            old.scope == data.scope
                && match (&old.page, &data.page) {
                    (Some(a), Some(b)) => Rc::ptr_eq(a, b),
                    (None, None) => true,
                    _ => false,
                }
        }) {
            return false;
        }
        let indices: Vec<usize> = data
            .page
            .as_ref()
            .filter(|page| page.kind == data.scope.kind)
            .map(|page| {
                page.items
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| {
                        matches_filter(item, &data.scope.query, data.scope.status.as_deref())
                    })
                    .map(|(index, _)| index)
                    .collect()
            })
            .unwrap_or_default();
        self.stats.filter_builds += 1;
        if self.row_sizes.len() != indices.len() {
            self.row_sizes = Rc::new(vec![size(px(42.0), px(42.0)); indices.len()]);
            self.stats.row_size_builds += 1;
        }
        self.indices = Rc::new(indices);
        self.data = Some(data);
        true
    }
}

pub(crate) struct ResourceTable {
    owner: WeakEntity<IceonApp>,
    cache: TableCache,
    scroll: VirtualListScrollHandle,
    scroll_state: ResourceScrollState,
    horizontal_scroll: ScrollHandle,
}

impl ResourceTable {
    pub(crate) fn new(owner: WeakEntity<IceonApp>, _cx: &mut Context<Self>) -> Self {
        Self {
            owner,
            cache: TableCache::default(),
            scroll: VirtualListScrollHandle::new(),
            scroll_state: ResourceScrollState::default(),
            horizontal_scroll: ScrollHandle::default(),
        }
    }

    pub(crate) fn sync(&mut self, data: TableData, cx: &mut Context<Self>) {
        let scope_changed = self
            .cache
            .data
            .as_ref()
            .is_none_or(|old| old.scope != data.scope);
        let scroll = self.scroll_state.for_scope(data.scope.clone());
        if self.cache.update(data) {
            self.scroll = scroll;
            if scope_changed {
                self.horizontal_scroll = ScrollHandle::default();
            }
            cx.notify();
        }
    }

    #[cfg(debug_assertions)]
    pub(crate) fn stats(&self) -> TableStats {
        self.cache.stats
    }

    #[cfg(debug_assertions)]
    pub(crate) fn scroll_offsets(&self) -> (Point<Pixels>, Point<Pixels>) {
        (self.scroll.offset(), self.horizontal_scroll.offset())
    }

    fn action<F>(
        &self,
        callback: F,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static + use<F>
    where
        F: Fn(&mut IceonApp, &ClickEvent, &mut Window, &mut Context<IceonApp>) + 'static,
    {
        let owner = self.owner.clone();
        move |event, window, cx| {
            let _ = owner.update(cx, |app, cx| callback(app, event, window, cx));
        }
    }

    fn render_resource_row(
        &mut self,
        kind: ResourceKind,
        item: &ResourceItem,
        idx: usize,
        show_ns: bool,
        cols: &'static [ColumnDef],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.cache.stats.row_renders += 1;
        let reference = ResourceReference {
            kind,
            name: item.name.clone(),
            namespace: item.namespace.clone(),
            uid: item.uid.clone(),
        };
        let row_actions = self
            .render_row_actions(kind, item, idx, cx)
            .into_any_element();
        let age_str = format_age(item.created_at.as_deref());

        h_flex()
            .w_full()
            .h(px(42.0))
            .items_center()
            .child(
                table_cell(RESOURCE_NAME_WIDTH, true, true, cx.theme().border)
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .truncate()
                    .child(item.name.clone()),
            )
            .when(show_ns, |el| {
                el.child(
                    table_cell(RESOURCE_NAMESPACE_WIDTH, false, true, cx.theme().border)
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .truncate()
                        .child(item.namespace.clone().unwrap_or_default()),
                )
            })
            .children(cols.iter().map(|c| {
                if kind == ResourceKind::Namespaces && c.key == "labels" {
                    let extra_count = item
                        .columns
                        .get("labels")
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(0)
                        .saturating_sub(1);
                    table_cell(c.width, false, true, cx.theme().border)
                        .gap_1()
                        .items_center()
                        .overflow_hidden()
                        .child(
                            Tag::secondary()
                                .small()
                                .child(format!("kubernetes.io/metadata.name={}", item.name)),
                        )
                        .when(extra_count > 0, |row| {
                            row.child(
                                Tag::secondary()
                                    .outline()
                                    .small()
                                    .child(format!("+{extra_count}")),
                            )
                        })
                        .into_any_element()
                } else {
                    table_cell(c.width, false, true, cx.theme().border)
                        .text_xs()
                        .truncate()
                        .child(cell_value(item, c.key))
                        .into_any_element()
                }
            }))
            .child(
                table_cell(RESOURCE_AGE_WIDTH, false, true, cx.theme().border)
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .truncate()
                    .child(age_str),
            )
            .child(
                table_cell(RESOURCE_STATUS_WIDTH, false, true, cx.theme().border)
                    .items_center()
                    .child(row_status_tag(&item.status)),
            )
            .child(
                table_cell(RESOURCE_ACTIONS_WIDTH, false, false, cx.theme().border)
                    .justify_end()
                    .gap_1()
                    .items_center()
                    .child(row_actions)
                    .child(
                        Button::new(SharedString::from(format!("inspect-row-{idx}")))
                            .xsmall()
                            .ghost()
                            .icon(IconName::Eye)
                            .tooltip("Inspect resource")
                            .accessibility_label("Inspect resource")
                            .on_click(self.action(move |this, _, window, cx| {
                                this.open_resource_inspector(reference.clone(), window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_row_actions(
        &self,
        kind: ResourceKind,
        item: &ResourceItem,
        idx: usize,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_pod = kind == ResourceKind::Pods;
        let can_restart = matches!(
            kind,
            ResourceKind::Deployments | ResourceKind::DaemonSets | ResourceKind::StatefulSets
        );
        let is_cronjob = kind == ResourceKind::CronJobs;
        let is_service = kind == ResourceKind::Services;
        let is_node = kind == ResourceKind::Nodes;
        let is_crd = kind == ResourceKind::CustomResourceDefinitions;

        let ns = item.namespace.clone().unwrap_or_default();
        let name = item.name.clone();
        let uid = item.uid.clone();
        let rv = item.resource_version.clone();

        let first_svc_port: Option<u16> = is_service
            .then(|| {
                item.columns
                    .get("ports")
                    .and_then(|s| s.split(',').next())
                    .and_then(|p| p.trim().parse::<u16>().ok())
            })
            .flatten();

        let node_schedulable = item
            .node
            .as_ref()
            .map(|n| n.schedulable)
            .unwrap_or(item.columns.get("schedulable").map(String::as_str) != Some("false"));

        let current_cid = self
            .cache
            .data
            .as_ref()
            .map(|data| data.scope.cluster_id.clone())
            .unwrap_or_default();

        let ns_logs = ns.clone();
        let name_logs = name.clone();
        let ns_shell = ns;
        let name_shell = name.clone();
        let del_target = uid.clone().zip(rv.clone()).map(|(u, r)| DeleteTarget {
            kind,
            name: name.clone(),
            namespace: item.namespace.clone(),
            uid: u,
            resource_version: r,
        });

        h_flex()
            .flex_shrink_0()
            .justify_end()
            .gap_1()
            .items_center()
            // Pod row quick actions: Logs & Shell
            .when(is_pod, |row| {
                row.child(
                    Button::new(SharedString::from(format!("row-pod-logs-{idx}")))
                        .xsmall()
                        .ghost()
                        .icon(IconName::FileText)
                        .tooltip("View pod logs")
                        .accessibility_label("View pod logs")
                        .on_click(self.action(move |this, _, window, cx| {
                            this.open_pod_stream_for_pod(
                                ns_logs.clone(),
                                name_logs.clone(),
                                PodStreamMode::Logs,
                                window,
                                cx,
                            );
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("row-pod-shell-{idx}")))
                        .xsmall()
                        .ghost()
                        .icon(IconName::SquareTerminal)
                        .tooltip("Open pod shell")
                        .accessibility_label("Open pod shell")
                        .on_click(self.action(move |this, _, window, cx| {
                            this.open_pod_stream_for_pod(
                                ns_shell.clone(),
                                name_shell.clone(),
                                PodStreamMode::Shell,
                                window,
                                cx,
                            );
                        })),
                )
            })
            // Workload row quick action: Restart
            .when_some(
                can_restart.then(|| uid.clone().zip(rv.clone())).flatten(),
                |row, (u, r)| {
                    let wref = ResourceReference {
                        kind,
                        name: name.clone(),
                        namespace: item.namespace.clone(),
                        uid: Some(u),
                    };
                    row.child(
                        Button::new(SharedString::from(format!("row-wl-restart-{idx}")))
                            .xsmall()
                            .ghost()
                            .icon(IconName::RefreshCw)
                            .tooltip("Restart workload")
                            .accessibility_label("Restart workload")
                            .on_click(self.action(move |this, _, window, cx| {
                                this.run_workload_action_for(
                                    wref.clone(),
                                    r.clone(),
                                    WorkloadAction::Restart,
                                    None,
                                    window,
                                    cx,
                                );
                            })),
                    )
                },
            )
            // CronJob row quick action: Trigger
            .when_some(
                is_cronjob.then(|| uid.clone().zip(rv.clone())).flatten(),
                |row, (u, r)| {
                    let cref = ResourceReference {
                        kind,
                        name: name.clone(),
                        namespace: item.namespace.clone(),
                        uid: Some(u),
                    };
                    row.child(
                        Button::new(SharedString::from(format!("row-cj-trigger-{idx}")))
                            .xsmall()
                            .ghost()
                            .icon(IconName::Play)
                            .tooltip("Trigger CronJob")
                            .accessibility_label("Trigger CronJob")
                            .on_click(self.action(move |this, _, window, cx| {
                                this.run_workload_action_for(
                                    cref.clone(),
                                    r.clone(),
                                    WorkloadAction::Trigger,
                                    None,
                                    window,
                                    cx,
                                );
                            })),
                    )
                },
            )
            // Service row quick action: Port-Forward first port
            .when_some(
                first_svc_port.zip(uid.clone().zip(rv.clone())),
                |row, (port, (u, r))| {
                    let sref = ResourceReference {
                        kind: ResourceKind::Services,
                        name: name.clone(),
                        namespace: item.namespace.clone(),
                        uid: Some(u),
                    };
                    row.child(
                        Button::new(SharedString::from(format!("row-svc-fwd-{idx}")))
                            .xsmall()
                            .ghost()
                            .icon(IconName::Network)
                            .tooltip(format!("Forward service port :{port}"))
                            .accessibility_label(format!("Forward service port :{port}"))
                            .on_click(self.action(move |this, _, window, cx| {
                                this.start_service_forward_for(
                                    sref.clone(),
                                    r.clone(),
                                    port,
                                    window,
                                    cx,
                                );
                            })),
                    )
                },
            )
            // Node row quick actions: Cordon/Uncordon + Drain
            .when_some(
                is_node.then(|| uid.clone().zip(rv.clone())).flatten(),
                |row, (u, r)| {
                    let nref = ResourceReference {
                        kind: ResourceKind::Nodes,
                        name: name.clone(),
                        namespace: None,
                        uid: Some(u),
                    };
                    let nref_drain = nref.clone();
                    let r_drain = r.clone();
                    let toggle_action = if node_schedulable {
                        NodeAction::Cordon
                    } else {
                        NodeAction::Uncordon
                    };
                    let toggle_label = if node_schedulable {
                        "Cordon"
                    } else {
                        "Uncordon"
                    };
                    row.child(
                        Button::new(SharedString::from(format!("row-node-cordon-{idx}")))
                            .xsmall()
                            .ghost()
                            .icon(if node_schedulable {
                                IconName::Ban
                            } else {
                                IconName::Check
                            })
                            .tooltip(toggle_label)
                            .accessibility_label(toggle_label)
                            .on_click(self.action(move |this, _, window, cx| {
                                this.run_node_action_for(
                                    nref.clone(),
                                    r.clone(),
                                    toggle_action,
                                    window,
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("row-node-drain-{idx}")))
                            .xsmall()
                            .ghost()
                            .icon(IconName::ArrowDown)
                            .tooltip("Review node drain")
                            .accessibility_label("Review node drain")
                            .on_click(self.action(move |this, _, window, cx| {
                                this.review_node_drain_for(
                                    nref_drain.clone(),
                                    r_drain.clone(),
                                    window,
                                    cx,
                                );
                            })),
                    )
                },
            )
            // CRD row quick action: View Instances
            .when(is_crd, |row| {
                let target_sec = format!("crd:{}", item.name);
                let cid = current_cid;
                row.child(
                    Button::new(SharedString::from(format!("row-crd-inst-{idx}")))
                        .xsmall()
                        .ghost()
                        .icon(IconName::GalleryVerticalEnd)
                        .tooltip("View custom resource instances")
                        .accessibility_label("View custom resource instances")
                        .on_click(self.action(move |this, _, window, cx| {
                            if !cid.is_empty() {
                                this.navigate(
                                    Route::Cluster {
                                        cluster_id: cid.clone(),
                                        section: target_sec.clone(),
                                    },
                                    true,
                                    window,
                                    cx,
                                );
                            }
                        })),
                )
            })
            // Delete row button (for resources with UID + resourceVersion)
            .when_some(
                del_target.filter(|_| kind != ResourceKind::Nodes),
                |row, target| {
                    row.child(
                        Button::new(SharedString::from(format!("row-del-{idx}")))
                            .xsmall()
                            .ghost()
                            .icon(IconName::Delete)
                            .tooltip("Delete resource")
                            .accessibility_label("Delete resource")
                            .on_click(self.action(move |this, _, window, cx| {
                                this.delete_resource_by_target(target.clone(), window, cx);
                            })),
                    )
                },
            )
            .into_any_element()
    }
}

impl Render for ResourceTable {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.cache.stats.renders += 1;
        let Some(data) = self.cache.data.as_ref() else {
            return div().into_any_element();
        };
        let kind = data.scope.kind;
        let cols = columns_for_kind(kind);
        let show_ns = namespaced_kind(kind);
        let page = data.page.clone();
        let indices = self.cache.indices.clone();
        let item_sizes = self.cache.row_sizes.clone();
        let scroll_handle = self.scroll.clone();
        let entity = cx.entity();
        v_flex()
            .flex_1()
            .h_full()
            .min_h_0()
            .min_w_0()
            .w_full()
            .overflow_hidden()
            // One horizontal viewport owns the header and rows so they scroll together.
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .w_full()
                    .id("resource-table-horizontal")
                    .overflow_x_scroll()
                    .track_scroll(&self.horizontal_scroll)
                    .scrollbar(&self.horizontal_scroll, ScrollbarAxis::Horizontal)
                    .child(
                        v_flex()
                            .min_w(px(resource_table_min_width(kind)))
                            .w_full()
                            .flex_1()
                            .min_h_0()
                            .child(
                                h_flex()
                                    .w_full()
                                    .h(px(40.0))
                                    .items_center()
                                    .flex_shrink_0()
                                    .child(
                                        table_cell(
                                            RESOURCE_NAME_WIDTH,
                                            true,
                                            true,
                                            cx.theme().border,
                                        )
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("Name"),
                                    )
                                    .when(show_ns, |el| {
                                        el.child(
                                            table_cell(
                                                RESOURCE_NAMESPACE_WIDTH,
                                                false,
                                                true,
                                                cx.theme().border,
                                            )
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("Namespace"),
                                        )
                                    })
                                    .children(cols.iter().map(|c| {
                                        table_cell(c.width, false, true, cx.theme().border)
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(c.header)
                                    }))
                                    .child(
                                        table_cell(
                                            RESOURCE_AGE_WIDTH,
                                            false,
                                            true,
                                            cx.theme().border,
                                        )
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("Age"),
                                    )
                                    .child(
                                        table_cell(
                                            RESOURCE_STATUS_WIDTH,
                                            false,
                                            true,
                                            cx.theme().border,
                                        )
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("Status"),
                                    )
                                    .child(
                                        table_cell(
                                            RESOURCE_ACTIONS_WIDTH,
                                            false,
                                            false,
                                            cx.theme().border,
                                        )
                                        .justify_end()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("Actions"),
                                    ),
                            )
                            .child(
                                v_virtual_list(
                                    entity,
                                    SharedString::from(format!("virtual-resource-table-{kind:?}")),
                                    item_sizes,
                                    move |this, visible_range, _window, cx| {
                                        visible_range
                                            .filter_map(|idx| {
                                                let item =
                                                    page.as_ref()?.items.get(*indices.get(idx)?)?;
                                                Some(
                                                    div()
                                                        .w_full()
                                                        .h(px(42.0))
                                                        .child(this.render_resource_row(
                                                            kind, item, idx, show_ns, cols, cx,
                                                        ))
                                                        .into_any_element(),
                                                )
                                            })
                                            .collect()
                                    },
                                )
                                .track_scroll(&scroll_handle)
                                .flex_1()
                                .min_h_0()
                                .w_full(),
                            ),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(query: &str, status: Option<&str>) -> ResourceScrollScope {
        ResourceScrollScope {
            cluster_id: "cluster-a".into(),
            kind: ResourceKind::Pods,
            namespace: None,
            query: query.into(),
            status: status.map(str::to_owned),
            page_token: None,
        }
    }

    fn item(name: &str, namespace: &str, status: &str, image: &str) -> ResourceItem {
        ResourceItem {
            name: name.into(),
            namespace: Some(namespace.into()),
            status: status.into(),
            uid: Some(format!("uid-{name}")),
            resource_version: Some("7".into()),
            created_at: None,
            columns: [("image".into(), image.into())].into(),
            node: None,
        }
    }

    fn page() -> Rc<ResourcePage> {
        Rc::new(ResourcePage {
            cluster_id: "cluster-a".into(),
            session_id: "session".into(),
            kind: ResourceKind::Pods,
            read_id: 1,
            items: vec![
                item("Web", "production", "Running", "Nginx"),
                item("worker", "jobs", "Pending", "batch"),
                item("web-old", "production", "NotRunning", "nginx"),
            ],
            truncated: false,
            fetched_at: 0,
            namespace: None,
            continue_token: None,
            metrics: None,
        })
    }

    #[::core::prelude::v1::test]
    fn same_page_and_scope_reuse_filter_and_sizes_for_100_redraws() {
        let page = page();
        let mut cache = TableCache::default();
        assert!(cache.update(TableData {
            page: Some(page.clone()),
            scope: scope("", None)
        }));
        let indices = cache.indices.clone();
        let sizes = cache.row_sizes.clone();
        for _ in 0..100 {
            assert!(!cache.update(TableData {
                page: Some(page.clone()),
                scope: scope("", None)
            }));
            assert!(Rc::ptr_eq(&cache.indices, &indices));
            assert!(Rc::ptr_eq(&cache.row_sizes, &sizes));
        }
        assert_eq!(cache.stats.filter_builds, 1);
        assert_eq!(cache.stats.row_size_builds, 1);
    }

    #[::core::prelude::v1::test]
    fn query_matches_name_namespace_and_columns_status_is_exact_case_insensitive() {
        let page = page();
        let mut cache = TableCache::default();
        for query in ["web", "production", "nginx"] {
            cache.update(TableData {
                page: Some(page.clone()),
                scope: scope(query, Some("rUnNiNg")),
            });
            assert_eq!(cache.indices.as_slice(), &[0]);
        }
        cache.update(TableData {
            page: Some(page.clone()),
            scope: scope("", Some("Pending")),
        });
        assert_eq!(cache.indices.as_slice(), &[1]);
        cache.update(TableData {
            page: Some(page),
            scope: scope("missing", None),
        });
        assert!(cache.indices.is_empty());
    }

    #[::core::prelude::v1::test]
    fn fresh_snapshot_refilters_without_rebuilding_same_length_sizes() {
        let page = page();
        let mut cache = TableCache::default();
        cache.update(TableData {
            page: Some(page.clone()),
            scope: scope("", Some("Running")),
        });
        let sizes = cache.row_sizes.clone();
        let mut updated = (*page).clone();
        updated.items[0].status = "Pending".into();
        updated.items[1].status = "Running".into();
        assert!(cache.update(TableData {
            page: Some(Rc::new(updated)),
            scope: scope("", Some("Running"))
        }));
        assert_eq!(cache.indices.as_slice(), &[1]);
        assert_eq!(cache.stats.filter_builds, 2);
        assert_eq!(cache.stats.row_size_builds, 1);
        assert!(Rc::ptr_eq(&sizes, &cache.row_sizes));
        let mut other_kind = scope("", None);
        other_kind.kind = ResourceKind::Nodes;
        cache.update(TableData {
            page: Some(page),
            scope: other_kind,
        });
        assert!(cache.indices.is_empty());
        cache.update(TableData {
            page: None,
            scope: scope("", None),
        });
        assert!(cache.indices.is_empty());
    }
}
