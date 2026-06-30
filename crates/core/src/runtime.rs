//! High-level ArcFlow Core runtime facade.

use arcflow_external_control::{ClientSession, JsonRpcRequest};
use serde_json::{json, Value};

use crate::{
    authorized_core_command_from_external_request, BleAdapterStatus, CoreCommand, CoreError,
    DeviceId, DeviceScanResult, DeviceStatus, SafetyLimits, StopOutputResult,
};

/// Application core facade shared by desktop, mobile, plugins, and external control.
#[derive(Debug, Clone)]
pub struct ArcFlowCore {
    safety_limits: SafetyLimits,
}

impl ArcFlowCore {
    /// Constructs a core runtime with explicit safety limits.
    #[must_use]
    pub fn new(safety_limits: SafetyLimits) -> Self {
        Self { safety_limits }
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
    #[must_use]
    pub fn scan_devices(&self) -> DeviceScanResult {
        DeviceScanResult::new(BleAdapterStatus::Unsupported, Vec::new())
    }

    /// Stops all active output sessions known by Core.
    #[must_use]
    pub fn stop_all_output(&self) -> StopOutputResult {
        StopOutputResult::new(Vec::new())
    }

    /// Executes an external-control request after capability authorization.
    pub fn execute_external_request(
        &self,
        session: &ClientSession,
        request: &JsonRpcRequest,
    ) -> Result<Value, CoreError> {
        let command = authorized_core_command_from_external_request(session, request)?;

        self.execute_command(command)
    }

    /// Executes one already-authorized core command.
    pub fn execute_command(&self, command: CoreCommand) -> Result<Value, CoreError> {
        match command {
            CoreCommand::ReadDeviceStatus { device_id } => {
                Ok(self.device_status_payload(&device_id))
            }
            CoreCommand::SubmitCoyoteV3Window { device_id, .. } => Ok(json!({
                "accepted": true,
                "command": "wave.submitWindow",
                "deviceId": device_id.as_str(),
            })),
            CoreCommand::StopOutput { device_id } => {
                let result = self.stop_all_output();
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
            CoreCommand::RunScript { script_id } => Ok(json!({
                "accepted": true,
                "command": "script.run",
                "scriptId": script_id,
                "queued": false,
            })),
        }
    }

    fn device_status_payload(&self, device_id: &DeviceId) -> Value {
        let scan = self.scan_devices();
        let matching_device = scan
            .devices
            .iter()
            .find(|device| device.id.as_str() == device_id.as_str());

        match matching_device {
            Some(device) => device_status_payload(device, scan.adapter_status),
            None => json!({
                "deviceId": device_id.as_str(),
                "connected": false,
                "adapterStatus": scan.adapter_status.as_str(),
            }),
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

    #[test]
    fn scan_devices_reports_missing_platform_adapter() {
        let core = ArcFlowCore::default();
        let scan = core.scan_devices();

        assert_eq!(scan.adapter_status, BleAdapterStatus::Unsupported);
        assert!(scan.devices.is_empty());
    }

    #[test]
    fn stop_all_output_is_safe_with_no_sessions() {
        let core = ArcFlowCore::default();
        let result = core.stop_all_output();

        assert!(result.stopped_devices.is_empty());
    }

    #[test]
    fn executes_external_device_status_request() {
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

        let result = core.execute_external_request(&session, &request).unwrap();

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
