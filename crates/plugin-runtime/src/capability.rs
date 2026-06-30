//! Plugin capability vocabulary.

use serde::{Deserialize, Serialize};

/// Capability requested by a plugin or external-control client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Capability {
    /// Read device and session state.
    #[serde(rename = "device.read")]
    DeviceRead,
    /// Submit wave generation requests.
    #[serde(rename = "wave.generate")]
    WaveGenerate,
    /// Control playback of already-validated wave output.
    #[serde(rename = "wave.control")]
    WaveControl,
    /// Run scripts through the Rust script engine.
    #[serde(rename = "script.run")]
    ScriptRun,
    /// Use plugin-private storage.
    #[serde(rename = "storage.private")]
    StoragePrivate,
    /// Register host-rendered UI panels.
    #[serde(rename = "ui.panel")]
    UiPanel,
    /// Install, list, enable, or disable plugins.
    #[serde(rename = "plugin.manage")]
    PluginManage,
    /// Subscribe to host events.
    #[serde(rename = "events.subscribe")]
    EventsSubscribe,
    /// Connect through the WebSocket external-control interface.
    #[serde(rename = "external.ws")]
    ExternalWebSocket,
}

impl Capability {
    /// Returns the stable wire string for this capability.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DeviceRead => "device.read",
            Self::WaveGenerate => "wave.generate",
            Self::WaveControl => "wave.control",
            Self::ScriptRun => "script.run",
            Self::StoragePrivate => "storage.private",
            Self::UiPanel => "ui.panel",
            Self::PluginManage => "plugin.manage",
            Self::EventsSubscribe => "events.subscribe",
            Self::ExternalWebSocket => "external.ws",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_capability_wire_names() {
        let value = serde_json::to_string(&Capability::WaveControl).unwrap();

        assert_eq!(value, "\"wave.control\"");
    }

    #[test]
    fn deserializes_capability_wire_names() {
        let capability: Capability = serde_json::from_str("\"plugin.manage\"").unwrap();

        assert_eq!(capability, Capability::PluginManage);
        assert_eq!(capability.as_str(), "plugin.manage");
    }
}
