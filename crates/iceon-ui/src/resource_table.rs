//! Cached resource-table entity. Scrolling never borrows or renders the application shell.
use crate::{
    IceonApp,
    cluster_view::{
        ColumnDef, RESOURCE_ACTIONS_WIDTH, RESOURCE_AGE_WIDTH, RESOURCE_NAME_WIDTH,
        RESOURCE_NAMESPACE_WIDTH, RESOURCE_STATUS_WIDTH, columns_for_kind, namespaced_kind,
        row_status_tag, table_cell,
    },
    inspector_view::DeleteTarget,
    navigation::{Route, cell_value},
    resource_scroll::{ResourceScrollScope, ResourceScrollState},
};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _, VirtualListScrollHandle,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    resizable::{HandleEdge, resize_handle},
    scroll::{ScrollableElement as _, ScrollbarAxis},
    tag::Tag,
    v_flex, v_virtual_list,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use iceon_core::connection::{
    ResourceItem, ResourceKind, ResourcePage, details::ResourceReference, node_actions::NodeAction,
    permissions::PermissionAction, pod_runtime::PodStreamMode, workload_actions::WorkloadAction,
};
use std::{cell::Cell, cmp::Ordering, collections::HashMap, rc::Rc};

fn selection_state(selected: usize, total: usize) -> gpui_kit::base::CheckboxState {
    use gpui_kit::base::CheckboxState;
    if selected == 0 || total == 0 {
        CheckboxState::Unchecked
    } else if selected == total {
        CheckboxState::Checked
    } else {
        CheckboxState::Indeterminate
    }
}

fn event_text_color(status: &str, cx: &App) -> Hsla {
    match status.to_ascii_lowercase().as_str() {
        "warning" => gpui_kit::component::ColorName::Orange.scale(if cx.theme().is_dark() {
            300
        } else {
            600
        }),
        "error" | "failed" => cx.theme().danger,
        _ => cx.theme().muted_foreground,
    }
}

fn deletion_target(kind: ResourceKind, item: &ResourceItem) -> Option<DeleteTarget> {
    if kind == ResourceKind::Nodes {
        return None;
    }

    Some(DeleteTarget {
        kind,
        name: item.name.clone(),
        namespace: item.namespace.clone(),
        uid: item.uid.clone().filter(|value| !value.is_empty())?,
        resource_version: item
            .resource_version
            .clone()
            .filter(|value| !value.is_empty())?,
    })
}

// Geometry, sorting and drag state belong to this entity, never to the shell.
#[derive(Clone)]
struct TableColumn {
    key: &'static str,
    label: &'static str,
    width: f32,
    min: f32,
}

#[derive(Clone)]
struct ColumnLayout {
    columns: Vec<TableColumn>,
    name_resized: bool,
}

impl ColumnLayout {
    fn new(kind: ResourceKind) -> Self {
        if kind == ResourceKind::Events {
            return Self {
                name_resized: false,
                columns: [
                    ("type", "Type", 96., 80.),
                    ("message", "Message", 360., 240.),
                    ("namespace", "Namespace", 144., 112.),
                    ("involvedObject", "Involved Object", 230., 160.),
                    ("source", "Source", 180., 120.),
                    ("count", "Count", 80., 64.),
                    ("age", "Age", 110., 90.),
                    ("lastSeen", "Last Seen", 110., 90.),
                    ("actions", "Actions", 80., 64.),
                ]
                .into_iter()
                .map(|(key, label, width, min)| TableColumn {
                    key,
                    label,
                    width,
                    min,
                })
                .collect(),
            };
        }
        let mut columns = vec![TableColumn {
            key: "name",
            label: if kind == ResourceKind::CustomResourceDefinitions {
                "Resource"
            } else {
                "Name"
            },
            width: RESOURCE_NAME_WIDTH,
            min: 180.0,
        }];
        if namespaced_kind(kind) {
            columns.push(TableColumn {
                key: "namespace",
                label: "Namespace",
                width: RESOURCE_NAMESPACE_WIDTH,
                min: 132.0,
            });
        }

        columns.extend(columns_for_kind(kind).iter().map(|c| {
            let min = if c.key.to_ascii_lowercase().contains("label") {
                160.0
            } else {
                96.0
            };
            TableColumn {
                key: c.key,
                label: c.header,
                width: c.width.max(min),
                min,
            }
        }));
        columns.extend([
            TableColumn {
                key: "age",
                label: "Age",
                width: RESOURCE_AGE_WIDTH,
                min: 164.0,
            },
            TableColumn {
                key: "status",
                label: "Status",
                width: RESOURCE_STATUS_WIDTH,
                min: 110.0,
            },
            TableColumn {
                key: "actions",
                label: "Actions",
                width: RESOURCE_ACTIONS_WIDTH,
                min: 124.0,
            },
        ]);
        Self {
            columns,
            name_resized: false,
        }
    }

    fn width(&self, key: &str) -> f32 {
        self.columns
            .iter()
            .find(|c| c.key == key)
            .map_or(96.0, |c| c.width)
    }

    fn resize(&mut self, key: &str, width: f32) -> bool {
        // Invalid measurements must never poison GPUI layout or destroy saved widths.
        if !width.is_finite() {
            return false;
        }
        let Some(column) = self.columns.iter_mut().find(|c| c.key == key) else {
            return false;
        };
        let width = width.max(column.min);
        let changed = column.width != width || (key == "name" && !self.name_resized);
        column.width = width;
        if key == "name" {
            self.name_resized = true;
        }
        changed
    }

