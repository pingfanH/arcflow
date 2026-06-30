//! Plugin manifest model.

use core::fmt;

use serde::{Deserialize, Serialize};

use crate::Capability;

/// Runtime used by a plugin bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeKind {
    /// WebAssembly plugin runtime.
    Wasm,
    /// JavaScript plugin runtime.
    #[serde(alias = "js")]
    JavaScript,
}

/// Plugin manifest loaded from `manifest.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    /// Stable plugin identifier, for example `com.example.plugin`.
    pub id: String,
    /// Human-readable plugin name.
    pub name: String,
    /// Plugin version string.
    pub version: String,
    /// Runtime required by this plugin.
    pub runtime: RuntimeKind,
    /// Bundle-relative entry file.
    pub entry: String,
    /// Plugin Interface version requested by the plugin.
    pub api_version: String,
    /// Capabilities requested by the plugin.
    pub capabilities: Vec<Capability>,
}

impl PluginManifest {
    /// Parses a manifest from JSON and validates required fields.
    pub fn from_json(json: &str) -> Result<Self, ManifestError> {
        let manifest: Self = serde_json::from_str(json).map_err(ManifestError::Json)?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Validates the manifest fields ArcFlow needs before installation.
    pub fn validate(&self) -> Result<(), ManifestError> {
        ensure_not_blank("id", &self.id)?;
        ensure_not_blank("name", &self.name)?;
        ensure_not_blank("version", &self.version)?;
        ensure_not_blank("entry", &self.entry)?;
        ensure_not_blank("apiVersion", &self.api_version)?;

        if self.capabilities.is_empty() {
            return Err(ManifestError::MissingCapability);
        }

        if self.entry.starts_with('/') || self.entry.contains("..") {
            return Err(ManifestError::UnsafeEntryPath);
        }

        Ok(())
    }
}

/// Error returned when a plugin manifest cannot be accepted.
#[derive(Debug)]
pub enum ManifestError {
    /// JSON parsing failed.
    Json(serde_json::Error),
    /// A required field was blank.
    BlankField(&'static str),
    /// The manifest requested no capabilities.
    MissingCapability,
    /// The manifest entry path is not bundle-relative.
    UnsafeEntryPath,
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(f, "invalid plugin manifest JSON: {error}"),
            Self::BlankField(field) => {
                write!(f, "plugin manifest field `{field}` must not be blank")
            }
            Self::MissingCapability => {
                write!(f, "plugin manifest must request at least one capability")
            }
            Self::UnsafeEntryPath => {
                write!(f, "plugin manifest entry must be a safe relative path")
            }
        }
    }
}

impl std::error::Error for ManifestError {}

fn ensure_not_blank(field: &'static str, value: &str) -> Result<(), ManifestError> {
    if value.trim().is_empty() {
        return Err(ManifestError::BlankField(field));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_wasm_manifest() {
        let manifest = PluginManifest::from_json(
            r#"{
                "id": "com.example.pulse-tools",
                "name": "Pulse Tools",
                "version": "1.0.0",
                "runtime": "wasm",
                "entry": "dist/plugin.wasm",
                "apiVersion": "1",
                "capabilities": ["device.read", "wave.generate"]
            }"#,
        )
        .unwrap();

        assert_eq!(manifest.runtime, RuntimeKind::Wasm);
        assert_eq!(
            manifest.capabilities,
            vec![Capability::DeviceRead, Capability::WaveGenerate]
        );
    }

    #[test]
    fn accepts_js_runtime_alias() {
        let manifest = PluginManifest::from_json(
            r#"{
                "id": "com.example.js",
                "name": "JS Plugin",
                "version": "1.0.0",
                "runtime": "js",
                "entry": "dist/plugin.js",
                "apiVersion": "1",
                "capabilities": ["ui.panel"]
            }"#,
        )
        .unwrap();

        assert_eq!(manifest.runtime, RuntimeKind::JavaScript);
    }

    #[test]
    fn rejects_unsafe_entry_path() {
        let error = PluginManifest::from_json(
            r#"{
                "id": "com.example.bad",
                "name": "Bad",
                "version": "1.0.0",
                "runtime": "wasm",
                "entry": "../plugin.wasm",
                "apiVersion": "1",
                "capabilities": ["device.read"]
            }"#,
        )
        .unwrap_err();

        assert!(matches!(error, ManifestError::UnsafeEntryPath));
    }
}
