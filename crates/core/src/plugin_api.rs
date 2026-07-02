//! Host API exposed to sandboxed plugins through Rust Core.

use core::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use arcflow_plugin_runtime::{Capability, PluginAction, PluginManifest, PluginOutput};
use arcflow_storage::{Storage, StorageError};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::wave_submit::coyote_v3_output_request_from_value;
use crate::{
    BleAdapterStatus, DeviceDiscoveryController, DeviceId, DeviceModel, DeviceOutputController,
    DeviceScanResult, DeviceStatus,
};

/// Core-owned plugin host API.
pub struct PluginApi {
    storage: Arc<Mutex<Storage>>,
    discovery_controller: Option<Arc<dyn DeviceDiscoveryController>>,
    output_controller: Option<Arc<dyn DeviceOutputController>>,
}

impl PluginApi {
    /// Constructs a plugin API over Core-owned storage.
    #[must_use]
    pub fn new(storage: Arc<Mutex<Storage>>) -> Self {
        Self {
            storage,
            discovery_controller: None,
            output_controller: None,
        }
    }

    /// Constructs a plugin API over Core-owned storage and output control.
    #[must_use]
    pub fn with_output_controller(
        storage: Arc<Mutex<Storage>>,
        output_controller: Arc<dyn DeviceOutputController>,
    ) -> Self {
        Self {
            storage,
            discovery_controller: None,
            output_controller: Some(output_controller),
        }
    }

    /// Constructs a plugin API over Core-owned storage and device discovery.
    #[must_use]
    pub fn with_discovery_controller(
        storage: Arc<Mutex<Storage>>,
        discovery_controller: Arc<dyn DeviceDiscoveryController>,
    ) -> Self {
        Self {
            storage,
            discovery_controller: Some(discovery_controller),
            output_controller: None,
        }
    }

    /// Constructs a plugin API over Core-owned storage and device controllers.
    #[must_use]
    pub fn with_host_controllers(
        storage: Arc<Mutex<Storage>>,
        discovery_controller: Arc<dyn DeviceDiscoveryController>,
        output_controller: Arc<dyn DeviceOutputController>,
    ) -> Self {
        Self {
            storage,
            discovery_controller: Some(discovery_controller),
            output_controller: Some(output_controller),
        }
    }

    /// Handles one action requested by a sandboxed plugin.
    pub async fn handle_action(
        &self,
        manifest: &PluginManifest,
        action: PluginAction,
    ) -> Result<Value, PluginApiError> {
        match action.method.as_str() {
            "storage.private.put" => self.storage_put(manifest, action.params),
            "storage.private.get" => self.storage_get(manifest, action.params),
            "storage.private.delete" => self.storage_delete(manifest, action.params),
            "storage.private.keys" => self.storage_keys(manifest),
            "device.scan" => self.device_scan(manifest).await,
            "device.status" => self.device_status(manifest, action.params).await,
            "device.activateOutput" => self.activate_output_device(manifest, action.params),
            "device.deactivateOutput" => self.deactivate_output_device(manifest, action.params),
            "wave.submitWindow" => self.submit_wave_window(manifest, action.params),
            "wave.stop" => self.wave_stop(manifest, action.params),
            method => Err(PluginApiError::UnknownMethod(method.to_owned())),
        }
    }

    /// Handles every action returned by a plugin invocation in order.
    pub async fn handle_output(
        &self,
        manifest: &PluginManifest,
        output: PluginOutput,
    ) -> Result<Vec<Value>, PluginApiError> {
        let mut results = Vec::with_capacity(output.actions.len());

        for action in output.actions {
            results.push(self.handle_action(manifest, action).await?);
        }

        Ok(results)
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

        self.storage()?
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
            .storage()?
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

        self.storage()?
            .plugin_kv()
            .delete(&manifest.id, &params.key)?;

        Ok(json!({
            "key": params.key,
            "deleted": true,
        }))
    }

    fn storage_keys(&self, manifest: &PluginManifest) -> Result<Value, PluginApiError> {
        ensure_capability(manifest, Capability::StoragePrivate)?;
        let keys = self.storage()?.plugin_kv().list_keys(&manifest.id)?;

        Ok(json!({
            "keys": keys,
        }))
    }

