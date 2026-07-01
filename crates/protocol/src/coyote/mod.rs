//! DG-LAB Coyote pulse host protocol support.
//!
//! The Module currently covers Coyote V2 and V3 protocol bytes. It does not
//! open Bluetooth connections or issue writes by itself.

use crate::ProtocolError;

pub mod v2;
pub mod v3;

const BATTERY_LEVEL_LEN: usize = 1;
const MAX_BATTERY_PERCENT: u8 = 100;

/// Coyote V2 advertised Bluetooth name documented by DG-LAB.
pub const V2_DEVICE_NAME: &str = "D-LAB ESTIM01";

/// Coyote V3 advertised Bluetooth name documented by DG-LAB.
pub const V3_DEVICE_NAME: &str = "47L121000";

/// Coyote output channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Channel {
    /// Channel A.
    A,
    /// Channel B.
    B,
}

/// Battery level reported by the shared Coyote battery characteristic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryLevel {
    percent: u8,
}

impl BatteryLevel {
    /// Constructs a battery level in the documented `0..=100` range.
    pub fn new(percent: u8) -> Result<Self, ProtocolError> {
        if percent > MAX_BATTERY_PERCENT {
            return Err(ProtocolError::FieldOutOfRange {
                field: "coyote battery level",
                value: u16::from(percent),
                min: 0,
                max: u16::from(MAX_BATTERY_PERCENT),
            });
        }

        Ok(Self { percent })
    }

    /// Parses the one-byte battery characteristic payload.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() != BATTERY_LEVEL_LEN {
            return Err(ProtocolError::InvalidLength {
                name: "coyote battery level",
                expected: BATTERY_LEVEL_LEN,
                actual: bytes.len(),
            });
        }

        Self::new(bytes[0])
    }

    /// Returns the battery percentage.
    #[must_use]
    pub fn percent(self) -> u8 {
        self.percent
    }

    /// Encodes the battery level as a single byte.
    #[must_use]
    pub fn to_byte(self) -> u8 {
        self.percent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_shared_battery_level() {
        let level = BatteryLevel::from_bytes(&[87]).unwrap();

        assert_eq!(level.percent(), 87);
        assert_eq!(level.to_byte(), 87);
    }

    #[test]
    fn rejects_invalid_shared_battery_payloads() {
        assert!(matches!(
            BatteryLevel::from_bytes(&[]),
            Err(ProtocolError::InvalidLength {
                name: "coyote battery level",
                expected: 1,
                actual: 0
            })
        ));
        assert!(matches!(
            BatteryLevel::from_bytes(&[101]),
            Err(ProtocolError::FieldOutOfRange {
                field: "coyote battery level",
                value: 101,
                min: 0,
                max: 100
            })
        ));
    }
}
