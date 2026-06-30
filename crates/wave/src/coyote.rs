//! Coyote wave plans and protocol conversion.

use arcflow_protocol::coyote::v3::{
    compress_wave_frequency, B0Command, ChannelWave, StrengthModes,
};

use crate::WaveError;

const MIN_PERIOD_MS: u16 = 10;
const MAX_PERIOD_MS: u16 = 1000;
const MAX_STRENGTH: u8 = 100;

/// One Coyote V3 25ms wave point before protocol compression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavePoint {
    period_ms: u16,
    strength: u8,
}

impl WavePoint {
    /// Constructs a wave point in ArcFlow's safe input range.
    pub fn new(period_ms: u16, strength: u8) -> Result<Self, WaveError> {
        if !(MIN_PERIOD_MS..=MAX_PERIOD_MS).contains(&period_ms) {
            return Err(WaveError::PeriodOutOfRange {
                value: period_ms,
                min: MIN_PERIOD_MS,
                max: MAX_PERIOD_MS,
            });
        }

        if strength > MAX_STRENGTH {
            return Err(WaveError::StrengthOutOfRange {
                value: strength,
                max: MAX_STRENGTH,
            });
        }

        Ok(Self {
            period_ms,
            strength,
        })
    }

    /// Returns the requested period in milliseconds.
    #[must_use]
    pub fn period_ms(self) -> u16 {
        self.period_ms
    }

    /// Returns the requested relative wave strength.
    #[must_use]
    pub fn strength(self) -> u8 {
        self.strength
    }

    /// Converts the point to a compressed Coyote V3 segment.
    pub fn to_v3_segment(self) -> Result<arcflow_protocol::coyote::v3::WaveSegment, WaveError> {
        arcflow_protocol::coyote::v3::WaveSegment::new(
            compress_wave_frequency(self.period_ms),
            self.strength,
        )
        .map_err(WaveError::from)
    }
}

/// Four wave points representing one 100ms channel window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelWindow {
    points: [WavePoint; 4],
}

impl ChannelWindow {
    /// Constructs a channel window from exactly four wave points.
    #[must_use]
    pub fn new(points: [WavePoint; 4]) -> Self {
        Self { points }
    }

    /// Returns the four wave points.
    #[must_use]
    pub fn points(self) -> [WavePoint; 4] {
        self.points
    }

    /// Converts the channel window to a Coyote V3 protocol channel wave.
    pub fn to_v3_channel_wave(self) -> Result<ChannelWave, WaveError> {
        Ok(ChannelWave::new([
            self.points[0].to_v3_segment()?,
            self.points[1].to_v3_segment()?,
            self.points[2].to_v3_segment()?,
            self.points[3].to_v3_segment()?,
        ])?)
    }

    /// Returns the maximum wave strength in this 100ms window.
    #[must_use]
    pub fn max_strength(self) -> u8 {
        self.points
            .iter()
            .map(|point| point.strength())
            .max()
            .unwrap_or(0)
    }
}

/// Channel output state for one 100ms Coyote V3 window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelOutput {
    /// Disable this channel by sending the documented invalid marker.
    Disabled,
    /// Output a validated four-point wave window.
    Window(ChannelWindow),
}

impl ChannelOutput {
    /// Returns the maximum wave strength for this channel output.
    #[must_use]
    pub fn max_strength(self) -> u8 {
        match self {
            Self::Disabled => 0,
            Self::Window(window) => window.max_strength(),
        }
    }

    fn to_v3_channel_wave(self) -> Result<ChannelWave, WaveError> {
        match self {
            Self::Disabled => Ok(ChannelWave::disabled()),
            Self::Window(window) => window.to_v3_channel_wave(),
        }
    }
}

/// Both-channel Coyote V3 wave data for one 100ms B0 command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoyoteV3Window {
    a: ChannelOutput,
    b: ChannelOutput,
}

impl CoyoteV3Window {
    /// Constructs a two-channel Coyote V3 window.
    #[must_use]
    pub fn new(a: ChannelOutput, b: ChannelOutput) -> Self {
        Self { a, b }
    }

    /// Returns channel A output state.
    #[must_use]
    pub fn a(self) -> ChannelOutput {
        self.a
    }

    /// Returns channel B output state.
    #[must_use]
    pub fn b(self) -> ChannelOutput {
        self.b
    }

    /// Converts this window to a Coyote V3 B0 command.
    pub fn to_b0_command(
        self,
        sequence: u8,
        modes: StrengthModes,
        a_strength: u8,
        b_strength: u8,
    ) -> Result<B0Command, WaveError> {
        Ok(B0Command::new(
            sequence,
            modes,
            a_strength,
            b_strength,
            self.a.to_v3_channel_wave()?,
            self.b.to_v3_channel_wave()?,
        )?)
    }

    /// Returns the maximum wave strength across both channels.
    #[must_use]
    pub fn max_strength(self) -> u8 {
        self.a.max_strength().max(self.b.max_strength())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcflow_protocol::coyote::v3::{StrengthMode, StrengthModes};

    #[test]
    fn converts_a_channel_window_to_documented_b0_shape() {
        let window = ChannelWindow::new([
            WavePoint::new(10, 0).unwrap(),
            WavePoint::new(10, 10).unwrap(),
            WavePoint::new(10, 20).unwrap(),
            WavePoint::new(10, 30).unwrap(),
        ]);
        let command = CoyoteV3Window::new(ChannelOutput::Window(window), ChannelOutput::Disabled)
            .to_b0_command(
                0,
                StrengthModes::new(StrengthMode::Unchanged, StrengthMode::Unchanged),
                0,
                0,
            )
            .unwrap();

        assert_eq!(
            command.to_bytes(),
            [
                0xB0, 0x00, 0x00, 0x00, 0x0A, 0x0A, 0x0A, 0x0A, 0x00, 0x0A, 0x14, 0x1E, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x65,
            ]
        );
    }

    #[test]
    fn rejects_periods_outside_safe_range() {
        let error = WavePoint::new(1, 10).unwrap_err();

        assert!(matches!(
            error,
            WaveError::PeriodOutOfRange { value: 1, .. }
        ));
    }
}
