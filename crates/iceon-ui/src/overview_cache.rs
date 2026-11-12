//! Lease-scoped, bounded in-memory overview snapshots. Never persists grants or data.
use std::{collections::VecDeque, rc::Rc};

use iceon_core::connection::{ResourceKind, ResourcePage};

use crate::cluster_view::OverviewSnapshot;

const CAPACITY: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OverviewCacheKey {
    pub cluster_id: String,
    pub session_id: String,
    pub namespace: Option<String>,
}

#[derive(Default)]
pub(crate) struct OverviewCache {
    entries: VecDeque<(OverviewCacheKey, OverviewSnapshot)>,
}

impl OverviewCache {
    pub(crate) fn get(&mut self, key: &OverviewCacheKey) -> Option<OverviewSnapshot> {
        let index = self
            .entries
            .iter()
            .position(|(candidate, _)| candidate == key)?;
        let entry = self.entries.remove(index)?;
        let snapshot = entry.1.clone();
        self.entries.push_back(entry);
        Some(snapshot)
    }

    pub(crate) fn insert(&mut self, key: OverviewCacheKey, snapshot: OverviewSnapshot) {
        self.entries.retain(|(candidate, _)| candidate != &key);
        self.entries.push_back((key, snapshot));
        while self.entries.len() > CAPACITY {
            self.entries.pop_front();
        }
    }
}

/// Apply only a successful kind result. A failed fetch never calls this, preserving
/// the prior page; a successful empty page deliberately replaces prior content.
pub(crate) fn apply_page(snapshot: &mut OverviewSnapshot, page: ResourcePage) {
    let slot = match page.kind {
        ResourceKind::Nodes => &mut snapshot.nodes,
        ResourceKind::Namespaces => &mut snapshot.namespaces,
        ResourceKind::Pods => &mut snapshot.pods,
        ResourceKind::Deployments => &mut snapshot.deployments,
        ResourceKind::DaemonSets => &mut snapshot.daemonsets,
        ResourceKind::StatefulSets => &mut snapshot.statefulsets,
        ResourceKind::ReplicaSets => &mut snapshot.replicasets,
        ResourceKind::Jobs => &mut snapshot.jobs,
        ResourceKind::CronJobs => &mut snapshot.cronjobs,
        ResourceKind::Events => &mut snapshot.events,
        _ => return,
    };
    *slot = Some(Rc::new(page));
}

/// Current access checks still apply to cached data within the same lease.
pub(crate) fn retain_allowed(snapshot: &mut OverviewSnapshot, allowed: &[ResourceKind]) {
    let slots = [
        (ResourceKind::Nodes, &mut snapshot.nodes),
        (ResourceKind::Namespaces, &mut snapshot.namespaces),
        (ResourceKind::Pods, &mut snapshot.pods),
        (ResourceKind::Deployments, &mut snapshot.deployments),
        (ResourceKind::DaemonSets, &mut snapshot.daemonsets),
        (ResourceKind::StatefulSets, &mut snapshot.statefulsets),
        (ResourceKind::ReplicaSets, &mut snapshot.replicasets),
        (ResourceKind::Jobs, &mut snapshot.jobs),
        (ResourceKind::CronJobs, &mut snapshot.cronjobs),
        (ResourceKind::Events, &mut snapshot.events),
    ];
    for (kind, slot) in slots {
        if !allowed.contains(&kind) {
            *slot = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(cluster: &str, lease: &str, namespace: Option<&str>) -> OverviewCacheKey {
        OverviewCacheKey {
            cluster_id: cluster.into(),
            session_id: lease.into(),
            namespace: namespace.map(str::to_owned),
        }
    }

    fn page(kind: ResourceKind, read_id: u64) -> ResourcePage {
        ResourcePage {
            cluster_id: "cluster".into(),
            session_id: "lease".into(),
            kind,
            read_id,
            items: vec![],
            truncated: false,
            fetched_at: 0,
            namespace: None,
            continue_token: None,
            metrics: None,
        }
    }

    #[test]
    fn cache_isolates_cluster_lease_and_namespace() {
        let mut cache = OverviewCache::default();
        cache.insert(key("a", "lease1", None), OverviewSnapshot::default());
        assert!(cache.get(&key("a", "lease1", None)).is_some());
        assert!(cache.get(&key("b", "lease1", None)).is_none());
        assert!(cache.get(&key("a", "lease2", None)).is_none());
        assert!(cache.get(&key("a", "lease1", Some("default"))).is_none());
    }

    #[test]
    fn cache_evicts_least_recently_used_and_replaces_without_growth() {
        let mut cache = OverviewCache::default();
        for index in 0..8 {
            cache.insert(
                key(&index.to_string(), "lease", None),
                OverviewSnapshot::default(),
            );
        }
        cache.get(&key("0", "lease", None));
        cache.insert(key("8", "lease", None), OverviewSnapshot::default());
        assert_eq!(cache.entries.len(), 8);
        assert!(cache.get(&key("1", "lease", None)).is_none());
        assert!(cache.get(&key("0", "lease", None)).is_some());
        cache.insert(key("0", "lease", None), OverviewSnapshot::default());
        assert_eq!(cache.entries.len(), 8);
    }

    #[test]
    fn independent_publications_preserve_other_kinds_and_empty_success_replaces() {
        let mut snapshot = OverviewSnapshot::default();
        let mut prior = page(ResourceKind::Pods, 1);
        prior.truncated = true;
        apply_page(&mut snapshot, prior);
        let pods = snapshot.pods.clone().unwrap();
        apply_page(&mut snapshot, page(ResourceKind::Events, 2));
        assert!(Rc::ptr_eq(&pods, snapshot.pods.as_ref().unwrap()));
        // A failed kind is not applied; publishing another kind retains its old data.
        apply_page(&mut snapshot, page(ResourceKind::Nodes, 3));
        assert_eq!(snapshot.pods.as_ref().unwrap().read_id, 1);
        apply_page(&mut snapshot, page(ResourceKind::Pods, 4));
        assert_eq!(snapshot.pods.as_ref().unwrap().read_id, 4);
        assert!(snapshot.pods.as_ref().unwrap().items.is_empty());
        assert_eq!(snapshot.events.as_ref().unwrap().read_id, 2);
        retain_allowed(&mut snapshot, &[ResourceKind::Events]);
        assert!(snapshot.pods.is_none());
        assert!(snapshot.events.is_some());
    }
}
