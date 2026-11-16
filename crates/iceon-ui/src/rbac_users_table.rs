//! Read-only, cached RBAC subject table.
//!
//! The table deliberately owns only a cloned response.  It never starts work or
//! calls the connection engine while rendering; the shell supplies snapshots via
//! [`RbacUsersTable::sync`].

use crate::cluster_view::table_cell;
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, VirtualListScrollHandle,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::InputState,
    scroll::{ScrollableElement as _, ScrollbarAxis},
    v_flex, v_virtual_list,
};
use gpui_kit::prelude::*;
#[cfg(debug_assertions)]
use gpui_kit::private::anyhow;
use gpui_kit::*;
use iceon_core::connection::rbac::{RbacUsersResponse, SubjectBinding, SubjectRecord};
use std::cmp::Ordering;
use std::rc::Rc;

const ROW_HEIGHT: f32 = 42.0;
const HEADER_HEIGHT: f32 = 40.0;
const NAME_WIDTH: f32 = 260.0;
const TYPE_WIDTH: f32 = 112.0;
const NAMESPACE_WIDTH: f32 = 168.0;
const BINDINGS_WIDTH: f32 = 104.0;
const ROLES_WIDTH: f32 = 300.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UsersSortKey {
    Name,
    Type,
    Namespace,
    Bindings,
    Roles,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct UsersSort {
    key: UsersSortKey,
    descending: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UsersRow {
    pub source_index: usize,
}

fn lower(value: &str) -> String {
    value.to_ascii_lowercase()
}

/// Search all user-visible subject and binding fields.  Binding names include
/// both the binding namespace and the referenced role name.
pub(crate) fn user_matches(subject: &SubjectRecord, query: &str) -> bool {
    let query = query.trim().to_ascii_lowercase();
    query.is_empty()
        || [
            subject.kind,
            subject.name.as_str(),
            subject.namespace.as_deref().unwrap_or_default(),
        ]
        .into_iter()
        .any(|value| lower(value).contains(&query))
        || subject.bindings.iter().any(|binding| {
            [
                binding.kind,
                binding.name.as_str(),
                binding.namespace.as_deref().unwrap_or_default(),
                binding.role_ref.kind,
                binding.role_ref.name.as_str(),
            ]
            .into_iter()
            .any(|value| lower(value).contains(&query))
        })
}

fn role_summary(binding: &SubjectBinding) -> String {
    let binding_name = match binding.namespace.as_deref() {
        Some(namespace) if !namespace.is_empty() => format!("{}/{}", namespace, binding.name),
        _ => binding.name.clone(),
    };
    format!(
        "{} {} via {} {}",
        binding.role_ref.kind, binding.role_ref.name, binding.kind, binding_name
    )
}

pub(crate) fn role_summaries(subject: &SubjectRecord) -> Vec<String> {
    subject.bindings.iter().map(role_summary).collect()
}

fn cmp_text(a: &str, b: &str) -> Ordering {
    lower(a).cmp(&lower(b)).then_with(|| a.cmp(b))
}

fn sort_value(subject: &SubjectRecord, key: UsersSortKey) -> String {
    match key {
        UsersSortKey::Name => subject.name.clone(),
        UsersSortKey::Type => subject.kind.to_owned(),
        UsersSortKey::Namespace => subject.namespace.clone().unwrap_or_default(),
        UsersSortKey::Bindings => subject.bindings.len().to_string(),
        UsersSortKey::Roles => role_summaries(subject).join("\u{1f}"),
    }
}

pub(crate) fn sorted_rows(
    subjects: &[SubjectRecord],
    query: &str,
    sort: Option<(UsersSortKey, bool)>,
) -> Vec<UsersRow> {
    let mut rows: Vec<UsersRow> = subjects
        .iter()
        .enumerate()
        .filter(|(_, subject)| user_matches(subject, query))
        .map(|(source_index, _)| UsersRow { source_index })
        .collect();
    if let Some((key, descending)) = sort {
        rows.sort_by(|a, b| {
            let left = &subjects[a.source_index];
            let right = &subjects[b.source_index];
            let order = if key == UsersSortKey::Bindings {
                left.bindings
                    .len()
                    .cmp(&right.bindings.len())
                    .then_with(|| cmp_text(&sort_value(left, key), &sort_value(right, key)))
            } else {
                cmp_text(&sort_value(left, key), &sort_value(right, key))
            };
            let order = if descending { order.reverse() } else { order };
            order.then_with(|| a.source_index.cmp(&b.source_index))
        });
    }
    rows
}

pub(crate) fn response_notices(response: Option<&RbacUsersResponse>) -> Vec<String> {
    let Some(response) = response else {
        return Vec::new();
    };
    let mut notices = Vec::new();
    if response.truncated {
        notices.push("Subject results truncated; this is not a complete user inventory.".into());
    }
    if response.items.iter().any(|item| item.bindings_truncated) {
        notices.push(
            "Some subjects have truncated bindings; counts marked + are lower bounds.".into(),
        );
    }
    for coverage in &response.coverage {
        if coverage.state != "complete" {
            notices.push(format!(
                "{} coverage: {}",
                coverage.resource, coverage.state
            ));
        }
    }
    notices
}

pub(crate) struct RbacUsersTable {
    search: Entity<InputState>,
    response: Option<RbacUsersResponse>,
    loading: bool,
    partial: bool,
    error: Option<String>,
    rows: Rc<Vec<UsersRow>>,
    row_sizes: Rc<Vec<Size<Pixels>>>,
    sort: Option<UsersSort>,
    scroll: VirtualListScrollHandle,
    horizontal_scroll: ScrollHandle,
    #[cfg(debug_assertions)]
    probe_renders: std::cell::Cell<(usize, usize)>,
}

impl RbacUsersTable {
    pub(crate) fn search_state(&self) -> Entity<InputState> {
        self.search.clone()
    }

    pub(crate) fn inventory_count(&self) -> Option<crate::inventory_count::InventoryCount> {
        if self.loading
            && self
                .response
                .as_ref()
                .is_none_or(|response| response.items.is_empty())
        {
            return None;
        }
        self.response.as_ref().map(|response| {
            crate::inventory_count::InventoryCount::new(
                self.rows.len(),
                response.items.len(),
                self.loading || self.partial,
            )
        })
    }

    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx
            .new(|cx| InputState::new(window, cx).placeholder("Search users, bindings, or roles…"));
        cx.observe(&search, |this, _, cx| this.rebuild_rows(cx))
            .detach();
        Self {
            search,
            response: None,
            loading: false,
            partial: false,
            error: None,
            rows: Rc::new(Vec::new()),
            row_sizes: Rc::new(Vec::new()),
            sort: None,
            scroll: VirtualListScrollHandle::new(),
            horizontal_scroll: ScrollHandle::default(),
            #[cfg(debug_assertions)]
            probe_renders: std::cell::Cell::new((0, 0)),
        }
    }

    /// Replace the local snapshot. `None` clears rows while retaining controls.
    pub(crate) fn sync(
        &mut self,
        response: Option<RbacUsersResponse>,
        loading: bool,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.response == response && self.loading == loading && self.error == error {
            return;
        }
        self.partial = response.as_ref().is_some_and(|response| {
            response.truncated
                || response
                    .coverage
                    .iter()
                    .any(|coverage| coverage.state != "complete")
                || response.items.iter().any(|item| item.bindings_truncated)
        });
        self.response = response;
        self.loading = loading;
        self.error = error;
        self.rebuild_rows(cx);
    }

    fn rebuild_rows(&mut self, cx: &mut Context<Self>) {
        let query = self.search.read(cx).value();
        let subjects = self
            .response
            .as_ref()
            .map_or(&[][..], |response| &response.items);
        self.rows = Rc::new(sorted_rows(
            subjects,
            &query,
            self.sort.map(|sort| (sort.key, sort.descending)),
        ));
        self.row_sizes = Rc::new(vec![
            Size {
                width: px(0.),
                height: px(ROW_HEIGHT)
            };
            self.rows.len()
        ]);
        cx.notify();
    }

    fn sort_by(&mut self, key: UsersSortKey, cx: &mut Context<Self>) {
        self.sort = match self.sort {
            Some(sort) if sort.key == key && !sort.descending => Some(UsersSort {
                key,
                descending: true,
            }),
            Some(sort) if sort.key == key => None,
            _ => Some(UsersSort {
                key,
                descending: false,
            }),
        };
        self.rebuild_rows(cx);
    }

    fn header(
        &self,
        key: UsersSortKey,
        label: &'static str,
        width: f32,
        flexible: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let marker = self
            .sort
            .filter(|sort| sort.key == key)
            .map_or("", |sort| if sort.descending { " ↓" } else { " ↑" });
        table_cell(width, flexible, true, cx.theme().border)
            .child(
                Button::new(SharedString::from(format!("rbac-users-sort-{label}")))
                    .ghost()
                    .xsmall()
                    .w_full()
                    .min_w_0()
                    .child(div().truncate().child(format!("{label}{marker}")))
                    .tooltip(format!(
                        "Sort by {label}; click again to reverse, third click restores source order"
                    ))
                    .accessibility_label(format!("Sort by {label}"))
                    .on_click(cx.listener(move |this, _, _, cx| this.sort_by(key, cx))),
            )
            .into_any_element()
    }

    fn cell_text(
        text: impl Into<SharedString>,
        width: f32,
        flexible: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        table_cell(width, flexible, true, cx.theme().border)
            .text_xs()
            .truncate()
            .child(text.into())
    }

    fn row(&self, subject: &SubjectRecord, cx: &mut Context<Self>) -> AnyElement {
        #[cfg(debug_assertions)]
        self.probe_renders
            .set((self.probe_renders.get().0, self.probe_renders.get().1 + 1));
        let roles = role_summaries(subject);
        let details = if roles.is_empty() {
            "No bindings reported".to_owned()
        } else {
            roles.join("\n")
        };
        let namespace = subject.namespace.clone().unwrap_or_else(|| "—".into());
        let bindings = format!(
            "{}{}",
            subject.bindings.len(),
            if subject.bindings_truncated { "+" } else { "" }
        );
        let roles_inline = if roles.is_empty() {
            "—".into()
        } else {
            roles.join(", ")
        };
        let accessible = format!(
            "{}, {}, namespace {}, {} bindings. {}",
            subject.name, subject.kind, namespace, bindings, details
        );
        h_flex()
            .id(SharedString::from(format!("rbac-user-{}", subject.id)))
            .w_full()
            .h(px(ROW_HEIGHT))
            .items_center()
            .border_b_1()
            .border_color(cx.theme().border)
            .hover(|row| row.bg(cx.theme().muted))
            .child(
                Self::cell_text(subject.name.clone(), NAME_WIDTH, true, cx)
                    .text_color(cx.theme().sidebar_accent_foreground),
            )
            .child(Self::cell_text(subject.kind, TYPE_WIDTH, false, cx))
            .child(Self::cell_text(namespace, NAMESPACE_WIDTH, false, cx))
            .child(Self::cell_text(bindings, BINDINGS_WIDTH, false, cx))
            .child(
                table_cell(ROLES_WIDTH, false, false, cx.theme().border).child(
                    Button::new(SharedString::from(format!(
                        "rbac-user-roles-{}",
                        subject.id
                    )))
                    .ghost()
                    .xsmall()
                    .w_full()
                    .min_w_0()
                    .child(div().truncate().child(roles_inline))
                    .tooltip(details)
                    .accessibility_label(accessible),
                ),
            )
            .into_any_element()
    }
}

impl Render for RbacUsersTable {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(debug_assertions)]
        self.probe_renders
            .set((self.probe_renders.get().0 + 1, self.probe_renders.get().1));
        let subjects = self
            .response
            .as_ref()
            .map(|response| response.items.clone())
            .unwrap_or_default();
        let rows = self.rows.clone();
        let sizes = self.row_sizes.clone();
        let scroll = self.scroll.clone();
        let entity = cx.entity();
        let query = self.search.read(cx).value();
        let status = if let Some(error) = self.error.clone() {
            Some(format!("Error: {error}"))
        } else if self.loading {
            Some("Loading users…".into())
        } else if self.response.is_some() && rows.is_empty() {
            Some(if query.trim().is_empty() {
                "No users reported".into()
            } else {
                "No users match the search".into()
            })
        } else {
            None
        };
        v_flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .w_full()
            .h_full()
            .gap_2()
            .when_some(status, |el, status| {
                el.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(status),
                )
            })
            .children(
                response_notices(self.response.as_ref())
                    .into_iter()
                    .map(|notice| div().text_xs().text_color(cx.theme().warning).child(notice)),
            )
            .child(
                v_flex()
                    .id("rbac-users-horizontal")
                    .overflow_x_scroll()
                    .track_scroll(&self.horizontal_scroll)
                    .scrollbar(&self.horizontal_scroll, ScrollbarAxis::Horizontal)
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .w_full()
                    .child(
                        v_flex()
                            .w_full()
                            .min_w(px(NAME_WIDTH
                                + TYPE_WIDTH
                                + NAMESPACE_WIDTH
                                + BINDINGS_WIDTH
                                + ROLES_WIDTH))
                            .flex_1()
                            .min_h_0()
                            .child(
                                h_flex()
                                    .w_full()
                                    .h(px(HEADER_HEIGHT))
                                    .flex_shrink_0()
                                    .items_center()
                                    .border_b_1()
                                    .border_color(cx.theme().border)
                                    .child(self.header(
                                        UsersSortKey::Name,
                                        "Name",
                                        NAME_WIDTH,
                                        true,
                                        cx,
                                    ))
                                    .child(self.header(
                                        UsersSortKey::Type,
                                        "Type",
                                        TYPE_WIDTH,
                                        false,
                                        cx,
                                    ))
                                    .child(self.header(
                                        UsersSortKey::Namespace,
                                        "Namespace",
                                        NAMESPACE_WIDTH,
                                        false,
                                        cx,
                                    ))
                                    .child(self.header(
                                        UsersSortKey::Bindings,
                                        "Bindings",
                                        BINDINGS_WIDTH,
                                        false,
                                        cx,
                                    ))
                                    .child(self.header(
                                        UsersSortKey::Roles,
                                        "Roles",
                                        ROLES_WIDTH,
                                        false,
                                        cx,
                                    )),
                            )
                            .child(
                                v_virtual_list(
                                    entity,
                                    SharedString::from("rbac-users-table"),
                                    sizes,
                                    move |this, range, _, cx| {
                                        range
                                            .filter_map(|index| {
                                                let row = rows.get(index)?;
                                                let subject = subjects.get(row.source_index)?;
                                                Some(this.row(subject, cx))
                                            })
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
    }
}

/// Offline native audit of the production Users view. No catalog, kubeconfig,
/// connection engine or host shell is constructed.
#[cfg(debug_assertions)]
pub fn run_probe() -> anyhow::Result<()> {
    use gpui_kit::component::ThemeMode;
    use std::{cell::RefCell, time::Duration};
    type Outcome = Rc<RefCell<Option<Result<(), String>>>>;
    let outcome: Outcome = Rc::default();
    let result = outcome.clone();
    gpui_kit::application()
        .with_assets(crate::app_assets::AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            crate::console_font::init(cx);
            crate::shell_theme::apply(ThemeMode::Dark, None, cx);
            let opened = crate::shell_theme::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None, size(px(1120.), px(640.)), cx,
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    window.set_window_title("Iceon Users — offline native probe");
                    cx.new(|cx| {
                        let mut table = RbacUsersTable::new(window, cx);
                        table.sync(Some(probe_fixture()), false, None, cx);
                        table
                    })
                },
            );
            let (handle, table) = match opened {
                Ok(opened) => opened,
                Err(error) => {
                    *result.borrow_mut() = Some(Err(error.to_string()));
                    cx.quit();
                    return;
                }
            };
            let watchdog = result.clone();
            cx.spawn(async move |cx| {
                cx.background_executor().timer(Duration::from_secs(15)).await;
                cx.update(|cx| {
                    if watchdog.borrow().is_none() {
                        *watchdog.borrow_mut() = Some(Err("native watchdog expired".into()));
                    }
                    cx.quit();
                });
            }).detach();
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() { cx.quit(); }
            }).detach();
            cx.spawn(async move |cx| {
                // Each mutation is followed by a later native draw: search is
                // driven through InputState so its real observer must fire.
                let mut previous_renders = 0;
                let mut previous_rows = 0;
                for stage in 0..10 {
                    cx.background_executor().timer(Duration::from_millis(220)).await;
                    let checked = handle.update(cx, |_, window, cx| -> anyhow::Result<()> {
                        window.draw(cx).clear(cx);
                        table.update(cx, |this, cx| -> anyhow::Result<()> {
                            let (renders, row_renders) = this.probe_renders.get();
                            let count = this.inventory_count().map(|count| count.label());
                            anyhow::ensure!(count.is_some() == this.response.is_some(), "missing snapshots must not claim zero items");
                            if let Some(count) = count {
                                anyhow::ensure!(count.starts_with(&this.rows.len().to_string()), "header count did not react to cached search");
                            }
                            anyhow::ensure!(renders > previous_renders, "stage {stage}: view did not render");
                            previous_renders = renders;
                            let expected = match stage { 4 => 24, 5 | 8 | 9 => 0, _ => 72 };
                            anyhow::ensure!(this.rows.len() == expected, "stage {stage}: expected {expected} rows, got {}", this.rows.len());
                            anyhow::ensure!(this.row_sizes.len() == expected, "row sizes out of sync");
                            if expected > 0 {
                                anyhow::ensure!(row_renders > previous_rows, "stage {stage}: virtual list callback rendered no rows");
                            }
                            previous_rows = row_renders;
                            if stage < 8 {
                                anyhow::ensure!(response_notices(this.response.as_ref()).len() == 3, "incomplete coverage notices lost");
                            }
                            match stage {
                                0 => this.sort_by(UsersSortKey::Name, cx),
                                1 => {
                                    anyhow::ensure!(this.rows[0].source_index == 71, "ascending sort failed");
                                    this.sort_by(UsersSortKey::Name, cx);
                                }
                                2 => {
                                    anyhow::ensure!(this.rows[0].source_index == 0, "descending sort failed");
                                    this.sort_by(UsersSortKey::Name, cx);
                                }
                                3 => {
                                    anyhow::ensure!(this.sort.is_none(), "third sort did not reset");
                                    this.search_state().update(cx, |input, cx| input.set_value("ServiceAccount", window, cx));
                                }
                                4 => {
                                    anyhow::ensure!(this.rows.iter().all(|r| this.response.as_ref().unwrap().items[r.source_index].kind == "ServiceAccount"), "type filter failed");
                                    this.search_state().update(cx, |input, cx| input.set_value("no-such-offline-subject", window, cx));
                                }
                                5 => { this.search_state().update(cx, |input, cx| input.set_value("team-fixture", window, cx)); }
                                6 => { this.search_state().update(cx, |input, cx| input.set_value("", window, cx)); }
                                7 => this.sync(None, true, None, cx),
                                8 => {
                                    anyhow::ensure!(this.loading, "loading state missing");
                                    this.sync(None, false, Some("Offline fixture error".into()), cx);
                                }
                                9 => {
                                    anyhow::ensure!(!this.loading && this.error.is_some(), "error state missing");
                                }
                                _ => unreachable!(),
                            }
                            Ok(())
                        })
                    });
                    let error = match checked {
                        Ok(Ok(())) => None,
                        Ok(Err(error)) => Some(error.to_string()),
                        Err(error) => Some(error.to_string()),
                    };
                    if let Some(error) = error {
                        cx.update(|cx| {
                            *result.borrow_mut() = Some(Err(format!("stage {stage}: {error}")));
                            cx.quit();
                        });
                        return;
                    }
                }
                cx.update(|cx| {
                    println!("RBAC_USERS_PROBE_OK: actual native Users view and virtual rows rendered; 72 synthetic subjects; sort cycle, InputState search, namespaced bindings, truncation, loading and error verified; no connections or credentials");
                    *result.borrow_mut() = Some(Ok(()));
                    cx.quit();
                });
            }).detach();
            cx.activate(true);
        });
    let result = outcome.borrow_mut().take();
    match result {
        Some(Ok(())) => Ok(()),
        Some(Err(error)) => anyhow::bail!("RBAC Users probe: {error}"),
        None => anyhow::bail!("RBAC Users probe closed before verification"),
    }
}

