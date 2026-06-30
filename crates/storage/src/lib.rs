#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! SQLite storage owned by Rust.
//!
//! React, plugins, and external-control clients must not access SQLite
//! directly. They request storage actions through Rust Core.

pub mod plugin_kv;
pub mod plugin_registry;

use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::Connection;

pub use plugin_kv::PluginKvStore;
pub use plugin_registry::{PluginRegistryStore, StoredPluginRecord};

/// Current storage schema version.
pub const SCHEMA_VERSION: i64 = 2;

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
    use super::*;

    #[test]
    fn applies_initial_schema() {
        let storage = Storage::in_memory().unwrap();

        assert_eq!(storage.schema_version().unwrap(), SCHEMA_VERSION);
    }
}