    fn total(&self) -> f32 {
        self.columns.iter().map(|c| c.width).sum()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TableSort {
    key: String,
    descending: bool,
}

fn sort_value(item: &ResourceItem, key: &str) -> Option<String> {
    let value = if key == "age" {
        // Compare creation instants, not rounded display strings such as "2d".
        item.created_at
            .as_deref()?
            .parse::<jiff::Timestamp>()
            .ok()?
            .to_string()
    } else {
        cell_value(item, key)
    };
    (!matches!(value.trim(), "" | "—" | "-" | "<none>" | "N/A")).then_some(value)
}

fn quantity(value: &str, key: &str) -> Option<f64> {
    let value = value.trim();
    let end = value
        .find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-' && c != '+')
        .unwrap_or(value.len());
    let number = value[..end].parse::<f64>().ok()?;
    let suffix = value[end..].trim();
    let factor = match suffix {
        "" | "%" => 1.0,
        "n" if key.to_ascii_lowercase().contains("cpu") => 1e-9,
        "u" if key.to_ascii_lowercase().contains("cpu") => 1e-6,
        "m" if key.to_ascii_lowercase().contains("cpu") => 1e-3,
        "Ki" | "KiB" => 1024.0,
        "Mi" | "MiB" => 1024.0_f64.powi(2),
        "Gi" | "GiB" => 1024.0_f64.powi(3),
        "Ti" | "TiB" => 1024.0_f64.powi(4),
        "K" | "k" | "KB" => 1e3,
        "M" | "MB" => 1e6,
        "G" | "GB" => 1e9,
        "T" | "TB" => 1e12,
        _ => return None,
    };
    let result = number * factor;
    result.is_finite().then_some(result)
}

/// Digit runs compare numerically (pod-2 before pod-10, 2/10 before 10/10),
/// without integer overflow even for arbitrarily long resource identifiers.
fn natural_compare(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let (start_i, start_j) = (i, j);
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let mut x = &a[start_i..i];
            let mut y = &b[start_j..j];
            while x.first() == Some(&b'0') {
                x = &x[1..];
            }
            while y.first() == Some(&b'0') {
                y = &y[1..];
            }
            let order = x.len().cmp(&y.len()).then_with(|| x.cmp(y));
            if order != Ordering::Equal {
                return order;
            }
        } else {
            let order = a[i].to_ascii_lowercase().cmp(&b[j].to_ascii_lowercase());
            if order != Ordering::Equal {
                return order;
            }
            i += 1;
            j += 1;
        }
    }
    (a.len() - i).cmp(&(b.len() - j))
}

fn compare_values(a: Option<&str>, b: Option<&str>, sort: &TableSort) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, _) => Ordering::Greater,
        (_, None) => Ordering::Less,
        (Some(a), Some(b)) => {
            let order = if sort.key == "age" {
                match (a.parse::<jiff::Timestamp>(), b.parse::<jiff::Timestamp>()) {
                    (Ok(a), Ok(b)) => a.cmp(&b),
                    _ => natural_compare(a, b),
                }
            } else {
                match (quantity(a, &sort.key), quantity(b, &sort.key)) {
                    (Some(a), Some(b)) => a.total_cmp(&b),
                    (Some(_), None) => Ordering::Less,
                    (None, Some(_)) => Ordering::Greater,
                    (None, None) => natural_compare(a, b),
                }
            };
            if sort.descending {
                order.reverse()
            } else {
                order
            }
        }
    }
}

#[derive(Clone)]
struct ColumnResize {
    kind: ResourceKind,
    key: &'static str,
    measured_width: Rc<Cell<f32>>,
    start: Rc<Cell<(f32, f32)>>,
}
impl Render for ColumnResize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

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
    sorts: HashMap<ResourceKind, TableSort>,
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
        self.apply_sort();
        true
    }
    fn apply_sort(&mut self) {
        let Some(data) = self.data.as_ref() else {
            return;
        };
        let Some(page) = data.page.as_ref() else {
            return;
        };
        let indices = Rc::make_mut(&mut self.indices);
        if let Some(sort) = self.sorts.get(&data.scope.kind) {
            // Materialize each key once, never allocate strings inside the comparator.
            let values: HashMap<usize, Option<String>> = indices
                .iter()
                .map(|&i| (i, sort_value(&page.items[i], &sort.key)))
                .collect();
            indices.sort_by(|a, b| {
                compare_values(values[a].as_deref(), values[b].as_deref(), sort)
                    .then_with(|| a.cmp(b))
            });
        } else {
            indices.sort_unstable(); // third click restores API/source order
        }
    }

    fn cycle_sort(&mut self, key: &str) -> bool {
        let Some(data) = &self.data else {
            return false;
        };
        let kind = data.scope.kind;
        if !ColumnLayout::new(kind)
            .columns
            .iter()
            .any(|c| c.key == key && c.key != "actions")
        {
            return false;
        }
        match self.sorts.get_mut(&kind) {
            Some(sort) if sort.key == key && !sort.descending => sort.descending = true,
            Some(sort) if sort.key == key => {
                self.sorts.remove(&kind);
            }
            _ => {
                self.sorts.insert(
                    kind,
                    TableSort {
                        key: key.into(),
                        descending: false,
                    },
                );
            }
        }
        self.apply_sort();
        true
    }
}

pub(crate) struct ResourceTable {
    owner: WeakEntity<IceonApp>,
    selection_enabled: bool,
    selected: Vec<DeleteTarget>,
    selection_lease: String,
    cache: TableCache,
    scroll: VirtualListScrollHandle,
    scroll_state: ResourceScrollState,
    horizontal_scroll: ScrollHandle,
    layouts: HashMap<ResourceKind, ColumnLayout>,
    header_bounds: HashMap<&'static str, Rc<Cell<Bounds<Pixels>>>>,
}

