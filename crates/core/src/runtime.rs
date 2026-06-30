//! High-level ArcFlow Core runtime facade.

use crate::{BleAdapterStatus, DeviceScanResult, SafetyLimits, StopOutputResult};

/// Application core facade shared by desktop, mobile, plugins, and external control.
#[derive(Debug, Clone)]
pub struct ArcFlowCore {
    safety_limits: SafetyLimits,
}

impl ArcFlowCore {
    /// Constructs a core runtime with explicit safety limits.
    #[must_use]
    pub fn new(safety_limits: SafetyLimits) -> Self {
        Self { safety_limits }
    }

    /// Returns the active safety limits.
    #[must_use]
    pub fn safety_limits(&self) -> SafetyLimits {
        self.safety_limits
    }

    /// Scans for supported devices through the platform BLE Adapter.
    ///
    /// The current scaffold has not attached a platform Adapter yet, so it
    /// returns an explicit unsupported Adapter status instead of reaching into
    /// React or platform APIs.
    #[must_use]
    pub fn scan_devices(&self) -> DeviceScanResult {
        DeviceScanResult::new(BleAdapterStatus::Unsupported, Vec::new())
    }

    /// Stops all active output sessions known by Core.
    #[must_use]
    pub fn stop_all_output(&self) -> StopOutputResult {
        StopOutputResult::new(Vec::new())
    }
}

impl Default for ArcFlowCore {
    fn default() -> Self {
        Self::new(SafetyLimits::conservative())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_devices_reports_missing_platform_adapter() {
        let core = ArcFlowCore::default();
        let scan = core.scan_devices();

        assert_eq!(scan.adapter_status, BleAdapterStatus::Unsupported);
        assert!(scan.devices.is_empty());
    }

    #[test]
    fn stop_all_output_is_safe_with_no_sessions() {
        let core = ArcFlowCore::default();
        let result = core.stop_all_output();

        assert!(result.stopped_devices.is_empty());
    }
}
