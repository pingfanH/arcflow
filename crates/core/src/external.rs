//! External-control request routing into Rust Core commands.

use arcflow_external_control::{ClientSession, JsonRpcRequest};
use arcflow_plugin_runtime::Capability;
use arcflow_protocol::coyote::v3::{StrengthMode, StrengthModes};
use arcflow_storage::Storage;
use arcflow_wave::{ChannelOutput, ChannelWindow, CoyoteV3Window, WavePoint};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    CoreCommand, CoreError, DeviceId, PluginBundle, PluginRegistryPersistence,
    ScriptDocumentPersistence,
};

const DEVICE_ACTIVATE_OUTPUT_METHOD: &str = "device.activateOutput";
const DEVICE_DEACTIVATE_OUTPUT_METHOD: &str = "device.deactivateOutput";
const PLUGIN_REGISTRY_METHOD: &str = "plugin.registry";
const PLUGIN_INSTALL_MANIFEST_METHOD: &str = "plugin.installManifest";
const PLUGIN_INSTALL_BUNDLE_METHOD: &str = "plugin.installBundle";
const PLUGIN_SET_ENABLED_METHOD: &str = "plugin.setEnabled";
const PLUGIN_DELETE_METHOD: &str = "plugin.delete";
const SCRIPT_LIST_METHOD: &str = "script.list";
const SCRIPT_UPSERT_METHOD: &str = "script.upsert";
const SCRIPT_DELETE_METHOD: &str = "script.delete";

/// Converts an authorized external-control JSON-RPC request into a core command.
pub fn authorized_core_command_from_external_request(
    session: &ClientSession,
    request: &JsonRpcRequest,
) -> Result<CoreCommand, CoreError> {
    let required_capability = required_capability_for_external_request(request)?;

    if !session.has_capability(required_capability) {
        return Err(CoreError::InvalidExternalRequest(format!(
            "session `{}` is missing required capability `{}` for `{}`",
            session.client_name(),
            required_capability.as_str(),
            request.method
        )));
    }

    core_command_from_external_request(request)
}

/// Returns the capability required by one external-control JSON-RPC method.
pub fn required_capability_for_external_request(
    request: &JsonRpcRequest,
) -> Result<Capability, CoreError> {
    match request.method.as_str() {
        "device.status" => Ok(Capability::DeviceRead),
        DEVICE_ACTIVATE_OUTPUT_METHOD | DEVICE_DEACTIVATE_OUTPUT_METHOD => {
            Ok(Capability::WaveControl)
        }
        "wave.submitWindow" => Ok(Capability::WaveControl),
        "wave.stop" => Ok(Capability::WaveControl),
        "script.run" => Ok(Capability::ScriptRun),
        method => Err(CoreError::InvalidExternalRequest(format!(
            "unsupported method `{method}`"
        ))),
    }
}

/// Converts an external-control JSON-RPC request into a core command.
pub fn core_command_from_external_request(
    request: &JsonRpcRequest,
) -> Result<CoreCommand, CoreError> {
    match request.method.as_str() {
        "device.status" => {
            let params: DeviceParams = read_params(request)?;
            Ok(CoreCommand::ReadDeviceStatus {
                device_id: DeviceId::new(params.device_id),
            })
        }
        DEVICE_ACTIVATE_OUTPUT_METHOD => {
            let params: DeviceParams = read_params(request)?;
            Ok(CoreCommand::ActivateOutputDevice {
                device_id: DeviceId::new(params.device_id),
            })
        }
        DEVICE_DEACTIVATE_OUTPUT_METHOD => {
            let params: DeviceParams = read_params(request)?;
            Ok(CoreCommand::DeactivateOutputDevice {
                device_id: DeviceId::new(params.device_id),
            })
        }
        "wave.submitWindow" => {
            let params: SubmitWindowParams = read_params(request)?;
            submit_window_command(params)
        }
        "wave.stop" => {
            let params: DeviceParams = read_params(request)?;
            Ok(CoreCommand::StopOutput {
                device_id: DeviceId::new(params.device_id),
            })
        }
        "script.run" => {
            let params: ScriptRunParams = read_params(request)?;
            Ok(CoreCommand::RunScript {
                script_id: params.script_id,
            })
        }
        method => Err(CoreError::InvalidExternalRequest(format!(
            "unsupported method `{method}`"
        ))),
    }
}

