use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use arcflow_core::{
    execute_plugin_registry_external_request, execute_script_documents_external_request,
    is_plugin_registry_external_request, is_plugin_registry_mutation_external_request,
    is_script_documents_external_request, ArcFlowCore, CoreScriptActionExecutor,
    CoyoteV3OutputController, CoyoteV3OutputRequest, CoyoteV3PreviewSession,
    CoyoteV3SequenceAllocator, DeviceDiscoveryController, DeviceId, DeviceModel,
    DeviceOutputController, DeviceScanResult, DeviceStatus, PluginApi, PluginBundle,
    PluginRegistryEntry, PluginRegistryPersistence, PluginRuntimeSyncReport,
    RecordingPluginRuntimeController, RecordingPluginRuntimeHookInvoker, SafetyLimits,
    ScriptDocumentEntry, ScriptDocumentPersistence, ScriptWorkerEvent, ScriptWorkerQueue,
    StopOutputResult, StorageScriptRunner, SubmitOutputResult, COYOTE_V3_PREVIEW_INTERVAL_MS,
};
use arcflow_external_control::{
    ClientSession, GatewayPolicy, JsonRpcRequest, JsonRpcResponse, RpcError, ServerEvent,
    WsGatewayHandle, WsGatewayService, WsRequestHandler, DEFAULT_LOCAL_BIND,
};
use arcflow_plugin_runtime::{Capability, RecordedPlugin, RuntimeKind};
use arcflow_script::ScriptCompiler;
use arcflow_storage::Storage;
#[cfg(feature = "native-ble")]
use arcflow_tauri_platform::NativeBlePlatformProvider;
use arcflow_tauri_platform::{
    TauriBleDiscoveryController, TauriBleDiscoveryProvider, TauriBleOutputEvent,
    TauriBleOutputSink, TauriBlePeripheralDiagnostic, TauriBlePlatformProvider,
    TauriBleScanDiagnostics, TauriBleTransportProvider,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::Manager;
use tokio::{
    sync::{broadcast, mpsc, oneshot, Mutex as AsyncMutex},
    time::{sleep, Duration},
};

#[derive(Default)]
struct ExternalControlState {
    handle: Mutex<Option<RunningExternalControl>>,
}

struct RunningExternalControl {
    handle: WsGatewayHandle,
    control_mode: bool,
    allowed_capabilities: Vec<Capability>,
}

#[derive(Clone, Default)]
struct PreviewPlaybackState {
    inner: Arc<Mutex<PreviewPlaybackInner>>,
}

#[derive(Default)]
struct PreviewPlaybackInner {
    next_id: u64,
    running: Option<RunningPreviewPlayback>,
}

struct RunningPreviewPlayback {
    id: u64,
    device_id: DeviceId,
    channel_a_strength: u8,
    channel_b_strength: u8,
    stop_sender: Option<oneshot::Sender<()>>,
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
    discovery_controller: Arc<dyn DeviceDiscoveryController>,
    ble_platform_provider: Arc<dyn TauriBlePlatformProvider>,
    output_controller: Arc<CoyoteV3OutputController<TauriBleOutputSink>>,
    output_host: Arc<dyn DeviceOutputController>,
    output_sink: TauriBleOutputSink,
    events: RuntimeEventLog,
    preview_sequences: Arc<Mutex<CoyoteV3SequenceAllocator>>,
}

#[derive(Clone)]
struct PluginRuntimeState {
    controller: Arc<AsyncMutex<RecordingPluginRuntimeController>>,
    events: RuntimeEventLog,
}

const RUNTIME_STATUS_METHOD: &str = "runtime.status";
const RUNTIME_EVENTS_METHOD: &str = "runtime.events";
const RUNTIME_PLUGINS_METHOD: &str = "runtime.plugins";
const DEVICE_SCAN_METHOD: &str = "device.scan";
const DEVICE_CONNECT_METHOD: &str = "device.connect";
const DEVICE_DISCONNECT_METHOD: &str = "device.disconnect";
const DEVICE_REFRESH_BATTERY_METHOD: &str = "device.refreshBattery";
const PLUGIN_INVOKE_HOOK_METHOD: &str = "plugin.invokeHook";
const WAVE_PREVIEW_STATUS_METHOD: &str = "wave.previewStatus";
const WAVE_PREVIEW_START_METHOD: &str = "wave.startPreview";
const WAVE_PREVIEW_UPDATE_METHOD: &str = "wave.updatePreview";
const WAVE_PREVIEW_STOP_METHOD: &str = "wave.stopPreview";
const RUNTIME_EVENT_LIMIT: usize = 128;

#[derive(Clone)]
struct RuntimeEventLog {
    inner: Arc<Mutex<RuntimeEventLogInner>>,
    publisher: broadcast::Sender<ServerEvent>,
}

#[derive(Default)]
struct RuntimeEventLogInner {
    next_sequence: u64,
    events: VecDeque<RuntimeEventRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeEventRecord {
    sequence: u64,
    kind: String,
    message: String,
}

impl RuntimeEventLog {
    fn push(&self, kind: impl Into<String>, message: impl Into<String>) {
        let mut inner = self.inner.lock().expect("runtime event log mutex poisoned");
        let sequence = inner.next_sequence;
        inner.next_sequence += 1;
        let record = RuntimeEventRecord {
            sequence,
            kind: kind.into(),
            message: message.into(),
        };
        inner.events.push_back(record.clone());

        while inner.events.len() > RUNTIME_EVENT_LIMIT {
            inner.events.pop_front();
        }

        let _ = self
            .publisher
            .send(ServerEvent::new(runtime_event_payload(&record)));
    }

    fn list(&self) -> Vec<RuntimeEventRecord> {
        self.inner
            .lock()
            .expect("runtime event log mutex poisoned")
            .events
            .iter()
            .cloned()
            .collect()
    }

    fn server_event_sender(&self) -> broadcast::Sender<ServerEvent> {
        self.publisher.clone()
    }
}

impl Default for RuntimeEventLog {
    fn default() -> Self {
        let (publisher, _) = broadcast::channel(RUNTIME_EVENT_LIMIT);

        Self {
            inner: Arc::new(Mutex::new(RuntimeEventLogInner::default())),
            publisher,
        }
    }
}

fn runtime_event_payload(record: &RuntimeEventRecord) -> serde_json::Value {
    serde_json::to_value(record).unwrap_or_else(|error| {
        serde_json::json!({
            "kind": "runtime.event.error",
            "message": error.to_string(),
        })
    })
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
enum FrontendPlatform {
    #[serde(rename = "desktop")]
    Desktop,
    #[serde(rename = "mobile")]
    Mobile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct FrontendPlatformResponse {
    platform: FrontendPlatform,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExternalControlStatus {
    running: bool,
    bind_address: Option<String>,
    accepted_sessions: usize,
    active_sessions: usize,
    control_mode: bool,
    allowed_capabilities: Vec<Capability>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceScanResponse {
    adapter_status: String,
    devices: Vec<DeviceResponse>,
    diagnostics: Option<DeviceScanDiagnosticsResponse>,
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
struct DeviceScanDiagnosticsResponse {
    scan_attempts: usize,
    discovered_peripherals: usize,
    inspected_peripherals: usize,
    matched_advertisements: usize,
    skipped_missing_properties: usize,
    skipped_unknown_peripherals: usize,
    matched_samples: Vec<DeviceScanDiagnosticSampleResponse>,
    skipped_unknown_samples: Vec<DeviceScanDiagnosticSampleResponse>,
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceScanDiagnosticSampleResponse {
    local_name: Option<String>,
    service_uuids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct StopOutputResponse {
    stopped_devices: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct OutputDeviceActivationResponse {
    active_output_devices: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SubmitWaveWindowResponse {
    device_id: String,
    sequence: u8,
    accepted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewPlaybackStatus {
    running: bool,
    device_id: Option<String>,
    channel_a_strength: u8,
    channel_b_strength: u8,
    interval_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartPreviewPlaybackParams {
    device_id: String,
    channel_a_strength: u8,
    channel_b_strength: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdatePreviewPlaybackParams {
    channel_a_strength: u8,
    channel_b_strength: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceParams {
    device_id: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginHookInvocationParams {
    plugin_id: String,
    hook: String,
    #[serde(default)]
    payload: Value,
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
    loaded_plugin_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeEventsResponse {
    events: Vec<RuntimeEventRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimePluginsResponse {
    plugins: Vec<RuntimePluginSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimePluginSnapshot {
    plugin_id: String,
    runtime: RuntimeKind,
    entry: String,
    bundle_root: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginHookInvocationResponse {
    plugin_id: String,
    hook: String,
    action_count: usize,
    results: Vec<Value>,
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

fn current_frontend_platform() -> FrontendPlatform {
    if cfg!(any(
        feature = "mobile-shell",
        target_os = "android",
        target_os = "ios"
    )) {
        FrontendPlatform::Mobile
    } else {
        FrontendPlatform::Desktop
    }
}

#[tauri::command]
fn frontend_platform() -> FrontendPlatformResponse {
    FrontendPlatformResponse {
        platform: current_frontend_platform(),
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
async fn runtime_status(
    state: tauri::State<'_, RuntimeState>,
    plugin_runtime: tauri::State<'_, PluginRuntimeState>,
) -> Result<RuntimeStatus, String> {
    let loaded_plugin_count = loaded_plugin_count(plugin_runtime.inner()).await;

    Ok(runtime_status_from_state(&state, loaded_plugin_count))
}

#[tauri::command]
fn runtime_events(state: tauri::State<'_, RuntimeState>) -> RuntimeEventsResponse {
    runtime_events_from_state(&state)
}

#[tauri::command]
async fn runtime_plugins(
    plugin_runtime: tauri::State<'_, PluginRuntimeState>,
) -> Result<RuntimePluginsResponse, String> {
    Ok(runtime_plugins_from_state(plugin_runtime.inner()).await)
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
async fn install_plugin_manifest(
    manifest_json: String,
    state: tauri::State<'_, StorageState>,
    plugin_runtime: tauri::State<'_, PluginRuntimeState>,
) -> Result<PluginRegistryResponse, String> {
    let plugins = {
        let storage = state.storage.lock().expect("storage state mutex poisoned");
        let persistence = PluginRegistryPersistence::new(&storage);
        persistence
            .install_manifest_json(&manifest_json)
            .map_err(|error| error.to_string())?
    };

    sync_plugin_runtime_from_storage(&state.storage, plugin_runtime.inner()).await?;

    Ok(PluginRegistryResponse { plugins })
}

#[tauri::command]
async fn install_plugin_bundle(
    bundle_path: String,
    state: tauri::State<'_, StorageState>,
    plugin_runtime: tauri::State<'_, PluginRuntimeState>,
) -> Result<PluginRegistryResponse, String> {
    let bundle = PluginBundle::load(bundle_path).map_err(|error| error.to_string())?;
    let manifest_json =
        serde_json::to_string(bundle.manifest()).map_err(|error| error.to_string())?;
    let plugins = {
        let storage = state.storage.lock().expect("storage state mutex poisoned");
        let persistence = PluginRegistryPersistence::new(&storage);
        persistence
            .install_manifest_json_with_bundle_root(
                &manifest_json,
                Some(bundle.root().display().to_string()),
            )
            .map_err(|error| error.to_string())?
    };

    sync_plugin_runtime_from_storage(&state.storage, plugin_runtime.inner()).await?;

    Ok(PluginRegistryResponse { plugins })
}

#[tauri::command]
async fn set_plugin_enabled(
    plugin_id: String,
    enabled: bool,
    state: tauri::State<'_, StorageState>,
    plugin_runtime: tauri::State<'_, PluginRuntimeState>,
) -> Result<PluginRegistryResponse, String> {
    let plugins = {
        let storage = state.storage.lock().expect("storage state mutex poisoned");
        let persistence = PluginRegistryPersistence::new(&storage);
        persistence
            .set_enabled(&plugin_id, enabled)
            .map_err(|error| error.to_string())?
    };

    sync_plugin_runtime_from_storage(&state.storage, plugin_runtime.inner()).await?;

    Ok(PluginRegistryResponse { plugins })
}

#[tauri::command]
async fn delete_plugin(
    plugin_id: String,
    state: tauri::State<'_, StorageState>,
    plugin_runtime: tauri::State<'_, PluginRuntimeState>,
) -> Result<PluginRegistryResponse, String> {
    let plugins = {
        let storage = state.storage.lock().expect("storage state mutex poisoned");
        let persistence = PluginRegistryPersistence::new(&storage);
        persistence
            .delete(&plugin_id)
            .map_err(|error| error.to_string())?
    };

    sync_plugin_runtime_from_storage(&state.storage, plugin_runtime.inner()).await?;

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
async fn scan_devices(
    state: tauri::State<'_, CoreState>,
    runtime_state: tauri::State<'_, RuntimeState>,
) -> Result<DeviceScanResponse, String> {
    scan_devices_and_sync(&state.core, &runtime_state).await
}

#[tauri::command]
async fn connect_device(
    device_id: String,
    state: tauri::State<'_, CoreState>,
    runtime_state: tauri::State<'_, RuntimeState>,
) -> Result<DeviceScanResponse, String> {
    connect_device_and_scan(&state.core, &runtime_state, DeviceId::new(device_id)).await
}

#[tauri::command]
async fn disconnect_device(
    device_id: String,
    state: tauri::State<'_, CoreState>,
    runtime_state: tauri::State<'_, RuntimeState>,
    preview_state: tauri::State<'_, PreviewPlaybackState>,
) -> Result<DeviceScanResponse, String> {
    disconnect_device_and_scan(
        &state.core,
        &runtime_state,
        Some(&preview_state),
        DeviceId::new(device_id),
    )
    .await
}

#[tauri::command]
async fn refresh_device_battery(
    device_id: String,
    state: tauri::State<'_, CoreState>,
    runtime_state: tauri::State<'_, RuntimeState>,
) -> Result<DeviceScanResponse, String> {
    refresh_device_battery_and_scan(&state.core, &runtime_state, DeviceId::new(device_id)).await
}

#[tauri::command]
fn preview_playback_status(state: tauri::State<'_, PreviewPlaybackState>) -> PreviewPlaybackStatus {
    preview_playback_status_from_state(&state)
}

#[tauri::command]
fn start_preview_playback(
    device_id: String,
    channel_a_strength: u8,
    channel_b_strength: u8,
    state: tauri::State<'_, CoreState>,
    runtime_state: tauri::State<'_, RuntimeState>,
    preview_state: tauri::State<'_, PreviewPlaybackState>,
) -> Result<PreviewPlaybackStatus, String> {
    start_preview_playback_session(
        &state.core,
        &runtime_state,
        &preview_state,
        StartPreviewPlaybackParams {
            device_id,
            channel_a_strength,
            channel_b_strength,
        },
    )
}

#[tauri::command]
fn stop_preview_playback(
    state: tauri::State<'_, CoreState>,
    runtime_state: tauri::State<'_, RuntimeState>,
    preview_state: tauri::State<'_, PreviewPlaybackState>,
) -> Result<PreviewPlaybackStatus, String> {
    stop_preview_playback_session(&state.core, &runtime_state, &preview_state)
}

#[tauri::command]
fn update_preview_playback(
    channel_a_strength: u8,
    channel_b_strength: u8,
    runtime_state: tauri::State<'_, RuntimeState>,
    preview_state: tauri::State<'_, PreviewPlaybackState>,
) -> Result<PreviewPlaybackStatus, String> {
    update_preview_playback_session(
        &runtime_state,
        &preview_state,
        UpdatePreviewPlaybackParams {
            channel_a_strength,
            channel_b_strength,
        },
    )
}

fn start_preview_playback_session(
    core: &ArcFlowCore,
    runtime_state: &RuntimeState,
    preview_state: &PreviewPlaybackState,
    params: StartPreviewPlaybackParams,
) -> Result<PreviewPlaybackStatus, String> {
    let session = CoyoteV3PreviewSession::new(
        DeviceId::new(params.device_id),
        params.channel_a_strength,
        params.channel_b_strength,
    )
    .map_err(|error| error.to_string())?;

    if !core.active_output_devices().contains(session.device_id()) {
        return Err(format!(
            "device `{}` is not connected",
            session.device_id().as_str()
        ));
    }

    let (id, stop_receiver) = {
        let mut inner = preview_state
            .inner
            .lock()
            .expect("preview playback state mutex poisoned");

        if let Some(running) = inner.running.as_ref() {
            return Ok(preview_playback_status_from_running(running));
        }

        let id = inner.next_id;
        inner.next_id = inner.next_id.wrapping_add(1);
        let (stop_sender, stop_receiver) = oneshot::channel();
        inner.running = Some(RunningPreviewPlayback {
            id,
            device_id: session.device_id().clone(),
            channel_a_strength: session.channel_a_strength(),
            channel_b_strength: session.channel_b_strength(),
            stop_sender: Some(stop_sender),
        });

        (id, stop_receiver)
    };

    runtime_state.events.push(
        "wave.preview.started",
        format!(
            "preview playback started for `{}`",
            session.device_id().as_str()
        ),
    );
    spawn_preview_playback_loop(
        id,
        core.clone(),
        runtime_state.clone(),
        preview_state.inner.clone(),
        stop_receiver,
    );

    Ok(preview_playback_status_from_state(preview_state))
}

fn update_preview_playback_session(
    runtime_state: &RuntimeState,
    preview_state: &PreviewPlaybackState,
    params: UpdatePreviewPlaybackParams,
) -> Result<PreviewPlaybackStatus, String> {
    let status = {
        let mut inner = preview_state
            .inner
            .lock()
            .expect("preview playback state mutex poisoned");
        let Some(running) = inner.running.as_mut() else {
            return Err("preview playback is not running".to_owned());
        };
        let session = CoyoteV3PreviewSession::new(
            running.device_id.clone(),
            params.channel_a_strength,
            params.channel_b_strength,
        )
        .map_err(|error| error.to_string())?;

        running.channel_a_strength = session.channel_a_strength();
        running.channel_b_strength = session.channel_b_strength();

        preview_playback_status_from_running(running)
    };

    runtime_state.events.push(
        "wave.preview.updated",
        format!(
            "preview playback updated for `{}` with A={} B={}",
            status.device_id.as_deref().unwrap_or("unknown"),
            status.channel_a_strength,
            status.channel_b_strength
        ),
    );

    Ok(status)
}

fn stop_preview_playback_session(
    core: &ArcFlowCore,
    runtime_state: &RuntimeState,
    preview_state: &PreviewPlaybackState,
) -> Result<PreviewPlaybackStatus, String> {
    cancel_preview_playback(preview_state, &runtime_state.events);
    core.stop_all_output().map_err(|error| error.to_string())?;

    Ok(preview_playback_status_from_state(preview_state))
}

#[tauri::command]
fn stop_output(
    state: tauri::State<'_, CoreState>,
    runtime_state: tauri::State<'_, RuntimeState>,
    preview_state: tauri::State<'_, PreviewPlaybackState>,
) -> Result<StopOutputResponse, String> {
    cancel_preview_playback(&preview_state, &runtime_state.events);

    state
        .core
        .stop_all_output()
        .map(stop_output_response)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn activate_output_device(
    device_id: String,
    state: tauri::State<'_, CoreState>,
    runtime_state: tauri::State<'_, RuntimeState>,
) -> Result<OutputDeviceActivationResponse, String> {
    let device_id = DeviceId::new(device_id);
    let active_devices = state
        .core
        .attach_output_device(device_id.clone())
        .map_err(|error| error.to_string())?;

    runtime_state.events.push(
        "device.output.activated",
        format!("device `{}` activated for output", device_id.as_str()),
    );

    Ok(output_device_activation_response(active_devices))
}

#[tauri::command]
fn deactivate_output_device(
    device_id: String,
    state: tauri::State<'_, CoreState>,
    runtime_state: tauri::State<'_, RuntimeState>,
) -> Result<OutputDeviceActivationResponse, String> {
    let device_id = DeviceId::new(device_id);
    let active_devices = state
        .core
        .detach_output_device(&device_id)
        .map_err(|error| error.to_string())?;

    runtime_state.events.push(
        "device.output.deactivated",
        format!("device `{}` deactivated for output", device_id.as_str()),
    );

    Ok(output_device_activation_response(active_devices))
}

#[tauri::command]
fn submit_preview_window(
    device_id: String,
    channel_a_strength: u8,
    channel_b_strength: u8,
    state: tauri::State<'_, CoreState>,
    runtime_state: tauri::State<'_, RuntimeState>,
) -> Result<SubmitWaveWindowResponse, String> {
    submit_preview_window_request(
        &state.core,
        &runtime_state,
        device_id,
        channel_a_strength,
        channel_b_strength,
    )
}

fn submit_preview_window_request(
    core: &ArcFlowCore,
    runtime_state: &RuntimeState,
    device_id: String,
    channel_a_strength: u8,
    channel_b_strength: u8,
) -> Result<SubmitWaveWindowResponse, String> {
    let device_id = DeviceId::new(device_id);
    let sequence = runtime_state
        .preview_sequences
        .lock()
        .expect("preview sequence allocator mutex poisoned")
        .next(&device_id);
    let request =
        CoyoteV3OutputRequest::preview(device_id, sequence, channel_a_strength, channel_b_strength)
            .map_err(|error| error.to_string())?;

    let result = core
        .submit_coyote_v3_window(request)
        .map_err(|error| error.to_string())?;

    runtime_state.events.push(
        "wave.window.submitted",
        format!(
            "submitted one output window for `{}` sequence {} with A={} B={}",
            result.device_id.as_str(),
            sequence,
            channel_a_strength,
            channel_b_strength
        ),
    );

    Ok(submit_wave_window_response(result, sequence))
}

#[tauri::command]
fn external_control_status(state: tauri::State<'_, ExternalControlState>) -> ExternalControlStatus {
    let guard = state
        .handle
        .lock()
        .expect("external-control state mutex poisoned");

    guard
        .as_ref()
        .map(external_control_status_from_running)
        .unwrap_or_else(stopped_external_control_status)
}

#[tauri::command]
async fn start_external_control(
    control_mode: bool,
    state: tauri::State<'_, ExternalControlState>,
    core_state: tauri::State<'_, CoreState>,
    storage_state: tauri::State<'_, StorageState>,
    runtime_state: tauri::State<'_, RuntimeState>,
    plugin_runtime_state: tauri::State<'_, PluginRuntimeState>,
    preview_state: tauri::State<'_, PreviewPlaybackState>,
) -> Result<ExternalControlStatus, String> {
    {
        let guard = state
            .handle
            .lock()
            .expect("external-control state mutex poisoned");

        if let Some(handle) = guard.as_ref() {
            return Ok(external_control_status_from_running(handle));
        }
    }

    let allowed_capabilities = external_control_allowed_capabilities(control_mode);
    let service = WsGatewayService::new(
        DEFAULT_LOCAL_BIND,
        GatewayPolicy::new(allowed_capabilities.clone()),
    );
    let runtime = runtime_state.inner().clone();
    let event_sender = runtime.events.server_event_sender();
    let handle = service
        .start_with_events(
            external_request_handler(
                core_state.core.clone(),
                Arc::clone(&storage_state.storage),
                runtime,
                plugin_runtime_state.inner().clone(),
                preview_state.inner().clone(),
            ),
            event_sender,
        )
        .await
        .map_err(|error| error.to_string())?;
    let running = RunningExternalControl {
        handle,
        control_mode,
        allowed_capabilities,
    };
    let status = external_control_status_from_running(&running);

    let existing_status = {
        let mut guard = state
            .handle
            .lock()
            .expect("external-control state mutex poisoned");

        if let Some(existing) = guard.as_ref() {
            Some(external_control_status_from_running(existing))
        } else {
            *guard = Some(running);
            None
        }
    };

    if let Some(existing_status) = existing_status {
        return Ok(existing_status);
    }

    log_external_control_started(&runtime_state.events, &status);
    Ok(status)
}

#[tauri::command]
async fn stop_external_control(
    state: tauri::State<'_, ExternalControlState>,
    runtime_state: tauri::State<'_, RuntimeState>,
) -> Result<ExternalControlStatus, String> {
    let handle = {
        let mut guard = state
            .handle
            .lock()
            .expect("external-control state mutex poisoned");

        guard.take()
    };

    if let Some(running) = handle {
        let status = external_control_status_from_running(&running);
        running.handle.stop().await;
        log_external_control_stopped(&runtime_state.events, &status);
    }

    Ok(stopped_external_control_status())
}

fn external_control_allowed_capabilities(control_mode: bool) -> Vec<Capability> {
    if control_mode {
        vec![
            Capability::ExternalWebSocket,
            Capability::DeviceRead,
            Capability::EventsSubscribe,
            Capability::WaveControl,
            Capability::ScriptRun,
            Capability::ScriptManage,
            Capability::PluginManage,
        ]
    } else {
        GatewayPolicy::local_default()
            .allowed_capabilities()
            .to_vec()
    }
}

fn stopped_external_control_status() -> ExternalControlStatus {
    ExternalControlStatus {
        running: false,
        bind_address: None,
        accepted_sessions: 0,
        active_sessions: 0,
        control_mode: false,
        allowed_capabilities: Vec::new(),
    }
}

fn external_control_status_from_running(running: &RunningExternalControl) -> ExternalControlStatus {
    let status = running.handle.status();

    ExternalControlStatus {
        running: status.running,
        bind_address: Some(status.bind_address),
        accepted_sessions: status.accepted_sessions,
        active_sessions: status.active_sessions,
        control_mode: running.control_mode,
        allowed_capabilities: running.allowed_capabilities.clone(),
    }
}

fn log_external_control_started(events: &RuntimeEventLog, status: &ExternalControlStatus) {
    let mode = if status.control_mode {
        "control"
    } else {
        "read-only"
    };
    let bind = status.bind_address.as_deref().unwrap_or(DEFAULT_LOCAL_BIND);
    events.push(
        "plugin.bridge.started",
        format!("plugin bridge started at `{bind}` in {mode} mode"),
    );
}

fn log_external_control_stopped(events: &RuntimeEventLog, status: &ExternalControlStatus) {
    let bind = status.bind_address.as_deref().unwrap_or(DEFAULT_LOCAL_BIND);
    events.push(
        "plugin.bridge.stopped",
        format!("plugin bridge stopped at `{bind}`"),
    );
}

fn external_request_handler(
    core: ArcFlowCore,
    storage: Arc<Mutex<Storage>>,
    runtime: RuntimeState,
    plugin_runtime: PluginRuntimeState,
    preview_state: PreviewPlaybackState,
) -> WsRequestHandler {
    Arc::new(move |session: ClientSession, request: JsonRpcRequest| {
        let core = core.clone();
        let storage = Arc::clone(&storage);
        let runtime = runtime.clone();
        let plugin_runtime = plugin_runtime.clone();
        let preview_state = preview_state.clone();

        Box::pin(async move {
            let id = request.id.clone();

            if is_plugin_registry_external_request(&request) {
                let is_mutation = is_plugin_registry_mutation_external_request(&request);
                let result = {
                    let storage = storage.lock().expect("storage state mutex poisoned");
                    execute_plugin_registry_external_request(&session, &request, &storage)
                };

                return match result {
                    Ok(result) => {
                        if is_mutation {
                            match sync_plugin_runtime_from_storage(&storage, &plugin_runtime).await
                            {
                                Ok(_) => JsonRpcResponse::ok(id, result),
                                Err(error) => {
                                    JsonRpcResponse::error(id, RpcError::new(-32000, error))
                                }
                            }
                        } else {
                            JsonRpcResponse::ok(id, result)
                        }
                    }
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

            if is_preview_playback_external_request(&request) {
                let response = execute_preview_playback_external_request(
                    &session,
                    &request,
                    &core,
                    &runtime,
                    &preview_state,
                );

                return match response {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(error) => JsonRpcResponse::error(id, error),
                };
            }

            if request.method == DEVICE_SCAN_METHOD {
                let response =
                    execute_device_scan_external_request(&session, &core, &runtime).await;

                return match response {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(error) => JsonRpcResponse::error(id, error),
                };
            }

            if request.method == DEVICE_CONNECT_METHOD {
                let response =
                    execute_device_connect_external_request(&session, &request, &core, &runtime)
                        .await;

                return match response {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(error) => JsonRpcResponse::error(id, error),
                };
            }

            if request.method == DEVICE_DISCONNECT_METHOD {
                let response = execute_device_disconnect_external_request(
                    &session,
                    &request,
                    &core,
                    &runtime,
                    &preview_state,
                )
                .await;

                return match response {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(error) => JsonRpcResponse::error(id, error),
                };
            }

            if request.method == DEVICE_REFRESH_BATTERY_METHOD {
                let response = execute_device_refresh_battery_external_request(
                    &session, &request, &core, &runtime,
                )
                .await;

                return match response {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(error) => JsonRpcResponse::error(id, error),
                };
            }

            if request.method == RUNTIME_STATUS_METHOD {
                let response =
                    execute_runtime_status_external_request(&session, &runtime, &plugin_runtime)
                        .await;

                return match response {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(error) => JsonRpcResponse::error(id, error),
                };
            }

            if request.method == RUNTIME_EVENTS_METHOD {
                let response = execute_runtime_events_external_request(&session, &runtime);

                return match response {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(error) => JsonRpcResponse::error(id, error),
                };
            }

            if request.method == RUNTIME_PLUGINS_METHOD {
                let response =
                    execute_runtime_plugins_external_request(&session, &plugin_runtime).await;

                return match response {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(error) => JsonRpcResponse::error(id, error),
                };
            }

            if request.method == PLUGIN_INVOKE_HOOK_METHOD {
                let response = execute_plugin_hook_external_request(
                    &session,
                    &request,
                    &storage,
                    &runtime,
                    &plugin_runtime,
                )
                .await;

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

fn is_preview_playback_external_request(request: &JsonRpcRequest) -> bool {
    matches!(
        request.method.as_str(),
        WAVE_PREVIEW_STATUS_METHOD
            | WAVE_PREVIEW_START_METHOD
            | WAVE_PREVIEW_UPDATE_METHOD
            | WAVE_PREVIEW_STOP_METHOD
    )
}

async fn execute_device_scan_external_request(
    session: &ClientSession,
    core: &ArcFlowCore,
    runtime: &RuntimeState,
) -> Result<serde_json::Value, RpcError> {
    authorize_external_capability(session, DEVICE_SCAN_METHOD, Capability::DeviceRead)?;
    scan_devices_and_sync(core, runtime)
        .await
        .and_then(|response| serde_json::to_value(response).map_err(|error| error.to_string()))
        .map_err(|error| RpcError::new(-32000, error))
}

async fn execute_device_connect_external_request(
    session: &ClientSession,
    request: &JsonRpcRequest,
    core: &ArcFlowCore,
    runtime: &RuntimeState,
) -> Result<serde_json::Value, RpcError> {
    authorize_external_capability(session, DEVICE_CONNECT_METHOD, Capability::WaveControl)?;
    let params: DeviceParams = read_external_params(request)?;
    connect_device_and_scan(core, runtime, DeviceId::new(params.device_id))
        .await
        .and_then(|response| serde_json::to_value(response).map_err(|error| error.to_string()))
        .map_err(|error| RpcError::new(-32000, error))
}

async fn execute_device_disconnect_external_request(
    session: &ClientSession,
    request: &JsonRpcRequest,
    core: &ArcFlowCore,
    runtime: &RuntimeState,
    preview_state: &PreviewPlaybackState,
) -> Result<serde_json::Value, RpcError> {
    authorize_external_capability(session, DEVICE_DISCONNECT_METHOD, Capability::WaveControl)?;
    let params: DeviceParams = read_external_params(request)?;
    disconnect_device_and_scan(
        core,
        runtime,
        Some(preview_state),
        DeviceId::new(params.device_id),
    )
    .await
    .and_then(|response| serde_json::to_value(response).map_err(|error| error.to_string()))
    .map_err(|error| RpcError::new(-32000, error))
}

async fn execute_device_refresh_battery_external_request(
    session: &ClientSession,
    request: &JsonRpcRequest,
    core: &ArcFlowCore,
    runtime: &RuntimeState,
) -> Result<serde_json::Value, RpcError> {
    authorize_external_capability(
        session,
        DEVICE_REFRESH_BATTERY_METHOD,
        Capability::DeviceRead,
    )?;
    let params: DeviceParams = read_external_params(request)?;
    refresh_device_battery_and_scan(core, runtime, DeviceId::new(params.device_id))
        .await
        .and_then(|response| serde_json::to_value(response).map_err(|error| error.to_string()))
        .map_err(|error| RpcError::new(-32000, error))
}

fn execute_preview_playback_external_request(
    session: &ClientSession,
    request: &JsonRpcRequest,
    core: &ArcFlowCore,
    runtime: &RuntimeState,
    preview_state: &PreviewPlaybackState,
) -> Result<serde_json::Value, RpcError> {
    match request.method.as_str() {
        WAVE_PREVIEW_STATUS_METHOD => {
            authorize_external_capability(
                session,
                request.method.as_str(),
                Capability::DeviceRead,
            )?;
            serde_json::to_value(preview_playback_status_from_state(preview_state))
                .map_err(|error| RpcError::new(-32000, error.to_string()))
        }
        WAVE_PREVIEW_START_METHOD => {
            authorize_external_capability(
                session,
                request.method.as_str(),
                Capability::WaveControl,
            )?;
            let params: StartPreviewPlaybackParams = read_external_params(request)?;
            start_preview_playback_session(core, runtime, preview_state, params)
                .and_then(|status| serde_json::to_value(status).map_err(|error| error.to_string()))
                .map_err(|error| RpcError::new(-32000, error))
        }
        WAVE_PREVIEW_UPDATE_METHOD => {
            authorize_external_capability(
                session,
                request.method.as_str(),
                Capability::WaveControl,
            )?;
            let params: UpdatePreviewPlaybackParams = read_external_params(request)?;
            update_preview_playback_session(runtime, preview_state, params)
                .and_then(|status| serde_json::to_value(status).map_err(|error| error.to_string()))
                .map_err(|error| RpcError::new(-32000, error))
        }
        WAVE_PREVIEW_STOP_METHOD => {
            authorize_external_capability(
                session,
                request.method.as_str(),
                Capability::WaveControl,
            )?;
            stop_preview_playback_session(core, runtime, preview_state)
                .and_then(|status| serde_json::to_value(status).map_err(|error| error.to_string()))
                .map_err(|error| RpcError::new(-32000, error))
        }
        method => Err(RpcError::new(
            -32000,
            format!("unsupported preview playback method `{method}`"),
        )),
    }
}

fn authorize_external_capability(
    session: &ClientSession,
    method: &str,
    capability: Capability,
) -> Result<(), RpcError> {
    if session.has_capability(capability) {
        return Ok(());
    }

    Err(RpcError::new(
        -32000,
        format!(
            "session `{}` is missing required capability `{}` for `{}`",
            session.client_name(),
            capability.as_str(),
            method
        ),
    ))
}

fn read_external_params<T>(request: &JsonRpcRequest) -> Result<T, RpcError>
where
    T: for<'de> Deserialize<'de>,
{
    let params = request
        .params
        .clone()
        .ok_or_else(|| RpcError::new(-32000, format!("missing params for `{}`", request.method)))?;

    serde_json::from_value(params).map_err(|error| {
        RpcError::new(
            -32000,
            format!("invalid params for `{}`: {error}", request.method),
        )
    })
}

fn execute_runtime_events_external_request(
    session: &ClientSession,
    runtime: &RuntimeState,
) -> Result<serde_json::Value, RpcError> {
    authorize_external_capability(session, RUNTIME_EVENTS_METHOD, Capability::EventsSubscribe)?;

    serde_json::to_value(runtime_events_from_state(runtime))
        .map_err(|error| RpcError::new(-32000, error.to_string()))
}

async fn execute_runtime_status_external_request(
    session: &ClientSession,
    runtime: &RuntimeState,
    plugin_runtime: &PluginRuntimeState,
) -> Result<serde_json::Value, RpcError> {
    if !session.has_capability(Capability::DeviceRead) {
        return Err(runtime_read_capability_error(
            session,
            RUNTIME_STATUS_METHOD,
        ));
    }

    let loaded_plugin_count = loaded_plugin_count(plugin_runtime).await;

    serde_json::to_value(runtime_status_from_state(runtime, loaded_plugin_count))
        .map_err(|error| RpcError::new(-32000, error.to_string()))
}

async fn execute_runtime_plugins_external_request(
    session: &ClientSession,
    plugin_runtime: &PluginRuntimeState,
) -> Result<serde_json::Value, RpcError> {
    if !session.has_capability(Capability::PluginManage) {
        return Err(plugin_manage_capability_error(
            session,
            RUNTIME_PLUGINS_METHOD,
        ));
    }

    serde_json::to_value(runtime_plugins_from_state(plugin_runtime).await)
        .map_err(|error| RpcError::new(-32000, error.to_string()))
}

async fn execute_plugin_hook_external_request(
    session: &ClientSession,
    request: &JsonRpcRequest,
    storage: &Arc<Mutex<Storage>>,
    runtime: &RuntimeState,
    plugin_runtime: &PluginRuntimeState,
) -> Result<Value, RpcError> {
    authorize_external_capability(session, PLUGIN_INVOKE_HOOK_METHOD, Capability::PluginManage)?;
    let params: PluginHookInvocationParams = read_external_params(request)?;
    let (manifest, output) = {
        let controller = plugin_runtime.controller.lock().await;
        controller
            .invoke_hook_with_manifest(&params.plugin_id, &params.hook, params.payload.clone())
            .await
            .map_err(|error| RpcError::new(-32000, error.to_string()))?
    };
    let action_count = output.actions.len();
    let api = PluginApi::with_host_controllers(
        Arc::clone(storage),
        Arc::clone(&runtime.discovery_controller),
        Arc::clone(&runtime.output_host),
    );
    let results = api
        .handle_output(&manifest, output)
        .await
        .map_err(|error| RpcError::new(-32000, error.to_string()))?;
    runtime.events.push(
        "plugin.hook.invoked",
        format!(
            "plugin `{}` hook `{}` invoked by `{}` with {} host actions",
            params.plugin_id,
            params.hook,
            session.client_name(),
            action_count
        ),
    );

    serde_json::to_value(PluginHookInvocationResponse {
        plugin_id: params.plugin_id,
        hook: params.hook,
        action_count,
        results,
    })
    .map_err(|error| RpcError::new(-32000, error.to_string()))
}

fn runtime_read_capability_error(session: &ClientSession, method: &str) -> RpcError {
    RpcError::new(
        -32000,
        format!(
            "session `{}` is missing required capability `{}` for `{}`",
            session.client_name(),
            Capability::DeviceRead.as_str(),
            method
        ),
    )
}

fn plugin_manage_capability_error(session: &ClientSession, method: &str) -> RpcError {
    RpcError::new(
        -32000,
        format!(
            "session `{}` is missing required capability `{}` for `{}`",
            session.client_name(),
            Capability::PluginManage.as_str(),
            method
        ),
    )
}

fn spawn_preview_playback_loop(
    id: u64,
    core: ArcFlowCore,
    runtime: RuntimeState,
    preview_inner: Arc<Mutex<PreviewPlaybackInner>>,
    mut stop_receiver: oneshot::Receiver<()>,
) {
    tauri::async_runtime::spawn(async move {
        loop {
            let Some(session) = current_preview_session(&preview_inner, id) else {
                break;
            };

            let request = {
                let mut sequences = runtime
                    .preview_sequences
                    .lock()
                    .expect("preview sequence allocator mutex poisoned");
                session.next_request(&mut sequences)
            };

            let request = match request {
                Ok(request) => request,
                Err(error) => {
                    runtime.events.push(
                        "wave.preview.failed",
                        format!(
                            "preview playback for `{}` failed: {error}",
                            session.device_id().as_str()
                        ),
                    );
                    clear_preview_playback_if_current(&preview_inner, id);
                    break;
                }
            };

            if let Err(error) = core.submit_coyote_v3_window(request) {
                runtime.events.push(
                    "wave.preview.failed",
                    format!(
                        "preview playback for `{}` failed: {error}",
                        session.device_id().as_str()
                    ),
                );
                clear_preview_playback_if_current(&preview_inner, id);
                break;
            }

            tokio::select! {
                _ = &mut stop_receiver => {
                    break;
                }
                () = sleep(Duration::from_millis(COYOTE_V3_PREVIEW_INTERVAL_MS)) => {}
            }
        }
    });
}

fn current_preview_session(
    preview_inner: &Arc<Mutex<PreviewPlaybackInner>>,
    id: u64,
) -> Option<CoyoteV3PreviewSession> {
    let inner = preview_inner
        .lock()
        .expect("preview playback state mutex poisoned");
    let running = inner.running.as_ref().filter(|running| running.id == id)?;

    CoyoteV3PreviewSession::new(
        running.device_id.clone(),
        running.channel_a_strength,
        running.channel_b_strength,
    )
    .ok()
}

fn cancel_preview_playback(state: &PreviewPlaybackState, events: &RuntimeEventLog) {
    let running = state
        .inner
        .lock()
        .expect("preview playback state mutex poisoned")
        .running
        .take();

    if let Some(mut running) = running {
        if let Some(stop_sender) = running.stop_sender.take() {
            let _ = stop_sender.send(());
        }

        events.push(
            "wave.preview.stopped",
            format!(
                "preview playback stopped for `{}`",
                running.device_id.as_str()
            ),
        );
    }
}

fn cancel_preview_playback_for_device(
    state: &PreviewPlaybackState,
    events: &RuntimeEventLog,
    device_id: &DeviceId,
) -> bool {
    let running = {
        let mut inner = state
            .inner
            .lock()
            .expect("preview playback state mutex poisoned");

        if inner
            .running
            .as_ref()
            .is_some_and(|running| &running.device_id == device_id)
        {
            inner.running.take()
        } else {
            None
        }
    };

    let Some(mut running) = running else {
        return false;
    };

    if let Some(stop_sender) = running.stop_sender.take() {
        let _ = stop_sender.send(());
    }

    events.push(
        "wave.preview.stopped",
        format!(
            "preview playback stopped for `{}`",
            running.device_id.as_str()
        ),
    );
    true
}

fn clear_preview_playback_if_current(
    state: &Arc<Mutex<PreviewPlaybackInner>>,
    id: u64,
) -> Option<RunningPreviewPlayback> {
    let mut inner = state.lock().expect("preview playback state mutex poisoned");

    if inner
        .running
        .as_ref()
        .is_some_and(|running| running.id == id)
    {
        inner.running.take()
    } else {
        None
    }
}

fn preview_playback_status_from_state(state: &PreviewPlaybackState) -> PreviewPlaybackStatus {
    let inner = state
        .inner
        .lock()
        .expect("preview playback state mutex poisoned");

    inner
        .running
        .as_ref()
        .map(preview_playback_status_from_running)
        .unwrap_or_else(stopped_preview_playback_status)
}

fn preview_playback_status_from_running(running: &RunningPreviewPlayback) -> PreviewPlaybackStatus {
    PreviewPlaybackStatus {
        running: true,
        device_id: Some(running.device_id.as_str().to_owned()),
        channel_a_strength: running.channel_a_strength,
        channel_b_strength: running.channel_b_strength,
        interval_ms: COYOTE_V3_PREVIEW_INTERVAL_MS,
    }
}

fn stopped_preview_playback_status() -> PreviewPlaybackStatus {
    PreviewPlaybackStatus {
        running: false,
        device_id: None,
        channel_a_strength: 0,
        channel_b_strength: 0,
        interval_ms: COYOTE_V3_PREVIEW_INTERVAL_MS,
    }
}

fn runtime_status_from_state(runtime: &RuntimeState, loaded_plugin_count: usize) -> RuntimeStatus {
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
        loaded_plugin_count,
    }
}

async fn scan_devices_and_sync(
    core: &ArcFlowCore,
    runtime: &RuntimeState,
) -> Result<DeviceScanResponse, String> {
    let scan = core
        .scan_devices()
        .await
        .map_err(|error| error.to_string())?;

    sync_active_coyote_v3_output_devices(runtime, &scan);
    let diagnostics = scan_diagnostics(runtime).await;
    log_ble_scan_diagnostics(runtime, diagnostics.as_ref());

    Ok(device_scan_response(scan, diagnostics))
}

async fn connect_device_and_scan(
    core: &ArcFlowCore,
    runtime: &RuntimeState,
    device_id: DeviceId,
) -> Result<DeviceScanResponse, String> {
    let connection = runtime
        .ble_platform_provider
        .connect_device(device_id)
        .await
        .map_err(|error| error.to_string())?;

    runtime.events.push(
        "device.connected",
        match connection.battery_percent {
            Some(percent) => format!(
                "device `{}` connected with {percent}% battery",
                connection.device_id.as_str()
            ),
            None => format!("device `{}` connected", connection.device_id.as_str()),
        },
    );

    let scan = core
        .scan_devices()
        .await
        .map_err(|error| error.to_string())?;
    sync_active_coyote_v3_output_devices(runtime, &scan);
    let diagnostics = scan_diagnostics(runtime).await;
    log_ble_scan_diagnostics(runtime, diagnostics.as_ref());

    Ok(device_scan_response(scan, diagnostics))
}

async fn disconnect_device_and_scan(
    core: &ArcFlowCore,
    runtime: &RuntimeState,
    preview_state: Option<&PreviewPlaybackState>,
    device_id: DeviceId,
) -> Result<DeviceScanResponse, String> {
    if let Some(preview_state) = preview_state {
        cancel_preview_playback_for_device(preview_state, &runtime.events, &device_id);
    }

    if core.active_output_devices().contains(&device_id) {
        core.stop_all_output().map_err(|error| error.to_string())?;
        core.detach_output_device(&device_id)
            .map_err(|error| error.to_string())?;
    }

    runtime
        .ble_platform_provider
        .disconnect_device(&device_id)
        .await
        .map_err(|error| error.to_string())?;

    runtime.events.push(
        "device.disconnected",
        format!("device `{}` disconnected", device_id.as_str()),
    );

    let scan = core
        .scan_devices()
        .await
        .map_err(|error| error.to_string())?;
    sync_active_coyote_v3_output_devices(runtime, &scan);
    let diagnostics = scan_diagnostics(runtime).await;
    log_ble_scan_diagnostics(runtime, diagnostics.as_ref());

    Ok(device_scan_response(scan, diagnostics))
}

async fn refresh_device_battery_and_scan(
    core: &ArcFlowCore,
    runtime: &RuntimeState,
    device_id: DeviceId,
) -> Result<DeviceScanResponse, String> {
    let mut scan = core
        .scan_devices()
        .await
        .map_err(|error| error.to_string())?;

    let Some(device) = scan
        .devices
        .iter_mut()
        .find(|device| device.id == device_id)
    else {
        return Err(format!("BLE device `{}` was not found", device_id.as_str()));
    };

    if !device.connected {
        return Err(format!(
            "BLE device `{}` is not connected",
            device_id.as_str()
        ));
    }

    let battery_percent = runtime
        .ble_platform_provider
        .read_battery_percent(&device_id)
        .await
        .map_err(|error| error.to_string())?;
    device.battery_percent = battery_percent;

    runtime.events.push(
        "device.battery.refreshed",
        match battery_percent {
            Some(percent) => format!(
                "device `{}` battery refreshed to {percent}%",
                device_id.as_str()
            ),
            None => format!("device `{}` battery unavailable", device_id.as_str()),
        },
    );

    sync_active_coyote_v3_output_devices(runtime, &scan);
    let diagnostics = scan_diagnostics(runtime).await;
    log_ble_scan_diagnostics(runtime, diagnostics.as_ref());

    Ok(device_scan_response(scan, diagnostics))
}

fn sync_active_coyote_v3_output_devices(runtime: &RuntimeState, scan: &DeviceScanResult) {
    let output_device_ids: Vec<DeviceId> = scan
        .devices
        .iter()
        .filter(|device| device.connected && device.model == DeviceModel::CoyoteV3)
        .map(|device| device.id.clone())
        .collect();

    for active_device_id in runtime.output_controller.active_devices() {
        if !output_device_ids.contains(&active_device_id) {
            runtime.output_controller.detach_device(&active_device_id);
        }
    }

    let active_device_ids = runtime.output_controller.active_devices();
    for device_id in output_device_ids {
        if active_device_ids.contains(&device_id) {
            continue;
        }

        if let Err(error) = runtime
            .output_controller
            .attach_output_device(device_id.clone())
        {
            runtime.events.push(
                "device.output.activation_failed",
                format!(
                    "device `{}` safety limit setup failed: {error}",
                    device_id.as_str()
                ),
            );
        }
    }
}

async fn scan_diagnostics(runtime: &RuntimeState) -> Option<TauriBleScanDiagnostics> {
    runtime.ble_platform_provider.scan_diagnostics().await
}

fn log_ble_scan_diagnostics(runtime: &RuntimeState, diagnostics: Option<&TauriBleScanDiagnostics>) {
    let Some(diagnostics) = diagnostics else {
        return;
    };
    runtime.events.push(
        "device.scan.diagnostics",
        ble_scan_diagnostics_message(&diagnostics),
    );
}

fn ble_scan_diagnostics_message(diagnostics: &TauriBleScanDiagnostics) -> String {
    let mut message = format!(
        "native BLE scan tried {} time(s), saw {} peripherals, inspected {}, matched {}, skipped unknown {}, missing properties {}",
        diagnostics.scan_attempts,
        diagnostics.discovered_peripherals,
        diagnostics.inspected_peripherals,
        diagnostics.matched_advertisements,
        diagnostics.skipped_unknown_peripherals,
        diagnostics.skipped_missing_properties
    );

    let matched = peripheral_samples_label(&diagnostics.matched_samples);
    if !matched.is_empty() {
        message.push_str("; matched: ");
        message.push_str(&matched);
    }

    let skipped = peripheral_samples_label(&diagnostics.skipped_unknown_samples);
    if !skipped.is_empty() {
        message.push_str("; skipped: ");
        message.push_str(&skipped);
    }

    message
}

fn peripheral_samples_label(samples: &[TauriBlePeripheralDiagnostic]) -> String {
    samples
        .iter()
        .map(peripheral_sample_label)
        .collect::<Vec<_>>()
        .join(", ")
}

fn peripheral_sample_label(sample: &TauriBlePeripheralDiagnostic) -> String {
    let name = sample
        .local_name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("unnamed");
    let services = if sample.service_uuids.is_empty() {
        "no services".to_owned()
    } else {
        sample
            .service_uuids
            .iter()
            .map(|service_uuid| format!("0x{service_uuid:04X}"))
            .collect::<Vec<_>>()
            .join("/")
    };

    format!("{name} [{services}]")
}

async fn loaded_plugin_count(plugin_runtime: &PluginRuntimeState) -> usize {
    plugin_runtime
        .controller
        .lock()
        .await
        .loaded_plugins()
        .len()
}

async fn runtime_plugins_from_state(plugin_runtime: &PluginRuntimeState) -> RuntimePluginsResponse {
    RuntimePluginsResponse {
        plugins: plugin_runtime
            .controller
            .lock()
            .await
            .loaded_plugins()
            .into_iter()
            .map(runtime_plugin_snapshot)
            .collect(),
    }
}

fn runtime_plugin_snapshot(plugin: RecordedPlugin) -> RuntimePluginSnapshot {
    RuntimePluginSnapshot {
        plugin_id: plugin.plugin_id,
        runtime: plugin.runtime,
        entry: plugin.entry,
        bundle_root: plugin.bundle_root,
    }
}

fn runtime_events_from_state(runtime: &RuntimeState) -> RuntimeEventsResponse {
    RuntimeEventsResponse {
        events: runtime.events.list(),
    }
}

async fn sync_plugin_runtime_from_storage(
    storage: &Arc<Mutex<Storage>>,
    plugin_runtime: &PluginRuntimeState,
) -> Result<PluginRuntimeSyncReport, String> {
    let registry = {
        let storage = storage.lock().expect("storage state mutex poisoned");
        PluginRegistryPersistence::new(&storage)
            .load()
            .map_err(|error| error.to_string())?
    };

    let report = {
        let mut controller = plugin_runtime.controller.lock().await;
        controller
            .sync_from_registry(&registry)
            .await
            .map_err(|error| error.to_string())?
    };

    log_plugin_runtime_sync(&plugin_runtime.events, &report);
    Ok(report)
}

fn log_plugin_runtime_sync(events: &RuntimeEventLog, report: &PluginRuntimeSyncReport) {
    for plugin_id in &report.installed_plugin_ids {
        events.push(
            "plugin.installed",
            format!("plugin `{plugin_id}` installed in runtime host"),
        );
    }

    for plugin_id in &report.loaded_plugin_ids {
        events.push(
            "plugin.enabled",
            format!("plugin `{plugin_id}` loaded into sandboxed runtime"),
        );
    }

    for plugin_id in &report.unloaded_plugin_ids {
        events.push(
            "plugin.disabled",
            format!("plugin `{plugin_id}` unloaded from sandboxed runtime"),
        );
    }

    for plugin_id in &report.uninstalled_plugin_ids {
        events.push(
            "plugin.uninstalled",
            format!("plugin `{plugin_id}` removed from runtime host"),
        );
    }
}

fn spawn_plugin_runtime_restore(storage: Arc<Mutex<Storage>>, plugin_runtime: PluginRuntimeState) {
    tauri::async_runtime::spawn(async move {
        match sync_plugin_runtime_from_storage(&storage, &plugin_runtime).await {
            Ok(report) if !report.changed() => plugin_runtime.events.push(
                "plugin.runtime.ready",
                "plugin runtime restored with no enabled plugins",
            ),
            Ok(_) => {}
            Err(error) => plugin_runtime.events.push(
                "plugin.runtime.failed",
                format!("plugin runtime restore failed: {error}"),
            ),
        }
    });
}

fn spawn_runtime_event_listeners(
    events: RuntimeEventLog,
    mut script_events: mpsc::UnboundedReceiver<ScriptWorkerEvent>,
    mut output_events: mpsc::UnboundedReceiver<TauriBleOutputEvent>,
) {
    let script_log = events.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = script_events.recv().await {
            match event {
                ScriptWorkerEvent::Completed(report) => script_log.push(
                    "script.completed",
                    format!(
                        "script `{}` completed {} steps",
                        report.script_id,
                        report.steps.len()
                    ),
                ),
                ScriptWorkerEvent::Failed { script_id, error } => script_log.push(
                    "script.failed",
                    format!("script `{script_id}` failed: {error}"),
                ),
            }
        }
    });

    tauri::async_runtime::spawn(async move {
        while let Some(event) = output_events.recv().await {
            match event {
                TauriBleOutputEvent::Written {
                    device_id,
                    characteristic,
                } => events.push(
                    "ble.write",
                    format!("device `{}` wrote {:?}", device_id.as_str(), characteristic),
                ),
                TauriBleOutputEvent::Failed { device_id, error } => events.push(
                    "ble.write.failed",
                    format!("device `{}` write failed: {error}", device_id.as_str()),
                ),
            }
        }
    });
}

fn device_scan_response(
    result: DeviceScanResult,
    diagnostics: Option<TauriBleScanDiagnostics>,
) -> DeviceScanResponse {
    DeviceScanResponse {
        adapter_status: result.adapter_status.as_str().to_owned(),
        devices: result.devices.into_iter().map(device_response).collect(),
        diagnostics: diagnostics.map(device_scan_diagnostics_response),
    }
}

fn device_scan_diagnostics_response(
    diagnostics: TauriBleScanDiagnostics,
) -> DeviceScanDiagnosticsResponse {
    let message = ble_scan_diagnostics_message(&diagnostics);
    DeviceScanDiagnosticsResponse {
        scan_attempts: diagnostics.scan_attempts,
        discovered_peripherals: diagnostics.discovered_peripherals,
        inspected_peripherals: diagnostics.inspected_peripherals,
        matched_advertisements: diagnostics.matched_advertisements,
        skipped_missing_properties: diagnostics.skipped_missing_properties,
        skipped_unknown_peripherals: diagnostics.skipped_unknown_peripherals,
        matched_samples: diagnostics
            .matched_samples
            .into_iter()
            .map(device_scan_diagnostic_sample_response)
            .collect(),
        skipped_unknown_samples: diagnostics
            .skipped_unknown_samples
            .into_iter()
            .map(device_scan_diagnostic_sample_response)
            .collect(),
        message,
    }
}

fn device_scan_diagnostic_sample_response(
    sample: TauriBlePeripheralDiagnostic,
) -> DeviceScanDiagnosticSampleResponse {
    DeviceScanDiagnosticSampleResponse {
        local_name: sample.local_name,
        service_uuids: sample
            .service_uuids
            .into_iter()
            .map(|service_uuid| format!("0x{service_uuid:04X}"))
            .collect(),
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

fn output_device_activation_response(
    active_devices: Vec<DeviceId>,
) -> OutputDeviceActivationResponse {
    OutputDeviceActivationResponse {
        active_output_devices: active_devices
            .into_iter()
            .map(|device_id| device_id.as_str().to_owned())
            .collect(),
    }
}

fn submit_wave_window_response(
    result: SubmitOutputResult,
    sequence: u8,
) -> SubmitWaveWindowResponse {
    SubmitWaveWindowResponse {
        device_id: result.device_id.as_str().to_owned(),
        sequence,
        accepted: result.accepted,
    }
}

fn platform_ble_provider() -> Arc<dyn TauriBlePlatformProvider> {
    #[cfg(feature = "native-ble")]
    {
        return Arc::new(NativeBlePlatformProvider::new());
    }

    #[cfg(not(feature = "native-ble"))]
    {
        Arc::new(arcflow_tauri_platform::UnsupportedTauriBlePlatformProvider)
    }
}

/// Runs the shared ArcFlow Tauri application with a platform app context.
pub fn run<R>(context: tauri::Context<R>)
where
    R: tauri::Runtime,
{
    let builder = tauri::Builder::<R>::new();
    #[cfg(feature = "single-instance")]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
        }
    }));

    builder
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_data_dir)?;
            let database_path = app_data_dir.join("arcflow.sqlite3");
            let storage = Arc::new(Mutex::new(Storage::open(&database_path)?));
            let safety_limits = SafetyLimits::conservative();
            let events = RuntimeEventLog::default();
            let plugin_runtime_controller =
                Arc::new(AsyncMutex::new(RecordingPluginRuntimeController::new()));
            let plugin_runtime = PluginRuntimeState {
                controller: Arc::clone(&plugin_runtime_controller),
                events: events.clone(),
            };
            let (script_event_sender, script_events) = mpsc::unbounded_channel();
            let (output_event_sender, output_events) = mpsc::unbounded_channel();
            let ble_platform_provider = platform_ble_provider();
            let discovery_provider: Arc<dyn TauriBleDiscoveryProvider> =
                ble_platform_provider.clone();
            let ble_transport_provider: Arc<dyn TauriBleTransportProvider> =
                ble_platform_provider.clone();
            let discovery_controller: Arc<dyn DeviceDiscoveryController> = Arc::new(
                TauriBleDiscoveryController::with_provider(discovery_provider),
            );
            let output_sink = TauriBleOutputSink::spawn_with_event_sink(
                ble_transport_provider,
                output_event_sender,
            );
            let output_controller = Arc::new(CoyoteV3OutputController::new(
                safety_limits,
                output_sink.clone(),
            ));
            let core_output_controller: Arc<dyn DeviceOutputController> = output_controller.clone();
            let plugin_hook_invoker = Arc::new(RecordingPluginRuntimeHookInvoker::with_plugin_api(
                Arc::clone(&plugin_runtime_controller),
                Arc::clone(&storage),
                Arc::clone(&discovery_controller),
                Arc::clone(&core_output_controller),
            ));
            let script_actions = CoreScriptActionExecutor::with_plugin_hooks(
                Arc::clone(&discovery_controller),
                Arc::clone(&core_output_controller),
                plugin_hook_invoker,
            );
            let script_queue = ScriptWorkerQueue::spawn_with_event_sink(
                Arc::new(script_actions),
                script_event_sender,
            );
            let script_runner = StorageScriptRunner::with_queue(
                Arc::clone(&storage),
                ScriptCompiler::default(),
                Arc::new(script_queue),
            );
            spawn_runtime_event_listeners(events.clone(), script_events, output_events);
            spawn_plugin_runtime_restore(Arc::clone(&storage), plugin_runtime.clone());

            app.manage(StorageState {
                storage,
                database_path,
            });
            app.manage(RuntimeState {
                discovery_controller: Arc::clone(&discovery_controller),
                ble_platform_provider: Arc::clone(&ble_platform_provider),
                output_controller,
                output_host: Arc::clone(&core_output_controller),
                output_sink,
                events,
                preview_sequences: Arc::new(Mutex::new(CoyoteV3SequenceAllocator::default())),
            });
            app.manage(plugin_runtime);
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
        .manage(PreviewPlaybackState::default())
        .invoke_handler(tauri::generate_handler![
            app_status,
            frontend_platform,
            storage_status,
            runtime_status,
            runtime_events,
            runtime_plugins,
            plugin_registry,
            install_plugin_manifest,
            install_plugin_bundle,
            set_plugin_enabled,
            delete_plugin,
            list_scripts,
            upsert_script,
            delete_script,
            run_script,
            scan_devices,
            connect_device,
            disconnect_device,
            refresh_device_battery,
            stop_output,
            activate_output_device,
            deactivate_output_device,
            submit_preview_window,
            preview_playback_status,
            start_preview_playback,
            update_preview_playback,
            stop_preview_playback,
            external_control_status,
            start_external_control,
            stop_external_control
        ])
        .run(context)
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::Arc,
        time::{SystemTime, UNIX_EPOCH},
    };

    use arcflow_core::NoopScriptRunner;
    use arcflow_external_control::{ClientHello, PROTOCOL_VERSION};
    use arcflow_plugin_runtime::{PluginManifest, PluginRegistry};
    use arcflow_tauri_platform::{
        TauriBleConnectionState, TauriBleDiscoveryState, TauriBleSubscriptionRequest,
        TauriBleWriteRequest, UnsupportedTauriBlePlatformProvider,
        UnsupportedTauriBleTransportProvider,
    };
    use tokio::sync::Mutex as AsyncMutex;

    use super::*;

    fn manifest(id: &str, runtime: RuntimeKind) -> PluginManifest {
        PluginManifest {
            id: id.to_owned(),
            name: "Plugin".to_owned(),
            version: "1.0.0".to_owned(),
            runtime,
            entry: match runtime {
                RuntimeKind::Wasm => "dist/plugin.wasm".to_owned(),
                RuntimeKind::JavaScript => "dist/plugin.js".to_owned(),
            },
            api_version: "1".to_owned(),
            capabilities: vec![Capability::DeviceRead],
        }
    }

    fn session_with(capabilities: Vec<Capability>) -> ClientSession {
        GatewayPolicy::new(capabilities.clone())
            .accept(ClientHello {
                client_name: "Runtime Inspector".to_owned(),
                protocol_version: PROTOCOL_VERSION,
                requested_capabilities: capabilities,
            })
            .unwrap()
    }

    fn empty_plugin_runtime_state() -> PluginRuntimeState {
        PluginRuntimeState {
            controller: Arc::new(AsyncMutex::new(RecordingPluginRuntimeController::new())),
            events: RuntimeEventLog::default(),
        }
    }

    fn runtime_state_with_core() -> (RuntimeState, ArcFlowCore) {
        runtime_state_with_platform_provider(Arc::new(UnsupportedTauriBlePlatformProvider))
    }

    fn runtime_state_with_platform_provider(
        platform_provider: Arc<dyn TauriBlePlatformProvider>,
    ) -> (RuntimeState, ArcFlowCore) {
        let discovery_provider: Arc<dyn TauriBleDiscoveryProvider> = platform_provider.clone();
        let transport_provider: Arc<dyn TauriBleTransportProvider> = platform_provider.clone();
        let discovery_controller: Arc<dyn DeviceDiscoveryController> = Arc::new(
            TauriBleDiscoveryController::with_provider(discovery_provider),
        );
        let output_sink = TauriBleOutputSink::spawn(transport_provider);
        let output_controller = Arc::new(CoyoteV3OutputController::new(
            SafetyLimits::conservative(),
            output_sink.clone(),
        ));
        let core_output_controller: Arc<dyn DeviceOutputController> = output_controller.clone();
        let core = ArcFlowCore::with_controllers(
            SafetyLimits::conservative(),
            Arc::clone(&discovery_controller),
            Arc::clone(&core_output_controller),
            Arc::new(NoopScriptRunner),
        );
        let runtime = RuntimeState {
            discovery_controller,
            ble_platform_provider: platform_provider,
            output_controller,
            output_host: core_output_controller,
            output_sink,
            events: RuntimeEventLog::default(),
            preview_sequences: Arc::new(Mutex::new(CoyoteV3SequenceAllocator::default())),
        };

        (runtime, core)
    }

    #[derive(Debug)]
    struct ConnectingBleProvider {
        connected: Mutex<Vec<DeviceId>>,
        battery_percent: Option<u8>,
        battery_reads: Mutex<usize>,
    }

    impl ConnectingBleProvider {
        fn new(battery_percent: Option<u8>) -> Self {
            Self {
                connected: Mutex::new(Vec::new()),
                battery_percent,
                battery_reads: Mutex::new(0),
            }
        }

        fn connected_devices(&self) -> Vec<DeviceId> {
            self.connected
                .lock()
                .expect("connected BLE test provider mutex poisoned")
                .clone()
        }

        fn disconnect(&self, device_id: &DeviceId) {
            self.connected
                .lock()
                .expect("connected BLE test provider mutex poisoned")
                .retain(|connected_id| connected_id != device_id);
        }

        fn battery_reads(&self) -> usize {
            *self
                .battery_reads
                .lock()
                .expect("connected BLE battery read mutex poisoned")
        }
    }

    #[async_trait::async_trait]
    impl TauriBleDiscoveryProvider for ConnectingBleProvider {
        async fn scan_state(&self) -> Result<TauriBleDiscoveryState, arcflow_core::CoreError> {
            let device_id = DeviceId::new("coyote-v3");
            let connected = self
                .connected
                .lock()
                .expect("connected BLE test provider mutex poisoned")
                .contains(&device_id);

            Ok(TauriBleDiscoveryState::Ready {
                advertisements: vec![arcflow_core::BleAdvertisement::new(
                    device_id,
                    Some("Coyote V3".to_owned()),
                    Some(-40),
                    vec![arcflow_core::COYOTE_V3_SERVICE_UUID],
                )
                .with_connected(connected)
                .with_battery_percent(connected.then_some(self.battery_percent).flatten())],
            })
        }
    }

    #[async_trait::async_trait]
    impl TauriBleTransportProvider for ConnectingBleProvider {
        async fn write(
            &self,
            _request: TauriBleWriteRequest,
        ) -> Result<(), arcflow_core::CoreError> {
            Ok(())
        }

        async fn subscribe(
            &self,
            _request: TauriBleSubscriptionRequest,
        ) -> Result<(), arcflow_core::CoreError> {
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl TauriBlePlatformProvider for ConnectingBleProvider {
        async fn connect_device(
            &self,
            device_id: DeviceId,
        ) -> Result<TauriBleConnectionState, arcflow_core::CoreError> {
            self.connected
                .lock()
                .expect("connected BLE test provider mutex poisoned")
                .push(device_id.clone());

            Ok(TauriBleConnectionState::new(
                device_id,
                self.battery_percent,
            ))
        }

        async fn read_battery_percent(
            &self,
            _device_id: &DeviceId,
        ) -> Result<Option<u8>, arcflow_core::CoreError> {
            *self
                .battery_reads
                .lock()
                .expect("connected BLE battery read mutex poisoned") += 1;
            Ok(self.battery_percent)
        }

        async fn disconnect_device(
            &self,
            device_id: &DeviceId,
        ) -> Result<(), arcflow_core::CoreError> {
            self.disconnect(device_id);
            Ok(())
        }

        async fn scan_diagnostics(&self) -> Option<TauriBleScanDiagnostics> {
            let mut diagnostics = TauriBleScanDiagnostics::new(1);
            diagnostics.inspected_peripherals = 1;
            diagnostics.record_matched(Some("Coyote V3"), &[arcflow_core::COYOTE_V3_SERVICE_UUID]);
            Some(diagnostics)
        }
    }

    fn test_bundle_root(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("arcflow-tauri-plugin-{name}-{suffix}"))
    }

    fn write_javascript_bundle(name: &str, plugin_id: &str, source: &[u8]) -> PathBuf {
        let root = test_bundle_root(name);
        let entry_path = root.join("dist/plugin.js");
        fs::create_dir_all(entry_path.parent().unwrap()).unwrap();
        fs::write(entry_path, source).unwrap();
        fs::write(
            root.join("manifest.json"),
            format!(
                r#"{{
                    "id":"{plugin_id}",
                    "name":"External Hook Plugin",
                    "version":"0.1.0",
                    "runtime":"javascript",
                    "entry":"dist/plugin.js",
                    "apiVersion":"1",
                    "capabilities":["storage.private"]
                }}"#
            ),
        )
        .unwrap();
        root
    }

    #[test]
    fn ble_scan_diagnostics_message_includes_counts_and_samples() {
        let mut diagnostics = TauriBleScanDiagnostics::new(3);
        diagnostics.inspected_peripherals = 2;
        diagnostics.skipped_missing_properties = 1;
        diagnostics.record_matched(Some("47L121000"), &[arcflow_core::COYOTE_V3_SERVICE_UUID]);
        diagnostics.record_unknown(Some("Keyboard"), &[]);

        let message = ble_scan_diagnostics_message(&diagnostics);

        assert!(message.contains("tried 1 time(s)"));
        assert!(message.contains("saw 3 peripherals"));
        assert!(message.contains("inspected 2"));
        assert!(message.contains("matched 1"));
        assert!(message.contains("skipped unknown 1"));
        assert!(message.contains("missing properties 1"));
        assert!(message.contains("matched: 47L121000 [0x180C]"));
        assert!(message.contains("skipped: Keyboard [no services]"));
    }

    #[tokio::test]
    async fn runtime_plugins_from_state_lists_loaded_plugins() {
        let state = empty_plugin_runtime_state();
        let mut registry = PluginRegistry::new();
        registry
            .install(manifest("plugin.wasm", RuntimeKind::Wasm))
            .unwrap();
        registry
            .install(manifest("plugin.js", RuntimeKind::JavaScript))
            .unwrap();
        registry.enable("plugin.wasm").unwrap();
        registry.enable("plugin.js").unwrap();
        state
            .controller
            .lock()
            .await
            .sync_from_registry(&registry)
            .await
            .unwrap();

        let response = runtime_plugins_from_state(&state).await;

        assert_eq!(
            response.plugins,
            vec![
                RuntimePluginSnapshot {
                    plugin_id: "plugin.js".to_owned(),
                    runtime: RuntimeKind::JavaScript,
                    entry: "dist/plugin.js".to_owned(),
                    bundle_root: None,
                },
                RuntimePluginSnapshot {
                    plugin_id: "plugin.wasm".to_owned(),
                    runtime: RuntimeKind::Wasm,
                    entry: "dist/plugin.wasm".to_owned(),
                    bundle_root: None,
                },
            ]
        );
    }

    #[tokio::test]
    async fn runtime_plugins_request_requires_plugin_manage() {
        let state = empty_plugin_runtime_state();
        let session = session_with(vec![Capability::DeviceRead]);

        let error = execute_runtime_plugins_external_request(&session, &state)
            .await
            .unwrap_err();

        assert_eq!(error.code, -32000);
        assert!(error.message.contains("plugin.manage"));
        assert!(error.message.contains(RUNTIME_PLUGINS_METHOD));
    }

    #[tokio::test]
    async fn plugin_hook_external_request_requires_plugin_manage() {
        let storage = Arc::new(Mutex::new(Storage::in_memory().unwrap()));
        let (runtime, _) = runtime_state_with_core();
        let state = empty_plugin_runtime_state();
        let session = session_with(vec![Capability::DeviceRead]);
        let request = JsonRpcRequest::new(
            arcflow_external_control::RequestId::Number(30),
            PLUGIN_INVOKE_HOOK_METHOD,
            Some(serde_json::json!({
                "pluginId": "plugin.js",
                "hook": "external.ping"
            })),
        );

        let error =
            execute_plugin_hook_external_request(&session, &request, &storage, &runtime, &state)
                .await
                .unwrap_err();

        assert_eq!(error.code, -32000);
        assert!(error.message.contains("plugin.manage"));
        assert!(error.message.contains(PLUGIN_INVOKE_HOOK_METHOD));
    }

    #[tokio::test]
    async fn plugin_hook_external_request_routes_output_through_plugin_api() {
        let storage = Arc::new(Mutex::new(Storage::in_memory().unwrap()));
        let (runtime, _) = runtime_state_with_core();
        let state = empty_plugin_runtime_state();
        let bundle_root = write_javascript_bundle(
            "invoke-hook",
            "plugin.js",
            br#"export const arcflowPlugin = {
                "hooks": {
                    "external.connected": {
                        "actions": [
                            {
                                "method": "storage.private.put",
                                "params": {
                                    "key": "lastExternalPayload",
                                    "value": {"source": "ws"}
                                }
                            }
                        ]
                    }
                }
            };"#,
        );
        let mut registry = PluginRegistry::new();
        let plugin = PluginManifest {
            id: "plugin.js".to_owned(),
            name: "Plugin".to_owned(),
            version: "1.0.0".to_owned(),
            runtime: RuntimeKind::JavaScript,
            entry: "dist/plugin.js".to_owned(),
            api_version: "1".to_owned(),
            capabilities: vec![Capability::StoragePrivate],
        };
        registry
            .install_with_bundle_root(plugin, bundle_root.display().to_string())
            .unwrap();
        registry.enable("plugin.js").unwrap();
        state
            .controller
            .lock()
            .await
            .sync_from_registry(&registry)
            .await
            .unwrap();
        let session = session_with(vec![Capability::PluginManage]);
        let request = JsonRpcRequest::new(
            arcflow_external_control::RequestId::Number(31),
            PLUGIN_INVOKE_HOOK_METHOD,
            Some(serde_json::json!({
                "pluginId": "plugin.js",
                "hook": "external.connected",
                "payload": {"clientName": "test"}
            })),
        );

        let response =
            execute_plugin_hook_external_request(&session, &request, &storage, &runtime, &state)
                .await
                .unwrap();

        assert_eq!(response["pluginId"], "plugin.js");
        assert_eq!(response["hook"], "external.connected");
        assert_eq!(response["actionCount"], 1);
        assert_eq!(response["results"][0]["stored"], true);
        let stored = storage
            .lock()
            .unwrap()
            .plugin_kv()
            .get("plugin.js", "lastExternalPayload")
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&stored).unwrap(),
            serde_json::json!({"source": "ws"})
        );
        assert_eq!(runtime.events.list()[0].kind, "plugin.hook.invoked");
        assert!(runtime.events.list()[0]
            .message
            .contains("plugin `plugin.js` hook `external.connected`"));

        fs::remove_dir_all(bundle_root).unwrap();
    }

    #[tokio::test]
    async fn runtime_events_request_requires_events_subscribe() {
        let (runtime, _) = runtime_state_with_core();
        let session = session_with(vec![Capability::DeviceRead]);

        let error = execute_runtime_events_external_request(&session, &runtime).unwrap_err();

        assert_eq!(error.code, -32000);
        assert!(error.message.contains("events.subscribe"));
        assert!(error.message.contains(RUNTIME_EVENTS_METHOD));
    }

    #[tokio::test]
    async fn runtime_events_request_accepts_events_subscribe_without_device_read() {
        let (runtime, _) = runtime_state_with_core();
        runtime.events.push("runtime.ready", "runtime ready");
        let session = session_with(vec![Capability::EventsSubscribe]);

        let response = execute_runtime_events_external_request(&session, &runtime).unwrap();

        assert_eq!(response["events"][0]["kind"], "runtime.ready");
        assert_eq!(response["events"][0]["message"], "runtime ready");
    }

    #[tokio::test]
    async fn device_scan_external_request_requires_device_read() {
        let (runtime, core) = runtime_state_with_core();
        let session = session_with(vec![Capability::WaveControl]);

        let error = execute_device_scan_external_request(&session, &core, &runtime)
            .await
            .unwrap_err();

        assert_eq!(error.code, -32000);
        assert!(error.message.contains("device.read"));
        assert!(error.message.contains(DEVICE_SCAN_METHOD));
    }

    #[tokio::test]
    async fn device_scan_external_request_returns_scan() {
        let provider = Arc::new(ConnectingBleProvider::new(Some(88)));
        let (runtime, core) = runtime_state_with_platform_provider(provider);
        let session = session_with(vec![Capability::DeviceRead]);

        let response = execute_device_scan_external_request(&session, &core, &runtime)
            .await
            .unwrap();

        assert_eq!(response["adapterStatus"], "ready");
        assert_eq!(response["devices"][0]["id"], "coyote-v3");
        assert_eq!(response["devices"][0]["connected"], false);
        assert_eq!(response["diagnostics"]["scanAttempts"], 1);
        assert_eq!(response["diagnostics"]["discoveredPeripherals"], 1);
        assert_eq!(response["diagnostics"]["matchedAdvertisements"], 1);
        assert_eq!(
            response["diagnostics"]["matchedSamples"][0]["serviceUuids"][0],
            "0x180C"
        );
        assert!(response["diagnostics"]["message"]
            .as_str()
            .unwrap()
            .contains("matched: Coyote V3 [0x180C]"));
    }

    #[tokio::test]
    async fn device_connect_external_request_requires_wave_control() {
        let (runtime, core) = runtime_state_with_core();
        let session = session_with(vec![Capability::DeviceRead]);
        let request = JsonRpcRequest::new(
            arcflow_external_control::RequestId::Number(40),
            DEVICE_CONNECT_METHOD,
            Some(serde_json::json!({ "deviceId": "coyote-v3" })),
        );

        let error = execute_device_connect_external_request(&session, &request, &core, &runtime)
            .await
            .unwrap_err();

        assert_eq!(error.code, -32000);
        assert!(error.message.contains("wave.control"));
        assert!(error.message.contains(DEVICE_CONNECT_METHOD));
    }

    #[tokio::test]
    async fn device_connect_external_request_connects_and_returns_scan() {
        let provider = Arc::new(ConnectingBleProvider::new(Some(88)));
        let (runtime, core) = runtime_state_with_platform_provider(provider.clone());
        let session = session_with(vec![Capability::WaveControl]);
        let request = JsonRpcRequest::new(
            arcflow_external_control::RequestId::Number(41),
            DEVICE_CONNECT_METHOD,
            Some(serde_json::json!({ "deviceId": "coyote-v3" })),
        );

        let response = execute_device_connect_external_request(&session, &request, &core, &runtime)
            .await
            .unwrap();

        assert_eq!(
            provider.connected_devices(),
            vec![DeviceId::new("coyote-v3")]
        );
        assert_eq!(response["adapterStatus"], "ready");
        assert_eq!(response["devices"][0]["id"], "coyote-v3");
        assert_eq!(response["devices"][0]["connected"], true);
        assert_eq!(response["devices"][0]["batteryPercent"], 88);
        assert_eq!(
            runtime.output_controller.active_devices(),
            vec![DeviceId::new("coyote-v3")]
        );
        assert_eq!(runtime.events.list()[0].kind, "device.connected");
    }

    #[tokio::test]
    async fn request_handler_routes_device_connect_before_core_fallback() {
        let provider = Arc::new(ConnectingBleProvider::new(Some(91)));
        let (runtime, core) = runtime_state_with_platform_provider(provider.clone());
        let handler = external_request_handler(
            core,
            Arc::new(Mutex::new(Storage::in_memory().unwrap())),
            runtime,
            empty_plugin_runtime_state(),
            PreviewPlaybackState::default(),
        );
        let session = session_with(vec![Capability::WaveControl]);
        let request = JsonRpcRequest::new(
            arcflow_external_control::RequestId::Number(42),
            DEVICE_CONNECT_METHOD,
            Some(serde_json::json!({ "deviceId": "coyote-v3" })),
        );

        let response = handler(session, request).await;

        assert!(response.error.is_none());
        assert_eq!(
            provider.connected_devices(),
            vec![DeviceId::new("coyote-v3")]
        );
        assert_eq!(response.result.unwrap()["devices"][0]["batteryPercent"], 91);
    }

    #[tokio::test]
    async fn device_disconnect_external_request_disconnects_and_returns_scan() {
        let provider = Arc::new(ConnectingBleProvider::new(Some(88)));
        let (runtime, core) = runtime_state_with_platform_provider(provider.clone());
        let preview_state = PreviewPlaybackState::default();
        let session = session_with(vec![Capability::WaveControl]);
        let connect = JsonRpcRequest::new(
            arcflow_external_control::RequestId::Number(43),
            DEVICE_CONNECT_METHOD,
            Some(serde_json::json!({ "deviceId": "coyote-v3" })),
        );
        let disconnect = JsonRpcRequest::new(
            arcflow_external_control::RequestId::Number(44),
            DEVICE_DISCONNECT_METHOD,
            Some(serde_json::json!({ "deviceId": "coyote-v3" })),
        );

        execute_device_connect_external_request(&session, &connect, &core, &runtime)
            .await
            .unwrap();
        let (stop_sender, _stop_receiver) = oneshot::channel();
        preview_state
            .inner
            .lock()
            .expect("preview playback state mutex poisoned")
            .running = Some(RunningPreviewPlayback {
            id: 6,
            device_id: DeviceId::new("coyote-v3"),
            channel_a_strength: 4,
            channel_b_strength: 0,
            stop_sender: Some(stop_sender),
        });
        let response = execute_device_disconnect_external_request(
            &session,
            &disconnect,
            &core,
            &runtime,
            &preview_state,
        )
        .await
        .unwrap();

        assert!(provider.connected_devices().is_empty());
        assert_eq!(response["adapterStatus"], "ready");
        assert_eq!(response["devices"][0]["id"], "coyote-v3");
        assert_eq!(response["devices"][0]["connected"], false);
        assert!(runtime.output_controller.active_devices().is_empty());
        assert!(!preview_playback_status_from_state(&preview_state).running);
        assert!(runtime
            .events
            .list()
            .iter()
            .any(|event| event.kind == "device.disconnected"));
    }

    #[tokio::test]
    async fn device_refresh_battery_external_request_requires_device_read() {
        let provider = Arc::new(ConnectingBleProvider::new(Some(88)));
        let (runtime, core) = runtime_state_with_platform_provider(provider);
        let session = session_with(vec![Capability::WaveControl]);
        let request = JsonRpcRequest::new(
            arcflow_external_control::RequestId::Number(45),
            DEVICE_REFRESH_BATTERY_METHOD,
            Some(serde_json::json!({ "deviceId": "coyote-v3" })),
        );

        let error =
            execute_device_refresh_battery_external_request(&session, &request, &core, &runtime)
                .await
                .unwrap_err();

        assert_eq!(error.code, -32000);
        assert!(error.message.contains("device.read"));
        assert!(error.message.contains(DEVICE_REFRESH_BATTERY_METHOD));
    }

    #[tokio::test]
    async fn device_refresh_battery_external_request_reads_connected_battery() {
        let provider = Arc::new(ConnectingBleProvider::new(Some(73)));
        let (runtime, core) = runtime_state_with_platform_provider(provider.clone());
        let connect_session = session_with(vec![Capability::WaveControl]);
        let refresh_session = session_with(vec![Capability::DeviceRead]);
        let connect = JsonRpcRequest::new(
            arcflow_external_control::RequestId::Number(46),
            DEVICE_CONNECT_METHOD,
            Some(serde_json::json!({ "deviceId": "coyote-v3" })),
        );
        let refresh = JsonRpcRequest::new(
            arcflow_external_control::RequestId::Number(47),
            DEVICE_REFRESH_BATTERY_METHOD,
            Some(serde_json::json!({ "deviceId": "coyote-v3" })),
        );

        execute_device_connect_external_request(&connect_session, &connect, &core, &runtime)
            .await
            .unwrap();
        let response = execute_device_refresh_battery_external_request(
            &refresh_session,
            &refresh,
            &core,
            &runtime,
        )
        .await
        .unwrap();

        assert_eq!(provider.battery_reads(), 1);
        assert_eq!(response["devices"][0]["batteryPercent"], 73);
        assert!(runtime
            .events
            .list()
            .iter()
            .any(|event| event.kind == "device.battery.refreshed"));
    }

    #[tokio::test]
    async fn preview_status_external_request_requires_device_read() {
        let (runtime, core) = runtime_state_with_core();
        let preview_state = PreviewPlaybackState::default();
        let session = session_with(vec![Capability::DeviceRead]);
        let request = JsonRpcRequest::new(
            arcflow_external_control::RequestId::Number(20),
            WAVE_PREVIEW_STATUS_METHOD,
            None,
        );

        let result = execute_preview_playback_external_request(
            &session,
            &request,
            &core,
            &runtime,
            &preview_state,
        )
        .unwrap();

        assert_eq!(result["running"], false);
        assert_eq!(result["deviceId"], serde_json::Value::Null);
    }

    #[tokio::test]
    async fn preview_start_external_request_requires_wave_control() {
        let (runtime, core) = runtime_state_with_core();
        let preview_state = PreviewPlaybackState::default();
        let session = session_with(vec![Capability::DeviceRead]);
        let request = JsonRpcRequest::new(
            arcflow_external_control::RequestId::Number(21),
            WAVE_PREVIEW_START_METHOD,
            Some(serde_json::json!({
                "deviceId": "coyote-v3",
                "channelAStrength": 12,
                "channelBStrength": 0
            })),
        );

        let error = execute_preview_playback_external_request(
            &session,
            &request,
            &core,
            &runtime,
            &preview_state,
        )
        .unwrap_err();

        assert!(error.message.contains("wave.control"));
        assert!(error.message.contains(WAVE_PREVIEW_START_METHOD));
    }

    #[tokio::test]
    async fn preview_external_requests_start_and_stop_backend_session() {
        let (runtime, core) = runtime_state_with_core();
        let preview_state = PreviewPlaybackState::default();
        let session = session_with(vec![Capability::WaveControl]);
        core.attach_output_device(DeviceId::new("coyote-v3"))
            .unwrap();
        let start = JsonRpcRequest::new(
            arcflow_external_control::RequestId::Number(22),
            WAVE_PREVIEW_START_METHOD,
            Some(serde_json::json!({
                "deviceId": "coyote-v3",
                "channelAStrength": 12,
                "channelBStrength": 0
            })),
        );
        let stop = JsonRpcRequest::new(
            arcflow_external_control::RequestId::Number(23),
            WAVE_PREVIEW_STOP_METHOD,
            None,
        );

        let started = execute_preview_playback_external_request(
            &session,
            &start,
            &core,
            &runtime,
            &preview_state,
        )
        .unwrap();
        let stopped = execute_preview_playback_external_request(
            &session,
            &stop,
            &core,
            &runtime,
            &preview_state,
        )
        .unwrap();

        assert_eq!(started["running"], true);
        assert_eq!(started["deviceId"], "coyote-v3");
        assert_eq!(stopped["running"], false);
    }

    #[tokio::test]
    async fn preview_update_external_request_updates_backend_session() {
        let (runtime, core) = runtime_state_with_core();
        let preview_state = PreviewPlaybackState::default();
        let session = session_with(vec![Capability::WaveControl]);
        core.attach_output_device(DeviceId::new("coyote-v3"))
            .unwrap();
        let start = JsonRpcRequest::new(
            arcflow_external_control::RequestId::Number(24),
            WAVE_PREVIEW_START_METHOD,
            Some(serde_json::json!({
                "deviceId": "coyote-v3",
                "channelAStrength": 4,
                "channelBStrength": 0
            })),
        );
        let update = JsonRpcRequest::new(
            arcflow_external_control::RequestId::Number(25),
            WAVE_PREVIEW_UPDATE_METHOD,
            Some(serde_json::json!({
                "channelAStrength": 9,
                "channelBStrength": 1
            })),
        );

        execute_preview_playback_external_request(
            &session,
            &start,
            &core,
            &runtime,
            &preview_state,
        )
        .unwrap();
        let updated = execute_preview_playback_external_request(
            &session,
            &update,
            &core,
            &runtime,
            &preview_state,
        )
        .unwrap();

        assert_eq!(updated["running"], true);
        assert_eq!(updated["channelAStrength"], 9);
        assert_eq!(updated["channelBStrength"], 1);
        assert!(runtime
            .events
            .list()
            .iter()
            .any(|event| event.kind == "wave.preview.updated"));
        cancel_preview_playback(&preview_state, &runtime.events);
    }

    #[tokio::test]
    async fn syncs_connected_coyote_v3_devices_to_output_controller() {
        let output_sink = TauriBleOutputSink::spawn(Arc::new(UnsupportedTauriBleTransportProvider));
        let output_controller = Arc::new(CoyoteV3OutputController::new(
            SafetyLimits::conservative(),
            output_sink.clone(),
        ));
        let discovery_controller: Arc<dyn DeviceDiscoveryController> =
            Arc::new(TauriBleDiscoveryController::unsupported());
        let output_host: Arc<dyn DeviceOutputController> = output_controller.clone();
        output_controller.attach_device(DeviceId::new("stale-v3"));
        output_controller.attach_device(DeviceId::new("kept-v3"));
        let runtime = RuntimeState {
            discovery_controller,
            ble_platform_provider: Arc::new(UnsupportedTauriBlePlatformProvider),
            output_controller,
            output_host,
            output_sink,
            events: RuntimeEventLog::default(),
            preview_sequences: Arc::new(Mutex::new(CoyoteV3SequenceAllocator::default())),
        };
        let scan = DeviceScanResult::new(
            arcflow_core::BleAdapterStatus::Ready,
            vec![
                DeviceStatus {
                    id: DeviceId::new("kept-v3"),
                    model: DeviceModel::CoyoteV3,
                    battery_percent: None,
                    connected: true,
                },
                DeviceStatus {
                    id: DeviceId::new("new-v3"),
                    model: DeviceModel::CoyoteV3,
                    battery_percent: None,
                    connected: true,
                },
                DeviceStatus {
                    id: DeviceId::new("ignored-v2"),
                    model: DeviceModel::CoyoteV2,
                    battery_percent: None,
                    connected: true,
                },
                DeviceStatus {
                    id: DeviceId::new("ignored-disconnected-v3"),
                    model: DeviceModel::CoyoteV3,
                    battery_percent: None,
                    connected: false,
                },
            ],
        );

        sync_active_coyote_v3_output_devices(&runtime, &scan);

        assert_eq!(
            runtime.output_controller.active_devices(),
            vec![DeviceId::new("kept-v3"), DeviceId::new("new-v3")]
        );
        assert_eq!(runtime.output_sink.stats().queued, 1);
    }

    #[test]
    fn output_activation_response_lists_active_devices() {
        let response = output_device_activation_response(vec![
            DeviceId::new("coyote-a"),
            DeviceId::new("coyote-b"),
        ]);

        assert_eq!(
            response,
            OutputDeviceActivationResponse {
                active_output_devices: vec!["coyote-a".to_owned(), "coyote-b".to_owned()],
            }
        );
    }

    #[test]
    fn submit_wave_window_response_reports_acceptance() {
        let response = submit_wave_window_response(
            SubmitOutputResult::new(DeviceId::new("coyote-v3"), true),
            7,
        );

        assert_eq!(
            response,
            SubmitWaveWindowResponse {
                device_id: "coyote-v3".to_owned(),
                sequence: 7,
                accepted: true,
            }
        );
    }

    #[test]
    fn submit_preview_window_request_logs_runtime_event() {
        let (runtime, core) = runtime_state_with_core();
        core.attach_output_device(DeviceId::new("coyote-v3"))
            .unwrap();

        let response =
            submit_preview_window_request(&core, &runtime, "coyote-v3".to_owned(), 3, 1).unwrap();

        assert_eq!(
            response,
            SubmitWaveWindowResponse {
                device_id: "coyote-v3".to_owned(),
                sequence: 0,
                accepted: true,
            }
        );
        let events = runtime.events.list();
        assert_eq!(events[0].kind, "wave.window.submitted");
        assert!(events[0].message.contains("A=3 B=1"));
    }

    #[test]
    fn preview_playback_status_defaults_to_stopped() {
        let state = PreviewPlaybackState::default();

        assert_eq!(
            preview_playback_status_from_state(&state),
            PreviewPlaybackStatus {
                running: false,
                device_id: None,
                channel_a_strength: 0,
                channel_b_strength: 0,
                interval_ms: COYOTE_V3_PREVIEW_INTERVAL_MS,
            }
        );
    }

    #[test]
    fn preview_playback_status_reports_running_session() {
        let state = PreviewPlaybackState::default();
        let (stop_sender, _stop_receiver) = oneshot::channel();
        let running = RunningPreviewPlayback {
            id: 3,
            device_id: DeviceId::new("coyote-v3"),
            channel_a_strength: 12,
            channel_b_strength: 4,
            stop_sender: Some(stop_sender),
        };
        state
            .inner
            .lock()
            .expect("preview playback state mutex poisoned")
            .running = Some(running);

        assert_eq!(
            preview_playback_status_from_state(&state),
            PreviewPlaybackStatus {
                running: true,
                device_id: Some("coyote-v3".to_owned()),
                channel_a_strength: 12,
                channel_b_strength: 4,
                interval_ms: COYOTE_V3_PREVIEW_INTERVAL_MS,
            }
        );
    }

    #[test]
    fn update_preview_playback_session_updates_running_strengths() {
        let (runtime, _core) = runtime_state_with_core();
        let state = PreviewPlaybackState::default();
        let (stop_sender, _stop_receiver) = oneshot::channel();
        state
            .inner
            .lock()
            .expect("preview playback state mutex poisoned")
            .running = Some(RunningPreviewPlayback {
            id: 5,
            device_id: DeviceId::new("coyote-v3"),
            channel_a_strength: 2,
            channel_b_strength: 0,
            stop_sender: Some(stop_sender),
        });

        let status = update_preview_playback_session(
            &runtime,
            &state,
            UpdatePreviewPlaybackParams {
                channel_a_strength: 9,
                channel_b_strength: 1,
            },
        )
        .unwrap();

        assert_eq!(status.channel_a_strength, 9);
        assert_eq!(status.channel_b_strength, 1);
        assert_eq!(runtime.events.list()[0].kind, "wave.preview.updated");
        let session = current_preview_session(&state.inner, 5).unwrap();
        assert_eq!(session.channel_a_strength(), 9);
        assert_eq!(session.channel_b_strength(), 1);
    }

    #[test]
    fn update_preview_playback_session_rejects_stopped_preview() {
        let (runtime, _core) = runtime_state_with_core();
        let state = PreviewPlaybackState::default();

        let error = update_preview_playback_session(
            &runtime,
            &state,
            UpdatePreviewPlaybackParams {
                channel_a_strength: 1,
                channel_b_strength: 0,
            },
        )
        .unwrap_err();

        assert!(error.contains("not running"));
    }

    #[test]
    fn cancel_preview_playback_clears_running_session_and_logs_event() {
        let state = PreviewPlaybackState::default();
        let events = RuntimeEventLog::default();
        let (stop_sender, _stop_receiver) = oneshot::channel();
        let running = RunningPreviewPlayback {
            id: 4,
            device_id: DeviceId::new("coyote-v3"),
            channel_a_strength: 10,
            channel_b_strength: 0,
            stop_sender: Some(stop_sender),
        };
        state
            .inner
            .lock()
            .expect("preview playback state mutex poisoned")
            .running = Some(running);

        cancel_preview_playback(&state, &events);

        assert!(!preview_playback_status_from_state(&state).running);
        assert_eq!(events.list()[0].kind, "wave.preview.stopped");
    }

    #[test]
    fn cancel_preview_playback_for_device_keeps_other_devices_running() {
        let state = PreviewPlaybackState::default();
        let events = RuntimeEventLog::default();
        let (stop_sender, _stop_receiver) = oneshot::channel();
        state
            .inner
            .lock()
            .expect("preview playback state mutex poisoned")
            .running = Some(RunningPreviewPlayback {
            id: 5,
            device_id: DeviceId::new("coyote-v3"),
            channel_a_strength: 10,
            channel_b_strength: 0,
            stop_sender: Some(stop_sender),
        });

        let stopped =
            cancel_preview_playback_for_device(&state, &events, &DeviceId::new("other-device"));

        assert!(!stopped);
        assert!(preview_playback_status_from_state(&state).running);
        assert!(events.list().is_empty());
    }

    #[test]
    fn stale_preview_loop_cannot_clear_new_running_session() {
        let state = PreviewPlaybackState::default();
        let (stop_sender, _stop_receiver) = oneshot::channel();
        let running = RunningPreviewPlayback {
            id: 8,
            device_id: DeviceId::new("new-session"),
            channel_a_strength: 10,
            channel_b_strength: 0,
            stop_sender: Some(stop_sender),
        };
        state
            .inner
            .lock()
            .expect("preview playback state mutex poisoned")
            .running = Some(running);

        let cleared = clear_preview_playback_if_current(&state.inner, 7);

        assert!(cleared.is_none());
        assert_eq!(
            preview_playback_status_from_state(&state).device_id,
            Some("new-session".to_owned())
        );
    }

    #[test]
    fn frontend_platform_reports_current_shell() {
        assert_eq!(
            frontend_platform(),
            FrontendPlatformResponse {
                platform: current_frontend_platform(),
            }
        );
    }

    #[test]
    fn external_control_policy_defaults_to_read_only() {
        assert_eq!(
            external_control_allowed_capabilities(false),
            vec![
                Capability::ExternalWebSocket,
                Capability::DeviceRead,
                Capability::EventsSubscribe,
            ]
        );
    }

    #[test]
    fn external_control_policy_can_enable_control_capabilities() {
        let capabilities = external_control_allowed_capabilities(true);

        assert!(capabilities.contains(&Capability::ExternalWebSocket));
        assert!(capabilities.contains(&Capability::DeviceRead));
        assert!(capabilities.contains(&Capability::EventsSubscribe));
        assert!(capabilities.contains(&Capability::WaveControl));
        assert!(capabilities.contains(&Capability::ScriptRun));
        assert!(capabilities.contains(&Capability::ScriptManage));
        assert!(capabilities.contains(&Capability::PluginManage));
    }

    #[test]
    fn logs_external_control_lifecycle_events() {
        let events = RuntimeEventLog::default();
        let started = ExternalControlStatus {
            running: true,
            bind_address: Some("127.0.0.1:49152".to_owned()),
            accepted_sessions: 0,
            active_sessions: 0,
            control_mode: true,
            allowed_capabilities: vec![Capability::ExternalWebSocket, Capability::PluginManage],
        };

        log_external_control_started(&events, &started);
        log_external_control_stopped(&events, &started);

        let records = events.list();
        assert_eq!(records[0].kind, "plugin.bridge.started");
        assert!(records[0]
            .message
            .contains("127.0.0.1:49152` in control mode"));
        assert_eq!(records[1].kind, "plugin.bridge.stopped");
        assert!(records[1].message.contains("127.0.0.1:49152"));
    }
}