impl ResourceTable {
    pub(crate) fn new(owner: WeakEntity<IceonApp>, cx: &mut Context<Self>) -> Self {
        // Access reviews update the owner without changing the cached page/scope.
        // Redraw this independently cached entity so newly reviewed actions reveal.
        if let Some(app) = owner.upgrade() {
            cx.observe(&app, |_, _, cx| cx.notify()).detach();
        }
        Self {
            owner,
            selection_enabled: true,
            selected: Vec::new(),
            selection_lease: String::new(),
            cache: TableCache::default(),
            scroll: VirtualListScrollHandle::new(),
            scroll_state: ResourceScrollState::default(),
            horizontal_scroll: ScrollHandle::default(),
            layouts: HashMap::new(),
            header_bounds: HashMap::new(),
        }
    }

    pub(crate) fn without_selection(mut self) -> Self {
        self.selection_enabled = false;
        self
    }

    pub(crate) fn sync(&mut self, data: TableData, cx: &mut Context<Self>) {
        self.layouts
            .entry(data.scope.kind)
            .or_insert_with(|| ColumnLayout::new(data.scope.kind));
        let scope_changed = self
            .cache
            .data
            .as_ref()
            .is_none_or(|old| old.scope != data.scope);
        if scope_changed {
            self.selected.clear();
        }
        let scroll = self.scroll_state.for_scope(data.scope.clone());
        if self.cache.update(data) {
            let loaded = self.loaded_targets();
            self.selected.retain(|target| loaded.contains(target));
            self.scroll = scroll;
            if scope_changed {
                self.horizontal_scroll = ScrollHandle::default();
            }
            cx.notify();
        }
    }

