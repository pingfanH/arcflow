//! Minimal WebSocket gateway for external-control clients.

use core::fmt;

use arcflow_plugin_runtime::Capability;
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{accept_async, tungstenite::Message};

use crate::{ClientHello, ClientSession, GatewayPolicy, SessionError, PROTOCOL_VERSION};

/// Accepted session response sent to a WebSocket client after hello.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptedSession {
    /// External-control protocol version accepted by ArcFlow.
    pub protocol_version: u16,
    /// Accepted client name.
    pub client_name: String,
    /// Capabilities granted to this session.
    pub granted_capabilities: Vec<Capability>,
}

impl AcceptedSession {
    fn from_session(session: &ClientSession) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            client_name: session.client_name().to_owned(),
            granted_capabilities: session.granted_capabilities().to_vec(),
        }
    }
}

/// WebSocket gateway that accepts external-control clients.
#[derive(Debug, Clone)]
pub struct WsGateway {
    policy: GatewayPolicy,
}

impl WsGateway {
    /// Constructs a gateway from an authorization policy.
    #[must_use]
    pub fn new(policy: GatewayPolicy) -> Self {
        Self { policy }
    }

    /// Returns the gateway policy.
    #[must_use]
    pub fn policy(&self) -> &GatewayPolicy {
        &self.policy
    }

    /// Accepts one TCP client from a listener.
    pub async fn accept_next(
        &self,
        listener: &TcpListener,
    ) -> Result<ClientSession, WsGatewayError> {
        let (stream, _) = listener.accept().await.map_err(WsGatewayError::Io)?;
        self.accept_stream(stream).await
    }

    /// Accepts one TCP stream as a WebSocket external-control session.
    pub async fn accept_stream(&self, stream: TcpStream) -> Result<ClientSession, WsGatewayError> {
        let mut socket = accept_async(stream)
            .await
            .map_err(|error| WsGatewayError::WebSocket(error.to_string()))?;

        let message = socket
            .next()
            .await
            .ok_or(WsGatewayError::MissingHello)?
            .map_err(|error| WsGatewayError::WebSocket(error.to_string()))?;

        let text = match message {
            Message::Text(text) => text,
            _ => return Err(WsGatewayError::ExpectedTextHello),
        };

        let hello: ClientHello = serde_json::from_str(&text).map_err(WsGatewayError::Json)?;
        let session = self.policy.accept(hello).map_err(WsGatewayError::Session)?;
        let accepted = serde_json::to_string(&AcceptedSession::from_session(&session))
            .map_err(WsGatewayError::Json)?;

        socket
            .send(Message::Text(accepted.into()))
            .await
            .map_err(|error| WsGatewayError::WebSocket(error.to_string()))?;

        Ok(session)
    }
}

/// Error returned by the WebSocket gateway.
#[derive(Debug)]
pub enum WsGatewayError {
    /// TCP IO failed.
    Io(std::io::Error),
    /// WebSocket handshake or message failed.
    WebSocket(String),
    /// Client did not send an initial hello.
    MissingHello,
    /// Initial hello was not a text frame.
    ExpectedTextHello,
    /// JSON parsing or serialization failed.
    Json(serde_json::Error),
    /// Session authorization failed.
    Session(SessionError),
}

impl fmt::Display for WsGatewayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "websocket gateway IO error: {error}"),
            Self::WebSocket(error) => write!(f, "websocket error: {error}"),
            Self::MissingHello => write!(f, "websocket client did not send hello"),
            Self::ExpectedTextHello => write!(f, "websocket hello must be a text frame"),
            Self::Json(error) => write!(f, "websocket JSON error: {error}"),
            Self::Session(error) => write!(f, "websocket session error: {error}"),
        }
    }
}

impl std::error::Error for WsGatewayError {}

#[cfg(test)]
mod tests {
    use arcflow_plugin_runtime::Capability;
    use futures_util::{SinkExt, StreamExt};
    use tokio::net::TcpListener;
    use tokio_tungstenite::{connect_async, tungstenite::Message};

    use super::*;

    #[tokio::test]
    async fn accepts_client_hello_over_websocket() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let gateway = WsGateway::new(GatewayPolicy::local_default());

        let server = tokio::spawn(async move { gateway.accept_next(&listener).await.unwrap() });

        let (mut client, _) = connect_async(format!("ws://{address}")).await.unwrap();
        let hello = ClientHello {
            client_name: "OBS ArcFlow Bridge".to_owned(),
            protocol_version: PROTOCOL_VERSION,
            requested_capabilities: vec![Capability::DeviceRead],
        };
        client
            .send(Message::Text(serde_json::to_string(&hello).unwrap().into()))
            .await
            .unwrap();

        let response = client.next().await.unwrap().unwrap().into_text().unwrap();
        let session = server.await.unwrap();

        assert!(response.contains("OBS ArcFlow Bridge"));
        assert_eq!(session.client_name(), "OBS ArcFlow Bridge");
        assert!(session.has_capability(Capability::DeviceRead));
    }
}
