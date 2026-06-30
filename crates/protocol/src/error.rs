//! Shared protocol parsing and construction errors.

use core::fmt;

/// Error returned when protocol bytes cannot be parsed or constructed safely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    /// A frame had an unexpected byte length.
    InvalidLength {
        /// Name of the frame being parsed.
        name: &'static str,
        /// Expected byte length.
        expected: usize,
        /// Actual byte length.
        actual: usize,
    },
    /// A frame header byte did not match the expected value.
    InvalidHeader {
        /// Name of the frame being parsed.
        name: &'static str,
        /// Expected header byte.
        expected: u8,
        /// Actual header byte.
        actual: u8,
    },
    /// A frame header is not supported by this parser.
    UnknownHeader {
        /// Name of the parser that rejected the header.
        name: &'static str,
        /// Actual header byte.
        actual: u8,
    },
    /// Reserved bits were set in a packed frame.
    ReservedBitsSet {
        /// Name of the frame being parsed.
        name: &'static str,
        /// Reserved bits after shifting them down to bit 0.
        bits: u32,
    },
    /// A field value is outside the range ArcFlow constructs for writes.
    FieldOutOfRange {
        /// Field name.
        field: &'static str,
        /// Actual value.
        value: u16,
        /// Minimum allowed value.
        min: u16,
        /// Maximum allowed value.
        max: u16,
    },
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength {
                name,
                expected,
                actual,
            } => write!(f, "{name} expected {expected} bytes, got {actual} bytes"),
            Self::InvalidHeader {
                name,
                expected,
                actual,
            } => write!(
                f,
                "{name} expected header 0x{expected:02X}, got 0x{actual:02X}"
            ),
            Self::UnknownHeader { name, actual } => {
                write!(f, "{name} has unsupported header 0x{actual:02X}")
            }
            Self::ReservedBitsSet { name, bits } => {
                write!(f, "{name} has reserved bits set: 0x{bits:X}")
            }
            Self::FieldOutOfRange {
                field,
                value,
                min,
                max,
            } => write!(f, "{field} must be in {min}..={max}, got {value}"),
        }
    }
}

impl std::error::Error for ProtocolError {}
