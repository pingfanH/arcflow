//! Persistent script document storage.

use rusqlite::{params, Connection, OptionalExtension};

use crate::{ensure_not_blank, unix_time_seconds, StorageError};

/// Script document record stored in SQLite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredScriptRecord {
    /// Stable script id.
    pub script_id: String,
    /// Serialized script document JSON.
    pub document_json: String,
}

impl StoredScriptRecord {
    /// Constructs a stored script record.
    #[must_use]
    pub fn new(script_id: impl Into<String>, document_json: impl Into<String>) -> Self {
        Self {
            script_id: script_id.into(),
            document_json: document_json.into(),
        }
    }
}

/// SQLite-backed script store.
pub struct ScriptStore<'a> {
    connection: &'a Connection,
}

impl<'a> ScriptStore<'a> {
    /// Constructs a script store over an existing SQLite connection.
    #[must_use]
    pub(crate) fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Inserts or updates a script document.
    pub fn upsert(&self, record: &StoredScriptRecord) -> Result<(), StorageError> {
        ensure_not_blank("script_id", &record.script_id)?;
        ensure_not_blank("document_json", &record.document_json)?;

        self.connection
            .execute(
                "
                INSERT INTO scripts (script_id, document_json, updated_at)
                VALUES (?1, ?2, ?3)
                ON CONFLICT(script_id) DO UPDATE SET
                    document_json = excluded.document_json,
                    updated_at = excluded.updated_at
                ",
                params![record.script_id, record.document_json, unix_time_seconds()],
            )
            .map_err(StorageError::Sql)?;

        Ok(())
    }

    /// Reads one script document.
    pub fn get(&self, script_id: &str) -> Result<Option<StoredScriptRecord>, StorageError> {
        ensure_not_blank("script_id", script_id)?;

        self.connection
            .query_row(
                "
                SELECT script_id, document_json
                FROM scripts
                WHERE script_id = ?1
                ",
                params![script_id],
                read_record,
            )
            .optional()
            .map_err(StorageError::Sql)
    }

    /// Lists script documents in stable script id order.
    pub fn list(&self) -> Result<Vec<StoredScriptRecord>, StorageError> {
        let mut statement = self
            .connection
            .prepare(
                "
                SELECT script_id, document_json
                FROM scripts
                ORDER BY script_id ASC
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

    /// Deletes one script document.
    pub fn delete(&self, script_id: &str) -> Result<(), StorageError> {
        ensure_not_blank("script_id", script_id)?;

        self.connection
            .execute(
                "DELETE FROM scripts WHERE script_id = ?1",
                params![script_id],
            )
            .map_err(StorageError::Sql)?;

        Ok(())
    }
}

fn read_record(row: &rusqlite::Row<'_>) -> Result<StoredScriptRecord, rusqlite::Error> {
    Ok(StoredScriptRecord {
        script_id: row.get(0)?,
        document_json: row.get(1)?,
    })
}

#[cfg(test)]
mod tests {
    use crate::Storage;

    use super::*;

    fn record(script_id: &str) -> StoredScriptRecord {
        StoredScriptRecord::new(
            script_id,
            format!(
                r#"{{"id":"{script_id}","version":1,"steps":[{{"type":"wait","durationMs":1}}]}}"#
            ),
        )
    }

    #[test]
    fn stores_and_lists_scripts() {
        let storage = Storage::in_memory().unwrap();
        let scripts = storage.scripts();

        scripts.upsert(&record("script.b")).unwrap();
        scripts.upsert(&record("script.a")).unwrap();

        let records = scripts.list().unwrap();

        assert_eq!(records[0].script_id, "script.a");
        assert_eq!(records[1].script_id, "script.b");
    }

    #[test]
    fn updates_and_deletes_script() {
        let storage = Storage::in_memory().unwrap();
        let scripts = storage.scripts();

        scripts.upsert(&record("script.a")).unwrap();
        scripts
            .upsert(&StoredScriptRecord::new(
                "script.a",
                r#"{"id":"script.a","version":1,"steps":[{"type":"wait","durationMs":2}]}"#,
            ))
            .unwrap();
        assert!(scripts
            .get("script.a")
            .unwrap()
            .unwrap()
            .document_json
            .contains("\"durationMs\":2"));

        scripts.delete("script.a").unwrap();

        assert_eq!(scripts.get("script.a").unwrap(), None);
    }
}
