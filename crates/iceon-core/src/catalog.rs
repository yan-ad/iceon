//! Catalog workflow reference: Freelens v1.10.3-533, cd07118f843c4a69f937a319a37a7c12ef2c92d8,
//! common/catalog/catalog-entity.ts (behavior only).
//! CatalogEngine owns imported disconnected contexts, entry pins and cluster-section pins.
//! Snapshot results cover empty/loaded states; sanitized errors cover rejected/unavailable states.
//! Acceptance fixtures live in catalog/tests.rs.

use crate::preferences::KubernetesPreferences;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
};

const MAX_INPUT: usize = 1024 * 1024;
const MAX_STORE: usize = 4 * 1024 * 1024;
const MAX_ENTRIES: usize = 1000;
const MAX_NAME: usize = 256;
const MAX_PATH: usize = 4096;

// Static destinations in iceon-ui/src/navigation.rs; dynamic custom-resource instances
// are deliberately not catalog pin targets.
pub const SECTIONS: &[&str] = &[
    "overview",
    "nodes",
    "workloads",
    "workloads-overview",
    "pods",
    "deployments",
    "daemon-sets",
    "stateful-sets",
    "replica-sets",
    "replication-controllers",
    "jobs",
    "cron-jobs",
    "config",
    "config-maps",
    "secrets",
    "resource-quotas",
    "limit-ranges",
    "horizontal-pod-autoscalers",
    "vertical-pod-autoscalers",
    "pod-disruption-budgets",
    "priority-classes",
    "runtime-classes",
    "leases",
    "mutating-webhook-configs",
    "validating-webhook-configs",
    "validating-admission-policies",
    "validating-admission-policy-bindings",
    "network",
    "services",
    "endpoint-slices",
    "endpoints",
    "ingresses",
    "ingress-classes",
    "network-policies",
    "port-forwarding",
    "storage",
    "persistent-volume-claims",
    "persistent-volumes",
    "storage-classes",
    "namespaces",
    "events",
    "helm",
    "helm-charts",
    "helm-releases",
    "access-control",
    "users",
    "service-accounts",
    "cluster-roles",
    "roles",
    "cluster-role-bindings",
    "role-bindings",
    "pod-security-policies",
    "custom-resources",
    "custom-resource-definitions",
];

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CatalogError {
    InvalidInput,
    InvalidKubeconfig,
    InputUnavailable,
    LimitExceeded,
    NotFound,
    StoreUnavailable,
    StoreCorrupt,
    OpenFailed,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PinTarget {
    pub id: String,
    pub section: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReorderPinsRequest {
    pub items: Vec<PinTarget>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SectionPinRequest {
    pub cluster_id: String,
    pub section: String,
    pub pinned: bool,
}

/// Ephemeral connection state projected onto a catalog entry. This is never
/// persisted with the catalog metadata.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ClusterConnectionState {
    pub cluster_id: String,
    pub status: &'static str,
}
type Result<T> = std::result::Result<T, CatalogError>;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSnapshot {
    pub revision: u64,
    pub clusters: Vec<ClusterSummary>,
    pub pinned_items: Vec<PinTarget>,
    pub discovery_error: Option<CatalogError>,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ClusterSummary {
    pub id: String,
    pub name: String,
    pub context_name: String,
    pub source: String,
    pub status: &'static str,
    pub pinned: bool,
    pub pinned_sections: Vec<String>,
    pub labels: Vec<String>,
    pub color: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportRequest {
    pub path: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinRequest {
    pub id: String,
    pub pinned: bool,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoveRequest {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConnectionTarget {
    pub source_path: PathBuf,
    pub provenance_path: PathBuf,
    pub context_name: String,
}

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Store {
    schema_version: u32,
    revision: u64,
    clusters: Vec<ClusterRecord>,
    #[serde(default)]
    pin_order: Vec<PinTarget>,
    auto_discover: bool, // Legacy storage field; discovery no longer consults it.
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ClusterRecord {
    id: String,
    context_name: String,
    source_path: String,
    provenance_path: String,
    pinned: bool,
    #[serde(default)]
    pinned_sections: Vec<String>,
    color: String,
}
impl Store {
    fn empty() -> Self {
        Self {
            schema_version: 5,
            ..Self::default()
        }
    }
    fn snapshot(
        &self,
        home: Option<&Path>,
        discovery_error: Option<CatalogError>,
    ) -> CatalogSnapshot {
        let mut clusters: Vec<_> = self
            .clusters
            .iter()
            .map(|c| ClusterSummary {
                id: c.id.clone(),
                name: c.context_name.clone(),
                context_name: c.context_name.clone(),
                source: "kubeconfig".into(),
                labels: vec![source_label(&c.provenance_path, home)],
                color: c.color.clone(),
                status: "disconnected",
                pinned: c.pinned,
                pinned_sections: c.pinned_sections.clone(),
            })
            .collect();
        clusters.sort_by(|a, b| (&a.name, &a.id).cmp(&(&b.name, &b.id)));
        CatalogSnapshot {
            revision: self.revision,
            clusters,
            pinned_items: self.pin_order.clone(),
            discovery_error,
        }
    }
    fn current_pins(&self) -> Vec<PinTarget> {
        let mut clusters: Vec<_> = self.clusters.iter().collect();
        clusters.sort_by(|a, b| (&a.context_name, &a.id).cmp(&(&b.context_name, &b.id)));
        let mut pins: Vec<_> = clusters
            .iter()
            .filter(|c| c.pinned)
            .map(|c| PinTarget {
                id: c.id.clone(),
                section: None,
            })
            .collect();
        for cluster in clusters {
            for section in SECTIONS.iter().filter(|section| **section != "overview") {
                if cluster
                    .pinned_sections
                    .iter()
                    .any(|pinned| pinned == section)
                {
                    pins.push(PinTarget {
                        id: cluster.id.clone(),
                        section: Some((*section).into()),
                    });
                }
            }
        }
        pins
    }

    fn reconcile_pins(&mut self) -> bool {
        let current = self.current_pins();
        let valid: HashSet<_> = current.iter().collect();
        let previous = self.pin_order.clone();
        let mut retained = HashSet::new();
        self.pin_order
            .retain(|target| valid.contains(target) && retained.insert(target.clone()));
        for target in current {
            if retained.insert(target.clone()) {
                self.pin_order.push(target);
            }
        }
        self.pin_order != previous
    }

    fn validate(&self) -> Result<()> {
        if self.schema_version != 5 || self.clusters.len() > MAX_ENTRIES {
            return Err(CatalogError::StoreCorrupt);
        }
        let mut targets = HashSet::new();
        if self.pin_order.len() > MAX_ENTRIES * SECTIONS.len()
            || self
                .pin_order
                .iter()
                .any(|target| !valid_pin_target(target) || !targets.insert(target))
        {
            return Err(CatalogError::StoreCorrupt);
        }
        let mut ids = HashSet::new();
        for c in &self.clusters {
            let path = Path::new(&c.source_path);
            let mut sections = HashSet::new();
            if !valid_name(&c.context_name)
                || c.pinned_sections.len() > SECTIONS.len()
                || c.pinned_sections.iter().any(|section| {
                    !SECTIONS.contains(&section.as_str()) || !sections.insert(section)
                })
                || c.source_path.len() > MAX_PATH
                || !path.is_absolute()
                || !path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .is_some_and(valid_name)
                || !valid_source_path(&c.provenance_path)
                || c.id != cluster_id(&c.source_path, &c.context_name)
                || !valid_color(&c.color)
                || !ids.insert(&c.id)
            {
                return Err(CatalogError::StoreCorrupt);
            }
        }
        Ok(())
    }
}

mod storage;

// Home is injected by the application runtime. Constructors never read it or infer a host HOME.
// Discovery is a bounded metadata import on list, not a watcher or connection.
pub struct CatalogEngine {
    directory: PathBuf,
    home: Option<PathBuf>,
    operations: Mutex<()>,
}
impl CatalogEngine {
    pub(crate) fn namespace_cache_storage<T>(
        &self,
        operation: impl FnOnce(&mut rusqlite::Connection) -> Result<T>,
    ) -> Result<T> {
        let _guard = self
            .operations
            .lock()
            .map_err(|_| CatalogError::StoreUnavailable)?;
        let _lock = storage::lock(&self.directory)?;
        let mut connection = storage::open(&self.directory)?;
        storage::ensure_namespace_cache(&connection)?;
        operation(&mut connection)
    }
    #[cfg(test)]
    pub fn new(directory: PathBuf) -> Self {
        Self::new_with_home(directory, None)
    }
    pub fn new_with_home(directory: PathBuf, home: Option<PathBuf>) -> Self {
        Self {
            directory,
            home,
            operations: Mutex::new(()),
        }
    }
    pub fn list(&self) -> Result<CatalogSnapshot> {
        self.change_inner(|_| Ok(false), true, None, None)
    }

    /// Reconcile metadata from the enabled default and configured files/directories.
    /// Directory scans are shallow and never follow child symlinks. Invalid/unreadable
    /// inputs retain their last valid metadata and report a sanitized discovery error.
    pub fn list_with_preferences(
        &self,
        preferences: &KubernetesPreferences,
    ) -> Result<CatalogSnapshot> {
        self.change_inner(|_| Ok(false), false, Some(preferences), None)
    }

    pub(crate) fn connection_target(&self, id: &str) -> Result<ConnectionTarget> {
        if !valid_name(id) {
            return Err(CatalogError::InvalidInput);
        }
        let _guard = self
            .operations
            .lock()
            .map_err(|_| CatalogError::StoreUnavailable)?;
        let store = storage::read_existing(&self.directory)?;
        let cluster = store
            .clusters
            .iter()
            .find(|cluster| cluster.id == id)
            .ok_or(CatalogError::NotFound)?;
        Ok(ConnectionTarget {
            source_path: PathBuf::from(&cluster.source_path),
            provenance_path: PathBuf::from(&cluster.provenance_path),
            context_name: cluster.context_name.clone(),
        })
    }
    fn change(&self, update: impl FnOnce(&mut Store) -> Result<bool>) -> Result<CatalogSnapshot> {
        self.change_inner(update, false, None, None)
    }
    fn change_inner(
        &self,
        update: impl FnOnce(&mut Store) -> Result<bool>,
        discover: bool,
        preferences: Option<&KubernetesPreferences>,
        manual_path: Option<&str>,
    ) -> Result<CatalogSnapshot> {
        let _guard = self
            .operations
            .lock()
            .map_err(|_| CatalogError::StoreUnavailable)?;
        let _file_lock = storage::lock(&self.directory)?;
        let mut connection = storage::open(&self.directory)?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage::sql_error)?;
        // Separate metadata-only ownership ledger avoids changing legacy catalog schemas.
        transaction.execute_batch("CREATE TABLE IF NOT EXISTS kubeconfig_sync_ownership (id TEXT PRIMARY KEY NOT NULL, alias TEXT NOT NULL) STRICT;").map_err(storage::sql_error)?;
        let mut store = storage::load(&transaction)?;
        let mut changed = update(&mut store)?;
        let mut discovery_error = None;
        if discover && let Some(home) = &self.home {
            match discover_default(&mut store, &home.join(".kube/config")) {
                Ok(import_changed) => changed |= import_changed,
                Err(error) => discovery_error = Some(error),
            }
        }
        if let Some(path) = manual_path {
            // Explicit imports take ownership, including an already-synchronized source.
            let source = fs::canonicalize(path).map_err(|_| CatalogError::InputUnavailable)?;
            for record in &store.clusters {
                if Path::new(&record.source_path) == source {
                    transaction
                        .execute(
                            "DELETE FROM kubeconfig_sync_ownership WHERE id=?1",
                            [&record.id],
                        )
                        .map_err(storage::sql_error)?;
                }
            }
        }
        if let Some(preferences) = preferences {
            let (sync_changed, error) =
                sync_preferences(&mut store, &transaction, self.home.as_deref(), preferences)?;
            changed |= sync_changed;
            discovery_error = error;
        }
        changed |= store.reconcile_pins();
        if changed {
            store.revision = store
                .revision
                .checked_add(1)
                .ok_or(CatalogError::LimitExceeded)?;
            storage::save(&transaction, &store)?;
        }
        transaction.commit().map_err(storage::sql_error)?;
        Ok(store.snapshot(self.home.as_deref(), discovery_error))
    }
    pub fn import_kubeconfig(&self, request: ImportRequest) -> Result<CatalogSnapshot> {
        if request.path.is_empty()
            || request.path.len() > MAX_PATH
            || request.path.contains('\0')
            || !Path::new(&request.path).is_absolute()
        {
            return Err(CatalogError::InvalidInput);
        }
        self.change_inner(
            |store| reconcile(store, Path::new(&request.path)),
            false,
            None,
            Some(&request.path),
        )
    }
    pub fn reorder_pins(&self, request: ReorderPinsRequest) -> Result<CatalogSnapshot> {
        self.change(|store| {
            let current: HashSet<_> = store.pin_order.iter().collect();
            let requested: HashSet<_> = request.items.iter().collect();
            if request.items.len() != current.len()
                || requested.len() != request.items.len()
                || request.items.iter().any(|target| !valid_pin_target(target))
                || requested != current
            {
                return Err(CatalogError::InvalidInput);
            }
            let changed = store.pin_order != request.items;
            store.pin_order = request.items;
            Ok(changed)
        })
    }

    #[allow(dead_code)] // Native menu consumers validate saved targets without discovery.
    pub fn validate_pin_target(&self, target: &PinTarget) -> Result<()> {
        if !valid_pin_target(target) {
            return Err(CatalogError::InvalidInput);
        }
        let _guard = self
            .operations
            .lock()
            .map_err(|_| CatalogError::StoreUnavailable)?;
        let store = storage::read_existing(&self.directory)?;
        if store.pin_order.contains(target) {
            Ok(())
        } else {
            Err(CatalogError::NotFound)
        }
    }

    pub fn set_pin(&self, request: PinRequest) -> Result<CatalogSnapshot> {
        if request.id.len() > MAX_NAME {
            return Err(CatalogError::InvalidInput);
        }
        self.change(|store| {
            let pin = if let Some(c) = store.clusters.iter_mut().find(|c| c.id == request.id) {
                &mut c.pinned
            } else {
                return Err(CatalogError::NotFound);
            };
            let changed = *pin != request.pinned;
            *pin = request.pinned;
            Ok(changed)
        })
    }
    pub fn set_section_pin(&self, request: SectionPinRequest) -> Result<CatalogSnapshot> {
        if !valid_name(&request.cluster_id) || !SECTIONS.contains(&request.section.as_str()) {
            return Err(CatalogError::InvalidInput);
        }
        self.change(|store| {
            let cluster = store
                .clusters
                .iter_mut()
                .find(|cluster| cluster.id == request.cluster_id)
                .ok_or(CatalogError::NotFound)?;
            if request.section == "overview" {
                let changed = cluster.pinned != request.pinned;
                cluster.pinned = request.pinned;
                return Ok(changed);
            }
            let position = cluster
                .pinned_sections
                .iter()
                .position(|section| section == &request.section);
            match (position, request.pinned) {
                (None, true) => {
                    if cluster.pinned_sections.len() >= SECTIONS.len() {
                        return Err(CatalogError::LimitExceeded);
                    }
                    cluster.pinned_sections.push(request.section);
                }
                (Some(position), false) => {
                    cluster.pinned_sections.remove(position);
                }
                _ => return Ok(false),
            }
            Ok(true)
        })
    }
    pub fn remove(&self, request: RemoveRequest) -> Result<CatalogSnapshot> {
        if request.id.len() > MAX_NAME {
            return Err(CatalogError::InvalidInput);
        }
        self.change(|store| {
            let count = store.clusters.len();
            store.clusters.retain(|c| c.id != request.id);
            if count == store.clusters.len() {
                return Err(CatalogError::NotFound);
            }
            Ok(true)
        })
    }
}
fn valid_pin_target(target: &PinTarget) -> bool {
    valid_name(&target.id)
        && target
            .section
            .as_deref()
            .is_none_or(|section| section != "overview" && SECTIONS.contains(&section))
}

fn source_label(source: &str, home: Option<&Path>) -> String {
    if home.is_some_and(|home| {
        let default = home.join(".kube/config");
        Path::new(source) == default
    }) {
        "file=~/.kube/config".into()
    } else {
        format!("file={source}")
    }
}
fn valid_color(color: &str) -> bool {
    color.len() == 7
        && color.starts_with('#')
        && color.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
}
fn new_color(store: &Store) -> Result<String> {
    color_with(store, |bytes| {
        getrandom::fill(bytes).map_err(|_| CatalogError::StoreUnavailable)
    })
}
fn color_with(store: &Store, mut fill: impl FnMut(&mut [u8]) -> Result<()>) -> Result<String> {
    for _ in 0..128 {
        let mut bytes = [0; 3];
        fill(&mut bytes)?;
        let color = format!("#{:02X}{:02X}{:02X}", bytes[0], bytes[1], bytes[2]);
        if !store.clusters.iter().any(|c| c.color == color) {
            return Ok(color);
        }
    }
    Err(CatalogError::StoreUnavailable)
}
// At most 64 roots, 512 directory entries and 64 files (64 MiB of input) per refresh.
// Never recurse: a sync folder is a collection of kubeconfig files, not a filesystem crawl.
fn sync_preferences(
    store: &mut Store,
    transaction: &rusqlite::Transaction<'_>,
    home: Option<&Path>,
    preferences: &KubernetesPreferences,
) -> Result<(bool, Option<CatalogError>)> {
    let mut ownership: HashMap<String, String> = {
        let mut statement = transaction
            .prepare("SELECT id, alias FROM kubeconfig_sync_ownership LIMIT 1001")
            .map_err(storage::sql_error)?;
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(storage::sql_error)?;
        rows.collect::<std::result::Result<_, _>>()
            .map_err(storage::sql_error)?
    };
    if ownership.len() > MAX_ENTRIES {
        return Err(CatalogError::StoreCorrupt);
    }
    if preferences.kubeconfig_sync_paths.len() > 64 {
        return Ok((false, Some(CatalogError::LimitExceeded)));
    }
    let before = serde_json::to_vec(&store.clusters).map_err(|_| CatalogError::StoreUnavailable)?;
    let mut roots = preferences
        .kubeconfig_sync_paths
        .iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    if preferences.discover_default_kubeconfig
        && let Some(home) = home
    {
        roots.push(home.join(".kube/config"));
    }
    let mut error = None;
    let mut candidates = Vec::new();
    let mut retained_aliases = HashSet::new();
    let mut entries = 0;
    // On an incomplete folder scan retain all its previous rows: absence is not proven.
    for root in roots {
        let Some(alias) = root.to_str().filter(|path| valid_source_path(path)) else {
            error.get_or_insert(CatalogError::InvalidInput);
            // Invalid preferences must not accidentally delete previously synchronized rows.
            retained_aliases.extend(ownership.values().cloned());
            continue;
        };
        match fs::metadata(&root) {
            Ok(metadata) if metadata.is_dir() => {
                let scan = (|| -> Result<Vec<PathBuf>> {
                    let mut files = Vec::new();
                    for entry in fs::read_dir(&root).map_err(|_| CatalogError::InputUnavailable)? {
                        entries += 1;
                        if entries > 512 {
                            return Err(CatalogError::LimitExceeded);
                        }
                        let entry = entry.map_err(|_| CatalogError::InputUnavailable)?;
                        let kind = entry
                            .file_type()
                            .map_err(|_| CatalogError::InputUnavailable)?;
                        if kind.is_file() {
                            files.push(entry.path());
                        }
                    }
                    files.sort();
                    Ok(files)
                })();
                match scan {
                    Ok(files) => candidates.extend(files),
                    Err(problem) => {
                        error.get_or_insert(problem);
                        retained_aliases.extend(
                            ownership
                                .values()
                                .filter(|alias| Path::new(alias).parent() == Some(root.as_path()))
                                .cloned(),
                        );
                    }
                }
            }
            Ok(metadata) if metadata.is_file() => candidates.push(root),
            Err(problem) if problem.kind() == std::io::ErrorKind::NotFound => {
                // A broken symlink is not confirmed deletion. Keep its last valid rows.
                if fs::symlink_metadata(&root).is_ok() {
                    retained_aliases.insert(alias.to_owned());
                    retained_aliases.extend(
                        ownership
                            .values()
                            .filter(|alias| Path::new(alias).parent() == Some(root.as_path()))
                            .cloned(),
                    );
                    error.get_or_insert(CatalogError::InputUnavailable);
                }
            }
            _ => {
                retained_aliases.insert(alias.to_owned());
                retained_aliases.extend(
                    ownership
                        .values()
                        .filter(|alias| Path::new(alias).parent() == Some(root.as_path()))
                        .cloned(),
                );
                error.get_or_insert(CatalogError::InputUnavailable);
            }
        }
    }
    candidates.sort();
    candidates.dedup();
    if candidates.len() > 64 {
        return Ok((false, Some(CatalogError::LimitExceeded)));
    }
    for path in candidates {
        let Some(alias) = path.to_str().filter(|alias| valid_source_path(alias)) else {
            error.get_or_insert(CatalogError::InvalidInput);
            continue;
        };
        retained_aliases.insert(alias.to_owned());
        // Independently imported sources are never rewritten or removed by sync.
        if let Ok(source) = fs::canonicalize(&path)
            && store.clusters.iter().any(|record| {
                (Path::new(&record.source_path) == source || record.provenance_path == alias)
                    && !ownership.contains_key(&record.id)
            })
        {
            continue;
        }
        match reconcile(store, &path) {
            Ok(_) => {
                ownership.retain(|_, owner| owner != alias);
                for record in &store.clusters {
                    if record.provenance_path == alias {
                        ownership.insert(record.id.clone(), alias.to_owned());
                    }
                }
            }
            Err(problem) => {
                error.get_or_insert(problem);
            }
        }
    }
    store.clusters.retain(|record| {
        ownership
            .get(&record.id)
            .is_none_or(|alias| retained_aliases.contains(alias))
    });
    ownership.retain(|id, _| store.clusters.iter().any(|record| &record.id == id));
    transaction
        .execute("DELETE FROM kubeconfig_sync_ownership", [])
        .map_err(storage::sql_error)?;
    for (id, alias) in ownership {
        transaction
            .execute(
                "INSERT INTO kubeconfig_sync_ownership (id, alias) VALUES (?1, ?2)",
                rusqlite::params![id, alias],
            )
            .map_err(storage::sql_error)?;
    }
    Ok((
        before
            != serde_json::to_vec(&store.clusters).map_err(|_| CatalogError::StoreUnavailable)?,
        error,
    ))
}

fn discover_default(store: &mut Store, input: &Path) -> Result<bool> {
    match fs::symlink_metadata(input) {
        Ok(_) => reconcile(store, input),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if store
                .clusters
                .iter()
                .any(|c| Path::new(&c.provenance_path) == input)
            {
                return Err(CatalogError::InputUnavailable);
            }
            // A missing default is normal; a broken directory alias or denied
            // ancestor is not. Inspect only ancestors of this one default path.
            for ancestor in input.ancestors().skip(1) {
                match fs::symlink_metadata(ancestor) {
                    Ok(_) => {
                        return match fs::metadata(ancestor) {
                            Ok(metadata) if metadata.is_dir() => Ok(false),
                            _ => Err(CatalogError::InputUnavailable),
                        };
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(_) => return Err(CatalogError::InputUnavailable),
                }
            }
            Ok(false)
        }
        Err(_) => Err(CatalogError::InputUnavailable),
    }
}
fn reconcile(store: &mut Store, input: &Path) -> Result<bool> {
    // Resolve first, then open only the canonical target. Storage reads deliberately
    // keep using bounded_read without resolution so storage symlinks stay rejected.
    let alias = input
        .to_str()
        .filter(|path| valid_source_path(path))
        .ok_or(CatalogError::InvalidInput)?;
    let path = fs::canonicalize(input).map_err(|_| CatalogError::InputUnavailable)?;
    let source = path
        .to_str()
        .filter(|p| p.len() <= MAX_PATH)
        .ok_or(CatalogError::InvalidInput)?
        .to_owned();
    if !path
        .file_name()
        .and_then(|s| s.to_str())
        .is_some_and(valid_name)
    {
        return Err(CatalogError::InvalidInput);
    }
    let bytes = bounded_read(&path, MAX_INPUT)?;
    let contexts = parse_metadata(&bytes)?;
    // Stage reconciliation so a rejected discovery never removes the last valid rows.
    let mut staged = store.clone();
    let mut next = Vec::new();
    for context in contexts {
        let id = cluster_id(&source, &context);
        let previous = store.clusters.iter().find(|c| c.id == id);
        let color = match previous {
            Some(c) => c.color.clone(),
            None => new_color(&staged)?,
        };
        let record = ClusterRecord {
            id,
            context_name: context,
            source_path: source.clone(),
            provenance_path: alias.to_owned(),
            pinned: previous.is_some_and(|c| c.pinned),
            pinned_sections: previous
                .map(|c| c.pinned_sections.clone())
                .unwrap_or_default(),
            color,
        };
        staged.clusters.push(record.clone());
        next.push(record);
    }
    // One provenance alias per canonical source: the last successful import wins.
    // Retargeting removes rows still owned by that alias, not independently selected
    // sources. A direct reimport can therefore deliberately retain the old target.
    staged
        .clusters
        .retain(|c| c.source_path != source && c.provenance_path != alias);
    staged.clusters.extend(next);
    staged.clusters.sort_by(|a, b| a.id.cmp(&b.id));
    storage::validate_size(&staged)?;
    let changed = serde_json::to_vec(&store.clusters)
        .map_err(|_| CatalogError::StoreUnavailable)?
        != serde_json::to_vec(&staged.clusters).map_err(|_| CatalogError::StoreUnavailable)?;
    *store = staged;
    Ok(changed)
}
fn valid_source_path(source: &str) -> bool {
    let path = Path::new(source);
    source.len() <= MAX_PATH
        && !source.chars().any(char::is_control)
        && path.is_absolute()
        && path
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(valid_name)
}
fn valid_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_NAME && s.trim() == s && !s.chars().any(char::is_control)
}
fn cluster_id(source: &str, context: &str) -> String {
    digest_id("cluster", &[source, context])
}
fn digest_id(kind: &str, parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part.as_bytes());
    }
    format!("{kind}-{:x}", hash.finalize())
}
pub(crate) fn bounded_read(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).map_err(|_| CatalogError::InputUnavailable)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(CatalogError::InputUnavailable);
    }
    if metadata.len() > limit as u64 {
        return Err(CatalogError::LimitExceeded);
    }
    let file = File::open(path).map_err(|_| CatalogError::InputUnavailable)?;
    let opened = file
        .metadata()
        .map_err(|_| CatalogError::InputUnavailable)?;
    if !opened.is_file() || !same_file(&metadata, &opened) {
        return Err(CatalogError::InputUnavailable);
    }
    if opened.len() > limit as u64 {
        return Err(CatalogError::LimitExceeded);
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CatalogError::InputUnavailable)?;
    if bytes.len() > limit {
        return Err(CatalogError::LimitExceeded);
    }
    Ok(bytes)
}
// Best-effort replacement detection, not a hostile-filesystem race guarantee.
fn same_file(before: &fs::Metadata, opened: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        before.dev() == opened.dev() && before.ino() == opened.ino()
    }
    #[cfg(not(unix))]
    {
        before.len() == opened.len() && before.modified().ok() == opened.modified().ok()
    }
}
#[derive(Deserialize)]
struct KubeMetadata {
    #[serde(rename = "apiVersion")]
    api_version: String,
    kind: String,
    clusters: Vec<NamedCluster>,
    contexts: Vec<NamedContext>,
}
#[derive(Deserialize)]
struct NamedCluster {
    name: String,
}
#[derive(Deserialize)]
struct NamedContext {
    name: String,
    context: ContextMetadata,
}
#[derive(Deserialize)]
struct ContextMetadata {
    cluster: String,
}
fn parse_metadata(bytes: &[u8]) -> Result<Vec<String>> {
    // Unknown fields (including users, exec, and certificates) are skipped by Serde.
    let config: KubeMetadata =
        serde_yaml_ng::from_slice(bytes).map_err(|_| CatalogError::InvalidKubeconfig)?;
    if config.api_version != "v1" || config.kind != "Config" {
        return Err(CatalogError::InvalidKubeconfig);
    }
    if config.clusters.len() > MAX_ENTRIES || config.contexts.len() > MAX_ENTRIES {
        return Err(CatalogError::LimitExceeded);
    }
    let mut clusters = HashSet::new();
    for cluster in config.clusters {
        if !valid_name(&cluster.name) || !clusters.insert(cluster.name) {
            return Err(CatalogError::InvalidKubeconfig);
        }
    }
    let mut names = HashSet::new();
    for context in config.contexts {
        if !valid_name(&context.name)
            || !clusters.contains(&context.context.cluster)
            || !names.insert(context.name)
        {
            return Err(CatalogError::InvalidKubeconfig);
        }
    }
    let mut names: Vec<_> = names.into_iter().collect();
    names.sort();
    Ok(names)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod discovery_tests;

#[cfg(test)]
mod symlink_tests;

#[cfg(test)]
mod section_pin_tests;

#[cfg(test)]
mod pin_order_tests;

#[cfg(test)]
mod sync_tests;
