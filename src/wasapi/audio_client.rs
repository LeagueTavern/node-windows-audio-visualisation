use num_integer::Integer;
use std::cmp;
use std::collections::VecDeque;
use std::mem::{size_of, ManuallyDrop};
use std::ops::Deref;
use std::pin::Pin;
use std::rc::Weak;
use std::sync::{Arc, Condvar, Mutex};
use std::{ptr, slice};

use crate::wasapi::events::{AudioSessionEvents, EventCallbacks};
use crate::wasapi::types::{Direction, SessionState, ShareMode};
use crate::wasapi::utils::calculate_period_100ns;
use crate::wasapi::wave_format::{make_channelmasks, WaveFormat};
use crate::wasapi::WasapiError;
use crate::wasapi::WasapiRes;
use windows::core::{implement, IUnknown, Interface, Ref, HRESULT, PCSTR};
use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
  ActivateAudioInterfaceAsync, AudioSessionStateActive, AudioSessionStateExpired,
  AudioSessionStateInactive, IActivateAudioInterfaceAsyncOperation,
  IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
  IAudioCaptureClient, IAudioClient, IAudioClock, IAudioRenderClient, IAudioSessionControl,
  IAudioSessionEvents, AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY, AUDCLNT_BUFFERFLAGS_SILENT,
  AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR, AUDCLNT_SHAREMODE_EXCLUSIVE, AUDCLNT_SHAREMODE_SHARED,
  AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
  AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
  AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_PARAMS_0,
  AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
  PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
  PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
};
use windows::Win32::System::Com::StructuredStorage::{
  PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
};
use windows::Win32::System::Com::BLOB;
use windows::Win32::System::Threading::{CreateEventA, WaitForSingleObject};
use windows::Win32::System::Variant::VT_BLOB;
use windows::{
  Win32::Media::Audio::{WAVEFORMATEX, WAVEFORMATEXTENSIBLE},
  Win32::Media::KernelStreaming::WAVE_FORMAT_EXTENSIBLE,
};

#[implement(IActivateAudioInterfaceCompletionHandler)]
struct Handler(Arc<(Mutex<bool>, Condvar)>);

impl Handler {
  pub fn new(object: Arc<(Mutex<bool>, Condvar)>) -> Handler {
    Handler(object)
  }
}

impl IActivateAudioInterfaceCompletionHandler_Impl for Handler_Impl {
  fn ActivateCompleted(
    &self,
    _activateoperation: Ref<IActivateAudioInterfaceAsyncOperation>,
  ) -> windows::core::Result<()> {
    let (lock, cvar) = &*self.0;
    let mut completed = lock.lock().unwrap();
    *completed = true;
    drop(completed);
    cvar.notify_one();
    Ok(())
  }
}

/// Struct wrapping an [IAudioClient](https://docs.microsoft.com/en-us/windows/win32/api/audioclient/nn-audioclient-iaudioclient).
pub struct AudioClient {
  pub(crate) client: IAudioClient,
  pub(crate) direction: Direction,
  pub(crate) sharemode: Option<ShareMode>,
  pub(crate) bytes_per_frame: Option<usize>,
}

