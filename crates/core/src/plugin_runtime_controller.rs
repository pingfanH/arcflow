//! Plugin runtime lifecycle controller.

use core::fmt;
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};

use arcflow_plugin_runtime::{
    JavaScriptValidationRuntimeAdapter, PluginHost, PluginHostError, PluginInvocation,
    PluginManifest, PluginOutput, PluginRegistry, RecordedPlugin, RecordingRuntimeSnapshot,
    RuntimeRouter, SandboxedRuntime, WasmValidationRuntimeAdapter,
};
use arcflow_storage::Storage;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex as AsyncMutex;

use crate::{CoreError, DeviceOutputController, PluginApi, PluginHookInvoker};

/// Runtime router used by the current plugin runtime.
pub type ArcFlowPluginRuntimeRouter =
    RuntimeRouter<WasmValidationRuntimeAdapter, JavaScriptValidationRuntimeAdapter>;

/// Sandboxed runtime used by the current plugin host.
pub type ArcFlowPluginRuntime = SandboxedRuntime<ArcFlowPluginRuntimeRouter>;

/// Plugin host used by the current plugin lifecycle controller.
pub type ArcFlowPluginHost = PluginHost<ArcFlowPluginRuntime>;

/// Core-owned controller for plugin load/unload lifecycle.
#[derive(Debug)]
pub struct RecordingPluginRuntimeController {
    host: ArcFlowPluginHost,
}

impl RecordingPluginRuntimeController {
    /// Constructs a plugin runtime controller backed by WASM and JS validation runtimes.
    #[must_use]
    pub fn new() -> Self {
        Self {
            host: new_recording_plugin_host(),
        }
    }

    /// Synchronizes the runtime host from the persisted plugin registry.
    ///
    /// Installed plugins are added to the host. Enabled plugins are loaded into
    /// the runtime, and disabled plugins are unloaded when they were previously
    /// enabled. This method is idempotent for unchanged registry records.
    pub async fn sync_from_registry(
        &mut self,
        registry: &PluginRegistry,
    ) -> Result<PluginRuntimeSyncReport, PluginRuntimeControllerError> {
        let mut report = PluginRuntimeSyncReport::default();
        let target_plugin_ids = registry
            .list()
            .into_iter()
            .map(|record| record.manifest().id.clone())
            .collect::<BTreeSet<_>>();
        let current_plugin_ids = self
            .host
            .registry()
            .list()
            .into_iter()
            .map(|record| record.manifest().id.clone())
            .collect::<Vec<_>>();

        for plugin_id in current_plugin_ids {
            if !target_plugin_ids.contains(&plugin_id) {
                let was_enabled = self
                    .host
                    .registry()
                    .get(&plugin_id)
                    .map_err(PluginHostError::from)?
                    .enabled();

                self.host.uninstall(&plugin_id).await?;
                if was_enabled {
                    report.unloaded_plugin_ids.push(plugin_id.clone());
                }
                report.uninstalled_plugin_ids.push(plugin_id);
            }
        }

        for record in registry.list() {
            let manifest = record.manifest().clone();
            let plugin_id = manifest.id.clone();

            if self.host.registry().get(&plugin_id).is_err() {
                match record.bundle_root() {
                    Some(bundle_root) => {
                        self.host.install_with_bundle_root(manifest, bundle_root)?
                    }
                    None => self.host.install(manifest)?,
                }
                report.installed_plugin_ids.push(plugin_id.clone());
            }

            let current_enabled = self
                .host
                .registry()
                .get(&plugin_id)
                .map_err(PluginHostError::from)?
                .enabled();
            match (current_enabled, record.enabled()) {
                (false, true) => {
                    self.host.enable(&plugin_id).await?;
                    report.loaded_plugin_ids.push(plugin_id);
                }
                (true, false) => {
                    self.host.disable(&plugin_id).await?;
                    report.unloaded_plugin_ids.push(plugin_id);
                }
                _ => {}
            }
        }

        Ok(report)
    }

