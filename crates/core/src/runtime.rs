//! High-level ArcFlow Core runtime facade.

use std::{fmt, sync::Arc};

use arcflow_external_control::{ClientSession, JsonRpcRequest};
use arcflow_script::SCRIPT_VERSION;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::{
    authorized_core_command_from_external_request, BleAdapterStatus, CoreCommand, CoreError,
    CoyoteV3OutputRequest, DeviceId, DeviceScanResult, DeviceStatus, SafetyLimits,
    StopOutputResult, SubmitOutputResult,
};

/// Discovery controller used by Core to scan for supported devices.
#[async_trait]
pub trait DeviceDiscoveryController: fmt::Debug + Send + Sync {
    /// Scans for supported devices through the platform BLE Adapter.
    async fn scan_devices(&self) -> Result<DeviceScanResult, CoreError>;
}

/// Discovery controller used before a platform BLE Adapter is attached.
#[derive(Debug, Default)]
pub struct NoopDeviceDiscoveryController;

#[async_trait]
impl DeviceDiscoveryController for NoopDeviceDiscoveryController {
    async fn scan_devices(&self) -> Result<DeviceScanResult, CoreError> {
        Ok(DeviceScanResult::new(
            BleAdapterStatus::Unsupported,
            Vec::new(),
        ))
    }
}

/// Output controller used by Core to stop active device sessions.
pub trait DeviceOutputController: fmt::Debug + Send + Sync {
    /// Marks a device as active for output writes.
    fn attach_output_device(&self, _device_id: DeviceId) -> Result<(), CoreError> {
        Err(CoreError::Transport(
            "device output controller does not support device attachment".to_owned(),
        ))
    }

    /// Removes a device from active output writes.
    fn detach_output_device(&self, _device_id: &DeviceId) -> Result<(), CoreError> {
        Ok(())
    }

    /// Lists devices currently active for output writes.
    fn active_output_devices(&self) -> Vec<DeviceId> {
        Vec::new()
    }

    /// Submits one Coyote V3 wave window to the output controller.
    fn submit_coyote_v3_window(
        &self,
        request: CoyoteV3OutputRequest,
    ) -> Result<SubmitOutputResult, CoreError>;

    /// Stops output on every active session known to the controller.
    fn stop_all_output(&self) -> Result<StopOutputResult, CoreError>;
}

/// Script runner used by Core to queue or execute compiled scripts.
pub trait ScriptRunner: fmt::Debug + Send + Sync {
    /// Runs a script by id.
    fn run_script(&self, script_id: &str) -> Result<ScriptRunResult, CoreError>;
}

/// Script runner used before the real script engine is attached.
#[derive(Debug, Default)]
pub struct NoopScriptRunner;

impl ScriptRunner for NoopScriptRunner {
    fn run_script(&self, script_id: &str) -> Result<ScriptRunResult, CoreError> {
        Ok(ScriptRunResult::new(script_id, false))
    }
}

/// Result of a script run request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptRunResult {
    script_id: String,
    queued: bool,
}

impl ScriptRunResult {
    /// Constructs a script run result.
    #[must_use]
    pub fn new(script_id: impl Into<String>, queued: bool) -> Self {
        Self {
            script_id: script_id.into(),
            queued,
        }
    }

    /// Returns the script id.
    #[must_use]
    pub fn script_id(&self) -> &str {
        &self.script_id
    }

    /// Returns whether the script was queued for asynchronous execution.
    #[must_use]
    pub fn queued(&self) -> bool {
        self.queued
    }
}

/// Output controller used before a platform BLE Adapter is attached.
#[derive(Debug, Default)]
pub struct NoopDeviceOutputController;

impl DeviceOutputController for NoopDeviceOutputController {
    fn attach_output_device(&self, _device_id: DeviceId) -> Result<(), CoreError> {
        Err(CoreError::Transport(
            "no device output controller attached".to_owned(),
        ))
    }