impl AudioClient {
  /// Creates a loopback capture [AudioClient] for a specific process.
  ///
  /// `include_tree` is equivalent to [PROCESS_LOOPBACK_MODE](https://learn.microsoft.com/en-us/windows/win32/api/audioclientactivationparams/ne-audioclientactivationparams-process_loopback_mode).
  /// If true, the loopback capture client will capture audio from the target process and all its child processes,
  /// if false only audio from the target process is captured.
  ///
  /// On versions of Windows prior to Windows 10, the thread calling this function
  /// must called in a COM Single-Threaded Apartment (STA).
  ///
  /// Additionally when calling [AudioClient::initialize_client] on the client returned by this method,
  /// the caller must use [Direction::Capture], and [ShareMode::Shared].
  /// Finally calls to [AudioClient::get_periods] do not work,
  /// however the period passed by the caller to [AudioClient::initialize_client] is irrelevant.
  ///
  /// # Non-functional methods
  /// In process loopback mode, the functionality of the AudioClient is limited.
  /// The following methods either do not work, or return incorrect results:
  /// * `get_mixformat` just returns `Not implemented`.
  /// * `is_supported` just returns `Not implemented` even if the format and mode work.
  /// * `is_supported_exclusive_with_quirks` just returns `Unable to find a supported format`.
  /// * `get_periods` just returns `Not implemented`.
  /// * `calculate_aligned_period_near` just returns `Not implemented` even for values that would later work.
  /// * `get_bufferframecount` returns huge values like 3131961357 but no error.
  /// * `get_current_padding` just returns `Not implemented`.
  /// * `get_available_space_in_frames` just returns `Client has not been initialised` even if it has.
  /// * `get_audiorenderclient` just returns `No such interface supported`.
  /// * `get_audiosessioncontrol` just returns `No such interface supported`.
  /// * `get_audioclock` just returns `No such interface supported`.
  /// * `get_sharemode` always returns `None` when it should return `Shared` after initialisation.
  ///
  /// # Example
  /// ```
  /// use wasapi::{WaveFormat, SampleType, AudioClient, Direction, ShareMode, initialize_mta};
  /// let desired_format = WaveFormat::new(32, 32, &SampleType::Float, 44100, 2, None);
  /// let hnsbufferduration = 200_000; // 20ms in hundreds of nanoseconds
  /// let autoconvert = true;
  /// let include_tree = false;
  /// let process_id = std::process::id();
  ///
  /// initialize_mta().ok().unwrap(); // Don't do this on a UI thread
  /// let mut audio_client = AudioClient::new_application_loopback_client(process_id, include_tree).unwrap();
  /// audio_client.initialize_client(&desired_format, hnsbufferduration, &Direction::Capture, &ShareMode::Shared, autoconvert).unwrap();
  /// ```
  pub fn new_application_loopback_client(process_id: u32, include_tree: bool) -> WasapiRes<Self> {
    unsafe {
      // Create audio client
      let mut audio_client_activation_params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
          ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
            TargetProcessId: process_id,
            ProcessLoopbackMode: if include_tree {
              PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE
            } else {
              PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE
            },
          },
        },
      };
      let pinned_params = Pin::new(&mut audio_client_activation_params);

      let raw_prop = PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
          Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
            vt: VT_BLOB,
            wReserved1: 0,
            wReserved2: 0,
            wReserved3: 0,
            Anonymous: PROPVARIANT_0_0_0 {
              blob: BLOB {
                cbSize: size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                pBlobData: pinned_params.get_mut() as *const _ as *mut _,
              },
            },
          }),
        },
      };

      let activation_prop = ManuallyDrop::new(raw_prop);
      let pinned_prop = Pin::new(activation_prop.deref());
      let activation_params = Some(pinned_prop.get_ref() as *const _);

      // Create completion handler
      let setup = Arc::new((Mutex::new(false), Condvar::new()));
      let callback: IActivateAudioInterfaceCompletionHandler = Handler::new(setup.clone()).into();

      // Activate audio interface
      let operation = ActivateAudioInterfaceAsync(
        VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
        &IAudioClient::IID,
        activation_params,
        &callback,
      )?;

      // Wait for completion
      let (lock, cvar) = &*setup;
      let mut completed = lock.lock().unwrap();
      while !*completed {
        completed = cvar.wait(completed).unwrap();
      }
      drop(completed);

      // Get audio client and result
      let mut audio_client: Option<IUnknown> = Default::default();
      let mut result: HRESULT = Default::default();
      operation.GetActivateResult(&mut result, &mut audio_client)?;

      // Ensure successful activation
      result.ok()?;
      // always safe to unwrap if result above is checked first
      let audio_client: IAudioClient = audio_client.unwrap().cast()?;

      Ok(AudioClient {
        client: audio_client,
        direction: Direction::Render,
        sharemode: Some(ShareMode::Shared),
        bytes_per_frame: None,
      })
    }
  }

  /// Get MixFormat of the device. This is the format the device uses in shared mode and should always be accepted.
  pub fn get_mixformat(&self) -> WasapiRes<WaveFormat> {
    let temp_fmt_ptr = unsafe { self.client.GetMixFormat()? };
    let temp_fmt = unsafe { *temp_fmt_ptr };
    let mix_format =
      if temp_fmt.cbSize == 22 && temp_fmt.wFormatTag as u32 == WAVE_FORMAT_EXTENSIBLE {
        unsafe {
          WaveFormat {
            wave_fmt: (temp_fmt_ptr as *const _ as *const WAVEFORMATEXTENSIBLE).read(),
          }
        }
      } else {
        WaveFormat::from_waveformatex(temp_fmt)?
      };
    Ok(mix_format)
  }

  /// Check if a format is supported.
  /// If it's directly supported, this returns Ok(None). If not, but a similar format is, then the nearest matching supported format is returned as Ok(Some(WaveFormat)).
  ///
  /// NOTE: For exclusive mode, this function may not always give the right result for 1- and 2-channel formats.
  /// From the [Microsoft documentation](https://docs.microsoft.com/en-us/windows/win32/coreaudio/device-formats#specifying-the-device-format):
  /// > For exclusive-mode formats, the method queries the device driver.
  /// > Some device drivers will report that they support a 1-channel or 2-channel PCM format if the format is specified by a stand-alone WAVEFORMATEX structure,
  /// > but will reject the same format if it is specified by a WAVEFORMATEXTENSIBLE structure.
  /// > To obtain reliable results from these drivers, exclusive-mode applications should call IsFormatSupported twice for each 1-channel or 2-channel PCM format.
  /// > One call should use a stand-alone WAVEFORMATEX structure to specify the format, and the other call should use a WAVEFORMATEXTENSIBLE structure to specify the same format.
  ///
  /// If the first call fails, use [WaveFormat::to_waveformatex] to get a copy of the WaveFormat in the simpler WAVEFORMATEX representation.
  /// Then call this function again with the new WafeFormat structure.
  /// If the driver then reports that the format is supported, use the original WaveFormat structure when calling [AudioClient::initialize_client].
  ///
  /// See also the helper function [is_supported_exclusive_with_quirks](AudioClient::is_supported_exclusive_with_quirks).
  pub fn is_supported(
    &self,
    wave_fmt: &WaveFormat,
    sharemode: &ShareMode,
  ) -> WasapiRes<Option<WaveFormat>> {
    let supported = match sharemode {
      ShareMode::Exclusive => {
        unsafe {
          self
            .client
            .IsFormatSupported(
              AUDCLNT_SHAREMODE_EXCLUSIVE,
              wave_fmt.as_waveformatex_ref(),
              None,
            )
            .ok()?
        };
        None
      }
      ShareMode::Shared => {
        let mut supported_format: *mut WAVEFORMATEX = std::ptr::null_mut();
        unsafe {
          self
            .client
            .IsFormatSupported(
              AUDCLNT_SHAREMODE_SHARED,
              wave_fmt.as_waveformatex_ref(),
              Some(&mut supported_format),
            )
            .ok()?
        };
        // Check if we got a pointer to a WAVEFORMATEX structure.
        if supported_format.is_null() {
          // The pointer is still null, thus the format is supported as is.
          None
        } else {
          // Read the structure
          let temp_fmt: WAVEFORMATEX = unsafe { supported_format.read() };
          let new_fmt =
            if temp_fmt.cbSize == 22 && temp_fmt.wFormatTag as u32 == WAVE_FORMAT_EXTENSIBLE {
              let temp_fmt_ext: WAVEFORMATEXTENSIBLE =
                unsafe { (supported_format as *const _ as *const WAVEFORMATEXTENSIBLE).read() };
              WaveFormat {
                wave_fmt: temp_fmt_ext,
              }
            } else {
              WaveFormat::from_waveformatex(temp_fmt)?
            };
          Some(new_fmt)
        }
      }
    };
    Ok(supported)
  }

  /// A helper function for checking if a format is supported.
  /// It calls `is_supported` several times with different options
  /// in order to find a format that the device accepts.
  ///
  /// The alternatives it tries are:
  /// - The format as given.
  /// - If one or two channels, try with the format as WAVEFORMATEX.
  /// - Try with different channel masks:
  ///   - If channels <= 8: Recommended mask(s) from ksmedia.h.
  ///   - If channels <= 18: Simple mask.
  ///   - Zero mask.
  ///
  /// If an accepted format is found, this is returned.
  /// An error means no accepted format was found.
  pub fn is_supported_exclusive_with_quirks(&self, wave_fmt: &WaveFormat) -> WasapiRes<WaveFormat> {
    let mut wave_fmt = wave_fmt.clone();
    let supported_direct = self.is_supported(&wave_fmt, &ShareMode::Exclusive);
    if supported_direct.is_ok() {
      return Ok(wave_fmt);
    }
    if wave_fmt.get_nchannels() <= 2 {
      let wave_formatex = wave_fmt.to_waveformatex().unwrap();
      if self
        .is_supported(&wave_formatex, &ShareMode::Exclusive)
        .is_ok()
      {
        return Ok(wave_formatex);
      }
    }
    let masks = make_channelmasks(wave_fmt.get_nchannels() as usize);
    for mask in masks {
      wave_fmt.wave_fmt.dwChannelMask = mask;
      if self.is_supported(&wave_fmt, &ShareMode::Exclusive).is_ok() {
        return Ok(wave_fmt);
      }
    }
    Err(WasapiError::UnsupportedFormat)
  }

  /// Get default and minimum periods in 100-nanosecond units
  pub fn get_periods(&self) -> WasapiRes<(i64, i64)> {
    let mut def_time = 0;
    let mut min_time = 0;
    unsafe {
      self
        .client
        .GetDevicePeriod(Some(&mut def_time), Some(&mut min_time))?
    };
    Ok((def_time, min_time))
  }

  /// Helper function for calculating a period size in 100-nanosecond units that is near a desired value,
  /// and always larger than the minimum value supported by the device.
  /// The returned value leads to a device buffer size that is aligned both to the frame size of the format,
  /// and the optional align_bytes value.
  /// This parameter is used for devices that require the buffer size to be a multiple of a certain number of bytes.
  /// Give None, Some(0) or Some(1) if the device has no special requirements for the alignment.
  /// For example, all devices following the Intel High Definition Audio specification require buffer sizes in multiples of 128 bytes.
  ///
  /// See also the `playnoise_exclusive` example.
  pub fn calculate_aligned_period_near(
    &self,
    desired_period: i64,
    align_bytes: Option<u32>,
    wave_fmt: &WaveFormat,
  ) -> WasapiRes<i64> {
    let (_default_period, min_period) = self.get_periods()?;
    let adjusted_desired_period = cmp::max(desired_period, min_period);
    let frame_bytes = wave_fmt.get_blockalign();
    let period_alignment_bytes = match align_bytes {
      Some(0) => frame_bytes,
      Some(bytes) => frame_bytes.lcm(&bytes),
      None => frame_bytes,
    };
    let period_alignment_frames = period_alignment_bytes as i64 / frame_bytes as i64;
    let desired_period_frames =
      (adjusted_desired_period as f64 * wave_fmt.get_samplespersec() as f64 / 10000000.0).round()
        as i64;
    let min_period_frames =
      (min_period as f64 * wave_fmt.get_samplespersec() as f64 / 10000000.0).ceil() as i64;
    let mut nbr_segments = desired_period_frames / period_alignment_frames;
    if nbr_segments * period_alignment_frames < min_period_frames {
      // Add one segment if the value got rounded down below the minimum
      nbr_segments += 1;
    }
    let aligned_period = calculate_period_100ns(
      period_alignment_frames * nbr_segments,
      wave_fmt.get_samplespersec() as i64,
    );
    Ok(aligned_period)
  }

  /// Initialize an [IAudioClient] for the given direction, sharemode and format.
  /// Setting `convert` to true enables automatic samplerate and format conversion, meaning that almost any format will be accepted.
  pub fn initialize_client(
    &mut self,
    wavefmt: &WaveFormat,
    period: i64,
    direction: &Direction,
    sharemode: &ShareMode,
    convert: bool,
  ) -> WasapiRes<()> {
    if sharemode == &ShareMode::Exclusive && convert {
      return Err(WasapiError::AutomaticFormatConversionInExclusiveMode);
    }
    let mut streamflags = match (&self.direction, direction, sharemode) {
      (Direction::Render, Direction::Capture, ShareMode::Shared) => {
        AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_LOOPBACK
      }
      (Direction::Render, Direction::Capture, ShareMode::Exclusive) => {
        return Err(WasapiError::LoopbackWithExclusiveMode);
      }
      (Direction::Capture, Direction::Render, _) => {
        return Err(WasapiError::RenderToCaptureDevice);
      }
      _ => AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
    };
    if convert {
      streamflags |= AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
    }
    let mode = match sharemode {
      ShareMode::Exclusive => AUDCLNT_SHAREMODE_EXCLUSIVE,
      ShareMode::Shared => AUDCLNT_SHAREMODE_SHARED,
    };
    let device_period = match sharemode {
      ShareMode::Exclusive => period,
      ShareMode::Shared => 0,
    };
    self.sharemode = Some(*sharemode);
    unsafe {
      self.client.Initialize(
        mode,
        streamflags,
        period,
        device_period,
        wavefmt.as_waveformatex_ref(),
        None,
      )?;
    }
    self.bytes_per_frame = Some(wavefmt.get_blockalign() as usize);
    Ok(())
  }

  /// Create and return an event handle for an [IAudioClient]
  pub fn set_get_eventhandle(&self) -> WasapiRes<Handle> {
    let h_event = unsafe { CreateEventA(None, false, false, PCSTR::null())? };
    unsafe { self.client.SetEventHandle(h_event)? };
    Ok(Handle { handle: h_event })
  }

  /// Get buffer size in frames
  pub fn get_bufferframecount(&self) -> WasapiRes<u32> {
    let buffer_frame_count = unsafe { self.client.GetBufferSize()? };
    Ok(buffer_frame_count)
  }

  /// Get current padding in frames.
  /// This represents the number of frames currently in the buffer, for both capture and render devices.
  pub fn get_current_padding(&self) -> WasapiRes<u32> {
    let padding_count = unsafe { self.client.GetCurrentPadding()? };
    Ok(padding_count)
  }

  /// Get buffer size minus padding in frames.
  /// Use this to find out how much free space is available in the buffer.
  pub fn get_available_space_in_frames(&self) -> WasapiRes<u32> {
    let frames = match self.sharemode {
      Some(ShareMode::Exclusive) => {
        let buffer_frame_count = unsafe { self.client.GetBufferSize()? };
        buffer_frame_count
      }
      Some(ShareMode::Shared) => {
        let padding_count = unsafe { self.client.GetCurrentPadding()? };
        let buffer_frame_count = unsafe { self.client.GetBufferSize()? };

        buffer_frame_count - padding_count
      }
      _ => return Err(WasapiError::ClientNotInit),
    };
    Ok(frames)
  }

  /// Start the stream on an [IAudioClient]
  pub fn start_stream(&self) -> WasapiRes<()> {
    unsafe { self.client.Start()? };
    Ok(())
  }

  /// Stop the stream on an [IAudioClient]
  pub fn stop_stream(&self) -> WasapiRes<()> {
    unsafe { self.client.Stop()? };
    Ok(())
  }

  /// Reset the stream on an [IAudioClient]
  pub fn reset_stream(&self) -> WasapiRes<()> {
    unsafe { self.client.Reset()? };
    Ok(())
  }

  /// Get a rendering (playback) client
  pub fn get_audiorenderclient(&self) -> WasapiRes<AudioRenderClient> {
    let client = unsafe { self.client.GetService::<IAudioRenderClient>()? };
    Ok(AudioRenderClient {
      client,
      bytes_per_frame: self.bytes_per_frame.unwrap_or_default(),
    })
  }

  /// Get a capture client
  pub fn get_audiocaptureclient(&self) -> WasapiRes<AudioCaptureClient> {
    let client = unsafe { self.client.GetService::<IAudioCaptureClient>()? };
    Ok(AudioCaptureClient {
      client,
      sharemode: self.sharemode,
      bytes_per_frame: self.bytes_per_frame.unwrap_or_default(),
    })
  }

  /// Get the [AudioSessionControl]
  pub fn get_audiosessioncontrol(&self) -> WasapiRes<AudioSessionControl> {
    let control = unsafe { self.client.GetService::<IAudioSessionControl>()? };
    Ok(AudioSessionControl { control })
  }

  /// Get the [AudioClock]
  pub fn get_audioclock(&self) -> WasapiRes<AudioClock> {
    let clock = unsafe { self.client.GetService::<IAudioClock>()? };
    Ok(AudioClock { clock })
  }

  /// Get the direction for this [AudioClient]
  pub fn get_direction(&self) -> Direction {
    self.direction
  }

  /// Get the sharemode for this [AudioClient].
  /// The sharemode is decided when the client is initialized.
  pub fn get_sharemode(&self) -> Option<ShareMode> {
    self.sharemode
  }
}

