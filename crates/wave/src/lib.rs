#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Safe wave plans and protocol conversion for ArcFlow.

pub mod coyote;
pub mod error;

pub use coyote::{ChannelOutput, ChannelWindow, CoyoteV3Window, WavePoint};
pub use error::WaveError;
