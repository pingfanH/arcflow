//! Coyote V3 protocol frames.
//!
//! V3 routes command writes through the 0x150A characteristic and responses
//! through the 0x150B notify characteristic. This module models the command
//! bytes, not the Bluetooth transport.

use crate::ProtocolError;

/// Header byte for a V3 B0 waveform and strength command.
pub const B0_HEADER: u8 = 0xB0;
/// Header byte for a V3 B1 strength notification.
pub const B1_HEADER: u8 = 0xB1;
/// Header byte for a V3 BF soft-limit and balance command.
pub const BF_HEADER: u8 = 0xBF;

const B0_LEN: usize = 20;
const B1_LEN: usize = 4;
const BF_LEN: usize = 7;
const CHANNEL_WAVE_LEN: usize = 4;
const MAX_SEQUENCE: u8 = 0x0F;
const MAX_STRENGTH_SETTING: u8 = 200;
const MIN_WAVE_FREQUENCY: u8 = 10;
const MAX_WAVE_FREQUENCY: u8 = 240;
const MAX_WAVE_STRENGTH: u8 = 100;

/// How a B0 strength setting should be interpreted for one channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrengthMode {
    /// Leave the channel strength unchanged.
    Unchanged,
    /// Increase the current channel strength by the setting value.
    Increase,
    /// Decrease the current channel strength by the setting value.
    Decrease,
    /// Set the channel strength to the setting value.
    Absolute,
}

impl StrengthMode {
    /// Decodes a two-bit strength mode.
    pub fn from_bits(bits: u8) -> Result<Self, ProtocolError> {
        match bits {
            0 => Ok(Self::Unchanged),
            1 => Ok(Self::Increase),
            2 => Ok(Self::Decrease),
            3 => Ok(Self::Absolute),
            value => Err(ProtocolError::FieldOutOfRange {
                field: "v3 strength mode bits",
                value: u16::from(value),
                min: 0,
                max: 3,
            }),
        }
    }

    /// Encodes this strength mode as two bits.
    #[must_use]
    pub fn bits(self) -> u8 {
        match self {
            Self::Unchanged => 0,
            Self::Increase => 1,
            Self::Decrease => 2,
            Self::Absolute => 3,
        }
    }
}

/// Strength interpretation modes for both Coyote channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StrengthModes {
    a: StrengthMode,
    b: StrengthMode,
}

impl StrengthModes {
    /// Constructs channel strength interpretation modes.
    #[must_use]
    pub fn new(a: StrengthMode, b: StrengthMode) -> Self {
        Self { a, b }
    }

    /// Parses the low nibble from the second B0 command byte.
    pub fn from_nibble(nibble: u8) -> Result<Self, ProtocolError> {
        if nibble > 0x0F {
            return Err(ProtocolError::FieldOutOfRange {
                field: "v3 strength mode nibble",
                value: u16::from(nibble),
                min: 0,
                max: 0x0F,
            });
        }

        Ok(Self {
            a: StrengthMode::from_bits((nibble >> 2) & 0x03)?,
            b: StrengthMode::from_bits(nibble & 0x03)?,
        })
    }

    /// Encodes the modes into the low nibble of the second B0 command byte.
    #[must_use]
    pub fn to_nibble(self) -> u8 {
        (self.a.bits() << 2) | self.b.bits()
    }

    /// Returns the channel A strength mode.
    #[must_use]
    pub fn a(self) -> StrengthMode {
        self.a
    }

    /// Returns the channel B strength mode.
    #[must_use]
    pub fn b(self) -> StrengthMode {
        self.b
    }
}

/// One 25ms V3 waveform segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaveSegment {
    frequency: u8,
    strength: u8,
}

impl WaveSegment {
    /// Constructs a valid output segment.
    pub fn new(frequency: u8, strength: u8) -> Result<Self, ProtocolError> {
        ensure_range(
            "v3 wave frequency",
            u16::from(frequency),
            u16::from(MIN_WAVE_FREQUENCY),
            u16::from(MAX_WAVE_FREQUENCY),
        )?;
        ensure_range(
            "v3 wave strength",
            u16::from(strength),
            0,
            u16::from(MAX_WAVE_STRENGTH),
        )?;
        Ok(Self {
            frequency,
            strength,
        })
    }

