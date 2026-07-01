//! Deterministic recording runtime used while real engines are wired in.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, MutexGuard},
};

use async_trait::async_trait;
use serde_json::Value;

use crate::{
    PluginInvocation, PluginLoadRequest, PluginOutput, RuntimeAdapter, RuntimeError, RuntimeHandle,
    RuntimeKind,
};

/// Runtime Adapter that records plugin lifecycle calls without executing code.
///
/// This adapter is intentionally side-effect free: `invoke` records the hook and
/// payload, then returns an empty [`PluginOutput`]. It gives ArcFlow a sandboxed
/// lifecycle implementation for development, integration tests, and host wiring
/// before real WASM and JavaScript engines are attached.
#[derive(Debug, Clone)]
pub struct RecordingRuntimeAdapter {
    inner: Arc<Mutex<RecordingRuntimeState>>,
}

impl RecordingRuntimeAdapter {
    /// Constructs a recording runtime for one runtime kind.
    #[must_use]
    pub fn new(runtime: RuntimeKind) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RecordingRuntimeState::new(runtime))),
        }
    }

    /// Constructs a recording WASM runtime.
    #[must_use]
    pub fn wasm() -> Self {
        Self::new(RuntimeKind::Wasm)
    }

    /// Constructs a recording JavaScript runtime.
    #[must_use]
    pub fn javascript() -> Self {
        Self::new(RuntimeKind::JavaScript)
    }

    /// Returns a point-in-time snapshot of loaded plugins and recorded events.
    #[must_use]
    pub fn snapshot(&self) -> RecordingRuntimeSnapshot {
        self.lock_state().snapshot()
    }

    /// Clears recorded lifecycle events while keeping loaded plugins intact.
    pub fn clear_events(&self) {
        self.lock_state().events.clear();
    }

    fn lock_state(&self) -> MutexGuard<'_, RecordingRuntimeState> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[async_trait]
impl RuntimeAdapter for RecordingRuntimeAdapter {
    async fn load(&self, request: &PluginLoadRequest) -> Result<RuntimeHandle, RuntimeError> {
        let manifest = request.manifest();
        let mut state = self.lock_state();
        state.ensure_runtime(manifest.runtime, &manifest.id)?;

        if state.loaded.contains_key(&manifest.id) {
            return Err(RuntimeError::Runtime(format!(
                "plugin `{}` is already loaded in {} recording runtime",
                manifest.id,
                runtime_name(state.runtime)
            )));
        }

        state.loaded.insert(
            manifest.id.clone(),
            RecordedPlugin {
                plugin_id: manifest.id.clone(),
                runtime: manifest.runtime,
                entry: manifest.entry.clone(),
                bundle_root: request.bundle_root().map(str::to_owned),
            },
        );
        state.events.push(RecordingRuntimeEvent::Loaded {
            plugin_id: manifest.id.clone(),
            runtime: manifest.runtime,
            entry: manifest.entry.clone(),
        });

        Ok(RuntimeHandle::new(&manifest.id, manifest.runtime))
    }

    async fn invoke(
        &self,
        handle: &RuntimeHandle,
        invocation: PluginInvocation,
    ) -> Result<PluginOutput, RuntimeError> {
        let mut state = self.lock_state();
        state.ensure_runtime(handle.runtime(), handle.plugin_id())?;
        state.ensure_loaded(handle.plugin_id())?;

        state.events.push(RecordingRuntimeEvent::Invoked {
            plugin_id: handle.plugin_id().to_owned(),
            runtime: handle.runtime(),
            hook: invocation.hook,
            payload: invocation.payload,
        });

        Ok(PluginOutput::default())
    }

    async fn unload(&self, handle: RuntimeHandle) -> Result<(), RuntimeError> {
        let mut state = self.lock_state();
        state.ensure_runtime(handle.runtime(), handle.plugin_id())?;

        let removed = state.loaded.remove(handle.plugin_id());
        if removed.is_none() {
            return Err(RuntimeError::PluginNotLoaded(handle.plugin_id().to_owned()));
        }

        state.events.push(RecordingRuntimeEvent::Unloaded {
            plugin_id: handle.plugin_id().to_owned(),
            runtime: handle.runtime(),
        });

        Ok(())
    }
}

