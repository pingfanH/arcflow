//! Safe script execution orchestration.

use std::{fmt, sync::Arc, time::Duration};

use arcflow_script::{CompiledScript, ScriptStep};
use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::{CoreError, DeviceDiscoveryController, DeviceOutputController};

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

/// Script action executor backed by Core-owned device controllers.
#[derive(Debug, Clone)]
pub struct CoreScriptActionExecutor {
    discovery_controller: Arc<dyn DeviceDiscoveryController>,
    output_controller: Arc<dyn DeviceOutputController>,
}

impl CoreScriptActionExecutor {
    /// Constructs a Core-backed script action executor.
    #[must_use]
    pub fn new(
        discovery_controller: Arc<dyn DeviceDiscoveryController>,
        output_controller: Arc<dyn DeviceOutputController>,
    ) -> Self {
        Self {
            discovery_controller,
            output_controller,
        }
    }
}

#[async_trait]
impl ScriptActionExecutor for CoreScriptActionExecutor {
    async fn wait(&self, _duration: Duration) -> Result<(), CoreError> {
        Ok(())
    }

    async fn read_device_status(&self, _device_id: &str) -> Result<(), CoreError> {
        self.discovery_controller.scan_devices().await?;
        Ok(())
    }

    async fn stop_wave(&self, _device_id: &str) -> Result<(), CoreError> {
        self.output_controller.stop_all_output()?;
        Ok(())
    }

    async fn invoke_plugin_hook(
        &self,
        plugin_id: &str,
        hook: &str,
        _payload: &Value,
    ) -> Result<(), CoreError> {
        Err(CoreError::Script(format!(
            "plugin hook `{hook}` for plugin `{plugin_id}` is not attached"
        )))
    }
}

/// Queue backed by an asynchronous script worker task.
#[derive(Clone)]
pub struct ScriptWorkerQueue {
    sender: mpsc::UnboundedSender<CompiledScript>,
}

impl ScriptWorkerQueue {
    /// Spawns a script worker and returns its queue handle.
    #[must_use]
    pub fn spawn(actions: Arc<dyn ScriptActionExecutor>) -> Self {
        Self::spawn_with_events(actions, None)
    }

    /// Spawns a script worker with an optional event sink for tests or telemetry.
    #[must_use]
    pub fn spawn_with_event_sink(
        actions: Arc<dyn ScriptActionExecutor>,
        event_sink: mpsc::UnboundedSender<ScriptWorkerEvent>,
    ) -> Self {
        Self::spawn_with_events(actions, Some(event_sink))
    }

    fn spawn_with_events(
        actions: Arc<dyn ScriptActionExecutor>,
        event_sink: Option<mpsc::UnboundedSender<ScriptWorkerEvent>>,
    ) -> Self {
        let (sender, mut receiver) = mpsc::unbounded_channel::<CompiledScript>();
        let engine = ScriptExecutionEngine::new(actions);

        tokio::spawn(async move {
            while let Some(script) = receiver.recv().await {
                let script_id = script.id().to_owned();
                let event = match engine.execute(&script).await {
                    Ok(report) => ScriptWorkerEvent::Completed(report),
                    Err(error) => ScriptWorkerEvent::Failed {
                        script_id,
                        error: error.to_string(),
                    },
                };

                if let Some(event_sink) = &event_sink {
                    let _ = event_sink.send(event);
                }
            }
        });

        Self { sender }
    }
}

impl fmt::Debug for ScriptWorkerQueue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScriptWorkerQueue").finish_non_exhaustive()
    }
}

impl CompiledScriptQueue for ScriptWorkerQueue {
    fn enqueue(&self, script: CompiledScript) -> Result<(), CoreError> {
        self.sender
            .send(script)
            .map_err(|_| CoreError::Script("script worker queue is closed".to_owned()))
    }
}

