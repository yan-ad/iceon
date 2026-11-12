CREATE TABLE metadata (singleton INTEGER PRIMARY KEY CHECK(singleton = 1), schema_version INTEGER NOT NULL CHECK(schema_version = 5), revision TEXT NOT NULL, auto_discover INTEGER NOT NULL CHECK(auto_discover IN (0,1))) STRICT;
CREATE TABLE clusters (id TEXT PRIMARY KEY NOT NULL, context_name TEXT NOT NULL, source_path TEXT NOT NULL, source_alias TEXT NOT NULL, pinned INTEGER NOT NULL CHECK(pinned IN (0,1)), color TEXT NOT NULL, pinned_sections TEXT NOT NULL, UNIQUE(source_path, context_name)) STRICT;
CREATE TABLE pin_order (position INTEGER PRIMARY KEY CHECK(position >= 0), id TEXT NOT NULL, section TEXT) STRICT;
CREATE TABLE IF NOT EXISTS namespace_snapshots (identity TEXT PRIMARY KEY NOT NULL CHECK(length(identity)=64), names TEXT NOT NULL CHECK(length(names)<=70000), fetched_at INTEGER NOT NULL, expires_at INTEGER NOT NULL, partial INTEGER NOT NULL CHECK(partial IN (0,1))) STRICT;

CREATE TABLE IF NOT EXISTS kubeconfig_sync_ownership (id TEXT PRIMARY KEY NOT NULL, alias TEXT NOT NULL) STRICT;
