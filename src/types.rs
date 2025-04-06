use napi_derive::napi;

#[napi(object)]
pub struct AudioDevice {
  pub id: String,
  pub name: String,
  pub state: u32,
  pub is_default: bool,
}