    /// Returns the current recording runtime snapshots.
    #[must_use]
    pub fn recording_snapshots(&self) -> RecordingPluginRuntimeSnapshots {
        let router = self.host.adapter().adapter();

        RecordingPluginRuntimeSnapshots {
            wasm: router.wasm().snapshot(),
            javascript: router.javascript().snapshot(),
        }
    }

    /// Returns loaded plugin snapshots across both runtimes.
    #[must_use]
    pub fn loaded_plugins(&self) -> Vec<RecordedPlugin> {
        let snapshots = self.recording_snapshots();
        let mut plugins = snapshots.wasm.loaded_plugins;
        plugins.extend(snapshots.javascript.loaded_plugins);
        plugins.sort_by(|left, right| left.plugin_id.cmp(&right.plugin_id));
        plugins
    }

    /// Invokes one hook on an enabled plugin.
    pub async fn invoke_hook(
        &self,
        plugin_id: &str,
        hook: &str,
        payload: Value,
    ) -> Result<PluginOutput, PluginRuntimeControllerError> {
        Ok(self
            .host
            .invoke(plugin_id, PluginInvocation::new(hook, payload))
            .await?)
    }

    /// Invokes one hook and returns the manifest that owns its output.
    pub async fn invoke_hook_with_manifest(
        &self,
        plugin_id: &str,
        hook: &str,
        payload: Value,
    ) -> Result<(PluginManifest, PluginOutput), PluginRuntimeControllerError> {
        let manifest = self
            .host
            .registry()
            .get(plugin_id)
            .map_err(PluginHostError::from)?
            .manifest()
            .clone();
        let output = self
            .host
            .invoke(plugin_id, PluginInvocation::new(hook, payload))
            .await?;

        Ok((manifest, output))
    }
}

impl Default for RecordingPluginRuntimeController {
    fn default() -> Self {
        Self::new()
    }
}

/// Report returned after synchronizing the plugin runtime host.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PluginRuntimeSyncReport {
    /// Plugins installed into the host registry.
    pub installed_plugin_ids: Vec<String>,
    /// Plugins removed from the host registry.
    pub uninstalled_plugin_ids: Vec<String>,
    /// Plugins loaded into a runtime.
    pub loaded_plugin_ids: Vec<String>,
    /// Plugins unloaded from a runtime.
    pub unloaded_plugin_ids: Vec<String>,
}

impl PluginRuntimeSyncReport {
    /// Returns whether synchronization changed the runtime host.
    #[must_use]
    pub fn changed(&self) -> bool {
        !self.installed_plugin_ids.is_empty()
            || !self.uninstalled_plugin_ids.is_empty()
            || !self.loaded_plugin_ids.is_empty()
            || !self.unloaded_plugin_ids.is_empty()
    }
}

/// Snapshots of the current recording WASM and JavaScript runtimes.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordingPluginRuntimeSnapshots {
    /// WASM recording runtime snapshot.
    pub wasm: RecordingRuntimeSnapshot,
    /// JavaScript recording runtime snapshot.
    pub javascript: RecordingRuntimeSnapshot,
}

/// Error returned by plugin runtime lifecycle operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginRuntimeControllerError {
    /// Plugin host operation failed.
    Host(PluginHostError),
}

impl fmt::Display for PluginRuntimeControllerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Host(error) => write!(f, "plugin runtime host error: {error}"),
        }
    }
}

impl std::error::Error for PluginRuntimeControllerError {}

impl From<PluginHostError> for PluginRuntimeControllerError {
    fn from(error: PluginHostError) -> Self {
        Self::Host(error)
    }
}

/// Script hook invoker backed by the recording plugin runtime controller.
#[derive(Clone)]
pub struct RecordingPluginRuntimeHookInvoker {
    controller: Arc<AsyncMutex<RecordingPluginRuntimeController>>,
    action_handler: Option<PluginOutputActionHandler>,
}

impl RecordingPluginRuntimeHookInvoker {
    /// Constructs a hook invoker over the shared recording runtime controller.
    #[must_use]
    pub fn new(controller: Arc<AsyncMutex<RecordingPluginRuntimeController>>) -> Self {
        Self {
            controller,
            action_handler: None,
        }
    }

