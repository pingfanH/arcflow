//! Device output controller implementations.

use core::fmt;
use std::sync::Mutex;

use crate::{
    BleWrite, CoreError, CoyoteV3CommandBuilder, CoyoteV3OutputRequest, DeviceId,
    DeviceOutputController, SafetyLimits, StopOutputResult, SubmitOutputResult,
};

/// Synchronous sink used by Core output controllers to hand BLE writes to a platform queue.
pub trait DeviceBleOutputSink: fmt::Debug + Send + Sync {
    /// Queues one BLE write for a connected device.
    fn write(&self, device_id: &DeviceId, write: BleWrite) -> Result<(), CoreError>;
}

/// Output controller for connected Coyote V3 devices.
#[derive(Debug)]
pub struct CoyoteV3OutputController<S> {
    sink: S,
    safety_limits: SafetyLimits,
    active_devices: Mutex<Vec<DeviceId>>,
}

impl<S> CoyoteV3OutputController<S>
where
    S: DeviceBleOutputSink,
{
    /// Constructs a Coyote V3 output controller.
    #[must_use]
    pub fn new(safety_limits: SafetyLimits, sink: S) -> Self {
        Self {
            sink,
            safety_limits,
            active_devices: Mutex::new(Vec::new()),
        }
    }

    /// Registers a connected Coyote V3 device as eligible for output writes.
    pub fn attach_device(&self, device_id: DeviceId) {
        let mut active_devices = self
            .active_devices
            .lock()
            .expect("output controller active device mutex poisoned");

        if !active_devices.contains(&device_id) {
            active_devices.push(device_id);
        }
    }

    /// Removes a device from the active output set.
    pub fn detach_device(&self, device_id: &DeviceId) {
        self.active_devices
            .lock()
            .expect("output controller active device mutex poisoned")
            .retain(|active_id| active_id != device_id);
    }

    /// Lists active device ids in attachment order.
    pub fn active_devices(&self) -> Vec<DeviceId> {
        self.active_devices
            .lock()
            .expect("output controller active device mutex poisoned")
            .clone()
    }

    fn ensure_attached(&self, device_id: &DeviceId) -> Result<(), CoreError> {
        if self
            .active_devices
            .lock()
            .expect("output controller active device mutex poisoned")
            .contains(device_id)
        {
            Ok(())
        } else {
            Err(CoreError::Transport(format!(
                "device `{}` is not connected",
                device_id.as_str()
            )))
        }
    }
}

impl<S> DeviceOutputController for CoyoteV3OutputController<S>
where
    S: DeviceBleOutputSink,
{
    fn attach_output_device(&self, device_id: DeviceId) -> Result<(), CoreError> {
        let write = CoyoteV3CommandBuilder::new(self.safety_limits).build_safety_limits_write()?;

        self.sink.write(&device_id, write)?;
        self.attach_device(device_id);
        Ok(())
    }

    fn detach_output_device(&self, device_id: &DeviceId) -> Result<(), CoreError> {
        self.detach_device(device_id);
        Ok(())
    }

    fn active_output_devices(&self) -> Vec<DeviceId> {
        self.active_devices()
    }

    fn submit_coyote_v3_window(
        &self,
        request: CoyoteV3OutputRequest,
    ) -> Result<SubmitOutputResult, CoreError> {
        self.ensure_attached(&request.device_id)?;
        let write = CoyoteV3CommandBuilder::new(self.safety_limits).build_wave_write(
            request.window,
            request.sequence,
            request.strength_modes,
            request.a_strength,
            request.b_strength,
        )?;

        self.sink.write(&request.device_id, write)?;

        Ok(SubmitOutputResult::new(request.device_id, true))
    }

    fn stop_all_output(&self) -> Result<StopOutputResult, CoreError> {
        let active_devices = self.active_devices();
        let write = CoyoteV3CommandBuilder::new(self.safety_limits).build_stop_write(0)?;

        for device_id in &active_devices {
            self.sink.write(device_id, write.clone())?;
        }

        Ok(StopOutputResult::new(active_devices))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use arcflow_protocol::coyote::v3::{StrengthMode, StrengthModes};
    use arcflow_wave::{ChannelOutput, ChannelWindow, CoyoteV3Window, WavePoint};

    use super::*;
    use crate::BleCharacteristic;

    #[derive(Debug, Default)]
    struct RecordingSink {
        writes: Mutex<Vec<(DeviceId, BleWrite)>>,
    }

    impl DeviceBleOutputSink for RecordingSink {
        fn write(&self, device_id: &DeviceId, write: BleWrite) -> Result<(), CoreError> {
            self.writes.lock().unwrap().push((device_id.clone(), write));
            Ok(())
        }
    }

    #[test]
    fn submits_coyote_v3_window_to_sink() {
        let controller =
            CoyoteV3OutputController::new(SafetyLimits::conservative(), RecordingSink::default());
        controller.attach_device(DeviceId::new("coyote-v3"));

        let result = controller
            .submit_coyote_v3_window(CoyoteV3OutputRequest::new(
                DeviceId::new("coyote-v3"),
                safe_window(),
                5,
                StrengthModes::new(StrengthMode::Unchanged, StrengthMode::Unchanged),
                0,
                0,
            ))
            .unwrap();

        assert!(result.accepted);
        let writes = controller.sink.writes.lock().unwrap();
        assert_eq!(writes[0].0, DeviceId::new("coyote-v3"));
        assert_eq!(writes[0].1.characteristic, BleCharacteristic::CoyoteV3Write);
        assert_eq!(writes[0].1.payload[1], 0x50);
    }

    #[test]
    fn activation_writes_coyote_v3_safety_limits_before_output() {
        let controller =
            CoyoteV3OutputController::new(SafetyLimits::conservative(), RecordingSink::default());

        controller
            .attach_output_device(DeviceId::new("coyote-v3"))
            .unwrap();

        assert_eq!(
            controller.active_devices(),
            vec![DeviceId::new("coyote-v3")]
        );
        let writes = controller.sink.writes.lock().unwrap();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].0, DeviceId::new("coyote-v3"));
        assert_eq!(writes[0].1.payload, vec![0xBF, 20, 20, 0, 0, 0, 0]);
    }

    #[test]
    fn rejects_submit_to_detached_device() {
        let controller =
            CoyoteV3OutputController::new(SafetyLimits::conservative(), RecordingSink::default());

        let error = controller
            .submit_coyote_v3_window(CoyoteV3OutputRequest::new(
                DeviceId::new("missing"),
                safe_window(),
                0,
                StrengthModes::new(StrengthMode::Unchanged, StrengthMode::Unchanged),
                0,
                0,
            ))
            .unwrap_err();

        assert!(matches!(error, CoreError::Transport(_)));
    }

    #[test]
    fn stops_all_attached_devices() {
        let controller =
            CoyoteV3OutputController::new(SafetyLimits::conservative(), RecordingSink::default());
        controller.attach_device(DeviceId::new("coyote-a"));
        controller.attach_device(DeviceId::new("coyote-b"));

        let result = controller.stop_all_output().unwrap();

        assert_eq!(
            result.stopped_devices,
            vec![DeviceId::new("coyote-a"), DeviceId::new("coyote-b")]
        );
        let writes = controller.sink.writes.lock().unwrap();
        assert_eq!(writes.len(), 2);
        assert_eq!(writes[0].1.payload[1], 0x0F);
        assert_eq!(writes[0].1.payload[11], 0x65);
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
}
