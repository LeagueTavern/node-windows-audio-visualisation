use log::debug;
use windows::core::HRESULT;
use windows::Win32::Media::Audio::{IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::System::Com::{
  CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
  COINIT_MULTITHREADED,
};

use crate::wasapi::device::Device;
use crate::wasapi::types::{Direction, Role};
use crate::wasapi::WasapiRes;

/// Initializes COM for use by the calling thread for the multi-threaded apartment (MTA).
pub fn initialize_mta() -> HRESULT {
  unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
}

/// Initializes COM for use by the calling thread for a single-threaded apartment (STA).
pub fn initialize_sta() -> HRESULT {
  unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
}

/// Close the COM library on the current thread.
pub fn deinitialize() {
  unsafe { CoUninitialize() }
}

/// Calculate a period in units of 100ns that corresponds to the given number of buffer frames at the given sample rate.
/// See the [IAudioClient documentation](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-initialize#remarks).
pub fn calculate_period_100ns(frames: i64, samplerate: i64) -> i64 {
  ((10000.0 * 1000.0 / samplerate as f64 * frames as f64) + 0.5) as i64
}

/// Get the default playback or capture device for the console role
pub fn get_default_device(direction: &Direction) -> WasapiRes<Device> {
  get_default_device_for_role(direction, &Role::Console)
}

/// Get the default playback or capture device for a specific role
pub fn get_default_device_for_role(direction: &Direction, role: &Role) -> WasapiRes<Device> {
  let dir = direction.into();
  let e_role = role.into();

  let enumerator: IMMDeviceEnumerator =
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
  let device = unsafe { enumerator.GetDefaultAudioEndpoint(dir, e_role)? };

  let dev = Device {
    device,
    direction: *direction,
  };
  debug!("default device {:?}", dev.get_friendlyname());
  Ok(dev)
}
