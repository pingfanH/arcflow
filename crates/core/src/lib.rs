#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Core orchestration types for ArcFlow.
//!
//! This crate is the future owner of Bluetooth sessions, device safety policy,
//! plugin dispatch, external-control dispatch, storage access, and Tauri IPC
//! commands. The first slice defines stable types used across those seams.

pub mod command;
pub mod device;
pub mod safety;

pub use command::CoreCommand;
pub use device::{DeviceId, DeviceModel, DeviceStatus};
pub use safety::SafetyLimits;
