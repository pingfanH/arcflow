//! Installed plugin registry.

use std::collections::BTreeMap;

use crate::{PluginManifest, RuntimeKind};

/// Installed plugin record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRecord {
    manifest: PluginManifest,
    enabled: bool,
}

impl PluginRecord {
    /// Constructs an installed, disabled plugin record.
    #[must_use]
    pub fn installed(manifest: PluginManifest) -> Self {
        Self {
            manifest,
            enabled: false,
        }
    }

    /// Returns the plugin manifest.
    #[must_use]
    pub fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    /// Returns whether this plugin is enabled.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Returns the plugin runtime kind.
    #[must_use]
    pub fn runtime(&self) -> RuntimeKind {
        self.manifest.runtime
    }
}

/// Registry of installed plugins.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PluginRegistry {
    records: BTreeMap<String, PluginRecord>,
}

impl PluginRegistry {
    /// Constructs an empty plugin registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Installs a plugin manifest as disabled.
    pub fn install(&mut self, manifest: PluginManifest) -> Result<(), PluginRegistryError> {
        manifest
            .validate()
            .map_err(|error| PluginRegistryError::InvalidManifest(error.to_string()))?;

        if self.records.contains_key(&manifest.id) {
            return Err(PluginRegistryError::AlreadyInstalled(manifest.id));
        }

        self.records
            .insert(manifest.id.clone(), PluginRecord::installed(manifest));
        Ok(())
    }

    /// Enables an installed plugin.
    pub fn enable(&mut self, plugin_id: &str) -> Result<(), PluginRegistryError> {
        self.get_mut(plugin_id)?.enabled = true;
        Ok(())
    }

    /// Disables an installed plugin.
    pub fn disable(&mut self, plugin_id: &str) -> Result<(), PluginRegistryError> {
        self.get_mut(plugin_id)?.enabled = false;
        Ok(())
    }

    /// Uninstalls a plugin record.
    pub fn uninstall(&mut self, plugin_id: &str) -> Result<PluginRecord, PluginRegistryError> {
        self.records
            .remove(plugin_id)
            .ok_or_else(|| PluginRegistryError::NotInstalled(plugin_id.to_owned()))
    }

    /// Returns an installed plugin record.
    pub fn get(&self, plugin_id: &str) -> Result<&PluginRecord, PluginRegistryError> {
        self.records
            .get(plugin_id)
            .ok_or_else(|| PluginRegistryError::NotInstalled(plugin_id.to_owned()))
    }

    /// Lists installed plugins in stable id order.
    #[must_use]
    pub fn list(&self) -> Vec<&PluginRecord> {
        self.records.values().collect()
    }

    fn get_mut(&mut self, plugin_id: &str) -> Result<&mut PluginRecord, PluginRegistryError> {
        self.records
            .get_mut(plugin_id)
            .ok_or_else(|| PluginRegistryError::NotInstalled(plugin_id.to_owned()))
    }
}

/// Error returned by plugin registry operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginRegistryError {
    /// Manifest validation failed.
    InvalidManifest(String),
    /// Plugin id is already installed.
    AlreadyInstalled(String),
    /// Plugin id is not installed.
    NotInstalled(String),
}

impl std::fmt::Display for PluginRegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidManifest(error) => write!(f, "invalid plugin manifest: {error}"),
            Self::AlreadyInstalled(plugin_id) => {
                write!(f, "plugin `{plugin_id}` is already installed")
            }
            Self::NotInstalled(plugin_id) => write!(f, "plugin `{plugin_id}` is not installed"),
        }
    }
}

impl std::error::Error for PluginRegistryError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Capability, RuntimeKind};

    fn manifest(id: &str) -> PluginManifest {
        PluginManifest {
            id: id.to_owned(),
            name: "Pulse Tools".to_owned(),
            version: "1.0.0".to_owned(),
            runtime: RuntimeKind::Wasm,
            entry: "dist/plugin.wasm".to_owned(),
            api_version: "1".to_owned(),
            capabilities: vec![Capability::DeviceRead],
        }
    }

    #[test]
    fn installs_plugins_disabled() {
        let mut registry = PluginRegistry::new();

        registry.install(manifest("com.example.a")).unwrap();

        let record = registry.get("com.example.a").unwrap();
        assert!(!record.enabled());
        assert_eq!(record.runtime(), RuntimeKind::Wasm);
    }

    #[test]
    fn enables_and_disables_plugins() {
        let mut registry = PluginRegistry::new();

        registry.install(manifest("com.example.a")).unwrap();
        registry.enable("com.example.a").unwrap();
        assert!(registry.get("com.example.a").unwrap().enabled());

        registry.disable("com.example.a").unwrap();
        assert!(!registry.get("com.example.a").unwrap().enabled());
    }

    #[test]
    fn rejects_duplicate_install() {
        let mut registry = PluginRegistry::new();

        registry.install(manifest("com.example.a")).unwrap();
        let error = registry.install(manifest("com.example.a")).unwrap_err();

        assert_eq!(
            error,
            PluginRegistryError::AlreadyInstalled("com.example.a".to_owned())
        );
    }

    #[test]
    fn uninstalls_plugins() {
        let mut registry = PluginRegistry::new();

        registry.install(manifest("com.example.a")).unwrap();
        let removed = registry.uninstall("com.example.a").unwrap();

        assert_eq!(removed.manifest().id, "com.example.a");
        assert!(matches!(
            registry.get("com.example.a"),
            Err(PluginRegistryError::NotInstalled(_))
        ));
    }
}
