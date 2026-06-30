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
pub mod safety;

pub use ble::{BleCharacteristic, BleNotification, BleTransport, BleWrite};
pub use command::CoreCommand;
pub use coyote::CoyoteV3CommandBuilder;
pub use device::{DeviceId, DeviceModel, DeviceStatus};
pub use error::CoreError;
pub use safety::SafetyLimits;
