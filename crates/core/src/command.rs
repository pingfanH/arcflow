//! Core command model.

use arcflow_wave::CoyoteV3Window;

use crate::DeviceId;

/// Command entering Rust Core from UI, plugin runtime, or external control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreCommand {
    /// Read the latest device status.
    ReadDeviceStatus {
        /// Device id.
        device_id: DeviceId,
    },
    /// Submit one validated Coyote V3 wave window.
    SubmitCoyoteV3Window {
        /// Device id.
        device_id: DeviceId,
        /// Wave window.
        window: CoyoteV3Window,
    },
    /// Stop all output for a device.
    StopOutput {
        /// Device id.
        device_id: DeviceId,
    },
    /// Run a script by id.
    RunScript {
        /// Script id.
        script_id: String,
    },
}