    fn submit_coyote_v3_window(
        &self,
        _request: CoyoteV3OutputRequest,
    ) -> Result<SubmitOutputResult, CoreError> {
        Err(CoreError::Transport(
            "no device output controller attached".to_owned(),
        ))
    }

    fn stop_all_output(&self) -> Result<StopOutputResult, CoreError> {
        Ok(StopOutputResult::new(Vec::new()))
    }
}

/// Application core facade shared by desktop, mobile, plugins, and external control.
#[derive(Debug, Clone)]
pub struct ArcFlowCore {
    safety_limits: SafetyLimits,
    discovery_controller: Arc<dyn DeviceDiscoveryController>,
    output_controller: Arc<dyn DeviceOutputController>,
    script_runner: Arc<dyn ScriptRunner>,
}

impl ArcFlowCore {
    /// Constructs a core runtime with explicit safety limits.
    #[must_use]
    pub fn new(safety_limits: SafetyLimits) -> Self {
        Self::with_controllers(
            safety_limits,
            Arc::new(NoopDeviceDiscoveryController),
            Arc::new(NoopDeviceOutputController),
            Arc::new(NoopScriptRunner),
        )
    }

    /// Constructs a core runtime with an explicit output controller.
    #[must_use]
    pub fn with_output_controller(
        safety_limits: SafetyLimits,
        output_controller: Arc<dyn DeviceOutputController>,
    ) -> Self {
        Self::with_controllers(
            safety_limits,
            Arc::new(NoopDeviceDiscoveryController),
            output_controller,
            Arc::new(NoopScriptRunner),
        )
    }

    /// Constructs a core runtime with an explicit script runner.
    #[must_use]
    pub fn with_script_runner(
        safety_limits: SafetyLimits,
        script_runner: Arc<dyn ScriptRunner>,
    ) -> Self {
        Self::with_controllers(
            safety_limits,
            Arc::new(NoopDeviceDiscoveryController),
            Arc::new(NoopDeviceOutputController),
            script_runner,
        )
    }

    /// Constructs a core runtime with explicit discovery and output controllers.
    #[must_use]
    pub fn with_controllers(
        safety_limits: SafetyLimits,
        discovery_controller: Arc<dyn DeviceDiscoveryController>,
        output_controller: Arc<dyn DeviceOutputController>,
        script_runner: Arc<dyn ScriptRunner>,
    ) -> Self {
        Self {
            safety_limits,
            discovery_controller,
            output_controller,
            script_runner,
        }
    }

    /// Returns the active safety limits.
    #[must_use]
    pub fn safety_limits(&self) -> SafetyLimits {
        self.safety_limits
    }

    /// Scans for supported devices through the platform BLE Adapter.
    ///
    /// The current scaffold has not attached a platform Adapter yet, so it
    /// returns an explicit unsupported Adapter status instead of reaching into
    /// React or platform APIs.
    pub async fn scan_devices(&self) -> Result<DeviceScanResult, CoreError> {
        self.discovery_controller.scan_devices().await
    }

    /// Stops all active output sessions known by Core.
    pub fn stop_all_output(&self) -> Result<StopOutputResult, CoreError> {
        self.output_controller.stop_all_output()
    }

    /// Marks a device as active for output writes and returns all active devices.
    pub fn attach_output_device(&self, device_id: DeviceId) -> Result<Vec<DeviceId>, CoreError> {
        self.output_controller.attach_output_device(device_id)?;
        Ok(self.output_controller.active_output_devices())
    }

    /// Removes a device from output writes and returns all active devices.
    pub fn detach_output_device(&self, device_id: &DeviceId) -> Result<Vec<DeviceId>, CoreError> {
        self.output_controller.detach_output_device(device_id)?;
        Ok(self.output_controller.active_output_devices())
    }

    /// Lists devices currently active for output writes.
    #[must_use]
    pub fn active_output_devices(&self) -> Vec<DeviceId> {
        self.output_controller.active_output_devices()
    }

