//! Native desktop BLE provider backed by `btleplug`.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};

use arcflow_core::{
    bluetooth_base_uuid, BleAdvertisement, BleCharacteristic, CoreError, DeviceId,
    COYOTE_BATTERY_SERVICE_UUID, COYOTE_V2_SERVICE_UUID, COYOTE_V3_SERVICE_UUID,
};
use arcflow_protocol::coyote::{v3::B1Notification, BatteryLevel, V2_DEVICE_NAME, V3_DEVICE_NAME};
use async_trait::async_trait;
use btleplug::{
    api::{
        Central, Characteristic, Manager as _, Peripheral as _, ScanFilter, ValueNotification,
        WriteType,
    },
    platform::{Adapter, Manager, Peripheral},
};
use futures_util::StreamExt;
use tokio::{sync::Mutex, task::JoinHandle, time::sleep};
use uuid::Uuid;

use crate::{
    TauriBleConnectionState, TauriBleDiscoveryProvider, TauriBleDiscoveryState,
    TauriBlePlatformProvider, TauriBleScanDiagnostics, TauriBleSubscriptionRequest,
    TauriBleTransportProvider, TauriBleWriteRequest,
};

const DEFAULT_SCAN_DURATION: Duration = Duration::from_secs(5);
const PROPERTIES_RETRY_DELAY: Duration = Duration::from_millis(200);
const PROPERTIES_RETRY_ATTEMPTS: usize = 3;

#[derive(Debug)]
struct NativeBleAdvertisementScan {
    advertisements: Vec<BleAdvertisement>,
    diagnostics: TauriBleScanDiagnostics,
}

#[derive(Debug, Clone, Default)]
struct NativeBleNotificationStatus {
    battery_percent: Option<u8>,
    channel_a_strength: Option<u8>,
    channel_b_strength: Option<u8>,
}

/// Desktop BLE provider used by Tauri shells on platforms supported by `btleplug`.
#[derive(Debug)]
pub struct NativeBlePlatformProvider {
    scan_duration: Duration,
    connections: Mutex<HashMap<DeviceId, Peripheral>>,
    notification_tasks: Mutex<HashMap<DeviceId, JoinHandle<()>>>,
    notification_status: Arc<StdMutex<HashMap<DeviceId, NativeBleNotificationStatus>>>,
    scan_diagnostics: StdMutex<Option<TauriBleScanDiagnostics>>,
}

impl NativeBlePlatformProvider {
    /// Constructs a native BLE provider with a default short scan window.
    #[must_use]
    pub fn new() -> Self {
        Self {
            scan_duration: DEFAULT_SCAN_DURATION,
            connections: Mutex::new(HashMap::new()),
            notification_tasks: Mutex::new(HashMap::new()),
            notification_status: Arc::new(StdMutex::new(HashMap::new())),
            scan_diagnostics: StdMutex::new(None),
        }
    }

    /// Constructs a native BLE provider with an explicit scan duration.
    #[must_use]
    pub fn with_scan_duration(scan_duration: Duration) -> Self {
        Self {
            scan_duration,
            connections: Mutex::new(HashMap::new()),
            notification_tasks: Mutex::new(HashMap::new()),
            notification_status: Arc::new(StdMutex::new(HashMap::new())),
            scan_diagnostics: StdMutex::new(None),
        }
    }

    async fn adapter(&self) -> Result<Adapter, CoreError> {
        let manager = Manager::new().await.map_err(transport_error)?;
        let adapters = manager.adapters().await.map_err(transport_error)?;
        adapters
            .into_iter()
            .next()
            .ok_or_else(|| CoreError::Transport("no BLE adapter is available".to_owned()))
    }

    async fn find_peripheral(&self, device_id: &DeviceId) -> Result<Peripheral, CoreError> {
        if let Some(peripheral) = self.connections.lock().await.get(device_id).cloned() {
            return Ok(peripheral);
        }

        let adapter = self.adapter().await?;
        for peripheral in self.scan_peripherals(&adapter).await? {
            if peripheral.id().to_string() == device_id.as_str() {
                return Ok(peripheral);
            }
        }
        for peripheral in self.scan_coyote_service_peripherals(&adapter).await? {
            if peripheral.id().to_string() == device_id.as_str() {
                return Ok(peripheral);
            }
        }

        Err(CoreError::Transport(format!(
            "BLE device `{}` was not found",
            device_id.as_str()
        )))
    }

