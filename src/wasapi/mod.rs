mod audio_client;
mod device;
mod errors;
mod events;
mod types;
mod utils;
mod wave_format;

// pub use audio_client::{
//   AudioCaptureClient, AudioClient, AudioClock, AudioRenderClient, AudioSessionControl, BufferFlags,
//   Handle,
// };
pub use device::{Device, DeviceCollection};
pub use errors::WasapiError;
// pub use events::EventCallbacks;
// pub use types::{DeviceState, Direction, Role, SampleType, SessionState, ShareMode};
pub use types::{Direction, SampleType, SessionState, ShareMode};
pub use utils::{get_default_device, initialize_mta};
pub use wave_format::WaveFormat;
// pub use utils::{
//   calculate_period_100ns, deinitialize, get_default_device, get_default_device_for_role,
//   initialize_mta, initialize_sta,
// };
// pub use wave_format::{WaveFormat, make_channelmasks};

pub(crate) type WasapiRes<T> = Result<T, errors::WasapiError>;