/// Event emitted by the asynchronous script worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ScriptWorkerEvent {
    /// Script finished successfully.
    Completed(
        /// Execution report.
        ScriptExecutionReport,
    ),
    /// Script failed during execution.
    Failed {
        /// Failed script id.
        script_id: String,
        /// Rendered error message.
        error: String,
    },
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
    use async_trait::async_trait;
    use serde_json::json;
    use tokio::time::{timeout, Duration as TokioDuration};

    use super::*;
    use crate::{
        BleAdapterStatus, CoyoteV3OutputRequest, DeviceId, DeviceModel, DeviceScanResult,
        DeviceStatus, StopOutputResult, SubmitOutputResult,
    };

    #[derive(Debug, Default)]
    struct RecordingActions {
        calls: Mutex<Vec<String>>,
        fail_wave_stop: bool,
    }

    #[derive(Debug, Default)]
    struct FakeDiscoveryController {
        scans: Mutex<usize>,
    }

    #[async_trait]
    impl DeviceDiscoveryController for FakeDiscoveryController {
        async fn scan_devices(&self) -> Result<DeviceScanResult, CoreError> {
            *self.scans.lock().unwrap() += 1;
            Ok(DeviceScanResult::new(
                BleAdapterStatus::Ready,
                vec![DeviceStatus {
                    id: DeviceId::new("coyote-v3"),
                    model: DeviceModel::CoyoteV3,
                    battery_percent: Some(90),
                    connected: true,
                }],
            ))
        }
    }

    #[derive(Debug, Default)]
    struct FakeOutputController {
        stops: Mutex<usize>,
    }

    impl DeviceOutputController for FakeOutputController {
        fn submit_coyote_v3_window(
            &self,
            request: CoyoteV3OutputRequest,
        ) -> Result<SubmitOutputResult, CoreError> {
            Ok(SubmitOutputResult::new(request.device_id, true))
        }

        fn stop_all_output(&self) -> Result<StopOutputResult, CoreError> {
            *self.stops.lock().unwrap() += 1;
            Ok(StopOutputResult::new(vec![DeviceId::new("coyote-v3")]))
        }
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

    #[tokio::test]
    async fn worker_executes_queued_scripts() {
        let actions = Arc::new(RecordingActions::default());
        let (event_sender, mut events) = mpsc::unbounded_channel();
        let queue = ScriptWorkerQueue::spawn_with_event_sink(actions.clone(), event_sender);
        let script = compile_script(ScriptDocument {
            id: "script.worker".to_owned(),
            version: 1,
            steps: vec![ScriptStep::Wait { duration_ms: 5 }],
        });

        queue.enqueue(script).unwrap();

        let event = timeout(TokioDuration::from_secs(1), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            event,
            ScriptWorkerEvent::Completed(ScriptExecutionReport {
                script_id: "script.worker".to_owned(),
                steps: vec![ScriptStepExecution {
                    index: 0,
                    kind: ScriptStepKind::Wait,
                }],
            })
        );
        assert_eq!(*actions.calls.lock().unwrap(), vec!["wait:5"]);
    }

    #[tokio::test]
    async fn worker_reports_failed_scripts() {
        let actions = Arc::new(RecordingActions {
            fail_wave_stop: true,
            ..RecordingActions::default()
        });
        let (event_sender, mut events) = mpsc::unbounded_channel();
        let queue = ScriptWorkerQueue::spawn_with_event_sink(actions, event_sender);
        let script = compile_script(ScriptDocument {
            id: "script.worker".to_owned(),
            version: 1,
            steps: vec![ScriptStep::WaveStop {
                device_id: "coyote-v3".to_owned(),
            }],
        });

        queue.enqueue(script).unwrap();

        let event = timeout(TokioDuration::from_secs(1), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            event,
            ScriptWorkerEvent::Failed {
                script_id: "script.worker".to_owned(),
                error: "script error: wave stop failed".to_owned(),
            }
        );
    }

    #[tokio::test]
    async fn core_action_executor_uses_device_controllers() {
        let discovery = Arc::new(FakeDiscoveryController::default());
        let output = Arc::new(FakeOutputController::default());
        let actions = CoreScriptActionExecutor::new(discovery.clone(), output.clone());

        actions.read_device_status("coyote-v3").await.unwrap();
        actions.stop_wave("coyote-v3").await.unwrap();

        assert_eq!(*discovery.scans.lock().unwrap(), 1);
        assert_eq!(*output.stops.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn core_action_executor_rejects_unattached_plugin_hooks() {
        let discovery = Arc::new(FakeDiscoveryController::default());
        let output = Arc::new(FakeOutputController::default());
        let actions = CoreScriptActionExecutor::new(discovery, output);

        let error = actions
            .invoke_plugin_hook("plugin.demo", "afterStop", &json!({}))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("not attached"));
    }

    fn compile_script(document: ScriptDocument) -> CompiledScript {
        ScriptCompiler::default().compile(document).unwrap()
    }
}
