//! WASM runtime adapter scaffolding.

use std::fs;

use async_trait::async_trait;
use wasmparser::Validator;

use crate::{
    PluginInvocation, PluginLoadRequest, PluginOutput, RecordedPlugin, RecordingRuntimeAdapter,
    RecordingRuntimeSnapshot, RuntimeAdapter, RuntimeError, RuntimeHandle, RuntimeKind,
};

/// WASM runtime adapter that validates bundle bytes before recording lifecycle.
///
/// Bundle-backed plugins are read from disk and validated as WebAssembly
/// modules. Manifest-only plugins still use recording lifecycle so development
/// and registry management can proceed before a bundle has been attached. This
/// adapter does not execute WASM exports yet.
#[derive(Debug, Clone)]
pub struct WasmValidationRuntimeAdapter {
    recorder: RecordingRuntimeAdapter,
}

impl WasmValidationRuntimeAdapter {
    /// Constructs a WASM validation runtime adapter.
    #[must_use]
    pub fn new() -> Self {
        Self {
            recorder: RecordingRuntimeAdapter::wasm(),
        }
    }

    /// Returns loaded plugin snapshots and lifecycle events.
    #[must_use]
    pub fn snapshot(&self) -> RecordingRuntimeSnapshot {
        self.recorder.snapshot()
    }

    /// Returns loaded plugin snapshots in stable plugin id order.
    #[must_use]
    pub fn loaded_plugins(&self) -> Vec<RecordedPlugin> {
        self.snapshot().loaded_plugins
    }
}

impl Default for WasmValidationRuntimeAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl RuntimeAdapter for WasmValidationRuntimeAdapter {
    async fn load(&self, request: &PluginLoadRequest) -> Result<RuntimeHandle, RuntimeError> {
        let manifest = request.manifest();
        if manifest.runtime != RuntimeKind::Wasm {
            return Err(RuntimeError::Runtime(format!(
                "wasm validation runtime cannot load {:?} plugin `{}`",
                manifest.runtime, manifest.id
            )));
        }

        if let Some(entry_path) = request.resolved_entry_path() {
            let bytes = fs::read(&entry_path).map_err(|error| {
                RuntimeError::Runtime(format!(
                    "failed to read wasm entry `{}`: {error}",
                    entry_path.display()
                ))
            })?;

            Validator::new().validate_all(&bytes).map_err(|error| {
                RuntimeError::Runtime(format!(
                    "invalid wasm entry `{}` for plugin `{}`: {error}",
                    entry_path.display(),
                    manifest.id
                ))
            })?;
        }

        self.recorder.load(request).await
    }

    async fn invoke(
        &self,
        handle: &RuntimeHandle,
        invocation: PluginInvocation,
    ) -> Result<PluginOutput, RuntimeError> {
        self.recorder.invoke(handle, invocation).await
    }

    async fn unload(&self, handle: RuntimeHandle) -> Result<(), RuntimeError> {
        self.recorder.unload(handle).await
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use crate::{Capability, PluginManifest};

    const MINIMAL_WASM_MODULE: &[u8] = b"\0asm\x01\0\0\0";

    fn manifest(runtime: RuntimeKind, entry: &str) -> PluginManifest {
        PluginManifest {
            id: "dev.arcflow.wasm-test".to_owned(),
            name: "WASM Test".to_owned(),
            version: "0.1.0".to_owned(),
            runtime,
            entry: entry.to_owned(),
            api_version: "1".to_owned(),
            capabilities: vec![Capability::DeviceRead],
        }
    }

    fn bundle_root(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("arcflow-wasm-runtime-{name}-{suffix}"))
    }

    fn write_bundle(name: &str, entry: &str, bytes: &[u8]) -> PathBuf {
        let root = bundle_root(name);
        let entry_path = root.join(entry);
        fs::create_dir_all(entry_path.parent().unwrap()).unwrap();
        fs::write(entry_path, bytes).unwrap();
        root
    }

    #[tokio::test]
    async fn loads_valid_wasm_bundle_entry() {
        let root = write_bundle("valid", "dist/plugin.wasm", MINIMAL_WASM_MODULE);
        let runtime = WasmValidationRuntimeAdapter::new();
        let request = PluginLoadRequest::with_bundle_root(
            manifest(RuntimeKind::Wasm, "dist/plugin.wasm"),
            root.display().to_string(),
        );

        let handle = runtime.load(&request).await.unwrap();

        assert_eq!(handle.plugin_id(), "dev.arcflow.wasm-test");
        assert_eq!(
            runtime.loaded_plugins()[0].bundle_root,
            request.bundle_root().map(str::to_owned)
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn records_manifest_only_wasm_loads() {
        let runtime = WasmValidationRuntimeAdapter::new();
        let request = PluginLoadRequest::new(manifest(RuntimeKind::Wasm, "dist/plugin.wasm"));

        let handle = runtime.load(&request).await.unwrap();

        assert_eq!(handle.plugin_id(), "dev.arcflow.wasm-test");
        assert_eq!(runtime.loaded_plugins()[0].bundle_root, None);
    }

    #[tokio::test]
    async fn rejects_invalid_wasm_bytes() {
        let root = write_bundle("invalid", "dist/plugin.wasm", b"not wasm");
        let runtime = WasmValidationRuntimeAdapter::new();
        let request = PluginLoadRequest::with_bundle_root(
            manifest(RuntimeKind::Wasm, "dist/plugin.wasm"),
            root.display().to_string(),
        );

        let error = runtime.load(&request).await.unwrap_err();

        assert!(error.to_string().contains("invalid wasm entry"));

        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn rejects_non_wasm_manifests() {
        let root = write_bundle("javascript", "dist/plugin.js", b"export default {};");
        let runtime = WasmValidationRuntimeAdapter::new();
        let request = PluginLoadRequest::with_bundle_root(
            manifest(RuntimeKind::JavaScript, "dist/plugin.js"),
            root.display().to_string(),
        );

        let error = runtime.load(&request).await.unwrap_err();

        assert!(error.to_string().contains("cannot load JavaScript"));

        fs::remove_dir_all(root).unwrap();
    }
}
