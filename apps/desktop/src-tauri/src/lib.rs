use std::sync::{Arc, Mutex};

use arcflow_core::{authorized_core_command_from_external_request, CoreCommand, SafetyLimits};
use arcflow_external_control::{
    ClientSession, GatewayPolicy, JsonRpcRequest, JsonRpcResponse, RpcError, WsGatewayHandle,
    WsGatewayService, WsRequestHandler, DEFAULT_LOCAL_BIND,
};
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Default)]
struct ExternalControlState {
    handle: Mutex<Option<WsGatewayHandle>>,
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
        .start(external_request_handler())
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

fn external_request_handler() -> WsRequestHandler {
    Arc::new(|session: &ClientSession, request: JsonRpcRequest| {
        let id = request.id.clone();

        match authorized_core_command_from_external_request(session, &request) {
            Ok(command) => JsonRpcResponse::ok(id, external_command_ack(command)),
            Err(error) => JsonRpcResponse::error(id, RpcError::new(-32000, error.to_string())),
        }
    })
}

fn external_command_ack(command: CoreCommand) -> Value {
    match command {
        CoreCommand::ReadDeviceStatus { device_id } => json!({
            "accepted": true,
            "command": "device.status",
            "deviceId": device_id.as_str(),
        }),
        CoreCommand::SubmitCoyoteV3Window { device_id, .. } => json!({
            "accepted": true,
            "command": "wave.submitWindow",
            "deviceId": device_id.as_str(),
        }),
        CoreCommand::StopOutput { device_id } => json!({
            "accepted": true,
            "command": "wave.stop",
            "deviceId": device_id.as_str(),
        }),
        CoreCommand::RunScript { script_id } => json!({
            "accepted": true,
            "command": "script.run",
            "scriptId": script_id,
        }),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(ExternalControlState::default())
        .invoke_handler(tauri::generate_handler![
            app_status,
            external_control_status,
            start_external_control,
            stop_external_control
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
