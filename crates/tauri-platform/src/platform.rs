//! Shared Tauri BLE platform provider contract.

use core::fmt;

use arcflow_core::{CoreError, DeviceId};
use async_trait::async_trait;

use crate::{
    TauriBleDiscoveryProvider, TauriBleDiscoveryState, TauriBleSubscriptionRequest,
    TauriBleTransportProvider, TauriBleWriteRequest,
};

/// Combined provider implemented by a Tauri 2 BLE backend.
///
/// Desktop and mobile shells should inject one provider that owns both device
/// discovery and device-scoped writes. Core still talks only to the narrow
/// discovery and transport traits.
#[async_trait]
pub trait TauriBlePlatformProvider:
    TauriBleDiscoveryProvider + TauriBleTransportProvider + fmt::Debug + Send + Sync
{
    /// Connects to a discovered BLE device and performs basic status setup.
    async fn connect_device(
        &self,
        device_id: DeviceId,
    ) -> Result<TauriBleConnectionState, CoreError>;

    /// Reads a connected device battery percentage when available.
    async fn read_battery_percent(&self, device_id: &DeviceId) -> Result<Option<u8>, CoreError>;
}

/// Result returned after connecting a platform BLE device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TauriBleConnectionState {
    /// Connected device id.
    pub device_id: DeviceId,
    /// Battery percentage if the platform can read it.
    pub battery_percent: Option<u8>,
}

impl TauriBleConnectionState {
    /// Constructs a connection state.
    #[must_use]
    pub fn new(device_id: DeviceId, battery_percent: Option<u8>) -> Self {
        Self {
            device_id,
            battery_percent,
        }
    }
}

/// Combined provider used until a real Tauri BLE backend is attached.
#[derive(Debug, Default)]
pub struct UnsupportedTauriBlePlatformProvider;

#[async_trait]
impl TauriBleDiscoveryProvider for UnsupportedTauriBlePlatformProvider {
    async fn scan_state(&self) -> Result<TauriBleDiscoveryState, CoreError> {
        Ok(TauriBleDiscoveryState::Unsupported)
    }
}

#[async_trait]
impl TauriBleTransportProvider for UnsupportedTauriBlePlatformProvider {
    async fn write(&self, _request: TauriBleWriteRequest) -> Result<(), CoreError> {
        Err(CoreError::Transport(
            "tauri BLE platform provider is not configured".to_owned(),
        ))
    }

    async fn subscribe(&self, _request: TauriBleSubscriptionRequest) -> Result<(), CoreError> {
        Err(CoreError::Transport(
            "tauri BLE platform provider is not configured".to_owned(),
        ))
    }
}

#[async_trait]
impl TauriBlePlatformProvider for UnsupportedTauriBlePlatformProvider {
    async fn connect_device(
        &self,
        _device_id: DeviceId,
    ) -> Result<TauriBleConnectionState, CoreError> {
        Err(CoreError::Transport(
            "tauri BLE platform provider is not configured".to_owned(),
        ))
    }

    async fn read_battery_percent(&self, _device_id: &DeviceId) -> Result<Option<u8>, CoreError> {
        Err(CoreError::Transport(
            "tauri BLE platform provider is not configured".to_owned(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arcflow_core::{BleCharacteristic, BleWrite, DeviceId};

    use super::*;

    #[tokio::test]
    async fn unsupported_platform_provider_reports_unsupported_discovery() {
        let provider = UnsupportedTauriBlePlatformProvider;

        assert_eq!(
            provider.scan_state().await.unwrap(),
            TauriBleDiscoveryState::Unsupported
        );
    }

    #[tokio::test]
    async fn unsupported_platform_provider_rejects_transport_operations() {
        let provider = UnsupportedTauriBlePlatformProvider;

        let write_error = provider
            .write(TauriBleWriteRequest::new(
                DeviceId::new("coyote-v3"),
                BleWrite::new(BleCharacteristic::CoyoteV3Write, [0xB0]),
            ))
            .await
            .unwrap_err();
        let subscribe_error = provider
            .subscribe(TauriBleSubscriptionRequest::new(
                DeviceId::new("coyote-v3"),
                BleCharacteristic::CoyoteV3Notify,
            ))
            .await
            .unwrap_err();

        assert!(write_error.to_string().contains("not configured"));
        assert!(subscribe_error.to_string().contains("not configured"));
    }

    #[tokio::test]
    async fn unsupported_platform_provider_rejects_connection_operations() {
        let provider = UnsupportedTauriBlePlatformProvider;

        let connect_error = provider
            .connect_device(DeviceId::new("coyote-v3"))
            .await
            .unwrap_err();
        let battery_error = provider
            .read_battery_percent(&DeviceId::new("coyote-v3"))
            .await
            .unwrap_err();

        assert!(connect_error.to_string().contains("not configured"));
        assert!(battery_error.to_string().contains("not configured"));
    }

    #[test]
    fn platform_provider_can_be_split_into_core_adapters() {
        let provider = Arc::new(UnsupportedTauriBlePlatformProvider);
        let discovery: Arc<dyn TauriBleDiscoveryProvider> = provider.clone();
        let transport: Arc<dyn TauriBleTransportProvider> = provider;

        assert!(format!("{discovery:?}").contains("UnsupportedTauriBlePlatformProvider"));
        assert!(format!("{transport:?}").contains("UnsupportedTauriBlePlatformProvider"));
    }
}
