//! Device safety limits owned by Rust Core.

/// User and device safety limits applied before writing to a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SafetyLimits {
    /// Maximum channel strength allowed for writes.
    pub max_channel_strength: u8,
    /// Maximum wave strength allowed for writes.
    pub max_wave_strength: u8,
}

impl SafetyLimits {
    /// Constructs safety limits with conservative defaults.
    #[must_use]
    pub fn conservative() -> Self {
        Self {
            max_channel_strength: 20,
            max_wave_strength: 30,
        }
    }

    /// Returns whether the channel strength is allowed by these limits.
    #[must_use]
    pub fn allows_channel_strength(self, value: u8) -> bool {
        value <= self.max_channel_strength
    }

    /// Returns whether the wave strength is allowed by these limits.
    #[must_use]
    pub fn allows_wave_strength(self, value: u8) -> bool {
        value <= self.max_wave_strength
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conservative_limits_are_strict() {
        let limits = SafetyLimits::conservative();

        assert!(limits.allows_channel_strength(20));
        assert!(!limits.allows_channel_strength(21));
        assert!(limits.allows_wave_strength(30));
        assert!(!limits.allows_wave_strength(31));
    }
}
