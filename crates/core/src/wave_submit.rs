//! Shared parsing for Coyote V3 submit-window host requests.

use core::fmt;

use arcflow_protocol::coyote::v3::{StrengthMode, StrengthModes};
use arcflow_wave::{ChannelOutput, ChannelWindow, CoyoteV3Window, WavePoint};
use serde::Deserialize;
use serde_json::Value;

use crate::{CoyoteV3OutputRequest, DeviceId};

pub(crate) fn coyote_v3_output_request_from_value(
    method: &'static str,
    params: Value,
) -> Result<CoyoteV3OutputRequest, SubmitWindowParamsError> {
    let params: SubmitWindowParams =
        serde_json::from_value(params).map_err(|error| SubmitWindowParamsError::Json {
            method,
            error: error.to_string(),
        })?;
    let device_id = DeviceId::new(params.device_id);

    Ok(CoyoteV3OutputRequest::new(
        device_id,
        CoyoteV3Window::new(
            channel_output(params.channel_a)?,
            channel_output(params.channel_b)?,
        ),
        params.sequence,
        StrengthModes::new(
            strength_mode(params.strength_modes.a),
            strength_mode(params.strength_modes.b),
        ),
        params.a_strength,
        params.b_strength,
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SubmitWindowParamsError {
    Json { method: &'static str, error: String },
    Wave(String),
}

impl fmt::Display for SubmitWindowParamsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json { method, error } => {
                write!(f, "invalid params for `{method}`: {error}")
            }
            Self::Wave(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for SubmitWindowParamsError {}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubmitWindowParams {
    device_id: String,
    sequence: u8,
    #[serde(default)]
    strength_modes: StrengthModesParams,
    #[serde(default)]
    a_strength: u8,
    #[serde(default)]
    b_strength: u8,
    #[serde(default)]
    channel_a: Option<[WavePointParams; 4]>,
    #[serde(default)]
    channel_b: Option<[WavePointParams; 4]>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StrengthModesParams {
    #[serde(default)]
    a: StrengthModeParam,
    #[serde(default)]
    b: StrengthModeParam,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
enum StrengthModeParam {
    #[default]
    Unchanged,
    Increase,
    Decrease,
    Absolute,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WavePointParams {
    period_ms: u16,
    strength: u8,
}

fn channel_output(
    points: Option<[WavePointParams; 4]>,
) -> Result<ChannelOutput, SubmitWindowParamsError> {
    let Some(points) = points else {
        return Ok(ChannelOutput::Disabled);
    };

    Ok(ChannelOutput::Window(ChannelWindow::new([
        wave_point(points[0])?,
        wave_point(points[1])?,
        wave_point(points[2])?,
        wave_point(points[3])?,
    ])))
}

fn wave_point(point: WavePointParams) -> Result<WavePoint, SubmitWindowParamsError> {
    WavePoint::new(point.period_ms, point.strength)
        .map_err(|error| SubmitWindowParamsError::Wave(error.to_string()))
}

fn strength_mode(mode: StrengthModeParam) -> StrengthMode {
    match mode {
        StrengthModeParam::Unchanged => StrengthMode::Unchanged,
        StrengthModeParam::Increase => StrengthMode::Increase,
        StrengthModeParam::Decrease => StrengthMode::Decrease,
        StrengthModeParam::Absolute => StrengthMode::Absolute,
    }
}

#[cfg(test)]
mod tests {
    use arcflow_protocol::coyote::v3::{StrengthMode, StrengthModes};
    use arcflow_wave::{ChannelOutput, ChannelWindow, CoyoteV3Window, WavePoint};
    use serde_json::{json, Value};

    use super::*;

    #[test]
    fn parses_coyote_v3_submit_window_params() {
        let request =
            coyote_v3_output_request_from_value("wave.submitWindow", submit_window_params())
                .unwrap();

        assert_eq!(request.device_id, DeviceId::new("coyote-v3"));
        assert_eq!(request.sequence, 4);
        assert_eq!(
            request.strength_modes,
            StrengthModes::new(StrengthMode::Absolute, StrengthMode::Unchanged)
        );
        assert_eq!(request.a_strength, 8);
        assert_eq!(request.b_strength, 0);
        assert_eq!(request.window, safe_window());
    }

    #[test]
    fn rejects_invalid_wave_points() {
        let error = coyote_v3_output_request_from_value(
            "wave.submitWindow",
            json!({
                "deviceId": "coyote-v3",
                "sequence": 4,
                "channelA": [
                    {"periodMs": 1, "strength": 0},
                    {"periodMs": 10, "strength": 10},
                    {"periodMs": 10, "strength": 20},
                    {"periodMs": 10, "strength": 30}
                ]
            }),
        )
        .unwrap_err();

        assert!(matches!(error, SubmitWindowParamsError::Wave(_)));
    }

    fn submit_window_params() -> Value {
        json!({
            "deviceId": "coyote-v3",
            "sequence": 4,
            "strengthModes": {
                "a": "absolute",
                "b": "unchanged"
            },
            "aStrength": 8,
            "bStrength": 0,
            "channelA": [
                {"periodMs": 10, "strength": 0},
                {"periodMs": 10, "strength": 10},
                {"periodMs": 10, "strength": 20},
                {"periodMs": 10, "strength": 30}
            ],
            "channelB": null
        })
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
