use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::fft;
use crate::utils::get_output_device_by_id;
use crate::wasapi::*;
use log::{debug, error};
use napi::{Error, Result, Status};
use napi_derive::napi;

#[napi(js_name = "AudioMonitor")]
pub struct AudioMonitor {
  chunk_size: usize,
  device_id: Option<String>,
  spectrum: Arc<Mutex<Vec<f32>>>,
  running: Arc<Mutex<bool>>,
  worker_handle: Option<JoinHandle<()>>,
}

#[napi]
impl AudioMonitor {
  #[napi(constructor, ts_args_type = "chunkSize?: number")]
  pub fn new(chunk_size: Option<u32>) -> Self {
    let chunk_size = chunk_size.unwrap_or(2048) as usize;
    AudioMonitor {
      chunk_size,
      device_id: None,
      spectrum: Arc::new(Mutex::new(Vec::new())),
      running: Arc::new(Mutex::new(false)),
      worker_handle: None,
    }
  }

  #[napi(ts_args_type = "deviceId?: string")]
  pub fn set_device(&mut self, device_id: Option<String>) {
    if self.running() {
      self.pause();
    }

    self.device_id = device_id;
  }

  #[napi]
  pub fn play(&mut self) -> Result<()> {
    self.pause();

    match self.running.lock() {
      Ok(mut running) => *running = true,
      Err(e) => return Err(Error::new(Status::GenericFailure, e.to_string())),
    }

    let (tx_capt, rx_capt): (SyncSender<Vec<f32>>, Receiver<Vec<f32>>) = mpsc::sync_channel(10);

    self.update_device_id();

    let running = Arc::clone(&self.running);
    let spectrum = Arc::clone(&self.spectrum);
    let chunk_size = self.chunk_size;
    let device_id = self.device_id.clone();

    self.worker_handle = match thread::Builder::new()
      .name("AudioMonitor".to_string())
      .spawn(move || {
        let capture_thread = thread::Builder::new()
          .name("LoopbackCapture".to_string())
          .spawn(move || {
            if let Err(err) = loopback_capture_loop(tx_capt, chunk_size, device_id) {
              error!("Loopback capture failed with error {}", err);
            }
          });

        while match running.lock() {
          Ok(guard) => *guard,
          Err(_) => false,
        } {
          match rx_capt.recv_timeout(Duration::from_millis(100)) {
            Ok(samples) => {
              let new_spectrum = samples;
              if let Ok(mut spec) = spectrum.lock() {
                *spec = new_spectrum.clone();
              }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
          }
        }

        if let Ok(handle) = capture_thread {
          let _ = handle.join();
        }
      }) {
      Ok(handle) => Some(handle),
      Err(e) => return Err(Error::new(Status::GenericFailure, e.to_string())),
    };

    Ok(())
  }

  fn update_device_id(&mut self) {
    let _: Option<String> = match &self.device_id {
      Some(id) if get_output_device_by_id(id.clone()).is_some() => {
        debug!("Using specified device: {}", id);
        return;
      }
      Some(id) => {
        debug!("Device not found: {}, will use default device", id);
        None
      }
      None => None,
    };

    // Get default device as fallback
    match get_default_device(&Direction::Render) {
      Ok(device) => match device.get_id() {
        Ok(default_id) => {
          debug!("Using default device: {}", default_id);
          self.device_id = Some(default_id);
        }
        Err(_) => {
          debug!("Unable to get default device ID");
          self.device_id = None;
        }
      },
      Err(e) => {
        error!("Unable to get default device: {}", e);
        self.device_id = None;
      }
    }
  }

  #[napi]
  pub fn pause(&mut self) {
    if let Ok(mut running) = self.running.lock() {
      *running = false;
    }

    if let Some(handle) = self.worker_handle.take() {
      let _ = handle.join();
    }
  }

  #[napi]
  pub fn get_spectrum(&self, num_bands: u32) -> Result<Vec<f32>> {
    match self.spectrum.lock() {
      Ok(spectrum) => {
        if spectrum.is_empty() {
          return Ok(vec![0.0; num_bands as usize]);
        }
        Ok(fft::analyze_spectrum(&spectrum, num_bands as usize))
      }
      Err(e) => Err(Error::new(Status::GenericFailure, e.to_string())),
    }
  }

  #[napi(getter)]
  pub fn current_device_id(&self) -> napi::Result<Option<String>> {
    Ok(self.device_id.clone())
  }

  #[napi(getter)]
  pub fn running(&self) -> bool {
    match self.running.lock() {
      Ok(running) => *running,
      Err(_) => false,
    }
  }
}

impl Drop for AudioMonitor {
  fn drop(&mut self) {
    self.pause();
  }
}

fn loopback_capture_loop(
  tx_capt: SyncSender<Vec<f32>>,
  chunk_size: usize,
  device_id: Option<String>,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
  let device = match device_id {
    Some(id) => match get_output_device_by_id(id.clone()) {
      Some(device) => {
        debug!("Successfully got device: {}", id);
        device
      }
      None => {
        debug!("Device not found during capture: {}, using default", id);
        get_default_device(&Direction::Render)?
      }
    },
    None => {
      debug!("No device ID specified, using default");
      get_default_device(&Direction::Render)?
    }
  };

  let mut audio_client = device.get_iaudioclient()?;
  let desired_format = WaveFormat::new(32, 32, &SampleType::Float, 44100, 2, None);
  let blockalign = desired_format.get_blockalign();

  debug!("Desired capture format: {:?}", desired_format);
  let (_, min_time) = audio_client.get_periods()?;

  audio_client.initialize_client(
    &desired_format,
    min_time,
    &Direction::Capture,
    &ShareMode::Shared,
    true,
  )?;

  let h_event = audio_client.set_get_eventhandle()?;
  let buffer_frame_count = audio_client.get_bufferframecount()?;
  let capture_client = audio_client.get_audiocaptureclient()?;

  let mut sample_queue: VecDeque<u8> =
    VecDeque::with_capacity(100 * blockalign as usize * (1024 + 2 * buffer_frame_count as usize));

  audio_client.start_stream()?;

  loop {
    if sample_queue.len() >= (blockalign as usize * chunk_size) {
      let mut float_samples = vec![0.0f32; chunk_size];

      for i in 0..chunk_size {
        let offset = i * blockalign as usize;
        if offset + 4 <= sample_queue.len() {
          let bytes = [
            sample_queue[offset],
            sample_queue[offset + 1],
            sample_queue[offset + 2],
            sample_queue[offset + 3],
          ];
          let sample = f32::from_le_bytes(bytes);
          float_samples[i] = sample;

          for _ in 0..4 {
            sample_queue.pop_front();
          }

          if offset + 8 <= sample_queue.len() {
            for _ in 0..4 {
              sample_queue.pop_front();
            }
          }
        }
      }

      if tx_capt.send(float_samples).is_err() {
        break;
      }
    }

    capture_client.read_from_device_to_deque(&mut sample_queue)?;

    if h_event.wait_for_event(100).is_err() {
      continue;
    }
  }

  let _ = audio_client.stop_stream();
  Ok(())
}