/// Struct wrapping an [IAudioSessionControl](https://docs.microsoft.com/en-us/windows/win32/api/audiopolicy/nn-audiopolicy-iaudiosessioncontrol).
pub struct AudioSessionControl {
  control: IAudioSessionControl,
}

impl AudioSessionControl {
  /// Get the current state
  pub fn get_state(&self) -> WasapiRes<SessionState> {
    let state = unsafe { self.control.GetState()? };
    #[allow(non_upper_case_globals)]
    let sessionstate = match state {
      _ if state == AudioSessionStateActive => SessionState::Active,
      _ if state == AudioSessionStateInactive => SessionState::Inactive,
      _ if state == AudioSessionStateExpired => SessionState::Expired,
      x => return Err(WasapiError::IllegalSessionState(x.0)),
    };
    Ok(sessionstate)
  }

  /// Register to receive notifications
  pub fn register_session_notification(&self, callbacks: Weak<EventCallbacks>) -> WasapiRes<()> {
    let events: IAudioSessionEvents = AudioSessionEvents::new(callbacks).into();

    match unsafe { self.control.RegisterAudioSessionNotification(&events) } {
      Ok(()) => Ok(()),
      Err(err) => Err(WasapiError::RegisterNotifications(err)),
    }
  }
}

/// Struct wrapping an [IAudioClock](https://docs.microsoft.com/en-us/windows/win32/api/audioclient/nn-audioclient-iaudioclock).
pub struct AudioClock {
  clock: IAudioClock,
}

