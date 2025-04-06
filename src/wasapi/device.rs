use log::trace;
use widestring::U16CString;
use windows::Win32::Devices::FunctionDiscovery::{
  PKEY_DeviceInterface_FriendlyName, PKEY_Device_DeviceDesc, PKEY_Device_FriendlyName,
};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::{
  EDataFlow, IAudioClient, IMMDevice, IMMDeviceCollection, IMMDeviceEnumerator, IMMEndpoint,
  MMDeviceEnumerator, DEVICE_STATE_ACTIVE, DEVICE_STATE_DISABLED, DEVICE_STATE_NOTPRESENT,
  DEVICE_STATE_UNPLUGGED,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL, STGM_READ};

use crate::wasapi::audio_client::AudioClient;
use crate::wasapi::types::{DeviceState, Direction};
use crate::wasapi::WasapiError;
use crate::wasapi::WasapiRes;
use windows_core::Interface;
/// Struct wrapping an [IMMDeviceCollection](https://docs.microsoft.com/en-us/windows/win32/api/mmdeviceapi/nn-mmdeviceapi-immdevicecollection).
pub struct DeviceCollection {
  pub(crate) collection: IMMDeviceCollection,
  pub(crate) direction: Direction,
}

impl DeviceCollection {
  /// Get an [IMMDeviceCollection] of all active playback or capture devices
  pub fn new(direction: &Direction) -> WasapiRes<DeviceCollection> {
    let dir: EDataFlow = direction.into();
    let enumerator: IMMDeviceEnumerator =
      unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let devs = unsafe { enumerator.EnumAudioEndpoints(dir, DEVICE_STATE_ACTIVE)? };
    Ok(DeviceCollection {
      collection: devs,
      direction: *direction,
    })
  }

  /// Get the number of devices in an [IMMDeviceCollection]
  pub fn get_nbr_devices(&self) -> WasapiRes<u32> {
    let count = unsafe { self.collection.GetCount()? };
    Ok(count)
  }

  /// Get a device from an [IMMDeviceCollection] using index
  pub fn get_device_at_index(&self, idx: u32) -> WasapiRes<Device> {
    let device = unsafe { self.collection.Item(idx)? };
    Ok(Device {
      device,
      direction: self.direction,
    })
  }

  /// Get a device from an [IMMDeviceCollection] using name
  pub fn get_device_with_name(&self, name: &str) -> WasapiRes<Device> {
    let count = unsafe { self.collection.GetCount()? };
    trace!("nbr devices {}", count);
    for n in 0..count {
      let device = self.get_device_at_index(n)?;
      let devname = device.get_friendlyname()?;
      if name == devname {
        return Ok(device);
      }
    }
    Err(WasapiError::DeviceNotFound(name.to_owned()))
  }

  /// Get the direction for this [DeviceCollection]
  pub fn get_direction(&self) -> Direction {
    self.direction
  }
}

/// Iterator for [DeviceCollection]
pub struct DeviceCollectionIter<'a> {
  collection: &'a DeviceCollection,
  index: u32,
}

impl Iterator for DeviceCollectionIter<'_> {
  type Item = WasapiRes<Device>;

  fn next(&mut self) -> Option<Self::Item> {
    if self.index < self.collection.get_nbr_devices().unwrap() {
      let device = self.collection.get_device_at_index(self.index);
      self.index += 1;
      Some(device)
    } else {
      None
    }
  }
}

/// Implement iterator for [DeviceCollection]
impl<'a> IntoIterator for &'a DeviceCollection {
  type Item = WasapiRes<Device>;
  type IntoIter = DeviceCollectionIter<'a>;

  fn into_iter(self) -> Self::IntoIter {
    DeviceCollectionIter {
      collection: self,
      index: 0,
    }
  }
}

