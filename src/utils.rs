use crate::types::AudioDevice;
use cpal::{
  traits::{DeviceTrait, HostTrait},
  BufferSize, SampleFormat,
};
use napi::Result;
use napi_derive::napi;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub fn visualize_fft_data(fft_data: &[f32], num_bands: usize, decay: f32) -> Vec<f32> {
  let mut bands = Vec::with_capacity(num_bands);
  let band_width = fft_data.len() / num_bands;

  for i in 0..num_bands {
    let start = i * band_width;
    let end = (i + 1) * band_width;
    let weight = (num_bands - i) as f32 / num_bands as f32;
    let mut sum = 0.0;

    for j in start..end {
      if j < fft_data.len() {
        sum += fft_data[j].abs() * weight;
        // sum += fft_data[j] * weight;
      }
    }

    let avg = if end > start {
      sum / (end - start) as f32
    } else {
      0.0
    };

    let value = exponential_decay(avg, decay, 1.0);
    bands.push(value);
  }

  bands
}

pub fn exponential_decay(x: f32, k: f32, max: f32) -> f32 {
  max * (1.0 - (-k * x).exp())
}

pub fn build_device(device: &cpal::Device, id: String, is_default: bool) -> Option<AudioDevice> {
  let name = device.name().ok()?;

  let input = device.default_input_config();
  let output = device.default_output_config();
  let supported = input.or(output).ok()?;

  if !(supported.channels() == 2 && supported.sample_format() == SampleFormat::F32) {
    return None;
  }

  let config = supported.config();
  let sample_rate = config.sample_rate.0;
  let buffer_size = match config.buffer_size {
    BufferSize::Fixed(size) => Some(size),
    BufferSize::Default => None,
  };

  Some(AudioDevice {
    id,
    name,
    sample_rate,
    buffer_size,
    is_default,
  })
}

pub fn get_native_device(id: String) -> Option<cpal::Device> {
  let host = cpal::default_host();
  let devices = host.devices().ok()?;

  for device in devices {
    if generate_stable_device_id(&device) == id {
      return Some(device);
    }
  }

  None
}

pub fn generate_stable_device_id(device: &cpal::Device) -> String {
  let name = device.name().unwrap_or_else(|_| String::from("未知设备"));
  let mut stable_info = String::new();

  if let Ok(supported_configs) = device.supported_output_configs() {
    for config in supported_configs {
      let min_rate = config.min_sample_rate().0;
      let max_rate = config.max_sample_rate().0;
      let channels = config.channels();
      let format = format!("{:?}", config.sample_format());

      stable_info.push_str(&format!(
        "{}_{}_{}_{}",
        format, channels, min_rate, max_rate
      ));
    }
  }

  if stable_info.is_empty() {
    stable_info = device
      .default_output_config()
      .map(|config| {
        format!(
          "{:?}_{}_{}_{}",
          config.sample_format(),
          config.channels(),
          config.sample_rate().0,
          name
        )
      })
      .unwrap_or_else(|_| "generic_audio_device".to_string());
  }

  if stable_info.is_empty() {
    stable_info = "generic_audio_device".to_string();
  }

  let mut hasher = DefaultHasher::new();
  let combined = format!("{}-{}", name, stable_info);
  combined.hash(&mut hasher);
  format!("{:x}", hasher.finish())
}

#[napi]
pub fn get_all_output_devices() -> Result<Vec<AudioDevice>> {
  let host = cpal::default_host();

  let devices = match host.devices() {
    Ok(devices) => devices,
    Err(_) => return Ok(Vec::new()),
  };

  let mut output_devices = Vec::new();
  let default_device = get_default_output_device().unwrap_or(None);

  for device in devices {
    let id = generate_stable_device_id(&device);
    let is_default = default_device.as_ref().map_or(false, |d| d.id == id);

    if let Some(audio_device) = build_device(&device, id.clone(), is_default) {
      output_devices.push(audio_device);
    }
  }

  Ok(output_devices)
}

#[napi]
pub fn get_default_output_device() -> Result<Option<AudioDevice>> {
  let host = cpal::default_host();

  let device = match host.default_output_device() {
    Some(device) => device,
    None => return Ok(None),
  };

  Ok(build_device(
    &device,
    generate_stable_device_id(&device),
    true,
  ))
}
