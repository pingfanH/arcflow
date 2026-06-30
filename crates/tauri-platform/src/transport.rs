//! Tauri 2 BLE transport adapter scaffold.

use std::{fmt, sync::Arc};

use arcflow_core::{BleCharacteristic, BleTransport, BleWrite, CoreError};
use async_trait::async_trait;

/// Provider used by the Tauri BLE transport to perform platform writes.
#[async_trait]
pub trait TauriBleTransportProvider: fmt::Debug + Send + Sync {
    /// Writes bytes to a platform BLE characteristic.
    async fn write(&self, write: BleWrite) -> Result<(), CoreError>;

    /// Subscribes to notifications for a platform BLE characteristic.
    async fn subscribe(&self, characteristic: BleCharacteristic) -> Result<(), CoreError>;
}

/// Provider used until real platform BLE transport is attached.
#[derive(Debug, Default)]
pub struct UnsupportedTauriBleTransportProvider;

#[async_trait]
impl TauriBleTransportProvider for UnsupportedTauriBleTransportProvider {
    async fn write(&self, _write: BleWrite) -> Result<(), CoreError> {
        Err(CoreError::Transport(
            "tauri BLE transport is not configured".to_owned(),
        ))
    }

    async fn subscribe(&self, _characteristic: BleCharacteristic) -> Result<(), CoreError> {
        Err(CoreError::Transport(
            "tauri BLE transport is not configured".to_owned(),
        ))
    }
}

/// BLE transport used by Tauri 2 desktop and mobile shells.
#[derive(Debug, Clone)]
pub struct TauriBleTransport {
    provider: Arc<dyn TauriBleTransportProvider>,
}

impl TauriBleTransport {
    /// Constructs an unsupported Tauri BLE transport.
    #[must_use]
    pub fn unsupported() -> Self {
        Self::with_provider(Arc::new(UnsupportedTauriBleTransportProvider))
    }

    /// Constructs a Tauri BLE transport with an explicit provider.
    #[must_use]
    pub fn with_provider(provider: Arc<dyn TauriBleTransportProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl BleTransport for TauriBleTransport {
    async fn write(&self, write: BleWrite) -> Result<(), CoreError> {
        self.provider.write(write).await
    }

    async fn subscribe(&self, characteristic: BleCharacteristic) -> Result<(), CoreError> {
        self.provider.subscribe(characteristic).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Debug, Default)]
    struct RecordingTransportProvider {
        writes: Mutex<Vec<BleWrite>>,
        subscriptions: Mutex<Vec<BleCharacteristic>>,
    }

    #[async_trait]
    impl TauriBleTransportProvider for RecordingTransportProvider {
        async fn write(&self, write: BleWrite) -> Result<(), CoreError> {
            self.writes.lock().unwrap().push(write);
            Ok(())
        }

        async fn subscribe(&self, characteristic: BleCharacteristic) -> Result<(), CoreError> {
            self.subscriptions.lock().unwrap().push(characteristic);
            Ok(())
        }
    }

    #[tokio::test]
    async fn unsupported_transport_rejects_operations() {
        let transport = TauriBleTransport::unsupported();

        let write_error = transport
            .write(BleWrite::new(BleCharacteristic::CoyoteV3Write, [0xBF]))
            .await
            .unwrap_err();
        let subscribe_error = transport
            .subscribe(BleCharacteristic::CoyoteV3Notify)
            .await
            .unwrap_err();

        assert!(write_error.to_string().contains("not configured"));
        assert!(subscribe_error.to_string().contains("not configured"));
    }

    #[tokio::test]
    async fn transport_delegates_to_provider() {
        let provider = Arc::new(RecordingTransportProvider::default());
        let transport = TauriBleTransport::with_provider(provider.clone());

        transport
            .write(BleWrite::new(BleCharacteristic::CoyoteV3Write, [0xB0]))
            .await
            .unwrap();
        transport
            .subscribe(BleCharacteristic::CoyoteV3Notify)
            .await
            .unwrap();

        assert_eq!(provider.writes.lock().unwrap().len(), 1);
        assert_eq!(
            *provider.subscriptions.lock().unwrap(),
            vec![BleCharacteristic::CoyoteV3Notify]
        );
    }
}
