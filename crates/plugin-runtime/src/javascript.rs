//! JavaScript runtime adapter scaffolding.

use std::fs;

use async_trait::async_trait;

use crate::{
    PluginInvocation, PluginLoadRequest, PluginOutput, RecordedPlugin, RecordingRuntimeAdapter,
    RecordingRuntimeSnapshot, RuntimeAdapter, RuntimeError, RuntimeHandle, RuntimeKind,
};

/// JavaScript runtime adapter that validates bundle entry text before recording lifecycle.
///
/// Bundle-backed plugins are read from disk and validated as non-empty UTF-8
/// JavaScript modules. Manifest-only plugins still use recording lifecycle so
/// development and registry management can proceed before a bundle has been
/// attached. This adapter does not execute JavaScript exports yet.
#[derive(Debug, Clone)]
pub struct JavaScriptValidationRuntimeAdapter {
    recorder: RecordingRuntimeAdapter,
}

impl JavaScriptValidationRuntimeAdapter {
    /// Constructs a JavaScript validation runtime adapter.
    #[must_use]
    pub fn new() -> Self {
        Self {
            recorder: RecordingRuntimeAdapter::javascript(),
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

impl Default for JavaScriptValidationRuntimeAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl RuntimeAdapter for JavaScriptValidationRuntimeAdapter {
    async fn load(&self, request: &PluginLoadRequest) -> Result<RuntimeHandle, RuntimeError> {
        let manifest = request.manifest();
        if manifest.runtime != RuntimeKind::JavaScript {
            return Err(RuntimeError::Runtime(format!(
                "javascript validation runtime cannot load {:?} plugin `{}`",
                manifest.runtime, manifest.id
            )));
        }

        if let Some(entry_path) = request.resolved_entry_path() {
            let bytes = fs::read(&entry_path).map_err(|error| {
                RuntimeError::Runtime(format!(
                    "failed to read javascript entry `{}`: {error}",
                    entry_path.display()
                ))
            })?;
            let source = String::from_utf8(bytes).map_err(|error| {
                RuntimeError::Runtime(format!(
                    "javascript entry `{}` for plugin `{}` is not UTF-8: {error}",
                    entry_path.display(),
                    manifest.id
                ))
            })?;

            if source.trim().is_empty() {
                return Err(RuntimeError::Runtime(format!(
                    "javascript entry `{}` for plugin `{}` is empty",
                    entry_path.display(),
                    manifest.id
                )));
            }
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

    fn manifest(runtime: RuntimeKind, entry: &str) -> PluginManifest {
        PluginManifest {
            id: "dev.arcflow.js-test".to_owned(),
            name: "JavaScript Test".to_owned(),
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
        std::env::temp_dir().join(format!("arcflow-js-runtime-{name}-{suffix}"))
    }

    fn write_bundle(name: &str, entry: &str, bytes: &[u8]) -> PathBuf {
        let root = bundle_root(name);
        let entry_path = root.join(entry);
        fs::create_dir_all(entry_path.parent().unwrap()).unwrap();
        fs::write(entry_path, bytes).unwrap();
        root
    }

    #[tokio::test]
    async fn loads_valid_javascript_bundle_entry() {
        let root = write_bundle("valid", "dist/plugin.js", b"export default {};");
        let runtime = JavaScriptValidationRuntimeAdapter::new();
        let request = PluginLoadRequest::with_bundle_root(
            manifest(RuntimeKind::JavaScript, "dist/plugin.js"),
            root.display().to_string(),
        );

        let handle = runtime.load(&request).await.unwrap();

        assert_eq!(handle.plugin_id(), "dev.arcflow.js-test");
        assert_eq!(
            runtime.loaded_plugins()[0].bundle_root,
            request.bundle_root().map(str::to_owned)
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn records_manifest_only_javascript_loads() {
        let runtime = JavaScriptValidationRuntimeAdapter::new();
        let request = PluginLoadRequest::new(manifest(RuntimeKind::JavaScript, "dist/plugin.js"));

        let handle = runtime.load(&request).await.unwrap();

        assert_eq!(handle.plugin_id(), "dev.arcflow.js-test");
        assert_eq!(runtime.loaded_plugins()[0].bundle_root, None);
    }

    #[tokio::test]
    async fn rejects_non_utf8_javascript_entry() {
        let root = write_bundle("non-utf8", "dist/plugin.js", &[0xff, 0xfe, 0xfd]);
        let runtime = JavaScriptValidationRuntimeAdapter::new();
        let request = PluginLoadRequest::with_bundle_root(
            manifest(RuntimeKind::JavaScript, "dist/plugin.js"),
            root.display().to_string(),
        );

        let error = runtime.load(&request).await.unwrap_err();

        assert!(error.to_string().contains("not UTF-8"));

        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn rejects_empty_javascript_entry() {
        let root = write_bundle("empty", "dist/plugin.js", b"  \n");
        let runtime = JavaScriptValidationRuntimeAdapter::new();
        let request = PluginLoadRequest::with_bundle_root(
            manifest(RuntimeKind::JavaScript, "dist/plugin.js"),
            root.display().to_string(),
        );

        let error = runtime.load(&request).await.unwrap_err();

        assert!(error.to_string().contains("is empty"));

        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn rejects_non_javascript_manifests() {
        let root = write_bundle("wasm", "dist/plugin.wasm", b"\0asm\x01\0\0\0");
        let runtime = JavaScriptValidationRuntimeAdapter::new();
        let request = PluginLoadRequest::with_bundle_root(
            manifest(RuntimeKind::Wasm, "dist/plugin.wasm"),
            root.display().to_string(),
        );

        let error = runtime.load(&request).await.unwrap_err();

        assert!(error.to_string().contains("cannot load Wasm"));

        fs::remove_dir_all(root).unwrap();
    }
}
