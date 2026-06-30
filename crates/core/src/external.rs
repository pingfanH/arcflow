//! External-control request routing into Rust Core commands.

use arcflow_external_control::JsonRpcRequest;
use serde::Deserialize;

use crate::{CoreCommand, CoreError, DeviceId};

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
    use super::*;
    use arcflow_external_control::RequestId;
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
    fn rejects_unknown_method() {
        let request = JsonRpcRequest::new(RequestId::Number(3), "device.rawWrite", None);

        let error = core_command_from_external_request(&request).unwrap_err();

        assert!(matches!(error, CoreError::InvalidExternalRequest(_)));
    }
}
