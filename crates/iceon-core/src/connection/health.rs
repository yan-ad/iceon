//! Ephemeral, read-only reachability for at most 32 previously connected clusters.
//! This is not a Session: no watches, mutations, config construction or auth helper renewal.
use super::*;
use futures_util::{StreamExt, stream};
use sha2::{Digest, Sha256};

const MAX_CLUSTERS: usize = 32;
const HEALTH_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ClusterHealthSnapshot {
    pub cluster_id: String,
    pub status: &'static str,
    pub api_version: Option<String>,
    pub error: Option<ConnectionError>,
}

#[derive(Clone)]
struct Entry {
    target: ConnectionTarget,
    source: [u8; 32],
    client: Client,
    cancel: CancellationToken,
    generation: u64,
    snapshot: ClusterHealthSnapshot,
}

pub(super) struct HealthRuntime {
    entries: Mutex<HashMap<String, Entry>>,
    generation: AtomicU64,
    sweep: tokio::sync::Semaphore,
}
impl Default for HealthRuntime {
    fn default() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            generation: AtomicU64::new(0),
            sweep: tokio::sync::Semaphore::new(1),
        }
    }
}
impl HealthRuntime {
    pub(super) fn clear(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            for (_, entry) in entries.drain() {
                entry.cancel.cancel();
            }
        }
    }
    pub(super) fn register(
        &self,
        id: String,
        target: ConnectionTarget,
        source: [u8; 32],
        client: Client,
        version: String,
    ) -> Result<()> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        if let Some(old) = entries.remove(&id) {
            old.cancel.cancel();
        }
        if entries.len() >= MAX_CLUSTERS {
            if let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, e)| e.generation)
                .map(|(id, _)| id.clone())
            {
                if let Some(old) = entries.remove(&oldest) {
                    old.cancel.cancel();
                }
            }
        }
        entries.insert(
            id.clone(),
            Entry {
                target,
                source,
                client,
                cancel: CancellationToken::new(),
                generation: self.generation.fetch_add(1, Ordering::SeqCst),
                snapshot: ClusterHealthSnapshot {
                    cluster_id: id,
                    status: "connected",
                    api_version: Some(version),
                    error: None,
                },
            },
        );
        Ok(())
    }
}
impl Drop for HealthRuntime {
    fn drop(&mut self) {
        self.clear();
    }
}

// Hash the bounded source bytes and target coordinates, not parsed credentials. Reading
// source files does not run exec auth. No source bytes or hashes are persisted or exposed.
fn source_identity_sync(target: &ConnectionTarget) -> Result<[u8; 32]> {
    if std::fs::canonicalize(&target.provenance_path)
        .map_err(|_| ConnectionError::KubeconfigUnavailable)?
        != target.source_path
    {
        return Err(ConnectionError::SourceChanged);
    }
    let bytes = crate::catalog::bounded_read(&target.source_path, 1024 * 1024)
        .map_err(map_catalog_error)?;
    let mut hash = Sha256::new();
    for part in [
        target.source_path.to_string_lossy().as_bytes(),
        target.provenance_path.to_string_lossy().as_bytes(),
        target.context_name.as_bytes(),
        &bytes,
    ] {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part);
    }
    Ok(hash.finalize().into())
}
pub(super) async fn source_identity(target: ConnectionTarget) -> Result<[u8; 32]> {
    crate::runtime::spawn_blocking(move || source_identity_sync(&target))
        .await
        .map_err(|_| ConnectionError::StoreUnavailable)?
}

impl ConnectionEngine {
    /// Explicit catalog removal/disconnect hook. Cancels in-flight probes and drops credentials.
    /// Does not release a workspace lease; use `disconnect` for the active workspace.
    pub fn clear_cluster_health(&self, cluster_id: &str) -> Result<()> {
        validate_id(cluster_id)?;
        let mut entries = self
            .health
            .entries
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        if let Some(entry) = entries.remove(cluster_id) {
            entry.cancel.cancel();
        }
        Ok(())
    }

    fn health_source_matches(&self, id: &str, entry: &Entry) -> bool {
        self.catalog
            .connection_target(id)
            .is_ok_and(|target| target == entry.target)
            && source_identity_sync(&entry.target).is_ok_and(|source| source == entry.source)
    }

    /// Metadata-only reachability/version projection, independent of the active lease.
    /// Removed/reimported/changed sources are pruned even before the next refresh.
    pub fn cluster_health_snapshot(&self) -> Result<Vec<ClusterHealthSnapshot>> {
        let mut entries = self
            .health
            .entries
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?;
        entries.retain(|id, entry| {
            let valid = self.health_source_matches(id, entry);
            if !valid {
                entry.cancel.cancel();
            }
            valid
        });
        let mut snapshots: Vec<_> = entries
            .values()
            .map(|entry| entry.snapshot.clone())
            .collect();
        snapshots.sort_by(|a, b| a.cluster_id.cmp(&b.cluster_id));
        Ok(snapshots)
    }

    /// One bounded sweep; call approximately every 30 seconds from the frontend runtime.
    /// At most two `/version` requests run at once, separately from workspace workers.
    /// Overlapping sweeps return Busy rather than enqueueing unbounded work.
    pub async fn refresh_cluster_health(&self) -> Result<Vec<ClusterHealthSnapshot>> {
        let _sweep = self
            .health
            .sweep
            .try_acquire()
            .map_err(|_| ConnectionError::Busy)?;
        self.cluster_health_snapshot()?;
        let entries: Vec<_> = self
            .health
            .entries
            .lock()
            .map_err(|_| ConnectionError::StoreUnavailable)?
            .iter()
            .map(|(id, entry)| (id.clone(), entry.clone()))
            .collect();
        stream::iter(entries)
            .for_each_concurrent(2, |(id, entry)| async move {
                let result = tokio::select! {
                    biased;
                    _ = entry.cancel.cancelled() => return,
                    result = tokio::time::timeout(HEALTH_TIMEOUT, async {
                        validate_credentials(&entry.client)?;
                        let version = transport::version(&entry.client, &entry.cancel).await?;
                        validate_credentials(&entry.client)?;
                        Ok(version)
                    }) => result.unwrap_or(Err(ConnectionError::Timeout)),
                };
                // Source and generation are checked again after the response. Reconnect, eviction,
                // explicit clear and shutdown cannot resurrect a stale entry.
                if !self.health_source_matches(&id, &entry) {
                    if let Ok(mut entries) = self.health.entries.lock() {
                        if entries
                            .get(&id)
                            .is_some_and(|e| e.generation == entry.generation)
                        {
                            if let Some(old) = entries.remove(&id) {
                                old.cancel.cancel();
                            }
                        }
                    }
                    return;
                }
                if let Ok(mut entries) = self.health.entries.lock() {
                    if let Some(current) = entries
                        .get_mut(&id)
                        .filter(|e| e.generation == entry.generation && !e.cancel.is_cancelled())
                    {
                        match result {
                            Ok(version) => {
                                current.snapshot.status = "connected";
                                current.snapshot.api_version = Some(version);
                                current.snapshot.error = None;
                            }
                            Err(error) => {
                                current.snapshot.status = "error";
                                current.snapshot.api_version = None;
                                current.snapshot.error = Some(error);
                            }
                        }
                    }
                }
            })
            .await;
        self.cluster_health_snapshot()
    }
}

#[cfg(test)]
mod tests;
