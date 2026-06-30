//! DG-LAB Coyote pulse host protocol support.
//!
//! The Module currently covers Coyote V2 and V3 protocol bytes. It does not
//! open Bluetooth connections or issue writes by itself.

pub mod v2;
pub mod v3;

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
