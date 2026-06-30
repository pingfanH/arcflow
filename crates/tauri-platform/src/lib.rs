#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Tauri 2 platform adapters shared by ArcFlow desktop and mobile shells.

mod ble;

pub use ble::{TauriBleDiscoveryController, TauriBleDiscoveryState};
