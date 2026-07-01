#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    arcflow_tauri_app::run::<tauri::Wry>(tauri::generate_context!())
}