impl AudioClock {
  /// Get the frequency for this [AudioClock].
  /// Note that the unit for the value is undefined.
  pub fn get_frequency(&self) -> WasapiRes<u64> {
    let freq = unsafe { self.clock.GetFrequency()? };
    Ok(freq)
  }

  /// Get the current device position. Returns the position, as well as the value of the
  /// performance counter at the time the position values was taken.
  /// The unit for the position value is undefined, but the frequency and position values are
  /// in the same unit. Dividing the position with the frequency gets the position in seconds.
  pub fn get_position(&self) -> WasapiRes<(u64, u64)> {
    let mut pos = 0;
    let mut timer = 0;
    unsafe { self.clock.GetPosition(&mut pos, Some(&mut timer))? };
    Ok((pos, timer))
  }
}

/// Struct wrapping an [IAudioRenderClient](https://docs.microsoft.com/en-us/windows/win32/api/audioclient/nn-audioclient-iaudiorenderclient).
pub struct AudioRenderClient {
  client: IAudioRenderClient,
  bytes_per_frame: usize,
}

impl AudioRenderClient {
  /// Write raw bytes data to a device from a slice.
  /// The number of frames to write should first be checked with the
  /// `get_available_space_in_frames()` method on the [AudioClient].
  /// The buffer_flags argument can be used to mark a buffer as silent.
  pub fn write_to_device(
    &self,
    nbr_frames: usize,
    data: &[u8],
    buffer_flags: Option<BufferFlags>,
  ) -> WasapiRes<()> {
    if nbr_frames == 0 {
      return Ok(());
    }
    let nbr_bytes = nbr_frames * self.bytes_per_frame;
    if nbr_bytes != data.len() {
      return Err(WasapiError::DataLengthMismatch {
        received: data.len(),
        expected: nbr_bytes,
      });
    }
    let bufferptr = unsafe { self.client.GetBuffer(nbr_frames as u32)? };
    let bufferslice = unsafe { slice::from_raw_parts_mut(bufferptr, nbr_bytes) };
    bufferslice.copy_from_slice(data);
    let flags = match buffer_flags {
      Some(bflags) => bflags.to_u32(),
      None => 0,
    };
    unsafe { self.client.ReleaseBuffer(nbr_frames as u32, flags)? };
    Ok(())
  }

