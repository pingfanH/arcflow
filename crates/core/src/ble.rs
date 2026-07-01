//! Bluetooth transport Interface for Rust Core.

use std::fmt;

use async_trait::async_trait;

use crate::{
    BleAdapterStatus, CoreError, DeviceDiscoveryController, DeviceId, DeviceModel,
    DeviceScanResult, DeviceStatus,
};

/// DG-LAB Coyote battery service short UUID.
pub const COYOTE_BATTERY_SERVICE_UUID: u16 = 0x180A;
/// DG-LAB Coyote V2 service short UUID.
pub const COYOTE_V2_SERVICE_UUID: u16 = 0x180B;
/// DG-LAB Coyote V3 service short UUID.
pub const COYOTE_V3_SERVICE_UUID: u16 = 0x180C;
/// Characteristics subscribed after connecting to a Coyote V3 pulse host.
pub const COYOTE_V3_STATUS_CHARACTERISTICS: [BleCharacteristic; 2] = [
    BleCharacteristic::CoyoteV3Notify,
    BleCharacteristic::CoyoteBattery,
];

/// BLE characteristic known to ArcFlow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BleCharacteristic {
    /// Coyote V3 write characteristic, UUID 0x150A under service 0x180C.
    CoyoteV3Write,
    /// Coyote V3 notify characteristic, UUID 0x150B under service 0x180C.
    CoyoteV3Notify,
    /// Coyote battery characteristic, UUID 0x1500 under service 0x180A.
    CoyoteBattery,
    /// Coyote V2 AB strength characteristic, UUID 0x1504 under service 0x180B.
    CoyoteV2PwmAb2,
    /// Coyote V2 A waveform characteristic, UUID 0x1505 under service 0x180B.
    CoyoteV2PwmA34,
    /// Coyote V2 B waveform characteristic, UUID 0x1506 under service 0x180B.
    CoyoteV2PwmB34,
}

impl BleCharacteristic {
    /// Returns the Bluetooth service short UUID containing this characteristic.
    #[must_use]
    pub fn service_short_uuid(self) -> u16 {
        match self {
            Self::CoyoteBattery => COYOTE_BATTERY_SERVICE_UUID,
            Self::CoyoteV2PwmAb2 | Self::CoyoteV2PwmA34 | Self::CoyoteV2PwmB34 => {
                COYOTE_V2_SERVICE_UUID
            }
            Self::CoyoteV3Write | Self::CoyoteV3Notify => COYOTE_V3_SERVICE_UUID,
        }
    }

    /// Returns the characteristic short UUID.
    #[must_use]
    pub fn short_uuid(self) -> u16 {
        match self {
            Self::CoyoteBattery => 0x1500,
            Self::CoyoteV2PwmAb2 => 0x1504,
            Self::CoyoteV2PwmA34 => 0x1505,
            Self::CoyoteV2PwmB34 => 0x1506,
            Self::CoyoteV3Write => 0x150A,
            Self::CoyoteV3Notify => 0x150B,
        }
    }

    /// Returns the canonical 128-bit Bluetooth service UUID string.
    #[must_use]
    pub fn service_uuid(self) -> String {
        bluetooth_base_uuid(self.service_short_uuid())
    }

    /// Returns the canonical 128-bit Bluetooth characteristic UUID string.
    #[must_use]
    pub fn uuid(self) -> String {
        bluetooth_base_uuid(self.short_uuid())
    }
}

/// Converts a 16-bit Bluetooth UUID into the canonical 128-bit UUID string.
#[must_use]
pub fn bluetooth_base_uuid(short_uuid: u16) -> String {
    format!("0000{short_uuid:04x}-0000-1000-8000-00805f9b34fb")
}

/// BLE advertisement discovered by a platform Adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BleAdvertisement {
    /// Platform-stable device id.
    pub device_id: DeviceId,
    /// Advertised local name if present.
    pub local_name: Option<String>,
    /// Received signal strength indicator if the platform exposes it.
    pub rssi: Option<i16>,
    /// Advertised service short UUIDs.
    pub service_uuids: Vec<u16>,
}