    /// Constructs a raw segment without enforcing the output validity range.
    ///
    /// DG-LAB documents intentionally invalid values as a way to make the
    /// device ignore an entire channel in a B0 command.
    #[must_use]
    pub fn raw(frequency: u8, strength: u8) -> Self {
        Self {
            frequency,
            strength,
        }
    }

    /// Returns the V3 compressed waveform frequency byte.
    #[must_use]
    pub fn frequency(self) -> u8 {
        self.frequency
    }

    /// Returns the V3 waveform strength byte.
    #[must_use]
    pub fn strength(self) -> u8 {
        self.strength
    }

    /// Returns whether this segment is in the documented output range.
    #[must_use]
    pub fn is_valid_for_output(self) -> bool {
        (MIN_WAVE_FREQUENCY..=MAX_WAVE_FREQUENCY).contains(&self.frequency)
            && self.strength <= MAX_WAVE_STRENGTH
    }
}

/// Four 25ms V3 waveform segments, representing one 100ms channel window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelWave {
    segments: [WaveSegment; CHANNEL_WAVE_LEN],
}

impl ChannelWave {
    /// Constructs a channel wave and validates all four output segments.
    pub fn new(segments: [WaveSegment; CHANNEL_WAVE_LEN]) -> Result<Self, ProtocolError> {
        for segment in segments {
            WaveSegment::new(segment.frequency(), segment.strength())?;
        }
        Ok(Self { segments })
    }

    /// Constructs a channel wave from raw frequency and strength bytes.
    #[must_use]
    pub fn raw(frequencies: [u8; CHANNEL_WAVE_LEN], strengths: [u8; CHANNEL_WAVE_LEN]) -> Self {
        Self {
            segments: [
                WaveSegment::raw(frequencies[0], strengths[0]),
                WaveSegment::raw(frequencies[1], strengths[1]),
                WaveSegment::raw(frequencies[2], strengths[2]),
                WaveSegment::raw(frequencies[3], strengths[3]),
            ],
        }
    }

    /// Constructs a documented disabled-channel marker.
    ///
    /// At least one invalid value causes the device to ignore the channel.
    #[must_use]
    pub fn disabled() -> Self {
        Self::raw([0, 0, 0, 0], [0, 0, 0, 101])
    }

    /// Returns the four waveform segments.
    #[must_use]
    pub fn segments(self) -> [WaveSegment; CHANNEL_WAVE_LEN] {
        self.segments
    }

    /// Returns the four frequency bytes.
    #[must_use]
    pub fn frequencies(self) -> [u8; CHANNEL_WAVE_LEN] {
        [
            self.segments[0].frequency(),
            self.segments[1].frequency(),
            self.segments[2].frequency(),
            self.segments[3].frequency(),
        ]
    }

    /// Returns the four strength bytes.
    #[must_use]
    pub fn strengths(self) -> [u8; CHANNEL_WAVE_LEN] {
        [
            self.segments[0].strength(),
            self.segments[1].strength(),
            self.segments[2].strength(),
            self.segments[3].strength(),
        ]
    }

    /// Returns whether all four segments are in the documented output range.
    #[must_use]
    pub fn is_valid_for_output(self) -> bool {
        self.segments
            .iter()
            .all(|segment| segment.is_valid_for_output())
    }
}

/// V3 B0 command containing channel strength changes and 100ms of wave data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct B0Command {
    sequence: u8,
    modes: StrengthModes,
    a_strength: u8,
    b_strength: u8,
    a_wave: ChannelWave,
    b_wave: ChannelWave,
}

impl B0Command {
    /// Constructs a B0 command for safe writes.
    pub fn new(
        sequence: u8,
        modes: StrengthModes,
        a_strength: u8,
        b_strength: u8,
        a_wave: ChannelWave,
        b_wave: ChannelWave,
    ) -> Result<Self, ProtocolError> {
        ensure_range(
            "v3 B0 sequence",
            u16::from(sequence),
            0,
            u16::from(MAX_SEQUENCE),
        )?;
        ensure_range(
            "v3 channel A strength setting",
            u16::from(a_strength),
            0,
            u16::from(MAX_STRENGTH_SETTING),
        )?;
        ensure_range(
            "v3 channel B strength setting",
            u16::from(b_strength),
            0,
            u16::from(MAX_STRENGTH_SETTING),
        )?;

        Ok(Self {
            sequence,
            modes,
            a_strength,
            b_strength,
            a_wave,
            b_wave,
        })
    }

