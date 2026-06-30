//! Host API exposed to sandboxed plugins through Rust Core.

use core::fmt;

use arcflow_plugin_runtime::{Capability, PluginAction, PluginManifest, PluginOutput};
use arcflow_storage::{Storage, StorageError};
use serde::Deserialize;
use serde_json::{json, Value};

/// Core-owned plugin host API.
pub struct PluginApi<'a> {
    storage: &'a Storage,
}

impl<'a> PluginApi<'a> {
    /// Constructs a plugin API over Core-owned storage.
    #[must_use]
    pub fn new(storage: &'a Storage) -> Self {
        Self { storage }
    }

    /// Handles one action requested by a sandboxed plugin.
    pub fn handle_action(
        &self,
        manifest: &PluginManifest,
        action: PluginAction,
    ) -> Result<Value, PluginApiError> {
        match action.method.as_str() {
            "storage.private.put" => self.storage_put(manifest, action.params),
            "storage.private.get" => self.storage_get(manifest, action.params),
            "storage.private.delete" => self.storage_delete(manifest, action.params),
            "storage.private.keys" => self.storage_keys(manifest),
            method => Err(PluginApiError::UnknownMethod(method.to_owned())),
        }
    }

    /// Handles every action returned by a plugin invocation in order.
    pub fn handle_output(
        &self,
        manifest: &PluginManifest,
        output: PluginOutput,
    ) -> Result<Vec<Value>, PluginApiError> {
        output
            .actions
            .into_iter()
            .map(|action| self.handle_action(manifest, action))
            .collect()
    }

    fn storage_put(
        &self,
        manifest: &PluginManifest,
        params: Value,
    ) -> Result<Value, PluginApiError> {
        ensure_capability(manifest, Capability::StoragePrivate)?;
        let params: StoragePutParams = read_params("storage.private.put", params)?;
        let bytes = serde_json::to_vec(&params.value)
            .map_err(|error| PluginApiError::InvalidParams(error.to_string()))?;

        self.storage
            .plugin_kv()
            .put(&manifest.id, &params.key, &bytes)?;

        Ok(json!({
            "key": params.key,
            "stored": true,
        }))
    }

    fn storage_get(
        &self,
        manifest: &PluginManifest,
        params: Value,
    ) -> Result<Value, PluginApiError> {
        ensure_capability(manifest, Capability::StoragePrivate)?;
        let params: StorageKeyParams = read_params("storage.private.get", params)?;
        let value = self
            .storage
            .plugin_kv()
            .get(&manifest.id, &params.key)?
            .map(|bytes| serde_json::from_slice::<Value>(&bytes))
            .transpose()
            .map_err(|error| PluginApiError::Storage(error.to_string()))?;

        Ok(json!({
            "key": params.key,
            "value": value,
        }))
    }

    fn storage_delete(
        &self,
        manifest: &PluginManifest,
        params: Value,
    ) -> Result<Value, PluginApiError> {
        ensure_capability(manifest, Capability::StoragePrivate)?;
        let params: StorageKeyParams = read_params("storage.private.delete", params)?;

        self.storage.plugin_kv().delete(&manifest.id, &params.key)?;

        Ok(json!({
            "key": params.key,
            "deleted": true,
        }))
    }

    fn storage_keys(&self, manifest: &PluginManifest) -> Result<Value, PluginApiError> {
        ensure_capability(manifest, Capability::StoragePrivate)?;
        let keys = self.storage.plugin_kv().list_keys(&manifest.id)?;

        Ok(json!({
            "keys": keys,
        }))
    }
}

/// Error returned by Core plugin API operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginApiError {
    /// Plugin requested a method without declaring the required capability.
    CapabilityDenied {
        /// Plugin id.
        plugin_id: String,
        /// Capability required by the method.
        capability: Capability,
    },
    /// Plugin requested a host method that is not exposed.
    UnknownMethod(String),
    /// Plugin passed malformed params.
    InvalidParams(String),
    /// Core-owned storage failed.
    Storage(String),
}

impl fmt::Display for PluginApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CapabilityDenied {
                plugin_id,
                capability,
            } => write!(
                f,
                "plugin `{plugin_id}` is missing required capability `{}`",
                capability.as_str()
            ),
            Self::UnknownMethod(method) => write!(f, "unknown plugin host method `{method}`"),
            Self::InvalidParams(error) => write!(f, "invalid plugin host params: {error}"),
            Self::Storage(error) => write!(f, "plugin storage error: {error}"),
        }
    }
}

