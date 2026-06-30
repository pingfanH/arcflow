#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Plugin manifest and capability model for ArcFlow.
//!
//! The runtime executes WASM and JavaScript plugins through a stable manifest
//! and capability interface. Device access still flows through Rust Core.

pub mod capability;
pub mod host;
pub mod manifest;
pub mod registry;
pub mod router;
pub mod runtime;
pub mod sandbox;

pub use capability::Capability;
pub use host::{PluginHost, PluginHostError};
pub use manifest::{ManifestError, PluginManifest, RuntimeKind};
pub use registry::{PluginRecord, PluginRegistry, PluginRegistryError};
pub use router::RuntimeRouter;
pub use runtime::{
    PluginAction, PluginInvocation, PluginOutput, RuntimeAdapter, RuntimeError, RuntimeHandle,
};
pub use sandbox::{SandboxPolicy, SandboxPolicyError, SandboxedRuntime, UnavailableRuntimeAdapter};
