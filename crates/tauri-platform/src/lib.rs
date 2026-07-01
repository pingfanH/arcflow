#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Tauri 2 platform adapters shared by ArcFlow desktop and mobile shells.

mod ble;
#[cfg(feature = "native-ble")]
mod native_ble;
mod platform;
mod transport;

pub use ble::{
    StaticTauriBleDiscoveryProvider, TauriBleDiscoveryController, TauriBleDiscoveryProvider,
    TauriBleDiscoveryState,
};
#[cfg(feature = "native-ble")]
pub use native_ble::NativeBlePlatformProvider;
pub use platform::{
    TauriBleConnectionState, TauriBlePlatformProvider, UnsupportedTauriBlePlatformProvider,
};
pub use transport::{
    TauriBleOutputCommand, TauriBleOutputEvent, TauriBleOutputSink, TauriBleOutputStats,
    TauriBleSubscriptionRequest, TauriBleTransport, TauriBleTransportProvider,
    TauriBleWriteRequest, UnsupportedTauriBleTransportProvider,
};
