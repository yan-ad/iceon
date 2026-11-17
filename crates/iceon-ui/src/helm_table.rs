//! Cached, read-only Helm inventory. Rendering and local search never query Helm.
use crate::cluster_view::table_cell;
use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _, VirtualListScrollHandle,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::InputState,
    scroll::{ScrollableElement as _, ScrollbarAxis},
    tag::Tag,
    v_flex, v_virtual_list,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use iceon_core::connection::helm::{HelmItem, HelmPage, HelmView};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Column {
    Name,
    Namespace,
    Chart,
    Revision,
    Version,
    AppVersion,
    Status,
    Updated,
    Description,
    Actions,
}

#[cfg(test)]
#[path = "helm_table_tests.rs"]
mod tests;
impl Column {
    fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Namespace => "Namespace",
            Self::Chart => "Chart",
            Self::Revision => "Revision",
            Self::Version => "Version",
            Self::AppVersion => "App Version",
            Self::Status => "Status",
            Self::Updated => "Updated",
            Self::Description => "Description",
            Self::Actions => "Actions",
        }
    }
    fn width(self) -> f32 {
        match self {
            Self::Name => 210.,
            Self::Namespace => 150.,
            Self::Chart => 180.,
            Self::Revision => 90.,
            Self::Version | Self::AppVersion => 125.,
            Self::Status => 140.,
            Self::Updated => 110.,
            Self::Description => 360.,
            Self::Actions => 90.,
        }
    }
    fn flexible(self) -> bool {
        matches!(self, Self::Name | Self::Description)
    }
}
fn columns(view: HelmView) -> &'static [Column] {
    match view {
        HelmView::Charts => &[
            Column::Name,
            Column::Version,
            Column::AppVersion,
            Column::Description,
            Column::Actions,
        ],
        HelmView::Releases => &[
            Column::Name,
            Column::Namespace,
            Column::Chart,
            Column::Revision,
            Column::Version,
            Column::AppVersion,
            Column::Status,
            Column::Updated,
            Column::Actions,
        ],
    }
}
fn available(value: Option<&str>) -> &str {
    value.filter(|v| !v.trim().is_empty()).unwrap_or("—")
}

/// The release projection's `version` is Helm's chart label, not a standalone
/// version. Split only an unambiguous plain x.y.z suffix. Preserve unfamiliar
/// labels verbatim (including prereleases) rather than guessing a chart name.
fn chart_parts(label: &str) -> (&str, &str) {
    if let Some((name, version)) = label.rsplit_once('-') {
        let parts: Vec<_> = version.split('.').collect();
        if !name.is_empty()
            && parts.len() == 3
            && parts.iter().all(|p| {
                !p.is_empty()
                    && p.bytes().all(|b| b.is_ascii_digit())
                    && (p.len() == 1 || !p.starts_with('0'))
            })
        {
            return (name, version);
        }
    }
    (label, "—")
}
fn value(item: &HelmItem, view: HelmView, column: Column) -> String {
    match column {
        Column::Name => item.name.clone(),
        Column::Namespace => available(item.namespace.as_deref()).into(),
        Column::Chart => available(Some(chart_parts(&item.version).0)).into(),
        Column::Version => if view == HelmView::Charts {
            available(Some(&item.version))
        } else {
            chart_parts(&item.version).1
        }
        .into(),
        Column::AppVersion => available(Some(&item.app_version)).into(),
        Column::Revision => item.revision.map_or_else(|| "—".into(), |r| r.to_string()),
        Column::Status => available(item.status.as_deref()).into(),
        Column::Updated => available(item.updated.as_deref()).into(),
        Column::Description => available(item.description.as_deref()).into(),
        Column::Actions => String::new(),
    }
}
fn sorted_rows(
    items: &[HelmItem],
    view: HelmView,
    query: &str,
    sort: Option<(Column, bool)>,
) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    let mut rows: Vec<_> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            columns(view)
                .iter()
                .any(|col| value(item, view, *col).to_lowercase().contains(&query))
        })
        .map(|(index, _)| index)
        .collect();
    if let Some((column, descending)) = sort {
        rows.sort_by(|a, b| {
            let order = if column == Column::Revision {
                items[*a].revision.cmp(&items[*b].revision)
            } else {
                value(&items[*a], view, column)
                    .to_lowercase()
                    .cmp(&value(&items[*b], view, column).to_lowercase())
            };
            (if descending { order.reverse() } else { order }).then_with(|| a.cmp(b))
        });
    }
    rows
}

