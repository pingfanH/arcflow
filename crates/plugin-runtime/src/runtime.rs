//! Runtime Adapter Interface for WASM and JavaScript plugins.

use async_trait::async_trait;
use serde_json::Value;

use crate::{PluginManifest, RuntimeKind};

/// Handle returned by a runtime after loading a plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeHandle {
    plugin_id: String,
    runtime: RuntimeKind,
}

impl RuntimeHandle {
    /// Constructs a runtime handle.
    #[must_use]
    pub fn new(plugin_id: impl Into<String>, runtime: RuntimeKind) -> Self {
        Self {
            plugin_id: plugin_id.into(),
            runtime,
        }
    }

    /// Returns the plugin id.
    #[must_use]
    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    /// Returns the runtime kind.
    #[must_use]
    pub fn runtime(&self) -> RuntimeKind {
        self.runtime
    }
}

/// Invocation sent by ArcFlow to a plugin runtime.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginInvocation {
    /// Hook or method name.
    pub hook: String,
    /// JSON payload for the hook.
    pub payload: Value,
}

impl PluginInvocation {
    /// Constructs a plugin invocation.
    #[must_use]
    pub fn new(hook: impl Into<String>, payload: Value) -> Self {
        Self {
            hook: hook.into(),
            payload,
        }
    }
}

/// Action requested by a plugin after invocation.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginAction {
    /// Host method requested by the plugin.
    pub method: String,
    /// Host method params.
    pub params: Value,
}

impl PluginAction {
    /// Constructs a plugin action.
    #[must_use]
    pub fn new(method: impl Into<String>, params: Value) -> Self {
        Self {
            method: method.into(),
            params,
        }
    }
}

/// Output returned by a plugin invocation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PluginOutput {
    /// Host actions requested by the plugin.
    pub actions: Vec<PluginAction>,
}

impl PluginOutput {
    /// Constructs plugin output.
    #[must_use]
    pub fn new(actions: Vec<PluginAction>) -> Self {
        Self { actions }
    }
}

/// Runtime Adapter implemented by WASM and JavaScript engines.
#[async_trait]
pub trait RuntimeAdapter {
    /// Load a plugin manifest and return a runtime handle.
    async fn load(&self, manifest: &PluginManifest) -> Result<RuntimeHandle, RuntimeError>;

    /// Invoke a loaded plugin.
    async fn invoke(
        &self,
        handle: &RuntimeHandle,
        invocation: PluginInvocation,
    ) -> Result<PluginOutput, RuntimeError>;

    /// Unload a plugin runtime handle.
    async fn unload(&self, handle: RuntimeHandle) -> Result<(), RuntimeError>;
}

/// Error returned by plugin runtime Adapters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeError {
    /// Plugin was not loaded in the runtime.
    PluginNotLoaded(String),
    /// Runtime implementation failed.
    Runtime(String),
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PluginNotLoaded(plugin_id) => {
                write!(f, "plugin `{plugin_id}` is not loaded")
            }
            Self::Runtime(error) => write!(f, "plugin runtime error: {error}"),
        }
    }
}

impl std::error::Error for RuntimeError {}