  /// Write raw bytes data to a device from a deque.
  /// The number of frames to write should first be checked with the
  /// `get_available_space_in_frames()` method on the [AudioClient].
  /// The buffer_flags argument can be used to mark a buffer as silent.
  pub fn write_to_device_from_deque(
    &self,
    nbr_frames: usize,
    data: &mut VecDeque<u8>,
    buffer_flags: Option<BufferFlags>,
  ) -> WasapiRes<()> {
    if nbr_frames == 0 {
      return Ok(());
    }
    let nbr_bytes = nbr_frames * self.bytes_per_frame;
    if nbr_bytes > data.len() {
      return Err(WasapiError::DataLengthTooShort {
        received: data.len(),
        expected: nbr_bytes,
      });
    }
    let bufferptr = unsafe { self.client.GetBuffer(nbr_frames as u32)? };
    let bufferslice = unsafe { slice::from_raw_parts_mut(bufferptr, nbr_bytes) };
    for element in bufferslice.iter_mut() {
      *element = data.pop_front().unwrap();
    }
    let flags = match buffer_flags {
      Some(bflags) => bflags.to_u32(),
      None => 0,
    };
    unsafe { self.client.ReleaseBuffer(nbr_frames as u32, flags)? };
    Ok(())
  }
}

/// Struct representing the [ _AUDCLNT_BUFFERFLAGS enum values](https://docs.microsoft.com/en-us/windows/win32/api/audioclient/ne-audioclient-_audclnt_bufferflags).
#[derive(Debug)]
pub struct BufferFlags {
  /// AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY
  pub data_discontinuity: bool,
  /// AUDCLNT_BUFFERFLAGS_SILENT
  pub silent: bool,
  /// AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR
  pub timestamp_error: bool,
}