/// Only explicit buttons emit these events; rows have no selection mutation.
#[derive(Clone)]
pub(crate) enum HelmTableEvent {
    Inspect(HelmView, HelmItem),
    Install(HelmItem),
}
impl EventEmitter<HelmTableEvent> for HelmTable {}
/// Read-only diagnostics for the offline native fixture, never an engine API.
#[cfg(debug_assertions)]
pub(crate) struct ProbeSnapshot {
    pub view_renders: usize,
    pub row_renders: usize,
    pub rows: Vec<usize>,
    pub sizes: usize,
    pub page: Option<HelmPage>,
    pub revision_sort: Option<bool>,
    pub loading: bool,
    pub error: Option<String>,
}
pub(crate) struct HelmTable {
    search: Entity<InputState>,
    page: Option<HelmPage>,
    view: HelmView,
    loading: bool,
    error: Option<String>,
    timezone: String,
    rows: Rc<Vec<usize>>,
    sizes: Rc<Vec<Size<Pixels>>>,
    sort: Option<(Column, bool)>,
    scroll: VirtualListScrollHandle,
    horizontal: ScrollHandle,
    #[cfg(debug_assertions)]
    renders: std::cell::Cell<usize>,
    #[cfg(debug_assertions)]
    view_renders: std::cell::Cell<usize>,
}
impl HelmTable {
    #[cfg(debug_assertions)]
    pub(crate) fn probe_snapshot(&self) -> ProbeSnapshot {
        ProbeSnapshot {
            view_renders: self.view_renders.get(),
            row_renders: self.renders.get(),
            rows: self.rows.as_ref().clone(),
            sizes: self.sizes.len(),
            page: self.page.clone(),
            revision_sort: self
                .sort
                .and_then(|(key, descending)| (key == Column::Revision).then_some(descending)),
            loading: self.loading,
            error: self.error.clone(),
        }
    }
    #[cfg(debug_assertions)]
    pub(crate) fn probe_sort_revision(&mut self, cx: &mut Context<Self>) {
        self.sort_by(Column::Revision, cx);
    }
    #[cfg(debug_assertions)]
    pub(crate) fn probe_query(&self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.search_state().update(cx, |input, cx| {
            input.set_value(query.to_owned(), window, cx)
        });
    }
    pub(crate) fn search_state(&self) -> Entity<InputState> {
        self.search.clone()
    }

