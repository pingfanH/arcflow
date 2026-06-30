//! JSON-RPC shaped WebSocket messages for external-control clients.

use arcflow_plugin_runtime::Capability;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Current external-control protocol version.
pub const PROTOCOL_VERSION: u16 = 1;

/// Initial hello message sent by an external-control client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientHello {
    /// Human-readable external client name.
    pub client_name: String,
    /// External-control protocol version requested by the client.
    pub protocol_version: u16,
    /// Capabilities requested by the client.
    pub requested_capabilities: Vec<Capability>,
}

/// JSON-RPC request id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    /// String request id.
    String(String),
    /// Numeric request id.
    Number(u64),
}

/// JSON-RPC request envelope used over WebSocket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    /// JSON-RPC version, always `2.0`.
    pub jsonrpc: String,
    /// Request id.
    pub id: RequestId,
    /// Method name, for example `device.status`.
    pub method: String,
    /// Method parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcRequest {
    /// Constructs a JSON-RPC 2.0 request.
    #[must_use]
    pub fn new(id: RequestId, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_owned(),
            id,
            method: method.into(),
            params,
        }
    }
}

/// JSON-RPC response envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    /// JSON-RPC version, always `2.0`.
    pub jsonrpc: String,
    /// Request id this response answers.
    pub id: RequestId,
    /// Successful result payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Error payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl JsonRpcResponse {
    /// Constructs a successful response.
    #[must_use]
    pub fn ok(id: RequestId, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_owned(),
            id,
            result: Some(result),
            error: None,
        }
    }

    /// Constructs an error response.
    #[must_use]
    pub fn error(id: RequestId, error: RpcError) -> Self {
        Self {
            jsonrpc: "2.0".to_owned(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// JSON-RPC error payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcError {
    /// Numeric error code.
    pub code: i32,
    /// Human-readable error message.
    pub message: String,
}

impl RpcError {
    /// Constructs an RPC error.
    #[must_use]
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Event envelope sent by ArcFlow to subscribed external clients.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerEvent {
    /// Method name, always `event`.
    pub method: String,
    /// Event payload.
    pub params: Value,
}

impl ServerEvent {
    /// Constructs an `event` envelope.
    #[must_use]
    pub fn new(params: Value) -> Self {
        Self {
            method: "event".to_owned(),
            params,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn serializes_client_hello_capabilities() {
        let hello = ClientHello {
            client_name: "OBS ArcFlow Bridge".to_owned(),
            protocol_version: 1,
            requested_capabilities: vec![Capability::DeviceRead, Capability::WaveControl],
        };

        let json = serde_json::to_value(hello).unwrap();

        assert_eq!(
            json,
            json!({
                "clientName": "OBS ArcFlow Bridge",
                "protocolVersion": 1,
                "requestedCapabilities": ["device.read", "wave.control"]
            })
        );
    }

    #[test]
    fn builds_json_rpc_request() {
        let request = JsonRpcRequest::new(
            RequestId::String("1".to_owned()),
            "device.status",
            Some(json!({"deviceId": "coyote-1"})),
        );

        assert_eq!(
            serde_json::to_value(request).unwrap(),
            json!({
                "jsonrpc": "2.0",
                "id": "1",
                "method": "device.status",
                "params": {"deviceId": "coyote-1"}
            })
        );
    }

    #[test]
    fn builds_event_envelope() {
        let event = ServerEvent::new(json!({
            "type": "wave.started",
            "payload": {"sessionId": "s1"}
        }));

        assert_eq!(
            serde_json::to_value(event).unwrap(),
            json!({
                "method": "event",
                "params": {
                    "type": "wave.started",
                    "payload": {"sessionId": "s1"}
                }
            })
        );
    }
}