/// Returns whether a request is handled by the storage-backed plugin registry surface.
#[must_use]
pub fn is_plugin_registry_external_request(request: &JsonRpcRequest) -> bool {
    matches!(
        request.method.as_str(),
        PLUGIN_REGISTRY_METHOD
            | PLUGIN_INSTALL_MANIFEST_METHOD
            | PLUGIN_INSTALL_BUNDLE_METHOD
            | PLUGIN_SET_ENABLED_METHOD
            | PLUGIN_DELETE_METHOD
    )
}

/// Returns whether a plugin registry request mutates persisted plugin state.
#[must_use]
pub fn is_plugin_registry_mutation_external_request(request: &JsonRpcRequest) -> bool {
    matches!(
        request.method.as_str(),
        PLUGIN_INSTALL_MANIFEST_METHOD
            | PLUGIN_INSTALL_BUNDLE_METHOD
            | PLUGIN_SET_ENABLED_METHOD
            | PLUGIN_DELETE_METHOD
    )
}

/// Returns whether a request is handled by the storage-backed script document surface.
#[must_use]
pub fn is_script_documents_external_request(request: &JsonRpcRequest) -> bool {
    matches!(
        request.method.as_str(),
        SCRIPT_LIST_METHOD | SCRIPT_UPSERT_METHOD | SCRIPT_DELETE_METHOD
    )
}

/// Executes one authorized storage-backed plugin registry external-control request.
pub fn execute_plugin_registry_external_request(
    session: &ClientSession,
    request: &JsonRpcRequest,
    storage: &Storage,
) -> Result<Value, CoreError> {
    authorize_plugin_registry_external_request(session, request)?;
    let persistence = PluginRegistryPersistence::new(storage);
    let entries = match request.method.as_str() {
        PLUGIN_REGISTRY_METHOD => persistence.list_entries(),
        PLUGIN_INSTALL_MANIFEST_METHOD => {
            let params: PluginInstallManifestParams = read_params(request)?;
            persistence.install_manifest_json(&params.manifest_json)
        }
        PLUGIN_INSTALL_BUNDLE_METHOD => {
            let params: PluginInstallBundleParams = read_params(request)?;
            let bundle = PluginBundle::load(&params.bundle_path)
                .map_err(|error| CoreError::InvalidExternalRequest(error.to_string()))?;
            let manifest_json = serde_json::to_string(bundle.manifest()).map_err(|error| {
                CoreError::InvalidExternalRequest(format!(
                    "failed to serialize bundle manifest: {error}"
                ))
            })?;

            persistence.install_manifest_json_with_bundle_root(
                &manifest_json,
                Some(bundle.root().display().to_string()),
            )
        }
        PLUGIN_SET_ENABLED_METHOD => {
            let params: PluginSetEnabledParams = read_params(request)?;
            persistence.set_enabled(&params.plugin_id, params.enabled)
        }
        PLUGIN_DELETE_METHOD => {
            let params: PluginDeleteParams = read_params(request)?;
            persistence.delete(&params.plugin_id)
        }
        method => {
            return Err(CoreError::InvalidExternalRequest(format!(
                "unsupported plugin registry method `{method}`"
            )));
        }
    }
    .map_err(|error| CoreError::InvalidExternalRequest(error.to_string()))?;

    Ok(json!({ "plugins": entries }))
}

