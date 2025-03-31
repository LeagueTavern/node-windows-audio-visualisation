use napi_derive::napi;

#[napi(object)]
pub struct AudioDevice {
  pub id: String,
  pub name: String,
  pub sample_rate: u32,
  pub buffer_size: Option<u32>,
  pub is_default: bool,
}
