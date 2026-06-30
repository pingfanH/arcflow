//! Core command model.

use arcflow_protocol::coyote::v3::StrengthModes;
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
        /// Coyote V3 B0 sequence number.
        sequence: u8,
        /// Strength modes for channel A and B.
        strength_modes: StrengthModes,
        /// Channel A strength command value.
        a_strength: u8,
        /// Channel B strength command value.
        b_strength: u8,
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
