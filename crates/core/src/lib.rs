#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Core orchestration types for ArcFlow.
//!
//! This crate is the future owner of Bluetooth sessions, device safety policy,
//! plugin dispatch, external-control dispatch, storage access, and Tauri IPC
//! commands. The first slice defines stable types used across those seams.

pub mod ble;
pub mod command;
pub mod coyote;
pub mod device;
pub mod error;
pub mod external;
pub mod output;
pub mod plugin_api;
pub mod plugin_bundle;
pub mod plugin_registry_persistence;
pub mod plugin_runtime_controller;
pub mod runtime;
pub mod safety;
pub mod script_execution;
pub mod script_persistence;
pub mod script_runner;
pub mod session;

pub use ble::{
    bluetooth_base_uuid, BleAdvertisement, BleCharacteristic, BleDeviceDiscoveryController,
    BleDiscovery, BleNotification, BleTransport, BleWrite, COYOTE_BATTERY_SERVICE_UUID,
    COYOTE_V2_SERVICE_UUID, COYOTE_V3_SERVICE_UUID, COYOTE_V3_STATUS_CHARACTERISTICS,
};
pub use command::CoreCommand;
pub use coyote::CoyoteV3CommandBuilder;
pub use device::{
    BleAdapterStatus, CoyoteV3OutputRequest, CoyoteV3PreviewSession, CoyoteV3SequenceAllocator,
    DeviceId, DeviceModel, DeviceScanResult, DeviceStatus, StopOutputResult, SubmitOutputResult,
    COYOTE_V3_PREVIEW_INTERVAL_MS,
};
pub use error::CoreError;
pub use external::{
    authorized_core_command_from_external_request, core_command_from_external_request,
    execute_plugin_registry_external_request, execute_script_documents_external_request,
    is_plugin_registry_external_request, is_plugin_registry_mutation_external_request,
    is_script_documents_external_request, required_capability_for_external_request,
};
pub use output::{CoyoteV3OutputController, DeviceBleOutputSink};
pub use plugin_api::{PluginApi, PluginApiError};
pub use plugin_bundle::{PluginBundle, PluginBundleError};
pub use plugin_registry_persistence::{
    PluginRegistryEntry, PluginRegistryPersistence, PluginRegistryPersistenceError,
};
pub use plugin_runtime_controller::{
    ArcFlowPluginHost, ArcFlowPluginRuntime, ArcFlowPluginRuntimeRouter,
    PluginRuntimeControllerError, PluginRuntimeSyncReport, RecordingPluginRuntimeController,
    RecordingPluginRuntimeHookInvoker, RecordingPluginRuntimeSnapshots,
};
pub use runtime::{
    ArcFlowCore, DeviceDiscoveryController, DeviceOutputController, NoopDeviceDiscoveryController,
    NoopDeviceOutputController, NoopScriptRunner, ScriptRunResult, ScriptRunner,
};
pub use safety::SafetyLimits;
pub use script_execution::{
    CompiledScriptQueue, CoreScriptActionExecutor, NoopCompiledScriptQueue,
    NoopScriptActionExecutor, PluginHookInvoker, ScriptActionExecutor, ScriptExecutionEngine,
    ScriptExecutionReport, ScriptStepExecution, ScriptStepKind, ScriptWorkerEvent,
    ScriptWorkerQueue, UnattachedPluginHookInvoker,
};
pub use script_persistence::{
    ScriptDocumentEntry, ScriptDocumentPersistence, ScriptDocumentPersistenceError,
};
pub use script_runner::StorageScriptRunner;
pub use session::{CoyoteV3StrengthStatus, DeviceSession};