    /// Parses a raw 20-byte B0 command.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ProtocolError> {
        ensure_len("v3 B0 command", bytes, B0_LEN)?;
        ensure_header("v3 B0 command", bytes[0], B0_HEADER)?;

        Ok(Self {
            sequence: bytes[1] >> 4,
            modes: StrengthModes::from_nibble(bytes[1] & 0x0F)?,
            a_strength: bytes[2],
            b_strength: bytes[3],
            a_wave: ChannelWave::raw(
                [bytes[4], bytes[5], bytes[6], bytes[7]],
                [bytes[8], bytes[9], bytes[10], bytes[11]],
            ),
            b_wave: ChannelWave::raw(
                [bytes[12], bytes[13], bytes[14], bytes[15]],
                [bytes[16], bytes[17], bytes[18], bytes[19]],
            ),
        })
    }

    /// Encodes the command as the 20-byte payload written to the V3 write characteristic.
    #[must_use]
    pub fn to_bytes(self) -> [u8; B0_LEN] {
        let a_frequencies = self.a_wave.frequencies();
        let a_strengths = self.a_wave.strengths();
        let b_frequencies = self.b_wave.frequencies();
        let b_strengths = self.b_wave.strengths();

        [
            B0_HEADER,
            (self.sequence << 4) | self.modes.to_nibble(),
            self.a_strength,
            self.b_strength,
            a_frequencies[0],
            a_frequencies[1],
            a_frequencies[2],
            a_frequencies[3],
            a_strengths[0],
            a_strengths[1],
            a_strengths[2],
            a_strengths[3],
            b_frequencies[0],
            b_frequencies[1],
            b_frequencies[2],
            b_frequencies[3],
            b_strengths[0],
            b_strengths[1],
            b_strengths[2],
            b_strengths[3],
        ]
    }

    /// Returns the B0 command sequence number.
    #[must_use]
    pub fn sequence(self) -> u8 {
        self.sequence
    }

    /// Returns the channel strength interpretation modes.
    #[must_use]
    pub fn modes(self) -> StrengthModes {
        self.modes
    }

    /// Returns the channel A strength setting byte.
    #[must_use]
    pub fn a_strength(self) -> u8 {
        self.a_strength
    }

    /// Returns the channel B strength setting byte.
    #[must_use]
    pub fn b_strength(self) -> u8 {
        self.b_strength
    }

    /// Returns channel A wave data.
    #[must_use]
    pub fn a_wave(self) -> ChannelWave {
        self.a_wave
    }

    /// Returns channel B wave data.
    #[must_use]
    pub fn b_wave(self) -> ChannelWave {
        self.b_wave
    }
}

/// V3 B1 notification containing actual channel strengths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct B1Notification {
    sequence: u8,
    a_strength: u8,
    b_strength: u8,
}

impl B1Notification {
    /// Constructs a B1 strength notification value.
    #[must_use]
    pub fn new(sequence: u8, a_strength: u8, b_strength: u8) -> Self {
        Self {
            sequence,
            a_strength,
            b_strength,
        }
    }

    /// Parses the four-byte B1 notify payload.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ProtocolError> {
        ensure_len("v3 B1 notification", bytes, B1_LEN)?;
        ensure_header("v3 B1 notification", bytes[0], B1_HEADER)?;
        Ok(Self::new(bytes[1], bytes[2], bytes[3]))
    }

    /// Encodes the notification as bytes.
    #[must_use]
    pub fn to_bytes(self) -> [u8; B1_LEN] {
        [B1_HEADER, self.sequence, self.a_strength, self.b_strength]
    }

    /// Returns the returned sequence number.
    #[must_use]
    pub fn sequence(self) -> u8 {
        self.sequence
    }

    /// Returns actual channel A strength.
    #[must_use]
    pub fn a_strength(self) -> u8 {
        self.a_strength
    }

    /// Returns actual channel B strength.
    #[must_use]
    pub fn b_strength(self) -> u8 {
        self.b_strength
    }
}

/// V3 BF command for soft limits and balance parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BfCommand {
    a_soft_limit: u8,
    b_soft_limit: u8,
    a_frequency_balance: u8,
    b_frequency_balance: u8,
    a_strength_balance: u8,
    b_strength_balance: u8,
}

