use num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};
use std::sync::Arc;
use std::time::Instant;

/// Configuration for the spectrum analyzer
#[derive(Clone, Debug)]
pub struct AnalyzerConfig {
    pub fft_size: usize,
    pub sample_rate: u32,
    pub update_rate_hz: u32,
    pub min_freq: f32,
    pub max_freq: f32,
    pub slope_db_per_octave: f32,
    pub reference_magnitude: f32, // Fixed reference level for normalization
}

impl Default for AnalyzerConfig {
    fn default() -> Self {
        Self {
            fft_size: 8192, // Increased from 4096 for better low-frequency resolution
            sample_rate: 48000,
            update_rate_hz: 30,
            min_freq: 20.0,
            max_freq: 20000.0,
            slope_db_per_octave: 4.5,
            reference_magnitude: 0.00025, // Calibrated so typical music (-12 dBFS) appears at 0 dB
        }
    }
}

// Constant-Q analyzer parameters
const OCTAVE_BANDWIDTH: f32 = 1.0 / 6.0; // 1/6 octave smoothing bandwidth
const TAU_LOW: f32 = 0.07; // 70ms time constant (uniform across all frequencies)
const TAU_HIGH: f32 = 0.07; // 70ms time constant (uniform across all frequencies)

/// Pre-computed analysis parameters for constant-Q processing
struct AnalysisParams {
    octave_bands: Vec<OctaveBand>,
    ema_coefficients: Vec<f32>,
}

/// Octave band definition for constant-Q analysis
struct OctaveBand {
    bin_low_frac: f32,  // Fractional bin index (lower bound)
    bin_high_frac: f32, // Fractional bin index (upper bound)
}

/// Result from FFT analysis - frequency band magnitudes
#[derive(Clone, Debug)]
pub struct AnalyzerFrame {
    pub bands: Vec<f32>,
}

/// The FFT processor - runs in dedicated thread
pub struct SpectrumAnalyzer {
    config: AnalyzerConfig,
    ring_buffer: Vec<f32>,
    write_pos: usize,
    samples_ready: usize,
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    previous_magnitudes: Vec<f32>,
    last_update: Instant,
    output_buffer: Vec<Complex<f32>>,
    input_buffer: Vec<f32>,
    params: Option<AnalysisParams>, // Pre-computed constant-Q analysis parameters
}

impl SpectrumAnalyzer {
    /// Create new analyzer with config
    pub fn new(config: AnalyzerConfig) -> Self {
        let fft_size = config.fft_size;

        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(fft_size);

        // Pre-compute Hann window
        let window = Self::create_hann_window(fft_size);

        let output_buffer = fft.make_output_vec();

        Self {
            config,
            ring_buffer: vec![0.0; fft_size],
            write_pos: 0,
            samples_ready: 0,
            fft,
            window,
            previous_magnitudes: Vec::new(),
            last_update: Instant::now(),
            output_buffer,
            input_buffer: vec![0.0; fft_size],
            params: None, // Initialized lazily on first push_samples call
        }
    }

    /// Update sample rate (when switching between files)
    pub fn set_sample_rate(&mut self, sample_rate: u32) {
        if self.config.sample_rate != sample_rate {
            self.config.sample_rate = sample_rate;
            // Clear previous state when sample rate changes
            self.previous_magnitudes.clear();
            // Reset params so they will be recalculated with new sample rate
            self.params = None;
        }
    }