/// Immutable snapshot of a recording runtime.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordingRuntimeSnapshot {
    /// Runtime kind owned by this adapter.
    pub runtime: RuntimeKind,
    /// Loaded plugin snapshots in stable plugin id order.
    pub loaded_plugins: Vec<RecordedPlugin>,
    /// Lifecycle events recorded by this adapter.
    pub events: Vec<RecordingRuntimeEvent>,
}

/// Snapshot of one plugin loaded into a recording runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedPlugin {
    /// Stable plugin id.
    pub plugin_id: String,
    /// Runtime kind used by the plugin.
    pub runtime: RuntimeKind,
    /// Bundle-relative entry file from the manifest.
    pub entry: String,
    /// Bundle root directory if the plugin was installed from disk.
    pub bundle_root: Option<String>,
}

/// Lifecycle event recorded by [`RecordingRuntimeAdapter`].
#[derive(Debug, Clone, PartialEq)]
pub enum RecordingRuntimeEvent {
    /// A plugin was loaded.
    Loaded {
        /// Stable plugin id.
        plugin_id: String,
        /// Runtime kind used by the plugin.
        runtime: RuntimeKind,
        /// Bundle-relative entry file from the manifest.
        entry: String,
    },
    /// A loaded plugin hook was invoked.
    Invoked {
        /// Stable plugin id.
        plugin_id: String,
        /// Runtime kind used by the plugin.
        runtime: RuntimeKind,
        /// Hook or method name.
        hook: String,
        /// JSON payload sent to the hook.
        payload: Value,
    },
    /// A plugin was unloaded.
    Unloaded {
        /// Stable plugin id.
        plugin_id: String,
        /// Runtime kind used by the plugin.
        runtime: RuntimeKind,
    },
}

#[derive(Debug)]
struct RecordingRuntimeState {
    runtime: RuntimeKind,
    loaded: BTreeMap<String, RecordedPlugin>,
    events: Vec<RecordingRuntimeEvent>,
}

impl RecordingRuntimeState {
    fn new(runtime: RuntimeKind) -> Self {
        Self {
            runtime,
            loaded: BTreeMap::new(),
            events: Vec::new(),
        }
    }

    fn snapshot(&self) -> RecordingRuntimeSnapshot {
        RecordingRuntimeSnapshot {
            runtime: self.runtime,
            loaded_plugins: self.loaded.values().cloned().collect(),
            events: self.events.clone(),
        }
    }

    fn ensure_runtime(&self, requested: RuntimeKind, plugin_id: &str) -> Result<(), RuntimeError> {
        if requested == self.runtime {
            Ok(())
        } else {
            Err(RuntimeError::Runtime(format!(
                "{} recording runtime cannot handle {} plugin `{plugin_id}`",
                runtime_name(self.runtime),
                runtime_name(requested)
            )))
        }
    }

