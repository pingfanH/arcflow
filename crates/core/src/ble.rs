//! Bluetooth transport Interface for Rust Core.

use async_trait::async_trait;

use crate::CoreError;

/// BLE characteristic known to ArcFlow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BleCharacteristic {
    /// Coyote V3 write characteristic, UUID 0x150A under service 0x180C.
    CoyoteV3Write,
    /// Coyote V3 notify characteristic, UUID 0x150B under service 0x180C.
    CoyoteV3Notify,
    /// Coyote battery characteristic, UUID 0x1500 under service 0x180A.
    CoyoteBattery,
    /// Coyote V2 AB strength characteristic, UUID 0x1504 under service 0x180B.
    CoyoteV2PwmAb2,
    /// Coyote V2 A waveform characteristic, UUID 0x1505 under service 0x180B.
    CoyoteV2PwmA34,
    /// Coyote V2 B waveform characteristic, UUID 0x1506 under service 0x180B.
    CoyoteV2PwmB34,
}

/// BLE write request produced by Rust Core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BleWrite {
    /// Target characteristic.
    pub characteristic: BleCharacteristic,
    /// Payload bytes.
    pub payload: Vec<u8>,
}

impl BleWrite {
    /// Constructs a BLE write request.
    #[must_use]
    pub fn new(characteristic: BleCharacteristic, payload: impl Into<Vec<u8>>) -> Self {
        Self {
            characteristic,
            payload: payload.into(),
        }
    }
}

/// BLE notification received by Rust Core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BleNotification {
    /// Source characteristic.
    pub characteristic: BleCharacteristic,
    /// Notification payload bytes.
    pub payload: Vec<u8>,
}

/// Asynchronous BLE transport Adapter seam.
///
/// Platform-specific desktop and mobile implementations must satisfy this
/// Interface. React and plugins never call it directly.
#[async_trait]
pub trait BleTransport {
    /// Write bytes to a known BLE characteristic.
    async fn write(&self, write: BleWrite) -> Result<(), CoreError>;

    /// Enable notifications for a known BLE characteristic.
    async fn subscribe(&self, characteristic: BleCharacteristic) -> Result<(), CoreError>;
}
