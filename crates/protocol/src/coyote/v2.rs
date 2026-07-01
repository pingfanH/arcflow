//! Coyote V2 protocol frames.
//!
//! V2 uses three-byte little-endian bit fields for channel strength and
//! waveform output. The Bluetooth transport characteristics live outside this
//! crate.

use crate::ProtocolError;

const U24_LEN: usize = 3;
const MAX_CHANNEL_STRENGTH: u16 = 2047;
const MAX_X: u8 = 31;
const MAX_Y: u16 = 1023;
const MAX_Z: u8 = 31;

/// Shared Coyote battery level type, kept here for V2 import compatibility.
pub use super::BatteryLevel;

/// Packed V2 AB channel strength frame from the `PWM_AB2` characteristic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelStrengths {
    a: u16,
    b: u16,
}

impl ChannelStrengths {
    /// Constructs channel strengths in the documented `0..=2047` range.
    pub fn new(a: u16, b: u16) -> Result<Self, ProtocolError> {
        ensure_range("v2 channel A strength", a, 0, MAX_CHANNEL_STRENGTH)?;
        ensure_range("v2 channel B strength", b, 0, MAX_CHANNEL_STRENGTH)?;
        Ok(Self { a, b })
    }

    /// Parses a three-byte little-endian `PWM_AB2` payload.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let raw = read_u24_le("v2 channel strengths", bytes)?;
        let reserved = raw >> 22;
        if reserved != 0 {
            return Err(ProtocolError::ReservedBitsSet {
                name: "v2 channel strengths",
                bits: reserved,
            });
        }

        let a = ((raw >> 11) & 0x07FF) as u16;
        let b = (raw & 0x07FF) as u16;
        Self::new(a, b)
    }

    /// Encodes the strengths as a three-byte little-endian payload.
    #[must_use]
    pub fn to_bytes(self) -> [u8; U24_LEN] {
        write_u24_le((u32::from(self.a) << 11) | u32::from(self.b))
    }

    /// Returns the channel A strength.
    #[must_use]
    pub fn a(self) -> u16 {
        self.a
    }

    /// Returns the channel B strength.
    #[must_use]
    pub fn b(self) -> u16 {
        self.b
    }
}

/// Packed V2 waveform frame containing X, Y, and Z pulse parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaveformFrame {
    x: u8,
    y: u16,
    z: u8,
}

impl WaveformFrame {
    /// Constructs a V2 waveform frame.
    pub fn new(x: u8, y: u16, z: u8) -> Result<Self, ProtocolError> {
        ensure_range("v2 waveform X", u16::from(x), 0, u16::from(MAX_X))?;
        ensure_range("v2 waveform Y", y, 0, MAX_Y)?;
        ensure_range("v2 waveform Z", u16::from(z), 0, u16::from(MAX_Z))?;
        Ok(Self { x, y, z })
    }

    /// Parses a three-byte little-endian waveform payload.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let raw = read_u24_le("v2 waveform frame", bytes)?;
        let reserved = raw >> 20;
        if reserved != 0 {
            return Err(ProtocolError::ReservedBitsSet {
                name: "v2 waveform frame",
                bits: reserved,
            });
        }

        let x = (raw & 0x001F) as u8;
        let y = ((raw >> 5) & 0x03FF) as u16;
        let z = ((raw >> 15) & 0x001F) as u8;
        Self::new(x, y, z)
    }

    /// Encodes the frame as a three-byte little-endian payload.
    #[must_use]
    pub fn to_bytes(self) -> [u8; U24_LEN] {
        write_u24_le(u32::from(self.x) | (u32::from(self.y) << 5) | (u32::from(self.z) << 15))
    }

    /// Returns X, the pulse count portion of the waveform cycle.
    #[must_use]
    pub fn x(self) -> u8 {
        self.x
    }

    /// Returns Y, the interval portion of the waveform cycle.
    #[must_use]
    pub fn y(self) -> u16 {
        self.y
    }

    /// Returns Z, the pulse width control value.
    #[must_use]
    pub fn z(self) -> u8 {
        self.z
    }

    /// Returns `X + Y`, the V2 waveform frequency period in milliseconds.
    #[must_use]
    pub fn frequency_ms(self) -> u16 {
        u16::from(self.x) + self.y
    }

    /// Returns the documented pulse width in microseconds (`Z * 5us`).
    #[must_use]
    pub fn pulse_width_micros(self) -> u16 {
        u16::from(self.z) * 5
    }
}

fn ensure_len(name: &'static str, bytes: &[u8], expected: usize) -> Result<(), ProtocolError> {
    if bytes.len() != expected {
        return Err(ProtocolError::InvalidLength {
            name,
            expected,
            actual: bytes.len(),
        });
    }
    Ok(())
}

fn ensure_range(field: &'static str, value: u16, min: u16, max: u16) -> Result<(), ProtocolError> {
    if !(min..=max).contains(&value) {
        return Err(ProtocolError::FieldOutOfRange {
            field,
            value,
            min,
            max,
        });
    }
    Ok(())
}

fn read_u24_le(name: &'static str, bytes: &[u8]) -> Result<u32, ProtocolError> {
    ensure_len(name, bytes, U24_LEN)?;
    Ok(u32::from(bytes[0]) | (u32::from(bytes[1]) << 8) | (u32::from(bytes[2]) << 16))
}

fn write_u24_le(value: u32) -> [u8; U24_LEN] {
    [
        (value & 0xFF) as u8,
        ((value >> 8) & 0xFF) as u8,
        ((value >> 16) & 0xFF) as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_documented_breathing_waveform_frame() {
        let frame = WaveformFrame::from_bytes(&[0x21, 0x01, 0x0A]).unwrap();

        assert_eq!(frame.x(), 1);
        assert_eq!(frame.y(), 9);
        assert_eq!(frame.z(), 20);
        assert_eq!(frame.frequency_ms(), 10);
        assert_eq!(frame.pulse_width_micros(), 100);
        assert_eq!(frame.to_bytes(), [0x21, 0x01, 0x0A]);
    }

    #[test]
    fn parses_documented_tide_waveform_frame() {
        let frame = WaveformFrame::from_bytes(&[0x41, 0x81, 0x01]).unwrap();

        assert_eq!(frame.x(), 1);
        assert_eq!(frame.y(), 10);
        assert_eq!(frame.z(), 3);
        assert_eq!(frame.to_bytes(), [0x41, 0x81, 0x01]);
    }

    #[test]
    fn rejects_waveform_reserved_bits() {
        let error = WaveformFrame::from_bytes(&[0x00, 0x00, 0x10]).unwrap_err();

        assert!(matches!(
            error,
            ProtocolError::ReservedBitsSet {
                name: "v2 waveform frame",
                bits: 1
            }
        ));
    }

    #[test]
    fn round_trips_channel_strengths() {
        let strengths = ChannelStrengths::new(127, 511).unwrap();

        assert_eq!(
            ChannelStrengths::from_bytes(&strengths.to_bytes()).unwrap(),
            strengths
        );
    }

    #[test]
    fn parses_battery_level() {
        let level = BatteryLevel::from_bytes(&[87]).unwrap();

        assert_eq!(level.percent(), 87);
        assert_eq!(level.to_byte(), 87);
    }
}