    async fn scan_peripherals(&self, adapter: &Adapter) -> Result<Vec<Peripheral>, CoreError> {
        self.scan_peripherals_with_filter(adapter, ScanFilter::default())
            .await
    }

    async fn scan_coyote_service_peripherals(
        &self,
        adapter: &Adapter,
    ) -> Result<Vec<Peripheral>, CoreError> {
        self.scan_peripherals_with_filter(adapter, coyote_scan_filter())
            .await
    }

    async fn scan_peripherals_with_filter(
        &self,
        adapter: &Adapter,
        filter: ScanFilter,
    ) -> Result<Vec<Peripheral>, CoreError> {
        adapter.start_scan(filter).await.map_err(transport_error)?;
        sleep(self.scan_duration).await;

        let peripherals = adapter.peripherals().await.map_err(transport_error);
        let stop_result = adapter.stop_scan().await.map_err(transport_error);

        match (peripherals, stop_result) {
            (Ok(peripherals), Ok(())) => Ok(peripherals),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }

    async fn scan_advertisements(
        &self,
        peripherals: Vec<Peripheral>,
    ) -> Result<NativeBleAdvertisementScan, CoreError> {
        let mut advertisements = Vec::new();
        let mut diagnostics = TauriBleScanDiagnostics::new(peripherals.len());

        for peripheral in peripherals {
            let Some(properties) = read_properties_with_retry(&peripheral).await? else {
                diagnostics.skipped_missing_properties += 1;
                continue;
            };
            diagnostics.inspected_peripherals += 1;
            let service_uuids = properties
                .services
                .iter()
                .filter_map(short_uuid)
                .collect::<Vec<_>>();
            let Some(service_uuids) =
                coyote_service_uuids(service_uuids.clone(), properties.local_name.as_deref())
            else {
                diagnostics.record_unknown(properties.local_name.as_deref(), &service_uuids);
                continue;
            };
            diagnostics.record_matched(properties.local_name.as_deref(), &service_uuids);

            let device_id = DeviceId::new(peripheral.id().to_string());
            let connected = peripheral.is_connected().await.unwrap_or(false);
            let battery_percent = if connected {
                peripheral
                    .discover_services()
                    .await
                    .map_err(transport_error)?;
                self.read_battery_from_peripheral(&peripheral).await?
            } else {
                None
            };
            let notification_status = self.notification_status_for_device(&device_id);

            advertisements.push(
                BleAdvertisement::new(
                    device_id,
                    properties.local_name,
                    properties.rssi,
                    service_uuids,
                )
                .with_connected(connected)
                .with_battery_percent(battery_percent.or(notification_status.battery_percent))
                .with_channel_strengths(
                    notification_status.channel_a_strength,
                    notification_status.channel_b_strength,
                ),
            );
        }

        Ok(NativeBleAdvertisementScan {
            advertisements,
            diagnostics,
        })
    }

    fn notification_status_for_device(&self, device_id: &DeviceId) -> NativeBleNotificationStatus {
        self.notification_status
            .lock()
            .expect("native BLE notification status mutex poisoned")
            .get(device_id)
            .cloned()
            .unwrap_or_default()
    }

    async fn ensure_connected(&self, device_id: &DeviceId) -> Result<Peripheral, CoreError> {
        let peripheral = self.find_peripheral(device_id).await?;
        if !peripheral.is_connected().await.map_err(transport_error)? {
            peripheral.connect().await.map_err(transport_error)?;
        }
        peripheral
            .discover_services()
            .await
            .map_err(transport_error)?;
        self.connections
            .lock()
            .await
            .insert(device_id.clone(), peripheral.clone());
        Ok(peripheral)
    }

    async fn read_battery_from_peripheral(
        &self,
        peripheral: &Peripheral,
    ) -> Result<Option<u8>, CoreError> {
        let Some(characteristic) =
            find_characteristic(peripheral, BleCharacteristic::CoyoteBattery)
        else {
            return Ok(None);
        };

        let payload = peripheral
            .read(&characteristic)
            .await
            .map_err(transport_error)?;
        Ok(payload.first().copied().filter(|percent| *percent <= 100))
    }

    async fn subscribe_if_present(
        &self,
        peripheral: &Peripheral,
        characteristic: BleCharacteristic,
    ) -> Result<(), CoreError> {
        let Some(characteristic) = find_characteristic(peripheral, characteristic) else {
            return Ok(());
        };

        peripheral
            .subscribe(&characteristic)
            .await
            .map_err(transport_error)
    }

    async fn ensure_notification_listener(
        &self,
        device_id: DeviceId,
        peripheral: Peripheral,
    ) -> Result<(), CoreError> {
        let mut tasks = self.notification_tasks.lock().await;
        if tasks
            .get(&device_id)
            .is_some_and(|handle| !handle.is_finished())
        {
            return Ok(());
        }

        if let Some(handle) = tasks.remove(&device_id) {
            handle.abort();
        }

        let mut notifications = peripheral.notifications().await.map_err(transport_error)?;
        let status_cache = Arc::clone(&self.notification_status);
        let task_device_id = device_id.clone();
        let handle = tokio::spawn(async move {
            while let Some(notification) = notifications.next().await {
                update_notification_status(&status_cache, &task_device_id, notification);
            }
        });

        tasks.insert(device_id, handle);
        Ok(())
    }

    async fn stop_notification_listener(&self, device_id: &DeviceId) {
        if let Some(handle) = self.notification_tasks.lock().await.remove(device_id) {
            handle.abort();
        }

        self.notification_status
            .lock()
            .expect("native BLE notification status mutex poisoned")
            .remove(device_id);
    }
}

impl Default for NativeBlePlatformProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TauriBleDiscoveryProvider for NativeBlePlatformProvider {
    async fn scan_state(&self) -> Result<TauriBleDiscoveryState, CoreError> {
        let adapter = match self.adapter().await {
            Ok(adapter) => adapter,
            Err(error) if is_powered_off_error(&error) => {
                return Ok(TauriBleDiscoveryState::PoweredOff)
            }
            Err(error) if is_permission_denied_error(&error) => {
                return Ok(TauriBleDiscoveryState::PermissionDenied);
            }
            Err(error) => return Err(error),
        };

        let peripherals = match self.scan_peripherals(&adapter).await {
            Ok(peripherals) => peripherals,
            Err(error) if is_powered_off_error(&error) => {
                return Ok(TauriBleDiscoveryState::PoweredOff);
            }
            Err(error) if is_permission_denied_error(&error) => {
                return Ok(TauriBleDiscoveryState::PermissionDenied);
            }
            Err(error) => return Err(error),
        };
        let mut scan = self.scan_advertisements(peripherals).await?;

        if scan.advertisements.is_empty() {
            let fallback_peripherals = match self.scan_coyote_service_peripherals(&adapter).await {
                Ok(peripherals) => peripherals,
                Err(error) if is_powered_off_error(&error) => {
                    return Ok(TauriBleDiscoveryState::PoweredOff);
                }
                Err(error) if is_permission_denied_error(&error) => {
                    return Ok(TauriBleDiscoveryState::PermissionDenied);
                }
                Err(error) => return Err(error),
            };
            let fallback_scan = self.scan_advertisements(fallback_peripherals).await?;
            scan.diagnostics.merge(fallback_scan.diagnostics);
            scan.advertisements = fallback_scan.advertisements;
        }

        *self
            .scan_diagnostics
            .lock()
            .expect("native BLE scan diagnostics mutex poisoned") = Some(scan.diagnostics);

        Ok(TauriBleDiscoveryState::Ready {
            advertisements: scan.advertisements,
        })
    }
}

#[async_trait]
impl TauriBleTransportProvider for NativeBlePlatformProvider {
    async fn write(&self, request: TauriBleWriteRequest) -> Result<(), CoreError> {
        let peripheral = self.ensure_connected(&request.device_id).await?;
        let characteristic = find_characteristic(&peripheral, request.write.characteristic)
            .ok_or_else(|| missing_characteristic_error(request.write.characteristic))?;

        peripheral
            .write(
                &characteristic,
                &request.write.payload,
                WriteType::WithoutResponse,
            )
            .await
            .map_err(transport_error)
    }

