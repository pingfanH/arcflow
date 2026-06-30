//! Persistence bridge between plugin-runtime registry and SQLite storage.

use core::fmt;
use std::collections::BTreeSet;

use arcflow_plugin_runtime::{PluginManifest, PluginRegistry};
use arcflow_storage::{Storage, StorageError, StoredPluginRecord};

/// Core-owned plugin registry persistence bridge.
pub struct PluginRegistryPersistence<'a> {
    storage: &'a Storage,
}

impl<'a> PluginRegistryPersistence<'a> {
    /// Constructs a plugin registry persistence bridge.
    #[must_use]
    pub fn new(storage: &'a Storage) -> Self {
        Self { storage }
    }

    /// Loads the persisted plugin registry into the plugin-runtime model.
    pub fn load(&self) -> Result<PluginRegistry, PluginRegistryPersistenceError> {
        let mut registry = PluginRegistry::new();

        for stored in self.storage.plugin_registry().list()? {
            let manifest = PluginManifest::from_json(&stored.manifest_json)
                .map_err(|error| PluginRegistryPersistenceError::Manifest(error.to_string()))?;
            let plugin_id = manifest.id.clone();

            registry
                .install(manifest)
                .map_err(|error| PluginRegistryPersistenceError::Registry(error.to_string()))?;

            if stored.enabled {
                registry
                    .enable(&plugin_id)
                    .map_err(|error| PluginRegistryPersistenceError::Registry(error.to_string()))?;
            }
        }

        Ok(registry)
    }

    /// Saves the current plugin-runtime registry as the persisted registry.
    pub fn save(&self, registry: &PluginRegistry) -> Result<(), PluginRegistryPersistenceError> {
        let store = self.storage.plugin_registry();
        let active_ids: BTreeSet<String> = registry
            .list()
            .into_iter()
            .map(|record| record.manifest().id.clone())
            .collect();

        for stored in store.list()? {
            if !active_ids.contains(&stored.plugin_id) {
                store.delete(&stored.plugin_id)?;
            }
        }

        for record in registry.list() {
            let manifest_json = serde_json::to_string(record.manifest())
                .map_err(|error| PluginRegistryPersistenceError::Json(error.to_string()))?;
            store.upsert(&StoredPluginRecord::new(
                record.manifest().id.clone(),
                manifest_json,
                record.enabled(),
            ))?;
        }

        Ok(())
    }
}

/// Error returned by plugin registry persistence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginRegistryPersistenceError {
    /// SQLite storage failed.
    Storage(String),
    /// Persisted manifest JSON could not be parsed or validated.
    Manifest(String),
    /// Runtime registry rejected a persisted record.
    Registry(String),
    /// Manifest serialization failed.
    Json(String),
}

impl fmt::Display for PluginRegistryPersistenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(error) => write!(f, "plugin registry storage error: {error}"),
            Self::Manifest(error) => write!(f, "persisted plugin manifest error: {error}"),
            Self::Registry(error) => write!(f, "plugin registry restore error: {error}"),
            Self::Json(error) => write!(f, "plugin registry serialization error: {error}"),
        }
    }
}

impl std::error::Error for PluginRegistryPersistenceError {}

impl From<StorageError> for PluginRegistryPersistenceError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use arcflow_plugin_runtime::{Capability, PluginManifest, RuntimeKind};
    use arcflow_storage::Storage;

    use super::*;

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
    fn saves_and_loads_plugin_registry() {
        let storage = Storage::in_memory().unwrap();
        let persistence = PluginRegistryPersistence::new(&storage);
        let mut registry = PluginRegistry::new();

        registry.install(manifest("plugin.a")).unwrap();
        registry.enable("plugin.a").unwrap();
        registry.install(manifest("plugin.b")).unwrap();

        persistence.save(&registry).unwrap();
        let restored = persistence.load().unwrap();

        assert!(restored.get("plugin.a").unwrap().enabled());
        assert!(!restored.get("plugin.b").unwrap().enabled());
    }

    #[test]
    fn save_removes_stale_persisted_records() {
        let storage = Storage::in_memory().unwrap();
        let persistence = PluginRegistryPersistence::new(&storage);
        storage
            .plugin_registry()
            .upsert(&StoredPluginRecord::new(
                "plugin.stale",
                serde_json::to_string(&manifest("plugin.stale")).unwrap(),
                true,
            ))
            .unwrap();

        persistence.save(&PluginRegistry::new()).unwrap();

        assert!(storage.plugin_registry().list().unwrap().is_empty());
    }
}
