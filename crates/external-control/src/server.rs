//! Minimal WebSocket gateway for external-control clients.

use core::fmt;
use std::{
    future::Future,
    net::SocketAddr,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use arcflow_plugin_runtime::Capability;
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::watch,
    task::JoinHandle,
};
use tokio_tungstenite::{accept_async, tungstenite::Message, WebSocketStream};

use crate::{
    ClientHello, ClientSession, GatewayPolicy, JsonRpcRequest, JsonRpcResponse, RpcError,
    SessionError, PROTOCOL_VERSION,
};

/// Future returned by a WebSocket JSON-RPC request handler.
pub type WsRequestFuture = Pin<Box<dyn Future<Output = JsonRpcResponse> + Send + 'static>>;

/// Function used by the WebSocket service to route one JSON-RPC request.
pub type WsRequestHandler =
    Arc<dyn Fn(ClientSession, JsonRpcRequest) -> WsRequestFuture + Send + Sync + 'static>;

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
        let client = self.accept_client(stream).await?;

        Ok(client.session)
    }

    /// Accepts one TCP stream and keeps the WebSocket open for JSON-RPC traffic.
    pub async fn accept_client(&self, stream: TcpStream) -> Result<AcceptedClient, WsGatewayError> {
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

        Ok(AcceptedClient { session, socket })
    }
}

/// Accepted WebSocket client after a successful external-control hello.
pub struct AcceptedClient {
    session: ClientSession,
    socket: WebSocketStream<TcpStream>,
}

impl AcceptedClient {
    /// Returns the authorized external-control session.
    #[must_use]
    pub fn session(&self) -> &ClientSession {
        &self.session
    }

    /// Splits the accepted client into its session and WebSocket stream.
    #[must_use]
    pub fn into_parts(self) -> (ClientSession, WebSocketStream<TcpStream>) {
        (self.session, self.socket)
    }
}

/// Runtime status for a WebSocket gateway service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WsGatewayRuntimeStatus {
    /// Whether the listener is currently managed by a running handle.
    pub running: bool,
    /// Bound listener address.
    pub bind_address: String,
    /// Number of sessions accepted since the listener started.
    pub accepted_sessions: usize,
    /// Number of sessions with an open WebSocket.
    pub active_sessions: usize,
}

/// Long-running WebSocket gateway service.
#[derive(Debug, Clone)]
pub struct WsGatewayService {
    bind_address: String,
    gateway: WsGateway,
}

impl WsGatewayService {
    /// Constructs a gateway service.
    #[must_use]
    pub fn new(bind_address: impl Into<String>, policy: GatewayPolicy) -> Self {
        Self {
            bind_address: bind_address.into(),
            gateway: WsGateway::new(policy),
        }
    }

    /// Starts listening and returns a handle that can query or stop the service.
    pub async fn start(
        self,
        request_handler: WsRequestHandler,
    ) -> Result<WsGatewayHandle, WsGatewayError> {
        let listener = TcpListener::bind(&self.bind_address)
            .await
            .map_err(WsGatewayError::Io)?;
        let bind_address = listener.local_addr().map_err(WsGatewayError::Io)?;
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let accepted_sessions = Arc::new(AtomicUsize::new(0));
        let active_sessions = Arc::new(AtomicUsize::new(0));
        let accepted_for_task = Arc::clone(&accepted_sessions);
        let active_for_task = Arc::clone(&active_sessions);
        let gateway = self.gateway;

        let shutdown_for_task = shutdown_tx.clone();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    result = listener.accept() => {
                        let Ok((stream, _)) = result else {
                            continue;
                        };
                        let gateway = gateway.clone();
                        let handler = Arc::clone(&request_handler);
                        let accepted_sessions = Arc::clone(&accepted_for_task);
                        let active_sessions = Arc::clone(&active_for_task);
                        let shutdown_rx = shutdown_for_task.subscribe();

                        tokio::spawn(async move {
                            serve_client(
                                gateway,
                                stream,
                                handler,
                                accepted_sessions,
                                active_sessions,
                                shutdown_rx,
                            )
                            .await;
                        });
                    }
                    changed = shutdown_rx.changed() => {
                        if changed.is_err() || *shutdown_rx.borrow() {
                            break;
                        }
                    }
                }
            }
        });

        Ok(WsGatewayHandle {
            bind_address,
            accepted_sessions,
            active_sessions,
            shutdown: Some(shutdown_tx),
            task: Some(task),
        })
    }
}