    /// Submits one Coyote V3 output window through the active output controller.
    pub fn submit_coyote_v3_window(
        &self,
        request: CoyoteV3OutputRequest,
    ) -> Result<SubmitOutputResult, CoreError> {
        self.output_controller.submit_coyote_v3_window(request)
    }

    /// Runs a script through the active script runner.
    pub fn run_script(&self, script_id: &str) -> Result<ScriptRunResult, CoreError> {
        self.script_runner.run_script(script_id)
    }

    /// Executes an external-control request after capability authorization.
    pub async fn execute_external_request(
        &self,
        session: &ClientSession,
        request: &JsonRpcRequest,
    ) -> Result<Value, CoreError> {
        let command = authorized_core_command_from_external_request(session, request)?;

        self.execute_command(command).await
    }

    /// Executes one already-authorized core command.
    pub async fn execute_command(&self, command: CoreCommand) -> Result<Value, CoreError> {
        match command {
            CoreCommand::ReadDeviceStatus { device_id } => {
                self.device_status_payload(&device_id).await
            }
            CoreCommand::SubmitCoyoteV3Window {
                device_id,
                window,
                sequence,
                strength_modes,
                a_strength,
                b_strength,
            } => {
                let result = self.submit_coyote_v3_window(CoyoteV3OutputRequest::new(
                    device_id,
                    window,
                    sequence,
                    strength_modes,
                    a_strength,
                    b_strength,
                ))?;

                Ok(json!({
                    "accepted": result.accepted,
                    "command": "wave.submitWindow",
                    "deviceId": result.device_id.as_str(),
                }))
            }
            CoreCommand::StopOutput { device_id } => {
                let result = self.stop_all_output()?;
                let stopped = result
                    .stopped_devices
                    .iter()
                    .any(|stopped_id| stopped_id == &device_id);

                Ok(json!({
                    "accepted": true,
                    "command": "wave.stop",
                    "deviceId": device_id.as_str(),
                    "stopped": stopped,
                }))
            }
            CoreCommand::RunScript { script_id } => {
                let result = self.run_script(&script_id)?;

                Ok(json!({
                    "accepted": true,
                    "command": "script.run",
                    "scriptId": result.script_id(),
                    "scriptVersion": SCRIPT_VERSION,
                    "queued": result.queued(),
                }))
            }
        }
    }

    async fn device_status_payload(&self, device_id: &DeviceId) -> Result<Value, CoreError> {
        let scan = self.scan_devices().await?;
        let matching_device = scan
            .devices
            .iter()
            .find(|device| device.id.as_str() == device_id.as_str());

        match matching_device {
            Some(device) => Ok(device_status_payload(device, scan.adapter_status)),
            None => Ok(json!({
                "deviceId": device_id.as_str(),
                "connected": false,
                "adapterStatus": scan.adapter_status.as_str(),
            })),
        }
    }
}