    pub(crate) fn inventory_count(&self) -> Option<crate::inventory_count::InventoryCount> {
        if self.loading && self.page.as_ref().is_none_or(|page| page.items.is_empty()) {
            return None;
        }
        self.page.as_ref().map(|page| {
            crate::inventory_count::InventoryCount::new(
                self.rows.len(),
                page.items.len(),
                page.truncated || self.loading,
            )
        })
    }

    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search Helm inventory…"));
        cx.observe(&search, |this, _, cx| this.rebuild(cx)).detach();
        Self {
            search,
            page: None,
            view: HelmView::Charts,
            loading: false,
            error: None,
            timezone: "system".into(),
            rows: Rc::default(),
            sizes: Rc::default(),
            sort: None,
            scroll: VirtualListScrollHandle::new(),
            horizontal: ScrollHandle::default(),
            #[cfg(debug_assertions)]
            renders: std::cell::Cell::new(0),
            #[cfg(debug_assertions)]
            view_renders: std::cell::Cell::new(0),
        }
    }
    pub(crate) fn sync(
        &mut self,
        page: Option<HelmPage>,
        view: HelmView,
        loading: bool,
        error: Option<String>,
        timezone: String,
        cx: &mut Context<Self>,
    ) {
        if self.page == page
            && self.view == view
            && self.loading == loading
            && self.error == error
            && self.timezone == timezone
        {
            return;
        }
        if self.view != view {
            self.sort = None;
        }
        self.page = page;
        self.view = view;
        self.loading = loading;
        self.error = error;
        self.timezone = timezone;
        self.rebuild(cx);
    }
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        self.rows = Rc::new(sorted_rows(
            self.page.as_ref().map_or(&[], |p| p.items.as_slice()),
            self.view,
            &self.search.read(cx).value(),
            self.sort,
        ));
        self.sizes = Rc::new(vec![size(px(0.), px(42.)); self.rows.len()]);
        cx.notify();
    }
    fn sort_by(&mut self, column: Column, cx: &mut Context<Self>) {
        self.sort = match self.sort {
            Some((key, false)) if key == column => Some((column, true)),
            Some((key, true)) if key == column => None,
            _ => Some((column, false)),
        };
        self.rebuild(cx);
    }
    fn header(&self, column: Column, cx: &mut Context<Self>) -> AnyElement {
        let cell = table_cell(column.width(), column.flexible(), true, cx.theme().border);
        if column == Column::Actions {
            return cell.text_xs().child(column.label()).into_any_element();
        }
        let marker = self
            .sort
            .filter(|(key, _)| *key == column)
            .map_or("", |(_, desc)| if desc { " ↓" } else { " ↑" });
        cell.child(
            Button::new(SharedString::from(format!("helm-sort-{}", column.label())))
                .ghost()
                .xsmall()
                .w_full()
                .min_w_0()
                .label(format!("{}{marker}", column.label()))
                .accessibility_label(format!("Sort by {}", column.label()))
                .tooltip("Sort ascending, descending, then restore source order")
                .on_click(cx.listener(move |this, _, _, cx| this.sort_by(column, cx))),
        )
        .into_any_element()
    }
    fn row(&self, item: &HelmItem, cx: &mut Context<Self>) -> AnyElement {
        #[cfg(debug_assertions)]
        self.renders.set(self.renders.get() + 1);
        h_flex()
            .id(SharedString::from(format!("helm-row-{}", item.id)))
            .w_full()
            .h(px(42.))
            .items_center()
            .border_b_1()
            .border_color(cx.theme().border)
            .hover(|el| el.bg(cx.theme().muted))
            .children(columns(self.view).iter().map(|column| {
                let cell = table_cell(column.width(), column.flexible(), true, cx.theme().border)
                    .text_xs();
                match column {
                    Column::Actions => {
                        let inspect = item.clone();
                        let install = item.clone();
                        let view = self.view;
                        cell.child(
                            h_flex()
                                .gap_1()
                                .when(view == HelmView::Charts, |el| {
                                    el.child(
                                        Button::new(SharedString::from(format!(
                                            "helm-install-{}",
                                            item.id
                                        )))
                                        .ghost()
                                        .xsmall()
                                        .icon(gpui_kit::component::IconName::Plus)
                                        .tooltip("Install Helm chart")
                                        .accessibility_label("Install Helm chart")
                                        .on_click(
                                            cx.listener(move |_, _, _, cx| {
                                                cx.emit(HelmTableEvent::Install(install.clone()))
                                            }),
                                        ),
                                    )
                                })
                                .child(
                                    Button::new(SharedString::from(format!(
                                        "helm-inspect-{}",
                                        item.id
                                    )))
                                    .ghost()
                                    .xsmall()
                                    .icon(gpui_kit::component::IconName::Eye)
                                    .tooltip("Inspect Helm details")
                                    .accessibility_label("Inspect Helm details")
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.emit(HelmTableEvent::Inspect(view, inspect.clone()))
                                    })),
                                ),
                        )
                        .into_any_element()
                    }
                    Column::Status => {
                        let status = available(item.status.as_deref());
                        let badge = match status.to_ascii_lowercase().as_str() {
                            "deployed" => Tag::success(),
                            "failed" => Tag::danger(),
                            s if s.starts_with("pending") || s == "uninstalling" => Tag::warning(),
                            _ => Tag::secondary(),
                        };
                        cell.child(badge.small().child(status.to_owned()))
                            .into_any_element()
                    }
                    Column::Updated => {
                        let timestamp = normalized_updated(item.updated.as_deref());
                        cell.child(crate::age_label::age_label(
                            SharedString::from(format!("helm-updated-{}", item.id)),
                            timestamp.as_deref(),
                            &self.timezone,
                        ))
                        .into_any_element()
                    }
                    _ => cell
                        .truncate()
                        .child(value(item, self.view, *column))
                        .into_any_element(),
                }
            }))
            .into_any_element()
    }
}

