//! Script runner implementations for Rust Core.

use std::{
    fmt,
    sync::{Arc, Mutex},
};

use arcflow_script::{ScriptCompiler, ScriptDocument};
use arcflow_storage::Storage;

use crate::{
    CompiledScriptQueue, CoreError, NoopCompiledScriptQueue, ScriptRunResult, ScriptRunner,
};

/// Script runner backed by Core-owned SQLite storage.
#[derive(Clone)]
pub struct StorageScriptRunner {
    storage: Arc<Mutex<Storage>>,
    compiler: ScriptCompiler,
    queue: Arc<dyn CompiledScriptQueue>,
}

impl StorageScriptRunner {
    /// Constructs a storage-backed script runner.
    #[must_use]
    pub fn new(storage: Arc<Mutex<Storage>>, compiler: ScriptCompiler) -> Self {
        Self::with_queue(storage, compiler, Arc::new(NoopCompiledScriptQueue))
    }

    /// Constructs a storage-backed script runner with an explicit execution queue.
    #[must_use]
    pub fn with_queue(
        storage: Arc<Mutex<Storage>>,
        compiler: ScriptCompiler,
        queue: Arc<dyn CompiledScriptQueue>,
    ) -> Self {
        Self {
            storage,
            compiler,
            queue,
        }
    }
}

impl fmt::Debug for StorageScriptRunner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StorageScriptRunner")
            .field("compiler", &self.compiler)
            .finish_non_exhaustive()
    }
}

impl ScriptRunner for StorageScriptRunner {
    fn run_script(&self, script_id: &str) -> Result<ScriptRunResult, CoreError> {
        let document_json = {
            let storage = self.storage.lock().expect("script storage mutex poisoned");
            storage
                .scripts()
                .get(script_id)?
                .ok_or_else(|| CoreError::Script(format!("script `{script_id}` was not found")))?
                .document_json
        };
        let document = ScriptDocument::from_json(&document_json)?;
        let compiled = self.compiler.compile(document)?;
        let script_id = compiled.id().to_owned();
        self.queue.enqueue(compiled)?;

        Ok(ScriptRunResult::new(script_id, true))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use arcflow_script::CompiledScript;
    use arcflow_storage::{Storage, StoredScriptRecord};

    use super::*;

    #[derive(Debug, Default)]
    struct RecordingQueue {
        script_ids: StdMutex<Vec<String>>,
        fail: bool,
    }

    impl CompiledScriptQueue for RecordingQueue {
        fn enqueue(&self, script: CompiledScript) -> Result<(), CoreError> {
            self.script_ids.lock().unwrap().push(script.id().to_owned());

            if self.fail {
                return Err(CoreError::Script("queue failed".to_owned()));
            }

            Ok(())
        }
    }

    #[test]
    fn runs_stored_script_after_compilation() {
        let storage = Arc::new(Mutex::new(Storage::in_memory().unwrap()));
        storage
            .lock()
            .unwrap()
            .scripts()
            .upsert(&StoredScriptRecord::new(
                "script.demo",
                r#"{"id":"script.demo","version":1,"steps":[{"type":"wait","durationMs":1}]}"#,
            ))
            .unwrap();
        let runner = StorageScriptRunner::new(storage, ScriptCompiler::default());

        let result = runner.run_script("script.demo").unwrap();

        assert_eq!(result.script_id(), "script.demo");
        assert!(result.queued());
    }

    #[test]
    fn enqueues_compiled_script() {
        let storage = Arc::new(Mutex::new(Storage::in_memory().unwrap()));
        storage
            .lock()
            .unwrap()
            .scripts()
            .upsert(&StoredScriptRecord::new(
                "script.demo",
                r#"{"id":"script.demo","version":1,"steps":[{"type":"wait","durationMs":1}]}"#,
            ))
            .unwrap();
        let queue = Arc::new(RecordingQueue::default());
        let runner =
            StorageScriptRunner::with_queue(storage, ScriptCompiler::default(), queue.clone());

        runner.run_script("script.demo").unwrap();

        assert_eq!(*queue.script_ids.lock().unwrap(), vec!["script.demo"]);
    }

    #[test]
    fn propagates_queue_failures() {
        let storage = Arc::new(Mutex::new(Storage::in_memory().unwrap()));
        storage
            .lock()
            .unwrap()
            .scripts()
            .upsert(&StoredScriptRecord::new(
                "script.demo",
                r#"{"id":"script.demo","version":1,"steps":[{"type":"wait","durationMs":1}]}"#,
            ))
            .unwrap();
        let queue = Arc::new(RecordingQueue {
            fail: true,
            ..RecordingQueue::default()
        });
        let runner = StorageScriptRunner::with_queue(storage, ScriptCompiler::default(), queue);

        let error = runner.run_script("script.demo").unwrap_err();

        assert!(matches!(error, CoreError::Script(_)));
    }

    #[test]
    fn rejects_missing_script() {
        let storage = Arc::new(Mutex::new(Storage::in_memory().unwrap()));
        let runner = StorageScriptRunner::new(storage, ScriptCompiler::default());

        let error = runner.run_script("missing").unwrap_err();

        assert!(matches!(error, CoreError::Script(_)));
    }
}
