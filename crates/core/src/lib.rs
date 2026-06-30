#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Core orchestration types for ArcFlow.
//!
//! This crate is the future owner of Bluetooth sessions, device safety policy,
//! plugin dispatch, external-control dispatch, storage access, and Tauri IPC
//! commands. The first slice defines stable types used across those seams.

pub mod ble;
pub mod command;
pub mod coyote;
pub mod device;
pub mod error;
pub mod external;
pub mod plugin_api;
pub mod runtime;
pub mod safety;
pub mod session;

pub use ble::{
    BleAdvertisement, BleCharacteristic, BleDiscovery, BleNotification, BleTransport, BleWrite,
    COYOTE_BATTERY_SERVICE_UUID, COYOTE_V2_SERVICE_UUID, COYOTE_V3_SERVICE_UUID,
};
pub use command::CoreCommand;
pub use coyote::CoyoteV3CommandBuilder;
pub use device::{
    BleAdapterStatus, DeviceId, DeviceModel, DeviceScanResult, DeviceStatus, StopOutputResult,
};
pub use error::CoreError;
pub use external::{
    authorized_core_command_from_external_request, core_command_from_external_request,
    required_capability_for_external_request,
};
pub use plugin_api::{PluginApi, PluginApiError};
pub use runtime::{
    ArcFlowCore, DeviceDiscoveryController, DeviceOutputController, NoopDeviceDiscoveryController,
    NoopDeviceOutputController,
};
pub use safety::SafetyLimits;
pub use session::{CoyoteV3StrengthStatus, DeviceSession};
