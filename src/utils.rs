use crate::types::AudioDevice;
use crate::wasapi::{get_default_device, initialize_mta, Device, DeviceCollection, Direction};
use napi::Result;
use napi_derive::napi;
use std::collections::VecDeque;

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

pub fn extract_float_samples(
  sample_queue: &mut VecDeque<u8>,
  chunk_size: usize,
  blockalign: usize,
) -> Vec<f32> {
  let mut float_samples = vec![0.0f32; chunk_size];

  for i in 0..chunk_size {
    let offset = i * blockalign;
    if offset + 4 <= sample_queue.len() {
      // 读取一个浮点样本（4字节）
      let bytes = [
        sample_queue[offset],
        sample_queue[offset + 1],
        sample_queue[offset + 2],
        sample_queue[offset + 3],
      ];
      float_samples[i] = f32::from_le_bytes(bytes);

      // 移除已处理的字节
      for _ in 0..4 {
        sample_queue.pop_front();
      }

      // 如果是立体声，跳过第二个通道数据
      if offset + 8 <= sample_queue.len() {
        for _ in 0..4 {
          sample_queue.pop_front();
        }
      }
    }
  }

  float_samples
}
