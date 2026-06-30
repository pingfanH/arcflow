//! Plugin bundle loading and validation.

use core::fmt;
use std::{
    fs,
    path::{Path, PathBuf},
};

use arcflow_plugin_runtime::{ManifestError, PluginManifest};

/// Validated plugin bundle on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginBundle {
    root: PathBuf,
    manifest: PluginManifest,
    entry_path: PathBuf,
}

impl PluginBundle {
    /// Loads and validates a plugin bundle directory.
    pub fn load(root: impl AsRef<Path>) -> Result<Self, PluginBundleError> {
        let root = root.as_ref().to_path_buf();
        let manifest_path = root.join("manifest.json");
        let manifest_json =
            fs::read_to_string(&manifest_path).map_err(|error| PluginBundleError::Io {
                path: manifest_path.clone(),
                message: error.to_string(),
            })?;
        let manifest =
            PluginManifest::from_json(&manifest_json).map_err(PluginBundleError::Manifest)?;
        let entry_path = root.join(&manifest.entry);

        if !entry_path.is_file() {
            return Err(PluginBundleError::MissingEntry { path: entry_path });
        }

        Ok(Self {
            root,
            manifest,
            entry_path,
        })
    }

    /// Returns the bundle root directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the parsed plugin manifest.
    #[must_use]
    pub fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    /// Returns the resolved entry file path.
    #[must_use]
    pub fn entry_path(&self) -> &Path {
        &self.entry_path
    }
}

/// Error returned when a plugin bundle cannot be loaded.
#[derive(Debug)]
pub enum PluginBundleError {
    /// A filesystem operation failed.
    Io {
        /// Path involved in the failed operation.
        path: PathBuf,
        /// Filesystem error message.
        message: String,
    },
    /// The bundle manifest was invalid.
    Manifest(ManifestError),
    /// The manifest entry file does not exist.
    MissingEntry {
        /// Resolved entry file path.
        path: PathBuf,
    },
}

impl fmt::Display for PluginBundleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, message } => {
                write!(f, "failed to read `{}`: {message}", path.display())
            }
            Self::Manifest(error) => write!(f, "invalid plugin bundle manifest: {error}"),
            Self::MissingEntry { path } => {
                write!(f, "plugin bundle entry `{}` does not exist", path.display())
            }
        }
    }
}

impl std::error::Error for PluginBundleError {}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    #[test]
    fn loads_valid_plugin_bundle() {
        let root = test_bundle_root("valid");
        fs::create_dir_all(root.join("dist")).unwrap();
        fs::write(root.join("dist/plugin.wasm"), b"").unwrap();
        fs::write(
            root.join("manifest.json"),
            manifest_json("dist/plugin.wasm"),
        )
        .unwrap();

        let bundle = PluginBundle::load(&root).unwrap();

        assert_eq!(bundle.manifest().id, "dev.arcflow.test");
        assert_eq!(bundle.entry_path(), root.join("dist/plugin.wasm"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_missing_entry_file() {
        let root = test_bundle_root("missing-entry");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("manifest.json"),
            manifest_json("dist/plugin.wasm"),
        )
        .unwrap();

        let error = PluginBundle::load(&root).unwrap_err();

        assert!(matches!(error, PluginBundleError::MissingEntry { .. }));

        fs::remove_dir_all(root).unwrap();
    }

    fn manifest_json(entry: &str) -> String {
        format!(
            r#"{{
                "id":"dev.arcflow.test",
                "name":"Test Plugin",
                "version":"0.1.0",
                "runtime":"wasm",
                "entry":"{entry}",
                "apiVersion":"1",
                "capabilities":["device.read"]
            }}"#
        )
    }

    fn test_bundle_root(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("arcflow-plugin-bundle-{name}-{suffix}"))
    }
}
