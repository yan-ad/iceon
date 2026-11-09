use super::tests::{CONFIG, fixture, import};
use super::*;

#[cfg(unix)]
#[test]
fn relative_chained_directory_aliases_dedup_and_retarget() {
    use std::os::unix::fs::symlink;
    let (dir, engine, input) = fixture();
    let direct = import(&engine, &input).unwrap().clusters.remove(0);
    engine
        .set_pin(PinRequest {
            id: direct.id.clone(),
            pinned: true,
        })
        .unwrap();
    let relative = dir.path().join("relative");
    symlink("fixture.yaml", &relative).unwrap();
    let chain = dir.path().join("chain");
    symlink("relative", &chain).unwrap();
    let directory = dir.path().join("directory");
    symlink(dir.path(), &directory).unwrap();
    for alias in [&relative, &chain, &directory.join("fixture.yaml")] {
        let result = import(&engine, alias).unwrap();
        assert_eq!(result.clusters.len(), 1);
        let cluster = &result.clusters[0];
        assert_eq!(cluster.id, direct.id);
        assert_eq!(cluster.color, direct.color);
        assert!(cluster.pinned);
        assert_eq!(cluster.labels, [format!("file={}", alias.display())]);
        assert_eq!(
            CatalogEngine::new(engine.directory.clone()).list().unwrap(),
            result
        );
    }
    import(&engine, &relative).unwrap();
    fs::write(&input, CONFIG.replace("development", "rewritten")).unwrap();
    let rewritten = import(&engine, &relative).unwrap();
    assert_eq!(rewritten.clusters[0].name, "rewritten");
    let other = dir.path().join("other.yaml");
    fs::write(&other, CONFIG).unwrap();
    fs::remove_file(&relative).unwrap();
    symlink("other.yaml", &relative).unwrap();
    let retargeted = import(&engine, &relative).unwrap();
    assert_eq!(retargeted.clusters.len(), 1);
    assert_eq!(retargeted.clusters[0].name, "development");
    assert_ne!(retargeted.clusters[0].id, rewritten.clusters[0].id);

    // Selecting the old target independently transfers its provenance ownership.
    fs::remove_file(&relative).unwrap();
    symlink("fixture.yaml", &relative).unwrap();
    import(&engine, &relative).unwrap();
    import(&engine, &input).unwrap();
    fs::remove_file(&relative).unwrap();
    symlink("other.yaml", &relative).unwrap();
    assert_eq!(import(&engine, &relative).unwrap().clusters.len(), 2);
}

#[cfg(unix)]
#[test]
fn default_symlink_discovery_and_failed_targets_preserve_rows() {
    use std::os::unix::fs::symlink;
    let (dir, _, target) = fixture();
    let home = dir.path().join("home");
    fs::create_dir_all(home.join(".kube")).unwrap();
    let alias = home.join(".kube/config");
    symlink(&target, &alias).unwrap();
    let engine = CatalogEngine::new_with_home(dir.path().join("discovery"), Some(home.clone()));
    let first = engine.list().unwrap();
    assert_eq!(first.clusters[0].labels, ["file=~/.kube/config"]);
    assert_eq!(
        CatalogEngine::new_with_home(engine.directory.clone(), Some(home))
            .list()
            .unwrap(),
        first
    );
    let alternate = dir.path().join("alternate");
    fs::write(&alternate, CONFIG.replace("development", "retargeted")).unwrap();
    fs::remove_file(&alias).unwrap();
    symlink(&alternate, &alias).unwrap();
    let first = engine.list().unwrap();
    assert_eq!(first.clusters.len(), 1);
    assert_eq!(first.clusters[0].name, "retargeted");
    assert_eq!(first.clusters[0].labels, ["file=~/.kube/config"]);
    let loop_path = dir.path().join("loop");
    symlink(&loop_path, &loop_path).unwrap();
    let large = dir.path().join("large");
    fs::write(&large, vec![b'x'; MAX_INPUT + 1]).unwrap();
    // A Unix-domain socket is a nonregular target without blocking FIFO opens.
    let socket = dir.path().join("socket");
    let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    for (path, error) in [
        (dir.path().join("missing"), CatalogError::InputUnavailable),
        (loop_path, CatalogError::InputUnavailable),
        (dir.path().to_owned(), CatalogError::InputUnavailable),
        (socket, CatalogError::InputUnavailable),
        (large, CatalogError::LimitExceeded),
    ] {
        fs::remove_file(&alias).unwrap();
        symlink(path, &alias).unwrap();
        assert_eq!(import(&engine, &alias), Err(error));
        let warning = engine.list().unwrap();
        assert_eq!(warning.clusters, first.clusters);
        assert_eq!(warning.revision, first.revision);
        assert_eq!(warning.discovery_error, Some(error));
    }
}

