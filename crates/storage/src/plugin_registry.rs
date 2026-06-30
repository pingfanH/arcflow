//! Persistent installed plugin registry.

use rusqlite::{params, Connection, OptionalExtension};

use crate::{ensure_not_blank, unix_time_seconds, StorageError};

/// Plugin registry record stored in SQLite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPluginRecord {
    /// Stable plugin id.
    pub plugin_id: String,
    /// Serialized plugin manifest JSON.
    pub manifest_json: String,
    /// Whether the plugin should be enabled at startup.
    pub enabled: bool,
    /// Plugin bundle root directory if installed from disk.
    pub bundle_root: Option<String>,
}

impl StoredPluginRecord {
    /// Constructs a stored plugin record.
    #[must_use]
    pub fn new(
        plugin_id: impl Into<String>,
        manifest_json: impl Into<String>,
        enabled: bool,
    ) -> Self {
        Self {
            plugin_id: plugin_id.into(),
            manifest_json: manifest_json.into(),
            enabled,
            bundle_root: None,
        }
    }

    /// Constructs a stored plugin record with a bundle root.
    #[must_use]
    pub fn with_bundle_root(
        plugin_id: impl Into<String>,
        manifest_json: impl Into<String>,
        enabled: bool,
        bundle_root: impl Into<String>,
    ) -> Self {
        Self {
            plugin_id: plugin_id.into(),
            manifest_json: manifest_json.into(),
            enabled,
            bundle_root: Some(bundle_root.into()),
        }
    }
}

/// SQLite-backed installed plugin registry.
pub struct PluginRegistryStore<'a> {
    connection: &'a Connection,
}

impl<'a> PluginRegistryStore<'a> {
    /// Constructs a plugin registry store over an existing SQLite connection.
    #[must_use]
    pub(crate) fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Inserts or updates a plugin registry record.
    pub fn upsert(&self, record: &StoredPluginRecord) -> Result<(), StorageError> {
        ensure_not_blank("plugin_id", &record.plugin_id)?;
        ensure_not_blank("manifest_json", &record.manifest_json)?;
        let now = unix_time_seconds();

        self.connection
            .execute(
                "
                INSERT INTO plugin_registry (
                    plugin_id,
                    manifest_json,
                    bundle_root,
                    enabled,
                    installed_at,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?5)
                ON CONFLICT(plugin_id) DO UPDATE SET
                    manifest_json = excluded.manifest_json,
                    bundle_root = excluded.bundle_root,
                    enabled = excluded.enabled,
                    updated_at = excluded.updated_at
                ",
                params![
                    record.plugin_id,
                    record.manifest_json,
                    record.bundle_root,
                    enabled_flag(record.enabled),
                    now
                ],
            )
            .map_err(StorageError::Sql)?;

        Ok(())
    }

    /// Reads one plugin registry record.
    pub fn get(&self, plugin_id: &str) -> Result<Option<StoredPluginRecord>, StorageError> {
        ensure_not_blank("plugin_id", plugin_id)?;

        self.connection
            .query_row(
                "
                SELECT plugin_id, manifest_json, bundle_root, enabled
                FROM plugin_registry
                WHERE plugin_id = ?1
                ",
                params![plugin_id],
                read_record,
            )
            .optional()
            .map_err(StorageError::Sql)
    }

    /// Lists installed plugin records in stable plugin id order.
    pub fn list(&self) -> Result<Vec<StoredPluginRecord>, StorageError> {
        let mut statement = self
            .connection
            .prepare(
                "
                SELECT plugin_id, manifest_json, bundle_root, enabled
                FROM plugin_registry
                ORDER BY plugin_id ASC
                ",
            )
            .map_err(StorageError::Sql)?;

        let rows = statement
            .query_map([], read_record)
            .map_err(StorageError::Sql)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(StorageError::Sql)?);
        }
        Ok(records)
    }

    /// Updates whether an installed plugin is enabled.
    pub fn set_enabled(&self, plugin_id: &str, enabled: bool) -> Result<(), StorageError> {
        ensure_not_blank("plugin_id", plugin_id)?;

        self.connection
            .execute(
                "
                UPDATE plugin_registry
                SET enabled = ?1, updated_at = ?2
                WHERE plugin_id = ?3
                ",
                params![enabled_flag(enabled), unix_time_seconds(), plugin_id],
            )
            .map_err(StorageError::Sql)?;

        Ok(())
    }

    /// Deletes an installed plugin record.
    pub fn delete(&self, plugin_id: &str) -> Result<(), StorageError> {
        ensure_not_blank("plugin_id", plugin_id)?;

        self.connection
            .execute(
                "DELETE FROM plugin_registry WHERE plugin_id = ?1",
                params![plugin_id],
            )
            .map_err(StorageError::Sql)?;

        Ok(())
    }
}

fn read_record(row: &rusqlite::Row<'_>) -> Result<StoredPluginRecord, rusqlite::Error> {
    Ok(StoredPluginRecord {
        plugin_id: row.get(0)?,
        manifest_json: row.get(1)?,
        bundle_root: row.get(2)?,
        enabled: row.get::<_, i64>(3)? != 0,
    })
}

fn enabled_flag(enabled: bool) -> i64 {
    i64::from(enabled)
}

#[cfg(test)]
mod tests {
    use crate::Storage;

    use super::*;

    fn record(plugin_id: &str, enabled: bool) -> StoredPluginRecord {
        StoredPluginRecord::new(
            plugin_id,
            format!(
                r#"{{"id":"{plugin_id}","name":"Plugin","version":"1.0.0","runtime":"wasm","entry":"dist/plugin.wasm","apiVersion":"1","capabilities":["device.read"]}}"#
            ),
            enabled,
        )
    }

    #[test]
    fn stores_and_lists_plugin_registry_records() {
        let storage = Storage::in_memory().unwrap();
        let registry = storage.plugin_registry();

        registry.upsert(&record("plugin.b", true)).unwrap();
        registry.upsert(&record("plugin.a", false)).unwrap();

        let records = registry.list().unwrap();

        assert_eq!(records[0].plugin_id, "plugin.a");
        assert!(!records[0].enabled);
        assert_eq!(records[1].plugin_id, "plugin.b");
        assert!(records[1].enabled);
    }

    #[test]
    fn updates_plugin_enabled_state() {
        let storage = Storage::in_memory().unwrap();
        let registry = storage.plugin_registry();

        registry.upsert(&record("plugin.a", false)).unwrap();
        registry.set_enabled("plugin.a", true).unwrap();

        assert!(registry.get("plugin.a").unwrap().unwrap().enabled);
    }

    #[test]
    fn deletes_plugin_registry_record() {
        let storage = Storage::in_memory().unwrap();
        let registry = storage.plugin_registry();

        registry.upsert(&record("plugin.a", false)).unwrap();
        registry.delete("plugin.a").unwrap();

        assert_eq!(registry.get("plugin.a").unwrap(), None);
    }

    #[test]
    fn stores_bundle_root() {
        let storage = Storage::in_memory().unwrap();
        let registry = storage.plugin_registry();

        registry
            .upsert(&StoredPluginRecord::with_bundle_root(
                "plugin.a",
                record("plugin.a", false).manifest_json,
                false,
                "/plugins/plugin.a",
            ))
            .unwrap();

        assert_eq!(
            registry.get("plugin.a").unwrap().unwrap().bundle_root,
            Some("/plugins/plugin.a".to_owned())
        );
    }
}
