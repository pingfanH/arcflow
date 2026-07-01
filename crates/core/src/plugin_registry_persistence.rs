//! Persistence bridge between plugin-runtime registry and SQLite storage.

use core::fmt;
use std::collections::{BTreeMap, BTreeSet};

use arcflow_plugin_runtime::{Capability, PluginManifest, PluginRegistry, RuntimeKind};
use arcflow_storage::{Storage, StorageError, StoredPluginRecord};
use serde::Serialize;

/// Plugin registry entry returned to platform UI and IPC callers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRegistryEntry {
    /// Stable plugin id.
    pub id: String,
    /// Human-readable plugin name.
    pub name: String,
    /// Plugin version string.
    pub version: String,
    /// Runtime required by this plugin.
    pub runtime: String,
    /// Plugin API version requested by this plugin.
    pub api_version: String,
    /// Capabilities requested by this plugin.
    pub capabilities: Vec<String>,
    /// Whether this plugin should be enabled at startup.
    pub enabled: bool,
    /// Plugin bundle root directory if installed from disk.
    pub bundle_root: Option<String>,
}

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

    /// Lists the persisted plugin registry as stable IPC-friendly records.
    pub fn list_entries(&self) -> Result<Vec<PluginRegistryEntry>, PluginRegistryPersistenceError> {
        self.storage
            .plugin_registry()
            .list()?
            .into_iter()
            .map(plugin_registry_entry_from_stored)
            .collect()
    }

    /// Installs a plugin manifest JSON document and persists the registry.
    pub fn install_manifest_json(
        &self,
        manifest_json: &str,
    ) -> Result<Vec<PluginRegistryEntry>, PluginRegistryPersistenceError> {
        self.install_manifest_json_with_bundle_root(manifest_json, None)
    }

    /// Installs a plugin manifest JSON document with an optional bundle root.
    pub fn install_manifest_json_with_bundle_root(
        &self,
        manifest_json: &str,
        bundle_root: Option<String>,
    ) -> Result<Vec<PluginRegistryEntry>, PluginRegistryPersistenceError> {
        let manifest = PluginManifest::from_json(manifest_json)
            .map_err(|error| PluginRegistryPersistenceError::Manifest(error.to_string()))?;
        let plugin_id = manifest.id.clone();
        let mut registry = self.load()?;

        registry
            .install(manifest)
            .map_err(|error| PluginRegistryPersistenceError::Registry(error.to_string()))?;
        self.save(&registry)?;
        if let Some(bundle_root) = bundle_root {
            let store = self.storage.plugin_registry();
            let stored = store.get(&plugin_id)?.ok_or_else(|| {
                PluginRegistryPersistenceError::Storage(format!(
                    "plugin `{plugin_id}` was not persisted after install"
                ))
            })?;
            store.upsert(&StoredPluginRecord::with_bundle_root(
                stored.plugin_id,
                stored.manifest_json,
                stored.enabled,
                bundle_root,
            ))?;
        }

        self.list_entries()
    }

    /// Updates a plugin enabled flag and persists the registry.
    pub fn set_enabled(
        &self,
        plugin_id: &str,
        enabled: bool,
    ) -> Result<Vec<PluginRegistryEntry>, PluginRegistryPersistenceError> {
        let mut registry = self.load()?;

        if enabled {
            registry
                .enable(plugin_id)
                .map_err(|error| PluginRegistryPersistenceError::Registry(error.to_string()))?;
        } else {
            registry
                .disable(plugin_id)
                .map_err(|error| PluginRegistryPersistenceError::Registry(error.to_string()))?;
        }

        self.save(&registry)?;

        self.list_entries()
    }

    /// Deletes a plugin from the persisted registry.
    pub fn delete(
        &self,
        plugin_id: &str,
    ) -> Result<Vec<PluginRegistryEntry>, PluginRegistryPersistenceError> {
        let mut registry = self.load()?;

        registry
            .uninstall(plugin_id)
            .map_err(|error| PluginRegistryPersistenceError::Registry(error.to_string()))?;
        self.save(&registry)?;

        self.list_entries()
    }

    /// Saves the current plugin-runtime registry as the persisted registry.
    pub fn save(&self, registry: &PluginRegistry) -> Result<(), PluginRegistryPersistenceError> {
        let store = self.storage.plugin_registry();
        let stored_records = store.list()?;
        let existing_bundle_roots: BTreeMap<String, Option<String>> = stored_records
            .iter()
            .map(|stored| (stored.plugin_id.clone(), stored.bundle_root.clone()))
            .collect();
        let active_ids: BTreeSet<String> = registry
            .list()
            .into_iter()
            .map(|record| record.manifest().id.clone())
            .collect();

        for stored in stored_records {
            if !active_ids.contains(&stored.plugin_id) {
                store.delete(&stored.plugin_id)?;
            }
        }

        for record in registry.list() {
            let manifest_json = serde_json::to_string(record.manifest())
                .map_err(|error| PluginRegistryPersistenceError::Json(error.to_string()))?;
            let plugin_id = record.manifest().id.clone();
            let bundle_root = existing_bundle_roots.get(&plugin_id).cloned().flatten();
            store.upsert(&stored_plugin_record(
                plugin_id,
                manifest_json,
                record.enabled(),
                bundle_root,
            ))?;
        }

        Ok(())
    }
}

