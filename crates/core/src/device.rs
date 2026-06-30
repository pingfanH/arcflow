//! Device identity and status types.

use arcflow_protocol::coyote::v3::StrengthModes;
use arcflow_wave::CoyoteV3Window;

/// Stable ArcFlow device identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeviceId(String);

impl DeviceId {
    /// Constructs a device id.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the device id as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Device model known to ArcFlow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceModel {
    /// DG-LAB Coyote V2 pulse host.
    CoyoteV2,
    /// DG-LAB Coyote V3 pulse host.
    CoyoteV3,
    /// Device model that is not recognized yet.
    Unknown(String),
}

/// Current device status exposed to UI, plugins, and external-control clients.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceStatus {
    /// Device id.
    pub id: DeviceId,
    /// Device model.
    pub model: DeviceModel,
    /// Battery percentage if known.
    pub battery_percent: Option<u8>,
    /// Whether the device is currently connected.
    pub connected: bool,
}

/// Current platform BLE Adapter state as seen by Rust Core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BleAdapterStatus {
    /// No platform BLE Adapter has been attached yet.
    Unsupported,
    /// BLE Adapter is ready for scanning.
    Ready,
    /// The operating system denied BLE access.
    PermissionDenied,
    /// BLE radio is powered off.
    PoweredOff,
}

impl BleAdapterStatus {
    /// Returns the stable wire string for UI and plugin-facing DTOs.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::Ready => "ready",
            Self::PermissionDenied => "permissionDenied",
            Self::PoweredOff => "poweredOff",
        }
    }
}

/// Result of one device scan request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceScanResult {
    /// BLE Adapter status at scan time.
    pub adapter_status: BleAdapterStatus,
    /// Devices discovered by the scan.
    pub devices: Vec<DeviceStatus>,
}

impl DeviceScanResult {
    /// Constructs a scan result.
    #[must_use]
    pub fn new(adapter_status: BleAdapterStatus, devices: Vec<DeviceStatus>) -> Self {
        Self {
            adapter_status,
            devices,
        }
    }
}

/// Result of a stop-all-output request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopOutputResult {
    /// Device ids for which output was stopped.
    pub stopped_devices: Vec<DeviceId>,
}

impl StopOutputResult {
    /// Constructs a stop-output result.
    #[must_use]
    pub fn new(stopped_devices: Vec<DeviceId>) -> Self {
        Self { stopped_devices }
    }
}

/// Output request for one Coyote V3 100ms window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoyoteV3OutputRequest {
    /// Target device id.
    pub device_id: DeviceId,
    /// Wave window to write.
    pub window: CoyoteV3Window,
    /// Coyote V3 B0 sequence number.
    pub sequence: u8,
    /// Strength modes for channel A and B.
    pub strength_modes: StrengthModes,
    /// Channel A strength command value.
    pub a_strength: u8,
    /// Channel B strength command value.
    pub b_strength: u8,
}

impl CoyoteV3OutputRequest {
    /// Constructs a Coyote V3 output request.
    #[must_use]
    pub fn new(
        device_id: DeviceId,
        window: CoyoteV3Window,
        sequence: u8,
        strength_modes: StrengthModes,
        a_strength: u8,
        b_strength: u8,
    ) -> Self {
        Self {
            device_id,
            window,
            sequence,
            strength_modes,
            a_strength,
            b_strength,
        }
    }
}

/// Result of submitting one output window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitOutputResult {
    /// Device id that received the request.
    pub device_id: DeviceId,
    /// Whether the controller accepted the request.
    pub accepted: bool,
}

impl SubmitOutputResult {
    /// Constructs a submit-output result.
    #[must_use]
    pub fn new(device_id: DeviceId, accepted: bool) -> Self {
        Self {
            device_id,
            accepted,
        }
    }
}
