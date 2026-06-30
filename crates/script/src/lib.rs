#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Safe script document model and compiler for ArcFlow.
//!
//! Scripts compile into validated plans before Core is allowed to execute them.

use core::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Current script document version.
pub const SCRIPT_VERSION: u16 = 1;

/// Script document loaded from storage or a file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptDocument {
    /// Stable script id.
    pub id: String,
    /// Script format version.
    pub version: u16,
    /// Ordered script steps.
    pub steps: Vec<ScriptStep>,
}

impl ScriptDocument {
    /// Parses a script document from JSON.
    pub fn from_json(json: &str) -> Result<Self, ScriptError> {
        serde_json::from_str(json).map_err(ScriptError::Json)
    }
}

/// One safe script step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ScriptStep {
    /// Wait before continuing.
    Wait {
        /// Duration in milliseconds.
        #[serde(rename = "durationMs")]
        duration_ms: u64,
    },
    /// Read device status.
    DeviceStatus {
        /// Device id.
        #[serde(rename = "deviceId")]
        device_id: String,
    },
    /// Stop output for a device.
    WaveStop {
        /// Device id.
        #[serde(rename = "deviceId")]
        device_id: String,
    },
    /// Invoke a plugin hook.
    PluginHook {
        /// Plugin id.
        #[serde(rename = "pluginId")]
        plugin_id: String,
        /// Hook name.
        hook: String,
        /// Hook payload.
        #[serde(default)]
        payload: Value,
    },
}

/// Validated script plan ready for Core execution.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledScript {
    document: ScriptDocument,
}

impl CompiledScript {
    /// Returns the script id.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.document.id
    }

    /// Returns the validated script steps.
    #[must_use]
    pub fn steps(&self) -> &[ScriptStep] {
        &self.document.steps
    }

    /// Returns the source document.
    #[must_use]
    pub fn document(&self) -> &ScriptDocument {
        &self.document
    }
}

/// Script compiler safety limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptCompilerLimits {
    /// Maximum number of steps in one script.
    pub max_steps: usize,
    /// Maximum allowed wait for one step.
    pub max_wait_ms: u64,
}

impl ScriptCompilerLimits {
    /// Conservative default compiler limits.
    #[must_use]
    pub fn conservative() -> Self {
        Self {
            max_steps: 256,
            max_wait_ms: 60_000,
        }
    }
}

/// Script compiler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptCompiler {
    limits: ScriptCompilerLimits,
}

impl ScriptCompiler {
    /// Constructs a script compiler with explicit limits.
    #[must_use]
    pub fn new(limits: ScriptCompilerLimits) -> Self {
        Self { limits }
    }

    /// Compiles a script document into a validated execution plan.
    pub fn compile(&self, document: ScriptDocument) -> Result<CompiledScript, ScriptError> {
        ensure_not_blank("script id", &document.id)?;

        if document.version != SCRIPT_VERSION {
            return Err(ScriptError::UnsupportedVersion {
                requested: document.version,
                supported: SCRIPT_VERSION,
            });
        }

        if document.steps.is_empty() {
            return Err(ScriptError::EmptyScript);
        }

        if document.steps.len() > self.limits.max_steps {
            return Err(ScriptError::TooManySteps {
                actual: document.steps.len(),
                max: self.limits.max_steps,
            });
        }

        for step in &document.steps {
            self.validate_step(step)?;
        }

        Ok(CompiledScript { document })
    }

    fn validate_step(&self, step: &ScriptStep) -> Result<(), ScriptError> {
        match step {
            ScriptStep::Wait { duration_ms } => {
                if *duration_ms > self.limits.max_wait_ms {
                    return Err(ScriptError::WaitTooLong {
                        duration_ms: *duration_ms,
                        max_wait_ms: self.limits.max_wait_ms,
                    });
                }
            }
            ScriptStep::DeviceStatus { device_id } | ScriptStep::WaveStop { device_id } => {
                ensure_not_blank("device id", device_id)?;
            }
            ScriptStep::PluginHook {
                plugin_id, hook, ..
            } => {
                ensure_not_blank("plugin id", plugin_id)?;
                ensure_not_blank("plugin hook", hook)?;
            }
        }

        Ok(())
    }
}

impl Default for ScriptCompiler {
    fn default() -> Self {
        Self::new(ScriptCompilerLimits::conservative())
    }
}

/// Error returned when a script cannot be parsed or compiled.
#[derive(Debug)]
pub enum ScriptError {
    /// Script JSON could not be parsed.
    Json(serde_json::Error),
    /// A required identifier was blank.
    BlankIdentifier(&'static str),
    /// Script format version is unsupported.
    UnsupportedVersion {
        /// Requested version.
        requested: u16,
        /// Supported version.
        supported: u16,
    },
    /// Script has no steps.
    EmptyScript,
    /// Script exceeds the configured step limit.
    TooManySteps {
        /// Actual step count.
        actual: usize,
        /// Maximum step count.
        max: usize,
    },
    /// Wait step exceeds the configured wait limit.
    WaitTooLong {
        /// Requested wait duration in milliseconds.
        duration_ms: u64,
        /// Maximum wait duration in milliseconds.
        max_wait_ms: u64,
    },
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(f, "invalid script JSON: {error}"),
            Self::BlankIdentifier(name) => write!(f, "{name} must not be blank"),
            Self::UnsupportedVersion {
                requested,
                supported,
            } => write!(
                f,
                "script version {requested} is unsupported; expected {supported}"
            ),
            Self::EmptyScript => write!(f, "script must contain at least one step"),
            Self::TooManySteps { actual, max } => {
                write!(f, "script has {actual} steps; maximum is {max}")
            }
            Self::WaitTooLong {
                duration_ms,
                max_wait_ms,
            } => write!(
                f,
                "script wait {duration_ms}ms exceeds maximum {max_wait_ms}ms"
            ),
        }
    }
}

impl std::error::Error for ScriptError {}

fn ensure_not_blank(name: &'static str, value: &str) -> Result<(), ScriptError> {
    if value.trim().is_empty() {
        return Err(ScriptError::BlankIdentifier(name));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn parses_and_compiles_script_document() {
        let document = ScriptDocument::from_json(
            r#"{
                "id": "script.demo",
                "version": 1,
                "steps": [
                    {"type": "deviceStatus", "deviceId": "coyote-v3"},
                    {"type": "wait", "durationMs": 250},
                    {"type": "waveStop", "deviceId": "coyote-v3"}
                ]
            }"#,
        )
        .unwrap();

        let compiled = ScriptCompiler::default().compile(document).unwrap();

        assert_eq!(compiled.id(), "script.demo");
        assert_eq!(compiled.steps().len(), 3);
    }

    #[test]
    fn rejects_waits_above_limit() {
        let document = ScriptDocument {
            id: "script.slow".to_owned(),
            version: SCRIPT_VERSION,
            steps: vec![ScriptStep::Wait {
                duration_ms: 60_001,
            }],
        };

        let error = ScriptCompiler::default().compile(document).unwrap_err();

        assert!(matches!(error, ScriptError::WaitTooLong { .. }));
    }

    #[test]
    fn rejects_blank_plugin_hook() {
        let document = ScriptDocument {
            id: "script.plugin".to_owned(),
            version: SCRIPT_VERSION,
            steps: vec![ScriptStep::PluginHook {
                plugin_id: "plugin.a".to_owned(),
                hook: " ".to_owned(),
                payload: json!({}),
            }],
        };

        let error = ScriptCompiler::default().compile(document).unwrap_err();

        assert!(matches!(error, ScriptError::BlankIdentifier("plugin hook")));
    }
}
