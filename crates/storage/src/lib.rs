#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! SQLite storage owned by Rust.
//!
//! React, plugins, and external-control clients must not access SQLite
//! directly. They request storage actions through Rust Core.

pub mod plugin_kv;
pub mod plugin_registry;
pub mod scripts;

use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::Connection;

pub use plugin_kv::PluginKvStore;
pub use plugin_registry::{PluginRegistryStore, StoredPluginRecord};
pub use scripts::{ScriptStore, StoredScriptRecord};

/// Current storage schema version.
pub const SCHEMA_VERSION: i64 = 4;

/// SQLite storage handle.
pub struct Storage {
    connection: Connection,
}

impl Storage {
    /// Opens a SQLite database at `path` and applies migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let connection = Connection::open(path).map_err(StorageError::Sql)?;
        let storage = Self { connection };
        storage.migrate()?;
        Ok(storage)
    }

    /// Opens an in-memory SQLite database and applies migrations.
    pub fn in_memory() -> Result<Self, StorageError> {
        let connection = Connection::open_in_memory().map_err(StorageError::Sql)?;
        let storage = Self { connection };
        storage.migrate()?;
        Ok(storage)
    }

    /// Returns a plugin-private KV store Interface.
    #[must_use]
    pub fn plugin_kv(&self) -> PluginKvStore<'_> {
        PluginKvStore::new(&self.connection)
    }

    /// Returns a persistent installed plugin registry Interface.
    #[must_use]
    pub fn plugin_registry(&self) -> PluginRegistryStore<'_> {
        PluginRegistryStore::new(&self.connection)
    }

    /// Returns a persistent script store Interface.
    #[must_use]
    pub fn scripts(&self) -> ScriptStore<'_> {
        ScriptStore::new(&self.connection)
    }

    /// Returns the current applied schema version.
    pub fn schema_version(&self) -> Result<i64, StorageError> {
        self.connection
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                [],
                |row| row.get(0),
            )
            .map_err(StorageError::Sql)
    }

    fn migrate(&self) -> Result<(), StorageError> {
        self.connection
            .execute_batch(
                "
                CREATE TABLE IF NOT EXISTS schema_migrations (
                    version INTEGER PRIMARY KEY,
                    applied_at INTEGER NOT NULL
                );
                ",
            )
            .map_err(StorageError::Sql)?;

        if self.schema_version()? < 1 {
            self.connection
                .execute_batch(
                    "
                    CREATE TABLE plugin_kv (
                        plugin_id TEXT NOT NULL,
                        key TEXT NOT NULL,
                        value BLOB NOT NULL,
                        updated_at INTEGER NOT NULL,
                        PRIMARY KEY (plugin_id, key)
                    );
                    ",
                )
                .map_err(StorageError::Sql)?;

            self.connection
                .execute(
                    "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                    rusqlite::params![1, unix_time_seconds()],
                )
                .map_err(StorageError::Sql)?;
        }

        if self.schema_version()? < 2 {
            self.connection
                .execute_batch(
                    "
                    CREATE TABLE plugin_registry (
                        plugin_id TEXT PRIMARY KEY,
                        manifest_json TEXT NOT NULL,
                        enabled INTEGER NOT NULL DEFAULT 0,
                        installed_at INTEGER NOT NULL,
                        updated_at INTEGER NOT NULL
                    );
                    ",
                )
                .map_err(StorageError::Sql)?;

            self.connection
                .execute(
                    "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                    rusqlite::params![2, unix_time_seconds()],
                )
                .map_err(StorageError::Sql)?;
        }

        if self.schema_version()? < 3 {
            self.connection
                .execute_batch(
                    "
                    ALTER TABLE plugin_registry
                    ADD COLUMN bundle_root TEXT;
                    ",
                )
                .map_err(StorageError::Sql)?;

            self.connection
                .execute(
                    "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                    rusqlite::params![3, unix_time_seconds()],
                )
                .map_err(StorageError::Sql)?;
        }

        if self.schema_version()? < 4 {
            self.connection
                .execute_batch(
                    "
                    CREATE TABLE scripts (
                        script_id TEXT PRIMARY KEY,
                        document_json TEXT NOT NULL,
                        updated_at INTEGER NOT NULL
                    );
                    ",
                )
                .map_err(StorageError::Sql)?;

            self.connection
                .execute(
                    "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                    rusqlite::params![4, unix_time_seconds()],
                )
                .map_err(StorageError::Sql)?;
        }

        Ok(())
    }
}

/// Error returned by storage operations.
#[derive(Debug)]
pub enum StorageError {
    /// SQLite returned an error.
    Sql(rusqlite::Error),
    /// Plugin id or key is blank.
    BlankIdentifier(&'static str),
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sql(error) => write!(f, "sqlite error: {error}"),
            Self::BlankIdentifier(name) => write!(f, "{name} must not be blank"),
        }
    }
}

impl std::error::Error for StorageError {}

fn unix_time_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn ensure_not_blank(name: &'static str, value: &str) -> Result<(), StorageError> {
    if value.trim().is_empty() {
        return Err(StorageError::BlankIdentifier(name));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use rusqlite::Connection;

    use super::*;

    #[test]
    fn applies_initial_schema() {
        let storage = Storage::in_memory().unwrap();

        assert_eq!(storage.schema_version().unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn migrates_v2_plugin_registry_to_v3_bundle_roots() {
        let path = temp_db_path("v2-plugin-registry");
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    r#"
                    CREATE TABLE schema_migrations (
                        version INTEGER PRIMARY KEY,
                        applied_at INTEGER NOT NULL
                    );
                    INSERT INTO schema_migrations (version, applied_at) VALUES (1, 1);
                    INSERT INTO schema_migrations (version, applied_at) VALUES (2, 2);

                    CREATE TABLE plugin_kv (
                        plugin_id TEXT NOT NULL,
                        key TEXT NOT NULL,
                        value BLOB NOT NULL,
                        updated_at INTEGER NOT NULL,
                        PRIMARY KEY (plugin_id, key)
                    );

                    CREATE TABLE plugin_registry (
                        plugin_id TEXT PRIMARY KEY,
                        manifest_json TEXT NOT NULL,
                        enabled INTEGER NOT NULL DEFAULT 0,
                        installed_at INTEGER NOT NULL,
                        updated_at INTEGER NOT NULL
                    );

                    INSERT INTO plugin_registry (
                        plugin_id,
                        manifest_json,
                        enabled,
                        installed_at,
                        updated_at
                    )
                    VALUES (
                        'plugin.legacy',
                        '{"id":"plugin.legacy","name":"Legacy","version":"1.0.0","runtime":"wasm","entry":"dist/plugin.wasm","apiVersion":"1","capabilities":["device.read"]}',
                        1,
                        2,
                        2
                    );
                    "#,
                )
                .unwrap();
        }

        let storage = Storage::open(&path).unwrap();
        let record = storage
            .plugin_registry()
            .get("plugin.legacy")
            .unwrap()
            .unwrap();

        assert_eq!(storage.schema_version().unwrap(), SCHEMA_VERSION);
        assert!(record.enabled);
        assert_eq!(record.bundle_root, None);
        assert!(storage.scripts().list().unwrap().is_empty());

        drop(storage);
        fs::remove_file(path).unwrap();
    }

    fn temp_db_path(name: &str) -> std::path::PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("arcflow-storage-{name}-{suffix}.sqlite3"))
    }
}
