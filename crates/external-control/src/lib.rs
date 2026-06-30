#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! WebSocket external-control protocol model for ArcFlow.
//!
//! This crate models the wire messages and security posture for external
//! software. The WebSocket server implementation belongs in Rust Core.

pub mod protocol;
pub mod security;
pub mod session;

pub use protocol::{
    ClientHello, JsonRpcRequest, JsonRpcResponse, RequestId, RpcError, ServerEvent,
    PROTOCOL_VERSION,
};
pub use security::{ExposureMode, DEFAULT_LOCAL_BIND};
pub use session::{ClientSession, GatewayPolicy, SessionError};