impl BfCommand {
    /// Constructs a BF command for safe writes.
    pub fn new(
        a_soft_limit: u8,
        b_soft_limit: u8,
        a_frequency_balance: u8,
        b_frequency_balance: u8,
        a_strength_balance: u8,
        b_strength_balance: u8,
    ) -> Result<Self, ProtocolError> {
        ensure_range(
            "v3 channel A soft limit",
            u16::from(a_soft_limit),
            0,
            u16::from(MAX_STRENGTH_SETTING),
        )?;
        ensure_range(
            "v3 channel B soft limit",
            u16::from(b_soft_limit),
            0,
            u16::from(MAX_STRENGTH_SETTING),
        )?;

        Ok(Self {
            a_soft_limit,
            b_soft_limit,
            a_frequency_balance,
            b_frequency_balance,
            a_strength_balance,
            b_strength_balance,
        })
    }

    /// Parses a raw seven-byte BF command.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ProtocolError> {
        ensure_len("v3 BF command", bytes, BF_LEN)?;
        ensure_header("v3 BF command", bytes[0], BF_HEADER)?;

        Ok(Self {
            a_soft_limit: bytes[1],
            b_soft_limit: bytes[2],
            a_frequency_balance: bytes[3],
            b_frequency_balance: bytes[4],
            a_strength_balance: bytes[5],
            b_strength_balance: bytes[6],
        })
    }

    /// Encodes the command as bytes.
    #[must_use]
    pub fn to_bytes(self) -> [u8; BF_LEN] {
        [
            BF_HEADER,
            self.a_soft_limit,
            self.b_soft_limit,
            self.a_frequency_balance,
            self.b_frequency_balance,
            self.a_strength_balance,
            self.b_strength_balance,
        ]
    }

    /// Returns channel A soft limit.
    #[must_use]
    pub fn a_soft_limit(self) -> u8 {
        self.a_soft_limit
    }

    /// Returns channel B soft limit.
    #[must_use]
    pub fn b_soft_limit(self) -> u8 {
        self.b_soft_limit
    }

    /// Returns channel A frequency balance.
    #[must_use]
    pub fn a_frequency_balance(self) -> u8 {
        self.a_frequency_balance
    }

    /// Returns channel B frequency balance.
    #[must_use]
    pub fn b_frequency_balance(self) -> u8 {
        self.b_frequency_balance
    }

    /// Returns channel A strength balance.
    #[must_use]
    pub fn a_strength_balance(self) -> u8 {
        self.a_strength_balance
    }

    /// Returns channel B strength balance.
    #[must_use]
    pub fn b_strength_balance(self) -> u8 {
        self.b_strength_balance
    }
}

/// V3 write command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// B0 waveform and strength command.
    B0(B0Command),
    /// BF soft-limit and balance command.
    Bf(BfCommand),
}

impl Command {
    /// Parses a V3 write command by header byte.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let Some(header) = bytes.first().copied() else {
            return Err(ProtocolError::InvalidLength {
                name: "v3 command",
                expected: 1,
                actual: 0,
            });
        };

        match header {
            B0_HEADER => Ok(Self::B0(B0Command::from_bytes(bytes)?)),
            BF_HEADER => Ok(Self::Bf(BfCommand::from_bytes(bytes)?)),
            actual => Err(ProtocolError::UnknownHeader {
                name: "v3 command",
                actual,
            }),
        }
    }
}

/// Compresses a desired V3 waveform period in milliseconds to the protocol byte.
///
/// Input values outside `10..=1000` are mapped to `10`, matching the documented
/// fallback formula.
#[must_use]
pub fn compress_wave_frequency(period_ms: u16) -> u8 {
    match period_ms {
        10..=100 => period_ms as u8,
        101..=600 => ((period_ms - 100) / 5 + 100) as u8,
        601..=1000 => ((period_ms - 600) / 10 + 200) as u8,
        _ => 10,
    }
}