/// Handle for a running WebSocket gateway service.
pub struct WsGatewayHandle {
    bind_address: SocketAddr,
    accepted_sessions: Arc<AtomicUsize>,
    active_sessions: Arc<AtomicUsize>,
    shutdown: Option<watch::Sender<bool>>,
    task: Option<JoinHandle<()>>,
}

impl WsGatewayHandle {
    /// Returns the address the service is listening on.
    #[must_use]
    pub fn bind_address(&self) -> SocketAddr {
        self.bind_address
    }

    /// Returns a serializable snapshot of the current runtime status.
    #[must_use]
    pub fn status(&self) -> WsGatewayRuntimeStatus {
        WsGatewayRuntimeStatus {
            running: true,
            bind_address: self.bind_address.to_string(),
            accepted_sessions: self.accepted_sessions.load(Ordering::Relaxed),
            active_sessions: self.active_sessions.load(Ordering::Relaxed),
        }
    }

    /// Requests shutdown and waits for the listener task to stop.
    pub async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(true);
        }

        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl fmt::Debug for WsGatewayHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WsGatewayHandle")
            .field("bind_address", &self.bind_address)
            .field(
                "accepted_sessions",
                &self.accepted_sessions.load(Ordering::Relaxed),
            )
            .field(
                "active_sessions",
                &self.active_sessions.load(Ordering::Relaxed),
            )
            .finish_non_exhaustive()
    }
}

impl Drop for WsGatewayHandle {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(true);
        }

        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn serve_client(
    gateway: WsGateway,
    stream: TcpStream,
    request_handler: WsRequestHandler,
    accepted_sessions: Arc<AtomicUsize>,
    active_sessions: Arc<AtomicUsize>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let Ok(client) = gateway.accept_client(stream).await else {
        return;
    };
    let (session, mut socket) = client.into_parts();

    accepted_sessions.fetch_add(1, Ordering::Relaxed);
    active_sessions.fetch_add(1, Ordering::Relaxed);

    loop {
        tokio::select! {
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    let _ = socket.close(None).await;
                    break;
                }
            }
            message = socket.next() => {
                let Some(message) = message else {
                    break;
                };
                match message {
                    Ok(Message::Text(text)) => {
                        let response = match serde_json::from_str::<JsonRpcRequest>(&text) {
                            Ok(request) => request_handler(session.clone(), request).await,
                            Err(error) => JsonRpcResponse::error(
                                crate::RequestId::String("invalid-request".to_owned()),
                                RpcError::new(-32700, format!("invalid JSON-RPC request: {error}")),
                            ),
                        };

                        let Ok(payload) = serde_json::to_string(&response) else {
                            break;
                        };

                        if socket.send(Message::Text(payload.into())).await.is_err() {
                            break;
                        }
                    }
                    Ok(Message::Close(_)) => break,
                    Ok(Message::Ping(_)) | Ok(Message::Pong(_)) | Ok(Message::Binary(_)) | Ok(Message::Frame(_)) => {}
                    Err(_) => break,
                }
            }
        }
    }

    active_sessions.fetch_sub(1, Ordering::Relaxed);
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
    use serde_json::json;
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

    #[tokio::test]
    async fn service_routes_json_rpc_messages() {
        let service = WsGatewayService::new("127.0.0.1:0", GatewayPolicy::local_default());
        let handler: WsRequestHandler = Arc::new(|_session, request| {
            Box::pin(
                async move { JsonRpcResponse::ok(request.id, json!({ "method": request.method })) },
            )
        });
        let handle = service.start(handler).await.unwrap();
        let address = handle.bind_address();

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
        let _accepted = client.next().await.unwrap().unwrap();

        let request = JsonRpcRequest::new(
            crate::RequestId::Number(7),
            "device.status",
            Some(json!({ "deviceId": "coyote-v3" })),
        );
        client
            .send(Message::Text(
                serde_json::to_string(&request).unwrap().into(),
            ))
            .await
            .unwrap();

        let response = client.next().await.unwrap().unwrap().into_text().unwrap();
        let response: JsonRpcResponse = serde_json::from_str(&response).unwrap();

        assert_eq!(response.id, crate::RequestId::Number(7));
        assert_eq!(response.result, Some(json!({ "method": "device.status" })));

        let status = handle.status();
        assert_eq!(status.accepted_sessions, 1);
        assert_eq!(status.active_sessions, 1);

        handle.stop().await;
    }
}