    /// Constructs a hook invoker that routes plugin output actions through the Plugin API.
    #[must_use]
    pub fn with_plugin_api(
        controller: Arc<AsyncMutex<RecordingPluginRuntimeController>>,
        storage: Arc<Mutex<Storage>>,
        output_controller: Arc<dyn DeviceOutputController>,
    ) -> Self {
        Self {
            controller,
            action_handler: Some(PluginOutputActionHandler::new(storage, output_controller)),
        }
    }
}

impl fmt::Debug for RecordingPluginRuntimeHookInvoker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecordingPluginRuntimeHookInvoker")
            .field("handles_plugin_api_actions", &self.action_handler.is_some())
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl PluginHookInvoker for RecordingPluginRuntimeHookInvoker {
    async fn invoke_plugin_hook(
        &self,
        plugin_id: &str,
        hook: &str,
        payload: &Value,
    ) -> Result<(), CoreError> {
        let (manifest, output) = self
            .controller
            .lock()
            .await
            .invoke_hook_with_manifest(plugin_id, hook, payload.clone())
            .await
            .map_err(|error| CoreError::Script(error.to_string()))?;

        if let Some(action_handler) = &self.action_handler {
            return action_handler.handle(&manifest, output);
        }

        if output.actions.is_empty() {
            Ok(())
        } else {
            Err(CoreError::Script(format!(
                "plugin hook `{hook}` for plugin `{plugin_id}` returned host actions, but script hook output handling is not attached"
            )))
        }
    }
}

#[derive(Clone)]
struct PluginOutputActionHandler {
    storage: Arc<Mutex<Storage>>,
    output_controller: Arc<dyn DeviceOutputController>,
}

impl PluginOutputActionHandler {
    fn new(
        storage: Arc<Mutex<Storage>>,
        output_controller: Arc<dyn DeviceOutputController>,
    ) -> Self {
        Self {
            storage,
            output_controller,
        }
    }

    fn handle(&self, manifest: &PluginManifest, output: PluginOutput) -> Result<(), CoreError> {
        if output.actions.is_empty() {
            return Ok(());
        }

        let storage = self
            .storage
            .lock()
            .expect("plugin API storage mutex poisoned");
        let api = PluginApi::with_output_controller(&storage, Arc::clone(&self.output_controller));
        api.handle_output(manifest, output)
            .map(|_| ())
            .map_err(|error| CoreError::Script(error.to_string()))
    }
}

impl fmt::Debug for PluginOutputActionHandler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PluginOutputActionHandler")
            .finish_non_exhaustive()
    }
}

