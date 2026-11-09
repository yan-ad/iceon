//! Normalized SQLite tables; DELETE journaling keeps sidecars short-lived. A private
//! lock file serializes initialization/migration across engine instances/processes;
//! BEGIN IMMEDIATE covers each read/modify/write. OS ACLs are inherited on Windows.
//! The legacy JSON and a non-overwriting .backup are retained, never used once SQLite exists.
use super::*;
use fs2::FileExt;
use rusqlite::{Connection, OpenFlags, params};
use std::{
    fs::OpenOptions,
    io::Write,
    time::{Duration, Instant},
};

const DATABASE: &str = "catalog.sqlite3";
const MAX_DATABASE: u64 = 16 * 1024 * 1024;
const SCHEMA: &str = "
CREATE TABLE metadata (singleton INTEGER PRIMARY KEY CHECK(singleton = 1), schema_version INTEGER NOT NULL CHECK(schema_version = 5), revision TEXT NOT NULL, auto_discover INTEGER NOT NULL CHECK(auto_discover IN (0,1))) STRICT;
CREATE TABLE clusters (id TEXT PRIMARY KEY NOT NULL, context_name TEXT NOT NULL, source_path TEXT NOT NULL, source_alias TEXT NOT NULL, pinned INTEGER NOT NULL CHECK(pinned IN (0,1)), color TEXT NOT NULL, pinned_sections TEXT NOT NULL, UNIQUE(source_path, context_name)) STRICT;
CREATE TABLE pin_order (position INTEGER PRIMARY KEY CHECK(position >= 0), id TEXT NOT NULL, section TEXT) STRICT;
CREATE TABLE IF NOT EXISTS namespace_snapshots (identity TEXT PRIMARY KEY NOT NULL CHECK(length(identity)=64), names TEXT NOT NULL CHECK(length(names)<=70000), fetched_at INTEGER NOT NULL, expires_at INTEGER NOT NULL, partial INTEGER NOT NULL CHECK(partial IN (0,1))) STRICT;
";