impl std::error::Error for PluginApiError {}

impl From<StorageError> for PluginApiError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error.to_string())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoragePutParams {
    key: String,
    value: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StorageKeyParams {
    key: String,
}

fn ensure_capability(
    manifest: &PluginManifest,
    capability: Capability,
) -> Result<(), PluginApiError> {
    if manifest.capabilities.contains(&capability) {
        Ok(())
    } else {
        Err(PluginApiError::CapabilityDenied {
            plugin_id: manifest.id.clone(),
            capability,
        })
    }
}

fn read_params<T>(method: &'static str, params: Value) -> Result<T, PluginApiError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value(params).map_err(|error| {
        PluginApiError::InvalidParams(format!("invalid params for `{method}`: {error}"))
    })
}

#[cfg(test)]
mod tests {
    use arcflow_plugin_runtime::{PluginAction, RuntimeKind};
    use arcflow_storage::Storage;
    use serde_json::json;

    use super::*;

    fn manifest_with(capabilities: Vec<Capability>) -> PluginManifest {
        PluginManifest {
            id: "com.example.plugin".to_owned(),
            name: "Plugin".to_owned(),
            version: "1.0.0".to_owned(),
            runtime: RuntimeKind::Wasm,
            entry: "dist/plugin.wasm".to_owned(),
            api_version: "1".to_owned(),
            capabilities,
        }
    }

    #[test]
    fn stores_and_reads_plugin_private_json() {
        let storage = Storage::in_memory().unwrap();
        let api = PluginApi::new(&storage);
        let manifest = manifest_with(vec![Capability::StoragePrivate]);

        api.handle_action(
            &manifest,
            PluginAction::new(
                "storage.private.put",
                json!({
                    "key": "settings",
                    "value": {"enabled": true}
                }),
            ),
        )
        .unwrap();

        let result = api
            .handle_action(
                &manifest,
                PluginAction::new("storage.private.get", json!({ "key": "settings" })),
            )
            .unwrap();

        assert_eq!(
            result,
            json!({
                "key": "settings",
                "value": {"enabled": true}
            })
        );
    }

    #[test]
    fn lists_and_deletes_plugin_private_keys() {
        let storage = Storage::in_memory().unwrap();
        let api = PluginApi::new(&storage);
        let manifest = manifest_with(vec![Capability::StoragePrivate]);

        api.handle_action(
            &manifest,
            PluginAction::new("storage.private.put", json!({ "key": "b", "value": 1 })),
        )
        .unwrap();
        api.handle_action(
            &manifest,
            PluginAction::new("storage.private.put", json!({ "key": "a", "value": 2 })),
        )
        .unwrap();

        let keys = api
            .handle_action(
                &manifest,
                PluginAction::new("storage.private.keys", json!({})),
            )
            .unwrap();

        assert_eq!(keys, json!({ "keys": ["a", "b"] }));

        api.handle_action(
            &manifest,
            PluginAction::new("storage.private.delete", json!({ "key": "a" })),
        )
        .unwrap();

        let keys = api
            .handle_action(
                &manifest,
                PluginAction::new("storage.private.keys", json!({})),
            )
            .unwrap();

        assert_eq!(keys, json!({ "keys": ["b"] }));
    }

    #[test]
    fn handles_plugin_output_actions_in_order() {
        let storage = Storage::in_memory().unwrap();
        let api = PluginApi::new(&storage);
        let manifest = manifest_with(vec![Capability::StoragePrivate]);

        let results = api
            .handle_output(
                &manifest,
                PluginOutput::new(vec![
                    PluginAction::new(
                        "storage.private.put",
                        json!({ "key": "theme", "value": "dark" }),
                    ),
                    PluginAction::new("storage.private.get", json!({ "key": "theme" })),
                ]),
            )
            .unwrap();

        assert_eq!(
            results,
            vec![
                json!({ "key": "theme", "stored": true }),
                json!({ "key": "theme", "value": "dark" }),
            ]
        );
    }

    #[test]
    fn rejects_storage_without_capability() {
        let storage = Storage::in_memory().unwrap();
        let api = PluginApi::new(&storage);
        let manifest = manifest_with(vec![Capability::DeviceRead]);

        let error = api
            .handle_action(
                &manifest,
                PluginAction::new("storage.private.keys", json!({})),
            )
            .unwrap_err();

        assert_eq!(
            error,
            PluginApiError::CapabilityDenied {
                plugin_id: "com.example.plugin".to_owned(),
                capability: Capability::StoragePrivate,
            }
        );
    }
}