/// Executes one authorized storage-backed script document external-control request.
pub fn execute_script_documents_external_request(
    session: &ClientSession,
    request: &JsonRpcRequest,
    storage: &Storage,
) -> Result<Value, CoreError> {
    authorize_script_documents_external_request(session, request)?;
    let persistence = ScriptDocumentPersistence::new(storage);
    let scripts = match request.method.as_str() {
        SCRIPT_LIST_METHOD => persistence.list(),
        SCRIPT_UPSERT_METHOD => {
            let params: ScriptUpsertParams = read_params(request)?;
            persistence.upsert(&params.script_id, &params.document_json)
        }
        SCRIPT_DELETE_METHOD => {
            let params: ScriptDeleteParams = read_params(request)?;
            persistence.delete(&params.script_id)
        }
        method => {
            return Err(CoreError::InvalidExternalRequest(format!(
                "unsupported script document method `{method}`"
            )));
        }
    }
    .map_err(|error| CoreError::InvalidExternalRequest(error.to_string()))?;

    Ok(json!({ "scripts": scripts }))
}

fn authorize_plugin_registry_external_request(
    session: &ClientSession,
    request: &JsonRpcRequest,
) -> Result<(), CoreError> {
    if !session.has_capability(Capability::PluginManage) {
        return Err(CoreError::InvalidExternalRequest(format!(
            "session `{}` is missing required capability `{}` for `{}`",
            session.client_name(),
            Capability::PluginManage.as_str(),
            request.method
        )));
    }

    Ok(())
}