    pub(crate) fn selected_targets(&self) -> Vec<DeleteTarget> {
        self.selected.clone()
    }
    pub(crate) fn clear_selection(&mut self, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            self.selected.clear();
            cx.notify();
        }
    }
    pub(crate) fn sync_selection_lease(&mut self, lease: String, cx: &mut Context<Self>) {
        if self.selection_lease != lease {
            self.selection_lease = lease;
            self.clear_selection(cx);
        }
    }
    fn selection_changed(&self, cx: &mut Context<Self>) {
        let owner = self.owner.clone();
        cx.defer(move |cx| {
            let _ = owner.update(cx, |_, cx| cx.notify());
        });
        cx.notify();
    }
    fn render_selection_checkbox(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::base::{Checkbox as BaseCheckbox, CheckboxState};
        let total = self.selectable_targets(cx).len();
        let state = selection_state(self.selected.len(), total);
        let active = state != CheckboxState::Unchecked;
        let owner = cx.entity().downgrade();
        BaseCheckbox::new("select-loaded-resources")
            .state(state)
            .disabled(total == 0)
            .accessibility_label(format!("Select all {total} loaded selectable resources"))
            .cursor_pointer()
            .rounded(px(4.0))
            .border_1()
            .border_color(if active {
                cx.theme().primary
            } else {
                cx.theme().input
            })
            .bg(if active {
                cx.theme().primary
            } else {
                cx.theme().input_background()
            })
            .text_color(cx.theme().primary_foreground)
            .size(px(16.0))
            .flex()
            .items_center()
            .justify_center()
            .child(match state {
                CheckboxState::Indeterminate => gpui_kit::component::Icon::new(IconName::Minus)
                    .xsmall()
                    .into_any_element(),
                CheckboxState::Checked => gpui_kit::component::Icon::new(IconName::Check)
                    .xsmall()
                    .into_any_element(),
                CheckboxState::Unchecked => Empty.into_any_element(),
            })
            .on_change(move |next, _, _, cx| {
                cx.stop_propagation();
                let _ = owner.update(cx, |table, cx| {
                    table.selected = if next == CheckboxState::Checked {
                        table.selectable_targets(cx)
                    } else {
                        Vec::new()
                    };
                    table.selection_changed(cx);
                });
            })
    }

    fn selectable_targets(&self, cx: &App) -> Vec<DeleteTarget> {
        self.loaded_targets()
            .into_iter()
            .filter(|target| {
                self.owner
                    .upgrade()
                    .is_some_and(|owner| owner.read(cx).can_delete_target(target))
            })
            .collect()
    }

    fn loaded_targets(&self) -> Vec<DeleteTarget> {
        let Some(data) = &self.cache.data else {
            return Vec::new();
        };
        let Some(page) = &data.page else {
            return Vec::new();
        };
        self.cache
            .indices
            .iter()
            .filter_map(|index| deletion_target(data.scope.kind, &page.items[*index]))
            .collect()
    }

    #[cfg(debug_assertions)]
    pub(crate) fn stats(&self) -> TableStats {
        self.cache.stats
    }

    #[cfg(debug_assertions)]
    pub(crate) fn scroll_offsets(&self) -> (Point<Pixels>, Point<Pixels>) {
        (self.scroll.offset(), self.horizontal_scroll.offset())
    }

    // Also used by the debug performance probe: these are the production paths.
    pub(crate) fn sort_by_key(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.cache.cycle_sort(key) {
            cx.notify();
        }
    }

    pub(crate) fn resize_column(&mut self, key: &str, width: f32, cx: &mut Context<Self>) {
        let Some(kind) = self.cache.data.as_ref().map(|d| d.scope.kind) else {
            return;
        };
        self.resize_for_kind(kind, key, width, cx);
    }

    fn resize_for_kind(
        &mut self,
        kind: ResourceKind,
        key: &str,
        width: f32,
        cx: &mut Context<Self>,
    ) {
        if self
            .layouts
            .entry(kind)
            .or_insert_with(|| ColumnLayout::new(kind))
            .resize(key, width)
        {
            cx.notify();
        }
    }

    #[cfg(debug_assertions)]
    pub(crate) fn column_widths(&self) -> Vec<(String, f32, f32)> {
        self.cache
            .data
            .as_ref()
            .and_then(|d| self.layouts.get(&d.scope.kind))
            .map(|layout| {
                layout
                    .columns
                    .iter()
                    .map(|c| (c.key.into(), c.width, c.min))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[cfg(debug_assertions)]
    pub(crate) fn visible_order(&self) -> Vec<String> {
        let Some(page) = self.cache.data.as_ref().and_then(|data| data.page.as_ref()) else {
            return Vec::new();
        };
        self.cache
            .indices
            .iter()
            .map(|index| page.items[*index].name.clone())
            .collect()
    }

    #[cfg(debug_assertions)]
    pub(crate) fn column_header_bounds(&self, key: &str) -> Option<Bounds<Pixels>> {
        self.header_bounds.get(key).map(|bounds| bounds.get())
    }

    pub(crate) fn filter_namespace(
        app: &mut IceonApp,
        namespace: &str,
        window: &mut Window,
        cx: &mut Context<IceonApp>,
    ) {
        if namespace.is_empty() {
            return;
        }
        app.selected_namespace = Some(namespace.to_owned());
        app.current_continue_token = None;
        app.continue_token_history.clear();
        // The parent render synchronizes the standard namespace Select to this value.
        app.reload_active_section(window, cx);
    }

    fn namespace_button(&self, namespace: String, idx: usize, foreground: Hsla) -> Button {
        Button::new(SharedString::from(format!("namespace-row-{idx}")))
            .xsmall()
            .ghost()
            .text_color(foreground)
            .max_w_full()
            .label(namespace.clone())
            .tooltip(format!("Filter namespace {namespace}"))
            .accessibility_label(format!("Filter namespace {namespace}"))
            .on_click(self.action(move |app, _, window, cx| {
                Self::filter_namespace(app, &namespace, window, cx)
            }))
    }

    fn render_header(
        &self,
        kind: ResourceKind,
        column: &TableColumn,
        flexible: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = column.key;
        let width = Rc::new(Cell::new(column.width));
        let measured = width.clone();
        let header_bounds = self
            .header_bounds
            .get(key)
            .expect("measured header")
            .clone();
        let resize_width = width.clone();
        let sort = self.cache.sorts.get(&kind).filter(|s| s.key == key);
        let label = format!(
            "{}{}",
            column.label,
            sort.map_or("", |s| if s.descending { " ↓" } else { " ↑" })
        );
        let sort_state = sort.map_or("API order", |s| {
            if s.descending {
                "descending"
            } else {
                "ascending"
            }
        });
        let help = if key == "actions" {
            "Drag separator to resize. Shift+Left/Right resizes by 16 pixels.".to_owned()
        } else {
            format!(
                "Sort {} among currently loaded results (ascending, descending, API order). Drag separator or Shift+Left/Right to resize.",
                column.label
            )
        };
        table_cell(column.width, flexible, true, cx.theme().border)
            .relative()
            .child(
                canvas(
                    move |bounds, _, _| {
                        measured.set(f32::from(bounds.size.width));
                        header_bounds.set(bounds);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .child(
                Button::new(SharedString::from(format!("table-sort-{key}")))
                    .xsmall()
                    .ghost()
                    .w_full()
                    .min_w_0()
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .truncate()
                            .when(key == "actions", |label| label.text_right())
                            .when(key != "actions", |label| label.text_left())
                            .child(label),
                    )
                    .tooltip(help.clone())
                    .accessibility_label(format!("{}, {sort_state}. {help}", column.label))
                    .on_click(cx.listener(move |this, _, _, cx| this.sort_by_key(key, cx)))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                        if event.keystroke.modifiers.shift {
                            let delta = match event.keystroke.key.as_str() {
                                "left" => -16.0,
                                "right" => 16.0,
                                _ => return,
                            };
                            this.resize_column(key, resize_width.get() + delta, cx);
                            cx.stop_propagation();
                        }
                    })),
            )
            .child(
                resize_handle(
                    SharedString::from(format!("table-resize-{key}")),
                    Axis::Horizontal,
                )
                .inside(HandleEdge::Trailing)
                .with_appearance(Rc::new(|_, _, _| Some(div().into_any_element())))
                .on_drag(
                    ColumnResize {
                        kind,
                        key,
                        measured_width: width,
                        start: Rc::new(Cell::new((0.0, 0.0))),
                    },
                    |drag, _, window, cx| {
                        drag.start.set((
                            f32::from(window.mouse_position().x),
                            drag.measured_width.get(),
                        ));
                        cx.new(|_| drag.clone())
                    },
                ),
            )
            .into_any_element()
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
            cx.stop_propagation();
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
        if kind == ResourceKind::Events {
            return self.render_event_row(item, idx, cx);
        }

        // The render/sync path installs this once per kind. Visible rows only borrow
        // geometry; scrolling does not allocate or rebuild column metadata.
        let target = deletion_target(kind, item).filter(|target| {
            self.owner
                .upgrade()
                .is_some_and(|owner| owner.read(cx).can_delete_target(target))
        });
        let selected = target
            .as_ref()
            .is_some_and(|target| self.selected.contains(target));
        let layout = self.layouts.get(&kind).expect("resource column layout");
        let reference = ResourceReference {
            kind,
            name: item.name.clone(),
            namespace: item.namespace.clone(),
            uid: item.uid.clone(),
        };
        let row_reference = reference.clone();
        let row_actions = self
            .render_row_actions(kind, item, idx, cx)
            .into_any_element();
        let timezone = cx
            .try_global::<crate::preferences_view::GlobalPreferencesState>()
            .map(|state| state.preferences.app.timezone.clone())
            .unwrap_or_else(|| "local".to_owned());

        h_flex()
            .id(SharedString::from(format!("resource-row-{idx}")))
            .cursor_pointer()
            .hover(|row| row.bg(cx.theme().muted))
            .on_click(self.action(move |app, _, window, cx| {
                app.open_resource_inspector(row_reference.clone(), window, cx);
            }))
            .w_full()
            .h(px(42.0))
            .border_b_1()
            .border_color(cx.theme().border)
            .items_center()
            .when(self.selection_enabled, |row| {
                row.child(
                    div()
                        .w(px(36.0))
                        .flex_shrink_0()
                        .flex()
                        .justify_center()
                        .child(
                            Checkbox::new(SharedString::from(format!("select-resource-{idx}")))
                                .checked(selected)
                                .disabled(target.is_none())
                                .on_click(cx.listener(move |table, checked: &bool, _, cx| {
                                    cx.stop_propagation();
                                    if let Some(target) = &target {
                                        if *checked {
                                            if !table.owner.upgrade().is_some_and(|owner| {
                                                owner.read(cx).can_delete_target(target)
                                            }) {
                                                return;
                                            }
                                            if !table.selected.contains(target) {
                                                table.selected.push(target.clone());
                                            }
                                        } else {
                                            table.selected.retain(|candidate| candidate != target);
                                        }
                                        table.selection_changed(cx);
                                    }
                                })),
                        ),
                )
            })
            .child(
                table_cell(
                    layout.width("name"),
                    !layout.name_resized,
                    true,
                    cx.theme().border,
                )
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(cx.theme().sidebar_accent_foreground)
                .truncate()
                .child(if kind == ResourceKind::Namespaces {
                    self.namespace_button(
                        item.name.clone(),
                        idx,
                        cx.theme().sidebar_accent_foreground,
                    )
                    .into_any_element()
                } else {
                    crate::selectable_text::selectable_text(
                        if kind == ResourceKind::CustomResourceDefinitions {
                            item.columns
                                .get("kind")
                                .cloned()
                                .unwrap_or_else(|| item.name.clone())
                        } else {
                            item.name.clone()
                        },
                    )
                    .text_color(cx.theme().sidebar_accent_foreground)
                    .into_any_element()
                }),
            )
            .when(show_ns, |el| {
                el.child(
                    table_cell(layout.width("namespace"), false, true, cx.theme().border)
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .truncate()
                        .when_some(item.namespace.clone(), |cell, ns| {
                            cell.child(self.namespace_button(
                                ns,
                                idx,
                                cx.theme().sidebar_accent_foreground,
                            ))
                        }),
                )
            })
            .children(cols.iter().map(|c| {
                if kind == ResourceKind::Pods && c.key == "containers" {
                    table_cell(layout.width(c.key), false, true, cx.theme().border)
                        .child(crate::pod_columns::containers(item, idx, cx))
                        .into_any_element()
                } else if kind == ResourceKind::Pods && matches!(c.key, "cpu" | "memory") {
                    table_cell(layout.width(c.key), false, true, cx.theme().border)
                        .text_xs()
                        .child(crate::selectable_text::selectable_text(
                            item.columns
                                .get(c.key)
                                .cloned()
                                .unwrap_or_else(|| "N/A".into()),
                        ))
                        .into_any_element()
                } else if kind == ResourceKind::Namespaces && c.key == "labels" {
                    let extra_count = item
                        .columns
                        .get("labels")
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(0)
                        .saturating_sub(1);
                    table_cell(layout.width(c.key), false, true, cx.theme().border)
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
                    table_cell(layout.width(c.key), false, true, cx.theme().border)
                        .text_xs()
                        .truncate()
                        .child(crate::selectable_text::selectable_text(cell_value(
                            item, c.key,
                        )))
                        .into_any_element()
                }
            }))
            .child(
                table_cell(layout.width("age"), false, true, cx.theme().border)
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .truncate()
                    .child(crate::age_label::age_label(
                        SharedString::from(format!("resource-age-{idx}")),
                        item.created_at.as_deref(),
                        &timezone,
                    )),
            )
            .child(
                table_cell(layout.width("status"), false, true, cx.theme().border)
                    .items_center()
                    .child(row_status_tag(&item.status)),
            )
            .when(layout.name_resized, |row| row.child(div().flex_1()))
            .child(
                table_cell(layout.width("actions"), false, false, cx.theme().border)
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

    fn render_event_row(
        &self,
        item: &ResourceItem,
        idx: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let layout = self
            .layouts
            .get(&ResourceKind::Events)
            .expect("event layout");
        let reference = ResourceReference {
            kind: ResourceKind::Events,
            namespace: item.namespace.clone(),
            name: item.name.clone(),
            uid: item.uid.clone(),
        };
        let row_reference = reference.clone();
        let timezone = cx
            .try_global::<crate::preferences_view::GlobalPreferencesState>()
            .map(|state| state.preferences.app.timezone.clone())
            .unwrap_or_else(|| "local".into());
        let severity = event_text_color(&item.status, cx);
        h_flex()
            .id(SharedString::from(format!("event-row-{idx}")))
            .w_full()
            .h(px(42.))
            .items_center()
            .border_b_1()
            .border_color(cx.theme().border)
            .cursor_pointer()
            .hover(|row| row.bg(cx.theme().muted))
            .on_click(self.action(move |app, _, window, cx| {
                app.open_resource_inspector(row_reference.clone(), window, cx)
            }))
            .children(layout.columns.iter().map(|column| {
                let cell = table_cell(
                    column.width,
                    false,
                    column.key != "actions",
                    cx.theme().border,
                )
                .text_xs()
                .items_center();
                match column.key {
                    "type" => cell
                        .text_color(severity)
                        .child(
                            crate::selectable_text::selectable_text(item.status.clone())
                                .text_color(severity),
                        )
                        .into_any_element(),
                    "message" => cell
                        .truncate()
                        .text_color(severity)
                        .child(
                            crate::selectable_text::selectable_text(cell_value(item, "message"))
                                .text_color(severity),
                        )
                        .into_any_element(),
                    "namespace" => cell
                        .when_some(item.namespace.clone(), |cell, namespace| {
                            cell.child(self.namespace_button(
                                namespace,
                                idx,
                                cx.theme().sidebar_accent_foreground,
                            ))
                        })
                        .into_any_element(),
                    "age" | "lastSeen" => cell
                        .text_color(cx.theme().muted_foreground)
                        .child(crate::age_label::age_label(
                            SharedString::from(format!("event-{}-{idx}", column.key)),
                            if column.key == "age" {
                                item.created_at.as_deref()
                            } else {
                                item.columns.get("lastSeen").map(String::as_str)
                            },
                            &timezone,
                        ))
                        .into_any_element(),
                    "actions" => cell
                        .justify_end()
                        .child(
                            Button::new(SharedString::from(format!("inspect-event-{idx}")))
                                .xsmall()
                                .ghost()
                                .icon(IconName::Eye)
                                .tooltip("Inspect event")
                                .accessibility_label("Inspect event")
                                .on_click(self.action({
                                    let reference = reference.clone();
                                    move |app, _, window, cx| {
                                        app.open_resource_inspector(reference.clone(), window, cx)
                                    }
                                })),
                        )
                        .into_any_element(),
                    "involvedObject" => cell
                        .truncate()
                        .text_color(cx.theme().sidebar_accent_foreground)
                        .child(
                            crate::selectable_text::selectable_text(cell_value(item, column.key))
                                .text_color(cx.theme().sidebar_accent_foreground),
                        )
                        .into_any_element(),
                    _ => cell
                        .truncate()
                        .text_color(cx.theme().muted_foreground)
                        .child(crate::selectable_text::selectable_text(cell_value(
                            item, column.key,
                        )))
                        .into_any_element(),
                }
            }))
            .into_any_element()
    }

    fn render_row_actions(
        &self,
        kind: ResourceKind,
        item: &ResourceItem,
        idx: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let allowed = |action| {
            self.owner.upgrade().is_some_and(|owner| {
                owner
                    .read(cx)
                    .can_action(kind, action, item.namespace.as_deref(), Some(&item.name))
            })
        };
        let get = allowed(PermissionAction::Get);
        let patch = get && allowed(PermissionAction::Patch);
        let delete = get && allowed(PermissionAction::Delete);
        let logs = get && allowed(PermissionAction::Logs);
        let exec = get && allowed(PermissionAction::Exec);
        let trigger = get
            && self.owner.upgrade().is_some_and(|owner| {
                owner.read(cx).can_action(
                    ResourceKind::Jobs,
                    PermissionAction::Create,
                    item.namespace.as_deref(),
                    None,
                )
            });
        let drain = patch
            && self.owner.upgrade().is_some_and(|owner| {
                owner
                    .read(cx)
                    .can_action(ResourceKind::Pods, PermissionAction::List, None, None)
            });
        let forward = get
            && self.owner.upgrade().is_some_and(|owner| {
                owner.read(cx).can_action(
                    ResourceKind::Pods,
                    PermissionAction::List,
                    item.namespace.as_deref(),
                    None,
                )
            });
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
                        .disabled(!logs)
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
                        .disabled(!exec)
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
                            .disabled(!patch)
                            .xsmall()
                            .ghost()
                            .text_color(cx.theme().danger)
                            .icon(IconName::RefreshCw)
                            .tooltip("Restart workload")
                            .accessibility_label("Restart workload")
                            .on_click(self.action(move |this, _, window, cx| {
                                let reference = wref.clone();
                                let version = r.clone();
                                this.confirm_destructive_action(
                                    "Restart workload?",
                                    format!(
                                        "Restart {}? This can interrupt running workloads.",
                                        reference.name
                                    ),
                                    "Restart",
                                    window,
                                    cx,
                                    move |app, window, cx| {
                                        app.run_workload_action_for(
                                            reference.clone(),
                                            version.clone(),
                                            WorkloadAction::Restart,
                                            None,
                                            window,
                                            cx,
                                        );
                                    },
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
                            .disabled(!trigger)
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
                            .disabled(!forward)
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
                            .disabled(!patch)
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
                            .disabled(!drain)
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
                        .disabled(true)
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
                            .disabled(!delete)
                            .xsmall()
                            .ghost()
                            .text_color(cx.theme().danger)
                            .icon(IconName::Delete)
                            .tooltip("Delete resource")
                            .accessibility_label("Delete resource")
                            .on_click(self.action(move |this, _, window, cx| {
                                let target = target.clone();
                                this.confirm_destructive_action(
                                    "Delete resource?",
                                    format!("Delete {}? This cannot be undone.", target.name),
                                    "Delete",
                                    window,
                                    cx,
                                    move |app, window, cx| {
                                        app.delete_resource_by_target(target.clone(), window, cx);
                                    },
                                );
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
        let layout = self
            .layouts
            .entry(kind)
            .or_insert_with(|| ColumnLayout::new(kind))
            .clone();
        for column in &layout.columns {
            self.header_bounds
                .entry(column.key)
                .or_insert_with(|| Rc::new(Cell::new(Bounds::default())));
        }
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
            .id("resource-table-resize-surface")
            .on_drag_move(
                cx.listener(|this, event: &DragMoveEvent<ColumnResize>, _, cx| {
                    let drag = event.drag(cx).clone();
                    let (start_x, start_width) = drag.start.get();
                    this.resize_for_kind(
                        drag.kind,
                        drag.key,
                        start_width + f32::from(event.event.position.x) - start_x,
                        cx,
                    );
                }),
            )
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
                            .min_w(px(layout.total()
                                + if self.selection_enabled && kind != ResourceKind::Events {
                                    36.0
                                } else {
                                    0.0
                                }))
                            .w_full()
                            .flex_1()
                            .min_h_0()
                            .child(
                                h_flex()
                                    .w_full()
                                    .h(px(40.0))
                                    .border_b_1()
                                    .border_color(cx.theme().border)
                                    .items_center()
                                    .flex_shrink_0()
                                    .when(
                                        self.selection_enabled && kind != ResourceKind::Events,
                                        |header| {
                                            header.child(
                                                div()
                                                    .w(px(36.0))
                                                    .flex_shrink_0()
                                                    .flex()
                                                    .justify_center()
                                                    .child(self.render_selection_checkbox(cx)),
                                            )
                                        },
                                    )
                                    .children(
                                        layout.columns.iter().filter(|c| c.key != "actions").map(
                                            |column| {
                                                self.render_header(
                                                    kind,
                                                    column,
                                                    column.key == "name" && !layout.name_resized,
                                                    cx,
                                                )
                                            },
                                        ),
                                    )
                                    .when(layout.name_resized, |row| row.child(div().flex_1()))
                                    .child(self.render_header(
                                        kind,
                                        layout.columns.last().expect("actions column"),
                                        false,
                                        cx,
                                    )),
                            )
                            .child(
                                v_virtual_list(
                                    entity,
                                    SharedString::from(format!("virtual-resource-table-{kind:?}")),
                                    item_sizes,
                                    move |this, visible_range, window, cx| {
                                        if page.as_ref().is_some_and(|p| p.continue_token.is_some())
                                            && visible_range.end.saturating_add(12) >= indices.len()
                                        {
                                            let owner = this.owner.clone();
                                            window.defer(cx, move |window, cx| {
                                                let _ = owner.update(cx, |app, cx| {
                                                    app.load_more_resources(kind, window, cx)
                                                });
                                            });
                                        }
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
    #[::core::prelude::v1::test]
    fn events_columns_match_reference_without_duplicate_name_or_status() {
        let layout = super::ColumnLayout::new(iceon_core::connection::ResourceKind::Events);
        assert_eq!(
            layout
                .columns
                .iter()
                .map(|column| column.key)
                .collect::<Vec<_>>(),
            [
                "type",
                "message",
                "namespace",
                "involvedObject",
                "source",
                "count",
                "age",
                "lastSeen",
                "actions"
            ]
        );
    }
    use super::*;
    #[::core::prelude::v1::test]
    fn header_selection_is_mixed_only_for_partial_selection() {
        use gpui_kit::base::CheckboxState::*;
        assert_eq!(selection_state(0, 0), Unchecked);
        assert_eq!(selection_state(0, 5), Unchecked);
        assert_eq!(selection_state(1, 5), Indeterminate);
        assert_eq!(selection_state(4, 5), Indeterminate);
        assert_eq!(selection_state(5, 5), Checked);
    }

    #[::core::prelude::v1::test]
    fn checklist_requires_reviewed_identity_and_never_selects_nodes() {
        let mut row = item("web", "production", "Running", "nginx");
        assert!(deletion_target(ResourceKind::Pods, &row).is_some());
        assert!(deletion_target(ResourceKind::Nodes, &row).is_none());
        row.uid = None;
        assert!(deletion_target(ResourceKind::Pods, &row).is_none());
        row.uid = Some("uid-web".into());
        row.resource_version = Some(String::new());
        assert!(deletion_target(ResourceKind::Pods, &row).is_none());
    }

    #[::core::prelude::v1::test]
    fn checklist_identity_changes_on_replacement_or_new_resource_version() {
        let mut row = item("web", "production", "Running", "nginx");
        let snapshot = deletion_target(ResourceKind::Pods, &row).unwrap();
        row.uid = Some("replacement-uid".into());
        assert_ne!(
            Some(snapshot.clone()),
            deletion_target(ResourceKind::Pods, &row)
        );
        row.uid = Some(snapshot.uid.clone());
        row.resource_version = Some("8".into());
        assert_ne!(Some(snapshot), deletion_target(ResourceKind::Pods, &row));
    }

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
            containers: Vec::new(),
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
    #[::core::prelude::v1::test]
    fn sorting_cycles_reapplies_to_live_pages_and_preserves_size_cache() {
        let page = page();
        let mut cache = TableCache::default();
        cache.update(TableData {
            page: Some(page.clone()),
            scope: scope("", None),
        });
        let sizes = cache.row_sizes.clone();
        assert!(cache.cycle_sort("name"));
        assert_eq!(cache.indices.as_slice(), &[0, 2, 1]);
        assert_eq!(cache.stats.filter_builds, 1);
        assert!(cache.cycle_sort("name"));
        assert_eq!(cache.indices.as_slice(), &[1, 2, 0]);
        let mut live = (*page).clone();
        live.items[0].name = "zzz".into();
        cache.update(TableData {
            page: Some(Rc::new(live)),
            scope: scope("", None),
        });
        assert_eq!(cache.indices.as_slice(), &[0, 1, 2]);
        assert!(Rc::ptr_eq(&sizes, &cache.row_sizes));
        assert!(cache.cycle_sort("name"));
        assert_eq!(cache.indices.as_slice(), &[0, 1, 2]);
        assert!(!cache.cycle_sort("actions"));
        assert!(!cache.cycle_sort("unknown"));
    }

    #[::core::prelude::v1::test]
    fn quantities_natural_counts_missing_and_creation_instants_sort_correctly() {
        let mut sort = TableSort {
            key: "cpu".into(),
            descending: false,
        };
        assert_eq!(
            compare_values(Some("900m"), Some("2"), &sort),
            Ordering::Less
        );
        sort.key = "memory".into();
        assert_eq!(
            compare_values(Some("900Mi"), Some("1Gi"), &sort),
            Ordering::Less
        );
        assert_eq!(natural_compare("pod-2", "pod-10"), Ordering::Less);
        assert_eq!(natural_compare("2/10", "10/10"), Ordering::Less);
        for descending in [false, true] {
            sort.descending = descending;
            assert_eq!(compare_values(None, Some("1Gi"), &sort), Ordering::Greater);
            assert_eq!(compare_values(Some("1Gi"), None, &sort), Ordering::Less);
        }
        sort = TableSort {
            key: "age".into(),
            descending: false,
        };
        assert_eq!(
            compare_values(
                Some("2026-01-01T01:00:00+01:00"),
                Some("2026-01-01T00:00:00.1Z"),
                &sort
            ),
            Ordering::Less
        );
        let mut resource = item("missing", "default", "Running", "nginx");
        resource.created_at = Some("invalid".into());
        assert_eq!(sort_value(&resource, "age"), None);
    }

    #[::core::prelude::v1::test]
    fn layouts_clamp_fractional_widths_and_persist_per_kind() {
        let mut layouts = HashMap::new();
        for kind in [
            ResourceKind::Pods,
            ResourceKind::Namespaces,
            ResourceKind::Nodes,
        ] {
            layouts.insert(kind, ColumnLayout::new(kind));
        }
        let pods = layouts.get_mut(&ResourceKind::Pods).unwrap();
        assert!(pods.resize("name", 351.25));
        assert_eq!(pods.width("name"), 351.25);
        assert!(pods.name_resized);
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(!pods.resize("name", invalid));
            assert_eq!(pods.width("name"), 351.25);
        }
        let keys: Vec<_> = pods.columns.iter().map(|c| c.key).collect();
        for key in keys {
            pods.resize(key, -0.25);
        }
        for column in &pods.columns {
            assert_eq!(column.width, column.min);
        }
        assert_eq!(pods.width("actions"), 124.0);
        assert_eq!(pods.width("namespace"), 132.0);
        assert!(!layouts[&ResourceKind::Nodes].name_resized);
        assert!(
            layouts[&ResourceKind::Namespaces]
                .columns
                .iter()
                .filter(|c| c.key.contains("label"))
                .all(|c| c.min >= 160.0)
        );
        layouts
            .get_mut(&ResourceKind::Pods)
            .unwrap()
            .resize("name", 351.25);
        assert_eq!(layouts[&ResourceKind::Pods].width("name"), 351.25);
    }

    #[::core::prelude::v1::test]
    fn sorted_cache_is_untouched_for_100_identical_snapshots() {
        let page = page();
        let mut cache = TableCache::default();
        cache.update(TableData {
            page: Some(page.clone()),
            scope: scope("", None),
        });
        cache.cycle_sort("namespace");
        let indices = cache.indices.clone();
        let sizes = cache.row_sizes.clone();
        for _ in 0..100 {
            assert!(!cache.update(TableData {
                page: Some(page.clone()),
                scope: scope("", None)
            }));
            assert!(Rc::ptr_eq(&indices, &cache.indices));
            assert!(Rc::ptr_eq(&sizes, &cache.row_sizes));
        }
        assert_eq!(cache.stats.filter_builds, 1);
        assert_eq!(cache.stats.row_size_builds, 1);
    }
    #[::core::prelude::v1::test]
    fn dynamic_sort_keeps_missing_last_and_equal_keys_in_source_order() {
        let mut snapshot = (*page()).clone();
        snapshot.kind = ResourceKind::Nodes;
        let mut node_scope = scope("", None);
        node_scope.kind = ResourceKind::Nodes;
        snapshot.items[0].columns.insert("cpu".into(), "2".into());
        snapshot.items[1]
            .columns
            .insert("cpu".into(), "900m".into());
        snapshot.items[2]
            .columns
            .insert("cpu".into(), "900m".into());
        let mut cache = TableCache::default();
        cache.update(TableData {
            page: Some(Rc::new(snapshot.clone())),
            scope: node_scope.clone(),
        });
        assert!(cache.cycle_sort("cpu"));
        assert_eq!(cache.indices.as_slice(), &[1, 2, 0]);
        snapshot.items[1].columns.remove("cpu");
        cache.update(TableData {
            page: Some(Rc::new(snapshot)),
            scope: node_scope.clone(),
        });
        assert_eq!(cache.indices.as_slice(), &[2, 0, 1]);
        cache.cycle_sort("cpu");
        assert_eq!(cache.indices.as_slice(), &[0, 2, 1]);
    }
}