    async fn subscribe(&self, request: TauriBleSubscriptionRequest) -> Result<(), CoreError> {
        let peripheral = self.ensure_connected(&request.device_id).await?;
        let characteristic = find_characteristic(&peripheral, request.characteristic)
            .ok_or_else(|| missing_characteristic_error(request.characteristic))?;

        peripheral
            .subscribe(&characteristic)
            .await
            .map_err(transport_error)
    }
}

#[async_trait]
impl TauriBlePlatformProvider for NativeBlePlatformProvider {
    async fn connect_device(
        &self,
        device_id: DeviceId,
    ) -> Result<TauriBleConnectionState, CoreError> {
        let peripheral = self.ensure_connected(&device_id).await?;
        self.subscribe_if_present(&peripheral, BleCharacteristic::CoyoteV3Notify)
            .await?;
        self.subscribe_if_present(&peripheral, BleCharacteristic::CoyoteBattery)
            .await?;
        self.ensure_notification_listener(device_id.clone(), peripheral.clone())
            .await?;
        let battery_percent = self.read_battery_from_peripheral(&peripheral).await?;

        Ok(TauriBleConnectionState::new(device_id, battery_percent))
    }

    async fn disconnect_device(&self, device_id: &DeviceId) -> Result<(), CoreError> {
        self.stop_notification_listener(device_id).await;
        let peripheral = self.connections.lock().await.remove(device_id);

        if let Some(peripheral) = peripheral {
            if peripheral.is_connected().await.map_err(transport_error)? {
                peripheral.disconnect().await.map_err(transport_error)?;
            }
        }

        Ok(())
    }