    /// Push audio samples (mono, f32)
    /// Returns Some(AnalyzerFrame) when FFT is ready and update interval has elapsed
    pub fn push_samples(&mut self, samples: &[f32], num_bands: usize) -> Option<AnalyzerFrame> {
        // Add samples to ring buffer
        for &sample in samples {
            self.ring_buffer[self.write_pos] = sample;
            self.write_pos = (self.write_pos + 1) % self.config.fft_size;
            self.samples_ready = (self.samples_ready + 1).min(self.config.fft_size);
        }

        // Check if we have enough samples and enough time has passed
        if self.samples_ready < self.config.fft_size {
            return None;
        }

        let elapsed = self.last_update.elapsed();
        let update_interval =
            std::time::Duration::from_millis(1000 / self.config.update_rate_hz as u64);

        if elapsed < update_interval {
            return None;
        }

        // Initialize params lazily on first use
        if self.params.is_none() {
            self.params = Some(Self::initialize_params(num_bands, &self.config));
        }

        // Perform FFT and generate frame
        let bands = self.process_fft(num_bands);
        self.last_update = Instant::now();

        Some(AnalyzerFrame { bands })
    }

    /// Perform FFT on accumulated buffer and map to frequency bands
    fn process_fft(&mut self, num_bands: usize) -> Vec<f32> {
        // Copy from ring buffer to input buffer (in correct order)
        let mut read_pos = self.write_pos;
        for i in 0..self.config.fft_size {
            self.input_buffer[i] = self.ring_buffer[read_pos] * self.window[i];
            read_pos = (read_pos + 1) % self.config.fft_size;
        }

        // Perform FFT
        // Process FFT (input is real, output is complex)
        if self
            .fft
            .process(&mut self.input_buffer, &mut self.output_buffer)
            .is_err()
        {
            // On error, return empty bands
            return vec![0.0; num_bands];
        }

        // Extract magnitudes from FFT output and normalize by FFT size
        // This scales the output to be independent of FFT size
        let fft_scale = 2.0 / self.config.fft_size as f32;
        let mut magnitudes: Vec<f32> = self
            .output_buffer
            .iter()
            .map(|c| c.norm() * fft_scale)
            .collect();

        // Apply 4.5 dB/octave slope correction
        self.apply_slope(&mut magnitudes);

        // Get params (should always be Some at this point)
        let params = self.params.as_ref().expect("params should be initialized");

        // Map FFT bins to constant-Q bands with octave smoothing
        let raw_bands = self.map_to_constant_q_bands(&magnitudes, params);

        // Convert to dB scale and normalize
        let bands = self.magnitude_to_db_normalized(&raw_bands);

        // Apply frequency-dependent exponential smoothing
        let smoothed = self.smooth_bands_frequency_dependent(&bands, &params.ema_coefficients);

        // Update previous magnitudes
        self.previous_magnitudes = smoothed.clone();

        smoothed
    }

    /// Apply 4.5 dB/octave slope correction
    fn apply_slope(&self, magnitudes: &mut [f32]) {
        let bin_hz = self.config.sample_rate as f32 / self.config.fft_size as f32;

        for (bin_idx, magnitude) in magnitudes.iter_mut().enumerate() {
            if bin_idx == 0 {
                continue; // Skip DC bin
            }

            let freq = bin_idx as f32 * bin_hz;
            if freq < 1.0 {
                continue;
            }

            // Calculate gain: 10^(slope * log2(freq/1000) / 20)
            let octaves = (freq / 1000.0).log2();
            let gain_db = self.config.slope_db_per_octave * octaves;
            let gain_linear = 10.0_f32.powf(gain_db / 20.0);

            *magnitude *= gain_linear;
        }
    }

    /// Convert magnitude bands to dB scale normalized to 0.0-1.0
    /// Maps -60 dB to 0.0, +30 dB to 1.0
    /// Calibrated so typical music (-12 dBFS) appears at 0 dB
    fn magnitude_to_db_normalized(&self, bands: &[f32]) -> Vec<f32> {
        const DB_MIN: f32 = -60.0;
        const DB_MAX: f32 = 30.0; // +30 dB headroom for peaks
        const DB_RANGE: f32 = DB_MAX - DB_MIN; // 90 dB total range

        bands
            .iter()
            .map(|&magnitude| {
                if magnitude <= 0.0 {
                    return 0.0;
                }

                let db = 20.0 * (magnitude / self.config.reference_magnitude).log10();
                let normalized = (db - DB_MIN) / DB_RANGE;
                normalized.clamp(0.0, 1.0)
            })
            .collect()
    }