impl BufferFlags {
  /// Create a new [BufferFlags] struct from a `u32` value.
  pub fn new(flags: u32) -> Self {
    BufferFlags {
      data_discontinuity: flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 > 0,
      silent: flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 > 0,
      timestamp_error: flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 > 0,
    }
  }

  pub fn none() -> Self {
    BufferFlags {
      data_discontinuity: false,
      silent: false,
      timestamp_error: false,
    }
  }

  /// Convert a [BufferFlags] struct to a `u32` value.
  pub fn to_u32(&self) -> u32 {
    let mut value = 0;
    if self.data_discontinuity {
      value += AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32;
    }
    if self.silent {
      value += AUDCLNT_BUFFERFLAGS_SILENT.0 as u32;
    }
    if self.timestamp_error {
      value += AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32;
    }
    value
  }
}

/// Struct wrapping an [IAudioCaptureClient](https://docs.microsoft.com/en-us/windows/win32/api/audioclient/nn-audioclient-iaudiocaptureclient).
pub struct AudioCaptureClient {
  client: IAudioCaptureClient,
  sharemode: Option<ShareMode>,
  bytes_per_frame: usize,
}

impl AudioCaptureClient {
  /// Get number of frames in next packet when in shared mode.
  /// In exclusive mode it returns None, instead use [AudioClient::get_bufferframecount()].
  pub fn get_next_nbr_frames(&self) -> WasapiRes<Option<u32>> {
    if let Some(ShareMode::Exclusive) = self.sharemode {
      return Ok(None);
    }
    let nbr_frames = unsafe { self.client.GetNextPacketSize()? };
    Ok(Some(nbr_frames))
  }

