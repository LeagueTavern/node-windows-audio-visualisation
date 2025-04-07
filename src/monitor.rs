use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::fft;
use crate::utils::{extract_float_samples, get_output_device_by_id};
use crate::wasapi::*;
use log::{debug, error, info};
use napi::{Error, Result, Status};
use napi_derive::napi;

type AudioData = Vec<f32>;

#[napi(js_name = "AudioMonitor")]
pub struct AudioMonitor {
  chunk_size: usize,
  device_id: Option<String>,
  spectrum: Arc<Mutex<AudioData>>,
  running: Arc<Mutex<bool>>,
  worker_handle: Option<JoinHandle<()>>,
}

#[napi]
impl AudioMonitor {
  #[napi(constructor)]
  pub fn new() -> Self {
    AudioMonitor {
      chunk_size: 2048, // 默认值
      device_id: None,
      spectrum: Arc::new(Mutex::new(Vec::new())),
      running: Arc::new(Mutex::new(false)),
      worker_handle: None,
    }
  }

  #[napi(ts_args_type = "deviceId?: string")]
  pub fn set_device(&mut self, device_id: Option<String>) {
    if self.device_id == device_id {
      return;
    }

    if self.running() {
      self.stop();
    }

    self.device_id = device_id;
  }

  #[napi(ts_args_type = "chunkSize?: number")]
  pub fn start(&mut self, chunk_size: Option<u32>) -> Result<()> {
    // 确保任何现有的播放被暂停
    self.stop();

    // 更新 chunk_size（如果提供）
    if let Some(size) = chunk_size {
      self.chunk_size = size as usize;
    }

    // 设置运行状态为 true
    *self
      .running
      .lock()
      .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))? = true;

    // 更新设备ID（如果需要，将使用默认设备）
    self.update_device_id();

    // 创建通信通道
    let (tx_capt, rx_capt): (SyncSender<AudioData>, Receiver<AudioData>) = mpsc::sync_channel(10);

    // 复制需要传递给工作线程的值
    let running = Arc::clone(&self.running);
    let spectrum = Arc::clone(&self.spectrum);
    let chunk_size = self.chunk_size;
    let device_id = self.device_id.clone();

    // 创建工作线程
    self.worker_handle = match spawn_audio_monitor_thread(
      rx_capt, tx_capt, running, spectrum, chunk_size, device_id,
    ) {
      Ok(handle) => Some(handle),
      Err(e) => return Err(Error::new(Status::GenericFailure, e.to_string())),
    };

    Ok(())
  }

  #[napi]
  pub fn stop(&mut self) {
    if !self.running() {
      return;
    }

    // 设置运行状态为 false
    if let Ok(mut running) = self.running.lock() {
      *running = false;
    }

    // 等待工作线程完成
    if let Some(handle) = self.worker_handle.take() {
      let _ = handle.join();
    }
  }

  #[napi]
  pub fn get_spectrum(&self, num_bands: u32) -> Result<Vec<f32>> {
    self
      .spectrum
      .lock()
      .map(|spectrum| {
        if spectrum.is_empty() {
          vec![0.0; num_bands as usize]
        } else {
          fft::analyze_spectrum(&spectrum, num_bands as usize)
        }
      })
      .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))
  }

  #[napi(getter)]
  pub fn current_device_id(&self) -> Result<Option<String>> {
    Ok(self.device_id.clone())
  }

  #[napi(getter)]
  pub fn running(&self) -> bool {
    self.running.lock().map(|running| *running).unwrap_or(false)
  }

  #[napi(getter)]
  pub fn chunk_size(&self) -> u32 {
    self.chunk_size as u32
  }

  fn update_device_id(&mut self) {
    // 检查指定的设备是否存在
    if let Some(id) = &self.device_id {
      if get_output_device_by_id(id.clone()).is_some() {
        debug!("Using specified device: {}", id);
        return;
      }
      debug!("Device not found: {}, will use default device", id);
    }

    // 获取默认设备作为备选
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
}

