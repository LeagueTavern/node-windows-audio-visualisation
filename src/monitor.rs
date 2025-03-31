use cpal::{
  traits::{DeviceTrait, StreamTrait},
  BufferSize, SampleRate, Stream, StreamConfig,
};
use std::cell::RefCell;

use crate::{
  fft::FFT,
  get_default_output_device,
  listener::Listener,
  utils::{build_device, get_native_device, visualize_fft_data},
};

#[napi(js_name = "AudioMonitor")]
pub struct AudioMonitor {
  listener: Listener,
  stream: RefCell<Option<Stream>>,
  fft: RefCell<FFT>,
}

#[napi]
impl AudioMonitor {
  #[napi(constructor)]
  pub fn new() -> Self {
    let fft = FFT::default();
    let listener = Listener::new();

    Self {
      listener,
      stream: RefCell::new(None),
      fft: RefCell::new(fft),
    }
  }

  #[napi]
  pub fn set_device(&self, id: String) -> () {
    let native = match get_native_device(id.clone()) {
      Some(device) => device,
      None => {
        eprintln!("Failed to get device");
        return;
      }
    };

    let device = match build_device(&native, id, false) {
      Some(info) => info,
      None => {
        eprintln!("Failed to build device info");
        return;
      }
    };

    let config = StreamConfig {
      channels: 2,
      sample_rate: SampleRate(device.sample_rate),
      buffer_size: device
        .buffer_size
        .map(BufferSize::Fixed)
        .unwrap_or(BufferSize::Default),
    };

    self.ensure_stream_stopped();

    let data_cb = self.listener.callback().get();
    let err_cb = |err| eprintln!("Stream error: {}", err);

    match native.build_input_stream(&config, data_cb, err_cb, None) {
      Ok(stream) => {
        *self.stream.borrow_mut() = Some(stream);
      }
      Err(e) => {
        eprintln!("Failed to build input stream: {}", e);
      }
    }
  }

  #[napi]
  pub fn play(&self) -> () {
    if self.stream.borrow().is_none() {
      match get_default_output_device() {
        Ok(Some(device)) => self.set_device(device.id.clone()),
        Ok(None) => {
          eprintln!("No default audio device found");
          return;
        }
        Err(err) => {
          eprintln!("Failed to get default device: {}", err);
          return;
        }
      }
    }

    self.with_stream(|stream| stream.play(), "play stream");
  }

  #[napi]
  pub fn pause(&self) -> () {
    self.with_stream(|stream| stream.pause(), "pause stream");
  }

  #[napi(ts_args_type = "bands?: number, decay?: number, size?: number")]
  pub fn get_spectrum(
    &self,
    bands: Option<u32>,
    decay: Option<f64>,
    size: Option<u32>,
  ) -> Vec<f32> {
    let mut binding = self.fft.borrow_mut();

    let size = size.unwrap_or(1024);
    let decay = decay.unwrap_or(12.0);
    let bands = bands.unwrap_or(64);
    let buffer = &self.listener.get_buffer();
    let result = &binding.process(&buffer, size as usize);

    visualize_fft_data(result, bands as usize, decay as f32)
  }

  fn ensure_stream_stopped(&self) {
    let mut stream_ref = self.stream.borrow_mut();
    if let Some(stream) = stream_ref.take() {
      if let Err(e) = stream.pause() {
        eprintln!("Warning: Failed to pause existing stream: {}", e);
      }
    }
  }

  fn with_stream<F, E>(&self, operation: F, operation_name: &str)
  where
    F: FnOnce(&Stream) -> Result<(), E>,
    E: std::fmt::Display,
  {
    let mut stream_ref = self.stream.borrow_mut();
    if let Some(stream) = stream_ref.take() {
      match operation(&stream) {
        Ok(_) => {
          *stream_ref = Some(stream);
        }
        Err(err) => {
          eprintln!("Failed to {}: {}", operation_name, err);
          *stream_ref = Some(stream);
        }
      }
    } else {
      eprintln!("Stream unavailable for {}", operation_name);
    }
  }
}
