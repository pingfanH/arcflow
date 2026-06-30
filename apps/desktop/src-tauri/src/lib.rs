use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use arcflow_core::{
    execute_plugin_registry_external_request, is_plugin_registry_external_request, ArcFlowCore,
    DeviceModel, DeviceScanResult, DeviceStatus, PluginBundle, PluginRegistryEntry,
    PluginRegistryPersistence, SafetyLimits, StopOutputResult, StorageScriptRunner,
};
use arcflow_external_control::{
    ClientSession, GatewayPolicy, JsonRpcRequest, JsonRpcResponse, RpcError, WsGatewayHandle,
    WsGatewayService, WsRequestHandler, DEFAULT_LOCAL_BIND,
};
use arcflow_script::{ScriptCompiler, ScriptDocument};
use arcflow_storage::{Storage, StoredScriptRecord};
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScriptDocumentResponse {
    script_id: String,
    document_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScriptsResponse {
    scripts: Vec<ScriptDocumentResponse>,
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
    scripts_response(&storage)
}

#[tauri::command]
fn upsert_script(
    script_id: String,
    document_json: String,
    state: tauri::State<'_, StorageState>,
) -> Result<ScriptsResponse, String> {
    let record = validate_script_record(&script_id, &document_json)?;
    let storage = state.storage.lock().expect("storage state mutex poisoned");

    storage
        .scripts()
        .upsert(&record)
        .map_err(|error| error.to_string())?;

    scripts_response(&storage)
}

#[tauri::command]
fn delete_script(
    script_id: String,
    state: tauri::State<'_, StorageState>,
) -> Result<ScriptsResponse, String> {
    let storage = state.storage.lock().expect("storage state mutex poisoned");

    storage
        .scripts()
        .delete(&script_id)
        .map_err(|error| error.to_string())?;

    scripts_response(&storage)
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

fn external_request_handler(core: ArcFlowCore, storage: Arc<Mutex<Storage>>) -> WsRequestHandler {
    Arc::new(move |session: ClientSession, request: JsonRpcRequest| {
        let core = core.clone();
        let storage = Arc::clone(&storage);

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

fn scripts_response(storage: &Storage) -> Result<ScriptsResponse, String> {
    let scripts = storage
        .scripts()
        .list()
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|record| ScriptDocumentResponse {
            script_id: record.script_id,
            document_json: record.document_json,
        })
        .collect();

    Ok(ScriptsResponse { scripts })
}

fn validate_script_record(
    script_id: &str,
    document_json: &str,
) -> Result<StoredScriptRecord, String> {
    let document = ScriptDocument::from_json(document_json).map_err(|error| error.to_string())?;

    if document.id != script_id {
        return Err(format!(
            "script id `{script_id}` does not match document id `{}`",
            document.id
        ));
    }

    ScriptCompiler::default()
        .compile(document)
        .map_err(|error| error.to_string())?;

    Ok(StoredScriptRecord::new(script_id, document_json))
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
            let script_runner =
                StorageScriptRunner::new(Arc::clone(&storage), ScriptCompiler::default());

            app.manage(StorageState {
                storage,
                database_path,
            });
            app.manage(CoreState {
                core: ArcFlowCore::with_script_runner(
                    SafetyLimits::conservative(),
                    Arc::new(script_runner),
                ),
            });

            Ok(())
        })
        .manage(ExternalControlState::default())
        .invoke_handler(tauri::generate_handler![
            app_status,
            storage_status,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_script_before_storage() {
        let record = validate_script_record(
            "script.demo",
            r#"{"id":"script.demo","version":1,"steps":[{"type":"wait","durationMs":1}]}"#,
        )
        .unwrap();

        assert_eq!(record.script_id, "script.demo");
    }

    #[test]
    fn rejects_mismatched_script_id() {
        let error = validate_script_record(
            "script.outer",
            r#"{"id":"script.inner","version":1,"steps":[{"type":"wait","durationMs":1}]}"#,
        )
        .unwrap_err();

        assert!(error.contains("does not match"));
    }

    #[test]
    fn rejects_invalid_script_documents() {
        let error = validate_script_record(
            "script.empty",
            r#"{"id":"script.empty","version":1,"steps":[]}"#,
        )
        .unwrap_err();

        assert!(error.contains("at least one step"));
    }

    #[test]
    fn lists_scripts_in_storage_order() {
        let storage = Storage::in_memory().unwrap();
        storage
            .scripts()
            .upsert(&StoredScriptRecord::new(
                "script.b",
                r#"{"id":"script.b","version":1,"steps":[{"type":"wait","durationMs":1}]}"#,
            ))
            .unwrap();
        storage
            .scripts()
            .upsert(&StoredScriptRecord::new(
                "script.a",
                r#"{"id":"script.a","version":1,"steps":[{"type":"wait","durationMs":1}]}"#,
            ))
            .unwrap();

        let response = scripts_response(&storage).unwrap();

        assert_eq!(response.scripts[0].script_id, "script.a");
        assert_eq!(response.scripts[1].script_id, "script.b");
    }
}
