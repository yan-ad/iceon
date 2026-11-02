use super::{Error, Profile, Result, validate_profile};
use fs2::FileExt;
use rusqlite::{Connection, OpenFlags, TransactionBehavior, params};
use std::{
    fs::{self, File, OpenOptions},
    io::ErrorKind,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const DATABASE: &str = "secrets-profiles.sqlite3";
const LOCK: &str = "secrets-profiles.lock";
const MAX_DATABASE: u64 = 2 * 1024 * 1024;
const MAX_PAYLOAD: usize = 16 * 1024;
const MAX_PROFILES: usize = 100;
const METADATA_SCHEMA: &str = "CREATE TABLE metadata (singleton INTEGER PRIMARY KEY CHECK(singleton = 1), schema_version INTEGER NOT NULL CHECK(schema_version = 1)) STRICT";
const PROFILES_SCHEMA: &str = "CREATE TABLE profiles (id TEXT PRIMARY KEY NOT NULL, payload TEXT NOT NULL CHECK(length(CAST(payload AS BLOB)) <= 16384)) STRICT";

pub(super) struct Store {
    directory: PathBuf,
}

impl Store {
    pub(super) fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    pub(super) fn list(&self) -> Result<Vec<Profile>> {
        self.access(false, |_, profiles| Ok(profiles))
    }

    pub(super) fn insert(&self, profile: &Profile) -> Result<()> {
        validate_profile(profile)?;
        validate_id(&profile.id)?;
        let payload = serde_json::to_string(profile).map_err(|_| Error::InvalidInput)?;
        if payload.len() > MAX_PAYLOAD {
            return Err(Error::TooLarge);
        }
        self.access(true, |transaction, profiles| {
            if profiles.iter().any(|saved| saved.id == profile.id) {
                return Err(Error::InvalidInput);
            }
            if profiles.len() >= MAX_PROFILES {
                return Err(Error::TooLarge);
            }
            transaction
                .execute(
                    "INSERT INTO profiles (id, payload) VALUES (?1, ?2)",
                    params![profile.id, payload],
                )
                .map_err(sql_error)?;
            Ok(())
        })
    }

    pub(super) fn remove(&self, id: &str) -> Result<()> {
        validate_id(id)?;
        self.access(false, |transaction, profiles| {
            if !profiles.iter().any(|profile| profile.id == id) {
                return Err(Error::NotFound);
            }
            transaction
                .execute("DELETE FROM profiles WHERE id = ?1", [id])
                .map_err(sql_error)?;
            Ok(())
        })
    }

    // The lock spans connection teardown, including DELETE-journal cleanup.
    fn access<T>(
        &self,
        create: bool,
        action: impl FnOnce(&rusqlite::Transaction<'_>, Vec<Profile>) -> Result<T>,
    ) -> Result<T> {
        let _lock = lock(&self.directory)?;
        check_files(&self.directory)?;
        let path = self.directory.join(DATABASE);
        let exists = metadata(&path)?.is_some();
        if !exists && !create {
            // A transaction on memory lets missing-list/remove share their usual semantics,
            // without creating a database on disk.
            let mut connection = Connection::open_in_memory().map_err(sql_error)?;
            let transaction = connection.transaction().map_err(sql_error)?;
            return action(&transaction, Vec::new());
        }
        let database = open_file(&path)?;
        if exists
            && database
                .metadata()
                .map_err(|_| Error::StoreUnavailable)?
                .len()
                == 0
        {
            return Err(Error::StoreUnavailable);
        }
        let canonical = fs::canonicalize(&path).map_err(|_| Error::StoreUnavailable)?;
        let mut connection = Connection::open_with_flags(
            &canonical,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(sql_error)?;
        connection
            .busy_timeout(Duration::from_secs(2))
            .map_err(sql_error)?;
        for (limit, value) in [
            (rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH, 64 * 1024),
            (rusqlite::limits::Limit::SQLITE_LIMIT_SQL_LENGTH, 16 * 1024),
            (rusqlite::limits::Limit::SQLITE_LIMIT_COLUMN, 16),
            (rusqlite::limits::Limit::SQLITE_LIMIT_EXPR_DEPTH, 32),
            (rusqlite::limits::Limit::SQLITE_LIMIT_ATTACHED, 0),
            (rusqlite::limits::Limit::SQLITE_LIMIT_VARIABLE_NUMBER, 16),
            (rusqlite::limits::Limit::SQLITE_LIMIT_TRIGGER_DEPTH, 0),
        ] {
            connection.set_limit(limit, value).map_err(sql_error)?;
        }
        connection
            .execute_batch("PRAGMA trusted_schema=OFF;")
            .map_err(sql_error)?;
        let journal: String = connection
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .map_err(sql_error)?;
        if journal != "delete" {
            return Err(Error::StoreUnavailable);
        }
        check_files(&self.directory)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        if !exists {
            transaction
                .execute_batch(METADATA_SCHEMA)
                .map_err(sql_error)?;
            transaction
                .execute_batch(PROFILES_SCHEMA)
                .map_err(sql_error)?;
            transaction
                .execute_batch("INSERT INTO metadata VALUES (1, 1); PRAGMA user_version=1;")
                .map_err(sql_error)?;
        }
        let profiles = load(&transaction)?;
        let page_size: u32 = transaction
            .query_row("PRAGMA page_size", [], |row| row.get(0))
            .map_err(sql_error)?;
        if !(512..=65536).contains(&page_size) || !page_size.is_power_of_two() {
            return Err(Error::StoreUnavailable);
        }
        transaction
            .execute_batch(&format!(
                "PRAGMA max_page_count={};",
                MAX_DATABASE / u64::from(page_size)
            ))
            .map_err(sql_error)?;
        let result = action(&transaction, profiles)?;
        check_files(&self.directory)?;
        transaction.commit().map_err(sql_error)?;
        check_files(&self.directory)?;
        Ok(result)
    }
}

fn validate_id(id: &str) -> Result<()> {
    if id.len() != 32
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(Error::InvalidInput);
    }
    Ok(())
}

fn sql_error(_: rusqlite::Error) -> Error {
    Error::StoreUnavailable
}

fn metadata(path: &Path) -> Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(_) => Err(Error::StoreUnavailable),
    }
}