    async fn device_status(
        &self,
        manifest: &PluginManifest,
        params: Value,
    ) -> Result<Value, PluginApiError> {
        ensure_capability(manifest, Capability::DeviceRead)?;
        let params: DeviceParams = read_params("device.status", params)?;
        let discovery_controller = self
            .discovery_controller
            .as_ref()
            .ok_or_else(|| PluginApiError::HostUnavailable("device.status".to_owned()))?;
        let device_id = DeviceId::new(params.device_id);
        let scan = discovery_controller
            .scan_devices()
            .await
            .map_err(|error| PluginApiError::Host(error.to_string()))?;

        match scan
            .devices
            .iter()
            .find(|device| device.id.as_str() == device_id.as_str())
        {
            Some(device) => Ok(device_status_payload(device, scan.adapter_status)),
            None => Ok(json!({
                "deviceId": device_id.as_str(),
                "connected": false,
                "adapterStatus": scan.adapter_status.as_str(),
            })),
        }
    }

    async fn device_scan(&self, manifest: &PluginManifest) -> Result<Value, PluginApiError> {
        ensure_capability(manifest, Capability::DeviceRead)?;
        let discovery_controller = self
            .discovery_controller
            .as_ref()
            .ok_or_else(|| PluginApiError::HostUnavailable("device.scan".to_owned()))?;
        let scan = discovery_controller
            .scan_devices()
            .await
            .map_err(|error| PluginApiError::Host(error.to_string()))?;

        Ok(device_scan_payload(&scan))
    }

    fn activate_output_device(
        &self,
        manifest: &PluginManifest,
        params: Value,
    ) -> Result<Value, PluginApiError> {
        ensure_capability(manifest, Capability::WaveControl)?;
        let params: DeviceParams = read_params("device.activateOutput", params)?;
        let output_controller = self.output_controller("device.activateOutput")?;
        let device_id = DeviceId::new(params.device_id);

        output_controller
            .attach_output_device(device_id.clone())
            .map_err(|error| PluginApiError::Host(error.to_string()))?;

        Ok(json!({
            "accepted": true,
            "command": "device.activateOutput",
            "deviceId": device_id.as_str(),
            "activeOutputDevices": device_ids_payload(output_controller.active_output_devices()),
        }))
    }

    fn deactivate_output_device(
        &self,
        manifest: &PluginManifest,
        params: Value,
    ) -> Result<Value, PluginApiError> {
        ensure_capability(manifest, Capability::WaveControl)?;
        let params: DeviceParams = read_params("device.deactivateOutput", params)?;
        let output_controller = self.output_controller("device.deactivateOutput")?;
        let device_id = DeviceId::new(params.device_id);

        output_controller
            .detach_output_device(&device_id)
            .map_err(|error| PluginApiError::Host(error.to_string()))?;

        Ok(json!({
            "accepted": true,
            "command": "device.deactivateOutput",
            "deviceId": device_id.as_str(),
            "activeOutputDevices": device_ids_payload(output_controller.active_output_devices()),
        }))
    }

    fn submit_wave_window(
        &self,
        manifest: &PluginManifest,
        params: Value,
    ) -> Result<Value, PluginApiError> {
        ensure_capability(manifest, Capability::WaveControl)?;
        let request = coyote_v3_output_request_from_value("wave.submitWindow", params)
            .map_err(|error| PluginApiError::InvalidParams(error.to_string()))?;
        let output_controller = self.output_controller("wave.submitWindow")?;
        let result = output_controller
            .submit_coyote_v3_window(request)
            .map_err(|error| PluginApiError::Host(error.to_string()))?;

        Ok(json!({
            "accepted": result.accepted,
            "command": "wave.submitWindow",
            "deviceId": result.device_id.as_str(),
        }))
    }

