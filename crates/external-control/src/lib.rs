#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! WebSocket external-control protocol and gateway transport for ArcFlow.
//!
//! This crate models the wire messages and security posture for external
//! software. Core still owns command authorization and execution.

pub mod protocol;
pub mod security;
pub mod server;
pub mod session;

pub use protocol::{
    ClientHello, JsonRpcRequest, JsonRpcResponse, RequestId, RpcError, ServerEvent,
    PROTOCOL_VERSION,
};
pub use security::{ExposureMode, DEFAULT_LOCAL_BIND};
pub use server::{
    AcceptedClient, AcceptedSession, WsGateway, WsGatewayError, WsGatewayHandle,
    WsGatewayRuntimeStatus, WsGatewayService, WsRequestFuture, WsRequestHandler,
};
pub use session::{ClientSession, GatewayPolicy, SessionError};