fn plugin_registry_entry_from_stored(
    stored: StoredPluginRecord,
) -> Result<PluginRegistryEntry, PluginRegistryPersistenceError> {
    let manifest = PluginManifest::from_json(&stored.manifest_json)
        .map_err(|error| PluginRegistryPersistenceError::Manifest(error.to_string()))?;

    Ok(PluginRegistryEntry {
        id: manifest.id.clone(),
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        runtime: runtime_name(manifest.runtime).to_owned(),
        api_version: manifest.api_version.clone(),
        capabilities: manifest
            .capabilities
            .iter()
            .map(|capability| capability_name(*capability).to_owned())
            .collect(),
        enabled: stored.enabled,
        bundle_root: stored.bundle_root,
    })
}

fn stored_plugin_record(
    plugin_id: String,
    manifest_json: String,
    enabled: bool,
    bundle_root: Option<String>,
) -> StoredPluginRecord {
    match bundle_root {
        Some(bundle_root) => {
            StoredPluginRecord::with_bundle_root(plugin_id, manifest_json, enabled, bundle_root)
        }
        None => StoredPluginRecord::new(plugin_id, manifest_json, enabled),
    }
}

fn runtime_name(runtime: RuntimeKind) -> &'static str {
    match runtime {
        RuntimeKind::Wasm => "wasm",
        RuntimeKind::JavaScript => "javascript",
    }
}

fn capability_name(capability: Capability) -> &'static str {
    capability.as_str()
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

    #[test]
    fn installs_manifest_json_and_returns_entries() {
        let storage = Storage::in_memory().unwrap();
        let persistence = PluginRegistryPersistence::new(&storage);
        let manifest_json = serde_json::to_string(&manifest("plugin.a")).unwrap();

        let entries = persistence.install_manifest_json(&manifest_json).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "plugin.a");
        assert_eq!(entries[0].runtime, "wasm");
        assert_eq!(entries[0].capabilities, vec!["device.read"]);
        assert!(!entries[0].enabled);
        assert_eq!(entries[0].bundle_root, None);
        assert!(storage.plugin_registry().get("plugin.a").unwrap().is_some());
    }

    #[test]
    fn installs_manifest_json_with_bundle_root() {
        let storage = Storage::in_memory().unwrap();
        let persistence = PluginRegistryPersistence::new(&storage);
        let manifest_json = serde_json::to_string(&manifest("plugin.a")).unwrap();

        let entries = persistence
            .install_manifest_json_with_bundle_root(
                &manifest_json,
                Some("/plugins/plugin.a".to_owned()),
            )
            .unwrap();

        assert_eq!(entries[0].bundle_root, Some("/plugins/plugin.a".to_owned()));
        assert_eq!(
            storage
                .plugin_registry()
                .get("plugin.a")
                .unwrap()
                .unwrap()
                .bundle_root,
            Some("/plugins/plugin.a".to_owned())
        );
    }

    #[test]
    fn save_preserves_existing_bundle_root() {
        let storage = Storage::in_memory().unwrap();
        let persistence = PluginRegistryPersistence::new(&storage);
        let mut registry = PluginRegistry::new();
        registry.install(manifest("plugin.a")).unwrap();
        storage
            .plugin_registry()
            .upsert(&StoredPluginRecord::with_bundle_root(
                "plugin.a",
                serde_json::to_string(&manifest("plugin.a")).unwrap(),
                false,
                "/plugins/plugin.a",
            ))
            .unwrap();

        persistence.save(&registry).unwrap();

        assert_eq!(
            storage
                .plugin_registry()
                .get("plugin.a")
                .unwrap()
                .unwrap()
                .bundle_root,
            Some("/plugins/plugin.a".to_owned())
        );
    }

    #[test]
    fn set_enabled_updates_persisted_registry() {
        let storage = Storage::in_memory().unwrap();
        let persistence = PluginRegistryPersistence::new(&storage);
        let manifest_json = serde_json::to_string(&manifest("plugin.a")).unwrap();

        persistence.install_manifest_json(&manifest_json).unwrap();
        let entries = persistence.set_enabled("plugin.a", true).unwrap();

        assert!(entries[0].enabled);
        assert!(persistence
            .load()
            .unwrap()
            .get("plugin.a")
            .unwrap()
            .enabled());
    }

    #[test]
    fn deletes_plugin_manifest() {
        let storage = Storage::in_memory().unwrap();
        let persistence = PluginRegistryPersistence::new(&storage);
        let manifest_json = serde_json::to_string(&manifest("plugin.a")).unwrap();

        persistence.install_manifest_json(&manifest_json).unwrap();
        let entries = persistence.delete("plugin.a").unwrap();

        assert!(entries.is_empty());
        assert!(storage.plugin_registry().get("plugin.a").unwrap().is_none());
    }
}