fn check_files(directory: &Path) -> Result<()> {
    let info = metadata(directory)?.ok_or(Error::StoreUnavailable)?;
    if info.file_type().is_symlink() || !info.is_dir() {
        return Err(Error::StoreUnavailable);
    }
    for name in [
        LOCK.to_owned(),
        DATABASE.to_owned(),
        format!("{DATABASE}-journal"),
        format!("{DATABASE}-wal"),
        format!("{DATABASE}-shm"),
    ] {
        if let Some(info) = metadata(&directory.join(name))?
            && (info.file_type().is_symlink() || !info.is_file() || info.len() > MAX_DATABASE)
        {
            return Err(Error::StoreUnavailable);
        }
    }
    Ok(())
}

fn open_file(path: &Path) -> Result<File> {
    let existing = metadata(path)?;
    if let Some(info) = &existing
        && (info.file_type().is_symlink() || !info.is_file())
    {
        return Err(Error::StoreUnavailable);
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    if existing.is_none() {
        options.create_new(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(|_| Error::StoreUnavailable)?;
    let opened = file.metadata().map_err(|_| Error::StoreUnavailable)?;
    let current = metadata(path)?.ok_or(Error::StoreUnavailable)?;
    if !opened.is_file() || current.file_type().is_symlink() || !current.is_file() {
        return Err(Error::StoreUnavailable);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if opened.dev() != current.dev() || opened.ino() != current.ino() {
            return Err(Error::StoreUnavailable);
        }
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| Error::StoreUnavailable)?;
    }
    // Windows files retain their parent directory's inherited ACL.
    Ok(file)
}

fn lock(directory: &Path) -> Result<File> {
    if let Some(info) = metadata(directory)?
        && (info.file_type().is_symlink() || !info.is_dir())
    {
        return Err(Error::StoreUnavailable);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder
            .create(directory)
            .map_err(|_| Error::StoreUnavailable)?;
    }
    #[cfg(not(unix))]
    fs::create_dir_all(directory).map_err(|_| Error::StoreUnavailable)?;
    check_files(directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
            .map_err(|_| Error::StoreUnavailable)?;
    }
    let file = open_file(&directory.join(LOCK))?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match FileExt::try_lock_exclusive(&file) {
            Ok(()) => return Ok(file),
            Err(error) if error.kind() == ErrorKind::WouldBlock && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return Err(Error::StoreUnavailable),
        }
    }
}

fn load(connection: &Connection) -> Result<Vec<Profile>> {
    let check: String = connection
        .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
        .map_err(sql_error)?;
    if check != "ok" {
        return Err(Error::StoreUnavailable);
    }
    let mut statement = connection
        .prepare("SELECT type, name, sql FROM sqlite_schema ORDER BY name")
        .map_err(sql_error)?;
    let schema = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(sql_error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(sql_error)?;
    let expected = vec![
        (
            "table".to_owned(),
            "metadata".to_owned(),
            Some(METADATA_SCHEMA.to_owned()),
        ),
        (
            "table".to_owned(),
            "profiles".to_owned(),
            Some(PROFILES_SCHEMA.to_owned()),
        ),
        (
            "index".to_owned(),
            "sqlite_autoindex_profiles_1".to_owned(),
            None,
        ),
    ];
    if schema != expected {
        return Err(Error::StoreUnavailable);
    }
    let versions: Vec<(i64, i64)> = connection
        .prepare("SELECT singleton, schema_version FROM metadata")
        .map_err(sql_error)?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(sql_error)?
        .collect::<std::result::Result<_, _>>()
        .map_err(sql_error)?;
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(sql_error)?;
    if versions != [(1, 1)] || version != 1 {
        return Err(Error::StoreUnavailable);
    }
    let mut statement = connection
        .prepare("SELECT id, payload FROM profiles ORDER BY id LIMIT 101")
        .map_err(sql_error)?;
    let mut rows = statement.query([]).map_err(sql_error)?;
    let mut profiles = Vec::new();
    while let Some(row) = rows.next().map_err(sql_error)? {
        if profiles.len() >= MAX_PROFILES {
            return Err(Error::StoreUnavailable);
        }
        let id: String = row.get(0).map_err(sql_error)?;
        let payload: String = row.get(1).map_err(sql_error)?;
        if payload.len() > MAX_PAYLOAD {
            return Err(Error::StoreUnavailable);
        }
        let value: serde_json::Value =
            serde_json::from_str(&payload).map_err(|_| Error::StoreUnavailable)?;
        let profile: Profile =
            serde_json::from_value(value.clone()).map_err(|_| Error::StoreUnavailable)?;
        // Reject unknown fields rather than silently accepting credential-bearing payloads.
        if serde_json::to_value(&profile).map_err(|_| Error::StoreUnavailable)? != value
            || profile.id != id
        {
            return Err(Error::StoreUnavailable);
        }
        validate_id(&id).map_err(|_| Error::StoreUnavailable)?;
        validate_profile(&profile).map_err(|_| Error::StoreUnavailable)?;
        profiles.push(profile);
    }
    Ok(profiles)
}

#[cfg(test)]
mod tests {
    use super::super::{Provider, new_id};
    use super::*;

    fn profile() -> Profile {
        Profile {
            id: new_id().unwrap(),
            provider: Provider::Vault,
            name: "Fixture".into(),
            address: "https://vault.example.test".into(),
            namespace: String::new(),
            auth_mount: "token".into(),
            kv_mount: "secret".into(),
            base_path: String::new(),
        }
    }

    #[test]
    fn persistence_duplicate_remove_and_lazy_list() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::new(temp.path().join("profiles"));
        assert!(store.list().unwrap().is_empty());
        assert!(!store.directory.join(DATABASE).exists());
        let profile = profile();
        store.insert(&profile).unwrap();
        assert!(matches!(store.insert(&profile), Err(Error::InvalidInput)));
        let other = Store::new(store.directory.clone());
        assert!(other.list().unwrap() == vec![profile.clone()]);
        other.remove(&profile.id).unwrap();
        assert!(store.list().unwrap().is_empty());
        assert!(matches!(store.remove(&profile.id), Err(Error::NotFound)));
    }

    #[test]
    fn separate_engines_obey_bounded_lock() {
        let temp = tempfile::tempdir().unwrap();
        let held = lock(temp.path()).unwrap();
        let store = Store::new(temp.path().to_owned());
        let start = Instant::now();
        assert!(matches!(store.list(), Err(Error::StoreUnavailable)));
        assert!(start.elapsed() >= Duration::from_secs(2));
        assert!(start.elapsed() < Duration::from_secs(5));
        drop(held);
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn corrupt_database_is_not_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(DATABASE);
        let bytes = b"not a SQLite database";
        fs::write(&path, bytes).unwrap();
        let store = Store::new(temp.path().to_owned());
        assert!(matches!(
            store.insert(&profile()),
            Err(Error::StoreUnavailable)
        ));
        assert_eq!(fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn capacity_and_metadata_only_payload() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::new(temp.path().to_owned());
        for _ in 0..MAX_PROFILES {
            store.insert(&profile()).unwrap();
        }
        assert!(matches!(store.insert(&profile()), Err(Error::TooLarge)));
        assert_eq!(store.list().unwrap().len(), MAX_PROFILES);
        let connection = Connection::open(temp.path().join(DATABASE)).unwrap();
        let payload: String = connection
            .query_row("SELECT payload FROM profiles LIMIT 1", [], |row| row.get(0))
            .unwrap();
        let object = serde_json::from_str::<serde_json::Value>(&payload).unwrap();
        assert_eq!(object.as_object().unwrap().len(), 8);
        for forbidden in ["token", "password", "credentials", "credential_key"] {
            assert!(object.get(forbidden).is_none());
        }
        assert!(fs::metadata(temp.path().join(DATABASE)).unwrap().len() <= MAX_DATABASE);
    }

    #[test]
    fn unexpected_schema_and_invalid_payload_fail_closed() {
        for mutation in [
            "PRAGMA user_version=2",
            "CREATE TABLE unexpected (value TEXT) STRICT",
            "UPDATE profiles SET payload = json_set(payload, '$.token', 'forbidden')",
            "UPDATE profiles SET id='00000000000000000000000000000000'",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let store = Store::new(temp.path().to_owned());
            let profile = profile();
            store.insert(&profile).unwrap();
            let path = temp.path().join(DATABASE);
            Connection::open(&path)
                .unwrap()
                .execute_batch(mutation)
                .unwrap();
            let before = fs::read(&path).unwrap();
            assert!(matches!(store.list(), Err(Error::StoreUnavailable)));
            assert!(matches!(
                store.remove(&profile.id),
                Err(Error::StoreUnavailable)
            ));
            assert!(matches!(
                store.insert(&self::profile()),
                Err(Error::StoreUnavailable)
            ));
            assert_eq!(fs::read(path).unwrap(), before);
        }
    }

    #[test]
    fn oversized_database_and_payload_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(DATABASE);
        File::create(&path)
            .unwrap()
            .set_len(MAX_DATABASE + 1)
            .unwrap();
        let store = Store::new(temp.path().to_owned());
        assert!(matches!(store.list(), Err(Error::StoreUnavailable)));
        assert_eq!(fs::metadata(path).unwrap().len(), MAX_DATABASE + 1);
        let mut profile = profile();
        profile.base_path = "a".repeat(MAX_PAYLOAD + 1);
        assert!(store.insert(&profile).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_including_broken_links_preserve_prior_store() {
        use std::os::unix::fs::symlink;
        for broken in [false, true] {
            for name in [
                DATABASE,
                LOCK,
                "secrets-profiles.sqlite3-journal",
                "secrets-profiles.sqlite3-wal",
                "secrets-profiles.sqlite3-shm",
                "directory",
            ] {
                let temp = tempfile::tempdir().unwrap();
                let directory = temp.path().join("store");
                let store = Store::new(directory.clone());
                let profile = profile();
                store.insert(&profile).unwrap();
                let database_before = fs::read(directory.join(DATABASE)).unwrap();
                let candidate = if name == "directory" {
                    directory.clone()
                } else {
                    directory.join(name)
                };
                let original = temp.path().join("original");
                if candidate.exists() {
                    fs::rename(&candidate, &original).unwrap();
                }
                let target = if broken {
                    temp.path().join("missing")
                } else if original.exists() {
                    original.clone()
                } else {
                    let target = temp.path().join("target");
                    fs::write(&target, b"preserve target").unwrap();
                    target
                };
                symlink(&target, &candidate).unwrap();
                assert!(matches!(store.list(), Err(Error::StoreUnavailable)));
                assert!(matches!(
                    store.insert(&self::profile()),
                    Err(Error::StoreUnavailable)
                ));
                assert!(matches!(
                    store.remove(&profile.id),
                    Err(Error::StoreUnavailable)
                ));
                assert!(
                    fs::symlink_metadata(&candidate)
                        .unwrap()
                        .file_type()
                        .is_symlink()
                );
                fs::remove_file(&candidate).unwrap();
                if original.exists() {
                    fs::rename(original, candidate).unwrap();
                }
                assert_eq!(fs::read(directory.join(DATABASE)).unwrap(), database_before);
                assert!(store.list().unwrap() == vec![profile]);
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn private_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let store = Store::new(temp.path().join("store"));
        store.insert(&profile()).unwrap();
        for (path, expected) in [
            (store.directory.clone(), 0o700),
            (store.directory.join(DATABASE), 0o600),
            (store.directory.join(LOCK), 0o600),
        ] {
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                expected
            );
        }
    }
}
