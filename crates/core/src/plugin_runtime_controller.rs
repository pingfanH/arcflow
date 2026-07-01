//! Plugin runtime lifecycle controller.

use core::fmt;

use arcflow_plugin_runtime::{
    PluginHost, PluginHostError, PluginRegistry, RecordedPlugin, RecordingRuntimeAdapter,
    RecordingRuntimeSnapshot, RuntimeRouter, SandboxedRuntime,
};

/// Runtime router used by the current recording plugin runtime.
pub type RecordingPluginRuntimeRouter =
    RuntimeRouter<RecordingRuntimeAdapter, RecordingRuntimeAdapter>;

/// Sandboxed runtime used by the current recording plugin host.
pub type RecordingPluginRuntime = SandboxedRuntime<RecordingPluginRuntimeRouter>;

/// Plugin host used by the current recording plugin lifecycle controller.
pub type RecordingPluginHost = PluginHost<RecordingPluginRuntime>;

/// Core-owned controller for plugin load/unload lifecycle.
#[derive(Debug)]
pub struct RecordingPluginRuntimeController {
    host: RecordingPluginHost,
}

impl RecordingPluginRuntimeController {
    /// Constructs a plugin runtime controller backed by recording WASM/JS runtimes.
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

        for record in registry.list() {
            let manifest = record.manifest().clone();
            let plugin_id = manifest.id.clone();

            if self.host.registry().get(&plugin_id).is_err() {
                self.host.install(manifest)?;
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

fn new_recording_plugin_host() -> RecordingPluginHost {
    PluginHost::new(SandboxedRuntime::default_policy(RuntimeRouter::new(
        RecordingRuntimeAdapter::wasm(),
        RecordingRuntimeAdapter::javascript(),
    )))
}

#[cfg(test)]
mod tests {
    use arcflow_plugin_runtime::{Capability, PluginManifest, RuntimeKind};

    use super::*;

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
}
