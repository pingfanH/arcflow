//! Plugin host orchestration.

use std::collections::BTreeMap;

use crate::{
    PluginInvocation, PluginLoadRequest, PluginManifest, PluginOutput, PluginRegistry,
    PluginRegistryError, RuntimeAdapter, RuntimeError, RuntimeHandle,
};

/// Plugin host that coordinates the registry and runtime Adapter.
#[derive(Debug)]
pub struct PluginHost<A> {
    registry: PluginRegistry,
    adapter: A,
    handles: BTreeMap<String, RuntimeHandle>,
}

impl<A> PluginHost<A>
where
    A: RuntimeAdapter,
{
    /// Constructs a plugin host.
    #[must_use]
    pub fn new(adapter: A) -> Self {
        Self {
            registry: PluginRegistry::new(),
            adapter,
            handles: BTreeMap::new(),
        }
    }

    /// Returns the plugin registry.
    #[must_use]
    pub fn registry(&self) -> &PluginRegistry {
        &self.registry
    }

    /// Returns the runtime Adapter.
    #[must_use]
    pub fn adapter(&self) -> &A {
        &self.adapter
    }

    /// Installs a plugin manifest as disabled.
    pub fn install(&mut self, manifest: PluginManifest) -> Result<(), PluginHostError> {
        self.registry.install(manifest)?;
        Ok(())
    }

    /// Installs a plugin manifest from a bundle root as disabled.
    pub fn install_with_bundle_root(
        &mut self,
        manifest: PluginManifest,
        bundle_root: impl Into<String>,
    ) -> Result<(), PluginHostError> {
        self.registry
            .install_with_bundle_root(manifest, bundle_root)?;
        Ok(())
    }

    /// Enables and loads an installed plugin.
    pub async fn enable(&mut self, plugin_id: &str) -> Result<(), PluginHostError> {
        let record = self.registry.get(plugin_id)?;
        let request = plugin_load_request_from_record(record);
        let handle = self.adapter.load(&request).await?;
        self.registry.enable(plugin_id)?;
        self.handles.insert(plugin_id.to_owned(), handle);
        Ok(())
    }

    /// Disables and unloads an installed plugin.
    pub async fn disable(&mut self, plugin_id: &str) -> Result<(), PluginHostError> {
        self.registry.disable(plugin_id)?;

        if let Some(handle) = self.handles.remove(plugin_id) {
            self.adapter.unload(handle).await?;
        }

        Ok(())
    }

    /// Unloads and uninstalls a plugin.
    pub async fn uninstall(&mut self, plugin_id: &str) -> Result<(), PluginHostError> {
        if let Some(handle) = self.handles.remove(plugin_id) {
            self.adapter.unload(handle).await?;
        }

        self.registry.uninstall(plugin_id)?;
        Ok(())
    }

    /// Invokes an enabled plugin.
    pub async fn invoke(
        &self,
        plugin_id: &str,
        invocation: PluginInvocation,
    ) -> Result<PluginOutput, PluginHostError> {
        let record = self.registry.get(plugin_id)?;
        if !record.enabled() {
            return Err(PluginHostError::NotEnabled(plugin_id.to_owned()));
        }

        let handle = self
            .handles
            .get(plugin_id)
            .ok_or_else(|| RuntimeError::PluginNotLoaded(plugin_id.to_owned()))?;

        Ok(self.adapter.invoke(handle, invocation).await?)
    }
}

fn plugin_load_request_from_record(record: &crate::PluginRecord) -> PluginLoadRequest {
    match record.bundle_root() {
        Some(bundle_root) => {
            PluginLoadRequest::with_bundle_root(record.manifest().clone(), bundle_root)
        }
        None => PluginLoadRequest::new(record.manifest().clone()),
    }
}

/// Error returned by plugin host operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginHostError {
    /// Registry operation failed.
    Registry(PluginRegistryError),
    /// Runtime Adapter failed.
    Runtime(RuntimeError),
    /// Plugin is installed but not enabled.
    NotEnabled(String),
}

