//! Device sessions built over BLE transport Adapters.

use arcflow_protocol::coyote::v3::StrengthModes;
use arcflow_wave::CoyoteV3Window;

use crate::{
    BleCharacteristic, BleTransport, CoreError, CoyoteV3CommandBuilder, DeviceId, SafetyLimits,
};

/// Runtime session for one connected device.
#[derive(Debug)]
pub struct DeviceSession<T> {
    device_id: DeviceId,
    transport: T,
    safety_limits: SafetyLimits,
}

impl<T> DeviceSession<T>
where
    T: BleTransport,
{
    /// Constructs a device session over a BLE transport Adapter.
    #[must_use]
    pub fn new(device_id: DeviceId, transport: T, safety_limits: SafetyLimits) -> Self {
        Self {
            device_id,
            transport,
            safety_limits,
        }
    }

    /// Returns the device id for this session.
    #[must_use]
    pub fn device_id(&self) -> &DeviceId {
        &self.device_id
    }

    /// Returns the active safety limits.
    #[must_use]
    pub fn safety_limits(&self) -> SafetyLimits {
        self.safety_limits
    }

    /// Subscribes to Coyote V3 notify messages.
    pub async fn subscribe_coyote_v3(&self) -> Result<(), CoreError> {
        self.transport
            .subscribe(BleCharacteristic::CoyoteV3Notify)
            .await
    }

    /// Writes one Coyote V3 wave window to the device.
    pub async fn write_coyote_v3_window(
        &self,
        window: CoyoteV3Window,
        sequence: u8,
        modes: StrengthModes,
        a_strength: u8,
        b_strength: u8,
    ) -> Result<(), CoreError> {
        let write = CoyoteV3CommandBuilder::new(self.safety_limits)
            .build_wave_write(window, sequence, modes, a_strength, b_strength)?;

        self.transport.write(write).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use arcflow_protocol::coyote::v3::{StrengthMode, StrengthModes};
    use arcflow_wave::{ChannelOutput, ChannelWindow, WavePoint};
    use async_trait::async_trait;

    use super::*;
    use crate::{BleWrite, SafetyLimits};

    #[derive(Debug, Default)]
    struct FakeTransport {
        writes: Mutex<Vec<BleWrite>>,
        subscriptions: Mutex<Vec<BleCharacteristic>>,
    }

    #[async_trait]
    impl BleTransport for FakeTransport {
        async fn write(&self, write: BleWrite) -> Result<(), CoreError> {
            self.writes.lock().unwrap().push(write);
            Ok(())
        }

        async fn subscribe(&self, characteristic: BleCharacteristic) -> Result<(), CoreError> {
            self.subscriptions.lock().unwrap().push(characteristic);
            Ok(())
        }
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

    #[tokio::test]
    async fn writes_coyote_v3_window_through_transport() {
        let session = DeviceSession::new(
            DeviceId::new("coyote-v3"),
            FakeTransport::default(),
            SafetyLimits::conservative(),
        );

        session
            .write_coyote_v3_window(
                safe_window(),
                0,
                StrengthModes::new(StrengthMode::Unchanged, StrengthMode::Unchanged),
                0,
                0,
            )
            .await
            .unwrap();

        let writes = session.transport.writes.lock().unwrap();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].characteristic, BleCharacteristic::CoyoteV3Write);
    }

    #[tokio::test]
    async fn subscribes_to_coyote_v3_notify() {
        let session = DeviceSession::new(
            DeviceId::new("coyote-v3"),
            FakeTransport::default(),
            SafetyLimits::conservative(),
        );

        session.subscribe_coyote_v3().await.unwrap();

        let subscriptions = session.transport.subscriptions.lock().unwrap();
        assert_eq!(*subscriptions, vec![BleCharacteristic::CoyoteV3Notify]);
    }
}
