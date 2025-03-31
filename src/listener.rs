use std::sync::{Arc, Mutex};

pub struct Listener {
  buffer: Arc<Mutex<Vec<f32>>>,
}

impl Listener {
  pub fn new() -> Self {
    Self {
      buffer: Arc::new(Mutex::new(Vec::new())),
    }
  }

  pub fn callback(&self) -> ListenerCallback {
    ListenerCallback {
      buffer: self.buffer.clone(),
    }
  }

  pub fn get_buffer(&self) -> Vec<f32> {
    let mut buf = self.buffer.lock().unwrap();
    let data = buf.clone();
    buf.clear();
    data
  }
}

pub struct ListenerCallback {
  buffer: Arc<Mutex<Vec<f32>>>,
}

impl ListenerCallback {
  pub fn get(self) -> impl FnMut(&[f32], &cpal::InputCallbackInfo) {
    let buffer = self.buffer;
    move |data: &[f32], _: &_| {
      let mut buf = buffer.lock().unwrap();
      buf.extend_from_slice(data);
    }
  }
}
