//! Plugin-private key-value storage.

use rusqlite::{params, Connection, OptionalExtension};

use crate::{ensure_not_blank, unix_time_seconds, StorageError};

/// Plugin-private key-value store.
pub struct PluginKvStore<'a> {
    connection: &'a Connection,
}

impl<'a> PluginKvStore<'a> {
    /// Constructs a plugin KV store over an existing SQLite connection.
    #[must_use]
    pub(crate) fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Stores a value for one plugin and key.
    pub fn put(&self, plugin_id: &str, key: &str, value: &[u8]) -> Result<(), StorageError> {
        ensure_not_blank("plugin_id", plugin_id)?;
        ensure_not_blank("key", key)?;

        self.connection
            .execute(
                "
                INSERT INTO plugin_kv (plugin_id, key, value, updated_at)
                VALUES (?1, ?2, ?3, ?4)
                ON CONFLICT(plugin_id, key) DO UPDATE SET
                    value = excluded.value,
                    updated_at = excluded.updated_at
                ",
                params![plugin_id, key, value, unix_time_seconds()],
            )
            .map_err(StorageError::Sql)?;

        Ok(())
    }

    /// Reads a value for one plugin and key.
    pub fn get(&self, plugin_id: &str, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        ensure_not_blank("plugin_id", plugin_id)?;
        ensure_not_blank("key", key)?;

        self.connection
            .query_row(
                "SELECT value FROM plugin_kv WHERE plugin_id = ?1 AND key = ?2",
                params![plugin_id, key],
                |row| row.get(0),
            )
            .optional()
            .map_err(StorageError::Sql)
    }

    /// Deletes a value for one plugin and key.
    pub fn delete(&self, plugin_id: &str, key: &str) -> Result<(), StorageError> {
        ensure_not_blank("plugin_id", plugin_id)?;
        ensure_not_blank("key", key)?;

        self.connection
            .execute(
                "DELETE FROM plugin_kv WHERE plugin_id = ?1 AND key = ?2",
                params![plugin_id, key],
            )
            .map_err(StorageError::Sql)?;

        Ok(())
    }

    /// Lists keys for one plugin in lexical order.
    pub fn list_keys(&self, plugin_id: &str) -> Result<Vec<String>, StorageError> {
        ensure_not_blank("plugin_id", plugin_id)?;

        let mut statement = self
            .connection
            .prepare("SELECT key FROM plugin_kv WHERE plugin_id = ?1 ORDER BY key ASC")
            .map_err(StorageError::Sql)?;

        let rows = statement
            .query_map(params![plugin_id], |row| row.get::<_, String>(0))
            .map_err(StorageError::Sql)?;

        let mut keys = Vec::new();
        for row in rows {
            keys.push(row.map_err(StorageError::Sql)?);
        }
        Ok(keys)
    }
}

#[cfg(test)]
mod tests {
    use crate::Storage;

    #[test]
    fn stores_plugin_values_in_separate_namespaces() {
        let storage = Storage::in_memory().unwrap();
        let kv = storage.plugin_kv();

        kv.put("plugin.a", "token", b"a").unwrap();
        kv.put("plugin.b", "token", b"b").unwrap();

        assert_eq!(kv.get("plugin.a", "token").unwrap(), Some(b"a".to_vec()));
        assert_eq!(kv.get("plugin.b", "token").unwrap(), Some(b"b".to_vec()));
    }

    #[test]
    fn updates_and_deletes_values() {
        let storage = Storage::in_memory().unwrap();
        let kv = storage.plugin_kv();

        kv.put("plugin.a", "setting", b"one").unwrap();
        kv.put("plugin.a", "setting", b"two").unwrap();

        assert_eq!(
            kv.get("plugin.a", "setting").unwrap(),
            Some(b"two".to_vec())
        );

        kv.delete("plugin.a", "setting").unwrap();

        assert_eq!(kv.get("plugin.a", "setting").unwrap(), None);
    }

    #[test]
    fn lists_keys_in_order() {
        let storage = Storage::in_memory().unwrap();
        let kv = storage.plugin_kv();

        kv.put("plugin.a", "z", b"1").unwrap();
        kv.put("plugin.a", "a", b"1").unwrap();

        assert_eq!(kv.list_keys("plugin.a").unwrap(), vec!["a", "z"]);
    }
}