    async fn scan_diagnostics(&self) -> Option<TauriBleScanDiagnostics> {
        self.scan_diagnostics
            .lock()
            .expect("native BLE scan diagnostics mutex poisoned")
            .clone()
    }

    async fn read_battery_percent(&self, device_id: &DeviceId) -> Result<Option<u8>, CoreError> {
        let peripheral = self.ensure_connected(device_id).await?;
        let battery_percent = self.read_battery_from_peripheral(&peripheral).await?;
        Ok(battery_percent.or_else(|| {
            self.notification_status_for_device(device_id)
                .battery_percent
        }))
    }
}

fn find_characteristic(
    peripheral: &Peripheral,
    characteristic: BleCharacteristic,
) -> Option<Characteristic> {
    let uuid = Uuid::parse_str(&characteristic.uuid()).ok()?;
    peripheral
        .characteristics()
        .into_iter()
        .find(|candidate| candidate.uuid == uuid)
}

fn update_notification_status(
    status_cache: &StdMutex<HashMap<DeviceId, NativeBleNotificationStatus>>,
    device_id: &DeviceId,
    notification: ValueNotification,
) {
    let Some(characteristic) = ble_characteristic_from_uuid(notification.uuid) else {
        return;
    };

    let mut statuses = status_cache
        .lock()
        .expect("native BLE notification status mutex poisoned");
    let status = statuses.entry(device_id.clone()).or_default();

    match characteristic {
        BleCharacteristic::CoyoteV3Notify => {
            if let Ok(notification) = B1Notification::from_bytes(&notification.value) {
                status.channel_a_strength = Some(notification.a_strength());
                status.channel_b_strength = Some(notification.b_strength());
            }
        }
        BleCharacteristic::CoyoteBattery => {
            if let Ok(battery) = BatteryLevel::from_bytes(&notification.value) {
                status.battery_percent = Some(battery.percent());
            }
        }
        _ => {}
    }
}

fn ble_characteristic_from_uuid(uuid: Uuid) -> Option<BleCharacteristic> {
    [
        BleCharacteristic::CoyoteV3Write,
        BleCharacteristic::CoyoteV3Notify,
        BleCharacteristic::CoyoteBattery,
        BleCharacteristic::CoyoteV2PwmAb2,
        BleCharacteristic::CoyoteV2PwmA34,
        BleCharacteristic::CoyoteV2PwmB34,
    ]
    .into_iter()
    .find(|characteristic| {
        Uuid::parse_str(&characteristic.uuid())
            .is_ok_and(|characteristic_uuid| characteristic_uuid == uuid)
    })
}

fn short_uuid(uuid: &Uuid) -> Option<u16> {
    let value = uuid.to_string();
    let Some(hex) = value
        .strip_prefix("0000")
        .and_then(|rest| rest.strip_suffix("-0000-1000-8000-00805f9b34fb"))
    else {
        return None;
    };

    u16::from_str_radix(hex, 16).ok()
}

fn coyote_scan_filter() -> ScanFilter {
    ScanFilter {
        services: coyote_service_filter_uuids(),
    }
}

fn coyote_service_filter_uuids() -> Vec<Uuid> {
    [
        COYOTE_BATTERY_SERVICE_UUID,
        COYOTE_V2_SERVICE_UUID,
        COYOTE_V3_SERVICE_UUID,
    ]
    .into_iter()
    .map(bluetooth_service_uuid)
    .collect()
}

async fn read_properties_with_retry(
    peripheral: &Peripheral,
) -> Result<Option<btleplug::api::PeripheralProperties>, CoreError> {
    for attempt in 0..PROPERTIES_RETRY_ATTEMPTS {
        let properties = peripheral.properties().await.map_err(transport_error)?;
        if properties.is_some() || attempt + 1 == PROPERTIES_RETRY_ATTEMPTS {
            return Ok(properties);
        }

        sleep(PROPERTIES_RETRY_DELAY).await;
    }

    Ok(None)
}

fn bluetooth_service_uuid(short_uuid: u16) -> Uuid {
    Uuid::parse_str(&bluetooth_base_uuid(short_uuid))
        .expect("core Bluetooth service UUID formatting must be valid")
}

fn coyote_service_uuids(mut service_uuids: Vec<u16>, local_name: Option<&str>) -> Option<Vec<u16>> {
    if service_uuids.contains(&COYOTE_V2_SERVICE_UUID)
        || service_uuids.contains(&COYOTE_V3_SERVICE_UUID)
    {
        return Some(service_uuids);
    }

    let service_uuid = coyote_service_uuid_from_local_name(local_name?)?;
    service_uuids.push(service_uuid);
    Some(service_uuids)
}

fn coyote_service_uuid_from_local_name(local_name: &str) -> Option<u16> {
    let name = local_name.trim();
    let uppercase_name = name.to_ascii_uppercase();
    let compact_name = uppercase_name
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect::<String>();

    if uppercase_name == V3_DEVICE_NAME
        || uppercase_name.starts_with("47L121")
        || compact_name == "COYOTE"
        || compact_name == "COYOTEV3"
        || compact_name == "COYOTE3"
    {
        return Some(COYOTE_V3_SERVICE_UUID);
    }

    if uppercase_name == V2_DEVICE_NAME || compact_name == "COYOTEV2" || compact_name == "COYOTE2" {
        return Some(COYOTE_V2_SERVICE_UUID);
    }

    if name.contains("郊狼") {
        return Some(COYOTE_V3_SERVICE_UUID);
    }

    None
}

fn transport_error(error: impl std::fmt::Display) -> CoreError {
    CoreError::Transport(error.to_string())
}

fn is_powered_off_error(error: &CoreError) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("no ble adapter")
        || message.contains("powered off")
        || message.contains("bluetooth is off")
}

