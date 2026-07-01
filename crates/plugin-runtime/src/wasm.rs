//! WASM runtime adapter scaffolding.

use std::{
    collections::BTreeMap,
    fs,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use serde::Deserialize;
use wasmparser::{Parser, Payload, Validator};

use crate::{
    PluginInvocation, PluginLoadRequest, PluginOutput, RecordedPlugin, RecordingRuntimeAdapter,
    RecordingRuntimeSnapshot, RuntimeAdapter, RuntimeError, RuntimeHandle, RuntimeKind,
};

const ARCFLOW_PLUGIN_SECTION: &str = "arcflowPlugin";

/// WASM runtime adapter that validates bundle bytes before recording lifecycle.
///
/// Bundle-backed plugins are read from disk and validated as WebAssembly
/// modules. If a bundle contains an `arcflowPlugin` custom section, the adapter
/// returns matching hook output envelopes from that declaration. Manifest-only
/// plugins still use recording lifecycle so development and registry management
/// can proceed before a bundle has been attached.
#[derive(Debug, Clone)]
pub struct WasmValidationRuntimeAdapter {
    recorder: RecordingRuntimeAdapter,
    definitions: Arc<Mutex<BTreeMap<String, WasmPluginDefinition>>>,
}

impl WasmValidationRuntimeAdapter {
    /// Constructs a WASM validation runtime adapter.
    #[must_use]
    pub fn new() -> Self {
        Self {
            recorder: RecordingRuntimeAdapter::wasm(),
            definitions: Arc::new(Mutex::new(BTreeMap::new())),
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

        let mut definition = None;
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

            definition = parse_arcflow_plugin_definition(&bytes)?;
        }

        let handle = self.recorder.load(request).await?;
        if let Some(definition) = definition {
            self.definitions
                .lock()
                .expect("wasm plugin definition mutex poisoned")
                .insert(manifest.id.clone(), definition);
        }

        Ok(handle)
    }

    async fn invoke(
        &self,
        handle: &RuntimeHandle,
        invocation: PluginInvocation,
    ) -> Result<PluginOutput, RuntimeError> {
        let hook = invocation.hook.clone();
        let recorded_output = self.recorder.invoke(handle, invocation).await?;

        Ok(self
            .definitions
            .lock()
            .expect("wasm plugin definition mutex poisoned")
            .get(handle.plugin_id())
            .and_then(|definition| definition.hooks.get(&hook).cloned())
            .unwrap_or(recorded_output))
    }

    async fn unload(&self, handle: RuntimeHandle) -> Result<(), RuntimeError> {
        let plugin_id = handle.plugin_id().to_owned();
        self.recorder.unload(handle).await?;
        self.definitions
            .lock()
            .expect("wasm plugin definition mutex poisoned")
            .remove(&plugin_id);
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct WasmPluginDefinition {
    #[serde(default)]
    hooks: BTreeMap<String, PluginOutput>,
}

fn parse_arcflow_plugin_definition(
    bytes: &[u8],
) -> Result<Option<WasmPluginDefinition>, RuntimeError> {
    for payload in Parser::new(0).parse_all(bytes) {
        let payload = payload.map_err(|error| {
            RuntimeError::Runtime(format!("failed to parse wasm custom sections: {error}"))
        })?;

        let Payload::CustomSection(section) = payload else {
            continue;
        };

        if section.name() != ARCFLOW_PLUGIN_SECTION {
            continue;
        }

        let json = std::str::from_utf8(section.data()).map_err(|error| {
            RuntimeError::Runtime(format!(
                "wasm arcflowPlugin custom section is not UTF-8: {error}"
            ))
        })?;

        return serde_json::from_str(json).map(Some).map_err(|error| {
            RuntimeError::Runtime(format!(
                "wasm arcflowPlugin custom section is not valid JSON: {error}"
            ))
        });
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use serde_json::json;

    use crate::{Capability, PluginAction, PluginManifest};

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
    async fn invokes_declared_wasm_hook_output() {
        let root = write_bundle(
            "declared-output",
            "dist/plugin.wasm",
            &wasm_with_arcflow_plugin_definition(
                br#"{
                    "hooks": {
                        "device.connected": {
                            "actions": [
                                {
                                    "method": "storage.private.put",
                                    "params": {
                                        "key": "lastDevice",
                                        "value": {"deviceId": "coyote-v3"}
                                    }
                                }
                            ]
                        }
                    }
                }"#,
            ),
        );
        let runtime = WasmValidationRuntimeAdapter::new();
        let request = PluginLoadRequest::with_bundle_root(
            manifest(RuntimeKind::Wasm, "dist/plugin.wasm"),
            root.display().to_string(),
        );
        let handle = runtime.load(&request).await.unwrap();

        let output = runtime
            .invoke(
                &handle,
                PluginInvocation::new("device.connected", json!({ "deviceId": "coyote-v3" })),
            )
            .await
            .unwrap();

        assert_eq!(
            output,
            PluginOutput::new(vec![PluginAction::new(
                "storage.private.put",
                json!({
                    "key": "lastDevice",
                    "value": {"deviceId": "coyote-v3"}
                })
            )])
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn returns_empty_output_for_undeclared_wasm_hook() {
        let root = write_bundle(
            "undeclared-output",
            "dist/plugin.wasm",
            &wasm_with_arcflow_plugin_definition(br#"{"hooks": {}}"#),
        );
        let runtime = WasmValidationRuntimeAdapter::new();
        let request = PluginLoadRequest::with_bundle_root(
            manifest(RuntimeKind::Wasm, "dist/plugin.wasm"),
            root.display().to_string(),
        );
        let handle = runtime.load(&request).await.unwrap();

        let output = runtime
            .invoke(&handle, PluginInvocation::new("missing", json!({})))
            .await
            .unwrap();

        assert_eq!(output, PluginOutput::default());

        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn rejects_invalid_wasm_plugin_definition() {
        let root = write_bundle(
            "invalid-output",
            "dist/plugin.wasm",
            &wasm_with_arcflow_plugin_definition(b"{ hooks: {} }"),
        );
        let runtime = WasmValidationRuntimeAdapter::new();
        let request = PluginLoadRequest::with_bundle_root(
            manifest(RuntimeKind::Wasm, "dist/plugin.wasm"),
            root.display().to_string(),
        );

        let error = runtime.load(&request).await.unwrap_err();

        assert!(error.to_string().contains("not valid JSON"));

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

    fn wasm_with_arcflow_plugin_definition(definition: &[u8]) -> Vec<u8> {
        let mut payload = Vec::new();
        write_u32_leb(ARCFLOW_PLUGIN_SECTION.len() as u32, &mut payload);
        payload.extend_from_slice(ARCFLOW_PLUGIN_SECTION.as_bytes());
        payload.extend_from_slice(definition);

        let mut module = MINIMAL_WASM_MODULE.to_vec();
        module.push(0);
        write_u32_leb(payload.len() as u32, &mut module);
        module.extend_from_slice(&payload);
        module
    }

    fn write_u32_leb(mut value: u32, output: &mut Vec<u8>) {
        loop {
            let mut byte = (value & 0x7F) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            output.push(byte);

            if value == 0 {
                break;
            }
        }
    }
}