fn authorize_script_documents_external_request(
    session: &ClientSession,
    request: &JsonRpcRequest,
) -> Result<(), CoreError> {
    if !session.has_capability(Capability::ScriptManage) {
        return Err(CoreError::InvalidExternalRequest(format!(
            "session `{}` is missing required capability `{}` for `{}`",
            session.client_name(),
            Capability::ScriptManage.as_str(),
            request.method
        )));
    }

    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceParams {
    device_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScriptRunParams {
    script_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScriptUpsertParams {
    script_id: String,
    document_json: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScriptDeleteParams {
    script_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubmitWindowParams {
    device_id: String,
    sequence: u8,
    #[serde(default)]
    strength_modes: StrengthModesParams,
    #[serde(default)]
    a_strength: u8,
    #[serde(default)]
    b_strength: u8,
    #[serde(default)]
    channel_a: Option<[WavePointParams; 4]>,
    #[serde(default)]
    channel_b: Option<[WavePointParams; 4]>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StrengthModesParams {
    #[serde(default)]
    a: StrengthModeParam,
    #[serde(default)]
    b: StrengthModeParam,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
enum StrengthModeParam {
    #[default]
    Unchanged,
    Increase,
    Decrease,
    Absolute,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WavePointParams {
    period_ms: u16,
    strength: u8,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginInstallManifestParams {
    manifest_json: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginInstallBundleParams {
    bundle_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginSetEnabledParams {
    plugin_id: String,
    enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginDeleteParams {
    plugin_id: String,
}

fn submit_window_command(params: SubmitWindowParams) -> Result<CoreCommand, CoreError> {
    Ok(CoreCommand::SubmitCoyoteV3Window {
        device_id: DeviceId::new(params.device_id),
        window: CoyoteV3Window::new(
            channel_output(params.channel_a)?,
            channel_output(params.channel_b)?,
        ),
        sequence: params.sequence,
        strength_modes: StrengthModes::new(
            strength_mode(params.strength_modes.a),
            strength_mode(params.strength_modes.b),
        ),
        a_strength: params.a_strength,
        b_strength: params.b_strength,
    })
}

fn channel_output(points: Option<[WavePointParams; 4]>) -> Result<ChannelOutput, CoreError> {
    let Some(points) = points else {
        return Ok(ChannelOutput::Disabled);
    };

    Ok(ChannelOutput::Window(ChannelWindow::new([
        wave_point(points[0])?,
        wave_point(points[1])?,
        wave_point(points[2])?,
        wave_point(points[3])?,
    ])))
}

fn wave_point(point: WavePointParams) -> Result<WavePoint, CoreError> {
    Ok(WavePoint::new(point.period_ms, point.strength)?)
}

fn strength_mode(mode: StrengthModeParam) -> StrengthMode {
    match mode {
        StrengthModeParam::Unchanged => StrengthMode::Unchanged,
        StrengthModeParam::Increase => StrengthMode::Increase,
        StrengthModeParam::Decrease => StrengthMode::Decrease,
        StrengthModeParam::Absolute => StrengthMode::Absolute,
    }
}

fn read_params<T>(request: &JsonRpcRequest) -> Result<T, CoreError>
where
    T: for<'de> Deserialize<'de>,
{
    let params = request.params.clone().ok_or_else(|| {
        CoreError::InvalidExternalRequest(format!("missing params for `{}`", request.method))
    })?;

    serde_json::from_value(params).map_err(|error| {
        CoreError::InvalidExternalRequest(format!(
            "invalid params for `{}`: {error}",
            request.method
        ))
    })
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use arcflow_external_control::{ClientHello, GatewayPolicy, RequestId, PROTOCOL_VERSION};
    use arcflow_storage::Storage;
    use serde_json::json;

    #[test]
    fn routes_device_status_request() {
        let request = JsonRpcRequest::new(
            RequestId::Number(1),
            "device.status",
            Some(json!({ "deviceId": "coyote-v3" })),
        );

        let command = core_command_from_external_request(&request).unwrap();

        assert_eq!(
            command,
            CoreCommand::ReadDeviceStatus {
                device_id: DeviceId::new("coyote-v3")
            }
        );
    }

    #[test]
    fn routes_output_activation_requests() {
        let activate = JsonRpcRequest::new(
            RequestId::Number(2),
            DEVICE_ACTIVATE_OUTPUT_METHOD,
            Some(json!({ "deviceId": "coyote-v3" })),
        );
        let deactivate = JsonRpcRequest::new(
            RequestId::Number(3),
            DEVICE_DEACTIVATE_OUTPUT_METHOD,
            Some(json!({ "deviceId": "coyote-v3" })),
        );

        assert_eq!(
            core_command_from_external_request(&activate).unwrap(),
            CoreCommand::ActivateOutputDevice {
                device_id: DeviceId::new("coyote-v3")
            }
        );
        assert_eq!(
            core_command_from_external_request(&deactivate).unwrap(),
            CoreCommand::DeactivateOutputDevice {
                device_id: DeviceId::new("coyote-v3")
            }
        );
        assert_eq!(
            required_capability_for_external_request(&activate).unwrap(),
            Capability::WaveControl
        );
    }

    #[test]
    fn routes_wave_stop_request() {
        let request = JsonRpcRequest::new(
            RequestId::Number(2),
            "wave.stop",
            Some(json!({ "deviceId": "coyote-v3" })),
        );

        let command = core_command_from_external_request(&request).unwrap();

        assert_eq!(
            command,
            CoreCommand::StopOutput {
                device_id: DeviceId::new("coyote-v3")
            }
        );
    }

    #[test]
    fn routes_wave_submit_window_request() {
        let request = JsonRpcRequest::new(
            RequestId::Number(10),
            "wave.submitWindow",
            Some(json!({
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
            })),
        );

        let command = core_command_from_external_request(&request).unwrap();

        assert_eq!(
            command,
            CoreCommand::SubmitCoyoteV3Window {
                device_id: DeviceId::new("coyote-v3"),
                window: safe_window(),
                sequence: 4,
                strength_modes: StrengthModes::new(StrengthMode::Absolute, StrengthMode::Unchanged),
                a_strength: 8,
                b_strength: 0,
            }
        );
    }

    #[test]
    fn rejects_unknown_method() {
        let request = JsonRpcRequest::new(RequestId::Number(3), "device.rawWrite", None);

        let error = core_command_from_external_request(&request).unwrap_err();

        assert!(matches!(error, CoreError::InvalidExternalRequest(_)));
    }

    #[test]
    fn authorizes_request_with_granted_capability() {
        let session = GatewayPolicy::local_default()
            .accept(ClientHello {
                client_name: "Status Panel".to_owned(),
                protocol_version: PROTOCOL_VERSION,
                requested_capabilities: vec![Capability::DeviceRead],
            })
            .unwrap();
        let request = JsonRpcRequest::new(
            RequestId::Number(4),
            "device.status",
            Some(json!({ "deviceId": "coyote-v3" })),
        );

        let command = authorized_core_command_from_external_request(&session, &request).unwrap();

        assert_eq!(
            command,
            CoreCommand::ReadDeviceStatus {
                device_id: DeviceId::new("coyote-v3")
            }
        );
    }

    #[test]
    fn rejects_request_without_granted_capability() {
        let session = GatewayPolicy::local_default()
            .accept(ClientHello {
                client_name: "Status Panel".to_owned(),
                protocol_version: PROTOCOL_VERSION,
                requested_capabilities: vec![Capability::DeviceRead],
            })
            .unwrap();
        let request = JsonRpcRequest::new(
            RequestId::Number(5),
            "wave.stop",
            Some(json!({ "deviceId": "coyote-v3" })),
        );

        let error = authorized_core_command_from_external_request(&session, &request).unwrap_err();

        assert!(matches!(error, CoreError::InvalidExternalRequest(_)));
    }

    #[test]
    fn detects_plugin_registry_external_requests() {
        let request = JsonRpcRequest::new(RequestId::Number(6), PLUGIN_REGISTRY_METHOD, None);

        assert!(is_plugin_registry_external_request(&request));
        assert!(!is_plugin_registry_mutation_external_request(&request));
    }

    #[test]
    fn detects_plugin_registry_mutation_requests() {
        let request = JsonRpcRequest::new(
            RequestId::Number(6),
            PLUGIN_INSTALL_BUNDLE_METHOD,
            Some(json!({ "bundlePath": "/plugins/plugin.external" })),
        );

        assert!(is_plugin_registry_external_request(&request));
        assert!(is_plugin_registry_mutation_external_request(&request));
    }

    #[test]
    fn deletes_plugin_manifest_through_external_request() {
        let storage = Storage::in_memory().unwrap();
        let session = GatewayPolicy::new(vec![Capability::PluginManage])
            .accept(ClientHello {
                client_name: "Plugin Manager".to_owned(),
                protocol_version: PROTOCOL_VERSION,
                requested_capabilities: vec![Capability::PluginManage],
            })
            .unwrap();
        let install = JsonRpcRequest::new(
            RequestId::Number(8),
            PLUGIN_INSTALL_MANIFEST_METHOD,
            Some(json!({
                "manifestJson": r#"{
                    "id":"plugin.external",
                    "name":"External Plugin",
                    "version":"1.0.0",
                    "runtime":"wasm",
                    "entry":"dist/plugin.wasm",
                    "apiVersion":"1",
                    "capabilities":["device.read"]
                }"#
            })),
        );
        execute_plugin_registry_external_request(&session, &install, &storage).unwrap();
        let delete = JsonRpcRequest::new(
            RequestId::Number(9),
            PLUGIN_DELETE_METHOD,
            Some(json!({ "pluginId": "plugin.external" })),
        );

        let result = execute_plugin_registry_external_request(&session, &delete, &storage).unwrap();

        assert_eq!(result, json!({ "plugins": [] }));
        assert!(storage
            .plugin_registry()
            .get("plugin.external")
            .unwrap()
            .is_none());
    }

    #[test]
    fn detects_script_document_external_requests() {
        let request = JsonRpcRequest::new(RequestId::Number(7), SCRIPT_LIST_METHOD, None);

        assert!(is_script_documents_external_request(&request));
    }

    #[test]
    fn executes_plugin_registry_request_with_granted_capability() {
        let storage = Storage::in_memory().unwrap();
        let session = GatewayPolicy::new(vec![Capability::PluginManage])
            .accept(ClientHello {
                client_name: "Plugin Manager".to_owned(),
                protocol_version: PROTOCOL_VERSION,
                requested_capabilities: vec![Capability::PluginManage],
            })
            .unwrap();
        let request = JsonRpcRequest::new(RequestId::Number(7), PLUGIN_REGISTRY_METHOD, None);

        let result =
            execute_plugin_registry_external_request(&session, &request, &storage).unwrap();

        assert_eq!(result, json!({ "plugins": [] }));
    }

    #[test]
    fn installs_plugin_manifest_through_external_request() {
        let storage = Storage::in_memory().unwrap();
        let session = GatewayPolicy::new(vec![Capability::PluginManage])
            .accept(ClientHello {
                client_name: "Plugin Manager".to_owned(),
                protocol_version: PROTOCOL_VERSION,
                requested_capabilities: vec![Capability::PluginManage],
            })
            .unwrap();
        let request = JsonRpcRequest::new(
            RequestId::Number(8),
            PLUGIN_INSTALL_MANIFEST_METHOD,
            Some(json!({
                "manifestJson": r#"{
                    "id":"plugin.external",
                    "name":"External Plugin",
                    "version":"1.0.0",
                    "runtime":"wasm",
                    "entry":"dist/plugin.wasm",
                    "apiVersion":"1",
                    "capabilities":["device.read"]
                }"#
            })),
        );

        let result =
            execute_plugin_registry_external_request(&session, &request, &storage).unwrap();

        assert_eq!(result["plugins"][0]["id"], "plugin.external");
        assert!(storage
            .plugin_registry()
            .get("plugin.external")
            .unwrap()
            .is_some());
    }

    #[test]
    fn installs_plugin_bundle_through_external_request() {
        let storage = Storage::in_memory().unwrap();
        let session = GatewayPolicy::new(vec![Capability::PluginManage])
            .accept(ClientHello {
                client_name: "Plugin Manager".to_owned(),
                protocol_version: PROTOCOL_VERSION,
                requested_capabilities: vec![Capability::PluginManage],
            })
            .unwrap();
        let bundle_root = test_bundle_root("external-install");
        write_test_bundle(&bundle_root, "plugin.bundle");
        let request = JsonRpcRequest::new(
            RequestId::Number(9),
            PLUGIN_INSTALL_BUNDLE_METHOD,
            Some(json!({ "bundlePath": bundle_root.display().to_string() })),
        );

        let result =
            execute_plugin_registry_external_request(&session, &request, &storage).unwrap();

        assert_eq!(result["plugins"][0]["id"], "plugin.bundle");
        assert_eq!(
            result["plugins"][0]["bundleRoot"],
            bundle_root.display().to_string()
        );
        assert_eq!(
            storage
                .plugin_registry()
                .get("plugin.bundle")
                .unwrap()
                .unwrap()
                .bundle_root,
            Some(bundle_root.display().to_string())
        );

        fs::remove_dir_all(bundle_root).unwrap();
    }

    #[test]
    fn rejects_plugin_registry_request_without_manage_capability() {
        let storage = Storage::in_memory().unwrap();
        let session = GatewayPolicy::local_default()
            .accept(ClientHello {
                client_name: "Read Only".to_owned(),
                protocol_version: PROTOCOL_VERSION,
                requested_capabilities: vec![Capability::DeviceRead],
            })
            .unwrap();
        let request = JsonRpcRequest::new(RequestId::Number(9), PLUGIN_REGISTRY_METHOD, None);

        let error =
            execute_plugin_registry_external_request(&session, &request, &storage).unwrap_err();

        assert!(matches!(error, CoreError::InvalidExternalRequest(_)));
    }

    #[test]
    fn lists_scripts_through_external_request() {
        let storage = Storage::in_memory().unwrap();
        let session = script_manager_session();
        let request = JsonRpcRequest::new(RequestId::Number(10), SCRIPT_LIST_METHOD, None);

        let result =
            execute_script_documents_external_request(&session, &request, &storage).unwrap();

        assert_eq!(result, json!({ "scripts": [] }));
    }

    #[test]
    fn upserts_script_through_external_request() {
        let storage = Storage::in_memory().unwrap();
        let session = script_manager_session();
        let request = JsonRpcRequest::new(
            RequestId::Number(11),
            SCRIPT_UPSERT_METHOD,
            Some(json!({
                "scriptId": "script.external",
                "documentJson": r#"{"id":"script.external","version":1,"steps":[{"type":"wait","durationMs":1}]}"#
            })),
        );

        let result =
            execute_script_documents_external_request(&session, &request, &storage).unwrap();

        assert_eq!(result["scripts"][0]["scriptId"], "script.external");
        assert!(storage.scripts().get("script.external").unwrap().is_some());
    }

    #[test]
    fn deletes_script_through_external_request() {
        let storage = Storage::in_memory().unwrap();
        let session = script_manager_session();
        execute_script_documents_external_request(
            &session,
            &JsonRpcRequest::new(
                RequestId::Number(12),
                SCRIPT_UPSERT_METHOD,
                Some(json!({
                    "scriptId": "script.external",
                    "documentJson": r#"{"id":"script.external","version":1,"steps":[{"type":"wait","durationMs":1}]}"#
                })),
            ),
            &storage,
        )
        .unwrap();
        let request = JsonRpcRequest::new(
            RequestId::Number(13),
            SCRIPT_DELETE_METHOD,
            Some(json!({ "scriptId": "script.external" })),
        );

        let result =
            execute_script_documents_external_request(&session, &request, &storage).unwrap();

        assert_eq!(result, json!({ "scripts": [] }));
        assert_eq!(storage.scripts().get("script.external").unwrap(), None);
    }

    #[test]
    fn rejects_script_document_request_without_manage_capability() {
        let storage = Storage::in_memory().unwrap();
        let session = GatewayPolicy::local_default()
            .accept(ClientHello {
                client_name: "Read Only".to_owned(),
                protocol_version: PROTOCOL_VERSION,
                requested_capabilities: vec![Capability::DeviceRead],
            })
            .unwrap();
        let request = JsonRpcRequest::new(RequestId::Number(14), SCRIPT_LIST_METHOD, None);

        let error =
            execute_script_documents_external_request(&session, &request, &storage).unwrap_err();

        assert!(matches!(error, CoreError::InvalidExternalRequest(_)));
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

    fn script_manager_session() -> ClientSession {
        GatewayPolicy::new(vec![Capability::ScriptManage])
            .accept(ClientHello {
                client_name: "Script Manager".to_owned(),
                protocol_version: PROTOCOL_VERSION,
                requested_capabilities: vec![Capability::ScriptManage],
            })
            .unwrap()
    }

    fn write_test_bundle(root: &PathBuf, plugin_id: &str) {
        fs::create_dir_all(root.join("dist")).unwrap();
        fs::write(root.join("dist/plugin.wasm"), b"").unwrap();
        fs::write(
            root.join("manifest.json"),
            format!(
                r#"{{
                    "id":"{plugin_id}",
                    "name":"External Plugin",
                    "version":"1.0.0",
                    "runtime":"wasm",
                    "entry":"dist/plugin.wasm",
                    "apiVersion":"1",
                    "capabilities":["device.read"]
                }}"#
            ),
        )
        .unwrap();
    }

    fn test_bundle_root(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("arcflow-external-bundle-{name}-{suffix}"))
    }
}
