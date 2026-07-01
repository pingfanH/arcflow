//! Device sessions built over BLE transport Adapters.

use arcflow_protocol::coyote::{
    v3::{B1Notification, StrengthModes},
    BatteryLevel,
};
use arcflow_wave::CoyoteV3Window;

use crate::{
    BleCharacteristic, BleNotification, BleTransport, CoreError, CoyoteV3CommandBuilder, DeviceId,
    SafetyLimits, COYOTE_V3_STATUS_CHARACTERISTICS,
};

/// Coyote V3 strength status parsed from a notify message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoyoteV3StrengthStatus {
    /// Returned B1 sequence number.
    pub sequence: u8,
    /// Actual channel A strength.
    pub a_strength: u8,
    /// Actual channel B strength.
    pub b_strength: u8,
}

impl From<B1Notification> for CoyoteV3StrengthStatus {
    fn from(notification: B1Notification) -> Self {
        Self {
            sequence: notification.sequence(),
            a_strength: notification.a_strength(),
            b_strength: notification.b_strength(),
        }
    }
}

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

    /// Subscribes to Coyote V3 strength and battery status notifications.
    pub async fn subscribe_coyote_v3(&self) -> Result<(), CoreError> {
        for characteristic in COYOTE_V3_STATUS_CHARACTERISTICS {
            self.transport.subscribe(characteristic).await?;
        }

        Ok(())
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

    /// Stops Coyote V3 output on both channels.
    pub async fn stop_coyote_v3_output(&self, sequence: u8) -> Result<(), CoreError> {
        let write = CoyoteV3CommandBuilder::new(self.safety_limits).build_stop_write(sequence)?;

        self.transport.write(write).await
    }

    /// Parses a Coyote V3 notify payload into channel strength status.
    pub fn parse_coyote_v3_notification(
        &self,
        notification: &BleNotification,
    ) -> Result<Option<CoyoteV3StrengthStatus>, CoreError> {
        if notification.characteristic != BleCharacteristic::CoyoteV3Notify {
            return Ok(None);
        }

        Ok(Some(
            B1Notification::from_bytes(&notification.payload)?.into(),
        ))
    }

    /// Parses a Coyote battery notification into a percentage.
    pub fn parse_coyote_battery_notification(
        &self,
        notification: &BleNotification,
    ) -> Result<Option<u8>, CoreError> {
        if notification.characteristic != BleCharacteristic::CoyoteBattery {
            return Ok(None);
        }

        Ok(Some(
            BatteryLevel::from_bytes(&notification.payload)?.percent(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use arcflow_protocol::coyote::v3::{StrengthMode, StrengthModes};
    use arcflow_wave::{ChannelOutput, ChannelWindow, WavePoint};
    use async_trait::async_trait;

    use super::*;
    use crate::{BleNotification, BleWrite, SafetyLimits};

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
    async fn subscribes_to_coyote_v3_status_notifications() {
        let session = DeviceSession::new(
            DeviceId::new("coyote-v3"),
            FakeTransport::default(),
            SafetyLimits::conservative(),
        );

        session.subscribe_coyote_v3().await.unwrap();

        let subscriptions = session.transport.subscriptions.lock().unwrap();
        assert_eq!(
            *subscriptions,
            vec![
                BleCharacteristic::CoyoteV3Notify,
                BleCharacteristic::CoyoteBattery
            ]
        );
    }

    #[tokio::test]
    async fn stops_coyote_v3_output_through_transport() {
        let session = DeviceSession::new(
            DeviceId::new("coyote-v3"),
            FakeTransport::default(),
            SafetyLimits::conservative(),
        );

        session.stop_coyote_v3_output(2).await.unwrap();

        let writes = session.transport.writes.lock().unwrap();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].characteristic, BleCharacteristic::CoyoteV3Write);
        assert_eq!(writes[0].payload[1], 0x2F);
        assert_eq!(writes[0].payload[11], 0x65);
        assert_eq!(writes[0].payload[19], 0x65);
    }

    #[test]
    fn parses_coyote_v3_strength_notification() {
        let session = DeviceSession::new(
            DeviceId::new("coyote-v3"),
            FakeTransport::default(),
            SafetyLimits::conservative(),
        );

        let status = session
            .parse_coyote_v3_notification(&BleNotification {
                characteristic: BleCharacteristic::CoyoteV3Notify,
                payload: vec![0xB1, 0x01, 0x19, 0x05],
            })
            .unwrap()
            .unwrap();

        assert_eq!(
            status,
            CoyoteV3StrengthStatus {
                sequence: 1,
                a_strength: 25,
                b_strength: 5,
            }
        );
    }

    #[test]
    fn ignores_non_coyote_v3_notification() {
        let session = DeviceSession::new(
            DeviceId::new("coyote-v3"),
            FakeTransport::default(),
            SafetyLimits::conservative(),
        );

        let status = session
            .parse_coyote_v3_notification(&BleNotification {
                characteristic: BleCharacteristic::CoyoteBattery,
                payload: vec![100],
            })
            .unwrap();

        assert_eq!(status, None);
    }

    #[test]
    fn parses_coyote_battery_notification() {
        let session = DeviceSession::new(
            DeviceId::new("coyote-v3"),
            FakeTransport::default(),
            SafetyLimits::conservative(),
        );

        let percent = session
            .parse_coyote_battery_notification(&BleNotification {
                characteristic: BleCharacteristic::CoyoteBattery,
                payload: vec![87],
            })
            .unwrap();

        assert_eq!(percent, Some(87));
    }

    #[test]
    fn ignores_non_battery_notification_for_battery_status() {
        let session = DeviceSession::new(
            DeviceId::new("coyote-v3"),
            FakeTransport::default(),
            SafetyLimits::conservative(),
        );

        let percent = session
            .parse_coyote_battery_notification(&BleNotification {
                characteristic: BleCharacteristic::CoyoteV3Notify,
                payload: vec![0xB1, 0x01, 0x19, 0x05],
            })
            .unwrap();

        assert_eq!(percent, None);
    }
}