/// Expands a V3 compressed waveform frequency byte to a representative period in milliseconds.
///
/// The protocol compresses longer periods into buckets. This returns the exact
/// period for `10..=100`, the lower aligned period for `101..=200`, and the
/// lower aligned period for `201..=240`. Bytes outside the documented output
/// frequency range return `None`.
#[must_use]
pub fn decompress_wave_frequency(frequency: u8) -> Option<u16> {
    match frequency {
        10..=100 => Some(u16::from(frequency)),
        101..=200 => Some((u16::from(frequency) - 100) * 5 + 100),
        201..=240 => Some((u16::from(frequency) - 200) * 10 + 600),
        _ => None,
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

fn ensure_header(name: &'static str, actual: u8, expected: u8) -> Result<(), ProtocolError> {
    if actual != expected {
        return Err(ProtocolError::InvalidHeader {
            name,
            expected,
            actual,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_documented_b0_a_channel_wave() {
        let bytes = [
            0xB0, 0x00, 0x00, 0x00, 0x0A, 0x0A, 0x0A, 0x0A, 0x00, 0x0A, 0x14, 0x1E, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x65,
        ];

        let command = B0Command::from_bytes(&bytes).unwrap();

        assert_eq!(command.sequence(), 0);
        assert_eq!(command.modes().a(), StrengthMode::Unchanged);
        assert_eq!(command.modes().b(), StrengthMode::Unchanged);
        assert_eq!(command.a_wave().frequencies(), [10, 10, 10, 10]);
        assert_eq!(command.a_wave().strengths(), [0, 10, 20, 30]);
        assert!(!command.b_wave().is_valid_for_output());
        assert_eq!(command.to_bytes(), bytes);
    }

    #[test]
    fn parses_documented_b0_sequence_and_strength_mode() {
        let bytes = [
            0xB0, 0x14, 0x0A, 0x00, 0x28, 0x3C, 0x50, 0x64, 0x64, 0x5A, 0x5A, 0x5A, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x65,
        ];

        let command = B0Command::from_bytes(&bytes).unwrap();

        assert_eq!(command.sequence(), 1);
        assert_eq!(command.modes().a(), StrengthMode::Increase);
        assert_eq!(command.modes().b(), StrengthMode::Unchanged);
        assert_eq!(command.a_strength(), 10);
        assert_eq!(command.a_wave().frequencies(), [40, 60, 80, 100]);
        assert_eq!(command.a_wave().strengths(), [100, 90, 90, 90]);
        assert_eq!(command.to_bytes(), bytes);
    }

    #[test]
    fn builds_documented_disabled_channel_marker() {
        let disabled = ChannelWave::disabled();

        assert_eq!(disabled.frequencies(), [0, 0, 0, 0]);
        assert_eq!(disabled.strengths(), [0, 0, 0, 101]);
        assert!(!disabled.is_valid_for_output());
    }

    #[test]
    fn parses_b1_notification() {
        let notification = B1Notification::from_bytes(&[0xB1, 0x01, 0x19, 0x05]).unwrap();

        assert_eq!(notification.sequence(), 1);
        assert_eq!(notification.a_strength(), 25);
        assert_eq!(notification.b_strength(), 5);
        assert_eq!(notification.to_bytes(), [0xB1, 0x01, 0x19, 0x05]);
    }

    #[test]
    fn round_trips_bf_command() {
        let command = BfCommand::new(150, 30, 1, 2, 3, 4).unwrap();

        assert_eq!(command.to_bytes(), [0xBF, 150, 30, 1, 2, 3, 4]);
        assert_eq!(BfCommand::from_bytes(&command.to_bytes()).unwrap(), command);
    }

    #[test]
    fn compresses_documented_v3_wave_frequencies() {
        assert_eq!(compress_wave_frequency(10), 10);
        assert_eq!(compress_wave_frequency(100), 100);
        assert_eq!(compress_wave_frequency(200), 120);
        assert_eq!(compress_wave_frequency(600), 200);
        assert_eq!(compress_wave_frequency(700), 210);
        assert_eq!(compress_wave_frequency(1000), 240);
        assert_eq!(compress_wave_frequency(1), 10);
    }

    #[test]
    fn decompresses_v3_wave_frequency_bytes() {
        assert_eq!(decompress_wave_frequency(10), Some(10));
        assert_eq!(decompress_wave_frequency(100), Some(100));
        assert_eq!(decompress_wave_frequency(101), Some(105));
        assert_eq!(decompress_wave_frequency(120), Some(200));
        assert_eq!(decompress_wave_frequency(200), Some(600));
        assert_eq!(decompress_wave_frequency(210), Some(700));
        assert_eq!(decompress_wave_frequency(240), Some(1000));
        assert_eq!(decompress_wave_frequency(0), None);
        assert_eq!(decompress_wave_frequency(241), None);
    }
}