fn new_recording_plugin_host() -> ArcFlowPluginHost {
    PluginHost::new(SandboxedRuntime::default_policy(RuntimeRouter::new(
        WasmValidationRuntimeAdapter::new(),
        JavaScriptValidationRuntimeAdapter::new(),
    )))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use arcflow_plugin_runtime::{Capability, PluginAction, PluginManifest, RuntimeKind};
    use arcflow_storage::Storage;
    use serde_json::json;

    use super::*;

    const MINIMAL_WASM_MODULE: &[u8] = b"\0asm\x01\0\0\0";

    #[derive(Debug)]
    struct FakeOutputController {
        stop_count: Mutex<usize>,
    }

    impl DeviceOutputController for FakeOutputController {
        fn submit_coyote_v3_window(
            &self,
            _request: crate::CoyoteV3OutputRequest,
        ) -> Result<crate::SubmitOutputResult, crate::CoreError> {
            Err(crate::CoreError::Transport(
                "fake output controller does not submit windows".to_owned(),
            ))
        }

        fn stop_all_output(&self) -> Result<crate::StopOutputResult, crate::CoreError> {
            *self.stop_count.lock().unwrap() += 1;
            Ok(crate::StopOutputResult::new(vec![crate::DeviceId::new(
                "coyote-v3",
            )]))
        }
    }

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

    fn manifest_with_capabilities(
        id: &str,
        runtime: RuntimeKind,
        capabilities: Vec<Capability>,
    ) -> PluginManifest {
        PluginManifest {
            capabilities,
            ..manifest(id, runtime)
        }
    }

    #[test]
    fn plugin_output_action_handler_routes_actions_through_plugin_api() {
        let storage = Arc::new(Mutex::new(Storage::in_memory().unwrap()));
        let output_controller = Arc::new(FakeOutputController {
            stop_count: Mutex::new(0),
        });
        let output: Arc<dyn DeviceOutputController> = output_controller.clone();
        let handler = PluginOutputActionHandler::new(storage, output);
        let manifest = manifest_with_capabilities(
            "plugin.wasm",
            RuntimeKind::Wasm,
            vec![Capability::WaveControl],
        );

        handler
            .handle(
                &manifest,
                PluginOutput::new(vec![PluginAction::new(
                    "wave.stop",
                    json!({ "deviceId": "coyote-v3" }),
                )]),
            )
            .unwrap();

        assert_eq!(*output_controller.stop_count.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn sync_installs_and_loads_enabled_plugins() {
        let mut registry = PluginRegistry::new();
        registry
            .install(manifest("plugin.wasm", RuntimeKind::Wasm))
            .unwrap();
        registry
            .install(manifest("plugin.js", RuntimeKind::JavaScript))
            .unwrap();
        registry.enable("plugin.wasm").unwrap();
        registry.enable("plugin.js").unwrap();
        let mut controller = RecordingPluginRuntimeController::new();

        let report = controller.sync_from_registry(&registry).await.unwrap();

        assert_eq!(
            report.installed_plugin_ids,
            vec!["plugin.js", "plugin.wasm"]
        );
        assert_eq!(report.loaded_plugin_ids, vec!["plugin.js", "plugin.wasm"]);
        assert_eq!(
            controller
                .loaded_plugins()
                .into_iter()
                .map(|plugin| plugin.plugin_id)
                .collect::<Vec<_>>(),
            vec!["plugin.js", "plugin.wasm"]
        );
    }

    #[tokio::test]
    async fn sync_preserves_bundle_roots_for_loaded_plugins() {
        let bundle_root = test_bundle_root("plugin-runtime-controller");
        fs::create_dir_all(bundle_root.join("dist")).unwrap();
        fs::write(bundle_root.join("dist/plugin.wasm"), MINIMAL_WASM_MODULE).unwrap();
        let mut registry = PluginRegistry::new();
        registry
            .install_with_bundle_root(
                manifest("plugin.wasm", RuntimeKind::Wasm),
                bundle_root.display().to_string(),
            )
            .unwrap();
        registry.enable("plugin.wasm").unwrap();
        let mut controller = RecordingPluginRuntimeController::new();

        controller.sync_from_registry(&registry).await.unwrap();

        assert_eq!(
            controller.loaded_plugins()[0].bundle_root,
            Some(bundle_root.display().to_string())
        );

        fs::remove_dir_all(bundle_root).unwrap();
    }

    #[tokio::test]
    async fn sync_loads_javascript_bundle_through_validation_runtime() {
        let bundle_root = test_bundle_root("plugin-runtime-controller-js");
        fs::create_dir_all(bundle_root.join("dist")).unwrap();
        fs::write(bundle_root.join("dist/plugin.js"), b"export default {};").unwrap();
        let mut registry = PluginRegistry::new();
        registry
            .install_with_bundle_root(
                manifest("plugin.js", RuntimeKind::JavaScript),
                bundle_root.display().to_string(),
            )
            .unwrap();
        registry.enable("plugin.js").unwrap();
        let mut controller = RecordingPluginRuntimeController::new();

        controller.sync_from_registry(&registry).await.unwrap();

        assert_eq!(
            controller.loaded_plugins()[0].bundle_root,
            Some(bundle_root.display().to_string())
        );

        fs::remove_dir_all(bundle_root).unwrap();
    }

    #[tokio::test]
    async fn sync_rejects_invalid_javascript_bundle_entry() {
        let bundle_root = test_bundle_root("plugin-runtime-controller-js-invalid");
        fs::create_dir_all(bundle_root.join("dist")).unwrap();
        fs::write(bundle_root.join("dist/plugin.js"), b" \n").unwrap();
        let mut registry = PluginRegistry::new();
        registry
            .install_with_bundle_root(
                manifest("plugin.js", RuntimeKind::JavaScript),
                bundle_root.display().to_string(),
            )
            .unwrap();
        registry.enable("plugin.js").unwrap();
        let mut controller = RecordingPluginRuntimeController::new();

        let error = controller.sync_from_registry(&registry).await.unwrap_err();

        assert!(error.to_string().contains("is empty"));

        fs::remove_dir_all(bundle_root).unwrap();
    }

    #[tokio::test]
    async fn sync_is_idempotent_for_unchanged_registry() {
        let mut registry = PluginRegistry::new();
        registry
            .install(manifest("plugin.wasm", RuntimeKind::Wasm))
            .unwrap();
        registry.enable("plugin.wasm").unwrap();
        let mut controller = RecordingPluginRuntimeController::new();

        controller.sync_from_registry(&registry).await.unwrap();
        let report = controller.sync_from_registry(&registry).await.unwrap();

        assert!(!report.changed());
        assert_eq!(controller.loaded_plugins().len(), 1);
    }

    #[tokio::test]
    async fn sync_unloads_disabled_plugins() {
        let mut registry = PluginRegistry::new();
        registry
            .install(manifest("plugin.wasm", RuntimeKind::Wasm))
            .unwrap();
        registry.enable("plugin.wasm").unwrap();
        let mut controller = RecordingPluginRuntimeController::new();
        controller.sync_from_registry(&registry).await.unwrap();

        registry.disable("plugin.wasm").unwrap();
        let report = controller.sync_from_registry(&registry).await.unwrap();

        assert_eq!(report.unloaded_plugin_ids, vec!["plugin.wasm"]);
        assert!(controller.loaded_plugins().is_empty());
    }

    #[tokio::test]
    async fn sync_uninstalls_removed_plugins() {
        let mut registry = PluginRegistry::new();
        registry
            .install(manifest("plugin.wasm", RuntimeKind::Wasm))
            .unwrap();
        registry.enable("plugin.wasm").unwrap();
        let mut controller = RecordingPluginRuntimeController::new();
        controller.sync_from_registry(&registry).await.unwrap();

        registry.uninstall("plugin.wasm").unwrap();
        let report = controller.sync_from_registry(&registry).await.unwrap();

        assert_eq!(report.unloaded_plugin_ids, vec!["plugin.wasm"]);
        assert_eq!(report.uninstalled_plugin_ids, vec!["plugin.wasm"]);
        assert!(controller.loaded_plugins().is_empty());
    }

    #[tokio::test]
    async fn invokes_enabled_plugin_hooks() {
        let mut registry = PluginRegistry::new();
        registry
            .install(manifest("plugin.wasm", RuntimeKind::Wasm))
            .unwrap();
        registry.enable("plugin.wasm").unwrap();
        let mut controller = RecordingPluginRuntimeController::new();
        controller.sync_from_registry(&registry).await.unwrap();

        let output = controller
            .invoke_hook("plugin.wasm", "afterStop", json!({ "ok": true }))
            .await
            .unwrap();

        assert!(output.actions.is_empty());
        assert_eq!(controller.recording_snapshots().wasm.events.len(), 2);
    }

    #[tokio::test]
    async fn hook_invoker_rejects_unloaded_plugins() {
        let controller = Arc::new(AsyncMutex::new(RecordingPluginRuntimeController::new()));
        let invoker = RecordingPluginRuntimeHookInvoker::new(controller);

        let error = invoker
            .invoke_plugin_hook("plugin.missing", "afterStop", &json!({}))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("not installed"));
    }

    fn test_bundle_root(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("arcflow-{name}-{suffix}"))
    }
}