  /// Read raw bytes from a device into a slice. Returns the number of frames
  /// that was read, and the BufferFlags describing the buffer that the data was read from.
  /// The slice must be large enough to hold all data.
  /// If it is longer that needed, the unused elements will not be modified.
  pub fn read_from_device(&self, data: &mut [u8]) -> WasapiRes<(u32, BufferFlags)> {
    let data_len_in_frames = data.len() / self.bytes_per_frame;
    if data_len_in_frames == 0 {
      return Ok((0, BufferFlags::none()));
    }
    let mut buffer_ptr = ptr::null_mut();
    let mut nbr_frames_returned = 0;
    let mut flags = 0;
    unsafe {
      self.client.GetBuffer(
        &mut buffer_ptr,
        &mut nbr_frames_returned,
        &mut flags,
        None,
        None,
      )?
    };
    let bufferflags = BufferFlags::new(flags);
    if nbr_frames_returned == 0 {
      unsafe { self.client.ReleaseBuffer(nbr_frames_returned)? };
      return Ok((0, bufferflags));
    }
    if data_len_in_frames < nbr_frames_returned as usize {
      unsafe { self.client.ReleaseBuffer(nbr_frames_returned)? };
      return Err(WasapiError::DataLengthTooShort {
        received: data_len_in_frames,
        expected: nbr_frames_returned as usize,
      });
    }
    let len_in_bytes = nbr_frames_returned as usize * self.bytes_per_frame;
    let bufferslice = unsafe { slice::from_raw_parts(buffer_ptr, len_in_bytes) };
    data[..len_in_bytes].copy_from_slice(bufferslice);
    if nbr_frames_returned > 0 {
      unsafe { self.client.ReleaseBuffer(nbr_frames_returned)? };
    }
    Ok((nbr_frames_returned, bufferflags))
  }

