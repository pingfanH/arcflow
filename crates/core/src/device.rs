//! Device identity and status types.

use std::collections::HashMap;

use arcflow_protocol::coyote::v3::{StrengthMode, StrengthModes};
use arcflow_wave::{ChannelOutput, ChannelWindow, CoyoteV3Window, WavePoint};

use crate::CoreError;

const PREVIEW_WAVE_PERIOD_MS: u16 = 10;
const PREVIEW_WAVE_STRENGTHS: [u8; 4] = [0, 10, 20, 30];
/// Interval between generated Coyote V3 preview windows.
pub const COYOTE_V3_PREVIEW_INTERVAL_MS: u64 = 100;

/// Stable ArcFlow device identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeviceId(String);

impl DeviceId {
    /// Constructs a device id.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the device id as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Device model known to ArcFlow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceModel {
    /// DG-LAB Coyote V2 pulse host.
    CoyoteV2,
    /// DG-LAB Coyote V3 pulse host.
    CoyoteV3,
    /// Device model that is not recognized yet.
    Unknown(String),
}

/// Current device status exposed to UI, plugins, and external-control clients.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceStatus {
    /// Device id.
    pub id: DeviceId,
    /// Device model.
    pub model: DeviceModel,
    /// Battery percentage if known.
    pub battery_percent: Option<u8>,
    /// Whether the device is currently connected.
    pub connected: bool,
}

/// Current platform BLE Adapter state as seen by Rust Core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BleAdapterStatus {
    /// No platform BLE Adapter has been attached yet.
    Unsupported,
    /// BLE Adapter is ready for scanning.
    Ready,
    /// The operating system denied BLE access.
    PermissionDenied,
    /// BLE radio is powered off.
    PoweredOff,
}

impl BleAdapterStatus {
    /// Returns the stable wire string for UI and plugin-facing DTOs.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::Ready => "ready",
            Self::PermissionDenied => "permissionDenied",
            Self::PoweredOff => "poweredOff",
        }
    }
}

/// Result of one device scan request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceScanResult {
    /// BLE Adapter status at scan time.
    pub adapter_status: BleAdapterStatus,
    /// Devices discovered by the scan.
    pub devices: Vec<DeviceStatus>,
}

impl DeviceScanResult {
    /// Constructs a scan result.
    #[must_use]
    pub fn new(adapter_status: BleAdapterStatus, devices: Vec<DeviceStatus>) -> Self {
        Self {
            adapter_status,
            devices,
        }
    }
}

/// Result of a stop-all-output request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopOutputResult {
    /// Device ids for which output was stopped.
    pub stopped_devices: Vec<DeviceId>,
}

impl StopOutputResult {
    /// Constructs a stop-output result.
    #[must_use]
    pub fn new(stopped_devices: Vec<DeviceId>) -> Self {
        Self { stopped_devices }
    }
}

/// Output request for one Coyote V3 100ms window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoyoteV3OutputRequest {
    /// Target device id.
    pub device_id: DeviceId,
    /// Wave window to write.
    pub window: CoyoteV3Window,
    /// Coyote V3 B0 sequence number.
    pub sequence: u8,
    /// Strength modes for channel A and B.
    pub strength_modes: StrengthModes,
    /// Channel A strength command value.
    pub a_strength: u8,
    /// Channel B strength command value.
    pub b_strength: u8,
}

impl CoyoteV3OutputRequest {
    /// Constructs a Coyote V3 output request.
    #[must_use]
    pub fn new(
        device_id: DeviceId,
        window: CoyoteV3Window,
        sequence: u8,
        strength_modes: StrengthModes,
        a_strength: u8,
        b_strength: u8,
    ) -> Self {
        Self {
            device_id,
            window,
            sequence,
            strength_modes,
            a_strength,
            b_strength,
        }
    }

    /// Constructs a conservative one-window preview from UI channel strengths.
    pub fn preview(
        device_id: DeviceId,
        sequence: u8,
        a_strength: u8,
        b_strength: u8,
    ) -> Result<Self, CoreError> {
        Ok(Self::new(
            device_id,
            CoyoteV3Window::new(
                preview_channel_output(a_strength)?,
                preview_channel_output(b_strength)?,
            ),
            sequence,
            StrengthModes::new(StrengthMode::Absolute, StrengthMode::Absolute),
            a_strength,
            b_strength,
        ))
    }
}

/// Allocates wrapping Coyote V3 B0 sequence numbers per device.
#[derive(Debug, Default)]
pub struct CoyoteV3SequenceAllocator {
    next_by_device: HashMap<DeviceId, u8>,
}

impl CoyoteV3SequenceAllocator {
    /// Returns the next sequence number for a device and advances it.
    pub fn next(&mut self, device_id: &DeviceId) -> u8 {
        let sequence = *self.next_by_device.entry(device_id.clone()).or_insert(0);
        self.next_by_device
            .insert(device_id.clone(), sequence.wrapping_add(1));

        sequence
    }
}