/// Struct wrapping an [IMMDevice](https://docs.microsoft.com/en-us/windows/win32/api/mmdeviceapi/nn-mmdeviceapi-immdevice).
pub struct Device {
  pub(crate) device: IMMDevice,
  pub(crate) direction: Direction,
}

impl Device {
  /// Build a [Device] from a supplied [IMMDevice] and [Direction]
  ///
  /// # Safety
  ///
  /// The caller must ensure that the [IMMDevice]'s data flow direction
  /// is the same as the [Direction] supplied to the function.
  ///
  /// Use [Device::from_immdevice], which queries the endpoint, for safe construction.
  pub unsafe fn from_raw(device: IMMDevice, direction: Direction) -> Device {
    Device { device, direction }
  }

  /// Attempts to build a [Device] from a supplied [IMMDevice],
  /// querying the endpoint for its data flow direction.
  pub fn from_immdevice(device: IMMDevice) -> WasapiRes<Device> {
    let endpoint: IMMEndpoint = device.cast()?;
    let direction: Direction = unsafe { endpoint.GetDataFlow()? }.try_into()?;

    Ok(Device { device, direction })
  }

  /// Get an [IAudioClient] from an [IMMDevice]
  pub fn get_iaudioclient(&self) -> WasapiRes<AudioClient> {
    let audio_client = unsafe { self.device.Activate::<IAudioClient>(CLSCTX_ALL, None)? };
    Ok(AudioClient {
      client: audio_client,
      direction: self.direction,
      sharemode: None,
      bytes_per_frame: None,
    })
  }

  /// Read state from an [IMMDevice]
  pub fn get_state(&self) -> WasapiRes<DeviceState> {
    let state = unsafe { self.device.GetState()? };
    trace!("state: {:?}", state);
    let state_enum = match state {
      _ if state == DEVICE_STATE_ACTIVE => DeviceState::Active,
      _ if state == DEVICE_STATE_DISABLED => DeviceState::Disabled,
      _ if state == DEVICE_STATE_NOTPRESENT => DeviceState::NotPresent,
      _ if state == DEVICE_STATE_UNPLUGGED => DeviceState::Unplugged,
      x => return Err(WasapiError::IllegalDeviceState(x.0)),
    };
    Ok(state_enum)
  }

  /// Read the friendly name of the endpoint device (for example, "Speakers (XYZ Audio Adapter)")
  pub fn get_friendlyname(&self) -> WasapiRes<String> {
    self.get_string_property(&PKEY_Device_FriendlyName)
  }

  /// Read the friendly name of the audio adapter to which the endpoint device is attached (for example, "XYZ Audio Adapter")
  pub fn get_interface_friendlyname(&self) -> WasapiRes<String> {
    self.get_string_property(&PKEY_DeviceInterface_FriendlyName)
  }

  /// Read the device description of the endpoint device (for example, "Speakers")
  pub fn get_description(&self) -> WasapiRes<String> {
    self.get_string_property(&PKEY_Device_DeviceDesc)
  }

  /// Read the FriendlyName of an [IMMDevice]
  fn get_string_property(&self, key: &PROPERTYKEY) -> WasapiRes<String> {
    let store = unsafe { self.device.OpenPropertyStore(STGM_READ)? };
    let prop = unsafe { store.GetValue(key)? };
    let propstr = unsafe { PropVariantToStringAlloc(&prop)? };
    let wide_name = unsafe { U16CString::from_ptr_str(propstr.0) };
    let name = wide_name.to_string_lossy();
    trace!("name: {}", name);
    Ok(name)
  }

  /// Get the Id of an [IMMDevice]
  pub fn get_id(&self) -> WasapiRes<String> {
    let idstr = unsafe { self.device.GetId()? };
    let wide_id = unsafe { U16CString::from_ptr_str(idstr.0) };
    let id = wide_id.to_string_lossy();
    trace!("id: {}", id);
    Ok(id)
  }

  /// Get the direction for this Device
  pub fn get_direction(&self) -> Direction {
    self.direction
  }
}
