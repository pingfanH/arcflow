//! Core-owned script document persistence bridge.

use core::fmt;

use arcflow_script::{ScriptCompiler, ScriptDocument};
use arcflow_storage::{Storage, StorageError, StoredScriptRecord};
use serde::Serialize;

/// IPC-friendly script document entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptDocumentEntry {
    /// Stable script id.
    pub script_id: String,
    /// Serialized script document JSON.
    pub document_json: String,
}

/// Core-owned script document persistence.
pub struct ScriptDocumentPersistence<'a> {
    storage: &'a Storage,
    compiler: ScriptCompiler,
}

impl<'a> ScriptDocumentPersistence<'a> {
    /// Constructs a script document persistence bridge.
    #[must_use]
    pub fn new(storage: &'a Storage) -> Self {
        Self::with_compiler(storage, ScriptCompiler::default())
    }

    /// Constructs a script document persistence bridge with an explicit compiler.
    #[must_use]
    pub fn with_compiler(storage: &'a Storage, compiler: ScriptCompiler) -> Self {
        Self { storage, compiler }
    }

    /// Lists stored script documents in stable script id order.
    pub fn list(&self) -> Result<Vec<ScriptDocumentEntry>, ScriptDocumentPersistenceError> {
        self.storage
            .scripts()
            .list()?
            .into_iter()
            .map(script_document_entry_from_stored)
            .collect()
    }

    /// Inserts or updates one validated script document.
    pub fn upsert(
        &self,
        script_id: &str,
        document_json: &str,
    ) -> Result<Vec<ScriptDocumentEntry>, ScriptDocumentPersistenceError> {
        let record = self.validate_script_record(script_id, document_json)?;
        self.storage.scripts().upsert(&record)?;
        self.list()
    }

    /// Deletes one script document.
    pub fn delete(
        &self,
        script_id: &str,
    ) -> Result<Vec<ScriptDocumentEntry>, ScriptDocumentPersistenceError> {
        self.storage.scripts().delete(script_id)?;
        self.list()
    }

    fn validate_script_record(
        &self,
        script_id: &str,
        document_json: &str,
    ) -> Result<StoredScriptRecord, ScriptDocumentPersistenceError> {
        let document = ScriptDocument::from_json(document_json)
            .map_err(|error| ScriptDocumentPersistenceError::Script(error.to_string()))?;

        if document.id != script_id {
            return Err(ScriptDocumentPersistenceError::MismatchedId {
                requested: script_id.to_owned(),
                document: document.id,
            });
        }

        self.compiler
            .compile(document)
            .map_err(|error| ScriptDocumentPersistenceError::Script(error.to_string()))?;

        Ok(StoredScriptRecord::new(script_id, document_json))
    }
}

fn script_document_entry_from_stored(
    record: StoredScriptRecord,
) -> Result<ScriptDocumentEntry, ScriptDocumentPersistenceError> {
    Ok(ScriptDocumentEntry {
        script_id: record.script_id,
        document_json: record.document_json,
    })
}

/// Error returned by script document persistence.
#[derive(Debug, PartialEq, Eq)]
pub enum ScriptDocumentPersistenceError {
    /// Storage operation failed.
    Storage(String),
    /// Script parsing or validation failed.
    Script(String),
    /// Request script id did not match the document id.
    MismatchedId {
        /// Requested script id.
        requested: String,
        /// Script id inside the document.
        document: String,
    },
}

impl fmt::Display for ScriptDocumentPersistenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(error) => write!(f, "script storage error: {error}"),
            Self::Script(error) => write!(f, "script validation error: {error}"),
            Self::MismatchedId {
                requested,
                document,
            } => write!(
                f,
                "script id `{requested}` does not match document id `{document}`"
            ),
        }
    }
}

impl std::error::Error for ScriptDocumentPersistenceError {}

impl From<StorageError> for ScriptDocumentPersistenceError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use arcflow_storage::Storage;

    use super::*;

    #[test]
    fn validates_script_before_storage() {
        let storage = Storage::in_memory().unwrap();
        let persistence = ScriptDocumentPersistence::new(&storage);

        let scripts = persistence
            .upsert(
                "script.demo",
                r#"{"id":"script.demo","version":1,"steps":[{"type":"wait","durationMs":1}]}"#,
            )
            .unwrap();

        assert_eq!(scripts[0].script_id, "script.demo");
    }

    #[test]
    fn rejects_mismatched_script_id() {
        let storage = Storage::in_memory().unwrap();
        let persistence = ScriptDocumentPersistence::new(&storage);

        let error = persistence
            .upsert(
                "script.outer",
                r#"{"id":"script.inner","version":1,"steps":[{"type":"wait","durationMs":1}]}"#,
            )
            .unwrap_err();

        assert_eq!(
            error,
            ScriptDocumentPersistenceError::MismatchedId {
                requested: "script.outer".to_owned(),
                document: "script.inner".to_owned(),
            }
        );
    }

    #[test]
    fn rejects_invalid_script_documents() {
        let storage = Storage::in_memory().unwrap();
        let persistence = ScriptDocumentPersistence::new(&storage);

        let error = persistence
            .upsert(
                "script.empty",
                r#"{"id":"script.empty","version":1,"steps":[]}"#,
            )
            .unwrap_err();

        assert!(error.to_string().contains("at least one step"));
    }

    #[test]
    fn lists_and_deletes_scripts_in_storage_order() {
        let storage = Storage::in_memory().unwrap();
        let persistence = ScriptDocumentPersistence::new(&storage);

        persistence
            .upsert(
                "script.b",
                r#"{"id":"script.b","version":1,"steps":[{"type":"wait","durationMs":1}]}"#,
            )
            .unwrap();
        persistence
            .upsert(
                "script.a",
                r#"{"id":"script.a","version":1,"steps":[{"type":"wait","durationMs":1}]}"#,
            )
            .unwrap();

        let scripts = persistence.list().unwrap();

        assert_eq!(scripts[0].script_id, "script.a");
        assert_eq!(scripts[1].script_id, "script.b");

        let scripts = persistence.delete("script.a").unwrap();

        assert_eq!(scripts[0].script_id, "script.b");
    }
}