    /// Apply frequency-dependent exponential smoothing
    fn smooth_bands_frequency_dependent(
        &self,
        bands: &[f32],
        ema_coefficients: &[f32],
    ) -> Vec<f32> {
        if self.previous_magnitudes.is_empty() || self.previous_magnitudes.len() != bands.len() {
            return bands.to_vec();
        }

        bands
            .iter()
            .zip(self.previous_magnitudes.iter())
            .zip(ema_coefficients.iter())
            .map(|((&current, &previous), &alpha)| {
                // Exponential moving average: y[n] = α*y[n-1] + (1-α)*x[n]
                alpha * previous + (1.0 - alpha) * current
            })
            .collect()
    }

    /// Create Hann window function
    fn create_hann_window(size: usize) -> Vec<f32> {
        (0..size)
            .map(|n| {
                let factor = 2.0 * std::f32::consts::PI * n as f32 / size as f32;
                0.5 * (1.0 - factor.cos())
            })
            .collect()
    }

    /// Map FFT bins to constant-Q bands using octave-based smoothing
    fn map_to_constant_q_bands(&self, magnitudes: &[f32], params: &AnalysisParams) -> Vec<f32> {
        params
            .octave_bands
            .iter()
            .map(|band| {
                // Get FFT bins in octave range [bin_low_frac, bin_high_frac]
                let bin_low = band.bin_low_frac.floor() as usize;
                let bin_high = band.bin_high_frac.ceil() as usize;

                // Average POWER (mag²) across octave bandwidth
                let mut power_sum = 0.0;
                let mut count = 0;

                let bin_hi = bin_high.min(magnitudes.len());
                for &mag in magnitudes.iter().take(bin_hi).skip(bin_low) {
                    power_sum += mag * mag; // Power = magnitude²
                    count += 1;
                }

                // Convert back to magnitude
                if count > 0 {
                    (power_sum / count as f32).sqrt()
                } else {
                    0.0
                }
            })
            .collect()
    }

    /// Mix multi-channel audio to mono by averaging all channels
    pub fn mixdown_to_mono(samples: &[f32], channels: u16) -> Vec<f32> {
        if channels == 1 {
            return samples.to_vec();
        }

        let channels = channels as usize;
        let num_frames = samples.len() / channels;
        let mut mono = Vec::with_capacity(num_frames);

        for frame in 0..num_frames {
            let mut sum = 0.0;
            for ch in 0..channels {
                sum += samples[frame * channels + ch];
            }
            mono.push(sum / channels as f32);
        }

        mono
    }

    /// Calculate logarithmically-spaced display frequencies
    fn calculate_display_frequencies(num_bands: usize, min_freq: f32, max_freq: f32) -> Vec<f32> {
        (0..num_bands)
            .map(|i| {
                let t = i as f32 / (num_bands - 1) as f32;
                min_freq * (max_freq / min_freq).powf(t)
            })
            .collect()
    }

    /// Calculate octave bands for constant-Q analysis
    fn calculate_octave_bands(
        display_freqs: &[f32],
        sample_rate: u32,
        fft_size: usize,
    ) -> Vec<OctaveBand> {
        let bin_hz = sample_rate as f32 / fft_size as f32;

        display_freqs
            .iter()
            .map(|&freq_center| {
                // Octave bandwidth: [f * 2^(-bw/2), f * 2^(bw/2)]
                let freq_low = freq_center * 2.0_f32.powf(-OCTAVE_BANDWIDTH / 2.0);
                let freq_high = freq_center * 2.0_f32.powf(OCTAVE_BANDWIDTH / 2.0);

                // Convert to fractional bin indices
                let bin_low_frac = freq_low / bin_hz;
                let bin_high_frac = freq_high / bin_hz;

                OctaveBand {
                    bin_low_frac,
                    bin_high_frac,
                }
            })
            .collect()
    }

