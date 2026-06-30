use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use arcflow_core::{
    ArcFlowCore, DeviceModel, DeviceScanResult, DeviceStatus, PluginRegistryEntry,
    PluginRegistryPersistence, SafetyLimits, StopOutputResult,
};
use arcflow_external_control::{
    ClientSession, GatewayPolicy, JsonRpcRequest, JsonRpcResponse, RpcError, WsGatewayHandle,
    WsGatewayService, WsRequestHandler, DEFAULT_LOCAL_BIND,
};
use arcflow_storage::Storage;
use serde::Serialize;
use tauri::Manager;

#[derive(Default)]
struct ExternalControlState {
    handle: Mutex<Option<WsGatewayHandle>>,
}

#[derive(Default)]
struct CoreState {
    core: ArcFlowCore,
}

struct StorageState {
    storage: Mutex<Storage>,
    database_path: PathBuf,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppStatus {
    protocol_crate: &'static str,
    plugin_runtimes: [&'static str; 2],
    external_control_bind: &'static str,
    max_channel_strength: u8,
    max_wave_strength: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExternalControlStatus {
    running: bool,
    bind_address: Option<String>,
    accepted_sessions: usize,
    active_sessions: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceScanResponse {
    adapter_status: String,
    devices: Vec<DeviceResponse>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceResponse {
    id: String,
    model: String,
    battery_percent: Option<u8>,
    connected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct StopOutputResponse {
    stopped_devices: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct StorageStatus {
    schema_version: i64,
    database_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginRegistryResponse {
    plugins: Vec<PluginRegistryEntry>,
}

#[tauri::command]
fn app_status() -> AppStatus {
    let limits = SafetyLimits::conservative();

    AppStatus {
        protocol_crate: "arcflow-protocol",
        plugin_runtimes: ["wasm", "javascript"],
        external_control_bind: DEFAULT_LOCAL_BIND,
        max_channel_strength: limits.max_channel_strength,
        max_wave_strength: limits.max_wave_strength,
    }
}

#[tauri::command]
fn storage_status(state: tauri::State<'_, StorageState>) -> Result<StorageStatus, String> {
    let storage = state.storage.lock().expect("storage state mutex poisoned");
    let schema_version = storage
        .schema_version()
        .map_err(|error| error.to_string())?;

    Ok(StorageStatus {
        schema_version,
        database_path: state.database_path.display().to_string(),
    })
}

#[tauri::command]
fn plugin_registry(
    state: tauri::State<'_, StorageState>,
) -> Result<PluginRegistryResponse, String> {
    let storage = state.storage.lock().expect("storage state mutex poisoned");
    let persistence = PluginRegistryPersistence::new(&storage);
    let plugins = persistence
        .list_entries()
        .map_err(|error| error.to_string())?;

    Ok(PluginRegistryResponse { plugins })
}

#[tauri::command]
fn install_plugin_manifest(
    manifest_json: String,
    state: tauri::State<'_, StorageState>,
) -> Result<PluginRegistryResponse, String> {
    let storage = state.storage.lock().expect("storage state mutex poisoned");
    let persistence = PluginRegistryPersistence::new(&storage);
    let plugins = persistence
        .install_manifest_json(&manifest_json)
        .map_err(|error| error.to_string())?;

    Ok(PluginRegistryResponse { plugins })
}

#[tauri::command]
fn set_plugin_enabled(
    plugin_id: String,
    enabled: bool,
    state: tauri::State<'_, StorageState>,
) -> Result<PluginRegistryResponse, String> {
    let storage = state.storage.lock().expect("storage state mutex poisoned");
    let persistence = PluginRegistryPersistence::new(&storage);
    let plugins = persistence
        .set_enabled(&plugin_id, enabled)
        .map_err(|error| error.to_string())?;

    Ok(PluginRegistryResponse { plugins })
}

#[tauri::command]
async fn scan_devices(state: tauri::State<'_, CoreState>) -> Result<DeviceScanResponse, String> {
    state
        .core
        .scan_devices()
        .await
        .map(device_scan_response)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn stop_output(state: tauri::State<'_, CoreState>) -> Result<StopOutputResponse, String> {
    state
        .core
        .stop_all_output()
        .map(stop_output_response)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn external_control_status(state: tauri::State<'_, ExternalControlState>) -> ExternalControlStatus {
    let guard = state
        .handle
        .lock()
        .expect("external-control state mutex poisoned");

    guard
        .as_ref()
        .map(external_control_status_from_handle)
        .unwrap_or_else(stopped_external_control_status)
}

#[tauri::command]
async fn start_external_control(
    state: tauri::State<'_, ExternalControlState>,
    core_state: tauri::State<'_, CoreState>,
) -> Result<ExternalControlStatus, String> {
    {
        let guard = state
            .handle
            .lock()
            .expect("external-control state mutex poisoned");

        if let Some(handle) = guard.as_ref() {
            return Ok(external_control_status_from_handle(handle));
        }
    }

    let service = WsGatewayService::new(DEFAULT_LOCAL_BIND, GatewayPolicy::local_default());
    let handle = service
        .start(external_request_handler(core_state.core.clone()))
        .await
        .map_err(|error| error.to_string())?;
    let status = external_control_status_from_handle(&handle);

    let existing_status = {
        let mut guard = state
            .handle
            .lock()
            .expect("external-control state mutex poisoned");

        if let Some(existing) = guard.as_ref() {
            Some(external_control_status_from_handle(existing))
        } else {
            *guard = Some(handle);
            None
        }
    };

    if let Some(existing_status) = existing_status {
        return Ok(existing_status);
    }

    Ok(status)
}

#[tauri::command]
async fn stop_external_control(
    state: tauri::State<'_, ExternalControlState>,
) -> Result<ExternalControlStatus, String> {
    let handle = {
        let mut guard = state
            .handle
            .lock()
            .expect("external-control state mutex poisoned");

        guard.take()
    };

    if let Some(handle) = handle {
        handle.stop().await;
    }

    Ok(stopped_external_control_status())
}

fn stopped_external_control_status() -> ExternalControlStatus {
    ExternalControlStatus {
        running: false,
        bind_address: None,
        accepted_sessions: 0,
        active_sessions: 0,
    }
}

fn external_control_status_from_handle(handle: &WsGatewayHandle) -> ExternalControlStatus {
    let status = handle.status();

    ExternalControlStatus {
        running: status.running,
        bind_address: Some(status.bind_address),
        accepted_sessions: status.accepted_sessions,
        active_sessions: status.active_sessions,
    }
}

fn external_request_handler(core: ArcFlowCore) -> WsRequestHandler {
    Arc::new(move |session: ClientSession, request: JsonRpcRequest| {
        let core = core.clone();

        Box::pin(async move {
            let id = request.id.clone();

            match core.execute_external_request(&session, &request).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(error) => JsonRpcResponse::error(id, RpcError::new(-32000, error.to_string())),
            }
        })
    })
}

fn device_scan_response(result: DeviceScanResult) -> DeviceScanResponse {
    DeviceScanResponse {
        adapter_status: result.adapter_status.as_str().to_owned(),
        devices: result.devices.into_iter().map(device_response).collect(),
    }
}

fn device_response(device: DeviceStatus) -> DeviceResponse {
    DeviceResponse {
        id: device.id.as_str().to_owned(),
        model: device_model_name(&device.model),
        battery_percent: device.battery_percent,
        connected: device.connected,
    }
}

fn device_model_name(model: &DeviceModel) -> String {
    match model {
        DeviceModel::CoyoteV2 => "coyoteV2".to_owned(),
        DeviceModel::CoyoteV3 => "coyoteV3".to_owned(),
        DeviceModel::Unknown(value) => value.clone(),
    }
}

fn stop_output_response(result: StopOutputResult) -> StopOutputResponse {
    StopOutputResponse {
        stopped_devices: result
            .stopped_devices
            .into_iter()
            .map(|device_id| device_id.as_str().to_owned())
            .collect(),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_data_dir)?;
            let database_path = app_data_dir.join("arcflow.sqlite3");
            let storage = Storage::open(&database_path)?;

            app.manage(StorageState {
                storage: Mutex::new(storage),
                database_path,
            });

            Ok(())
        })
        .manage(CoreState::default())
        .manage(ExternalControlState::default())
        .invoke_handler(tauri::generate_handler![
            app_status,
            storage_status,
            plugin_registry,
            install_plugin_manifest,
            set_plugin_enabled,
            scan_devices,
            stop_output,
            external_control_status,
            start_external_control,
            stop_external_control
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
