//! Sandbox policy wrappers for plugin runtimes.

use async_trait::async_trait;

use crate::{
    PluginInvocation, PluginManifest, PluginOutput, RuntimeAdapter, RuntimeError, RuntimeHandle,
    RuntimeKind,
};

/// Policy enforced before a plugin is loaded into a runtime engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxPolicy {
    allowed_api_versions: Vec<String>,
    allowed_runtimes: Vec<RuntimeKind>,
}

impl SandboxPolicy {
    /// Constructs a sandbox policy.
    #[must_use]
    pub fn new(allowed_api_versions: Vec<String>, allowed_runtimes: Vec<RuntimeKind>) -> Self {
        Self {
            allowed_api_versions,
            allowed_runtimes,
        }
    }

    /// Constructs the default ArcFlow plugin sandbox policy.
    #[must_use]
    pub fn arcflow_default() -> Self {
        Self::new(
            vec!["1".to_owned()],
            vec![RuntimeKind::Wasm, RuntimeKind::JavaScript],
        )
    }

    /// Returns whether a manifest can enter a runtime sandbox.
    pub fn validate(&self, manifest: &PluginManifest) -> Result<(), SandboxPolicyError> {
        manifest
            .validate()
            .map_err(|error| SandboxPolicyError::InvalidManifest(error.to_string()))?;

        if !self.allowed_api_versions.contains(&manifest.api_version) {
            return Err(SandboxPolicyError::UnsupportedApiVersion(
                manifest.api_version.clone(),
            ));
        }

        if !self.allowed_runtimes.contains(&manifest.runtime) {
            return Err(SandboxPolicyError::UnsupportedRuntime(manifest.runtime));
        }

        Ok(())
    }
}

impl Default for SandboxPolicy {
    fn default() -> Self {
        Self::arcflow_default()
    }
}

/// Error returned when a plugin fails sandbox policy validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxPolicyError {
    /// Manifest validation failed.
    InvalidManifest(String),
    /// Plugin requested an unsupported API version.
    UnsupportedApiVersion(String),
    /// Plugin requested an unsupported runtime.
    UnsupportedRuntime(RuntimeKind),
}

impl std::fmt::Display for SandboxPolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidManifest(error) => write!(f, "invalid plugin manifest: {error}"),
            Self::UnsupportedApiVersion(version) => {
                write!(f, "unsupported plugin API version `{version}`")
            }
            Self::UnsupportedRuntime(runtime) => {
                write!(f, "unsupported plugin runtime `{}`", runtime_name(*runtime))
            }
        }
    }
}

impl std::error::Error for SandboxPolicyError {}

/// Runtime wrapper that enforces sandbox policy before loading plugins.
#[derive(Debug)]
pub struct SandboxedRuntime<A> {
    adapter: A,
    policy: SandboxPolicy,
}

impl<A> SandboxedRuntime<A> {
    /// Constructs a sandboxed runtime wrapper.
    #[must_use]
    pub fn new(adapter: A, policy: SandboxPolicy) -> Self {
        Self { adapter, policy }
    }

    /// Constructs a sandboxed runtime with the default ArcFlow policy.
    #[must_use]
    pub fn default_policy(adapter: A) -> Self {
        Self::new(adapter, SandboxPolicy::default())
    }

    /// Returns the wrapped runtime Adapter.
    #[must_use]
    pub fn adapter(&self) -> &A {
        &self.adapter
    }

    /// Returns the sandbox policy.
    #[must_use]
    pub fn policy(&self) -> &SandboxPolicy {
        &self.policy
    }
}

#[async_trait]
impl<A> RuntimeAdapter for SandboxedRuntime<A>
where
    A: RuntimeAdapter + Send + Sync,
{
    async fn load(&self, manifest: &PluginManifest) -> Result<RuntimeHandle, RuntimeError> {
        self.policy
            .validate(manifest)
            .map_err(|error| RuntimeError::Runtime(error.to_string()))?;

        self.adapter.load(manifest).await
    }

    async fn invoke(
        &self,
        handle: &RuntimeHandle,
        invocation: PluginInvocation,
    ) -> Result<PluginOutput, RuntimeError> {
        self.adapter.invoke(handle, invocation).await
    }

    async fn unload(&self, handle: RuntimeHandle) -> Result<(), RuntimeError> {
        self.adapter.unload(handle).await
    }
}