#[cfg(unix)]
#[test]
fn storage_symlinks_remain_rejected() {
    use std::os::unix::fs::symlink;
    for name in [
        "catalog.lock",
        "catalog.sqlite3",
        "catalog.sqlite3-journal",
        "catalog.sqlite3-wal",
        "catalog.sqlite3-shm",
        "catalog.json",
        "catalog.json.backup",
    ] {
        let (dir, engine, input) = fixture();
        fs::create_dir_all(&engine.directory).unwrap();
        if name == "catalog.json.backup" {
            fs::write(
                engine.directory.join("catalog.json"),
                br#"{"schemaVersion":1,"revision":0,"clusters":[],"webLinks":[]}"#,
            )
            .unwrap();
        }
        symlink(&input, engine.directory.join(name)).unwrap();
        assert!(engine.list().is_err(), "{name}");
        assert_eq!(
            fs::read(dir.path().join("fixture.yaml")).unwrap(),
            CONFIG.as_bytes()
        );
    }
}

fn schema_one(engine: &CatalogEngine, source: &str) {
    fs::create_dir_all(&engine.directory).unwrap();
    let connection = rusqlite::Connection::open(engine.directory.join("catalog.sqlite3")).unwrap();
    connection.execute_batch("
        CREATE TABLE metadata (singleton INTEGER PRIMARY KEY CHECK(singleton=1), schema_version INTEGER NOT NULL CHECK(schema_version=1), revision TEXT NOT NULL, auto_discover INTEGER NOT NULL CHECK(auto_discover IN (0,1))) STRICT;
        CREATE TABLE clusters (id TEXT PRIMARY KEY NOT NULL, context_name TEXT NOT NULL, source_path TEXT NOT NULL, pinned INTEGER NOT NULL CHECK(pinned IN (0,1)), color TEXT NOT NULL, UNIQUE(source_path,context_name)) STRICT;
        CREATE TABLE web_links (id TEXT PRIMARY KEY NOT NULL, name TEXT NOT NULL, url TEXT NOT NULL UNIQUE, pinned INTEGER NOT NULL CHECK(pinned IN (0,1)), color TEXT NOT NULL) STRICT;
        INSERT INTO metadata VALUES(1,1,'18446744073709551614',1);
    ").unwrap();
    connection
        .execute(
            "INSERT INTO clusters VALUES(?1,'development',?2,1,'#123456')",
            rusqlite::params![cluster_id(source, "development"), source],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO web_links VALUES(?1,'Docs','https://example.test/',1,'#ABCDEF')",
            ["retired-link"],
        )
        .unwrap();
}

#[test]
fn schema_one_upgrade_preserves_records_revision_settings_colors() {
    let (_dir, engine, input) = fixture();
    let source = fs::canonicalize(input).unwrap();
    schema_one(&engine, source.to_str().unwrap());
    // No injected home: do not infer or read the host home.
    let migrated = engine.list().unwrap();
    assert_eq!(migrated.revision, u64::MAX - 1);
    assert!(migrated.discovery_error.is_none());
    assert_eq!(migrated.clusters[0].color, "#123456");
    assert!(migrated.clusters[0].pinned);
    assert_eq!(
        migrated.clusters[0].labels,
        [format!("file={}", source.display())]
    );
    assert_eq!(
        CatalogEngine::new(engine.directory.clone()).list().unwrap(),
        migrated
    );
    let connection = storage::open(&engine.directory).unwrap();
    let stored = storage::load(&connection).unwrap();
    assert_eq!(stored.schema_version, 5);
    assert!(stored.auto_discover);
    drop(connection);
    let updated = engine
        .set_pin(PinRequest {
            id: migrated.clusters[0].id.clone(),
            pinned: false,
        })
        .unwrap();
    assert_eq!(updated.revision, u64::MAX);
    assert_eq!(updated.clusters[0].color, "#123456");
    assert_eq!(
        CatalogEngine::new(engine.directory.clone())
            .list()
            .unwrap()
            .clusters,
        updated.clusters
    );
}

#[test]
fn invalid_and_newer_schema_upgrade_fail_closed() {
    for newer in [false, true] {
        let (_dir, engine, input) = fixture();
        schema_one(&engine, input.to_str().unwrap());
        let path = engine.directory.join("catalog.sqlite3");
        let connection = rusqlite::Connection::open(&path).unwrap();
        if newer {
            connection
                .execute_batch(
                    "PRAGMA ignore_check_constraints=ON; UPDATE metadata SET schema_version=6;",
                )
                .unwrap();
        } else {
            connection
                .execute("UPDATE clusters SET id='invalid'", [])
                .unwrap();
        }
        drop(connection);
        let before = fs::read(&path).unwrap();
        assert_eq!(engine.list(), Err(CatalogError::StoreCorrupt));
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}

#[cfg(unix)]
#[test]
fn broken_default_aliases_warn_even_without_prior_rows() {
    use std::os::unix::fs::symlink;
    let (dir, _, _) = fixture();
    let home = dir.path().join("home");
    fs::create_dir_all(home.join(".kube")).unwrap();
    let input = home.join(".kube/config");
    let engine = CatalogEngine::new_with_home(dir.path().join("new-store"), Some(home.clone()));
    symlink("missing-target", &input).unwrap();
    assert_eq!(
        engine.list().unwrap().discovery_error,
        Some(CatalogError::InputUnavailable)
    );
    fs::remove_file(&input).unwrap();
    fs::remove_dir(home.join(".kube")).unwrap();
    symlink("missing-directory", home.join(".kube")).unwrap();
    assert_eq!(
        engine.list().unwrap().discovery_error,
        Some(CatalogError::InputUnavailable)
    );
}
