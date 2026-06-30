#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Plugin manifest and capability model for ArcFlow.
//!
//! The runtime executes WASM and JavaScript plugins through a stable manifest
//! and capability interface. Device access still flows through Rust Core.

pub mod capability;
pub mod manifest;

pub use capability::Capability;
pub use manifest::{ManifestError, PluginManifest, RuntimeKind};
