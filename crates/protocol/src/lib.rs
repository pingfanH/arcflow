#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Bluetooth protocol frames and helpers for ArcFlow-compatible devices.
//!
//! This crate deliberately stops at protocol bytes. Bluetooth discovery,
//! connection management, retries, logging, and device safety policy belong in
//! higher-level Rust crates.

pub mod coyote;
pub mod error;

pub use error::ProtocolError;