impl BleAdvertisement {
    /// Constructs a BLE advertisement.
    #[must_use]
    pub fn new(
        device_id: DeviceId,
        local_name: Option<String>,
        rssi: Option<i16>,
        service_uuids: Vec<u16>,
    ) -> Self {
        Self {
            device_id,
            local_name,
            rssi,
            service_uuids,
        }
    }
}

/// BLE write request produced by Rust Core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BleWrite {
    /// Target characteristic.
    pub characteristic: BleCharacteristic,
    /// Payload bytes.
    pub payload: Vec<u8>,
}

impl BleWrite {
    /// Constructs a BLE write request.
    #[must_use]
    pub fn new(characteristic: BleCharacteristic, payload: impl Into<Vec<u8>>) -> Self {
        Self {
            characteristic,
            payload: payload.into(),
        }
    }
}

/// BLE notification received by Rust Core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BleNotification {
    /// Source characteristic.
    pub characteristic: BleCharacteristic,
    /// Notification payload bytes.
    pub payload: Vec<u8>,
}

/// Asynchronous BLE discovery Adapter seam.
#[async_trait]
pub trait BleDiscovery {
    /// Scans for BLE advertisements.
    async fn scan(&self) -> Result<Vec<BleAdvertisement>, CoreError>;
}

/// Device discovery controller backed by a platform BLE discovery Adapter.
#[derive(Debug, Clone)]
pub struct BleDeviceDiscoveryController<D> {
    discovery: D,
}

impl<D> BleDeviceDiscoveryController<D> {
    /// Constructs a BLE-backed device discovery controller.
    #[must_use]
    pub fn new(discovery: D) -> Self {
        Self { discovery }
    }
}

#[async_trait]
impl<D> DeviceDiscoveryController for BleDeviceDiscoveryController<D>
where
    D: BleDiscovery + fmt::Debug + Send + Sync,
{
    async fn scan_devices(&self) -> Result<DeviceScanResult, CoreError> {
        let devices = self
            .discovery
            .scan()
            .await?
            .into_iter()
            .filter_map(device_status_from_advertisement)
            .collect();

        Ok(DeviceScanResult::new(BleAdapterStatus::Ready, devices))
    }
}

fn device_status_from_advertisement(advertisement: BleAdvertisement) -> Option<DeviceStatus> {
    coyote_model_from_services(&advertisement.service_uuids).map(|model| DeviceStatus {
        id: advertisement.device_id,
        model,
        battery_percent: None,
        connected: true,
    })
}

fn coyote_model_from_services(service_uuids: &[u16]) -> Option<DeviceModel> {
    if service_uuids.contains(&COYOTE_V3_SERVICE_UUID) {
        return Some(DeviceModel::CoyoteV3);
    }

    if service_uuids.contains(&COYOTE_V2_SERVICE_UUID) {
        return Some(DeviceModel::CoyoteV2);
    }

    None
}

/// Asynchronous BLE transport Adapter seam.
///
/// Platform-specific desktop and mobile implementations must satisfy this
/// Interface. React and plugins never call it directly.
#[async_trait]
pub trait BleTransport {
    /// Write bytes to a known BLE characteristic.
    async fn write(&self, write: BleWrite) -> Result<(), CoreError>;

    /// Enable notifications for a known BLE characteristic.
    async fn subscribe(&self, characteristic: BleCharacteristic) -> Result<(), CoreError>;
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::*;

    #[test]
    fn exposes_coyote_characteristic_metadata() {
        assert_eq!(
            BleCharacteristic::CoyoteV3Write.service_short_uuid(),
            0x180C
        );
        assert_eq!(BleCharacteristic::CoyoteV3Write.short_uuid(), 0x150A);
        assert_eq!(
            BleCharacteristic::CoyoteV3Write.service_uuid(),
            "0000180c-0000-1000-8000-00805f9b34fb"
        );
        assert_eq!(
            BleCharacteristic::CoyoteV3Write.uuid(),
            "0000150a-0000-1000-8000-00805f9b34fb"
        );
        assert_eq!(
            BleCharacteristic::CoyoteV2PwmA34.service_short_uuid(),
            0x180B
        );
        assert_eq!(
            BleCharacteristic::CoyoteBattery.service_short_uuid(),
            0x180A
        );
        assert_eq!(
            COYOTE_V3_STATUS_CHARACTERISTICS,
            [
                BleCharacteristic::CoyoteV3Notify,
                BleCharacteristic::CoyoteBattery
            ]
        );
    }

