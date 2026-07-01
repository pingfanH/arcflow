use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use arcflow_core::{
    execute_plugin_registry_external_request, execute_script_documents_external_request,
    is_plugin_registry_external_request, is_script_documents_external_request, ArcFlowCore,
    CoreScriptActionExecutor, CoyoteV3OutputController, DeviceDiscoveryController, DeviceModel,
    DeviceOutputController, DeviceScanResult, DeviceStatus, PluginBundle, PluginRegistryEntry,
    PluginRegistryPersistence, SafetyLimits, ScriptDocumentEntry, ScriptDocumentPersistence,
    ScriptWorkerQueue, StopOutputResult, StorageScriptRunner,
};
use arcflow_external_control::{
    ClientSession, GatewayPolicy, JsonRpcRequest, JsonRpcResponse, RpcError, WsGatewayHandle,
    WsGatewayService, WsRequestHandler, DEFAULT_LOCAL_BIND,
};
use arcflow_plugin_runtime::Capability;
use arcflow_script::ScriptCompiler;
use arcflow_storage::Storage;
use arcflow_tauri_platform::{TauriBleDiscoveryController, TauriBleOutputSink, TauriBleTransport};
use serde::Serialize;
use tauri::Manager;

#[derive(Default)]
struct ExternalControlState {
    handle: Mutex<Option<WsGatewayHandle>>,
}

struct CoreState {
    core: ArcFlowCore,
}

struct StorageState {
    storage: Arc<Mutex<Storage>>,
    database_path: PathBuf,
}

#[derive(Clone)]
struct RuntimeState {
    output_controller: Arc<CoyoteV3OutputController<TauriBleOutputSink>>,
    output_sink: TauriBleOutputSink,
}

