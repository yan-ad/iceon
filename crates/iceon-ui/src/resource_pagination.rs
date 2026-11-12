//! Bounded UI accumulation; transport limits still apply independently to every request.
use iceon_core::connection::{ResourcePage, ResourceRequest};
use std::collections::{HashMap, HashSet};

pub(crate) const MAX_ITEMS: usize = 50_000;
const MAX_PAGES: usize = 2_048;

#[derive(Default)]
pub(crate) struct ResourcePagination {
    pub request: Option<ResourceRequest>,
    pub loading: bool,
    pub stopped: bool,
    tokens: HashSet<String>,
    pages: usize,
}
impl ResourcePagination {
    pub fn accepts(&self, request: &ResourceRequest, page: &ResourcePage) -> bool {
        self.request.as_ref() == Some(request)
            && page.read_id == request.read_id
            && page.cluster_id == request.cluster_id
            && page.session_id == request.session_id
            && page.namespace == request.namespace
            && page.kind == request.kind
    }
    pub fn begin_next(&mut self, token: &str) -> Result<(), &'static str> {
        if self.loading || self.stopped {
            return Err("Resource scan is not available");
        }
        if self.pages >= MAX_PAGES || self.tokens.len() >= MAX_PAGES {
            self.stopped = true;
            return Err(
                "Resource scan stopped at the page limit; results are incomplete. Narrow the namespace or search.",
            );
        }
        if !self.tokens.insert(token.to_owned()) {
            self.stopped = true;
            return Err(
                "Resource scan returned a repeated continuation token; results are incomplete. Refresh to retry.",
            );
        }
        Ok(())
    }
    pub fn completed(&mut self) {
        self.loading = false;
        self.pages += 1;
    }
}

/// Namespace/name is the API object identity; newer UIDs replace recreated objects.
/// Preserve insertion order and update overlapping objects rather than adding duplicates.
pub(crate) fn merge_page(
    old: Option<&ResourcePage>,
    mut next: ResourcePage,
) -> (ResourcePage, bool) {
    let mut items = old.map(|p| p.items.clone()).unwrap_or_default();
    let mut positions: HashMap<_, _> = items
        .iter()
        .enumerate()
        .map(|(i, item)| ((item.namespace.clone(), item.name.clone()), i))
        .collect();
    let mut limited = false;
    for item in next.items.drain(..) {
        let key = (item.namespace.clone(), item.name.clone());
        if let Some(&index) = positions.get(&key) {
            items[index] = item;
        } else if items.len() < MAX_ITEMS {
            positions.insert(key, items.len());
            items.push(item);
        } else {
            limited = true;
        }
    }
    next.items = items;
    if limited {
        next.truncated = true;
    }
    (next, limited)
}

#[cfg(test)]
mod tests {
    use super::*;
    use iceon_core::connection::{ResourceItem, ResourceKind};
    fn request() -> ResourceRequest {
        ResourceRequest {
            cluster_id: "c".into(),
            session_id: "s".into(),
            kind: ResourceKind::Pods,
            read_id: 42,
            namespace: Some("ns".into()),
            continue_token: None,
            query: Some("hidden".into()),
            status: None,
            group: None,
        }
    }
    fn page(names: &[&str], token: Option<&str>) -> ResourcePage {
        let r = request();
        ResourcePage {
            cluster_id: r.cluster_id,
            session_id: r.session_id,
            kind: r.kind,
            read_id: r.read_id,
            namespace: r.namespace,
            fetched_at: 0,
            metrics: None,
            truncated: token.is_some(),
            continue_token: token.map(str::to_owned),
            items: names
                .iter()
                .map(|name| ResourceItem {
                    name: (*name).into(),
                    namespace: Some("ns".into()),
                    uid: None,
                    resource_version: None,
                    status: "Running".into(),
                    created_at: None,
                    columns: Default::default(),
                    node: None,
                    containers: Vec::new(),
                })
                .collect(),
        }
    }
    #[test]
    fn append_deduplicates_and_updates_without_reordering() {
        let first = page(&["a", "b"], Some("next"));
        let mut next = page(&["b", "c", "c"], None);
        next.items[0].uid = Some("recreated".into());
        let (merged, limited) = merge_page(Some(&first), next);
        assert!(!limited);
        assert_eq!(
            merged
                .items
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        assert_eq!(merged.items[1].uid.as_deref(), Some("recreated"));
        assert_eq!(merged.continue_token, None);
    }
    #[test]
    fn rejects_stale_response_identity_and_request_scope() {
        let r = request();
        let state = ResourcePagination {
            request: Some(r.clone()),
            ..Default::default()
        };
        assert!(state.accepts(&r, &page(&[], None)));
        for field in 0..5 {
            let mut stale = page(&[], None);
            match field {
                0 => stale.read_id += 1,
                1 => stale.session_id = "old".into(),
                2 => stale.namespace = None,
                3 => stale.cluster_id = "other".into(),
                _ => stale.kind = ResourceKind::Nodes,
            }
            assert!(!state.accepts(&r, &stale));
        }
        for field in 0..5 {
            let mut stale_request = r.clone();
            match field {
                0 => stale_request.query = None,
                1 => stale_request.status = Some("Running".into()),
                2 => stale_request.group = Some("other".into()),
                3 => stale_request.continue_token = Some("previous-page".into()),
                _ => stale_request.read_id += 1,
            }
            assert!(!state.accepts(&stale_request, &page(&[], None)));
        }
        let cancelled = ResourcePagination::default();
        assert!(!cancelled.accepts(&r, &page(&[], None)));
    }
    #[test]
    fn empty_search_pages_continue_and_token_cycles_stop() {
        let first = page(&[], Some("next"));
        let mut state = ResourcePagination::default();
        state.completed();
        assert!(
            state
                .begin_next(first.continue_token.as_deref().unwrap())
                .is_ok()
        );
        let (merged, limited) = merge_page(Some(&first), page(&["hidden"], None));
        assert!(!limited);
        assert_eq!(merged.items.len(), 1);
        assert!(state.begin_next("next").is_err());
        assert!(state.stopped);
    }
    #[test]
    fn in_flight_and_scan_budget_prevent_duplicate_or_unbounded_fetches() {
        let mut state = ResourcePagination {
            loading: true,
            ..Default::default()
        };
        assert!(state.begin_next("next").is_err());
        state.loading = false;
        state.pages = MAX_PAGES;
        assert!(state.begin_next("next").is_err());
        assert!(state.stopped);
    }
    #[test]
    fn retained_inventory_is_bounded() {
        let mut first = page(&[], Some("next"));
        first.items = (0..MAX_ITEMS)
            .map(|i| {
                let mut item = page(&["a"], None).items.remove(0);
                item.name = i.to_string();
                item
            })
            .collect();
        let (merged, limited) = merge_page(Some(&first), page(&["extra"], None));
        assert!(limited);
        assert_eq!(merged.items.len(), MAX_ITEMS);
        assert!(merged.truncated);
    }
}