/// Runtime Adapter used when a WASM or JavaScript engine has not been configured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnavailableRuntimeAdapter {
    runtime: RuntimeKind,
}

impl UnavailableRuntimeAdapter {
    /// Constructs an unavailable runtime Adapter.
    #[must_use]
    pub fn new(runtime: RuntimeKind) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl RuntimeAdapter for UnavailableRuntimeAdapter {
    async fn load(&self, manifest: &PluginManifest) -> Result<RuntimeHandle, RuntimeError> {
        Err(RuntimeError::Runtime(format!(
            "{} runtime is not configured for plugin `{}`",
            runtime_name(self.runtime),
            manifest.id
        )))
    }

    async fn invoke(
        &self,
        handle: &RuntimeHandle,
        _invocation: PluginInvocation,
    ) -> Result<PluginOutput, RuntimeError> {
        Err(RuntimeError::PluginNotLoaded(handle.plugin_id().to_owned()))
    }

    async fn unload(&self, _handle: RuntimeHandle) -> Result<(), RuntimeError> {
        Ok(())
    }
}

fn runtime_name(runtime: RuntimeKind) -> &'static str {
    match runtime {
        RuntimeKind::Wasm => "wasm",
        RuntimeKind::JavaScript => "javascript",
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use serde_json::json;

    use super::*;
    use crate::{Capability, PluginAction};

    #[derive(Debug, Default)]
    struct RecordingRuntime;

    #[async_trait]
    impl RuntimeAdapter for RecordingRuntime {
        async fn load(&self, manifest: &PluginManifest) -> Result<RuntimeHandle, RuntimeError> {
            Ok(RuntimeHandle::new(&manifest.id, manifest.runtime))
        }

        async fn invoke(
            &self,
            handle: &RuntimeHandle,
            invocation: PluginInvocation,
        ) -> Result<PluginOutput, RuntimeError> {
            Ok(PluginOutput::new(vec![PluginAction::new(
                format!("{}.{}", handle.plugin_id(), invocation.hook),
                invocation.payload,
            )]))
        }

        async fn unload(&self, _handle: RuntimeHandle) -> Result<(), RuntimeError> {
            Ok(())
        }
    }

    fn manifest(api_version: &str, runtime: RuntimeKind) -> PluginManifest {
        PluginManifest {
            id: "com.example.plugin".to_owned(),
            name: "Plugin".to_owned(),
            version: "1.0.0".to_owned(),
            runtime,
            entry: "dist/plugin.wasm".to_owned(),
            api_version: api_version.to_owned(),
            capabilities: vec![Capability::DeviceRead],
        }
    }

    #[tokio::test]
    async fn sandboxed_runtime_delegates_valid_plugins() {
        let runtime = SandboxedRuntime::new(RecordingRuntime, SandboxPolicy::default());

        let handle = runtime
            .load(&manifest("1", RuntimeKind::Wasm))
            .await
            .unwrap();
        let output = runtime
            .invoke(&handle, PluginInvocation::new("tick", json!({ "n": 1 })))
            .await
            .unwrap();

        assert_eq!(handle.plugin_id(), "com.example.plugin");
        assert_eq!(output.actions[0].method, "com.example.plugin.tick");
    }

    #[tokio::test]
    async fn sandboxed_runtime_rejects_unsupported_api_version() {
        let runtime = SandboxedRuntime::new(RecordingRuntime, SandboxPolicy::default());

        let error = runtime
            .load(&manifest("2", RuntimeKind::Wasm))
            .await
            .unwrap_err();

        assert_eq!(
            error,
            RuntimeError::Runtime("unsupported plugin API version `2`".to_owned())
        );
    }

    #[tokio::test]
    async fn unavailable_runtime_rejects_loads() {
        let runtime = UnavailableRuntimeAdapter::new(RuntimeKind::JavaScript);

        let error = runtime
            .load(&manifest("1", RuntimeKind::JavaScript))
            .await
            .unwrap_err();

        assert_eq!(
            error,
            RuntimeError::Runtime(
                "javascript runtime is not configured for plugin `com.example.plugin`".to_owned()
            )
        );
    }
}
