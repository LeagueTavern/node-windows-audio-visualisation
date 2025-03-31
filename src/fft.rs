use dasp::window::{Hanning, Window};
use rustfft::{num_complex::Complex32, num_traits::Zero, FftPlanner};

pub struct FFT<const BUF_SIZE: usize = 16384> {
  planner: FftPlanner<f32>,
  data: Vec<f32>,
}

impl Default for FFT<16384> {
  fn default() -> Self {
    Self {
      planner: FftPlanner::new(),
      data: vec![0.; 16384],
    }
  }
}

impl<const BUF_SIZE: usize> FFT<BUF_SIZE> {
  pub fn process(&mut self, buf: &[f32], size: usize) -> &[f32] {
    if size > BUF_SIZE {
      panic!("{size} is higher then max buf size of {BUF_SIZE}")
    }

    let mut buffer = vec![Complex32::zero(); size * 2];
    let mut scratch = vec![Complex32::zero(); size * 2];

    buffer[..size].copy_from_slice(&vec![Complex32::zero(); size][..size]);

    if buf.len() > size {
      let chunk_size = (buf.len() as f64 / size as f64).floor() as usize;

      let mut bins = buf
        .chunks(chunk_size)
        .map(|chunk: &[f32]| {
          chunk.iter().copied().map(Hanning::window).sum::<f32>() / chunk.len() as f32
        })
        .map(Complex32::from);

      for i in 0..size {
        buffer[i] = bins.next().unwrap_or_default();
      }
    } else {
      for i in 0..buf.len().min(size) {
        let value = Hanning::window(buf[i]);
        buffer[i] = Complex32::from(value);
      }
    }

    let max = (size as f32).sqrt();
    let fft = self.planner.plan_fft_forward(size);

    let scratch_len = fft.get_inplace_scratch_len();

    if scratch_len >= scratch.len() {
      scratch.resize(scratch_len, Complex32::zero());
    }

    fft.process_with_scratch(&mut buffer[..size], &mut scratch[..scratch_len]);

    for i in 0..size {
      self.data[i] = buffer[i].re / max;
    }

    &self.data[..size]
  }
}