    fn wave_stop(&self, manifest: &PluginManifest, params: Value) -> Result<Value, PluginApiError> {
        ensure_capability(manifest, Capability::WaveControl)?;
        let params: DeviceParams = read_params("wave.stop", params)?;
        let output_controller = self.output_controller("wave.stop")?;
        let device_id = DeviceId::new(params.device_id);
        let result = output_controller
            .stop_all_output()
            .map_err(|error| PluginApiError::Host(error.to_string()))?;
        let stopped = result
            .stopped_devices
            .iter()
            .any(|stopped_id| stopped_id == &device_id);

        Ok(json!({
            "accepted": true,
            "command": "wave.stop",
            "deviceId": device_id.as_str(),
            "stopped": stopped,
        }))
    }

    fn output_controller(
        &self,
        method: &'static str,
    ) -> Result<&Arc<dyn DeviceOutputController>, PluginApiError> {
        self.output_controller
            .as_ref()
            .ok_or_else(|| PluginApiError::HostUnavailable(method.to_owned()))
    }

    fn storage(&self) -> Result<MutexGuard<'_, Storage>, PluginApiError> {
        self.storage
            .lock()
            .map_err(|_| PluginApiError::Host("plugin storage mutex poisoned".to_owned()))
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
    /// A host surface required by the action has not been attached.
    HostUnavailable(String),
    /// A Core-owned host surface failed.
    Host(String),
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
            Self::HostUnavailable(method) => {
                write!(f, "plugin host method `{method}` is not attached")
            }
            Self::Host(error) => write!(f, "plugin host error: {error}"),
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceParams {
    device_id: String,
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

fn device_status_payload(device: &DeviceStatus, adapter_status: BleAdapterStatus) -> Value {
    json!({
        "deviceId": device.id.as_str(),
        "connected": device.connected,
        "adapterStatus": adapter_status.as_str(),
        "batteryPercent": device.battery_percent,
        "channelAStrength": device.channel_a_strength,
        "channelBStrength": device.channel_b_strength,
    })
}

fn device_scan_payload(scan: &DeviceScanResult) -> Value {
    json!({
        "adapterStatus": scan.adapter_status.as_str(),
        "devices": scan.devices.iter().map(device_scan_item_payload).collect::<Vec<_>>(),
    })
}

fn device_scan_item_payload(device: &DeviceStatus) -> Value {
    json!({
        "deviceId": device.id.as_str(),
        "model": device_model_name(&device.model),
        "connected": device.connected,
        "batteryPercent": device.battery_percent,
        "channelAStrength": device.channel_a_strength,
        "channelBStrength": device.channel_b_strength,
    })
}

fn device_model_name(model: &DeviceModel) -> &str {
    match model {
        DeviceModel::CoyoteV2 => "coyoteV2",
        DeviceModel::CoyoteV3 => "coyoteV3",
        DeviceModel::Unknown(name) => name.as_str(),
    }
}

fn device_ids_payload(device_ids: Vec<DeviceId>) -> Vec<String> {
    device_ids
        .into_iter()
        .map(|device_id| device_id.as_str().to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use arcflow_plugin_runtime::{PluginAction, RuntimeKind};
    use arcflow_protocol::coyote::v3::{StrengthMode, StrengthModes};
    use arcflow_storage::Storage;
    use arcflow_wave::{ChannelOutput, ChannelWindow, CoyoteV3Window, WavePoint};
    use async_trait::async_trait;
    use serde_json::json;

    use super::*;
    use crate::CoyoteV3OutputRequest;

    #[derive(Debug)]
    struct FakeDiscoveryController {
        scan: crate::DeviceScanResult,
        scan_count: Mutex<usize>,
    }

    impl FakeDiscoveryController {
        fn new(scan: crate::DeviceScanResult) -> Self {
            Self {
                scan,
                scan_count: Mutex::new(0),
            }
        }
    }

    #[async_trait]
    impl DeviceDiscoveryController for FakeDiscoveryController {
        async fn scan_devices(&self) -> Result<crate::DeviceScanResult, crate::CoreError> {
            *self.scan_count.lock().unwrap() += 1;
            Ok(self.scan.clone())
        }
    }

    #[derive(Debug)]
    struct FakeOutputController {
        active_devices: Mutex<Vec<DeviceId>>,
        submitted_requests: Mutex<Vec<CoyoteV3OutputRequest>>,
        stopped_devices: Vec<DeviceId>,
        stop_count: Mutex<usize>,
    }

    impl FakeOutputController {
        fn new(stopped_devices: Vec<DeviceId>) -> Self {
            Self {
                active_devices: Mutex::new(Vec::new()),
                submitted_requests: Mutex::new(Vec::new()),
                stopped_devices,
                stop_count: Mutex::new(0),
            }
        }
    }

    impl DeviceOutputController for FakeOutputController {
        fn attach_output_device(&self, device_id: DeviceId) -> Result<(), crate::CoreError> {
            let mut active_devices = self.active_devices.lock().unwrap();
            if !active_devices.contains(&device_id) {
                active_devices.push(device_id);
            }
            Ok(())
        }

        fn detach_output_device(&self, device_id: &DeviceId) -> Result<(), crate::CoreError> {
            self.active_devices
                .lock()
                .unwrap()
                .retain(|active_device| active_device != device_id);
            Ok(())
        }

        fn active_output_devices(&self) -> Vec<DeviceId> {
            self.active_devices.lock().unwrap().clone()
        }

        fn submit_coyote_v3_window(
            &self,
            request: crate::CoyoteV3OutputRequest,
        ) -> Result<crate::SubmitOutputResult, crate::CoreError> {
            let device_id = request.device_id.clone();
            self.submitted_requests.lock().unwrap().push(request);
            Ok(crate::SubmitOutputResult::new(device_id, true))
        }

        fn stop_all_output(&self) -> Result<crate::StopOutputResult, crate::CoreError> {
            *self.stop_count.lock().unwrap() += 1;
            Ok(crate::StopOutputResult::new(self.stopped_devices.clone()))
        }
    }

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

    fn in_memory_storage() -> Arc<Mutex<Storage>> {
        Arc::new(Mutex::new(Storage::in_memory().unwrap()))
    }

    #[tokio::test]
    async fn stores_and_reads_plugin_private_json() {
        let storage = in_memory_storage();
        let api = PluginApi::new(storage);
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
        .await
        .unwrap();

        let result = api
            .handle_action(
                &manifest,
                PluginAction::new("storage.private.get", json!({ "key": "settings" })),
            )
            .await
            .unwrap();

        assert_eq!(
            result,
            json!({
                "key": "settings",
                "value": {"enabled": true}
            })
        );
    }

    #[tokio::test]
    async fn lists_and_deletes_plugin_private_keys() {
        let storage = in_memory_storage();
        let api = PluginApi::new(storage);
        let manifest = manifest_with(vec![Capability::StoragePrivate]);

        api.handle_action(
            &manifest,
            PluginAction::new("storage.private.put", json!({ "key": "b", "value": 1 })),
        )
        .await
        .unwrap();
        api.handle_action(
            &manifest,
            PluginAction::new("storage.private.put", json!({ "key": "a", "value": 2 })),
        )
        .await
        .unwrap();

        let keys = api
            .handle_action(
                &manifest,
                PluginAction::new("storage.private.keys", json!({})),
            )
            .await
            .unwrap();

        assert_eq!(keys, json!({ "keys": ["a", "b"] }));

        api.handle_action(
            &manifest,
            PluginAction::new("storage.private.delete", json!({ "key": "a" })),
        )
        .await
        .unwrap();

        let keys = api
            .handle_action(
                &manifest,
                PluginAction::new("storage.private.keys", json!({})),
            )
            .await
            .unwrap();

        assert_eq!(keys, json!({ "keys": ["b"] }));
    }

    #[tokio::test]
    async fn handles_plugin_output_actions_in_order() {
        let storage = in_memory_storage();
        let api = PluginApi::new(storage);
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
            .await
            .unwrap();

        assert_eq!(
            results,
            vec![
                json!({ "key": "theme", "stored": true }),
                json!({ "key": "theme", "value": "dark" }),
            ]
        );
    }

    #[tokio::test]
    async fn rejects_storage_without_capability() {
        let storage = in_memory_storage();
        let api = PluginApi::new(storage);
        let manifest = manifest_with(vec![Capability::DeviceRead]);

        let error = api
            .handle_action(
                &manifest,
                PluginAction::new("storage.private.keys", json!({})),
            )
            .await
            .unwrap_err();

        assert_eq!(
            error,
            PluginApiError::CapabilityDenied {
                plugin_id: "com.example.plugin".to_owned(),
                capability: Capability::StoragePrivate,
            }
        );
    }

    #[tokio::test]
    async fn reads_device_status_through_discovery_controller() {
        let storage = in_memory_storage();
        let discovery = Arc::new(FakeDiscoveryController::new(crate::DeviceScanResult::new(
            BleAdapterStatus::Ready,
            vec![DeviceStatus {
                id: DeviceId::new("coyote-v3"),
                model: crate::DeviceModel::CoyoteV3,
                battery_percent: Some(87),
                channel_a_strength: None,
                channel_b_strength: None,
                connected: true,
            }],
        )));
        let discovery_controller: Arc<dyn DeviceDiscoveryController> = discovery.clone();
        let api = PluginApi::with_discovery_controller(storage, discovery_controller);
        let manifest = manifest_with(vec![Capability::DeviceRead]);

        let result = api
            .handle_action(
                &manifest,
                PluginAction::new("device.status", json!({ "deviceId": "coyote-v3" })),
            )
            .await
            .unwrap();

        assert_eq!(
            result,
            json!({
                "deviceId": "coyote-v3",
                "connected": true,
                "adapterStatus": "ready",
                "batteryPercent": 87,
                "channelAStrength": null,
                "channelBStrength": null,
            })
        );
        assert_eq!(*discovery.scan_count.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn scans_devices_through_discovery_controller() {
        let storage = in_memory_storage();
        let discovery = Arc::new(FakeDiscoveryController::new(crate::DeviceScanResult::new(
            BleAdapterStatus::Ready,
            vec![DeviceStatus {
                id: DeviceId::new("coyote-v3"),
                model: crate::DeviceModel::CoyoteV3,
                battery_percent: Some(87),
                channel_a_strength: None,
                channel_b_strength: None,
                connected: true,
            }],
        )));
        let discovery_controller: Arc<dyn DeviceDiscoveryController> = discovery.clone();
        let api = PluginApi::with_discovery_controller(storage, discovery_controller);
        let manifest = manifest_with(vec![Capability::DeviceRead]);

        let result = api
            .handle_action(&manifest, PluginAction::new("device.scan", json!({})))
            .await
            .unwrap();

        assert_eq!(
            result,
            json!({
                "adapterStatus": "ready",
                "devices": [
                    {
                        "deviceId": "coyote-v3",
                        "model": "coyoteV3",
                        "connected": true,
                        "batteryPercent": 87,
                        "channelAStrength": null,
                        "channelBStrength": null,
                    }
                ],
            })
        );
        assert_eq!(*discovery.scan_count.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn rejects_device_scan_without_capability() {
        let storage = in_memory_storage();
        let discovery_controller: Arc<dyn DeviceDiscoveryController> =
            Arc::new(FakeDiscoveryController::new(crate::DeviceScanResult::new(
                BleAdapterStatus::Ready,
                Vec::new(),
            )));
        let api = PluginApi::with_discovery_controller(storage, discovery_controller);
        let manifest = manifest_with(vec![Capability::WaveControl]);

        let error = api
            .handle_action(&manifest, PluginAction::new("device.scan", json!({})))
            .await
            .unwrap_err();

        assert_eq!(
            error,
            PluginApiError::CapabilityDenied {
                plugin_id: "com.example.plugin".to_owned(),
                capability: Capability::DeviceRead,
            }
        );
    }

    #[tokio::test]
    async fn rejects_device_scan_without_discovery_controller() {
        let storage = in_memory_storage();
        let api = PluginApi::new(storage);
        let manifest = manifest_with(vec![Capability::DeviceRead]);

        let error = api
            .handle_action(&manifest, PluginAction::new("device.scan", json!({})))
            .await
            .unwrap_err();

        assert_eq!(
            error,
            PluginApiError::HostUnavailable("device.scan".to_owned())
        );
    }

    #[tokio::test]
    async fn reports_disconnected_device_status_when_scan_misses_device() {
        let storage = in_memory_storage();
        let discovery_controller: Arc<dyn DeviceDiscoveryController> =
            Arc::new(FakeDiscoveryController::new(crate::DeviceScanResult::new(
                BleAdapterStatus::PoweredOff,
                Vec::new(),
            )));
        let api = PluginApi::with_discovery_controller(storage, discovery_controller);
        let manifest = manifest_with(vec![Capability::DeviceRead]);

        let result = api
            .handle_action(
                &manifest,
                PluginAction::new("device.status", json!({ "deviceId": "coyote-v3" })),
            )
            .await
            .unwrap();

        assert_eq!(
            result,
            json!({
                "deviceId": "coyote-v3",
                "connected": false,
                "adapterStatus": "poweredOff",
            })
        );
    }

    #[tokio::test]
    async fn rejects_device_status_without_capability() {
        let storage = in_memory_storage();
        let discovery_controller: Arc<dyn DeviceDiscoveryController> =
            Arc::new(FakeDiscoveryController::new(crate::DeviceScanResult::new(
                BleAdapterStatus::Ready,
                Vec::new(),
            )));
        let api = PluginApi::with_discovery_controller(storage, discovery_controller);
        let manifest = manifest_with(vec![Capability::WaveControl]);

        let error = api
            .handle_action(
                &manifest,
                PluginAction::new("device.status", json!({ "deviceId": "coyote-v3" })),
            )
            .await
            .unwrap_err();

        assert_eq!(
            error,
            PluginApiError::CapabilityDenied {
                plugin_id: "com.example.plugin".to_owned(),
                capability: Capability::DeviceRead,
            }
        );
    }

    #[tokio::test]
    async fn rejects_device_status_without_discovery_controller() {
        let storage = in_memory_storage();
        let api = PluginApi::new(storage);
        let manifest = manifest_with(vec![Capability::DeviceRead]);

        let error = api
            .handle_action(
                &manifest,
                PluginAction::new("device.status", json!({ "deviceId": "coyote-v3" })),
            )
            .await
            .unwrap_err();

        assert_eq!(
            error,
            PluginApiError::HostUnavailable("device.status".to_owned())
        );
    }

    #[tokio::test]
    async fn activates_output_device_through_output_controller() {
        let storage = in_memory_storage();
        let controller = Arc::new(FakeOutputController::new(Vec::new()));
        let output_controller: Arc<dyn DeviceOutputController> = controller.clone();
        let api = PluginApi::with_output_controller(storage, output_controller);
        let manifest = manifest_with(vec![Capability::WaveControl]);

        let result = api
            .handle_action(
                &manifest,
                PluginAction::new("device.activateOutput", json!({ "deviceId": "coyote-v3" })),
            )
            .await
            .unwrap();

        assert_eq!(
            result,
            json!({
                "accepted": true,
                "command": "device.activateOutput",
                "deviceId": "coyote-v3",
                "activeOutputDevices": ["coyote-v3"],
            })
        );
        assert_eq!(
            controller.active_output_devices(),
            vec![DeviceId::new("coyote-v3")]
        );
    }

    #[tokio::test]
    async fn deactivates_output_device_through_output_controller() {
        let storage = in_memory_storage();
        let controller = Arc::new(FakeOutputController::new(Vec::new()));
        controller
            .attach_output_device(DeviceId::new("coyote-v3"))
            .unwrap();
        let output_controller: Arc<dyn DeviceOutputController> = controller.clone();
        let api = PluginApi::with_output_controller(storage, output_controller);
        let manifest = manifest_with(vec![Capability::WaveControl]);

        let result = api
            .handle_action(
                &manifest,
                PluginAction::new(
                    "device.deactivateOutput",
                    json!({ "deviceId": "coyote-v3" }),
                ),
            )
            .await
            .unwrap();

        assert_eq!(
            result,
            json!({
                "accepted": true,
                "command": "device.deactivateOutput",
                "deviceId": "coyote-v3",
                "activeOutputDevices": [],
            })
        );
        assert!(controller.active_output_devices().is_empty());
    }

    #[tokio::test]
    async fn rejects_output_activation_without_capability() {
        let storage = in_memory_storage();
        let controller = Arc::new(FakeOutputController::new(Vec::new()));
        let output_controller: Arc<dyn DeviceOutputController> = controller;
        let api = PluginApi::with_output_controller(storage, output_controller);
        let manifest = manifest_with(vec![Capability::DeviceRead]);

        let error = api
            .handle_action(
                &manifest,
                PluginAction::new("device.activateOutput", json!({ "deviceId": "coyote-v3" })),
            )
            .await
            .unwrap_err();

        assert_eq!(
            error,
            PluginApiError::CapabilityDenied {
                plugin_id: "com.example.plugin".to_owned(),
                capability: Capability::WaveControl,
            }
        );
    }

    #[tokio::test]
    async fn rejects_output_activation_without_output_controller() {
        let storage = in_memory_storage();
        let api = PluginApi::new(storage);
        let manifest = manifest_with(vec![Capability::WaveControl]);

        let error = api
            .handle_action(
                &manifest,
                PluginAction::new("device.activateOutput", json!({ "deviceId": "coyote-v3" })),
            )
            .await
            .unwrap_err();

        assert_eq!(
            error,
            PluginApiError::HostUnavailable("device.activateOutput".to_owned())
        );
    }

    #[tokio::test]
    async fn submits_coyote_v3_window_through_output_controller() {
        let storage = in_memory_storage();
        let controller = Arc::new(FakeOutputController::new(Vec::new()));
        let output_controller: Arc<dyn DeviceOutputController> = controller.clone();
        let api = PluginApi::with_output_controller(storage, output_controller);
        let manifest = manifest_with(vec![Capability::WaveControl]);

        let result = api
            .handle_action(
                &manifest,
                PluginAction::new("wave.submitWindow", submit_window_params()),
            )
            .await
            .unwrap();

        assert_eq!(
            result,
            json!({
                "accepted": true,
                "command": "wave.submitWindow",
                "deviceId": "coyote-v3",
            })
        );

        let requests = controller.submitted_requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].device_id, DeviceId::new("coyote-v3"));
        assert_eq!(requests[0].sequence, 4);
        assert_eq!(
            requests[0].strength_modes,
            StrengthModes::new(StrengthMode::Absolute, StrengthMode::Unchanged)
        );
        assert_eq!(requests[0].a_strength, 8);
        assert_eq!(requests[0].b_strength, 0);
        assert_eq!(requests[0].window, safe_window());
    }

    #[tokio::test]
    async fn rejects_wave_submit_window_without_capability() {
        let storage = in_memory_storage();
        let controller = Arc::new(FakeOutputController::new(Vec::new()));
        let output_controller: Arc<dyn DeviceOutputController> = controller;
        let api = PluginApi::with_output_controller(storage, output_controller);
        let manifest = manifest_with(vec![Capability::DeviceRead]);

        let error = api
            .handle_action(
                &manifest,
                PluginAction::new("wave.submitWindow", submit_window_params()),
            )
            .await
            .unwrap_err();

        assert_eq!(
            error,
            PluginApiError::CapabilityDenied {
                plugin_id: "com.example.plugin".to_owned(),
                capability: Capability::WaveControl,
            }
        );
    }

    #[tokio::test]
    async fn rejects_wave_submit_window_without_output_controller() {
        let storage = in_memory_storage();
        let api = PluginApi::new(storage);
        let manifest = manifest_with(vec![Capability::WaveControl]);

        let error = api
            .handle_action(
                &manifest,
                PluginAction::new("wave.submitWindow", submit_window_params()),
            )
            .await
            .unwrap_err();

        assert_eq!(
            error,
            PluginApiError::HostUnavailable("wave.submitWindow".to_owned())
        );
    }

    #[tokio::test]
    async fn rejects_wave_submit_window_with_invalid_wave_point() {
        let storage = in_memory_storage();
        let controller = Arc::new(FakeOutputController::new(Vec::new()));
        let output_controller: Arc<dyn DeviceOutputController> = controller;
        let api = PluginApi::with_output_controller(storage, output_controller);
        let manifest = manifest_with(vec![Capability::WaveControl]);

        let error = api
            .handle_action(
                &manifest,
                PluginAction::new(
                    "wave.submitWindow",
                    json!({
                        "deviceId": "coyote-v3",
                        "sequence": 4,
                        "channelA": [
                            {"periodMs": 1, "strength": 0},
                            {"periodMs": 10, "strength": 10},
                            {"periodMs": 10, "strength": 20},
                            {"periodMs": 10, "strength": 30}
                        ]
                    }),
                ),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, PluginApiError::InvalidParams(_)));
    }

    #[tokio::test]
    async fn stops_wave_through_output_controller() {
        let storage = in_memory_storage();
        let controller = Arc::new(FakeOutputController::new(vec![DeviceId::new("coyote-v3")]));
        let output_controller: Arc<dyn DeviceOutputController> = controller.clone();
        let api = PluginApi::with_output_controller(storage, output_controller);
        let manifest = manifest_with(vec![Capability::WaveControl]);

        let result = api
            .handle_action(
                &manifest,
                PluginAction::new("wave.stop", json!({ "deviceId": "coyote-v3" })),
            )
            .await
            .unwrap();

        assert_eq!(
            result,
            json!({
                "accepted": true,
                "command": "wave.stop",
                "deviceId": "coyote-v3",
                "stopped": true,
            })
        );
        assert_eq!(*controller.stop_count.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn rejects_wave_stop_without_capability() {
        let storage = in_memory_storage();
        let controller = Arc::new(FakeOutputController::new(vec![DeviceId::new("coyote-v3")]));
        let output_controller: Arc<dyn DeviceOutputController> = controller;
        let api = PluginApi::with_output_controller(storage, output_controller);
        let manifest = manifest_with(vec![Capability::DeviceRead]);

        let error = api
            .handle_action(
                &manifest,
                PluginAction::new("wave.stop", json!({ "deviceId": "coyote-v3" })),
            )
            .await
            .unwrap_err();

        assert_eq!(
            error,
            PluginApiError::CapabilityDenied {
                plugin_id: "com.example.plugin".to_owned(),
                capability: Capability::WaveControl,
            }
        );
    }

    #[tokio::test]
    async fn rejects_wave_stop_without_output_controller() {
        let storage = in_memory_storage();
        let api = PluginApi::new(storage);
        let manifest = manifest_with(vec![Capability::WaveControl]);

        let error = api
            .handle_action(
                &manifest,
                PluginAction::new("wave.stop", json!({ "deviceId": "coyote-v3" })),
            )
            .await
            .unwrap_err();

        assert_eq!(
            error,
            PluginApiError::HostUnavailable("wave.stop".to_owned())
        );
    }

    fn submit_window_params() -> Value {
        json!({
            "deviceId": "coyote-v3",
            "sequence": 4,
            "strengthModes": {
                "a": "absolute",
                "b": "unchanged"
            },
            "aStrength": 8,
            "bStrength": 0,
            "channelA": [
                {"periodMs": 10, "strength": 0},
                {"periodMs": 10, "strength": 10},
                {"periodMs": 10, "strength": 20},
                {"periodMs": 10, "strength": 30}
            ],
            "channelB": null
        })
    }

    fn safe_window() -> CoyoteV3Window {
        let channel = ChannelWindow::new([
            WavePoint::new(10, 0).unwrap(),
            WavePoint::new(10, 10).unwrap(),
            WavePoint::new(10, 20).unwrap(),
            WavePoint::new(10, 30).unwrap(),
        ]);

        CoyoteV3Window::new(ChannelOutput::Window(channel), ChannelOutput::Disabled)
    }
}