impl std::fmt::Display for PluginHostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Registry(error) => write!(f, "plugin registry error: {error}"),
            Self::Runtime(error) => write!(f, "plugin runtime error: {error}"),
            Self::NotEnabled(plugin_id) => write!(f, "plugin `{plugin_id}` is not enabled"),
        }
    }
}

impl std::error::Error for PluginHostError {}

impl From<PluginRegistryError> for PluginHostError {
    fn from(error: PluginRegistryError) -> Self {
        Self::Registry(error)
    }
}

impl From<RuntimeError> for PluginHostError {
    fn from(error: RuntimeError) -> Self {
        Self::Runtime(error)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use serde_json::json;

    use super::*;
    use crate::{Capability, PluginAction, RuntimeKind};

    #[derive(Debug, Default)]
    struct FakeRuntime {
        loaded: Mutex<Vec<String>>,
        bundle_roots: Mutex<Vec<Option<String>>>,
    }

    #[async_trait]
    impl RuntimeAdapter for FakeRuntime {
        async fn load(&self, request: &PluginLoadRequest) -> Result<RuntimeHandle, RuntimeError> {
            let manifest = request.manifest();
            self.loaded.lock().unwrap().push(manifest.id.clone());
            self.bundle_roots
                .lock()
                .unwrap()
                .push(request.bundle_root().map(str::to_owned));
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

        async fn unload(&self, handle: RuntimeHandle) -> Result<(), RuntimeError> {
            self.loaded
                .lock()
                .unwrap()
                .retain(|plugin_id| plugin_id != handle.plugin_id());
            Ok(())
        }
    }

    fn manifest() -> PluginManifest {
        PluginManifest {
            id: "com.example.plugin".to_owned(),
            name: "Plugin".to_owned(),
            version: "1.0.0".to_owned(),
            runtime: RuntimeKind::Wasm,
            entry: "dist/plugin.wasm".to_owned(),
            api_version: "1".to_owned(),
            capabilities: vec![Capability::DeviceRead],
        }
    }

    #[tokio::test]
    async fn enables_and_invokes_plugin() {
        let mut host = PluginHost::new(FakeRuntime::default());
        host.install(manifest()).unwrap();
        host.enable("com.example.plugin").await.unwrap();

        let output = host
            .invoke(
                "com.example.plugin",
                PluginInvocation::new("device.connected", json!({"deviceId": "coyote"})),
            )
            .await
            .unwrap();

        assert_eq!(
            output.actions[0].method,
            "com.example.plugin.device.connected"
        );
        assert!(host.registry().get("com.example.plugin").unwrap().enabled());
    }

    #[tokio::test]
    async fn enables_plugin_with_bundle_root() {
        let mut host = PluginHost::new(FakeRuntime::default());
        host.install_with_bundle_root(manifest(), "/plugins/com.example.plugin")
            .unwrap();

        host.enable("com.example.plugin").await.unwrap();

        assert_eq!(
            *host.adapter().bundle_roots.lock().unwrap(),
            vec![Some("/plugins/com.example.plugin".to_owned())]
        );
    }

    #[tokio::test]
    async fn disables_and_unloads_plugin() {
        let mut host = PluginHost::new(FakeRuntime::default());
        host.install(manifest()).unwrap();
        host.enable("com.example.plugin").await.unwrap();
        host.disable("com.example.plugin").await.unwrap();

        assert!(!host.registry().get("com.example.plugin").unwrap().enabled());
    }

    #[tokio::test]
    async fn uninstalls_and_unloads_plugin() {
        let mut host = PluginHost::new(FakeRuntime::default());
        host.install(manifest()).unwrap();
        host.enable("com.example.plugin").await.unwrap();
        host.uninstall("com.example.plugin").await.unwrap();

        assert!(matches!(
            host.registry().get("com.example.plugin"),
            Err(PluginRegistryError::NotInstalled(_))
        ));
        assert!(host.adapter().loaded.lock().unwrap().is_empty());
    }
}