  /// Read raw bytes data from a device into a deque.
  /// Returns the [BufferFlags] describing the buffer that the data was read from.
  pub fn read_from_device_to_deque(&self, data: &mut VecDeque<u8>) -> WasapiRes<BufferFlags> {
    let mut buffer_ptr = ptr::null_mut();
    let mut nbr_frames_returned = 0;
    let mut flags = 0;
    unsafe {
      self.client.GetBuffer(
        &mut buffer_ptr,
        &mut nbr_frames_returned,
        &mut flags,
        None,
        None,
      )?
    };
    let bufferflags = BufferFlags::new(flags);
    if nbr_frames_returned == 0 {
      // There is no need to release a buffer of 0 bytes
      return Ok(bufferflags);
    }
    let len_in_bytes = nbr_frames_returned as usize * self.bytes_per_frame;
    let bufferslice = unsafe { slice::from_raw_parts(buffer_ptr, len_in_bytes) };
    for element in bufferslice.iter() {
      data.push_back(*element);
    }
    if nbr_frames_returned > 0 {
      unsafe { self.client.ReleaseBuffer(nbr_frames_returned).unwrap() };
    }
    Ok(bufferflags)
  }

  /// Get the sharemode for this [AudioCaptureClient].
  /// The sharemode is decided when the client is initialized.
  pub fn get_sharemode(&self) -> Option<ShareMode> {
    self.sharemode
  }
}

/// Struct wrapping a [HANDLE] to an [Event Object](https://docs.microsoft.com/en-us/windows/win32/sync/event-objects).
pub struct Handle {
  handle: HANDLE,
}

impl Handle {
  /// Wait for an event on a handle, with a timeout given in ms
  pub fn wait_for_event(&self, timeout_ms: u32) -> WasapiRes<()> {
    let retval = unsafe { WaitForSingleObject(self.handle, timeout_ms) };
    if retval.0 != WAIT_OBJECT_0.0 {
      return Err(WasapiError::EventTimeout);
    }
    Ok(())
  }
}
