CREATE TABLE IF NOT EXISTS preferences_store (
                    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                    schema_version INTEGER NOT NULL CHECK(schema_version = 1),
                    revision INTEGER NOT NULL,
                    payload TEXT NOT NULL
                ) STRICT;
                CREATE TABLE IF NOT EXISTS route_history (
                    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                    payload TEXT NOT NULL
                ) STRICT;