const RUNTIME_STATUS_METHOD: &str = "runtime.status";

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
struct RuntimeStatus {
    active_output_devices: Vec<String>,
    ble_output_queued: u64,
    ble_output_written: u64,
    ble_output_failed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginRegistryResponse {
    plugins: Vec<PluginRegistryEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScriptsResponse {
    scripts: Vec<ScriptDocumentEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScriptRunResponse {
    script_id: String,
    queued: bool,
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
fn runtime_status(state: tauri::State<'_, RuntimeState>) -> RuntimeStatus {
    runtime_status_from_state(&state)
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
fn install_plugin_bundle(
    bundle_path: String,
    state: tauri::State<'_, StorageState>,
) -> Result<PluginRegistryResponse, String> {
    let bundle = PluginBundle::load(bundle_path).map_err(|error| error.to_string())?;
    let manifest_json =
        serde_json::to_string(bundle.manifest()).map_err(|error| error.to_string())?;
    let storage = state.storage.lock().expect("storage state mutex poisoned");
    let persistence = PluginRegistryPersistence::new(&storage);
    let plugins = persistence
        .install_manifest_json_with_bundle_root(
            &manifest_json,
            Some(bundle.root().display().to_string()),
        )
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
fn list_scripts(state: tauri::State<'_, StorageState>) -> Result<ScriptsResponse, String> {
    let storage = state.storage.lock().expect("storage state mutex poisoned");
    let scripts = ScriptDocumentPersistence::new(&storage)
        .list()
        .map_err(|error| error.to_string())?;

    Ok(ScriptsResponse { scripts })
}

#[tauri::command]
fn upsert_script(
    script_id: String,
    document_json: String,
    state: tauri::State<'_, StorageState>,
) -> Result<ScriptsResponse, String> {
    let storage = state.storage.lock().expect("storage state mutex poisoned");
    let scripts = ScriptDocumentPersistence::new(&storage)
        .upsert(&script_id, &document_json)
        .map_err(|error| error.to_string())?;

    Ok(ScriptsResponse { scripts })
}

#[tauri::command]
fn delete_script(
    script_id: String,
    state: tauri::State<'_, StorageState>,
) -> Result<ScriptsResponse, String> {
    let storage = state.storage.lock().expect("storage state mutex poisoned");
    let scripts = ScriptDocumentPersistence::new(&storage)
        .delete(&script_id)
        .map_err(|error| error.to_string())?;

    Ok(ScriptsResponse { scripts })
}

#[tauri::command]
fn run_script(
    script_id: String,
    state: tauri::State<'_, CoreState>,
) -> Result<ScriptRunResponse, String> {
    let result = state
        .core
        .run_script(&script_id)
        .map_err(|error| error.to_string())?;

    Ok(ScriptRunResponse {
        script_id: result.script_id().to_owned(),
        queued: result.queued(),
    })
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
    storage_state: tauri::State<'_, StorageState>,
    runtime_state: tauri::State<'_, RuntimeState>,
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
        .start(external_request_handler(
            core_state.core.clone(),
            Arc::clone(&storage_state.storage),
            runtime_state.inner().clone(),
        ))
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

fn external_request_handler(
    core: ArcFlowCore,
    storage: Arc<Mutex<Storage>>,
    runtime: RuntimeState,
) -> WsRequestHandler {
    Arc::new(move |session: ClientSession, request: JsonRpcRequest| {
        let core = core.clone();
        let storage = Arc::clone(&storage);
        let runtime = runtime.clone();

        Box::pin(async move {
            let id = request.id.clone();

            if is_plugin_registry_external_request(&request) {
                let result = {
                    let storage = storage.lock().expect("storage state mutex poisoned");
                    execute_plugin_registry_external_request(&session, &request, &storage)
                };

                return match result {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(error) => {
                        JsonRpcResponse::error(id, RpcError::new(-32000, error.to_string()))
                    }
                };
            }

            if is_script_documents_external_request(&request) {
                let result = {
                    let storage = storage.lock().expect("storage state mutex poisoned");
                    execute_script_documents_external_request(&session, &request, &storage)
                };

                return match result {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(error) => {
                        JsonRpcResponse::error(id, RpcError::new(-32000, error.to_string()))
                    }
                };
            }

            if request.method == RUNTIME_STATUS_METHOD {
                let response = execute_runtime_status_external_request(&session, &runtime);

                return match response {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(error) => JsonRpcResponse::error(id, error),
                };
            }

            match core.execute_external_request(&session, &request).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(error) => JsonRpcResponse::error(id, RpcError::new(-32000, error.to_string())),
            }
        })
    })
}

fn execute_runtime_status_external_request(
    session: &ClientSession,
    runtime: &RuntimeState,
) -> Result<serde_json::Value, RpcError> {
    if !session.has_capability(Capability::DeviceRead) {
        return Err(RpcError::new(
            -32000,
            format!(
                "session `{}` is missing required capability `{}` for `{}`",
                session.client_name(),
                Capability::DeviceRead.as_str(),
                RUNTIME_STATUS_METHOD
            ),
        ));
    }

    serde_json::to_value(runtime_status_from_state(runtime))
        .map_err(|error| RpcError::new(-32000, error.to_string()))
}

fn runtime_status_from_state(runtime: &RuntimeState) -> RuntimeStatus {
    let stats = runtime.output_sink.stats();

    RuntimeStatus {
        active_output_devices: runtime
            .output_controller
            .active_devices()
            .into_iter()
            .map(|device_id| device_id.as_str().to_owned())
            .collect(),
        ble_output_queued: stats.queued,
        ble_output_written: stats.written,
        ble_output_failed: stats.failed,
    }
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
            let storage = Arc::new(Mutex::new(Storage::open(&database_path)?));
            let safety_limits = SafetyLimits::conservative();
            let discovery_controller: Arc<dyn DeviceDiscoveryController> =
                Arc::new(TauriBleDiscoveryController::unsupported());
            let ble_transport = Arc::new(TauriBleTransport::unsupported());
            let output_sink = TauriBleOutputSink::spawn(ble_transport);
            let output_controller = Arc::new(CoyoteV3OutputController::new(
                safety_limits,
                output_sink.clone(),
            ));
            let core_output_controller: Arc<dyn DeviceOutputController> = output_controller.clone();
            let script_actions = CoreScriptActionExecutor::new(
                Arc::clone(&discovery_controller),
                Arc::clone(&core_output_controller),
            );
            let script_queue = ScriptWorkerQueue::spawn(Arc::new(script_actions));
            let script_runner = StorageScriptRunner::with_queue(
                Arc::clone(&storage),
                ScriptCompiler::default(),
                Arc::new(script_queue),
            );

            app.manage(StorageState {
                storage,
                database_path,
            });
            app.manage(RuntimeState {
                output_controller,
                output_sink,
            });
            app.manage(CoreState {
                core: ArcFlowCore::with_controllers(
                    safety_limits,
                    discovery_controller,
                    core_output_controller,
                    Arc::new(script_runner),
                ),
            });

            Ok(())
        })
        .manage(ExternalControlState::default())
        .invoke_handler(tauri::generate_handler![
            app_status,
            storage_status,
            runtime_status,
            plugin_registry,
            install_plugin_manifest,
            install_plugin_bundle,
            set_plugin_enabled,
            list_scripts,
            upsert_script,
            delete_script,
            run_script,
            scan_devices,
            stop_output,
            external_control_status,
            start_external_control,
            stop_external_control
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