fn is_permission_denied_error(error: &CoreError) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("permission")
        || message.contains("unauthorized")
        || message.contains("not authorized")
        || message.contains("denied")
        || message.contains("not permitted")
        || message.contains("entitlement")
}

fn missing_characteristic_error(characteristic: BleCharacteristic) -> CoreError {
    CoreError::Transport(format!(
        "BLE characteristic {} is not available",
        characteristic.uuid()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_short_bluetooth_uuid() {
        let uuid = Uuid::parse_str("0000180c-0000-1000-8000-00805f9b34fb").unwrap();

        assert_eq!(short_uuid(&uuid), Some(COYOTE_V3_SERVICE_UUID));
    }

    #[test]
    fn ignores_non_bluetooth_base_uuid() {
        let uuid = Uuid::parse_str("f000180c-0000-1000-8000-00805f9b34fb").unwrap();

        assert_eq!(short_uuid(&uuid), None);
    }

    #[test]
    fn keeps_coyote_service_advertisements() {
        assert_eq!(
            coyote_service_uuids(vec![COYOTE_V3_SERVICE_UUID], None),
            Some(vec![COYOTE_V3_SERVICE_UUID])
        );
        assert_eq!(
            coyote_service_uuids(vec![COYOTE_V2_SERVICE_UUID], Some("Other")),
            Some(vec![COYOTE_V2_SERVICE_UUID])
        );
    }

    #[test]
    fn builds_coyote_service_scan_filter() {
        let filter = coyote_scan_filter();

        assert_eq!(
            filter.services,
            vec![
                bluetooth_service_uuid(COYOTE_BATTERY_SERVICE_UUID),
                bluetooth_service_uuid(COYOTE_V2_SERVICE_UUID),
                bluetooth_service_uuid(COYOTE_V3_SERVICE_UUID)
            ]
        );
    }

    #[test]
    fn maps_known_characteristic_uuids() {
        let uuid = Uuid::parse_str(&BleCharacteristic::CoyoteV3Notify.uuid()).unwrap();

        assert_eq!(
            ble_characteristic_from_uuid(uuid),
            Some(BleCharacteristic::CoyoteV3Notify)
        );
    }

    #[test]
    fn notification_status_caches_strength_and_battery() {
        let cache = StdMutex::new(HashMap::new());
        let device_id = DeviceId::new("coyote-v3");

        update_notification_status(
            &cache,
            &device_id,
            ValueNotification {
                uuid: Uuid::parse_str(&BleCharacteristic::CoyoteV3Notify.uuid()).unwrap(),
                value: vec![0xB1, 0x01, 12, 4],
            },
        );
        update_notification_status(
            &cache,
            &device_id,
            ValueNotification {
                uuid: Uuid::parse_str(&BleCharacteristic::CoyoteBattery.uuid()).unwrap(),
                value: vec![88],
            },
        );

        let statuses = cache.lock().unwrap();
        let status = statuses.get(&device_id).unwrap();
        assert_eq!(status.channel_a_strength, Some(12));
        assert_eq!(status.channel_b_strength, Some(4));
        assert_eq!(status.battery_percent, Some(88));
    }

    #[test]
    fn infers_coyote_v3_service_from_documented_local_name() {
        assert_eq!(
            coyote_service_uuids(Vec::new(), Some(V3_DEVICE_NAME)),
            Some(vec![COYOTE_V3_SERVICE_UUID])
        );
    }

    #[test]
    fn infers_coyote_v3_service_from_serial_name_variants() {
        assert_eq!(
            coyote_service_uuids(Vec::new(), Some("47L121009")),
            Some(vec![COYOTE_V3_SERVICE_UUID])
        );
        assert_eq!(
            coyote_service_uuids(Vec::new(), Some("Coyote-V3")),
            Some(vec![COYOTE_V3_SERVICE_UUID])
        );
    }

    #[test]
    fn infers_coyote_v2_service_from_documented_local_name() {
        assert_eq!(
            coyote_service_uuids(Vec::new(), Some(V2_DEVICE_NAME)),
            Some(vec![COYOTE_V2_SERVICE_UUID])
        );
    }

    #[test]
    fn ignores_unknown_named_devices_without_coyote_services() {
        assert_eq!(coyote_service_uuids(Vec::new(), Some("Keyboard")), None);
        assert_eq!(coyote_service_uuids(Vec::new(), None), None);
    }

    #[test]
    fn maps_native_permission_errors_to_adapter_state() {
        assert!(is_permission_denied_error(&CoreError::Transport(
            "Bluetooth permission denied".to_owned()
        )));
        assert!(is_permission_denied_error(&CoreError::Transport(
            "CoreBluetooth is not authorized".to_owned()
        )));
        assert!(is_permission_denied_error(&CoreError::Transport(
            "missing bluetooth entitlement".to_owned()
        )));
        assert!(!is_permission_denied_error(&CoreError::Transport(
            "connection timed out".to_owned()
        )));
    }

    #[test]
    fn maps_native_power_errors_to_adapter_state() {
        assert!(is_powered_off_error(&CoreError::Transport(
            "no BLE adapter is available".to_owned()
        )));
        assert!(is_powered_off_error(&CoreError::Transport(
            "Bluetooth is off".to_owned()
        )));
        assert!(!is_powered_off_error(&CoreError::Transport(
            "Bluetooth permission denied".to_owned()
        )));
    }
}