/// Helm uses Go's time.String representation, while shared age labels consume RFC3339.
fn normalized_updated(value: Option<&str>) -> Option<String> {
    let value = value.filter(|v| !v.trim().is_empty())?;
    if let Ok(time) = value.parse::<jiff::Timestamp>() {
        return Some(time.to_string());
    }
    // Ignore only Go's trailing zone abbreviation, retaining its numeric offset.
    let fields: Vec<_> = value.split_whitespace().collect();
    if fields.len() == 4 {
        let input = format!("{} {} {}", fields[0], fields[1], fields[2]);
        if let Ok(time) = jiff::Timestamp::strptime("%Y-%m-%d %H:%M:%S%.f %z", input) {
            return Some(time.to_string());
        }
    }
    Some(value.to_owned())
}
impl Render for HelmTable {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(debug_assertions)]
        self.view_renders.set(self.view_renders.get() + 1);
        let items = self
            .page
            .as_ref()
            .map(|p| p.items.clone())
            .unwrap_or_default();
        let rows = self.rows.clone();
        let scroll = self.scroll.clone();

        if let Some(error) = &self.error {
            return v_flex()
                .size_full()
                .min_h_0()
                .min_w_0()
                .items_center()
                .justify_center()
                .child(crate::empty_state::error_state(error.clone()))
                .into_any_element();
        }

        if self.page.is_some() && rows.is_empty() && !self.loading {
            let search_val = self.search.read(cx).value().trim().to_string();
            let state = if !search_val.is_empty() {
                crate::empty_state::empty_state(
                    IconName::Search,
                    "No Helm items match the search",
                    "Try a different search query or select another namespace.",
                )
            } else {
                crate::empty_state::empty_state(
                    IconName::Inbox,
                    "No Helm items found",
                    "No Helm releases or charts found in this scope.",
                )
            };
            return v_flex()
                .size_full()
                .min_h_0()
                .min_w_0()
                .items_center()
                .justify_center()
                .child(state)
                .into_any_element();
        }

        let status: Option<String> = if self.loading {
            Some("Loading Helm inventory…".into())
        } else {
            None
        };
        v_flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .size_full()
            .gap_2()
            .when_some(status, |el, status| {
                el.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(status),
                )
            })
            .when(self.page.as_ref().is_some_and(|p| p.truncated), |el| {
                el.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().warning)
                        .child("Showing a partial Helm inventory; results may be truncated."),
                )
            })
            .child(
                v_flex()
                    .id("helm-horizontal")
                    .overflow_x_scroll()
                    .track_scroll(&self.horizontal)
                    .scrollbar(&self.horizontal, ScrollbarAxis::Horizontal)
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .w_full()
                    .child(
                        v_flex()
                            .w_full()
                            .min_w(px(columns(self.view).iter().map(|c| c.width()).sum()))
                            .flex_1()
                            .min_h_0()
                            .child(
                                h_flex()
                                    .w_full()
                                    .h(px(40.))
                                    .flex_shrink_0()
                                    .items_center()
                                    .border_b_1()
                                    .border_color(cx.theme().border)
                                    .children(
                                        columns(self.view)
                                            .iter()
                                            .map(|column| self.header(*column, cx)),
                                    ),
                            )
                            .child(
                                v_virtual_list(
                                    cx.entity(),
                                    "helm-inventory-table",
                                    self.sizes.clone(),
                                    move |this, range, _, cx| {
                                        range
                                            .filter_map(|index| items.get(*rows.get(index)?))
                                            .map(|item| this.row(item, cx))
                                            .collect::<Vec<_>>()
                                    },
                                )
                                .track_scroll(&scroll)
                                .flex_1()
                                .min_h_0()
                                .w_full(),
                            ),
                    ),
            )
            .into_any_element()
    }
}
