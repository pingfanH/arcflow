use arcflow_core::SafetyLimits;
use arcflow_external_control::DEFAULT_LOCAL_BIND;
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppStatus {
    protocol_crate: &'static str,
    plugin_runtimes: [&'static str; 2],
    external_control_bind: &'static str,
    max_channel_strength: u8,
    max_wave_strength: u8,
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![app_status])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