    fn ensure_loaded(&self, plugin_id: &str) -> Result<(), RuntimeError> {
        if self.loaded.contains_key(plugin_id) {
            Ok(())
        } else {
            Err(RuntimeError::PluginNotLoaded(plugin_id.to_owned()))
        }
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
    use serde_json::json;

    use super::*;
    use crate::{Capability, PluginHost, PluginManifest, RuntimeRouter, SandboxedRuntime};

    fn manifest(id: &str, runtime: RuntimeKind) -> PluginManifest {
        PluginManifest {
            id: id.to_owned(),
            name: "Plugin".to_owned(),
            version: "1.0.0".to_owned(),
            runtime,
            entry: match runtime {
                RuntimeKind::Wasm => "dist/plugin.wasm".to_owned(),
                RuntimeKind::JavaScript => "dist/plugin.js".to_owned(),
            },
            api_version: "1".to_owned(),
            capabilities: vec![Capability::DeviceRead],
        }
    }

    #[tokio::test]
    async fn records_load_invoke_and_unload() {
        let runtime = RecordingRuntimeAdapter::wasm();
        let manifest = manifest("com.example.wasm", RuntimeKind::Wasm);

        let handle = runtime
            .load(&PluginLoadRequest::new(manifest))
            .await
            .unwrap();
        let output = runtime
            .invoke(&handle, PluginInvocation::new("tick", json!({ "n": 1 })))
            .await
            .unwrap();
        runtime.unload(handle).await.unwrap();

        let snapshot = runtime.snapshot();
        assert!(output.actions.is_empty());
        assert!(snapshot.loaded_plugins.is_empty());
        assert_eq!(
            snapshot.events,
            vec![
                RecordingRuntimeEvent::Loaded {
                    plugin_id: "com.example.wasm".to_owned(),
                    runtime: RuntimeKind::Wasm,
                    entry: "dist/plugin.wasm".to_owned(),
                },
                RecordingRuntimeEvent::Invoked {
                    plugin_id: "com.example.wasm".to_owned(),
                    runtime: RuntimeKind::Wasm,
                    hook: "tick".to_owned(),
                    payload: json!({ "n": 1 }),
                },
                RecordingRuntimeEvent::Unloaded {
                    plugin_id: "com.example.wasm".to_owned(),
                    runtime: RuntimeKind::Wasm,
                },
            ]
        );
    }

    #[tokio::test]
    async fn rejects_invocation_when_plugin_is_not_loaded() {
        let runtime = RecordingRuntimeAdapter::javascript();
        let handle = RuntimeHandle::new("com.example.js", RuntimeKind::JavaScript);

        let error = runtime
            .invoke(&handle, PluginInvocation::new("tick", json!({})))
            .await
            .unwrap_err();

        assert_eq!(
            error,
            RuntimeError::PluginNotLoaded("com.example.js".to_owned())
        );
    }

    #[tokio::test]
    async fn rejects_runtime_mismatches() {
        let runtime = RecordingRuntimeAdapter::wasm();

        let error = runtime
            .load(&PluginLoadRequest::new(manifest(
                "com.example.js",
                RuntimeKind::JavaScript,
            )))
            .await
            .unwrap_err();

        assert_eq!(
            error,
            RuntimeError::Runtime(
                "wasm recording runtime cannot handle javascript plugin `com.example.js`"
                    .to_owned()
            )
        );
    }

    #[tokio::test]
    async fn routes_wasm_and_javascript_recording_runtimes() {
        let router = RuntimeRouter::new(
            RecordingRuntimeAdapter::wasm(),
            RecordingRuntimeAdapter::javascript(),
        );
        let wasm = manifest("com.example.wasm", RuntimeKind::Wasm);
        let javascript = manifest("com.example.js", RuntimeKind::JavaScript);

        let wasm_handle = router.load(&PluginLoadRequest::new(wasm)).await.unwrap();
        let javascript_handle = router
            .load(&PluginLoadRequest::new(javascript))
            .await
            .unwrap();
        router
            .invoke(&javascript_handle, PluginInvocation::new("tick", json!({})))
            .await
            .unwrap();
        router.unload(wasm_handle).await.unwrap();

        assert!(router.wasm().snapshot().loaded_plugins.is_empty());
        assert_eq!(
            router.javascript().snapshot().loaded_plugins,
            vec![RecordedPlugin {
                plugin_id: "com.example.js".to_owned(),
                runtime: RuntimeKind::JavaScript,
                entry: "dist/plugin.js".to_owned(),
                bundle_root: None,
            }]
        );
    }

    #[tokio::test]
    async fn plugin_host_can_use_sandboxed_recording_router() {
        let router = RuntimeRouter::new(
            RecordingRuntimeAdapter::wasm(),
            RecordingRuntimeAdapter::javascript(),
        );
        let runtime = SandboxedRuntime::default_policy(router);
        let mut host = PluginHost::new(runtime);

        host.install(manifest("com.example.wasm", RuntimeKind::Wasm))
            .unwrap();
        host.install(manifest("com.example.js", RuntimeKind::JavaScript))
            .unwrap();

        host.enable("com.example.wasm").await.unwrap();
        host.enable("com.example.js").await.unwrap();
        host.invoke(
            "com.example.js",
            PluginInvocation::new("device.connected", json!({ "deviceId": "coyote-v3" })),
        )
        .await
        .unwrap();
        host.disable("com.example.wasm").await.unwrap();

        let runtime = host.adapter().adapter();
        assert!(runtime.wasm().snapshot().loaded_plugins.is_empty());
        assert_eq!(runtime.javascript().snapshot().events.len(), 2);
    }
}
