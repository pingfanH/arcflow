//! External-control exposure policy.

/// Default local-only bind address for the WebSocket gateway.
pub const DEFAULT_LOCAL_BIND: &str = "127.0.0.1:0";

/// Network exposure mode for the WebSocket gateway.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExposureMode {
    /// Listen only on localhost.
    Localhost,
    /// Listen on a LAN address after explicit user approval.
    Lan,
}