    #[test]
    fn expands_bluetooth_base_uuids() {
        assert_eq!(
            bluetooth_base_uuid(0x180A),
            "0000180a-0000-1000-8000-00805f9b34fb"
        );
    }

    #[test]
    fn constructs_ble_advertisement() {
        let advertisement = BleAdvertisement::new(
            DeviceId::new("device-1"),
            Some("Coyote".to_owned()),
            Some(-42),
            vec![COYOTE_V3_SERVICE_UUID],
        );

        assert_eq!(advertisement.device_id, DeviceId::new("device-1"));
        assert_eq!(advertisement.local_name, Some("Coyote".to_owned()));
        assert_eq!(advertisement.rssi, Some(-42));
        assert_eq!(advertisement.service_uuids, vec![COYOTE_V3_SERVICE_UUID]);
    }

    #[derive(Debug)]
    struct FakeDiscovery {
        advertisements: Vec<BleAdvertisement>,
    }

    #[async_trait]
    impl BleDiscovery for FakeDiscovery {
        async fn scan(&self) -> Result<Vec<BleAdvertisement>, CoreError> {
            Ok(self.advertisements.clone())
        }
    }

    #[tokio::test]
    async fn discovery_trait_returns_advertisements() {
        let advertisements = FakeDiscovery {
            advertisements: vec![BleAdvertisement::new(
                DeviceId::new("device-1"),
                Some("Coyote".to_owned()),
                None,
                vec![COYOTE_V3_SERVICE_UUID],
            )],
        }
        .scan()
        .await
        .unwrap();

        assert_eq!(advertisements.len(), 1);
        assert_eq!(
            advertisements[0].service_uuids,
            vec![COYOTE_V3_SERVICE_UUID]
        );
    }

    #[tokio::test]
    async fn ble_device_discovery_maps_coyote_advertisements() {
        let controller = BleDeviceDiscoveryController::new(FakeDiscovery {
            advertisements: vec![
                BleAdvertisement::new(
                    DeviceId::new("v3"),
                    Some("Coyote".to_owned()),
                    Some(-41),
                    vec![COYOTE_V3_SERVICE_UUID],
                ),
                BleAdvertisement::new(
                    DeviceId::new("v2"),
                    Some("Coyote 2".to_owned()),
                    Some(-50),
                    vec![COYOTE_V2_SERVICE_UUID],
                ),
                BleAdvertisement::new(
                    DeviceId::new("unknown"),
                    Some("Other".to_owned()),
                    Some(-60),
                    vec![0xFFFF],
                ),
            ],
        });

        let scan = controller.scan_devices().await.unwrap();

        assert_eq!(scan.adapter_status, BleAdapterStatus::Ready);
        assert_eq!(
            scan.devices,
            vec![
                DeviceStatus {
                    id: DeviceId::new("v3"),
                    model: DeviceModel::CoyoteV3,
                    battery_percent: None,
                    connected: true,
                },
                DeviceStatus {
                    id: DeviceId::new("v2"),
                    model: DeviceModel::CoyoteV2,
                    battery_percent: None,
                    connected: true,
                },
            ]
        );
    }

    #[tokio::test]
    async fn ble_device_discovery_prefers_v3_when_services_overlap() {
        let controller = BleDeviceDiscoveryController::new(FakeDiscovery {
            advertisements: vec![BleAdvertisement::new(
                DeviceId::new("hybrid"),
                None,
                None,
                vec![COYOTE_V2_SERVICE_UUID, COYOTE_V3_SERVICE_UUID],
            )],
        });

        let scan = controller.scan_devices().await.unwrap();

        assert_eq!(scan.devices[0].model, DeviceModel::CoyoteV3);
    }
}
