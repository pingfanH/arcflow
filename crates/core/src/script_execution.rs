//! Safe script execution orchestration.

use std::{fmt, sync::Arc, time::Duration};

use arcflow_script::{CompiledScript, ScriptStep};
use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;

use crate::CoreError;

/// Queue that accepts compiled scripts for asynchronous execution.
pub trait CompiledScriptQueue: fmt::Debug + Send + Sync {
    /// Enqueues one compiled script.
    fn enqueue(&self, script: CompiledScript) -> Result<(), CoreError>;
}

/// Queue used before a background script worker is attached.
#[derive(Debug, Default)]
pub struct NoopCompiledScriptQueue;

impl CompiledScriptQueue for NoopCompiledScriptQueue {
    fn enqueue(&self, _script: CompiledScript) -> Result<(), CoreError> {
        Ok(())
    }
}

/// Host action surface used by the script execution engine.
#[async_trait]
pub trait ScriptActionExecutor: fmt::Debug + Send + Sync {
    /// Waits for a validated duration before continuing.
    async fn wait(&self, duration: Duration) -> Result<(), CoreError>;

    /// Reads device status through Core-owned device state.
    async fn read_device_status(&self, device_id: &str) -> Result<(), CoreError>;

    /// Stops wave output for one device through Core-owned output control.
    async fn stop_wave(&self, device_id: &str) -> Result<(), CoreError>;

    /// Invokes a plugin hook through the sandboxed plugin API.
    async fn invoke_plugin_hook(
        &self,
        plugin_id: &str,
        hook: &str,
        payload: &Value,
    ) -> Result<(), CoreError>;
}

/// Action executor used before real device/plugin actions are attached.
#[derive(Debug, Default)]
pub struct NoopScriptActionExecutor;

#[async_trait]
impl ScriptActionExecutor for NoopScriptActionExecutor {
    async fn wait(&self, _duration: Duration) -> Result<(), CoreError> {
        Ok(())
    }

    async fn read_device_status(&self, _device_id: &str) -> Result<(), CoreError> {
        Ok(())
    }

    async fn stop_wave(&self, _device_id: &str) -> Result<(), CoreError> {
        Ok(())
    }

    async fn invoke_plugin_hook(
        &self,
        _plugin_id: &str,
        _hook: &str,
        _payload: &Value,
    ) -> Result<(), CoreError> {
        Ok(())
    }
}

/// Executes compiled scripts through an injected host action surface.
#[derive(Debug, Clone)]
pub struct ScriptExecutionEngine {
    actions: Arc<dyn ScriptActionExecutor>,
}

impl ScriptExecutionEngine {
    /// Constructs a script execution engine.
    #[must_use]
    pub fn new(actions: Arc<dyn ScriptActionExecutor>) -> Self {
        Self { actions }
    }

    /// Executes a compiled script step by step.
    pub async fn execute(
        &self,
        script: &CompiledScript,
    ) -> Result<ScriptExecutionReport, CoreError> {
        let mut steps = Vec::with_capacity(script.steps().len());

        for (index, step) in script.steps().iter().enumerate() {
            let kind = self.execute_step(step).await?;
            steps.push(ScriptStepExecution { index, kind });
        }

        Ok(ScriptExecutionReport {
            script_id: script.id().to_owned(),
            steps,
        })
    }

    async fn execute_step(&self, step: &ScriptStep) -> Result<ScriptStepKind, CoreError> {
        match step {
            ScriptStep::Wait { duration_ms } => {
                self.actions
                    .wait(Duration::from_millis(*duration_ms))
                    .await?;
                Ok(ScriptStepKind::Wait)
            }
            ScriptStep::DeviceStatus { device_id } => {
                self.actions.read_device_status(device_id).await?;
                Ok(ScriptStepKind::DeviceStatus)
            }
            ScriptStep::WaveStop { device_id } => {
                self.actions.stop_wave(device_id).await?;
                Ok(ScriptStepKind::WaveStop)
            }
            ScriptStep::PluginHook {
                plugin_id,
                hook,
                payload,
            } => {
                self.actions
                    .invoke_plugin_hook(plugin_id, hook, payload)
                    .await?;
                Ok(ScriptStepKind::PluginHook)
            }
        }
    }
}

impl Default for ScriptExecutionEngine {
    fn default() -> Self {
        Self::new(Arc::new(NoopScriptActionExecutor))
    }
}

