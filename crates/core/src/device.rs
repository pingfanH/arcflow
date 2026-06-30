//! Device identity and status types.

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