impl Drop for AudioMonitor {
  fn drop(&mut self) {
    self.stop();
  }
}

// 创建音频监控线程
fn spawn_audio_monitor_thread(
  rx_capt: Receiver<AudioData>,
  tx_capt: SyncSender<AudioData>,
  running: Arc<Mutex<bool>>,
  spectrum: Arc<Mutex<AudioData>>,
  chunk_size: usize,
  device_id: Option<String>,
) -> std::result::Result<JoinHandle<()>, std::io::Error> {
  thread::Builder::new()
    .name("AudioMonitor".to_string())
    .spawn(move || {
      // 创建音频捕获线程
      let capture_thread = thread::Builder::new()
        .name("LoopbackCapture".to_string())
        .spawn(move || {
          if let Err(err) = loopback_capture_loop(tx_capt, chunk_size, device_id) {
            error!("Loopback capture failed with error {}", err);
          }
        })
        .unwrap_or_else(|e| {
          error!("Failed to spawn capture thread: {}", e);
          panic!("Critical thread creation failure");
        });

      // 主循环处理接收到的音频数据
      process_audio_data(rx_capt, running, spectrum);

      // 等待捕获线程结束
      if let Err(e) = capture_thread.join() {
        error!("Error joining capture thread: {:?}", e);
      }
    })
}

// 处理音频数据的主循环
fn process_audio_data(
  rx_capt: Receiver<AudioData>,
  running: Arc<Mutex<bool>>,
  spectrum: Arc<Mutex<AudioData>>,
) {
  while match running.lock() {
    Ok(guard) => *guard,
    Err(_) => false,
  } {
    match rx_capt.recv_timeout(Duration::from_millis(100)) {
      Ok(samples) => {
        if let Ok(mut spec) = spectrum.lock() {
          *spec = samples;
        }
      }
      Err(mpsc::RecvTimeoutError::Timeout) => continue,
      Err(mpsc::RecvTimeoutError::Disconnected) => break,
    }
  }
}

fn loopback_capture_loop(
  tx_capt: SyncSender<AudioData>,
  chunk_size: usize,
  device_id: Option<String>,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
  // 获取音频设备
  let device = get_audio_device(device_id)?;

  // 初始化音频客户端
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

  // 样本队列，缓存从设备读取的数据
  let mut sample_queue: VecDeque<u8> =
    VecDeque::with_capacity(100 * blockalign as usize * (1024 + 2 * buffer_frame_count as usize));

  // 开始音频流
  audio_client.start_stream()?;
  info!("Audio capture started");

  loop {
    // 当积累了足够的样本时，处理并发送它们
    if sample_queue.len() >= (blockalign as usize * chunk_size) {
      let float_samples = extract_float_samples(&mut sample_queue, chunk_size, blockalign as usize);

      // 发送处理好的样本，如果接收端已关闭则退出循环
      if tx_capt.send(float_samples).is_err() {
        break;
      }
    }

    // 从设备读取数据到队列
    capture_client.read_from_device_to_deque(&mut sample_queue)?;

    // 等待事件或超时
    if h_event.wait_for_event(100).is_err() {
      continue;
    }
  }

  // 停止音频流
  let _ = audio_client.stop_stream();
  info!("Audio capture stopped");

  Ok(())
}

// 获取音频设备，优先使用指定ID的设备，如果不存在则使用默认设备
fn get_audio_device(
  device_id: Option<String>,
) -> std::result::Result<Device, Box<dyn std::error::Error>> {
  match device_id {
    Some(id) => match get_output_device_by_id(id.clone()) {
      Some(device) => {
        debug!("Successfully got device: {}", id);
        Ok(device)
      }
      None => {
        debug!("Device not found during capture: {}, using default", id);
        get_default_device(&Direction::Render).map_err(|e| e.into())
      }
    },
    None => {
      debug!("No device ID specified, using default");
      get_default_device(&Direction::Render).map_err(|e| e.into())
    }
  }
}
