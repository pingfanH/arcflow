//! JavaScript runtime adapter scaffolding.

use std::{
    collections::BTreeMap,
    fs,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use serde::Deserialize;

use crate::{
    PluginInvocation, PluginLoadRequest, PluginOutput, RecordedPlugin, RecordingRuntimeAdapter,
    RecordingRuntimeSnapshot, RuntimeAdapter, RuntimeError, RuntimeHandle, RuntimeKind,
};

const ARCFLOW_PLUGIN_EXPORT: &str = "export const arcflowPlugin =";

/// JavaScript runtime adapter that validates bundle entry text before recording lifecycle.
///
/// Bundle-backed plugins are read from disk and validated as non-empty UTF-8
/// JavaScript modules. If a bundle exports an `arcflowPlugin` JSON object, the
/// adapter returns matching hook output envelopes from that declaration.
/// Manifest-only plugins still use recording lifecycle so development and
/// registry management can proceed before a bundle has been attached.
#[derive(Debug, Clone)]
pub struct JavaScriptValidationRuntimeAdapter {
    recorder: RecordingRuntimeAdapter,
    definitions: Arc<Mutex<BTreeMap<String, JavaScriptPluginDefinition>>>,
}

impl JavaScriptValidationRuntimeAdapter {
    /// Constructs a JavaScript validation runtime adapter.
    #[must_use]
    pub fn new() -> Self {
        Self {
            recorder: RecordingRuntimeAdapter::javascript(),
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

        let mut definition = None;
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

            definition = parse_arcflow_plugin_definition(&source)?;
        }

        let handle = self.recorder.load(request).await?;
        if let Some(definition) = definition {
            self.definitions
                .lock()
                .expect("javascript plugin definition mutex poisoned")
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
            .expect("javascript plugin definition mutex poisoned")
            .get(handle.plugin_id())
            .and_then(|definition| definition.hooks.get(&hook).cloned())
            .unwrap_or(recorded_output))
    }

    async fn unload(&self, handle: RuntimeHandle) -> Result<(), RuntimeError> {
        let plugin_id = handle.plugin_id().to_owned();
        self.recorder.unload(handle).await?;
        self.definitions
            .lock()
            .expect("javascript plugin definition mutex poisoned")
            .remove(&plugin_id);
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct JavaScriptPluginDefinition {
    #[serde(default)]
    hooks: BTreeMap<String, PluginOutput>,
}

fn parse_arcflow_plugin_definition(
    source: &str,
) -> Result<Option<JavaScriptPluginDefinition>, RuntimeError> {
    let Some(export_start) = source.find(ARCFLOW_PLUGIN_EXPORT) else {
        return Ok(None);
    };

    let object_start = export_start
        + ARCFLOW_PLUGIN_EXPORT.len()
        + source[export_start + ARCFLOW_PLUGIN_EXPORT.len()..]
            .find('{')
            .ok_or_else(|| {
                RuntimeError::Runtime(
                    "javascript arcflowPlugin export must be assigned a JSON object".to_owned(),
                )
            })?;
    let object_end = matching_json_object_end(source, object_start)?;
    let object_source = &source[object_start..object_end];

    serde_json::from_str(object_source)
        .map(Some)
        .map_err(|error| {
            RuntimeError::Runtime(format!(
                "javascript arcflowPlugin export is not valid JSON: {error}"
            ))
        })
}

fn matching_json_object_end(source: &str, object_start: usize) -> Result<usize, RuntimeError> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (offset, character) in source[object_start..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }

        match character {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(object_start + offset + character.len_utf8());
                }
            }
            _ => {}
        }
    }

    Err(RuntimeError::Runtime(
        "javascript arcflowPlugin export has an unterminated JSON object".to_owned(),
    ))
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
    async fn invokes_declared_javascript_hook_output() {
        let root = write_bundle(
            "declared-output",
            "dist/plugin.js",
            br#"export const arcflowPlugin = {
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
            };"#,
        );
        let runtime = JavaScriptValidationRuntimeAdapter::new();
        let request = PluginLoadRequest::with_bundle_root(
            manifest(RuntimeKind::JavaScript, "dist/plugin.js"),
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
    async fn returns_empty_output_for_undeclared_javascript_hook() {
        let root = write_bundle(
            "undeclared-output",
            "dist/plugin.js",
            br#"export const arcflowPlugin = {"hooks": {}};"#,
        );
        let runtime = JavaScriptValidationRuntimeAdapter::new();
        let request = PluginLoadRequest::with_bundle_root(
            manifest(RuntimeKind::JavaScript, "dist/plugin.js"),
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
    async fn rejects_invalid_javascript_plugin_definition() {
        let root = write_bundle(
            "invalid-output",
            "dist/plugin.js",
            br#"export const arcflowPlugin = { hooks: {} };"#,
        );
        let runtime = JavaScriptValidationRuntimeAdapter::new();
        let request = PluginLoadRequest::with_bundle_root(
            manifest(RuntimeKind::JavaScript, "dist/plugin.js"),
            root.display().to_string(),
        );

        let error = runtime.load(&request).await.unwrap_err();

        assert!(error.to_string().contains("not valid JSON"));

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
