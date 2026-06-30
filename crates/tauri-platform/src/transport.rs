//! Tauri 2 BLE transport adapter scaffold.

use std::{fmt, sync::Arc};

use arcflow_core::{
    BleCharacteristic, BleTransport, BleWrite, CoreError, DeviceBleOutputSink, DeviceId,
};
use async_trait::async_trait;
use tokio::sync::mpsc;

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

/// Output sink that queues Core BLE writes onto an async Tauri BLE transport.
#[derive(Clone)]
pub struct TauriBleOutputSink {
    sender: mpsc::UnboundedSender<TauriBleOutputCommand>,
}

impl TauriBleOutputSink {
    /// Spawns an output sink worker.
    #[must_use]
    pub fn spawn(transport: Arc<dyn BleTransport + Send + Sync>) -> Self {
        Self::spawn_with_events(transport, None)
    }

    /// Spawns an output sink worker with an optional event sink for tests.
    #[must_use]
    pub fn spawn_with_event_sink(
        transport: Arc<dyn BleTransport + Send + Sync>,
        event_sink: mpsc::UnboundedSender<TauriBleOutputEvent>,
    ) -> Self {
        Self::spawn_with_events(transport, Some(event_sink))
    }

    fn spawn_with_events(
        transport: Arc<dyn BleTransport + Send + Sync>,
        event_sink: Option<mpsc::UnboundedSender<TauriBleOutputEvent>>,
    ) -> Self {
        let (sender, mut receiver) = mpsc::unbounded_channel::<TauriBleOutputCommand>();

        tokio::spawn(async move {
            while let Some(command) = receiver.recv().await {
                let event = match transport.write(command.write.clone()).await {
                    Ok(()) => TauriBleOutputEvent::Written {
                        device_id: command.device_id,
                        characteristic: command.write.characteristic,
                    },
                    Err(error) => TauriBleOutputEvent::Failed {
                        device_id: command.device_id,
                        error: error.to_string(),
                    },
                };

                if let Some(event_sink) = &event_sink {
                    let _ = event_sink.send(event);
                }
            }
        });

        Self { sender }
    }
}

impl fmt::Debug for TauriBleOutputSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TauriBleOutputSink").finish_non_exhaustive()
    }
}

impl DeviceBleOutputSink for TauriBleOutputSink {
    fn write(&self, device_id: &DeviceId, write: BleWrite) -> Result<(), CoreError> {
        self.sender
            .send(TauriBleOutputCommand {
                device_id: device_id.clone(),
                write,
            })
            .map_err(|_| CoreError::Transport("tauri BLE output sink is closed".to_owned()))
    }
}

/// Command queued from Core output controllers to the Tauri transport worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TauriBleOutputCommand {
    /// Target device id.
    pub device_id: DeviceId,
    /// BLE write payload.
    pub write: BleWrite,
}

/// Event emitted by the Tauri output sink worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TauriBleOutputEvent {
    /// Write was accepted by the transport provider.
    Written {
        /// Target device id.
        device_id: DeviceId,
        /// Target characteristic.
        characteristic: BleCharacteristic,
    },
    /// Write failed in the transport provider.
    Failed {
        /// Target device id.
        device_id: DeviceId,
        /// Rendered error message.
        error: String,
    },
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use tokio::time::{timeout, Duration};

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

    #[tokio::test]
    async fn output_sink_queues_writes_to_transport() {
        let provider = Arc::new(RecordingTransportProvider::default());
        let transport = Arc::new(TauriBleTransport::with_provider(provider.clone()));
        let (event_sender, mut events) = mpsc::unbounded_channel();
        let sink = TauriBleOutputSink::spawn_with_event_sink(transport, event_sender);

        sink.write(
            &DeviceId::new("coyote-v3"),
            BleWrite::new(BleCharacteristic::CoyoteV3Write, [0xB0]),
        )
        .unwrap();

        let event = timeout(Duration::from_secs(1), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            event,
            TauriBleOutputEvent::Written {
                device_id: DeviceId::new("coyote-v3"),
                characteristic: BleCharacteristic::CoyoteV3Write,
            }
        );
        assert_eq!(provider.writes.lock().unwrap().len(), 1);
    }
}
