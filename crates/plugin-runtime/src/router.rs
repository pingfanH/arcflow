//! Runtime router for WASM and JavaScript plugin engines.

use async_trait::async_trait;

use crate::{
    PluginInvocation, PluginManifest, PluginOutput, RuntimeAdapter, RuntimeError, RuntimeHandle,
    RuntimeKind,
};

/// Runtime Adapter that routes plugins to WASM or JavaScript engines.
#[derive(Debug)]
pub struct RuntimeRouter<W, J> {
    wasm: W,
    javascript: J,
}

impl<W, J> RuntimeRouter<W, J> {
    /// Constructs a runtime router from concrete WASM and JavaScript Adapters.
    #[must_use]
    pub fn new(wasm: W, javascript: J) -> Self {
        Self { wasm, javascript }
    }

    /// Returns the WASM Adapter.
    #[must_use]
    pub fn wasm(&self) -> &W {
        &self.wasm
    }

    /// Returns the JavaScript Adapter.
    #[must_use]
    pub fn javascript(&self) -> &J {
        &self.javascript
    }
}

#[async_trait]
impl<W, J> RuntimeAdapter for RuntimeRouter<W, J>
where
    W: RuntimeAdapter + Send + Sync,
    J: RuntimeAdapter + Send + Sync,
{
    async fn load(&self, manifest: &PluginManifest) -> Result<RuntimeHandle, RuntimeError> {
        match manifest.runtime {
            RuntimeKind::Wasm => self.wasm.load(manifest).await,
            RuntimeKind::JavaScript => self.javascript.load(manifest).await,
        }
    }

    async fn invoke(
        &self,
        handle: &RuntimeHandle,
        invocation: PluginInvocation,
    ) -> Result<PluginOutput, RuntimeError> {
        match handle.runtime() {
            RuntimeKind::Wasm => self.wasm.invoke(handle, invocation).await,
            RuntimeKind::JavaScript => self.javascript.invoke(handle, invocation).await,
        }
    }

    async fn unload(&self, handle: RuntimeHandle) -> Result<(), RuntimeError> {
        match handle.runtime() {
            RuntimeKind::Wasm => self.wasm.unload(handle).await,
            RuntimeKind::JavaScript => self.javascript.unload(handle).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::json;

    use super::*;
    use crate::{Capability, PluginAction};

    #[derive(Debug)]
    struct RecordingRuntime {
        name: &'static str,
        loaded: Mutex<Vec<String>>,
        invoked: Mutex<Vec<String>>,
        unloaded: Mutex<Vec<String>>,
    }

    impl RecordingRuntime {
        fn new(name: &'static str) -> Self {
            Self {
                name,
                loaded: Mutex::new(Vec::new()),
                invoked: Mutex::new(Vec::new()),
                unloaded: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl RuntimeAdapter for RecordingRuntime {
        async fn load(&self, manifest: &PluginManifest) -> Result<RuntimeHandle, RuntimeError> {
            self.loaded.lock().unwrap().push(manifest.id.clone());
            Ok(RuntimeHandle::new(&manifest.id, manifest.runtime))
        }

        async fn invoke(
            &self,
            handle: &RuntimeHandle,
            invocation: PluginInvocation,
        ) -> Result<PluginOutput, RuntimeError> {
            self.invoked
                .lock()
                .unwrap()
                .push(handle.plugin_id().to_owned());
            Ok(PluginOutput::new(vec![PluginAction::new(
                format!("{}.{}", self.name, invocation.hook),
                invocation.payload,
            )]))
        }

        async fn unload(&self, handle: RuntimeHandle) -> Result<(), RuntimeError> {
            self.unloaded
                .lock()
                .unwrap()
                .push(handle.plugin_id().to_owned());
            Ok(())
        }
    }

    fn manifest(id: &str, runtime: RuntimeKind) -> PluginManifest {
        PluginManifest {
            id: id.to_owned(),
            name: "Plugin".to_owned(),
            version: "1.0.0".to_owned(),
            runtime,
            entry: "dist/plugin".to_owned(),
            api_version: "1".to_owned(),
            capabilities: vec![Capability::DeviceRead],
        }
    }

    #[tokio::test]
    async fn routes_load_by_manifest_runtime() {
        let router = RuntimeRouter::new(
            RecordingRuntime::new("wasm"),
            RecordingRuntime::new("javascript"),
        );

        router
            .load(&manifest("com.example.wasm", RuntimeKind::Wasm))
            .await
            .unwrap();
        router
            .load(&manifest("com.example.js", RuntimeKind::JavaScript))
            .await
            .unwrap();

        assert_eq!(
            *router.wasm().loaded.lock().unwrap(),
            vec!["com.example.wasm"]
        );
        assert_eq!(
            *router.javascript().loaded.lock().unwrap(),
            vec!["com.example.js"]
        );
    }

    #[tokio::test]
    async fn routes_invocation_by_handle_runtime() {
        let router = RuntimeRouter::new(
            RecordingRuntime::new("wasm"),
            RecordingRuntime::new("javascript"),
        );
        let handle = RuntimeHandle::new("com.example.js", RuntimeKind::JavaScript);

        let output = router
            .invoke(&handle, PluginInvocation::new("tick", json!({ "n": 1 })))
            .await
            .unwrap();

        assert_eq!(output.actions[0].method, "javascript.tick");
        assert!(router.wasm().invoked.lock().unwrap().is_empty());
        assert_eq!(
            *router.javascript().invoked.lock().unwrap(),
            vec!["com.example.js"]
        );
    }

    #[tokio::test]
    async fn routes_unload_by_handle_runtime() {
        let router = RuntimeRouter::new(
            RecordingRuntime::new("wasm"),
            RecordingRuntime::new("javascript"),
        );
        let handle = RuntimeHandle::new("com.example.wasm", RuntimeKind::Wasm);

        router.unload(handle).await.unwrap();

        assert_eq!(
            *router.wasm().unloaded.lock().unwrap(),
            vec!["com.example.wasm"]
        );
        assert!(router.javascript().unloaded.lock().unwrap().is_empty());
    }
}
