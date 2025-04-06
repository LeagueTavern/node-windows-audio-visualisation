use crate::types::AudioDevice;
use crate::wasapi::{Device, DeviceCollection, Direction, get_default_device, initialize_mta};
use napi::Result;
use napi_derive::napi;

pub fn get_output_device_by_id(id: String) -> Option<Device> {
  for device in &DeviceCollection::new(&Direction::Render).unwrap() {
    let dev = device.unwrap();
    if dev.get_id().unwrap() == id {
      return Some(dev);
    }
  }
  None
}

#[napi]
pub fn get_all_output_devices() -> Result<Vec<AudioDevice>> {
  let mut output_devices = Vec::new();
  let default_output_device = get_default_output_device()?;

  for device in &DeviceCollection::new(&Direction::Render).unwrap() {
    let dev = device.unwrap();
    let id = dev.get_id().unwrap();
    let name = dev.get_friendlyname().unwrap();
    let state = dev.get_state().unwrap() as u32;
    let is_default = default_output_device.as_ref().map_or(false, |d| d.id == id);

    output_devices.push(AudioDevice {
      id,
      name,
      state,
      is_default,
    });
  }

  Ok(output_devices)
}

#[napi]
pub fn get_default_output_device() -> Result<Option<AudioDevice>> {
  initialize_mta().unwrap();

  let device = match get_default_device(&Direction::Render) {
    Ok(device) => device,
    Err(_) => return Ok(None),
  };

  let id = device.get_id().unwrap();
  let name = device.get_friendlyname().unwrap();
  let state = device.get_state().unwrap() as u32;

  Ok(Some(AudioDevice {
    id,
    name,
    state,
    is_default: true,
  }))
}
