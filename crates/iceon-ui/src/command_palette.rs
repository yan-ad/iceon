//! Global navigation through GPUI Kit's keyboard-accessible Command component.
use crate::{IceonApp, navigation::Route};
use gpui_kit::{
    component::{
        ActiveTheme as _, Icon,
        command::{Command, CommandGroup, CommandItem},
        h_flex,
    },
    prelude::*,
    *,
};

#[derive(Clone, Debug, PartialEq, Eq)]
enum JumpAction {
    CreateCluster,
    Settings,
    Navigate(Route),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JumpIcon {
    Asset(gpui_kit_assets::IconName),
    Brand(&'static str),
}

impl JumpIcon {
    fn icon(self) -> Icon {
        match self {
            Self::Asset(name) => Icon::new(name),
            Self::Brand(path) => Icon::default().path(path),
        }
    }
}

struct JumpEntry {
    title: String,
    keywords: Vec<String>,
    icon: JumpIcon,
    action: JumpAction,
    // Dynamic destinations must not outlive their discovery lease.
    lease: Option<String>,
}

struct JumpGroup {
    title: String,
    entries: Vec<JumpEntry>,
}

fn entry_at(groups: &[JumpGroup], section: usize, row: usize) -> Option<&JumpEntry> {
    groups.get(section)?.entries.get(row)
}

fn destinations(
    clusters: &[iceon_core::catalog::ClusterSummary],
    lease: Option<(&str, &str)>,
    custom_page: Option<&iceon_core::connection::ResourcePage>,
    allowed: impl Fn(&str, &str) -> bool,
) -> Vec<JumpGroup> {
    use gpui_kit_assets::IconName;
    let mut groups = vec![JumpGroup {
        title: "Actions".into(),
        entries: [
            (
                "Create Cluster",
                "add import kubeconfig connect",
                IconName::Plus,
                JumpAction::CreateCluster,
            ),
            (
                "Iceon Settings",
                "preferences appearance theme",
                IconName::Settings,
                JumpAction::Settings,
            ),
            (
                "Secrets",
                "vault",
                IconName::KeyRound,
                JumpAction::Navigate(Route::Secrets),
            ),
        ]
        .into_iter()
        .map(|(title, keywords, icon, action)| JumpEntry {
            title: title.into(),
            keywords: vec![keywords.into()],
            icon: JumpIcon::Asset(icon),
            action,
            lease: None,
        })
        .collect(),
    }];
    for cluster in clusters {
        let duplicate_name = clusters
            .iter()
            .filter(|other| other.name == cluster.name)
            .count()
            > 1;
        let title = if duplicate_name {
            format!("{} · {}", cluster.name, cluster.context_name)
        } else {
            cluster.name.clone()
        };
        let mut entries = Vec::new();
        let mut push =
            |section: &str, title: String, lease: Option<String>, api_group: Option<&str>| {
                if !allowed(&cluster.id, section) {
                    return;
                }
                let mut keywords = vec![
                    section.to_owned(),
                    cluster.name.clone(),
                    cluster.context_name.clone(),
                ];
                // Command's filter is a substring search within each individual keyword.
                // Keep combined terms so e.g. "prod pods" can cross the cluster/title boundary.
                let leaf = title.rsplit(" · ").next().unwrap_or(&title);
                for prefix in [&cluster.name, &cluster.context_name] {
                    keywords.push(format!("{prefix} {leaf}"));
                    keywords.push(format!("{prefix} {title}"));
                    keywords.push(format!("{prefix} {section}"));
                }
                entries.push(JumpEntry {
                    title,
                    keywords,
                    icon: api_group.map_or_else(
                        || JumpIcon::Asset(crate::sidebar_view::section_asset_icon(section)),
                        |group| JumpIcon::Brand(crate::brand_icons::api_group_path(group)),
                    ),
                    action: JumpAction::Navigate(Route::Cluster {
                        cluster_id: cluster.id.clone(),
                        section: section.into(),
                    }),
                    lease,
                });
            };
        for group in crate::navigation::ordered_cluster_sections() {
            if group.children.is_empty() {
                push(
                    group.default_child,
                    if group.default_child == "overview" {
                        "Open Cluster · Overview".into()
                    } else {
                        group.label.into()
                    },
                    None,
                    None,
                );
            } else {
                for child in group.children {
                    push(
                        child.id,
                        format!("{} · {}", group.label, child.label),
                        None,
                        None,
                    );
                }
            }
        }
        let page = custom_page.filter(|page| {
            lease.is_some_and(|(cluster_id, session_id)| {
                cluster_id == cluster.id
                    && page.cluster_id == cluster_id
                    && page.session_id == session_id
                    && page.kind == iceon_core::connection::ResourceKind::CustomResourceDefinitions
            })
        });
        for (api_group, definitions) in crate::custom_resource_navigation::grouped_definitions(page)
        {
            for definition in definitions {
                let section = format!("crd:{}", definition.name);
                let Some(page) = page else {
                    continue;
                };
                if !crate::custom_resource_navigation::discovered_route(
                    Some(page),
                    &cluster.id,
                    &page.session_id,
                    &section,
                ) {
                    continue;
                }
                let kind = definition
                    .columns
                    .get("kind")
                    .filter(|kind| !kind.is_empty())
                    .unwrap_or(&definition.name);
                push(
                    &section,
                    format!("Custom Resources · {api_group} · {kind}"),
                    Some(page.session_id.clone()),
                    Some(&api_group),
                );
            }
        }
        if !entries.is_empty() {
            groups.push(JumpGroup { title, entries });
        }
    }
    groups
}
impl IceonApp {
    pub(crate) fn toggle_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search_modal_open {
            self.close_command_palette(window, cx);
            return;
        }
        self.theme_picker_open = false;
        self.search_modal_open = true;
        self.command_state.update(cx, |state, cx| {
            state.set_query("", window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }
    pub(crate) fn close_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_modal_open = false;
        self.shell_focus.focus(window, cx);
        cx.notify();
    }
    pub(crate) fn render_search_modal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let destinations = destinations(
            &self.catalog_snapshot.clusters,
            self.cluster_session
                .as_ref()
                .filter(|session| session.status == "connected")
                .map(|session| (session.cluster_id.as_str(), session.session_id.as_str())),
            self.custom_definitions_page.as_ref(),
            |cluster_id, section| {
                self.cluster_route_visible(cluster_id, section)
                    && (!section.starts_with("crd:") || self.route_allowed(section))
            },
        );
        let count = destinations
            .iter()
            .map(|group| group.entries.len())
            .sum::<usize>();
        let mut command = Command::new(&self.command_state);
        // All entries are explicitly grouped: IndexPath.section is the model group,
        // and row is the original unfiltered item index, not a flattened result index.
        for group in &destinations {
            command = command.group(CommandGroup::new().label(group.title.clone()).items(
                group.entries.iter().map(|entry| CommandItem::new()
                    .label(entry.title.clone())
                    .keywords(entry.keywords.clone())
                    .icon(entry.icon.icon())
                    .checked(matches!(&entry.action, JumpAction::Navigate(route) if route == self.history.current())))
            ));
        }
        let owner = cx.entity().downgrade();
        let cancel_owner = owner.clone();
        command
            .w(px(680.0))
            .max_w(relative(0.9))
            .max_h(px(520.0))
            .placeholder("Quick Jump · Search actions, clusters, and resources…")
            .on_confirm(move |index, window, cx| {
                if let Some(entry) = entry_at(&destinations, index.section, index.row) {
                    let _ = owner.update(cx, |app, cx| {
                        if let JumpAction::Navigate(Route::Cluster {
                            cluster_id,
                            section,
                        }) = &entry.action
                        {
                            if !app
                                .catalog_snapshot
                                .clusters
                                .iter()
                                .any(|cluster| &cluster.id == cluster_id)
                                || !app.cluster_route_visible(cluster_id, section)
                            {
                                return;
                            }
                            if let Some(lease) = &entry.lease
                                && (!app.cluster_session.as_ref().is_some_and(|session| {
                                    &session.cluster_id == cluster_id
                                        && &session.session_id == lease
                                        && session.status == "connected"
                                }) || !app.route_allowed(section))
                            {
                                return;
                            }
                        }
                        app.close_command_palette(window, cx);
                        match &entry.action {
                            JumpAction::CreateCluster => app.open_add_config(window, cx),
                            JumpAction::Settings => app.open_preferences(window, cx),
                            JumpAction::Navigate(route) => {
                                app.navigate(route.clone(), true, window, cx)
                            }
                        }
                    });
                }
            })
            .on_cancel(move |window, cx| {
                let _ = cancel_owner.update(cx, |app, cx| app.close_command_palette(window, cx));
            })
            .footer(move |_, _, cx| {
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_3()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("{count} destinations"))
                    .child("↑↓ Navigate")
                    .child("Enter Open")
                    .child("Esc Clear / Close")
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{JumpAction, JumpGroup, JumpIcon, Route, destinations, entry_at};
    use iceon_core::{
        catalog::ClusterSummary,
        connection::{ResourceItem, ResourceKind, ResourcePage},
    };
    use std::collections::HashSet;

    fn clusters() -> Vec<ClusterSummary> {
        ["prod", "staging"]
            .into_iter()
            .map(|context| ClusterSummary {
                id: context.into(),
                name: "Shared".into(),
                context_name: context.into(),
                source: "kubeconfig".into(),
                status: "disconnected",
                pinned: false,
                pinned_sections: vec![],
                labels: vec![],
                color: "".into(),
            })
            .collect()
    }

    fn sections(group: &JumpGroup) -> Vec<&str> {
        group
            .entries
            .iter()
            .filter_map(|entry| match &entry.action {
                JumpAction::Navigate(Route::Cluster { section, .. }) => Some(section.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn global_actions_exist_without_clusters() {
        let groups = destinations(&[], None, None, |_, _| false);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].title, "Actions");
        assert_eq!(
            groups[0]
                .entries
                .iter()
                .map(|e| &e.action)
                .collect::<Vec<_>>(),
            [
                &JumpAction::CreateCluster,
                &JumpAction::Settings,
                &JumpAction::Navigate(Route::Secrets),
            ]
        );
    }

    #[test]
    fn every_cluster_gets_all_sidebar_leaves_without_aliases_or_duplicates() {
        let clusters = clusters();
        let groups = destinations(&clusters, None, None, |_, _| true);
        assert_eq!(groups[1].title, "Shared · prod");
        assert_eq!(groups[2].title, "Shared · staging");
        let expected: HashSet<_> = crate::navigation::ordered_cluster_sections()
            .into_iter()
            .flat_map(|group| {
                if group.children.is_empty() {
                    vec![group.default_child]
                } else {
                    group.children.iter().map(|child| child.id).collect()
                }
            })
            .collect();
        for (group, cluster) in groups[1..].iter().zip(&clusters) {
            let targets = sections(group);
            assert_eq!(targets.len(), expected.len());
            assert_eq!(targets.into_iter().collect::<HashSet<_>>(), expected);
            assert_eq!(
                sections(group).iter().filter(|s| **s == "overview").count(),
                1
            );
            for entry in &group.entries {
                assert!(
                    matches!(&entry.action, JumpAction::Navigate(Route::Cluster { cluster_id, .. }) if cluster_id == &cluster.id)
                );
            }
        }
        let pods = groups[1]
            .entries
            .iter()
            .find(|entry| entry.title == "Workloads · Pods")
            .unwrap();
        assert!(
            pods.keywords
                .iter()
                .any(|keyword| keyword.to_lowercase().contains("prod pods"))
        );
    }

    #[test]
    fn permissions_are_per_cluster_and_lookup_uses_group_and_original_row() {
        let groups = destinations(&clusters(), None, None, |cluster, section| {
            !(cluster == "prod" && section == "nodes")
        });
        assert!(!sections(&groups[1]).contains(&"nodes"));
        assert!(sections(&groups[2]).contains(&"nodes"));
        for (section, group) in groups.iter().enumerate() {
            for (row, entry) in group.entries.iter().enumerate() {
                assert_eq!(
                    entry_at(&groups, section, row).unwrap().action,
                    entry.action
                );
            }
        }
        assert_ne!(
            entry_at(&groups, 1, 0).unwrap().action,
            entry_at(&groups, 2, 0).unwrap().action
        );
        assert!(entry_at(&groups, 99, 0).is_none());
        assert!(entry_at(&groups, 0, 99).is_none());
        let denied = destinations(&clusters(), None, None, |_, _| false);
        assert_eq!(denied.len(), 1);
        assert_eq!(denied[0].entries.len(), 3);
    }

    fn custom_page() -> ResourcePage {
        ResourcePage {
            cluster_id: "prod".into(),
            session_id: "lease".into(),
            kind: ResourceKind::CustomResourceDefinitions,
            read_id: 1,
            items: vec![ResourceItem {
                name: "widgets.example.io".into(),
                uid: Some("fixture-uid".into()),
                namespace: None,
                resource_version: None,
                created_at: None,
                status: "Established".into(),
                node: None,
                containers: vec![],
                columns: [
                    ("group".into(), "example.io".into()),
                    ("kind".into(), "Widget".into()),
                ]
                .into(),
            }],
            truncated: false,
            fetched_at: 0,
            namespace: None,
            continue_token: None,
            metrics: None,
        }
    }

    #[test]
    fn custom_brand_icons_do_not_change_routes_or_original_row_lookup() {
        let mut page = custom_page();
        for (api_group, path) in [
            ("argoproj.io", "icons/custom-brand/argoproj.svg"),
            ("networking.gke.io", "icons/custom-brand/gke.svg"),
            ("example.io", crate::brand_icons::KUBERNETES_PATH),
        ] {
            page.items[0]
                .columns
                .insert("group".into(), api_group.into());
            let groups = destinations(&clusters(), Some(("prod", "lease")), Some(&page), |_, _| {
                true
            });
            let (row, entry) = groups[1]
                .entries
                .iter()
                .enumerate()
                .find(|(_, entry)| entry.lease.is_some())
                .unwrap();
            assert_eq!(entry.icon, JumpIcon::Brand(path));
            assert_eq!(entry_at(&groups, 1, row).unwrap().action, entry.action);
            assert_eq!(entry.lease.as_deref(), Some("lease"));
            assert!(
                matches!(&entry.action, JumpAction::Navigate(Route::Cluster { cluster_id, section }) if cluster_id == "prod" && section == "crd:widgets.example.io")
            );
            assert!(groups[2].entries.iter().all(|entry| entry.lease.is_none()));
        }
    }

    #[test]
    fn custom_types_require_matching_current_discovery_lease_and_permission() {
        let page = custom_page();
        let clusters = clusters();
        let custom = "crd:widgets.example.io";
        let groups = destinations(&clusters, Some(("prod", "lease")), Some(&page), |_, _| true);
        assert!(sections(&groups[1]).contains(&custom));
        assert!(!sections(&groups[2]).contains(&custom));
        let entry = groups[1]
            .entries
            .iter()
            .find(|e| e.lease.is_some())
            .unwrap();
        assert_eq!(entry.title, "Custom Resources · example.io · Widget");
        assert_eq!(entry.lease.as_deref(), Some("lease"));
        for lease in [None, Some(("prod", "stale")), Some(("staging", "lease"))] {
            let groups = destinations(&clusters, lease, Some(&page), |_, _| true);
            assert!(
                groups
                    .iter()
                    .all(|group| !sections(group).contains(&custom))
            );
        }
        let groups = destinations(
            &clusters,
            Some(("prod", "lease")),
            Some(&page),
            |_, section| section != custom,
        );
        assert!(
            groups
                .iter()
                .all(|group| !sections(group).contains(&custom))
        );
    }
}