    /// Calculate frequency-dependent EMA coefficients
    fn calculate_ema_coefficients(
        display_freqs: &[f32],
        min_freq: f32,
        max_freq: f32,
        update_rate_hz: u32,
    ) -> Vec<f32> {
        let hop_time = 1.0 / update_rate_hz as f32;

        display_freqs
            .iter()
            .map(|&freq| {
                // Interpolate tau based on log frequency position
                let log_pos = (freq.log2() - min_freq.log2()) / (max_freq.log2() - min_freq.log2());
                let t = log_pos.clamp(0.0, 1.0);

                // Linear interpolation: low frequencies get TAU_LOW, high get TAU_HIGH
                let tau = TAU_LOW * (1.0 - t) + TAU_HIGH * t;

                // Convert tau to EMA coefficient: α = exp(-hop_time / tau)
                (-hop_time / tau).exp()
            })
            .collect()
    }

    /// Initialize analysis parameters for constant-Q processing
    fn initialize_params(num_bands: usize, config: &AnalyzerConfig) -> AnalysisParams {
        let display_frequencies =
            Self::calculate_display_frequencies(num_bands, config.min_freq, config.max_freq);

        let octave_bands =
            Self::calculate_octave_bands(&display_frequencies, config.sample_rate, config.fft_size);

        let ema_coefficients = Self::calculate_ema_coefficients(
            &display_frequencies,
            config.min_freq,
            config.max_freq,
            config.update_rate_hz,
        );

        AnalysisParams {
            octave_bands,
            ema_coefficients,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hann_window() {
        // This is the periodic (DFT-even) Hann window used for FFT analysis:
        // w[n] = 0.5 * (1 - cos(2*pi*n / N)). Only the first sample is zero;
        // it is NOT symmetric about its endpoints (unlike the symmetric window
        // that divides by N-1).
        let window = SpectrumAnalyzer::create_hann_window(8);
        assert_eq!(window.len(), 8);

        // First sample is exactly zero, peak is at the center.
        assert!(window[0] < 1e-6);
        assert!((window[4] - 1.0).abs() < 1e-6);

        // The window is symmetric about its center (w[n] == w[N-n]).
        assert!((window[1] - window[7]).abs() < 1e-6);
        assert!((window[2] - window[6]).abs() < 1e-6);
        assert!((window[3] - window[5]).abs() < 1e-6);

        // It rises monotonically from the first sample up to the center.
        for n in 0..4 {
            assert!(
                window[n] < window[n + 1],
                "expected w[{}] < w[{}]",
                n,
                n + 1
            );
        }
    }

    #[test]
    fn test_mixdown_mono() {
        let stereo = vec![1.0, 2.0, 3.0, 4.0]; // [L, R, L, R]
        let mono = SpectrumAnalyzer::mixdown_to_mono(&stereo, 2);
        assert_eq!(mono, vec![1.5, 3.5]); // [(1+2)/2, (3+4)/2]
    }

    #[test]
    fn test_mixdown_already_mono() {
        let mono_in = vec![1.0, 2.0, 3.0];
        let mono_out = SpectrumAnalyzer::mixdown_to_mono(&mono_in, 1);
        assert_eq!(mono_in, mono_out);
    }

    #[test]
    fn test_process_fft_reuses_output_buffer() {
        let config = AnalyzerConfig {
            fft_size: 64,
            ..AnalyzerConfig::default()
        };
        let mut analyzer = SpectrumAnalyzer::new(config);
        analyzer.params = Some(SpectrumAnalyzer::initialize_params(8, &analyzer.config));

        for (index, sample) in analyzer.ring_buffer.iter_mut().enumerate() {
            *sample = (2.0 * std::f32::consts::PI * index as f32 / 8.0).sin();
        }

        let output_ptr = analyzer.output_buffer.as_ptr();
        let bands = analyzer.process_fft(8);

        assert_eq!(analyzer.output_buffer.as_ptr(), output_ptr);
        assert_eq!(bands.len(), 8);
        assert!(bands.iter().all(|band| band.is_finite()));
    }
}
