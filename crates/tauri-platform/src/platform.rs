//! Shared Tauri BLE platform provider contract.

use core::fmt;

use arcflow_core::{CoreError, DeviceId};
use async_trait::async_trait;

use crate::{
    TauriBleDiscoveryProvider, TauriBleDiscoveryState, TauriBleSubscriptionRequest,
    TauriBleTransportProvider, TauriBleWriteRequest,
};

const BLE_SCAN_SAMPLE_LIMIT: usize = 5;

/// Summary of the most recent platform BLE scan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TauriBleScanDiagnostics {
    /// Number of peripherals returned by the platform adapter.
    pub discovered_peripherals: usize,
    /// Number of peripherals whose properties were readable.
    pub inspected_peripherals: usize,
    /// Number of peripherals accepted as ArcFlow-compatible advertisements.
    pub matched_advertisements: usize,
    /// Number of peripherals skipped because the platform exposed no properties.
    pub skipped_missing_properties: usize,
    /// Number of peripherals skipped because neither service UUID nor local name
    /// matched a known Coyote device.
    pub skipped_unknown_peripherals: usize,
    /// Small sample of matched peripherals for runtime diagnostics.
    pub matched_samples: Vec<TauriBlePeripheralDiagnostic>,
    /// Small sample of unknown peripherals for runtime diagnostics.
    pub skipped_unknown_samples: Vec<TauriBlePeripheralDiagnostic>,
}

impl TauriBleScanDiagnostics {
    /// Constructs an empty diagnostic snapshot for one scan.
    #[must_use]
    pub fn new(discovered_peripherals: usize) -> Self {
        Self {
            discovered_peripherals,
            ..Self::default()
        }
    }

    /// Records one matched Coyote peripheral sample.
    pub fn record_matched(&mut self, local_name: Option<&str>, service_uuids: &[u16]) {
        self.matched_advertisements += 1;
        push_limited_sample(
            &mut self.matched_samples,
            TauriBlePeripheralDiagnostic::new(local_name, service_uuids),
        );
    }

    /// Records one skipped unknown peripheral sample.
    pub fn record_unknown(&mut self, local_name: Option<&str>, service_uuids: &[u16]) {
        self.skipped_unknown_peripherals += 1;
        push_limited_sample(
            &mut self.skipped_unknown_samples,
            TauriBlePeripheralDiagnostic::new(local_name, service_uuids),
        );
    }
}

/// Diagnostic details for one sampled BLE peripheral.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TauriBlePeripheralDiagnostic {
    /// Advertised local name if present.
    pub local_name: Option<String>,
    /// Advertised service short UUIDs after platform UUID conversion.
    pub service_uuids: Vec<u16>,
}

impl TauriBlePeripheralDiagnostic {
    /// Constructs a sampled BLE peripheral diagnostic.
    #[must_use]
    pub fn new(local_name: Option<&str>, service_uuids: &[u16]) -> Self {
        Self {
            local_name: local_name.map(ToOwned::to_owned),
            service_uuids: service_uuids.to_vec(),
        }
    }
}

fn push_limited_sample(
    samples: &mut Vec<TauriBlePeripheralDiagnostic>,
    sample: TauriBlePeripheralDiagnostic,
) {
    if samples.len() < BLE_SCAN_SAMPLE_LIMIT {
        samples.push(sample);
    }
}

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

    /// Disconnects a connected BLE device if the backend owns that connection.
    async fn disconnect_device(&self, device_id: &DeviceId) -> Result<(), CoreError>;

    /// Returns diagnostics captured during the most recent platform scan.
    async fn scan_diagnostics(&self) -> Option<TauriBleScanDiagnostics> {
        None
    }

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

    async fn disconnect_device(&self, _device_id: &DeviceId) -> Result<(), CoreError> {
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
        let disconnect_error = provider
            .disconnect_device(&DeviceId::new("coyote-v3"))
            .await
            .unwrap_err();

        assert!(connect_error.to_string().contains("not configured"));
        assert!(battery_error.to_string().contains("not configured"));
        assert!(disconnect_error.to_string().contains("not configured"));
    }

    #[test]
    fn platform_provider_can_be_split_into_core_adapters() {
        let provider = Arc::new(UnsupportedTauriBlePlatformProvider);
        let discovery: Arc<dyn TauriBleDiscoveryProvider> = provider.clone();
        let transport: Arc<dyn TauriBleTransportProvider> = provider;

        assert!(format!("{discovery:?}").contains("UnsupportedTauriBlePlatformProvider"));
        assert!(format!("{transport:?}").contains("UnsupportedTauriBlePlatformProvider"));
    }

    #[test]
    fn scan_diagnostics_samples_are_limited() {
        let mut diagnostics = TauriBleScanDiagnostics::new(8);

        for index in 0..8 {
            diagnostics.record_unknown(Some(&format!("device-{index}")), &[0x180F]);
        }

        assert_eq!(diagnostics.discovered_peripherals, 8);
        assert_eq!(diagnostics.skipped_unknown_peripherals, 8);
        assert_eq!(diagnostics.skipped_unknown_samples.len(), 5);
    }
}