#[cfg(debug_assertions)]
fn probe_fixture() -> RbacUsersResponse {
    use iceon_core::connection::rbac::{Coverage, SubjectRoleRef};
    RbacUsersResponse {
        cluster_id: "offline-fixture".into(),
        session_id: "offline-session".into(),
        read_id: 1,
        namespace: None,
        fetched_at: 0,
        truncated: true,
        coverage: vec![
            Coverage {
                resource: "RoleBindings",
                state: "complete",
            },
            Coverage {
                resource: "ClusterRoleBindings",
                state: "forbidden",
            },
        ],
        items: (0..72)
            .map(|index| SubjectRecord {
                id: format!("fixture-{index}"),
                kind: ["User", "Group", "ServiceAccount"][index % 3],
                name: format!("subject-{:03}", 71 - index),
                namespace: (index % 3 == 2).then(|| "workloads-fixture".into()),
                bindings_truncated: index == 0,
                bindings: vec![SubjectBinding {
                    kind: "RoleBinding",
                    name: format!("grant-{index:03}"),
                    namespace: Some("team-fixture".into()),
                    uid: format!("fixture-binding-{index}"),
                    resource_version: "1".into(),
                    role_ref: SubjectRoleRef {
                        kind: "Role",
                        name: "fixture-reader".into(),
                    },
                }],
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iceon_core::connection::rbac::SubjectRoleRef;

    #[cfg(debug_assertions)]
    #[::core::prelude::v1::test]
    fn native_probe_fixture_covers_types_namespaces_and_incompleteness() {
        let response = probe_fixture();
        assert_eq!(response.items.len(), 72);
        for kind in ["User", "Group", "ServiceAccount"] {
            assert_eq!(
                response
                    .items
                    .iter()
                    .filter(|item| item.kind == kind)
                    .count(),
                24
            );
        }
        assert!(
            response
                .items
                .iter()
                .all(|item| item.bindings[0].namespace.as_deref() == Some("team-fixture"))
        );
        assert_eq!(response_notices(Some(&response)).len(), 3);
        assert_eq!(sorted_rows(&response.items, "team-fixture", None).len(), 72);
    }

    fn subject(
        kind: &'static str,
        name: &str,
        namespace: Option<&str>,
        bindings: Vec<SubjectBinding>,
    ) -> SubjectRecord {
        SubjectRecord {
            id: name.into(),
            kind,
            name: name.into(),
            namespace: namespace.map(str::to_owned),
            bindings,
            bindings_truncated: false,
        }
    }
    fn binding(name: &str, namespace: Option<&str>, role: &str) -> SubjectBinding {
        SubjectBinding {
            kind: "RoleBinding",
            name: name.into(),
            namespace: namespace.map(str::to_owned),
            uid: "u".into(),
            resource_version: "1".into(),
            role_ref: SubjectRoleRef {
                kind: "Role",
                name: role.into(),
            },
        }
    }
    #[::core::prelude::v1::test]
    fn search_matches_subject_and_all_binding_fields() {
        let item = subject(
            "ServiceAccount",
            "robot",
            Some("payments"),
            vec![binding("grant", Some("ops"), "read-secrets")],
        );
        for query in [
            "serviceaccount",
            "robot",
            "payments",
            "grant",
            "ops",
            "read-secrets",
            "rolebinding",
        ] {
            assert!(user_matches(&item, query), "{query}");
        }
        assert!(!user_matches(&item, "unrelated"));
    }
    #[::core::prelude::v1::test]
    fn sorting_is_stable_and_numeric_for_binding_counts() {
        let items = vec![
            subject("User", "z", None, vec![]),
            subject(
                "User",
                "a",
                None,
                vec![binding("b", None, "r"), binding("c", None, "r")],
            ),
            subject("User", "b", None, vec![binding("a", None, "r")]),
            subject("User", "b", None, vec![]),
        ];
        let rows = sorted_rows(&items, "", Some((UsersSortKey::Bindings, false)));
        assert_eq!(
            rows.iter()
                .map(|row| items[row.source_index].bindings.len())
                .collect::<Vec<_>>(),
            vec![0, 0, 1, 2]
        );
        assert_eq!(rows[0].source_index, 0);
        assert_eq!(rows[1].source_index, 3);
        let desc = sorted_rows(&items, "", Some((UsersSortKey::Bindings, true)));
        assert_eq!(
            desc.iter()
                .map(|row| items[row.source_index].bindings.len())
                .collect::<Vec<_>>(),
            vec![2, 1, 0, 0]
        );
    }
    #[::core::prelude::v1::test]
    fn role_summaries_include_every_binding_and_namespace() {
        let item = subject(
            "User",
            "a",
            None,
            vec![
                binding("one", Some("ns"), "reader"),
                binding("two", None, "admin"),
            ],
        );
        assert_eq!(
            role_summaries(&item),
            vec![
                "Role reader via RoleBinding ns/one",
                "Role admin via RoleBinding two"
            ]
        );
    }
    #[::core::prelude::v1::test]
    fn truncation_and_partial_coverage_remain_visible_without_matching_rows() {
        use iceon_core::connection::rbac::Coverage;
        let mut item = subject("User", "a", None, vec![]);
        item.bindings_truncated = true;
        let response = RbacUsersResponse {
            cluster_id: "fixture".into(),
            session_id: "session".into(),
            read_id: 1,
            namespace: None,
            items: vec![item],
            coverage: vec![
                Coverage {
                    resource: "RoleBindings",
                    state: "forbidden",
                },
                Coverage {
                    resource: "ClusterRoleBindings",
                    state: "complete",
                },
            ],
            truncated: true,
            fetched_at: 0,
        };
        let notices = response_notices(Some(&response));
        assert_eq!(notices.len(), 3);
        assert!(notices[0].contains("Subject results truncated"));
        assert!(notices[1].contains("lower bounds"));
        assert!(notices[2].contains("RoleBindings coverage: forbidden"));
        assert!(sorted_rows(&response.items, "missing", None).is_empty());
        assert_eq!(response_notices(Some(&response)), notices);
        assert!(response_notices(None).is_empty());
    }

    #[::core::prelude::v1::test]
    fn more_than_eight_bindings_are_searchable_and_summarized() {
        let bindings = (0..12)
            .map(|n| binding(&format!("binding-{n}"), Some("ns"), &format!("role-{n}")))
            .collect();
        let item = subject("User", "a", None, bindings);
        assert_eq!(role_summaries(&item).len(), 12);
        assert!(user_matches(&item, "role-11"));
        assert!(user_matches(&item, "binding-11"));
        assert!(user_matches(&item, "   "));
    }

    #[::core::prelude::v1::test]
    fn source_order_is_restored_and_descending_ties_do_not_reverse() {
        let items = vec![
            subject("User", "same", None, vec![]),
            subject("Group", "same", None, vec![]),
            subject("User", "first", None, vec![]),
        ];
        let indexes = |rows: Vec<UsersRow>| {
            rows.into_iter()
                .map(|row| row.source_index)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            indexes(sorted_rows(&items, "", Some((UsersSortKey::Name, true)))),
            vec![0, 1, 2]
        );
        assert_eq!(indexes(sorted_rows(&items, "", None)), vec![0, 1, 2]);
        assert_eq!(
            indexes(sorted_rows(&items, "", Some((UsersSortKey::Name, false)))),
            vec![2, 0, 1]
        );
    }
}
