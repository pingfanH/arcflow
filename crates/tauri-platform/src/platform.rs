//! Shared Tauri BLE platform provider contract.

use core::fmt;

use arcflow_core::CoreError;
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
pub trait TauriBlePlatformProvider:
    TauriBleDiscoveryProvider + TauriBleTransportProvider + fmt::Debug + Send + Sync
{
}

impl<T> TauriBlePlatformProvider for T where
    T: TauriBleDiscoveryProvider + TauriBleTransportProvider + fmt::Debug + Send + Sync
{
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

    #[test]
    fn platform_provider_can_be_split_into_core_adapters() {
        let provider = Arc::new(UnsupportedTauriBlePlatformProvider);
        let discovery: Arc<dyn TauriBleDiscoveryProvider> = provider.clone();
        let transport: Arc<dyn TauriBleTransportProvider> = provider;

        assert!(format!("{discovery:?}").contains("UnsupportedTauriBlePlatformProvider"));
        assert!(format!("{transport:?}").contains("UnsupportedTauriBlePlatformProvider"));
    }
}
