//! Core error types.

use core::fmt;

use arcflow_protocol::ProtocolError;
use arcflow_script::ScriptError;
use arcflow_storage::StorageError;
use arcflow_wave::WaveError;

/// Error returned by Rust Core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    /// A requested value exceeds the active safety limit.
    SafetyLimitExceeded {
        /// Field name.
        field: &'static str,
        /// Requested value.
        value: u16,
        /// Active limit.
        limit: u16,
    },
    /// Wave planning failed.
    Wave(WaveError),
    /// Protocol construction or parsing failed.
    Protocol(ProtocolError),
    /// BLE transport failed.
    Transport(String),
    /// Script parsing, compilation, or execution failed.
    Script(String),
    /// Core-owned storage failed.
    Storage(String),
    /// External-control request could not be routed.
    InvalidExternalRequest(String),
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SafetyLimitExceeded {
                field,
                value,
                limit,
            } => write!(f, "{field} value {value} exceeds safety limit {limit}"),
            Self::Wave(error) => write!(f, "wave error: {error}"),
            Self::Protocol(error) => write!(f, "protocol error: {error}"),
            Self::Transport(error) => write!(f, "transport error: {error}"),
            Self::Script(error) => write!(f, "script error: {error}"),
            Self::Storage(error) => write!(f, "storage error: {error}"),
            Self::InvalidExternalRequest(error) => {
                write!(f, "invalid external-control request: {error}")
            }
        }
    }
}

impl std::error::Error for CoreError {}

impl From<WaveError> for CoreError {
    fn from(error: WaveError) -> Self {
        Self::Wave(error)
    }
}

impl From<ProtocolError> for CoreError {
    fn from(error: ProtocolError) -> Self {
        Self::Protocol(error)
    }
}

impl From<ScriptError> for CoreError {
    fn from(error: ScriptError) -> Self {
        Self::Script(error.to_string())
    }
}

impl From<StorageError> for CoreError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error.to_string())
    }
}