pub(super) fn sql_error(error: rusqlite::Error) -> CatalogError {
    match error {
        rusqlite::Error::SqliteFailure(ref failure, _)
            if matches!(
                failure.code,
                rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase
            ) =>
        {
            CatalogError::StoreCorrupt
        }
        _ => CatalogError::StoreUnavailable,
    }
}
pub(super) fn ensure_namespace_cache(connection: &Connection) -> Result<()> {
    connection.execute_batch("CREATE TABLE IF NOT EXISTS namespace_snapshots (identity TEXT PRIMARY KEY NOT NULL CHECK(length(identity)=64), names TEXT NOT NULL CHECK(length(names)<=70000), fetched_at INTEGER NOT NULL, expires_at INTEGER NOT NULL, partial INTEGER NOT NULL CHECK(partial IN (0,1))) STRICT;").map_err(sql_error)
}
fn private(path: &Path, directory: bool) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|_| CatalogError::StoreUnavailable)?;
    if metadata.file_type().is_symlink()
        || (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
    {
        return Err(CatalogError::StoreCorrupt);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            path,
            fs::Permissions::from_mode(if directory { 0o700 } else { 0o600 }),
        )
        .map_err(|_| CatalogError::StoreUnavailable)?;
    }
    Ok(())
}
pub(super) fn lock(directory: &Path) -> Result<File> {
    if let Ok(metadata) = fs::symlink_metadata(directory)
        && (!metadata.is_dir() || metadata.file_type().is_symlink())
    {
        return Err(CatalogError::StoreUnavailable);
    }
    fs::create_dir_all(directory).map_err(|_| CatalogError::StoreUnavailable)?;
    private(directory, true)?;
    let path = directory.join("catalog.lock");
    match fs::symlink_metadata(&path) {
        Ok(_) => private(&path, false)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(CatalogError::StoreUnavailable),
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(&path)
        .map_err(|_| CatalogError::StoreUnavailable)?;
    private(&path, false)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match FileExt::try_lock_exclusive(&file) {
            Ok(()) => break,
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => return Err(CatalogError::StoreUnavailable),
        }
    }
    Ok(file)
}
fn connect(path: &Path) -> Result<Connection> {
    connect_with_access(path, false)
}
fn connect_with_access(path: &Path, read_only: bool) -> Result<Connection> {
    let path = fs::canonicalize(path).map_err(|_| CatalogError::StoreUnavailable)?;
    let connection = Connection::open_with_flags(
        &path,
        (if read_only {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        } else {
            OpenFlags::SQLITE_OPEN_READ_WRITE
        }) | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(sql_error)?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(sql_error)?;
    connection
        .set_limit(
            rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH,
            MAX_STORE as i32,
        )
        .map_err(sql_error)?;
    connection
        .execute_batch(if read_only {
            "PRAGMA trusted_schema=OFF; PRAGMA query_only=ON;"
        } else {
            "PRAGMA trusted_schema=OFF; PRAGMA synchronous=FULL; PRAGMA max_page_count=4096;"
        })
        .map_err(sql_error)?;
    Ok(connection)
}
// Saved connection and pin lookups never initialize, migrate, discover, or rewrite storage.
pub(super) fn read_existing(directory: &Path) -> Result<Store> {
    let path = directory.join(DATABASE);
    for (candidate, is_directory) in [(directory.to_path_buf(), true), (path.clone(), false)] {
        let metadata = fs::symlink_metadata(candidate).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                CatalogError::NotFound
            } else {
                CatalogError::StoreUnavailable
            }
        })?;
        if metadata.file_type().is_symlink()
            || (is_directory && !metadata.is_dir())
            || (!is_directory
                && (!metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_DATABASE))
        {
            return Err(CatalogError::StoreCorrupt);
        }
    }
    for name in [
        "catalog.lock",
        "catalog.sqlite3-journal",
        "catalog.sqlite3-wal",
        "catalog.sqlite3-shm",
    ] {
        match fs::symlink_metadata(directory.join(name)) {
            Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
                return Err(CatalogError::StoreCorrupt);
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(CatalogError::StoreUnavailable),
        }
    }
    let mut connection = connect_with_access(&path, true)?;
    let transaction = connection.transaction().map_err(sql_error)?;
    let check: String = transaction
        .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
        .map_err(|_| CatalogError::StoreCorrupt)?;
    let journal: String = transaction
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .map_err(sql_error)?;
    if check != "ok" || journal != "delete" {
        return Err(CatalogError::StoreCorrupt);
    }
    load(&transaction)
}
pub(super) fn open(directory: &Path) -> Result<Connection> {
    let path = directory.join(DATABASE);
    let exists = match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if metadata.len() == 0 || metadata.len() > MAX_DATABASE {
                return Err(CatalogError::StoreCorrupt);
            }
            private(&path, false)?;
            true
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => return Err(CatalogError::StoreUnavailable),
    };
    for suffix in ["-journal", "-wal", "-shm"] {
        let sidecar = directory.join(format!("{DATABASE}{suffix}"));
        match fs::symlink_metadata(&sidecar) {
            Ok(_) => private(&sidecar, false)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(CatalogError::StoreUnavailable),
        }
    }
    if !exists {
        let legacy = directory.join("catalog.json");
        let store = match fs::symlink_metadata(&legacy) {
            Ok(_) => {
                let bytes =
                    bounded_read(&legacy, MAX_STORE).map_err(|_| CatalogError::StoreCorrupt)?;
                let store = migrate(&bytes)?;
                backup(directory, &bytes)?;
                private(&legacy, false)?;
                store
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Store::empty(),
            Err(_) => return Err(CatalogError::StoreUnavailable),
        };
        let temp = tempfile::NamedTempFile::new_in(directory)
            .map_err(|_| CatalogError::StoreUnavailable)?;
        let mut connection = connect(temp.path())?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        transaction.execute_batch(SCHEMA).map_err(sql_error)?;
        save(&transaction, &store)?;
        transaction.commit().map_err(sql_error)?;
        drop(connection);
        temp.as_file()
            .sync_all()
            .map_err(|_| CatalogError::StoreUnavailable)?;
        temp.persist_noclobber(&path)
            .map_err(|_| CatalogError::StoreUnavailable)?;
        #[cfg(unix)]
        File::open(directory)
            .and_then(|f| f.sync_all())
            .map_err(|_| CatalogError::StoreUnavailable)?;
    }
    let mut connection = connect(&path)?;
    let check: String = connection
        .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
        .map_err(|_| CatalogError::StoreCorrupt)?;
    if check != "ok" {
        return Err(CatalogError::StoreCorrupt);
    }
    let journal: String = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .map_err(sql_error)?;
    if journal != "delete" {
        return Err(CatalogError::StoreCorrupt);
    }
    upgrade(&mut connection)?;
    ensure_namespace_cache(&connection)?;
    load(&connection)?;
    Ok(connection)
}
fn upgrade(connection: &mut Connection) -> Result<()> {
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(sql_error)?;
    let version: u32 = transaction
        .query_one(
            "SELECT schema_version FROM metadata WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .map_err(|_| CatalogError::StoreCorrupt)?;
    match version {
        1..=4 => {
            // Retired link rows are deliberately not decoded or validated.
            let store = load_version(&transaction, version)?;
            if version == 1 {
                transaction
                    .execute_batch(
                        "
                    ALTER TABLE clusters ADD COLUMN source_alias TEXT NOT NULL DEFAULT '';
                    UPDATE clusters SET source_alias=source_path;
                ",
                    )
                    .map_err(|_| CatalogError::StoreCorrupt)?;
            }
            if version < 3 {
                transaction.execute_batch("ALTER TABLE clusters ADD COLUMN pinned_sections TEXT NOT NULL DEFAULT '[]';")
                    .map_err(|_| CatalogError::StoreCorrupt)?;
            }
            transaction.execute_batch("
                ALTER TABLE metadata RENAME TO metadata_old;
                CREATE TABLE metadata (singleton INTEGER PRIMARY KEY CHECK(singleton = 1), schema_version INTEGER NOT NULL CHECK(schema_version = 5), revision TEXT NOT NULL, auto_discover INTEGER NOT NULL CHECK(auto_discover IN (0,1))) STRICT;
                DROP TABLE metadata_old;
                DROP TABLE IF EXISTS web_links;
            ").map_err(|_| CatalogError::StoreCorrupt)?;
            if version < 4 {
                transaction.execute_batch("CREATE TABLE pin_order (position INTEGER PRIMARY KEY CHECK(position >= 0), id TEXT NOT NULL, section TEXT) STRICT;")
                    .map_err(|_| CatalogError::StoreCorrupt)?;
            }
            save(&transaction, &store)?;
            if serde_json::to_vec(&load(&transaction)?).map_err(|_| CatalogError::StoreCorrupt)?
                != serde_json::to_vec(&store).map_err(|_| CatalogError::StoreCorrupt)?
            {
                return Err(CatalogError::StoreCorrupt);
            }
        }
        5 => {}
        _ => return Err(CatalogError::StoreCorrupt),
    }
    transaction.commit().map_err(sql_error)
}
pub(super) fn load(connection: &Connection) -> Result<Store> {
    load_version(connection, 5)
}
fn load_version(connection: &Connection, expected_version: u32) -> Result<Store> {
    let (schema_version, revision, auto_discover): (u32, String, i64) = connection
        .query_one(
            "SELECT schema_version, revision, auto_discover FROM metadata WHERE singleton=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| CatalogError::StoreCorrupt)?;
    if schema_version != expected_version || !matches!(auto_discover, 0 | 1) {
        return Err(CatalogError::StoreCorrupt);
    }
    let mut store = Store {
        schema_version: 5,
        revision: revision.parse().map_err(|_| CatalogError::StoreCorrupt)?,
        auto_discover: auto_discover == 1,
        clusters: vec![],
        pin_order: vec![],
    };
    let query = match expected_version {
        1 => {
            "SELECT id, context_name, source_path, pinned, color, source_path, '[]' FROM clusters ORDER BY id LIMIT 1001"
        }
        2 => {
            "SELECT id, context_name, source_path, pinned, color, source_alias, '[]' FROM clusters ORDER BY id LIMIT 1001"
        }
        _ => {
            "SELECT id, context_name, source_path, pinned, color, source_alias, pinned_sections FROM clusters ORDER BY id LIMIT 1001"
        }
    };
    let mut statement = connection
        .prepare(query)
        .map_err(|_| CatalogError::StoreCorrupt)?;
    let rows = statement
        .query_map([], |r| {
            Ok(ClusterRecord {
                id: r.get(0)?,
                context_name: r.get(1)?,
                source_path: r.get(2)?,
                provenance_path: r.get(5)?,
                pinned: r.get(3)?,
                pinned_sections: serde_json::from_str(&r.get::<_, String>(6)?)
                    .map_err(|_| rusqlite::Error::InvalidQuery)?,
                color: r.get(4)?,
            })
        })
        .map_err(|_| CatalogError::StoreCorrupt)?;
    for row in rows {
        store
            .clusters
            .push(row.map_err(|_| CatalogError::StoreCorrupt)?);
    }
    if expected_version >= 4 {
        let mut statement = connection
            .prepare(if expected_version == 4 {
                "SELECT position, id, section FROM pin_order WHERE id IN (SELECT id FROM clusters) ORDER BY position LIMIT ?1"
            } else {
                "SELECT position, id, section FROM pin_order ORDER BY position LIMIT ?1"
            })
            .map_err(|_| CatalogError::StoreCorrupt)?;
        let rows = statement
            .query_map([(MAX_ENTRIES * SECTIONS.len() + 1) as u32], |row| {
                Ok((
                    row.get::<_, u32>(0)?,
                    PinTarget {
                        id: row.get(1)?,
                        section: row.get(2)?,
                    },
                ))
            })
            .map_err(|_| CatalogError::StoreCorrupt)?;
        for (position, row) in rows.enumerate() {
            let (stored_position, target) = row.map_err(|_| CatalogError::StoreCorrupt)?;
            if expected_version == 5 && stored_position as usize != position {
                return Err(CatalogError::StoreCorrupt);
            }
            store.pin_order.push(target);
        }
    }
    store.validate()?;
    validate_size(&store).map_err(|_| CatalogError::StoreCorrupt)?;
    // Older section pins treated overview as a second route to the same cluster.
    for cluster in &mut store.clusters {
        if cluster
            .pinned_sections
            .iter()
            .any(|section| section == "overview")
        {
            cluster.pinned = true;
            cluster
                .pinned_sections
                .retain(|section| section != "overview");
        }
    }
    store.reconcile_pins();
    validate_size(&store).map_err(|_| CatalogError::StoreCorrupt)?;
    Ok(store)
}
pub(super) fn validate_size(store: &Store) -> Result<()> {
    if store.clusters.len() > MAX_ENTRIES
        || serde_json::to_vec(store)
            .map_err(|_| CatalogError::StoreUnavailable)?
            .len()
            > MAX_STORE
    {
        return Err(CatalogError::LimitExceeded);
    }
    Ok(())
}
pub(super) fn save(connection: &rusqlite::Transaction<'_>, store: &Store) -> Result<()> {
    validate_size(store)?;
    store.validate()?;
    connection
        .execute_batch("DELETE FROM clusters; DELETE FROM pin_order; DELETE FROM metadata;")
        .map_err(sql_error)?;
    connection
        .execute(
            "INSERT INTO metadata VALUES (1, ?1, ?2, ?3)",
            params![
                store.schema_version,
                store.revision.to_string(),
                store.auto_discover
            ],
        )
        .map_err(sql_error)?;
    for c in &store.clusters {
        connection
            .execute(
                "INSERT INTO clusters (id, context_name, source_path, pinned, color, source_alias, pinned_sections) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![c.id, c.context_name, c.source_path, c.pinned, c.color, c.provenance_path,
                    serde_json::to_string(&c.pinned_sections).map_err(|_| CatalogError::StoreUnavailable)?],
            )
            .map_err(sql_error)?;
    }
    for (position, target) in store.pin_order.iter().enumerate() {
        connection
            .execute(
                "INSERT INTO pin_order VALUES (?1, ?2, ?3)",
                params![position as u32, target.id, target.section],
            )
            .map_err(sql_error)?;
    }
    Ok(())
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LegacyStore {
    schema_version: u32,
    revision: u64,
    clusters: Vec<LegacyCluster>,
    #[serde(default, rename = "webLinks")]
    _retired_links: serde::de::IgnoredAny,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LegacyCluster {
    id: String,
    context_name: String,
    source_path: String,
    pinned: bool,
}
fn migrate(bytes: &[u8]) -> Result<Store> {
    let legacy: LegacyStore =
        serde_json::from_slice(bytes).map_err(|_| CatalogError::StoreCorrupt)?;
    let mut store = Store {
        revision: legacy.revision,
        ..Store::empty()
    };
    if legacy.schema_version != 1 || legacy.clusters.len() > MAX_ENTRIES {
        return Err(CatalogError::StoreCorrupt);
    }
    for c in legacy.clusters {
        let color = new_color(&store)?;
        store.clusters.push(ClusterRecord {
            id: c.id,
            context_name: c.context_name,
            provenance_path: c.source_path.clone(),
            source_path: c.source_path,
            pinned: c.pinned,
            pinned_sections: vec![],
            color,
        });
    }
    store.reconcile_pins();
    store.validate()?;
    validate_size(&store).map_err(|_| CatalogError::StoreCorrupt)?;
    Ok(store)
}
fn backup(directory: &Path, bytes: &[u8]) -> Result<()> {
    let path = directory.join("catalog.json.backup");
    match fs::symlink_metadata(&path) {
        Ok(_) => {
            if bounded_read(&path, MAX_STORE).map_err(|_| CatalogError::StoreCorrupt)? != bytes {
                return Err(CatalogError::StoreCorrupt);
            }
            private(&path, false)?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut temp = tempfile::NamedTempFile::new_in(directory)
                .map_err(|_| CatalogError::StoreUnavailable)?;
            temp.write_all(bytes)
                .and_then(|_| temp.as_file().sync_all())
                .map_err(|_| CatalogError::StoreUnavailable)?;
            temp.persist_noclobber(path)
                .map_err(|_| CatalogError::StoreUnavailable)?;
        }
        Err(_) => return Err(CatalogError::StoreUnavailable),
    }
    Ok(())
}