impl Default for ArcFlowCore {
    fn default() -> Self {
        Self::new(SafetyLimits::conservative())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcflow_external_control::{ClientHello, GatewayPolicy, RequestId, PROTOCOL_VERSION};
    use arcflow_plugin_runtime::Capability;
    use arcflow_protocol::coyote::v3::{StrengthMode, StrengthModes};
    use arcflow_wave::{ChannelOutput, ChannelWindow, CoyoteV3Window, WavePoint};
    use std::sync::Mutex;

    #[tokio::test]
    async fn scan_devices_reports_missing_platform_adapter() {
        let core = ArcFlowCore::default();
        let scan = core.scan_devices().await.unwrap();

        assert_eq!(scan.adapter_status, BleAdapterStatus::Unsupported);
        assert!(scan.devices.is_empty());
    }

    #[tokio::test]
    async fn scan_devices_uses_discovery_controller() {
        #[derive(Debug)]
        struct FakeDiscoveryController;

        #[async_trait]
        impl DeviceDiscoveryController for FakeDiscoveryController {
            async fn scan_devices(&self) -> Result<DeviceScanResult, CoreError> {
                Ok(DeviceScanResult::new(
                    BleAdapterStatus::Ready,
                    vec![DeviceStatus {
                        id: DeviceId::new("coyote-v3"),
                        model: crate::DeviceModel::CoyoteV3,
                        battery_percent: Some(87),
                        connected: true,
                    }],
                ))
            }
        }

        let core = ArcFlowCore::with_controllers(
            SafetyLimits::conservative(),
            Arc::new(FakeDiscoveryController),
            Arc::new(NoopDeviceOutputController),
            Arc::new(NoopScriptRunner),
        );
        let scan = core.scan_devices().await.unwrap();

        assert_eq!(scan.adapter_status, BleAdapterStatus::Ready);
        assert_eq!(scan.devices[0].id, DeviceId::new("coyote-v3"));
    }

    #[test]
    fn stop_all_output_is_safe_with_no_sessions() {
        let core = ArcFlowCore::default();
        let result = core.stop_all_output().unwrap();

        assert!(result.stopped_devices.is_empty());
    }

    #[test]
    fn stop_all_output_uses_output_controller() {
        #[derive(Debug)]
        struct FakeOutputController;

        impl DeviceOutputController for FakeOutputController {
            fn submit_coyote_v3_window(
                &self,
                _request: CoyoteV3OutputRequest,
            ) -> Result<SubmitOutputResult, CoreError> {
                Ok(SubmitOutputResult::new(DeviceId::new("coyote-v3"), true))
            }

            fn stop_all_output(&self) -> Result<StopOutputResult, CoreError> {
                Ok(StopOutputResult::new(vec![DeviceId::new("coyote-v3")]))
            }
        }

        let core = ArcFlowCore::with_output_controller(
            SafetyLimits::conservative(),
            Arc::new(FakeOutputController),
        );
        let result = core.stop_all_output().unwrap();

        assert_eq!(result.stopped_devices, vec![DeviceId::new("coyote-v3")]);
    }

    #[test]
    fn output_device_activation_uses_output_controller() {
        #[derive(Debug, Default)]
        struct FakeOutputController {
            active_devices: Mutex<Vec<DeviceId>>,
        }

        impl DeviceOutputController for FakeOutputController {
            fn attach_output_device(&self, device_id: DeviceId) -> Result<(), CoreError> {
                let mut active_devices = self.active_devices.lock().unwrap();
                if !active_devices.contains(&device_id) {
                    active_devices.push(device_id);
                }
                Ok(())
            }

            fn detach_output_device(&self, device_id: &DeviceId) -> Result<(), CoreError> {
                self.active_devices
                    .lock()
                    .unwrap()
                    .retain(|active_device_id| active_device_id != device_id);
                Ok(())
            }

            fn active_output_devices(&self) -> Vec<DeviceId> {
                self.active_devices.lock().unwrap().clone()
            }

            fn submit_coyote_v3_window(
                &self,
                request: CoyoteV3OutputRequest,
            ) -> Result<SubmitOutputResult, CoreError> {
                Ok(SubmitOutputResult::new(request.device_id, true))
            }

            fn stop_all_output(&self) -> Result<StopOutputResult, CoreError> {
                Ok(StopOutputResult::new(Vec::new()))
            }
        }

        let core = ArcFlowCore::with_output_controller(
            SafetyLimits::conservative(),
            Arc::new(FakeOutputController::default()),
        );

        let active_devices = core
            .attach_output_device(DeviceId::new("coyote-v3"))
            .unwrap();
        assert_eq!(active_devices, vec![DeviceId::new("coyote-v3")]);
        assert_eq!(
            core.active_output_devices(),
            vec![DeviceId::new("coyote-v3")]
        );

        let active_devices = core
            .detach_output_device(&DeviceId::new("coyote-v3"))
            .unwrap();
        assert!(active_devices.is_empty());
    }

    #[tokio::test]
    async fn submit_coyote_v3_window_uses_output_controller() {
        #[derive(Debug, Default)]
        struct FakeOutputController {
            requests: Mutex<Vec<CoyoteV3OutputRequest>>,
        }

        impl DeviceOutputController for FakeOutputController {
            fn submit_coyote_v3_window(
                &self,
                request: CoyoteV3OutputRequest,
            ) -> Result<SubmitOutputResult, CoreError> {
                let device_id = request.device_id.clone();
                self.requests.lock().unwrap().push(request);
                Ok(SubmitOutputResult::new(device_id, true))
            }

            fn stop_all_output(&self) -> Result<StopOutputResult, CoreError> {
                Ok(StopOutputResult::new(Vec::new()))
            }
        }

        let output_controller = Arc::new(FakeOutputController::default());
        let core = ArcFlowCore::with_output_controller(
            SafetyLimits::conservative(),
            output_controller.clone(),
        );
        let command = CoreCommand::SubmitCoyoteV3Window {
            device_id: DeviceId::new("coyote-v3"),
            window: safe_window(),
            sequence: 3,
            strength_modes: StrengthModes::new(StrengthMode::Unchanged, StrengthMode::Unchanged),
            a_strength: 0,
            b_strength: 0,
        };

        let result = core.execute_command(command).await.unwrap();

        assert_eq!(
            result,
            json!({
                "accepted": true,
                "command": "wave.submitWindow",
                "deviceId": "coyote-v3",
            })
        );
        assert_eq!(output_controller.requests.lock().unwrap()[0].sequence, 3);
    }

    fn safe_window() -> CoyoteV3Window {
        let channel = ChannelWindow::new([
            WavePoint::new(10, 0).unwrap(),
            WavePoint::new(10, 10).unwrap(),
            WavePoint::new(10, 20).unwrap(),
            WavePoint::new(10, 30).unwrap(),
        ]);

        CoyoteV3Window::new(ChannelOutput::Window(channel), ChannelOutput::Disabled)
    }

    #[tokio::test]
    async fn executes_script_run_through_runner() {
        #[derive(Debug)]
        struct FakeScriptRunner;

        impl ScriptRunner for FakeScriptRunner {
            fn run_script(&self, script_id: &str) -> Result<ScriptRunResult, CoreError> {
                Ok(ScriptRunResult::new(script_id, true))
            }
        }

        let core = ArcFlowCore::with_script_runner(
            SafetyLimits::conservative(),
            Arc::new(FakeScriptRunner),
        );

        let result = core
            .execute_command(CoreCommand::RunScript {
                script_id: "script.demo".to_owned(),
            })
            .await
            .unwrap();

        assert_eq!(
            result,
            json!({
                "accepted": true,
                "command": "script.run",
                "scriptId": "script.demo",
                "scriptVersion": SCRIPT_VERSION,
                "queued": true,
            })
        );
    }

    #[tokio::test]
    async fn executes_external_device_status_request() {
        let core = ArcFlowCore::default();
        let session = GatewayPolicy::local_default()
            .accept(ClientHello {
                client_name: "Status Panel".to_owned(),
                protocol_version: PROTOCOL_VERSION,
                requested_capabilities: vec![Capability::DeviceRead],
            })
            .unwrap();
        let request = JsonRpcRequest::new(
            RequestId::Number(1),
            "device.status",
            Some(json!({ "deviceId": "coyote-v3" })),
        );

        let result = core
            .execute_external_request(&session, &request)
            .await
            .unwrap();

        assert_eq!(
            result,
            json!({
                "deviceId": "coyote-v3",
                "connected": false,
                "adapterStatus": "unsupported",
            })
        );
    }
}

fn device_status_payload(device: &DeviceStatus, adapter_status: BleAdapterStatus) -> Value {
    json!({
        "deviceId": device.id.as_str(),
        "connected": device.connected,
        "adapterStatus": adapter_status.as_str(),
        "batteryPercent": device.battery_percent,
    })
}
