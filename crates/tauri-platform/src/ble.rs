//! Tauri 2 BLE discovery adapter scaffold.

use arcflow_core::{
    BleAdapterStatus, BleAdvertisement, BleDeviceDiscoveryController, BleDiscovery, CoreError,
    DeviceDiscoveryController, DeviceScanResult,
};
use async_trait::async_trait;

/// Tauri BLE discovery state exposed by the platform adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TauriBleDiscoveryState {
    /// No Tauri BLE provider is configured for this platform yet.
    Unsupported,
    /// The operating system denied BLE access.
    PermissionDenied,
    /// BLE radio is powered off.
    PoweredOff,
    /// BLE is ready and returned advertisements.
    Ready {
        /// Advertisements returned by the platform scan.
        advertisements: Vec<BleAdvertisement>,
    },
}

/// Device discovery controller used by Tauri 2 desktop and mobile shells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TauriBleDiscoveryController {
    state: TauriBleDiscoveryState,
}

impl TauriBleDiscoveryController {
    /// Constructs an unsupported Tauri BLE discovery controller.
    #[must_use]
    pub fn unsupported() -> Self {
        Self {
            state: TauriBleDiscoveryState::Unsupported,
        }
    }

    /// Constructs a controller with an explicit state.
    #[must_use]
    pub fn with_state(state: TauriBleDiscoveryState) -> Self {
        Self { state }
    }

    /// Returns the configured discovery state.
    #[must_use]
    pub fn state(&self) -> &TauriBleDiscoveryState {
        &self.state
    }
}

#[async_trait]
impl DeviceDiscoveryController for TauriBleDiscoveryController {
    async fn scan_devices(&self) -> Result<DeviceScanResult, CoreError> {
        match &self.state {
            TauriBleDiscoveryState::Unsupported => Ok(DeviceScanResult::new(
                BleAdapterStatus::Unsupported,
                Vec::new(),
            )),
            TauriBleDiscoveryState::PermissionDenied => Ok(DeviceScanResult::new(
                BleAdapterStatus::PermissionDenied,
                Vec::new(),
            )),
            TauriBleDiscoveryState::PoweredOff => Ok(DeviceScanResult::new(
                BleAdapterStatus::PoweredOff,
                Vec::new(),
            )),
            TauriBleDiscoveryState::Ready { advertisements } => {
                BleDeviceDiscoveryController::new(StaticBleDiscovery {
                    advertisements: advertisements.clone(),
                })
                .scan_devices()
                .await
            }
        }
    }
}

#[derive(Debug, Clone)]
struct StaticBleDiscovery {
    advertisements: Vec<BleAdvertisement>,
}

#[async_trait]
impl BleDiscovery for StaticBleDiscovery {
    async fn scan(&self) -> Result<Vec<BleAdvertisement>, CoreError> {
        Ok(self.advertisements.clone())
    }
}

#[cfg(test)]
mod tests {
    use arcflow_core::{DeviceId, DeviceModel, COYOTE_V2_SERVICE_UUID, COYOTE_V3_SERVICE_UUID};

    use super::*;

    #[tokio::test]
    async fn reports_unsupported_adapter_state() {
        let controller = TauriBleDiscoveryController::unsupported();

        let scan = controller.scan_devices().await.unwrap();

        assert_eq!(scan.adapter_status, BleAdapterStatus::Unsupported);
        assert!(scan.devices.is_empty());
    }

    #[tokio::test]
    async fn reports_permission_and_power_states() {
        let permission =
            TauriBleDiscoveryController::with_state(TauriBleDiscoveryState::PermissionDenied);
        let powered_off =
            TauriBleDiscoveryController::with_state(TauriBleDiscoveryState::PoweredOff);

        assert_eq!(
            permission.scan_devices().await.unwrap().adapter_status,
            BleAdapterStatus::PermissionDenied
        );
        assert_eq!(
            powered_off.scan_devices().await.unwrap().adapter_status,
            BleAdapterStatus::PoweredOff
        );
    }

    #[tokio::test]
    async fn maps_ready_advertisements_to_coyote_devices() {
        let controller = TauriBleDiscoveryController::with_state(TauriBleDiscoveryState::Ready {
            advertisements: vec![
                BleAdvertisement::new(
                    DeviceId::new("v2"),
                    Some("Coyote 2".to_owned()),
                    None,
                    vec![COYOTE_V2_SERVICE_UUID],
                ),
                BleAdvertisement::new(
                    DeviceId::new("v3"),
                    Some("Coyote".to_owned()),
                    None,
                    vec![COYOTE_V3_SERVICE_UUID],
                ),
            ],
        });

        let scan = controller.scan_devices().await.unwrap();

        assert_eq!(scan.adapter_status, BleAdapterStatus::Ready);
        assert_eq!(scan.devices[0].model, DeviceModel::CoyoteV2);
        assert_eq!(scan.devices[1].model, DeviceModel::CoyoteV3);
    }
}
