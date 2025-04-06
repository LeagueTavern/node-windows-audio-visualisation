use num_complex::Complex;
use rustfft::FftPlanner;

pub fn analyze_spectrum(samples: &[f32], num_bands: usize) -> Vec<f32> {
  let fft_size = samples.len().next_power_of_two();
  let mut fft_input: Vec<Complex<f32>> = samples
    .iter()
    .take(fft_size)
    .map(|&s| Complex::new(s, 0.0))
    .collect();

  for i in 0..fft_size {
    let window =
      0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (fft_size as f32 - 1.0)).cos());
    fft_input[i] = fft_input[i] * window;
  }

  fft_input.resize(fft_size, Complex::new(0.0, 0.0));

  let mut planner = FftPlanner::new();
  let fft = planner.plan_fft_forward(fft_size);
  let mut fft_output = fft_input.clone();
  fft.process(&mut fft_output);

  let magnitudes: Vec<f32> = fft_output
    .iter()
    .take(fft_size / 2)
    .map(|c| c.norm())
    .collect();

  let mut spectrum = vec![0.0f32; num_bands];
  let bins_per_band = (fft_size / 2) / num_bands;

  for i in 0..num_bands {
    let start = i * bins_per_band;
    let end = (i + 1) * bins_per_band;

    spectrum[i] = magnitudes[start..end].iter().sum::<f32>() / bins_per_band as f32;
    spectrum[i] = (1.0 + spectrum[i]).log10();
  }

  spectrum
}
