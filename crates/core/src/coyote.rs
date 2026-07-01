//! Coyote device command builders.

use arcflow_protocol::coyote::v3::{BfCommand, StrengthMode, StrengthModes};
use arcflow_wave::{ChannelOutput, CoyoteV3Window};

use crate::{BleCharacteristic, BleWrite, CoreError, SafetyLimits};

/// Builds safe Coyote V3 BLE writes from validated wave windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoyoteV3CommandBuilder {
    limits: SafetyLimits,
}

impl CoyoteV3CommandBuilder {
    /// Constructs a command builder with active safety limits.
    #[must_use]
    pub fn new(limits: SafetyLimits) -> Self {
        Self { limits }
    }

    /// Returns the active safety limits.
    #[must_use]
    pub fn limits(self) -> SafetyLimits {
        self.limits
    }

    /// Builds a BLE write for one Coyote V3 B0 command.
    pub fn build_wave_write(
        self,
        window: CoyoteV3Window,
        sequence: u8,
        modes: StrengthModes,
        a_strength: u8,
        b_strength: u8,
    ) -> Result<BleWrite, CoreError> {
        self.ensure_channel_strength("channel A strength", a_strength)?;
        self.ensure_channel_strength("channel B strength", b_strength)?;
        self.ensure_wave_strength(window.max_strength())?;

        let command = window.to_b0_command(sequence, modes, a_strength, b_strength)?;
        Ok(BleWrite::new(
            BleCharacteristic::CoyoteV3Write,
            command.to_bytes(),
        ))
    }

    /// Builds a BLE write that stops Coyote V3 output on both channels.
    pub fn build_stop_write(self, sequence: u8) -> Result<BleWrite, CoreError> {
        self.build_wave_write(
            CoyoteV3Window::new(ChannelOutput::Disabled, ChannelOutput::Disabled),
            sequence,
            StrengthModes::new(StrengthMode::Absolute, StrengthMode::Absolute),
            0,
            0,
        )
    }

    /// Builds a BF write that applies the active channel strength soft limits.
    pub fn build_safety_limits_write(self) -> Result<BleWrite, CoreError> {
        let command = BfCommand::new(
            self.limits.max_channel_strength,
            self.limits.max_channel_strength,
            0,
            0,
            0,
            0,
        )?;

        Ok(BleWrite::new(
            BleCharacteristic::CoyoteV3Write,
            command.to_bytes(),
        ))
    }

    fn ensure_channel_strength(self, field: &'static str, value: u8) -> Result<(), CoreError> {
        if !self.limits.allows_channel_strength(value) {
            return Err(CoreError::SafetyLimitExceeded {
                field,
                value: u16::from(value),
                limit: u16::from(self.limits.max_channel_strength),
            });
        }
        Ok(())
    }

    fn ensure_wave_strength(self, value: u8) -> Result<(), CoreError> {
        if !self.limits.allows_wave_strength(value) {
            return Err(CoreError::SafetyLimitExceeded {
                field: "wave strength",
                value: u16::from(value),
                limit: u16::from(self.limits.max_wave_strength),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcflow_protocol::coyote::v3::{StrengthMode, StrengthModes};
    use arcflow_wave::{ChannelOutput, ChannelWindow, WavePoint};

    fn test_modes() -> StrengthModes {
        StrengthModes::new(StrengthMode::Unchanged, StrengthMode::Unchanged)
    }

    fn safe_window() -> CoyoteV3Window {
        let channel = ChannelWindow::new([
            WavePoint::new(10, 0).unwrap(),
            WavePoint::new(10, 10).unwrap(),
            WavePoint::new(10, 20).unwrap(),
            WavePoint::new(10, 30).unwrap(),
        ]);

        CoyoteV3Window::new(ChannelOutput::Window(channel), ChannelOutput::Disabled)
    }

    #[test]
    fn builds_coyote_v3_ble_write() {
        let write = CoyoteV3CommandBuilder::new(SafetyLimits::conservative())
            .build_wave_write(safe_window(), 0, test_modes(), 0, 0)
            .unwrap();

        assert_eq!(write.characteristic, BleCharacteristic::CoyoteV3Write);
        assert_eq!(
            write.payload,
            vec![
                0xB0, 0x00, 0x00, 0x00, 0x0A, 0x0A, 0x0A, 0x0A, 0x00, 0x0A, 0x14, 0x1E, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x65,
            ]
        );
    }

    #[test]
    fn builds_coyote_v3_stop_write() {
        let write = CoyoteV3CommandBuilder::new(SafetyLimits::conservative())
            .build_stop_write(2)
            .unwrap();

        assert_eq!(write.characteristic, BleCharacteristic::CoyoteV3Write);
        assert_eq!(
            write.payload,
            vec![
                0xB0, 0x2F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x65, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x65,
            ]
        );
    }

    #[test]
    fn builds_coyote_v3_safety_limits_write() {
        let write = CoyoteV3CommandBuilder::new(SafetyLimits::conservative())
            .build_safety_limits_write()
            .unwrap();

        assert_eq!(write.characteristic, BleCharacteristic::CoyoteV3Write);
        assert_eq!(write.payload, vec![0xBF, 20, 20, 0, 0, 0, 0]);
    }

    #[test]
    fn rejects_channel_strength_above_limit() {
        let error = CoyoteV3CommandBuilder::new(SafetyLimits::conservative())
            .build_wave_write(safe_window(), 0, test_modes(), 21, 0)
            .unwrap_err();

        assert!(matches!(
            error,
            CoreError::SafetyLimitExceeded {
                field: "channel A strength",
                value: 21,
                limit: 20
            }
        ));
    }

    #[test]
    fn rejects_wave_strength_above_limit() {
        let channel = ChannelWindow::new([
            WavePoint::new(10, 0).unwrap(),
            WavePoint::new(10, 10).unwrap(),
            WavePoint::new(10, 31).unwrap(),
            WavePoint::new(10, 30).unwrap(),
        ]);
        let window = CoyoteV3Window::new(ChannelOutput::Window(channel), ChannelOutput::Disabled);

        let error = CoyoteV3CommandBuilder::new(SafetyLimits::conservative())
            .build_wave_write(window, 0, test_modes(), 0, 0)
            .unwrap_err();

        assert!(matches!(
            error,
            CoreError::SafetyLimitExceeded {
                field: "wave strength",
                value: 31,
                limit: 30
            }
        ));
    }
}
