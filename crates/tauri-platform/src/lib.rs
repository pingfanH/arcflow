#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Tauri 2 platform adapters shared by ArcFlow desktop and mobile shells.

mod ble;
mod platform;
mod transport;

pub use ble::{
    StaticTauriBleDiscoveryProvider, TauriBleDiscoveryController, TauriBleDiscoveryProvider,
    TauriBleDiscoveryState,
};
pub use platform::{TauriBlePlatformProvider, UnsupportedTauriBlePlatformProvider};
pub use transport::{
    TauriBleOutputCommand, TauriBleOutputEvent, TauriBleOutputSink, TauriBleOutputStats,
    TauriBleSubscriptionRequest, TauriBleTransport, TauriBleTransportProvider,
    TauriBleWriteRequest, UnsupportedTauriBleTransportProvider,
};
