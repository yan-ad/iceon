//! Lease-scoped CRD discovery for navigation, independent of the Definitions table.
use crate::IceonApp;
use gpui_kit::{prelude::*, *};
use iceon_core::connection::{
    ResourceItem, ResourcePage, custom_resources::CustomDefinitionRequest,
};
use std::collections::BTreeMap;

pub(crate) fn grouped_definitions(
    page: Option<&ResourcePage>,
) -> BTreeMap<String, Vec<ResourceItem>> {
    let mut groups = BTreeMap::<String, Vec<ResourceItem>>::new();
    if let Some(page) = page {
        for item in &page.items {
            if let Some(group) = item.columns.get("group").filter(|group| !group.is_empty()) {
                groups.entry(group.clone()).or_default().push(item.clone());
            }
        }
    }
    for items in groups.values_mut() {
        items.sort_by(|a, b| {
            a.columns
                .get("kind")
                .cmp(&b.columns.get("kind"))
                .then(a.name.cmp(&b.name))
        });
    }
    groups
}
impl IceonApp {
    pub(crate) fn discover_custom_resources(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        self.custom_definitions_page = None;
        self.custom_discovery_error = None;
        self.expanded_custom_groups.clear();
        let generation = self.next_operation_id();
        self.custom_discovery_generation = generation;
        self.custom_discovery_loading = true;
        let connection = self.connection.clone();
        cx.spawn_in(window, async move |owner, cx| {
            let mut token = None;
            let mut tokens = std::collections::HashSet::new();
            let mut aggregate: Option<ResourcePage> = None;
            loop {
                let read_id = owner.update_in(cx, |app, _, _| {
                    (app.custom_discovery_generation == generation && app.cluster_session.as_ref().is_some_and(|s| s.cluster_id == session.cluster_id && s.session_id == session.session_id)).then(|| app.next_operation_id())
                }).ok().flatten();
                let Some(read_id) = read_id else { return; };
                let result = connection.custom_resource_definitions(CustomDefinitionRequest { cluster_id: session.cluster_id.clone(), session_id: session.session_id.clone(), read_id, continue_token: token.clone() }).await;
                let page = match result {
                    Ok(page) => page,
                    Err(error) => {
                        let _ = owner.update_in(cx, |app, _, cx| {
                            if app.custom_discovery_generation == generation && app.cluster_session.as_ref().is_some_and(|s| s.cluster_id == session.cluster_id && s.session_id == session.session_id) {
                                app.custom_discovery_loading = false;
                                app.custom_discovery_error = Some(format!("CRD discovery failed: {error:?}"));
                                if crate::cluster_view::parse_custom_resource_target(&app.active_cluster_section).is_some() {
                                    app.cluster_loading = false;
                                    app.cluster_error = app.custom_discovery_error.clone();
                                }
                                cx.notify();
                            }
                        }); return;
                    }
                };
                token = page.continue_token.clone();
                if let Some(all) = &mut aggregate {
                    for item in page.items { if !all.items.iter().any(|old| old.name == item.name) { all.items.push(item); } }
                    all.continue_token = token.clone(); all.truncated = page.truncated;
                } else { aggregate = Some(page); }
                let limited = aggregate.as_ref().is_some_and(|page| page.items.len() >= 10000);
                let repeated = token.as_ref().is_some_and(|token| !tokens.insert(token.clone()));
                let done = token.is_none() || limited || repeated;
                let snapshot = aggregate.clone();
                let _ = owner.update_in(cx, |app, window, cx| {
                    if app.custom_discovery_generation != generation || !app.cluster_session.as_ref().is_some_and(|s| s.cluster_id == session.cluster_id && s.session_id == session.session_id) { return; }
                    app.custom_definitions_page = snapshot;
                    app.custom_discovery_loading = !done;
                    if limited || repeated { app.custom_discovery_error = Some("CRD discovery incomplete: catalog limit or repeated continuation token. Refresh to retry.".into()); }
                    if done && crate::cluster_view::parse_custom_resource_target(&app.active_cluster_section).is_some() { app.reload_active_section(window, cx); }
                    cx.notify();
                });
                if done { return; }
            }
        }).detach();
    }
}

#[cfg(test)]
mod tests {
    use super::grouped_definitions;
    use iceon_core::connection::{ResourceItem, ResourceKind, ResourcePage};

    #[test]
    fn navigation_groups_all_definitions_without_the_old_25_item_cap() {
        let items = (0..60)
            .rev()
            .map(|index| ResourceItem {
                name: format!("resource-{index:02}.example.io"),
                namespace: None,
                uid: Some(format!("uid-{index}")),
                resource_version: Some("1".into()),
                created_at: None,
                status: "Established".into(),
                node: None,
                containers: Vec::new(),
                columns: [
                    (
                        "group".into(),
                        if index % 2 == 0 {
                            "alpha.io"
                        } else {
                            "beta.io"
                        }
                        .into(),
                    ),
                    ("kind".into(), format!("Resource{index:02}")),
                ]
                .into(),
            })
            .collect();
        let page = ResourcePage {
            cluster_id: "cluster".into(),
            session_id: "lease".into(),
            kind: ResourceKind::CustomResourceDefinitions,
            read_id: 1,
            items,
            truncated: false,
            fetched_at: 0,
            namespace: None,
            continue_token: None,
            metrics: None,
        };
        let groups = grouped_definitions(Some(&page));
        assert_eq!(
            groups.keys().cloned().collect::<Vec<_>>(),
            ["alpha.io", "beta.io"]
        );
        assert_eq!(groups.values().map(Vec::len).sum::<usize>(), 60);
        assert_eq!(groups["alpha.io"][0].columns["kind"], "Resource00");
        assert_eq!(groups["beta.io"][0].columns["kind"], "Resource01");
    }

    #[test]
    fn unavailable_catalog_does_not_invent_sidebar_groups() {
        assert!(grouped_definitions(None).is_empty());
    }
}
