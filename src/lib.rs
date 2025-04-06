#![deny(clippy::all)]

#[macro_use]
extern crate napi_derive;

mod fft;
mod monitor;
mod types;
mod utils;
mod wasapi;

pub use crate::types::AudioDevice;
pub use crate::utils::{get_all_output_devices, get_default_output_device};