/// Core-owned state for generating a repeating Coyote V3 preview stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoyoteV3PreviewSession {
    device_id: DeviceId,
    channel_a_strength: u8,
    channel_b_strength: u8,
}

impl CoyoteV3PreviewSession {
    /// Constructs a preview session for one active Coyote V3 output device.
    pub fn new(
        device_id: DeviceId,
        channel_a_strength: u8,
        channel_b_strength: u8,
    ) -> Result<Self, CoreError> {
        CoyoteV3OutputRequest::preview(
            device_id.clone(),
            0,
            channel_a_strength,
            channel_b_strength,
        )?;

        Ok(Self {
            device_id,
            channel_a_strength,
            channel_b_strength,
        })
    }

    /// Returns the target device id.
    #[must_use]
    pub fn device_id(&self) -> &DeviceId {
        &self.device_id
    }

    /// Returns the channel A strength used for generated preview windows.
    #[must_use]
    pub fn channel_a_strength(&self) -> u8 {
        self.channel_a_strength
    }

    /// Returns the channel B strength used for generated preview windows.
    #[must_use]
    pub fn channel_b_strength(&self) -> u8 {
        self.channel_b_strength
    }

    /// Builds the next preview output request using the shared sequence allocator.
    pub fn next_request(
        &self,
        sequences: &mut CoyoteV3SequenceAllocator,
    ) -> Result<CoyoteV3OutputRequest, CoreError> {
        let sequence = sequences.next(&self.device_id);

        CoyoteV3OutputRequest::preview(
            self.device_id.clone(),
            sequence,
            self.channel_a_strength,
            self.channel_b_strength,
        )
    }
}

fn preview_channel_output(channel_strength: u8) -> Result<ChannelOutput, CoreError> {
    if channel_strength == 0 {
        return Ok(ChannelOutput::Disabled);
    }

    Ok(ChannelOutput::Window(ChannelWindow::new([
        WavePoint::new(PREVIEW_WAVE_PERIOD_MS, PREVIEW_WAVE_STRENGTHS[0])?,
        WavePoint::new(PREVIEW_WAVE_PERIOD_MS, PREVIEW_WAVE_STRENGTHS[1])?,
        WavePoint::new(PREVIEW_WAVE_PERIOD_MS, PREVIEW_WAVE_STRENGTHS[2])?,
        WavePoint::new(PREVIEW_WAVE_PERIOD_MS, PREVIEW_WAVE_STRENGTHS[3])?,
    ])))
}

/// Result of submitting one output window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitOutputResult {
    /// Device id that received the request.
    pub device_id: DeviceId,
    /// Whether the controller accepted the request.
    pub accepted: bool,
}

impl SubmitOutputResult {
    /// Constructs a submit-output result.
    #[must_use]
    pub fn new(device_id: DeviceId, accepted: bool) -> Self {
        Self {
            device_id,
            accepted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_preview_output_request_from_channel_strengths() {
        let request = CoyoteV3OutputRequest::preview(DeviceId::new("coyote-v3"), 3, 12, 0)
            .expect("preview request");

        assert_eq!(request.device_id, DeviceId::new("coyote-v3"));
        assert_eq!(request.sequence, 3);
        assert_eq!(request.strength_modes.a(), StrengthMode::Absolute);
        assert_eq!(request.strength_modes.b(), StrengthMode::Absolute);
        assert_eq!(request.a_strength, 12);
        assert_eq!(request.b_strength, 0);
        assert!(matches!(request.window.b(), ChannelOutput::Disabled));
        let ChannelOutput::Window(channel_a) = request.window.a() else {
            panic!("channel A should be enabled");
        };
        assert_eq!(
            channel_a.points().map(|point| point.period_ms()),
            [10, 10, 10, 10]
        );
        assert_eq!(
            channel_a.points().map(|point| point.strength()),
            [0, 10, 20, 30]
        );
    }

    #[test]
    fn allocates_coyote_v3_sequences_per_device() {
        let mut allocator = CoyoteV3SequenceAllocator::default();
        let first = DeviceId::new("coyote-a");
        let second = DeviceId::new("coyote-b");

        assert_eq!(allocator.next(&first), 0);
        assert_eq!(allocator.next(&first), 1);
        assert_eq!(allocator.next(&second), 0);
        assert_eq!(allocator.next(&first), 2);
    }

    #[test]
    fn preview_session_allocates_next_request_sequences() {
        let session =
            CoyoteV3PreviewSession::new(DeviceId::new("coyote-v3"), 7, 2).expect("preview session");
        let mut allocator = CoyoteV3SequenceAllocator::default();

        let first = session.next_request(&mut allocator).expect("first request");
        let second = session
            .next_request(&mut allocator)
            .expect("second request");

        assert_eq!(session.device_id(), &DeviceId::new("coyote-v3"));
        assert_eq!(session.channel_a_strength(), 7);
        assert_eq!(session.channel_b_strength(), 2);
        assert_eq!(first.sequence, 0);
        assert_eq!(second.sequence, 1);
        assert_eq!(second.device_id, DeviceId::new("coyote-v3"));
    }
}