/// Report returned after a compiled script finishes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptExecutionReport {
    /// Executed script id.
    pub script_id: String,
    /// Steps completed before the script finished.
    pub steps: Vec<ScriptStepExecution>,
}

/// One completed script step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptStepExecution {
    /// Zero-based step index.
    pub index: usize,
    /// Executed step kind.
    pub kind: ScriptStepKind,
}

/// Kind of script step executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ScriptStepKind {
    /// Wait step.
    Wait,
    /// Device status read step.
    DeviceStatus,
    /// Wave stop step.
    WaveStop,
    /// Plugin hook invocation step.
    PluginHook,
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use arcflow_script::{ScriptCompiler, ScriptDocument};
    use serde_json::json;

    use super::*;

    #[derive(Debug, Default)]
    struct RecordingActions {
        calls: Mutex<Vec<String>>,
        fail_wave_stop: bool,
    }

    #[async_trait]
    impl ScriptActionExecutor for RecordingActions {
        async fn wait(&self, duration: Duration) -> Result<(), CoreError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("wait:{}", duration.as_millis()));
            Ok(())
        }

        async fn read_device_status(&self, device_id: &str) -> Result<(), CoreError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("status:{device_id}"));
            Ok(())
        }

        async fn stop_wave(&self, device_id: &str) -> Result<(), CoreError> {
            self.calls.lock().unwrap().push(format!("stop:{device_id}"));

            if self.fail_wave_stop {
                return Err(CoreError::Script("wave stop failed".to_owned()));
            }

            Ok(())
        }

        async fn invoke_plugin_hook(
            &self,
            plugin_id: &str,
            hook: &str,
            payload: &Value,
        ) -> Result<(), CoreError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("hook:{plugin_id}:{hook}:{payload}"));
            Ok(())
        }
    }

    #[tokio::test]
    async fn executes_compiled_steps_in_order() {
        let actions = Arc::new(RecordingActions::default());
        let engine = ScriptExecutionEngine::new(actions.clone());
        let script = compile_script(ScriptDocument {
            id: "script.demo".to_owned(),
            version: 1,
            steps: vec![
                ScriptStep::Wait { duration_ms: 5 },
                ScriptStep::DeviceStatus {
                    device_id: "coyote-v3".to_owned(),
                },
                ScriptStep::WaveStop {
                    device_id: "coyote-v3".to_owned(),
                },
                ScriptStep::PluginHook {
                    plugin_id: "plugin.demo".to_owned(),
                    hook: "afterStop".to_owned(),
                    payload: json!({ "ok": true }),
                },
            ],
        });

        let report = engine.execute(&script).await.unwrap();

        assert_eq!(report.script_id, "script.demo");
        assert_eq!(
            report.steps,
            vec![
                ScriptStepExecution {
                    index: 0,
                    kind: ScriptStepKind::Wait,
                },
                ScriptStepExecution {
                    index: 1,
                    kind: ScriptStepKind::DeviceStatus,
                },
                ScriptStepExecution {
                    index: 2,
                    kind: ScriptStepKind::WaveStop,
                },
                ScriptStepExecution {
                    index: 3,
                    kind: ScriptStepKind::PluginHook,
                },
            ]
        );
        assert_eq!(
            *actions.calls.lock().unwrap(),
            vec![
                "wait:5",
                "status:coyote-v3",
                "stop:coyote-v3",
                "hook:plugin.demo:afterStop:{\"ok\":true}",
            ]
        );
    }

    #[tokio::test]
    async fn stops_on_first_failed_step() {
        let actions = Arc::new(RecordingActions {
            fail_wave_stop: true,
            ..RecordingActions::default()
        });
        let engine = ScriptExecutionEngine::new(actions.clone());
        let script = compile_script(ScriptDocument {
            id: "script.demo".to_owned(),
            version: 1,
            steps: vec![
                ScriptStep::Wait { duration_ms: 5 },
                ScriptStep::WaveStop {
                    device_id: "coyote-v3".to_owned(),
                },
                ScriptStep::Wait { duration_ms: 10 },
            ],
        });

        let error = engine.execute(&script).await.unwrap_err();

        assert!(matches!(error, CoreError::Script(_)));
        assert_eq!(
            *actions.calls.lock().unwrap(),
            vec!["wait:5", "stop:coyote-v3"]
        );
    }

    fn compile_script(document: ScriptDocument) -> CompiledScript {
        ScriptCompiler::default().compile(document).unwrap()
    }
}
