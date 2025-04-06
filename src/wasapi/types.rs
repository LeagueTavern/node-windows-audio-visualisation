use std::fmt;
use windows::Win32::Media::Audio::{
  EDataFlow, ERole, eCapture, eCommunications, eConsole, eMultimedia, eRender,
};

use crate::wasapi::WasapiError;

/// Audio direction, playback or capture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
  Render,
  Capture,
}

impl fmt::Display for Direction {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match *self {
      Direction::Render => write!(f, "Render"),
      Direction::Capture => write!(f, "Capture"),
    }
  }
}

impl TryFrom<&EDataFlow> for Direction {
  type Error = WasapiError;

  fn try_from(value: &EDataFlow) -> Result<Self, Self::Error> {
    match value {
      EDataFlow(0) => Ok(Self::Render),
      EDataFlow(1) => Ok(Self::Capture),
      // EDataFlow(2) => All/Both,
      x => Err(WasapiError::IllegalDeviceDirection(x.0)),
    }
  }
}

impl TryFrom<EDataFlow> for Direction {
  type Error = WasapiError;

  fn try_from(value: EDataFlow) -> Result<Self, Self::Error> {
    Self::try_from(&value)
  }
}

impl From<&Direction> for EDataFlow {
  fn from(value: &Direction) -> Self {
    match value {
      Direction::Capture => eCapture,
      Direction::Render => eRender,
    }
  }
}

impl From<Direction> for EDataFlow {
  fn from(value: Direction) -> Self {
    Self::from(&value)
  }
}

/// Wrapper for [ERole](https://learn.microsoft.com/en-us/windows/win32/api/mmdeviceapi/ne-mmdeviceapi-erole).
/// Console is the role used by most applications
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
  Console,
  Multimedia,
  Communications,
}

impl fmt::Display for Role {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match *self {
      Role::Console => write!(f, "Console"),
      Role::Multimedia => write!(f, "Multimedia"),
      Role::Communications => write!(f, "Communications"),
    }
  }
}

impl TryFrom<&ERole> for Role {
  type Error = WasapiError;

  fn try_from(value: &ERole) -> Result<Self, Self::Error> {
    match value {
      ERole(0) => Ok(Self::Console),
      ERole(1) => Ok(Self::Multimedia),
      ERole(2) => Ok(Self::Communications),
      x => Err(WasapiError::IllegalDeviceRole(x.0)),
    }
  }
}

impl TryFrom<ERole> for Role {
  type Error = WasapiError;

  fn try_from(value: ERole) -> Result<Self, Self::Error> {
    Self::try_from(&value)
  }
}

impl From<&Role> for ERole {
  fn from(value: &Role) -> Self {
    match value {
      Role::Communications => eCommunications,
      Role::Multimedia => eMultimedia,
      Role::Console => eConsole,
    }
  }
}

impl From<Role> for ERole {
  fn from(value: Role) -> Self {
    Self::from(&value)
  }
}

/// Sharemode for device
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShareMode {
  Shared,
  Exclusive,
}

impl fmt::Display for ShareMode {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match *self {
      ShareMode::Shared => write!(f, "Shared"),
      ShareMode::Exclusive => write!(f, "Exclusive"),
    }
  }
}

/// Sample type, float or integer
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SampleType {
  Float,
  Int,
}

impl fmt::Display for SampleType {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match *self {
      SampleType::Float => write!(f, "Float"),
      SampleType::Int => write!(f, "Int"),
    }
  }
}

/// Possible states for an [AudioSessionControl], an enum representing the
/// [AudioSessionStateXxx constants](https://learn.microsoft.com/en-us/windows/win32/api/audiosessiontypes/ne-audiosessiontypes-audiosessionstate)
#[derive(Debug, Eq, PartialEq)]
pub enum SessionState {
  /// The audio session is active. (At least one of the streams in the session is running.)
  Active,
  /// The audio session is inactive. (It contains at least one stream, but none of the streams in the session is currently running.)
  Inactive,
  /// The audio session has expired. (It contains no streams.)
  Expired,
}

impl fmt::Display for SessionState {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match *self {
      SessionState::Active => write!(f, "Active"),
      SessionState::Inactive => write!(f, "Inactive"),
      SessionState::Expired => write!(f, "Expired"),
    }
  }
}

/// Possible states for an [IMMDevice], an enum representing the
/// [DEVICE_STATE_XXX constants](https://learn.microsoft.com/en-us/windows/win32/coreaudio/device-state-xxx-constants)
#[derive(Debug, Eq, PartialEq, Clone)]
pub enum DeviceState {
  /// The audio endpoint device is active. That is, the audio adapter that connects to the
  /// endpoint device is present and enabled. In addition, if the endpoint device plugs int
  /// a jack on the adapter, then the endpoint device is plugged in.
  Active,
  /// The audio endpoint device is disabled. The user has disabled the device in the Windows
  /// multimedia control panel, Mmsys.cpl
  Disabled,
  /// The audio endpoint device is not present because the audio adapter that connects to the
  /// endpoint device has been removed from the system, or the user has disabled the adapter
  /// device in Device Manager.
  NotPresent,
  /// The audio endpoint device is unplugged. The audio adapter that contains the jack for the
  /// endpoint device is present and enabled, but the endpoint device is not plugged into the
  /// jack. Only a device with jack-presence detection can be in this state.
  Unplugged,
}

impl fmt::Display for DeviceState {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match *self {
      DeviceState::Active => write!(f, "Active"),
      DeviceState::Disabled => write!(f, "Disabled"),
      DeviceState::NotPresent => write!(f, "NotPresent"),
      DeviceState::Unplugged => write!(f, "Unplugged"),
    }
  }
}
