//! Wave validation errors.

use core::fmt;

use arcflow_protocol::ProtocolError;

/// Error returned when a wave plan cannot be constructed or encoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaveError {
    /// Wave period was outside the supported range.
    PeriodOutOfRange {
        /// Actual period in milliseconds.
        value: u16,
        /// Minimum supported period.
        min: u16,
        /// Maximum supported period.
        max: u16,
    },
    /// Wave strength was outside the supported range.
    StrengthOutOfRange {
        /// Actual strength.
        value: u8,
        /// Maximum supported strength.
        max: u8,
    },
    /// Protocol command construction failed.
    Protocol(ProtocolError),
}

impl fmt::Display for WaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PeriodOutOfRange { value, min, max } => {
                write!(f, "wave period must be in {min}..={max}ms, got {value}ms")
            }
            Self::StrengthOutOfRange { value, max } => {
                write!(f, "wave strength must be in 0..={max}, got {value}")
            }
            Self::Protocol(error) => write!(f, "protocol error: {error}"),
        }
    }
}

impl std::error::Error for WaveError {}

impl From<ProtocolError> for WaveError {
    fn from(error: ProtocolError) -> Self {
        Self::Protocol(error)
    }
}
